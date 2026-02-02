use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::commands::Command;
use super::events::GameEvent;
use super::movement::process_movement;
use crate::models::{PlayerPosition, PlayerState};
use crate::state::AppState;

const TICK_RATE_MS: u64 = 100;

pub struct GameState {
    pub players: HashMap<Uuid, PlayerState>,
    pub command_queue: Vec<Command>,
    pub event_sender: broadcast::Sender<GameEvent>,
    tick_count: u64,
}

impl GameState {
    pub fn new() -> Self {
        let (event_sender, _) = broadcast::channel(1024);
        Self {
            players: HashMap::new(),
            command_queue: Vec::new(),
            event_sender,
            tick_count: 0,
        }
    }

    pub fn queue_command(&mut self, command: Command) {
        self.command_queue.push(command);
    }

    pub fn add_player(&mut self, player: PlayerState) {
        let position = PlayerPosition {
            id: player.id,
            name: player.name.clone(),
            x: player.x,
            y: player.y,
        };
        self.players.insert(player.id, player);
        let _ = self.event_sender.send(GameEvent::PlayerJoined(position));
    }

    pub fn remove_player(&mut self, player_id: Uuid) {
        self.players.remove(&player_id);
        let _ = self.event_sender.send(GameEvent::PlayerLeft { player_id });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GameEvent> {
        self.event_sender.subscribe()
    }

    fn process_commands(&mut self) {
        let commands: Vec<Command> = self.command_queue.drain(..).collect();

        for command in commands {
            match command {
                Command::Move { player_id, dest_x, dest_y } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.dest_x = Some(dest_x);
                        player.dest_y = Some(dest_y);
                    }
                }
                Command::StopMoving { player_id } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.dest_x = None;
                        player.dest_y = None;
                    }
                }
            }
        }
    }

    fn process_movement(&mut self) {
        for player in self.players.values_mut() {
            process_movement(player);
        }
    }

    fn broadcast_positions(&self) {
        let positions: Vec<PlayerPosition> = self.players.values()
            .map(|p| PlayerPosition {
                id: p.id,
                name: p.name.clone(),
                x: p.x,
                y: p.y,
            })
            .collect();

        if !positions.is_empty() {
            let _ = self.event_sender.send(GameEvent::PlayerPositions(positions));
        }
    }

    fn tick(&mut self) {
        self.tick_count += 1;
        self.process_commands();
        self.process_movement();
        self.broadcast_positions();
    }
}

pub async fn run_tick_loop(game_state: Arc<RwLock<GameState>>, app_state: AppState) {
    tracing::info!("Starting tick engine at {}ms interval", TICK_RATE_MS);

    let mut interval = tokio::time::interval(Duration::from_millis(TICK_RATE_MS));
    let mut last_log = Instant::now();
    let mut tick_count = 0u64;

    loop {
        interval.tick().await;

        {
            let mut game = game_state.write().await;
            game.tick();
        }

        tick_count += 1;

        // Log tick rate every 10 seconds
        if last_log.elapsed() >= Duration::from_secs(10) {
            let tps = tick_count as f64 / last_log.elapsed().as_secs_f64();
            tracing::debug!("Tick rate: {:.1} ticks/sec", tps);
            last_log = Instant::now();
            tick_count = 0;
        }

        // Periodically persist player positions
        if tick_count % 100 == 0 {
            persist_positions(&game_state, &app_state).await;
        }
    }
}

async fn persist_positions(game_state: &Arc<RwLock<GameState>>, app_state: &AppState) {
    let game = game_state.read().await;

    for player in game.players.values() {
        let result = sqlx::query(
            "UPDATE players SET position_x = $1, position_y = $2, destination_x = $3, destination_y = $4 WHERE id = $5"
        )
        .bind(player.x)
        .bind(player.y)
        .bind(player.dest_x)
        .bind(player.dest_y)
        .bind(player.id)
        .execute(&app_state.db)
        .await;

        if let Err(e) = result {
            tracing::error!("Failed to persist player position: {:?}", e);
        }
    }
}
