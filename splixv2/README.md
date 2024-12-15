[![Review Assignment Due Date](https://classroom.github.com/assets/deadline-readme-button-22041afd0340ce965d47ae6ef1cefeee28c7c493a6346c4f15d667ab976d596c.svg)](https://classroom.github.com/a/VHTjc6Nm)
Goal: Apply the knowledge you've learned in new ways.

# Project description
This is an open-ended project. Students can extend their BearTV project or do something new from the ground up. Project ideas must be approved by Dr. Freeman.

You must give a **formal presentation** of your project in place of a final exam. Each group will have ~15 minutes to present their work. Each member of the group must speak. You should have slides. Your presentation must include a demo of your project, although it may invlude a pre-recorded screen capture. In your presentation, you should introduce the problem that you addressed, how you addressed it, challenges you faced, what you learned, and next steps (if you were to continue developing it).

You may use AI LLM tools to assist with the development of your project, including code assistant tools like GitHub Copilot.

## Milestones
- You must meet with Dr. Freeman within the first week to get your project idea approved
- You must meet with Dr. Freeman within the first 3 weeks to give a status update and discuss roadblocks
- See the course schedule spreadhseet for specific dates

## Project Ideas
- Simulate UDP packet loss and packet corruption in BearTV in a non-deterministic way (i.e., don't just drop every Nth packet). Then, extend the application protocol to be able to detect and handle this packet loss.
- Extend the BearTV protocol to support streaming images (or video!) alongside the CC data, and visually display them on the client. This should be done in such a way that it is safely deliver*able* over *any* implementation of IPv4. The images don't have to be relevant to the caption data--you can get them randomly on the server from some image source.
- Create a 4-player CLI-based game using the transport protocol of your choice. You'll design the application protocol as you see fit.
- Do something hands on with a video streaming protocol such as MoQ, DASH, or HLS.
- Implement an HTTP protocol and have a simple website demo

--> These are just examples. I hope that you'll come up with a better idea to suit your own interests!

## Libraries

Depending on the project, there may be helpful libraries you find to help you out. However, there may also be libraries that do all the interesting work for you. Depending on the project, you'll need to determine what should be fair game. For example, if your project is to implement HTTP, then you shouldn't leverage an HTTP library that does it for you.

If you're unsure if a library is okay to use, just ask me.

## Languages

The core of your project should, ideally, be written in Rust. Depending on the project idea, however, I'm open to allowing the use of other languages if there's a good reason for it. For me to approve such a request, the use of a different language should enable greater learning opportunities for your group.

# Submission

## Questions
- What is your project?
- What novel work did you do?
- What did you learn?
- What was challenging?
- What AI tools did you use, and what did you use them for? What were their benefits and drawbacks?
- What would you do differently next time?

## What to submit
- Push your working code to the main branch of your team's GitHub Repository before the deadline
- Edit the README to answer the above questions
- On Teams, *each* member of the group must individually upload answers to these questions:
	- What did you (as an individual) contribute to this project?
	- What did the other members of your team contribute?
	- Do you have any concerns about your own performance or that of your team members? Any comments will remain confidential, and Dr. Freeman will try to address them in a way that preserves anonymity.
	- What feedback do you have about this course?

## Grading

Grading wilil be based on...
- The technical merit of the group's project
- The contribution of each individual group member
- Evidence of consistent work, as revealed during milestone meetings
- The quality of the final presentation

# Answers to Questions
1. We leveraged Rust to create a multiplayer territory conquest game heavily inspired by the popular game Splix.io. Our 
game lets players move around a grid, claiming territory by connecting their path back to their own territory. We 
added a lobby system that allows players to 'ready-up', territory claiming mechanics, sound effects, score tracking, and 
player death / respawn mechanics as well
2. We built a hybrid networking architecture by leveraging both TCP and UDP. TCP was primarily used for reliable game 
state updates, such as client-server connection where the client has to reliably establish a connection to the server.
It ensures that they are assigned a correct player ID. We also created a complex and efficient flood fill algorithm that
utilized spatial partition in order to reduce the risks of packet drops. Client-side prediction and interpolation was also
used to reduce packet drops by predicting movements made. Also, we synchronized player state across multiple clients to
ensure all data was being updated correctly throughout the course of the game.
3. We learned a lot about what it takes to create a full fledged game that not only was single player, but allowed for 
multiplier connectivity. We learned about the choice of choosing TCP or UDP and how to leverage both in game development. 
We also had to learn about potential tradeoffs to mitigate the effects of packet loss. One of the biggest things we 
learned as well was state synchronization, and how to manage game state across multiple clients simultaneously.   
4. As mentioned earlier, we had to learn a lot about state synchronization, and that was definitely one of, if not the 
hardest challenge we came across. We had to ensure consistency across all clients while maintaining real-time responsiveness
as well. We also had to find a work-around for LAN connectivity to Baylor's Wi-Fi since it is automatically segmented for
each connection, assigning a new IPv4 address for each machine that connects. Lastly, for each group member this was 
their first time developing a game that not only worked in single player, but allowed for multiplayer connectivity as well.
So developing the mechanics for the game while also worrying about networking was a big challenge.
5. In terms of AI, in the beginning we used Claude and ChatGPT to help us with some boilerplate code and ideas. However,
we quickly realized that AI tools quickly become irrelevant as the scale of the project became larger. AI could not comprehend
all of the code that we had implemented, and we ended up only utilizing it for debugging from then on.
6. Next time, we will start off by using a combination of both TCP and UDP for the game. This is because for the first week,
we only used TCP for the entire state of the game, which introduced severe lag and synchronization issues between clients.
We would also add more reliable strategies to reduce the effects of packet loss over UDP.

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

