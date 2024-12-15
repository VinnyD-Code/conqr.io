use splixv2::shared::{GameMessage, GamePhase, Player};
use ggez::{Context, GameResult};
use ggez::graphics::{self, Color};
use ggez::event::{self, EventHandler};
use ggez::input::keyboard::KeyCode;
use ggez::input::keyboard;
use ggez::audio::{SoundSource, Source, AudioContext};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::io::Write;
use std::net::IpAddr;
use tokio::net::{TcpStream, UdpSocket};
use tokio::io::AsyncReadExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;


struct GameState {
    player: Arc<StdMutex<Player>>,
    other_players: Arc<StdMutex<HashMap<u32, Player>>>,
    udp_socket: Arc<UdpSocket>,
    movement_speed: f32,
    network_tx: mpsc::UnboundedSender<GameMessage>,
    last_received_positions: HashMap<u32, (f32, f32)>,
    interpolation_targets: HashMap<u32, (f32, f32)>,
    server_receiver: mpsc::UnboundedReceiver<GameMessage>,
    last_update: f64,
    current_direction: Option<KeyCode>,
    game_start_time: std::time::Instant,
    game_duration: Duration,
    game_over: bool,
    shutdown_timer: Option<std::time::Instant>,
    shutdown_duration: Duration,
    game_start_offset: f64,
    game_state: GamePhase,
    in_lobby: bool,
    countdown_message: Option<String>,
    last_ready_toggle: std::time::Instant,
    audio_game_start: Source,
    audio_death: Source,
    audio_territory_claim: Source,
    audio_game_end: Source,
}

async fn get_server_ip() -> io::Result<String> {
    loop {
        print!("Input your server's IPv4 address: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        //Validate the IP address format
        match input.parse::<IpAddr>() {
            Ok(_) => return Ok(input.to_string()),
            Err(_) => {
                println!("Invalid IP address format. Please try again.");
                continue;
            }
        }
    }
}
impl GameState {
    async fn new(_ctx: &mut Context) -> GameResult<Self> {

        //initialize sound effects
        let audio_game_start = Source::new(_ctx,  "/audio/gameMusic.mp3")?;
        let audio_death = Source::new(_ctx, "/audio/8bit_bomb_explosion.wav")?;
        let audio_territory_claim = Source::new(_ctx, "/audio/mixkit-player-jumping-in-a-video-game-2043.wav")?;
        let audio_game_end = Source::new(_ctx, "/audio/Mario & Luigi Bowser's Inside Story Soundtrack - Victory Theme.mp3")?;

        let server_ip = get_server_ip().await
            .expect("Failed to get server IP address");

        println!("Attempting to connect to server at {}...", server_ip);

        //Connect TCP with retry logic
        let tcp_address = format!("{}:8080", server_ip);
        let tcp_stream = match TcpStream::connect(&tcp_address).await {
            Ok(stream) => stream,
            Err(e) => {
                println!("Failed to connect to TCP server at {}: {}", tcp_address, e);
                println!("Please check if:");
                println!("1. The server is running");
                println!("2. The IP address is correct");
                println!("3. Your firewall settings allow the connection");
                return Err(ggez::GameError::CustomError(String::from("Failed to connect to server")));
            }
        };

        //Connect UDP with retry logic
        let udp_socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        let udp_address = format!("{}:8081", server_ip);
        match udp_socket.connect(&udp_address).await {
            Ok(_) => {},
            Err(e) => {
                println!("Failed to connect to UDP server at {}: {}", udp_address, e);
                println!("Please check if:");
                println!("1. The server is running");
                println!("2. The IP address is correct");
                println!("3. Your firewall settings allow the connection");
                return Err(ggez::GameError::CustomError(String::from("Failed to connect to server")));
            }
        };
        let udp_socket = Arc::new(udp_socket);

        println!("Connected to server at {}", server_ip);

        //Create channels for network communication
        let (network_tx, _network_rx) = mpsc::unbounded_channel();
        let (server_tx, server_receiver) = mpsc::unbounded_channel();

        //Initialize player and shared state
        let player = Arc::new(StdMutex::new(Player {
            id: 0,
            x: 100.0,
            y: 100.0,
            color: (255, 0, 0),
            territory: Vec::new(),
            current_path: Vec::new(),
            ready: false,
        }));

        let other_players = Arc::new(StdMutex::new(HashMap::new()));
        let last_received_positions = HashMap::new();
        let interpolation_targets = HashMap::new();

        //Clone references for the network tasks
        let player_clone_tcp = player.clone();
        let player_clone_udp = player.clone();
        let other_players_clone = other_players.clone();
        let udp_socket_clone = udp_socket.clone();
        let server_tx_clone = server_tx.clone();

        //Spawn TCP message handling task
        let tcp_server_tx = server_tx.clone();
        tokio::spawn(async move {
            let mut tcp_stream = tcp_stream;

            //first read our player update message
            let mut buffer = vec![0u8; 8192];
            let size = tcp_stream.read(&mut buffer).await.unwrap();

            if let Ok(GameMessage::LobbyFull) = bincode::deserialize(&buffer[..size]) {
                println!("Cannot join - lobby is full (4 players maximum)");
                std::process::exit(0);
            }

            if let Ok(GameMessage::PlayerUpdate(p)) = bincode::deserialize(&buffer[..size]) {
                let player_id = p.id;  //Store ID before moving p
                let mut player_lock = player_clone_tcp.lock().unwrap();
                *player_lock = p;
                println!("Got my player ID: {}", player_id);
            }

            //Then read initial game state
            let size = tcp_stream.read(&mut buffer).await.unwrap();
            if let Ok(GameMessage::GameState(players)) = bincode::deserialize(&buffer[..size]) {
                let player_id = player_clone_tcp.lock().unwrap().id;
                let mut others_lock = other_players_clone.lock().unwrap();

                for p in players {
                    if p.id != player_id {
                        others_lock.insert(p.id, p.clone());
                        //Initialize interpolation data
                        let _ = tcp_server_tx.send(GameMessage::PlayerUpdate(p));
                    }
                }
            }

            //Handle ongoing TCP messages
            loop {
                let mut buffer = vec![0u8; 8192];
                match tcp_stream.read(&mut buffer).await {
                    Ok(0) => {
                        println!("Server disconnected");
                        break;
                    }
                    Ok(n) => {
                        if let Ok(msg) = bincode::deserialize(&buffer[..n]) {
                            if let Err(e) = tcp_server_tx.send(msg) {
                                println!("Failed to forward TCP message: {}", e);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        println!("TCP read error: {}", e);
                        break;
                    }
                }
            }
        });

        //Spawn UDP message handling task
        tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            loop {
                match udp_socket_clone.recv_from(&mut buf).await {
                    Ok((size, _)) => {
                        if let Ok(msg) = bincode::deserialize(&buf[..size]) {
                            //Forward ALL messages to main game loop for processing
                            if let Err(e) = server_tx_clone.send(msg) {
                                println!("Failed to forward UDP message: {}", e);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        println!("UDP read error: {}", e);
                        break;
                    }
                }
            }
        });

        println!("Client initialized and connected to server");

        let game_start_offset = 0.0;

        //Create and return GameState
        Ok(GameState {
            player,
            other_players,
            udp_socket,
            movement_speed: 3.0,
            network_tx,
            server_receiver,
            last_update: 0.0,
            last_received_positions,
            interpolation_targets,
            current_direction: None,
            game_start_time: std::time::Instant::now(),
            game_duration: Duration::from_secs(60),
            game_over: false,
            shutdown_timer: None,
            shutdown_duration: Duration::from_secs(10),
            game_start_offset,
            game_state: GamePhase::Lobby,
            in_lobby: true,
            countdown_message: None,
            last_ready_toggle: std::time::Instant::now(),
            audio_game_start,
            audio_death,
            audio_territory_claim,
            audio_game_end,
        })
    }

    fn validate_territory_claim(&self, new_territory: &[(i32, i32)]) -> bool {
        let current_territory = &self.player.lock().unwrap().territory;

        //Check if claim is too large
        if new_territory.len() > current_territory.len() + 200 {
            println!("Local validation: Territory claim too large");
            return false;
        }

        //Check if claim is contiguous with existing territory
        if !current_territory.is_empty() && !new_territory.is_empty() {
            let mut is_contiguous = false;
            for new_point in new_territory {
                for old_point in current_territory {
                    let dx = (new_point.0 - old_point.0).abs();
                    let dy = (new_point.1 - old_point.1).abs();
                    if dx <= 20 && dy <= 20 {
                        is_contiguous = true;
                        break;
                    }
                }
                if is_contiguous {
                    break;
                }
            }

            if !is_contiguous {
                return false;
            }
        }

        true
    }

    fn check_trail_collision(&mut self) -> Option<u32> {
        let current_player = self.player.lock().unwrap();
        let grid_x = (current_player.x / 20.0).floor() as i32 * 20;
        let grid_y = (current_player.y / 20.0).floor() as i32 * 20;
        let current_pos = (grid_x, grid_y);
        drop(current_player);

        let others = self.other_players.lock().unwrap();
        for (other_id, other_player) in others.iter() {
            if other_player.current_path.contains(&current_pos) {
                //Return the ID of the player whose trail was hit
                return Some(*other_id);
            }
        }
        None
    }

    fn send_death_message(&self, killed_player_id: u32) {
        let msg = GameMessage::PlayerDied { id: killed_player_id };
        if let Ok(data) = bincode::serialize(&msg) {
            let udp_socket = self.udp_socket.clone();
            tokio::spawn(async move {
                let _ = udp_socket.send(&data).await;
            });
        }
    }

    fn flood_fill(&mut self) -> Vec<(i32, i32)> {
        let (min_x, max_x, min_y, max_y) = {
            let path = &self.player.lock().unwrap().current_path;
            (
                path.iter().map(|(x, _)| *x).min().unwrap_or(0),
                path.iter().map(|(x, _)| *x).max().unwrap_or(0),
                path.iter().map(|(_, y)| *y).min().unwrap_or(0),
                path.iter().map(|(_, y)| *y).max().unwrap_or(0)
            )
        };

        let mut visited = std::collections::HashSet::new();
        let mut queue = VecDeque::new();
        let mut filled_area = Vec::new();

        //Create boundary set for efficient lookups
        let (boundary_set, territory_set): (HashSet<_>, HashSet<_>) = {
            let player = self.player.lock().unwrap();
            let others = self.other_players.lock().unwrap();

            //Keep path boundaries separate from territory
            let mut boundary = player.current_path.iter().cloned().collect::<HashSet<_>>();
            for other in others.values() {
                for point in &other.current_path {
                    boundary.insert(*point);
                }
            }

            let territory = player.territory.iter().cloned().collect();
            (boundary, territory)
        };

        //Start flood fill from multiple points along boundary
        //Include corners and midpoints of the bounding box
        let start_points = vec![
            (min_x - 20, min_y - 20), //Top-left corner
            (max_x + 20, min_y - 20), //Top-right corner
            (min_x - 20, max_y + 20), //Bottom-left corner
            (max_x + 20, max_y + 20), //Bottom-right corner
            ((min_x + max_x) / 2, min_y - 20), //Top middle
            ((min_x + max_x) / 2, max_y + 20), //Bottom middle
            (min_x - 20, (min_y + max_y) / 2), //Left middle
            (max_x + 20, (min_y + max_y) / 2), //Right middle
            ((min_x + max_x) / 2, (min_y + max_y) / 2),  //Center
            (min_x + 20, min_y + 20),  //Inner points
            (max_x - 20, min_y + 20),
            (min_x + 20, max_y - 20),
            (max_x - 20, max_y - 20),
        ];

        for &start_point in &start_points {
            queue.push_back(start_point);
        }

        let padding = 60;
        while let Some((x, y)) = queue.pop_front() {
            if x < min_x - padding || x > max_x + padding ||
                y < min_y - padding || y > max_y + padding {
                continue;
            }

            if visited.contains(&(x, y)) || boundary_set.contains(&(x, y)) {
                continue;
            }

            visited.insert((x, y));

            //Check if point is inside the shape using ray casting
            let is_inside = self.is_point_inside((x, y), &boundary_set);
            if is_inside {
                filled_area.push((x, y));
            }

            //Add neighbors with diagonal movements
            for &(dx, dy) in &[
                (-20, 0), (20, 0), (0, -20), (0, 20),
                (-20, -20), (-20, 20), (20, -20), (20, 20)
            ] {
                queue.push_back((x + dx, y + dy));
            }
        }

        filled_area
    }

    fn is_point_inside(&self, point: (i32, i32), boundary: &HashSet<(i32, i32)>) -> bool {
        let (x, y) = point;
        let mut count = 0;
        let mut last_y = None;

        //Cast ray to the right
        for test_x in (x..=800).step_by(20) {
            if let Some(boundary_y) = boundary.get(&(test_x, y)) {
                if Some(*boundary_y) != last_y {
                    count += 1;
                    last_y = Some(*boundary_y);
                }
            } else {
                last_y = None;
            }
        }

        count % 2 == 1
    }

    fn spawn_in_territory(&mut self) {
        let player = self.player.lock().unwrap();
        //Collect all territory points
        let territory_points: Vec<(i32, i32)> = player.territory.clone();
        drop(player); //Release lock before spawning

        if !territory_points.is_empty() {
            //Pick random territory point
            let spawn_point = territory_points[rand::random::<usize>() % territory_points.len()];

            let mut player = self.player.lock().unwrap();
            //Set position to center of grid cell
            player.x = spawn_point.0 as f32 + 10.0;
            player.y = spawn_point.1 as f32 + 10.0;
            player.current_path.clear();

            //Send update to other clients
            let msg = GameMessage::PlayerUpdate(player.clone());
            if let Ok(data) = bincode::serialize(&msg) {
                let udp_socket = self.udp_socket.clone();
                tokio::spawn(async move {
                    let _ = udp_socket.send(&data).await;
                });
            }
        }
    }
}

impl EventHandler for GameState {
    fn update(&mut self, ctx: &mut Context) -> GameResult {
        if self.game_state == GamePhase::Lobby {
            //Handle ready state toggling
            if keyboard::is_key_pressed(ctx, KeyCode::Space) {
                let now = std::time::Instant::now();
                if now.duration_since(self.last_ready_toggle).as_millis() > 500 {  //500ms debounce
                    self.last_ready_toggle = now;
                    let mut player = self.player.lock().unwrap();
                    player.ready = !player.ready;
                    let msg = GameMessage::PlayerReady { id: player.id };
                    drop(player);

                    if let Ok(data) = bincode::serialize(&msg) {
                        let udp_socket = self.udp_socket.clone();
                        tokio::spawn(async move {
                            let _ = udp_socket.send(&data).await;
                        });
                    }
                    let _ = self.network_tx.send(msg);
                }
            }

            //In client's update loop where we handle messages in lobby
            while let Ok(message) = self.server_receiver.try_recv() {
                match message {
                    GameMessage::PlayerJoined(new_player) => {
                        let player_id = self.player.lock().unwrap().id;
                        if new_player.id != player_id {
                            self.other_players.lock().unwrap()
                                .insert(new_player.id, new_player);
                        }
                    },
                    GameMessage::GameStarting => {
                        self.game_state = GamePhase::InGame;
                        self.in_lobby = false;
                        self.game_start_time = std::time::Instant::now();

                        //Start game music
                        self.audio_game_start.play(ctx)?;
                    },
                    GameMessage::PlayerUpdate(updated_player) => {
                        let player_id = self.player.lock().unwrap().id;
                        if updated_player.id != player_id {
                            //Update other player's state including ready status
                            self.other_players.lock().unwrap()
                                .insert(updated_player.id, updated_player);
                        } else {
                            //Update own ready state
                            let mut my_player = self.player.lock().unwrap();
                            my_player.ready = updated_player.ready;
                        }
                    },
                    GameMessage::GameState(players) => {
                        //Process full game state updates for consistency
                        let my_id = self.player.lock().unwrap().id;
                        for player in players {
                            if player.id == my_id {
                                let mut my_player = self.player.lock().unwrap();
                                my_player.ready = player.ready;
                            } else {
                                self.other_players.lock().unwrap()
                                    .insert(player.id, player);
                            }
                        }
                    },
                    GameMessage::GameUpdate(msg) => {
                        self.countdown_message = Some(msg);
                    },
                    _ => {}
                }
            }
            return Ok(());
        }

        if !self.game_over {
            //Check if time is up
            if self.game_start_time.elapsed() >= self.game_duration {
                self.game_over = true;

                //Find winner
                let mut max_territory = 0;
                let mut winner_id = 0;
                let mut winner_territory = 0;

                //Check main player
                {
                    let player = self.player.lock().unwrap();
                    let territory_size = player.territory.len();
                    if territory_size > max_territory {
                        max_territory = territory_size;
                        winner_id = player.id;
                        winner_territory = territory_size;
                    }
                }

                //Check other players
                {
                    let others = self.other_players.lock().unwrap();
                    for player in others.values() {
                        if player.territory.len() > max_territory {
                            max_territory = player.territory.len();
                            winner_id = player.id;
                            winner_territory = player.territory.len();
                        }
                    }
                }

                self.game_over = true;
                self.shutdown_timer = Some(std::time::Instant::now());

                self.audio_game_start.pause();

                let my_id = self.player.lock().unwrap().id;
                if winner_id == my_id {
                    // Main player wins

                    self.audio_game_end.play(ctx)?;
                } else {
                    // Main player loses
                    self.audio_death.play(ctx)?;
                }

                println!("Game Over! Player {} wins with claimed territory of: {}",
                         winner_id, winner_territory);

                let msg = GameMessage::GameOver {
                    winner_id,
                    territory_size: winner_territory
                };
                let _ = self.network_tx.send(msg);

                return Ok(());
            }
        } else {
            if let Some(shutdown_start) = self.shutdown_timer {
                if shutdown_start.elapsed() >= self.shutdown_duration {
                    std::process::exit(0);
                }
            }

            return Ok(());
        }

        let dt = ctx.time.delta().as_secs_f32();

        //Process all pending server messages
        while let Ok(message) = self.server_receiver.try_recv() {
            match message {
                GameMessage::GameTime(server_time) => {
                    self.game_start_offset = server_time;
                },
                GameMessage::GameUpdate(msg) => {
                    self.countdown_message = Some(msg); //Add this field to GameState
                },
                GameMessage::PlayerDied { id } => {
                    let my_id = self.player.lock().unwrap().id;
                    if id == my_id {
                        //We died, respawn in our territory
                        self.audio_death.play(ctx)?;
                        self.spawn_in_territory();
                    } else {
                        //Another player died, clear their path
                        if let Some(player) = self.other_players.lock().unwrap().get_mut(&id) {
                            player.current_path.clear();
                        }
                    }
                },
                GameMessage::GameState(players) => {
                    let start_time = std::time::SystemTime::now()
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_millis();

                    let mut others = self.other_players.lock().unwrap();
                    let my_id = self.player.lock().unwrap().id;

                    for updated_player in players {
                        if updated_player.id == my_id {
                            let mut my_player = self.player.lock().unwrap();

                            my_player.territory = updated_player.territory.clone();
                        } else {
                            others.insert(updated_player.id, updated_player.clone());
                        }
                    }
                }
                GameMessage::PlayerUpdate(updated_player) => {
                    let my_id = self.player.lock().unwrap().id;
                    if updated_player.id == my_id {
                        let mut my_player = self.player.lock().unwrap();
                        my_player.ready = updated_player.ready; //Make sure this is updated
                    } else {
                        let mut others = self.other_players.lock().unwrap();
                        others.insert(updated_player.id, updated_player);
                    }
                }
                GameMessage::PlayerLeft { id } => {
                    self.other_players.lock().unwrap().remove(&id);
                    self.last_received_positions.remove(&id);
                    self.interpolation_targets.remove(&id);
                }
                _ => {}
            }
        }

        //Interpolate other players' positions
        for (&id, &target) in &self.interpolation_targets.clone() {
            if let Some(current) = self.last_received_positions.get_mut(&id) {
                let dx = target.0 - current.0;
                let dy = target.1 - current.1;

                const INTERPOLATION_SPEED: f32 = 15.0;
                current.0 += dx * dt * INTERPOLATION_SPEED;
                current.1 += dy * dt * INTERPOLATION_SPEED;

                if let Some(player) = self.other_players.lock().unwrap().get_mut(&id) {
                    player.x = current.0;
                    player.y = current.1;
                }
            }
        }

        //Handle key presses to set the current direction
        if keyboard::is_key_pressed(ctx, KeyCode::Up) || keyboard::is_key_pressed(ctx, KeyCode::W) {
            self.current_direction = Some(KeyCode::Up);
        }
        if keyboard::is_key_pressed(ctx, KeyCode::Down) || keyboard::is_key_pressed(ctx, KeyCode::S) {
            self.current_direction = Some(KeyCode::Down);
        }
        if keyboard::is_key_pressed(ctx, KeyCode::Left) || keyboard::is_key_pressed(ctx, KeyCode::A) {
            self.current_direction = Some(KeyCode::Left);
        }
        if keyboard::is_key_pressed(ctx, KeyCode::Right) || keyboard::is_key_pressed(ctx, KeyCode::D) {
            self.current_direction = Some(KeyCode::Right);
        }

        //Handle movement and related logic
        {
            let mut player = self.player.lock().unwrap();

            if let Some(direction) = self.current_direction {
                match direction {
                    KeyCode::Up => player.y -= self.movement_speed * 0.25,
                    KeyCode::Down => player.y += self.movement_speed * 0.25,
                    KeyCode::Left => player.x -= self.movement_speed * 0.25,
                    KeyCode::Right => player.x += self.movement_speed * 0.25,
                    _ => {}
                }
            }

            //Define scoreboard bounds
            let scoreboard_height = 40.0;
            let scoreboard_y_end = 0.0 + scoreboard_height + 5.0;

            //Define grid boundaries
            let grid_min_x = 0.0;
            let grid_max_x = 800.0;
            let grid_min_y = scoreboard_height + 5.0;
            let grid_max_y = 600.0;

            //Restrict player's position within boundaries
            if player.x < grid_min_x {
                player.x = grid_min_x;
            } else if player.x > grid_max_x {
                player.x = grid_max_x;
            }
            if player.y < grid_min_y {
                player.y = grid_min_y;
            } else if player.y > grid_max_y {
                player.y = grid_max_y;
            }

            let grid_x = (player.x / 20.0).floor() as i32 * 20;
            let grid_y = (player.y / 20.0).floor() as i32 * 20;
            let current_pos = (grid_x, grid_y);

            let hit_player_id = {
                let others = self.other_players.lock().unwrap();
                others.iter()
                    .find(|(_, other_player)| other_player.current_path.contains(&current_pos))
                    .map(|(id, _)| *id)
            };

            if let Some(other_id) = hit_player_id {
                //Send death message for the player whose path was hit
                let msg = GameMessage::PlayerDied { id: other_id };
                if let Ok(data) = bincode::serialize(&msg) {
                    let udp_socket = self.udp_socket.clone();
                    tokio::spawn(async move {
                        let _ = udp_socket.send(&data).await;
                    });
                }
                //Also send via TCP for reliability
                let _ = self.network_tx.send(msg);

                return Ok(());
            }

            if player.y < scoreboard_y_end {
                player.y = scoreboard_y_end;
            }

            let is_in_territory = player.territory.contains(&current_pos);
            if !is_in_territory {
                let current_point = (grid_x, grid_y);

                //Modified path check
                if !player.current_path.is_empty() && player.current_path.len() > 1 &&
                    player.current_path[..player.current_path.len() - 1].contains(&current_point) {
                    let hit_path = {
                        let others = self.other_players.lock().unwrap();

                        //Single combined intersection check
                        let hit_own_path = !player.current_path.is_empty() &&
                            player.current_path.len() > 2 &&  //Allow at least 3 points
                            (0..player.current_path.len()-2)  //Don't check the last two points
                                .any(|i| player.current_path[i] == current_point);

                        let hit_other_path = others.values().any(|other_player|
                            other_player.current_path.contains(&current_point));

                        hit_own_path || hit_other_path
                    };

                    if hit_path {
                        drop(player);
                        self.spawn_in_territory();
                        self.audio_death.play(ctx)?;
                        return Ok(());
                    }
                }


                if player.current_path.is_empty() || player.current_path.last() != Some(&current_point) {
                    player.current_path.push(current_point);

                    //Send immediate path update
                    let msg = GameMessage::PlayerUpdate(player.clone());
                    if let Ok(data) = bincode::serialize(&msg) {
                        let udp_socket = self.udp_socket.clone();
                        tokio::spawn(async move {
                            let _ = udp_socket.send(&data).await;
                        });
                    }
                }
            } else if !player.current_path.is_empty() {
                let path = player.current_path.clone();

                //First validate that path doesn't intersect with other players' paths
                let mut valid_claim = true;
                {
                    let others = self.other_players.lock().unwrap();
                    for other_player in others.values() {
                        if path.iter().any(|point| other_player.current_path.contains(point)) {
                            valid_claim = false;
                            break;
                        }
                    }
                }

                if !valid_claim {
                    drop(player);
                    self.spawn_in_territory();
                    return Ok(());
                }

                //Calculate bounds first
                let min_x = path.iter().map(|(x, _)| *x).min().unwrap();
                let max_x = path.iter().map(|(x, _)| *x).max().unwrap();
                let min_y = path.iter().map(|(_, y)| *y).min().unwrap();
                let max_y = path.iter().map(|(_, y)| *y).max().unwrap();

                {
                    let mut others = self.other_players.lock().unwrap();
                    for other_player in others.values_mut() {
                        //Remove points from other player's territory that we're claiming
                        other_player.territory.retain(|point| !path.contains(point));
                    }
                }

                if !player.territory.is_empty() {
                    let mut is_contiguous = false;
                    for new_point in &path {
                        for old_point in &player.territory {
                            let dx = (new_point.0 - old_point.0).abs();
                            let dy = (new_point.1 - old_point.1).abs();
                            if dx <= 20 && dy <= 20 {
                                is_contiguous = true;
                                break;
                            }
                        }
                        if is_contiguous {
                            break;
                        }
                    }

                    if !is_contiguous {
                        drop(player);
                        self.spawn_in_territory();
                        return Ok(());
                    }
                }

                //Now proceed with the claim
                player.territory.extend(path.iter().cloned());
                self.audio_territory_claim.play(ctx)?;
                let boundary_set: std::collections::HashSet<_> = player.territory.iter().cloned().collect();

                //create a 2D grid to track visited points (only within claim bounds)
                let mut visited = std::collections::HashSet::new();
                let mut queue = VecDeque::new();

                //Start flood fill from edges of the bounding box
                for x in (min_x..=max_x).step_by(20) {
                    queue.push_back((x, min_y - 20)); //Top edge
                    queue.push_back((x, max_y + 20)); //Bottom edge
                }
                for y in (min_y..=max_y).step_by(20) {
                    queue.push_back((min_x - 20, y)); //Left edge
                    queue.push_back((max_x + 20, y)); //Right edge
                }

                //Get reference to others before flood fill
                let others_ref = self.other_players.lock().unwrap();

                while let Some((x, y)) = queue.pop_front() {
                    //Only process points within or adjacent to the bounding box
                    if x < min_x - 40 || x > max_x + 40 || y < min_y - 40 || y > max_y + 40 {
                        continue;
                    }

                    let crosses_path = player.current_path.contains(&(x, y)) ||
                        others_ref.values().any(|other| other.current_path.contains(&(x, y)));
                    if visited.contains(&(x, y)) || boundary_set.contains(&(x, y)) || crosses_path {
                        continue;
                    }

                    visited.insert((x, y));

                    //Add neighboring points
                    for (dx, dy) in [
                        (-20, 0), (20, 0), (0, -20), (0, 20),  //orthogonal
                        (-20, -20), (-20, 20), (20, -20), (20, 20)  //diagonal
                    ] {
                        queue.push_back((x + dx, y + dy));
                    }
                }

                //Drop the reference after we're done with flood fill
                drop(others_ref);

                //Points inside the claim that weren't reached by flood fill are part of the territory
                let mut new_territory = player.territory.clone();
                let points_to_add: Vec<_> = (min_y..=max_y).step_by(20)
                    .flat_map(|y| (min_x..=max_x).step_by(20)
                        .map(move |x| (x, y)))
                    .filter(|point| !visited.contains(point))
                    .filter(|point| !boundary_set.contains(point))
                    .collect();
                new_territory.extend(points_to_add);
                player.territory = new_territory;

                let mut others = self.other_players.lock().unwrap();
                for other_player in others.values_mut() {
                    let territory_before = other_player.territory.len();
                    let mut removed_points = Vec::new();

                    other_player.territory.retain(|point| {
                        let (px, py) = *point;
                        let in_capture_box = px >= min_x && px <= max_x && py >= min_y && py <= max_y;
                        let should_remove = in_capture_box && !visited.contains(point);  //If it wasn't visited during flood fill, it's inside

                        if should_remove {
                            removed_points.push(*point);
                            false
                        } else {
                            true
                        }
                    });

                    //If territory changed, send updates to both TCP and UDP
                    if territory_before != other_player.territory.len() {

                        //Create comprehensive update
                        let update_msg = GameMessage::PlayerUpdate(other_player.clone());

                        //Send via UDP for immediate update
                        if let Ok(data) = bincode::serialize(&update_msg) {
                            let udp_socket = self.udp_socket.clone();
                            tokio::spawn(async move {
                                let _ = udp_socket.send(&data).await;
                            });
                        }

                        //Also send via TCP for reliability
                        let _ = self.network_tx.send(update_msg.clone());

                        //Send territory update specifically
                        let territory_msg = GameMessage::TerritoryUpdate {
                            player_id: other_player.id,
                            removed_points,
                            new_territory: other_player.territory.clone(),
                        };
                        let _ = self.network_tx.send(territory_msg);
                    }
                }
                drop(others);

                player.current_path.clear();
            }

            let msg = GameMessage::PlayerUpdate(player.clone());
            if let Ok(data) = bincode::serialize(&msg) {
                let udp_socket = self.udp_socket.clone();
                tokio::spawn(async move {
                    let _ = udp_socket.send(&data).await;
                });
            }
            let _ = self.network_tx.send(msg.clone());
        }

        let current_time = ctx.time.time_since_start().as_secs_f64();
        if current_time - self.last_update >= 0.016 {
            let player = self.player.lock().unwrap();
            let msg = GameMessage::Position {
                id: player.id,
                x: player.x,
                y: player.y,
            };
            let _ = self.network_tx.send(msg);
            self.last_update = current_time;
        }

        Ok(())
    }


    fn draw(&mut self, ctx: &mut Context) -> GameResult {
        let mut canvas = graphics::Canvas::from_frame(ctx, Color::BLACK);

        match self.game_state {
            GamePhase::Lobby => {
                //Draw lobby UI
                let title = graphics::Text::new(
                    graphics::TextFragment::new("Waiting for players...").scale(32.0)
                );
                let display_text = if let Some(ref countdown) = self.countdown_message {
                    countdown.clone()
                } else {
                    "Press SPACE to toggle ready state".to_string()
                };

                let instructions = graphics::Text::new(display_text);

                canvas.draw(&title,
                            graphics::DrawParam::default()
                                .dest([400.0 - title.measure(ctx)?.x / 2.0, 100.0]));
                canvas.draw(&instructions,
                            graphics::DrawParam::default()
                                .dest([400.0 - instructions.measure(ctx)?.x / 2.0, 150.0]));

                //Draw player list
                let mut y_pos = 200.0;
                {
                    let player = self.player.lock().unwrap();
                    let others = self.other_players.lock().unwrap();

                    //Draw main player status
                    let status = if player.ready { "Ready" } else { "Not Ready" };
                    let player_text = graphics::Text::new(
                        format!("Player {} (You) - {}", player.id, status)
                    );
                    canvas.draw(&player_text,
                                graphics::DrawParam::default()
                                    .dest([400.0 - player_text.measure(ctx)?.x / 2.0, y_pos])
                                    .color(Color::from_rgb(player.color.0, player.color.1, player.color.2))
                    );
                    y_pos += 30.0;

                    //Draw other players
                    for other in others.values() {
                        let status = if other.ready { "Ready" } else { "Not Ready" };
                        let text = graphics::Text::new(
                            format!("Player {} - {}", other.id, status)
                        );
                        canvas.draw(&text,
                                    graphics::DrawParam::default()
                                        .dest([400.0 - text.measure(ctx)?.x / 2.0, y_pos])
                                        .color(Color::from_rgb(other.color.0, other.color.1, other.color.2))
                        );
                        y_pos += 30.0;
                    }
                }
            },
            GamePhase::CountdownStarted => {
                let countdown_text = graphics::Text::new(
                    graphics::TextFragment::new("Game starting...").scale(48.0)
                );
                canvas.draw(&countdown_text,
                            graphics::DrawParam::default()
                                .dest([400.0 - countdown_text.measure(ctx)?.x / 2.0, 250.0]));
            },
            GamePhase::InGame => {
                if self.game_over {
                    let game_over_text = graphics::Text::new(
                        graphics::TextFragment::new("Game Over!").scale(48.0)
                    );
                    let mut others = self.other_players.lock().unwrap();
                    let player = self.player.lock().unwrap();

                    //Find winner again
                    let mut max_territory = player.territory.len();
                    let mut winner_id = player.id;

                    for other in others.values() {
                        if other.territory.len() > max_territory {
                            max_territory = other.territory.len();
                            winner_id = other.id;
                        }
                    }

                    let winner_text = graphics::Text::new(
                        graphics::TextFragment::new(
                            format!("Player {} wins with territory: {}", winner_id, max_territory)
                        ).scale(32.0)
                    );

                    //Only show shutdown timer if we have one
                    if let Some(shutdown_start) = self.shutdown_timer {
                        let remaining = self.shutdown_duration.as_secs().saturating_sub(shutdown_start.elapsed().as_secs());
                        let shutdown_text = graphics::Text::new(
                            graphics::TextFragment::new(
                                format!("Game will shut down in {} seconds", remaining)
                            ).scale(24.0)
                        );
                        canvas.draw(&shutdown_text,
                                    graphics::DrawParam::default()
                                        .dest([400.0 - shutdown_text.measure(ctx)?.x / 2.0, 350.0]));
                    }

                    canvas.draw(&game_over_text,
                                graphics::DrawParam::default()
                                    .dest([400.0 - game_over_text.measure(ctx)?.x / 2.0, 250.0]));
                    canvas.draw(&winner_text,
                                graphics::DrawParam::default()
                                    .dest([400.0 - winner_text.measure(ctx)?.x / 2.0, 300.0]));
                } else {
                    let remaining = self.game_duration.as_secs().saturating_sub(self.game_start_time.elapsed().as_secs());
                    let time_text = graphics::Text::new(
                        format!("Time: {}s", remaining)
                    );
                    canvas.draw(&time_text,
                                graphics::DrawParam::default().dest([700.0, 10.0]));
                    {
                        //Lock other players and the main player
                        let others = self.other_players.lock().unwrap();
                        let player = self.player.lock().unwrap();

                        //Prepare an initial x-coordinate for text placement
                        let mut x_position = 10.0;
                        let mut total_text_width = 0.0;
                        let mut text_entries = vec![];

                        //Add the main player (Player 1) first
                        let main_player_text = format!("Player {} Territory: {}", player.id, player.territory.len());
                        let main_player_graphic = graphics::Text::new(main_player_text.clone());
                        let text_dimensions = main_player_graphic.measure(ctx)?;
                        total_text_width += text_dimensions.x + 10.0; //Include spacing
                        text_entries.push((main_player_graphic, player.color, x_position));

                        x_position += text_dimensions.x + 10.0;

                        //Collect other players sorted by their unique IDs, excluding the main player
                        let mut other_players: Vec<_> = others.values().collect();
                        other_players.sort_by_key(|p| p.id);

                        //Add other players to the scoreboard
                        for other_player in other_players {
                            let other_player_text = format!("Player {} Territory: {}", other_player.id, other_player.territory.len());
                            let other_player_graphic = graphics::Text::new(other_player_text.clone());
                            let text_dimensions = other_player_graphic.measure(ctx)?;
                            total_text_width += text_dimensions.x + 10.0;
                            text_entries.push((other_player_graphic, other_player.color, x_position));

                            x_position += text_dimensions.x + 10.0;
                        }

                        //Draw a solid background rectangle for the scoreboard
                        let background_color = Color::new(0.0, 0.0, 0.0, 1.0);
                        let background = graphics::Mesh::new_rectangle(
                            ctx,
                            graphics::DrawMode::fill(),
                            graphics::Rect::new(0.0, 0.0, total_text_width + 20.0, 40.0),
                            background_color,
                        )?;
                        canvas.draw(&background, graphics::DrawParam::default());

                        //Draw all text entries
                        for (text, color, position_x) in text_entries {
                            let player_color = Color::from_rgb(color.0, color.1, color.2);
                            canvas.draw(
                                &text,
                                graphics::DrawParam::default()
                                    .dest([position_x, 10.0]) //Draw at the specified x position
                                    .color(player_color),
                            );
                        }
                    }

                    //Draw other players
                    let others = self.other_players.lock().unwrap();
                    for player in others.values() {
                        //Draw the player's territory
                        for &(tx, ty) in &player.territory {
                            let rect = graphics::Mesh::new_rectangle(
                                ctx,
                                graphics::DrawMode::fill(),
                                graphics::Rect::new(tx as f32, ty as f32, 20.0, 20.0),
                                Color::from_rgb(player.color.0, player.color.1, player.color.2),
                            )?;
                            canvas.draw(&rect, graphics::DrawParam::default());
                        }

                        //Draw other players' trails
                        for &(px, py) in &player.current_path {
                            let rect = graphics::Mesh::new_rectangle(
                                ctx,
                                graphics::DrawMode::fill(),
                                graphics::Rect::new(px as f32, py as f32, 20.0, 20.0),
                                Color::from_rgb(player.color.0, player.color.1, player.color.2),
                            )?;
                            canvas.draw(&rect, graphics::DrawParam::default().color(Color::new(
                                player.color.0 as f32 / 255.0,
                                player.color.1 as f32 / 255.0,
                                player.color.2 as f32 / 255.0,
                                0.5,
                            )));
                        }

                        //Draw other players' positions
                        let darker_color = Color::from_rgb(
                            (player.color.0 as f32 * 0.7) as u8,
                            (player.color.1 as f32 * 0.7) as u8,
                            (player.color.2 as f32 * 0.7) as u8,
                        );
                        let circle = graphics::Mesh::new_circle(
                            ctx,
                            graphics::DrawMode::fill(),
                            [player.x, player.y],
                            10.0,
                            0.1,
                            darker_color,
                        )?;
                        canvas.draw(&circle, graphics::DrawParam::default());
                    }

                    //Draw the main player
                    let player = self.player.lock().unwrap();

                    //Draw main player's territory
                    for &(tx, ty) in &player.territory {
                        let rect = graphics::Mesh::new_rectangle(
                            ctx,
                            graphics::DrawMode::fill(),
                            graphics::Rect::new(tx as f32, ty as f32, 20.0, 20.0),
                            Color::from_rgb(player.color.0, player.color.1, player.color.2),
                        )?;
                        canvas.draw(&rect, graphics::DrawParam::default());
                    }

                    //Draw main player's trail
                    for &(px, py) in &player.current_path {
                        let rect = graphics::Mesh::new_rectangle(
                            ctx,
                            graphics::DrawMode::fill(),
                            graphics::Rect::new(px as f32, py as f32, 20.0, 20.0),
                            Color::from_rgb(player.color.0, player.color.1, player.color.2),
                        )?;
                        canvas.draw(&rect, graphics::DrawParam::default().color(Color::new(
                            player.color.0 as f32 / 255.0,
                            player.color.1 as f32 / 255.0,
                            player.color.2 as f32 / 255.0,
                            0.5,
                        )));
                    }

                    //Draw main player's position
                    let darker_color = Color::from_rgb(
                        (player.color.0 as f32 * 0.7) as u8,
                        (player.color.1 as f32 * 0.7) as u8,
                        (player.color.2 as f32 * 0.7) as u8,
                    );
                    let circle = graphics::Mesh::new_circle(
                        ctx,
                        graphics::DrawMode::fill(),
                        [player.x, player.y],
                        10.0,
                        0.1,
                        darker_color,
                    )?;

                    canvas.draw(&circle, graphics::DrawParam::default());
                }
            },
            GamePhase::GameOver => {
                let game_over_text = graphics::Text::new(
                    graphics::TextFragment::new("Game Over!").scale(48.0)
                );
                let mut others = self.other_players.lock().unwrap();
                let player = self.player.lock().unwrap();

                let mut max_territory = player.territory.len();
                let mut winner_id = player.id;

                for other in others.values() {
                    if other.territory.len() > max_territory {
                        max_territory = other.territory.len();
                        winner_id = other.id;
                    }
                }



                let winner_text = graphics::Text::new(


                    graphics::TextFragment::new(
                        format!("Player {} wins with territory: {}", winner_id, max_territory)
                    ).scale(32.0)

                );

                if let Some(shutdown_start) = self.shutdown_timer {
                    let remaining = self.shutdown_duration.as_secs().saturating_sub(shutdown_start.elapsed().as_secs());
                    let shutdown_text = graphics::Text::new(
                        graphics::TextFragment::new(
                            format!("Game will shut down in {} seconds", remaining)
                        ).scale(24.0)
                    );
                    canvas.draw(&shutdown_text,
                                graphics::DrawParam::default()
                                    .dest([400.0 - shutdown_text.measure(ctx)?.x / 2.0, 350.0]));
                }

                canvas.draw(&game_over_text,
                            graphics::DrawParam::default()
                                .dest([400.0 - game_over_text.measure(ctx)?.x / 2.0, 250.0]));
                canvas.draw(&winner_text,
                            graphics::DrawParam::default()
                                .dest([400.0 - winner_text.measure(ctx)?.x / 2.0, 300.0]));
            }
        }

        canvas.finish(ctx)?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> GameResult {
    let cb = ggez::ContextBuilder::new("splix_clone", "you").add_resource_path("./resources");;
    let (mut ctx, event_loop) = cb.build()?;

    let game_state = GameState::new(&mut ctx).await?;
    event::run(ctx, event_loop, game_state)
}