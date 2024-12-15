use splixv2::shared::{GameMessage, Player};
use ggez::{Context, GameResult};
use ggez::graphics::{self, Color};
use ggez::event::{self, EventHandler};
use ggez::input::keyboard::KeyCode;
use ggez::input::keyboard;
use std::collections::{HashMap, VecDeque};
use tokio::net::{TcpStream, UdpSocket};
use tokio::io::AsyncReadExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use std::sync::Mutex as StdMutex;
use ggez::conf::{WindowMode, WindowSetup};

struct GameState {
    player: Arc<StdMutex<Player>>,
    other_players: Arc<StdMutex<HashMap<u32, Player>>>,
    udp_socket: Arc<UdpSocket>,
    movement_speed: f32,
    network_tx: mpsc::UnboundedSender<GameMessage>,
    last_received_positions: HashMap<u32, (f32, f32)>,
    interpolation_targets: HashMap<u32, (f32, f32)>,
    server_receiver: mpsc::UnboundedReceiver<GameMessage>, // Add this
    last_update: f64,
    current_direction: Option<KeyCode>,

}

impl GameState {
    async fn new(_ctx: &mut Context) -> GameResult<Self> {
        
        
        // Connect TCP
        let tcp_stream = TcpStream::connect("127.0.0.1:8080").await.unwrap();

        // Connect UDP
        let udp_socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        udp_socket.connect("127.0.0.1:8081").await.unwrap();
        let udp_socket = Arc::new(udp_socket);

        // Create channels for network communication
        let (network_tx, _network_rx) = mpsc::unbounded_channel();
        let (server_tx, server_receiver) = mpsc::unbounded_channel();

        // Initialize player and shared state
        let player = Arc::new(StdMutex::new(Player {
            id: 0,
            x: 100.0,
            y: 100.0,
            color: (255, 0, 0),
            territory: Vec::new(),
            current_path: Vec::new(),
        }));

        let other_players = Arc::new(StdMutex::new(HashMap::new()));
        let last_received_positions = HashMap::new();
        let interpolation_targets = HashMap::new();

        // Clone references for the network tasks
        let player_clone_tcp = player.clone();
        let player_clone_udp = player.clone();
        let other_players_clone = other_players.clone();
        let udp_socket_clone = udp_socket.clone();
        let server_tx_clone = server_tx.clone();

        // Spawn TCP message handling task
        let tcp_server_tx = server_tx.clone();
        tokio::spawn(async move {
            let mut tcp_stream = tcp_stream;

            // First, read our player update message
            let mut buffer = vec![0u8; 4096];
            let size = tcp_stream.read(&mut buffer).await.unwrap();
            if let Ok(GameMessage::PlayerUpdate(p)) = bincode::deserialize(&buffer[..size]) {
                let player_id = p.id;  // Store ID before moving p
                let mut player_lock = player_clone_tcp.lock().unwrap();
                *player_lock = p;
                println!("Got my player ID: {}", player_id);
            }

            // Then read initial game state
            let size = tcp_stream.read(&mut buffer).await.unwrap();
            if let Ok(GameMessage::GameState(players)) = bincode::deserialize(&buffer[..size]) {
                let player_id = player_clone_tcp.lock().unwrap().id;
                let mut others_lock = other_players_clone.lock().unwrap();

                for p in players {
                    if p.id != player_id {
                        others_lock.insert(p.id, p.clone());
                        // Initialize interpolation data
                        let _ = tcp_server_tx.send(GameMessage::PlayerUpdate(p));
                    }
                }
            }

            // Handle ongoing TCP messages
            loop {
                let mut buffer = vec![0u8; 4096];
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

        // Spawn UDP message handling task
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                match udp_socket_clone.recv_from(&mut buf).await {
                    Ok((size, _)) => {
                        if let Ok(msg) = bincode::deserialize(&buf[..size]) {
                            match &msg {
                                GameMessage::Position { id, .. } => {
                                    // Only forward position updates from other players
                                    let player_id = player_clone_udp.lock().unwrap().id;
                                    if *id != player_id {
                                        if let Err(e) = server_tx_clone.send(msg) {
                                            println!("Failed to forward UDP message: {}", e);
                                            break;
                                        }
                                    }
                                }
                                _ => {
                                    if let Err(e) = server_tx_clone.send(msg) {
                                        println!("Failed to forward UDP message: {}", e);
                                        break;
                                    }
                                }
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

        // Create and return GameState
        Ok(GameState {
            player,
            other_players,
            udp_socket,
            movement_speed: 5.0,
            network_tx,
            server_receiver,
            last_update: 0.0,
            last_received_positions,
            interpolation_targets,
            current_direction: None,
        })
    }

    fn flood_fill(territory: &mut Vec<(i32, i32)>, path: &[(i32, i32)]) {
        let mut visited = std::collections::HashSet::new();
        let mut queue = VecDeque::new();

        // Add the path to the territory and queue for flood-fill
        for &point in path {
            territory.push(point);
            visited.insert(point);
            queue.push_back(point);
        }

        while let Some((x, y)) = queue.pop_front() {
            // Check neighbors
            for (nx, ny) in [(x - 20, y), (x + 20, y), (x, y - 20), (x, y + 20)] {
                if !visited.contains(&(nx, ny)) {
                    visited.insert((nx, ny));
                    territory.push((nx, ny));
                    queue.push_back((nx, ny));
                }
            }
        }
    }

    fn spawn_in_territory(&mut self) {
        let player = self.player.lock().unwrap();
        // Collect all territory points
        let territory_points: Vec<(i32, i32)> = player.territory.clone();
        drop(player); // Release lock before spawning

        if !territory_points.is_empty() {
            // Pick random territory point
            let spawn_point = territory_points[rand::random::<usize>() % territory_points.len()];

            let mut player = self.player.lock().unwrap();
            // Set position to center of grid cell
            player.x = spawn_point.0 as f32 + 10.0; // Center in grid cell
            player.y = spawn_point.1 as f32 + 10.0;
            player.current_path.clear();

            // Send update to other clients
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
        let dt = ctx.time.delta().as_secs_f32();

        // Process all pending server messages
        while let Ok(message) = self.server_receiver.try_recv() {
            match message {
                GameMessage::GameState(players) => {
                    for player in players {
                        if player.id != self.player.lock().unwrap().id {
                            self.interpolation_targets.insert(player.id, (player.x, player.y));

                            if !self.last_received_positions.contains_key(&player.id) {
                                self.last_received_positions.insert(player.id, (player.x, player.y));
                            }

                            self.other_players.lock().unwrap().insert(player.id, player.clone());
                        }
                    }
                }
                GameMessage::PlayerUpdate(updated_player) => {
                    if updated_player.id != self.player.lock().unwrap().id {
                        self.interpolation_targets.insert(updated_player.id, (updated_player.x, updated_player.y));
                        self.other_players.lock().unwrap().insert(updated_player.id, updated_player);
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

        // Interpolate other players' positions
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

        // Handle key presses to set the current direction
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

        // Handle movement and related logic
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

            // Define scoreboard bounds
            let scoreboard_height = 40.0;
            let scoreboard_y_end = 0.0 + scoreboard_height + 5.0;

            // Define grid boundaries
            let grid_min_x = 0.0;
            let grid_max_x = 800.0;
            let grid_min_y = scoreboard_height + 5.0;
            let grid_max_y = 600.0;

            // Restrict player's position within boundaries
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

            if player.y < scoreboard_y_end {
                player.y = scoreboard_y_end;
            }

            let is_in_territory = player.territory.contains(&current_pos);
            if !is_in_territory {
                let current_point = (grid_x, grid_y);

                if !player.current_path.is_empty() &&
                    player.current_path[..player.current_path.len() - 1].contains(&current_point) {
                    drop(player);
                    self.spawn_in_territory();
                    return Ok(());
                }

                let others = self.other_players.lock().unwrap();
                for other_player in others.values() {
                    if other_player.current_path.contains(&current_point) {
                        drop(others);
                        drop(player);
                        self.spawn_in_territory();
                        return Ok(());
                    }
                }

                if player.current_path.is_empty() || player.current_path.last() != Some(&current_point) {
                    player.current_path.push(current_point);
                }
            } else if !player.current_path.is_empty() {
                let path = player.current_path.clone();
                player.territory.extend(path.iter().cloned());

                let min_x = path.iter().map(|(x, _)| *x).min().unwrap();
                let max_x = path.iter().map(|(x, _)| *x).max().unwrap();
                let min_y = path.iter().map(|(_, y)| *y).min().unwrap();
                let max_y = path.iter().map(|(_, y)| *y).max().unwrap();

                let path_set: std::collections::HashSet<_> = path.iter().cloned().collect();
                let territory_set: std::collections::HashSet<_> = player.territory.iter().cloned().collect();

                for y in (min_y..=max_y).step_by(20) {
                    let mut inside = false;
                    let mut last_boundary = None;

                    for x in (min_x..=max_x).step_by(20) {
                        let point = (x, y);

                        if path_set.contains(&point) || territory_set.contains(&point) {
                            if let Some(last_x) = last_boundary {
                                if x - last_x > 20 {
                                    inside = !inside;
                                }
                            } else {
                                inside = !inside;
                            }
                            last_boundary = Some(x);
                        } else if inside && !territory_set.contains(&point) {
                            player.territory.push(point);
                        }
                    }
                }

                player.current_path.clear();
            }

            let msg = GameMessage::PlayerUpdate(player.clone());
            if let Ok(data) = bincode::serialize(&msg) {
                let udp_socket = self.udp_socket.clone();
                tokio::spawn(async move {
                    let _ = udp_socket.send(&data).await;
                });
            }
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

        {
            // Lock other players and the main player
            let others = self.other_players.lock().unwrap();
            let player = self.player.lock().unwrap();

            // Prepare an initial x-coordinate for text placement
            let mut x_position = 10.0;
            let mut total_text_width = 0.0;
            let mut text_entries = vec![];

            // Add the main player (Player 1) first
            let main_player_text = format!("Player {} Territory: {}", player.id, player.territory.len());
            let main_player_graphic = graphics::Text::new(main_player_text.clone());
            let text_dimensions = main_player_graphic.measure(ctx)?;
            total_text_width += text_dimensions.x + 10.0; // Include spacing
            text_entries.push((main_player_graphic, player.color, x_position));

            x_position += text_dimensions.x + 10.0;

            // Collect other players sorted by their unique IDs, excluding the main player
            let mut other_players: Vec<_> = others.values().collect();
            other_players.sort_by_key(|p| p.id);

            // Add other players to the scoreboard
            for other_player in other_players {
                let other_player_text = format!("Player {} Territory: {}", other_player.id, other_player.territory.len());
                let other_player_graphic = graphics::Text::new(other_player_text.clone());
                let text_dimensions = other_player_graphic.measure(ctx)?;
                total_text_width += text_dimensions.x + 10.0; // Include spacing
                text_entries.push((other_player_graphic, other_player.color, x_position));

                x_position += text_dimensions.x + 10.0;
            }

            // Draw a solid background rectangle for the scoreboard
            let background_color = Color::new(0.0, 0.0, 0.0, 1.0); // Opaque black
            let background = graphics::Mesh::new_rectangle(
                ctx,
                graphics::DrawMode::fill(),
                graphics::Rect::new(0.0, 0.0, total_text_width + 20.0, 40.0), // Add padding for better spacing
                background_color,
            )?;
            canvas.draw(&background, graphics::DrawParam::default());

            // Draw all text entries
            for (text, color, position_x) in text_entries {
                let player_color = Color::from_rgb(color.0, color.1, color.2);
                canvas.draw(
                    &text,
                    graphics::DrawParam::default()
                        .dest([position_x, 10.0]) // Draw at the specified x position
                        .color(player_color),
                );
            }
        }

        // Draw other players
        let others = self.other_players.lock().unwrap();
        for player in others.values() {
            // Draw the player's territory
            for &(tx, ty) in &player.territory {
                let rect = graphics::Mesh::new_rectangle(
                    ctx,
                    graphics::DrawMode::fill(),
                    graphics::Rect::new(tx as f32, ty as f32, 20.0, 20.0),
                    Color::from_rgb(player.color.0, player.color.1, player.color.2),
                )?;
                canvas.draw(&rect, graphics::DrawParam::default());
            }

            // Draw other players' trails
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

            // Draw other players' positions
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

        // Draw the main player
        let player = self.player.lock().unwrap();

        // Draw main player's territory
        for &(tx, ty) in &player.territory {
            let rect = graphics::Mesh::new_rectangle(
                ctx,
                graphics::DrawMode::fill(),
                graphics::Rect::new(tx as f32, ty as f32, 20.0, 20.0),
                Color::from_rgb(player.color.0, player.color.1, player.color.2),
            )?;
            canvas.draw(&rect, graphics::DrawParam::default());
        }

        // Draw main player's trail
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

        // Draw main player's position
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

        canvas.finish(ctx)?;
        Ok(())
    }
}


#[tokio::main]
async fn main() -> GameResult {
    // Suppress ALSA warnings with all known methods
    std::env::set_var("ALSA_DEBUG", "0");
    std::env::set_var("RUST_LOG", "error");
    std::env::set_var("ALSA_CONFIG_PATH", "/dev/null");

    let cb = ggez::ContextBuilder::new("splix_clone", "you")
        .window_mode(WindowMode::default().dimensions(800.0, 600.0))
        .window_setup(WindowSetup::default()
            .title("Splix Clone")
            .vsync(true)
            .srgb(true));


    // Build context and handle errors
    let (mut ctx, event_loop) = match cb.clone().build() {  // Added clone() here
        Ok(result) => result,
        Err(_) => {
            // If first attempt fails, try again
            cb.build()?  // Using ? operator for error handling
        }
    };

    // Initialize game state
    let game_state = GameState::new(&mut ctx).await?;

    // Run the game
    event::run(ctx, event_loop, game_state)
}