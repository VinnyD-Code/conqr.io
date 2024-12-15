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

## Features

- Real-time multiplayer (up to 4 players)
- Client-server architecture using TCP and UDP
- Player color assignments
- Territory claiming and stealing mechanics
- Lobby system with ready states
- Game countdown timer
- Sound effects for various game events
- Score tracking
- Interpolated player movement
- Collision detection
- Territory flood-fill algorithm

## Technical Implementation

### Network Architecture

The game uses a hybrid networking approach:
- TCP: Reliable communication for game state, player updates, and critical events
- UDP: Fast communication for player positions and immediate updates

### Server Components

- Player management and state tracking
- Game loop and timing
- Territory validation
- Collision detection
- Broadcast messaging system

### Client Components

- Game state management
- Input handling
- Rendering system
- Sound system
- Network message processing

## Setup and Running

### Prerequisites

```bash
# Required Rust version
rustc 1.75.0 or higher

# Required external dependencies
GGEZ game framework
tokio async runtime
