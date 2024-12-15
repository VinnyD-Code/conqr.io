use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub color: (u8, u8, u8),
    pub territory: Vec<(i32, i32)>,
    pub current_path: Vec<(i32, i32)>,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GameMessage {
    Position { id: u32, x: f32, y: f32 },
    PlayerUpdate(Player),
    GameState(Vec<Player>),
    PlayerLeft { id: u32 },
}