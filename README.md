# Splix Clone

A multiplayer territory conquest game implemented in Rust using the GGEZ game framework. Players compete to claim territory and eliminate opponents in a fast-paced grid-based environment.

## Game Mechanics

- Each player starts with a small territory in one of the grid corners
- Players can move outside their territory, leaving a trail behind them
- When players return to their territory, the enclosed area becomes their territory
- Players can steal territory from other players by enclosing it
- If a player hits another player's trail, they die and respawn in their territory
- Match duration: 60 seconds
- Winner: Player with the most territory at the end of the match

## Controls

- Movement: Arrow keys or WASD
- Ready/Unready (in lobby): Space bar

# How to Run the Game
## Prerequisites
- Rust installed on your system
- ggez graphics library dependencies (for audio / graphics)
- Local network connection
- Ensure the project is ran on your local machine and not in remote development
## Setup & Running
1. ### **Build the Project**
Use Cargo, Rust's build tool, to build the game:
```bash
cargo build --release
```
2. ### **Start the Server**
Launch the server to handle player connections and manage the game state:

```bash
cargo run --bin server
```

---

3. ### **Start the Client**
Run the client on each player's machine to connect to the server and join the game:

```bash
cargo run --bin client
```

- When prompted, enter the server's IP address (e.g., `192.168.x.x`).
---
4. ### **Start Subsequent Clients**
Repeat Step 3 for each additional player who wants to join the game:

```bash
cargo run --bin client
```

- Each client should enter the same server IP address.
