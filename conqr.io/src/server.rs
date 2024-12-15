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
}

impl GameServer {
    fn new(tcp_tx: broadcast::Sender<GameMessage>) -> Self {
        Self {
            players: Arc::new(Mutex::new(HashMap::new())),
            tcp_tx: tcp_tx,
            next_player_id: Arc::new(Mutex::new(1)),
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
}

#[tokio::main]
async fn main() {
    let tcp_listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();
    let udp_socket = Arc::new(UdpSocket::bind("127.0.0.1:8081").await.unwrap());
    let (tcp_tx, _) = broadcast::channel(100);

    let game_server = Arc::new(GameServer::new(tcp_tx.clone()));

    println!("Server listening on TCP: 127.0.0.1:8080, UDP: 127.0.0.1:8081");

    // Create periodic state broadcast task
    let game_server_clone = game_server.clone();
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_millis(16));
        loop {
            interval.tick().await;
            let players = game_server_clone.players.lock().await;
            let state = GameMessage::GameState(players.values().cloned().collect());
            let _ = game_server_clone.tcp_tx.send(state);
        }
    });

    // Main UDP message handling task
    let udp_socket_clone = udp_socket.clone();
    let game_server_clone = game_server.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        loop {
            if let Ok((size, _)) = udp_socket_clone.recv_from(&mut buf).await {
                if let Ok(msg) = bincode::deserialize::<GameMessage>(&buf[..size]) {
                    match msg {
                        GameMessage::PlayerUpdate(updated_player) => {
                            let mut players = game_server_clone.players.lock().await;
                            if let Some(player) = players.get_mut(&updated_player.id) {
                                *player = updated_player.clone();
                                let _ = game_server_clone.tcp_tx.send(GameMessage::PlayerUpdate(player.clone()));
                            }
                        }
                        GameMessage::Position { id, x, y } => {
                            let mut players = game_server_clone.players.lock().await;
                            if let Some(player) = players.get_mut(&id) {
                                player.x = x;
                                player.y = y;
                                let update_msg = GameMessage::PlayerUpdate(player.clone());
                                let _ = game_server_clone.tcp_tx.send(update_msg);
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

        // Generate color before spawning the task
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
    let player_id = {
        let mut id = game_server.next_player_id.lock().await;
        let current_id = *id;
        *id += 1;
        current_id
    };

    // Determine starting position and territory based on player ID
    let (start_x, start_y, territory) = match player_id % 4 {
        1 => (50, 50, (40..=60).step_by(10).flat_map(|x| (40..=60).step_by(10).map(move |y| (x, y))).collect()),          // Top-left
        2 => (750, 50, (740..=760).step_by(10).flat_map(|x| (40..=60).step_by(10).map(move |y| (x, y))).collect()),     // Top-right
        3 => (50, 550, (40..=60).step_by(10).flat_map(|x| (540..=560).step_by(10).map(move |y| (x, y))).collect()),     // Bottom-left
        0 => (750, 550, (740..=760).step_by(10).flat_map(|x| (540..=560).step_by(10).map(move |y| (x, y))).collect()),  // Bottom-right
        _ => (100, 100, (90..=110).step_by(10).flat_map(|x| (90..=110).step_by(10).map(move |y| (x, y))).collect()),    // Fallback
    };


    // Convert start_x and start_y to f32
    let start_x = start_x as f32;
    let start_y = start_y as f32;

    let new_player = Player {
        id: player_id,
        x: start_x,
        y: start_y,
        color,
        territory,
        current_path: Vec::new(),
    };

    {
        let player_init = GameMessage::PlayerUpdate(new_player.clone());
        let data = bincode::serialize(&player_init)?;
        tcp_stream.write_all(&data).await?;
        tcp_stream.flush().await?;

        tokio::time::sleep(Duration::from_millis(10)).await;

        let mut players = game_server.players.lock().await;
        players.insert(player_id, new_player.clone());

        let state = GameMessage::GameState(players.values().cloned().collect());
        let data = bincode::serialize(&state)?;
        tcp_stream.write_all(&data).await?;
        tcp_stream.flush().await?;

        let _ = game_server.tcp_tx.send(GameMessage::PlayerUpdate(new_player));
    }

    let mut rx = game_server.tcp_tx.subscribe();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(msg) => {
                        match &msg {
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