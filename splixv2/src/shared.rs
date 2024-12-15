use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;
use std::time::Instant;


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub color: (u8, u8, u8),
    pub territory: Vec<(i32, i32)>,
    pub current_path: Vec<(i32, i32)>,
    pub ready: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GameState {
    Lobby,
    CountdownStarted,
    InGame,
    GameOver,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum GamePhase {
    Lobby,
    CountdownStarted,
    InGame,
    GameOver,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GameMessage {
    Position { id: u32, x: f32, y: f32 },
    PlayerUpdate(Player),
    GameState(Vec<Player>),
    PlayerLeft { id: u32 },
    TerritoryUpdate {
        player_id: u32,
        removed_points: Vec<(i32, i32)>,
        new_territory: Vec<(i32, i32)>,
    },
    GameOver {
        winner_id: u32,
        territory_size: usize,
    },
    GameUpdate(String),
    GameTime(f64),
    PlayerJoined(Player),
    PlayerReady { id: u32 },
    GameStarting, //triggers countdown
    StartCountdown, //internal message for countdown start
    TerritoryClaim {
        player_id: u32,
        path: Vec<(i32, i32)>,
        current_territory: Vec<(i32, i32)>,
    },
    TerritoryClaimRejected {
        player_id: u32,
        territory: Vec<(i32, i32)>,
    },
    PlayerDied { id: u32 },
    Reliable(Box<ReliableMessage>),  // Add this new variant
    Ack(u32),  // Add acknowledgment message
    LobbyFull,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReliableMessage {
    pub seq_num: u32,
    pub message: Box<GameMessage>,
    pub attempts: u32,
}
