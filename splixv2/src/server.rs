use splixv2::shared::{GameMessage, Player};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{broadcast, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{interval, Duration};
use rand::Rng;

struct GameServer {
    players: Arc<Mutex<HashMap<u32, Player>>>,
    tcp_tx: broadcast::Sender<GameMessage>,
    next_player_id: Arc<Mutex<u32>>,
    game_over: Arc<Mutex<bool>>,
    game_start_time: std::time::Instant,
    game_started: Arc<Mutex<bool>>,
    countdown_cancel: Arc<Mutex<bool>>,
}

impl GameServer {
    fn new(tcp_tx: broadcast::Sender<GameMessage>) -> Self {
        Self {
            players: Arc::new(Mutex::new(HashMap::new())),
            tcp_tx: tcp_tx,
            next_player_id: Arc::new(Mutex::new(1)),
            game_over: Arc::new(Mutex::new(false)),
            game_start_time: std::time::Instant::now(),
            game_started: Arc::new(Mutex::new(false)),
            countdown_cancel: Arc::new(Mutex::new(false)),
        }
    }

    fn generate_random_color() -> (u8, u8, u8) {
        let mut rng = rand::thread_rng();
        (
            rng.gen_range(50..200),
            rng.gen_range(50..200),
            rng.gen_range(50..200),
        )
    }

    async fn check_game_start(server: Arc<GameServer>, udp_socket: Arc<UdpSocket>) {
        let should_start = {
            let players = server.players.lock().await;
            let ready_count = players.values().filter(|p| p.ready).count();
            let total_count = players.len();
            ready_count > 0 && ready_count == total_count
        };

        //If not all players are ready
        if !should_start {
            *server.countdown_cancel.lock().await = true;
            *server.game_started.lock().await = false;

            let wait_msg = GameMessage::GameUpdate("Waiting for all players to be ready...".to_string());


            //Send via UDP first for immediate update
            if let Ok(data) = bincode::serialize(&wait_msg) {
                let _ = udp_socket.send(&data).await;
            }
            //Then via TCP for reliability
            let _ = server.tcp_tx.send(wait_msg);
            return;
        }

        //Only start countdown if everyone is ready and we haven't started yet
        if should_start && !*server.game_started.lock().await {
            *server.game_started.lock().await = true;
            *server.countdown_cancel.lock().await = false;

            let server_clone = server.clone();
            let udp_socket = udp_socket.clone();

            tokio::spawn(async move {
                for i in (1..=5).rev() {
                    if *server_clone.countdown_cancel.lock().await {
                        let cancel_msg = GameMessage::GameUpdate("Waiting for all players to be ready...".to_string());

                        //Send via UDP first
                        if let Ok(data) = bincode::serialize(&cancel_msg) {
                            let _ = udp_socket.send(&data).await;
                        }
                        //Then TCP
                        let _ = server_clone.tcp_tx.send(cancel_msg);
                        return;
                    }

                    let countdown_msg = GameMessage::GameUpdate(format!("Game starting in {}...", i));


                    //Send countdown via UDP first
                    if let Ok(data) = bincode::serialize(&countdown_msg) {
                        let _ = udp_socket.send(&data).await;
                    }

                    //Then TCP
                    let _ = server_clone.tcp_tx.send(countdown_msg);


                    tokio::time::sleep(Duration::from_secs(1)).await;
                }

                if !*server_clone.countdown_cancel.lock().await {
                    let start_msg = GameMessage::GameStarting;
                    //Send final start message on both channels
                    if let Ok(data) = bincode::serialize(&start_msg) {
                        let _ = udp_socket.send(&data).await;
                    }
                    let _ = server_clone.tcp_tx.send(start_msg);
                }
            });
        }
    }
}

#[tokio::main]
async fn main() {
    let tcp_listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
    let udp_socket = Arc::new(UdpSocket::bind("0.0.0.0:8081").await.unwrap());
    let (tcp_tx, _) = broadcast::channel(1000);

    let game_server = Arc::new(GameServer::new(tcp_tx.clone()));

    println!("Server listening on TCP: 10.63.27.143:8080, UDP: 10.63.27.143:8081");

    //Create periodic state broadcast task
    let game_server_clone = game_server.clone();
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_millis(50));
        loop {
            interval.tick().await;

            let players = game_server_clone.players.lock().await;

            let state = GameMessage::GameState(players.values().cloned().collect());

            let _ = game_server_clone.tcp_tx.send(state);
        }
    });
    //Main UDP message handling task
    let udp_socket_clone = udp_socket.clone();
    let game_server_clone = game_server.clone();
    let game_server_clone2 = game_server_clone.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 8192];
        loop {
            if let Ok((size, _)) = udp_socket_clone.recv_from(&mut buf).await {
                //Add the message size check right here
                if let Ok(msg) = bincode::deserialize::<GameMessage>(&buf[..size]) {
                    if let GameMessage::PlayerUpdate(ref player) = msg {
                        if let Ok(serialized) = bincode::serialize(&msg) {
                            println!("Message size for territory update: {} bytes", serialized.len());
                        }
                    }
                    match msg {
                        GameMessage::GameStarting => {
                            *game_server_clone.game_started.lock().await = true;

                            //Start the game timer
                            let game_server_timer = game_server_clone.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(60)).await;

                                let mut game_over = game_server_timer.game_over.lock().await;
                                *game_over = true;

                                //Get final player states
                                let players = game_server_timer.players.lock().await;

                                //Find winner
                                let mut max_territory = 0;
                                let mut winner_id = 0;

                                for player in players.values() {
                                    if player.territory.len() > max_territory {
                                        max_territory = player.territory.len();
                                        winner_id = player.id;
                                    }
                                }

                                //Broadcast game over
                                let game_over_msg = GameMessage::GameOver {
                                    winner_id,
                                    territory_size: max_territory,
                                };

                                let _ = game_server_timer.tcp_tx.send(game_over_msg);

                                //Add small delay for clients to receive game over
                                tokio::time::sleep(Duration::from_secs(2)).await;

                                //Exit the server process
                                std::process::exit(0);
                            });
                        },
                        GameMessage::PlayerDied { id } => {
                            if *game_server_clone.game_over.lock().await {
                                continue;  //Skip if game is over
                            }

                            //Get the player who died
                            let mut players = game_server_clone.players.lock().await;
                            if let Some(player) = players.get_mut(&id) {
                                //Clear their current path
                                player.current_path.clear();

                                //Reset their position to a random point in their territory
                                if !player.territory.is_empty() {
                                    let territory_points: Vec<(i32, i32)> = player.territory.clone();
                                    let spawn_point = territory_points[rand::random::<usize>() % territory_points.len()];
                                    player.x = spawn_point.0 as f32 + 10.0;
                                    player.y = spawn_point.1 as f32 + 10.0;
                                }

                                //Broadcast the death and updated player state
                                let death_msg = GameMessage::PlayerDied { id };
                                let update_msg = GameMessage::PlayerUpdate(player.clone());

                                //Send via both UDP for immediate update and TCP for reliability
                                if let Ok(data) = bincode::serialize(&death_msg) {
                                    let _ = udp_socket_clone.send(&data).await;
                                }
                                if let Ok(data) = bincode::serialize(&update_msg) {
                                    let _ = udp_socket_clone.send(&data).await;
                                }

                                let _ = game_server_clone.tcp_tx.send(death_msg);
                                let _ = game_server_clone.tcp_tx.send(update_msg);
                            }
                        },
                        GameMessage::PlayerReady { id } => {
                            let mut players = game_server_clone.players.lock().await;
                            if let Some(player) = players.get_mut(&id) {
                                player.ready = !player.ready; //Toggle ready state

                                //Create a full player update that includes the ready state
                                let update_msg = GameMessage::PlayerUpdate(player.clone());

                                //Send this update through TCP and UDP
                                if let Ok(data) = bincode::serialize(&update_msg) {
                                    let _ = udp_socket_clone.send(&data).await;
                                }
                                let _ = game_server_clone.tcp_tx.send(update_msg.clone());

                                //Also send a full GameState update to ensure consistency
                                let state_msg = GameMessage::GameState(players.values().cloned().collect());
                                let _ = game_server_clone.tcp_tx.send(state_msg);

                                drop(players);
                                GameServer::check_game_start(game_server_clone.clone(), udp_socket_clone.clone()).await;
                            }
                        },
                        GameMessage::PlayerUpdate(updated_player) => {
                            if *game_server_clone.game_over.lock().await {
                                continue;  //Skip updates if game is over
                            }

                            let mut players = game_server_clone.players.lock().await;

                            if let Some(existing_player) = players.get(&updated_player.id) {
                                //Calculate overlap
                                let existing_set: std::collections::HashSet<_> = existing_player.territory.iter().collect();
                                let new_set: std::collections::HashSet<_> = updated_player.territory.iter().collect();
                                let overlap = existing_set.intersection(&new_set).count();
                            }

                            //First collect player data with immutable reference
                            let player_data = players.get(&updated_player.id).map(|p| p.territory.len());
                            let other_territories: Vec<(u32, Vec<(i32, i32)>)> = players.iter()
                                .filter(|(&id, _)| id != updated_player.id)
                                .map(|(&id, p)| (id, p.territory.clone()))
                                .collect();

                            //If we found the player, proceed with update
                            if let Some(old_territory_size) = player_data {
                                //Calculate bounds for logging
                                let (min_x, max_x, min_y, max_y) = if !updated_player.territory.is_empty() {
                                    let min_x = updated_player.territory.iter().map(|(x, _)| *x).min().unwrap();
                                    let max_x = updated_player.territory.iter().map(|(x, _)| *x).max().unwrap();
                                    let min_y = updated_player.territory.iter().map(|(_, y)| *y).min().unwrap();
                                    let max_y = updated_player.territory.iter().map(|(_, y)| *y).max().unwrap();
                                    (min_x, max_x, min_y, max_y)
                                } else {
                                    (0, 0, 0, 0)
                                };

                                let area_width = (max_x - min_x) / 20;
                                let area_height = (max_y - min_y) / 20;
                                let area_squares = area_width * area_height;

                                //Handle territory update and stealing in one atomic operation
                                if let Some(player) = players.get_mut(&updated_player.id) {
                                    //Update the player's territory directly
                                    *player = updated_player.clone();

                                    //Handle territory stealing
                                    let mut territories_changed = false;
                                    for (&id, other_player) in players.iter_mut() {
                                        if id != updated_player.id {
                                            let territory_before = other_player.territory.len();
                                            let mut removed_points = Vec::new();

                                            other_player.territory.retain(|point| {
                                                let (px, py) = *point;
                                                let in_capture_box = px >= min_x && px <= max_x && py >= min_y && py <= max_y;
                                                let should_remove = in_capture_box && updated_player.territory.contains(point);

                                                if should_remove {
                                                    removed_points.push(*point);
                                                    false
                                                } else {
                                                    true
                                                }
                                            });

                                            if territory_before != other_player.territory.len() {
                                                territories_changed = true;

                                                //Send territory update
                                                let territory_msg = GameMessage::TerritoryUpdate {
                                                    player_id: id,
                                                    removed_points,
                                                    new_territory: other_player.territory.clone(),
                                                };
                                                let _ = game_server_clone.tcp_tx.send(territory_msg);
                                            }
                                        }
                                    }

                                    //Send final state updates
                                    let state_msg = GameMessage::GameState(players.values().cloned().collect());
                                    drop(players);
                                    let _ = game_server_clone.tcp_tx.send(state_msg);
                                    let _ = game_server_clone.tcp_tx.send(GameMessage::PlayerUpdate(updated_player.clone()));
                                }
                            }
                        },
                        GameMessage::Position { id, x, y } => {
                            if *game_server_clone.game_over.lock().await {
                                continue;  //Skip movement if game is over
                            }

                            let mut players = game_server_clone.players.lock().await;
                            if let Some(player) = players.get_mut(&id) {
                                player.x = x;
                                player.y = y;
                                let update_msg = GameMessage::PlayerUpdate(player.clone());
                                drop(players);
                                //Make sure to broadcast position updates
                                let _ = game_server_clone.tcp_tx.send(update_msg);
                            }
                        }
                        GameMessage::TerritoryUpdate { player_id, removed_points: _, new_territory } => {
                            if *game_server_clone2.game_over.lock().await {
                                continue;  //Skip updates if game is over
                            }

                            let mut players = game_server_clone2.players.lock().await;
                            if let Some(player) = players.get_mut(&player_id) {
                                //First clear the old territory
                                let old_territory = player.territory.clone();
                                player.territory.clear();
                                let clear_msg = GameMessage::PlayerUpdate(player.clone());
                                let _ = game_server_clone2.tcp_tx.send(clear_msg);

                                //Then update with the new territory
                                player.territory = new_territory;
                                let update_msg = GameMessage::PlayerUpdate(player.clone());
                                let all_players: Vec<Player> = players.values().cloned().collect();
                                drop(players);

                                let state_msg = GameMessage::GameState(all_players);
                                let _ = game_server_clone2.tcp_tx.send(state_msg);
                                let _ = game_server_clone2.tcp_tx.send(update_msg);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    while let Ok((tcp_stream, addr)) = tcp_listener.accept().await {
        println!("New client connected from: {}", addr);
        let game_server = game_server.clone();

        //Generate color before spawning the task
        let color = GameServer::generate_random_color();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(tcp_stream, game_server, color).await {
                println!("Client connection error: {}", e);
            }
        });
    }
}

async fn handle_connection(
    mut tcp_stream: TcpStream,
    game_server: Arc<GameServer>,
    color: (u8, u8, u8),
) -> Result<(), Box<dyn std::error::Error>> {
    let player_count = {
        let players = game_server.players.lock().await;
        players.len()
    };

    if player_count >= 4 {
        // serialize and send LobbyFull message
        let msg = GameMessage::LobbyFull;
        if let Ok(data) = bincode::serialize(&msg) {
            tcp_stream.write_all(&data).await?;
            tcp_stream.flush().await?;
        }
        return Ok(());
    }

    let player_id = {
        let mut id = game_server.next_player_id.lock().await;
        let current_id = *id;
        *id += 1;
        current_id
    };

    //Determine starting position and territory based on player ID
    let (start_x, start_y, territory) = match player_id % 4 {
        1 => (50, 50, (40..=60).step_by(10).flat_map(|x| (40..=60).step_by(10).map(move |y| (x, y))).collect()),          //Top-left
        2 => (750, 50, (740..=760).step_by(10).flat_map(|x| (40..=60).step_by(10).map(move |y| (x, y))).collect()),     //Top-right
        3 => (50, 550, (40..=60).step_by(10).flat_map(|x| (540..=560).step_by(10).map(move |y| (x, y))).collect()),     //Bottom-left
        0 => (750, 550, (740..=760).step_by(10).flat_map(|x| (540..=560).step_by(10).map(move |y| (x, y))).collect()),  //Bottom-right
        _ => (100, 100, (90..=110).step_by(10).flat_map(|x| (90..=110).step_by(10).map(move |y| (x, y))).collect()),    //Fallback
    };


    //Convert start_x and start_y to f32
    let start_x = start_x as f32;
    let start_y = start_y as f32;

    let new_player = Player {
        id: player_id,
        x: start_x,
        y: start_y,
        color,
        territory,
        current_path: Vec::new(),
        ready: false,
    };

    let (init_data, state_data) = {
        let mut players = game_server.players.lock().await;

        //First add the new player to server state
        players.insert(player_id, new_player.clone());

        //Prepare both messages while holding the lock
        let init_msg = GameMessage::PlayerUpdate(new_player.clone());
        let state_msg = GameMessage::GameState(players.values().cloned().collect());

        //Serialize while still holding the lock to ensure consistency
        (
            bincode::serialize(&init_msg)?,
            bincode::serialize(&state_msg)?
        )
    };

    //Now send both messages to the new player
    tcp_stream.write_all(&init_data).await?;
    tcp_stream.flush().await?;
    tcp_stream.write_all(&state_data).await?;
    tcp_stream.flush().await?;

    //Broadcast join notification to all other players
    let _ = game_server.tcp_tx.send(GameMessage::PlayerJoined(new_player.clone()));
    let _ = game_server.tcp_tx.send(GameMessage::PlayerUpdate(new_player));

    let mut rx = game_server.tcp_tx.subscribe();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(msg) => {
                        match &msg {
                            GameMessage::GameOver { .. } => {
                                if let Ok(data) = bincode::serialize(&msg) {
                                    let _ = tcp_stream.write_all(&data).await;
                                    let _ = tcp_stream.flush().await;
                                }
                            },
                            GameMessage::PlayerUpdate(player) if player.id == player_id => continue,
                            _ => {}
                        }

                        if let Ok(data) = bincode::serialize(&msg) {
                            if tcp_stream.write_all(&data).await.is_err() ||
                               tcp_stream.flush().await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }

    {
        let mut players = game_server.players.lock().await;
        players.remove(&player_id);
        let _ = game_server.tcp_tx.send(GameMessage::PlayerLeft { id: player_id });
    }
    println!("Client disconnected: {}", player_id);

    Ok(())
}