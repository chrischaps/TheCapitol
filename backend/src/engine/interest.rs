//! Positions by interest (C0.4).
//!
//! Every tick the players are put in a grid of
//! [`INTEREST_CELL`](capitol_protocol::rules::INTEREST_CELL) squares, and
//! each socket is sent the players in its own player's cell and the eight
//! round it. A cell's players are serialised once, the first time a socket
//! asks for them, however many sockets see that cell, so the cost per tick
//! grows with the players, not with players times sockets.
//!
//! Before this, every socket serialised every player ten times a second:
//! with 200 citizens on, that's 200 copies of a 15 kB list a tick.
//!
//! Gathering and crafting go by the same grid (#275): a socket is sent a
//! player's activity events only while that player is in sight
//! ([`Positions::sees`]), and when someone comes into sight mid-gather
//! ([`Positions::come_into_sight`]) it's sent what they're doing
//! ([`Positions::catch_up`]), so their activity shows without waiting for
//! the next one to start. Only a player who changed cell can come into
//! sight of a socket that didn't, so each tick lists who did, and a socket
//! looks further only when it moved itself.

use std::collections::HashMap;
use std::sync::OnceLock;

use capitol_protocol::rules::interest_cell;
use capitol_protocol::ws::ServerMessage;
use serde::{Serialize, Serializer};
use uuid::Uuid;

use crate::models::{ActionState, PlayerPosition, PlayerState, Recipe};

/// A square of the interest grid, by column and row
pub type Cell = (i32, i32);

/// The cell a point is in
pub fn cell_at(x: f64, y: f64) -> Cell {
    interest_cell(x, y)
}

/// Whether a player in cell `b` is in sight of one in cell `a`: the same
/// cell or one of the eight round it
pub fn near(a: Cell, b: Cell) -> bool {
    (a.0 - b.0).abs() <= 1 && (a.1 - b.1).abs() <= 1
}

/// One cell's players, and their JSON once a socket has asked for it
#[derive(Debug, Default)]
struct Bucket {
    players: Vec<PlayerPosition>,
    json: OnceLock<String>,
}

impl Bucket {
    /// The players as the inside of a JSON array, without the brackets
    fn json(&self) -> &str {
        self.json.get_or_init(|| {
            let mut out = String::new();
            for (i, p) in self.players.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(p).expect("a position serialises"));
            }
            out
        })
    }
}

/// Every player's position on one tick, by cell, and what the busy ones are
/// doing
#[derive(Debug, Default)]
pub struct Positions {
    cells: HashMap<Cell, Bucket>,
    at: HashMap<Uuid, Cell>,
    /// Each gathering or crafting player's activity, with the recipe's name
    doing: HashMap<Uuid, (ActionState, String)>,
    /// Which tick's this is, counted by [`Self::after`]
    seq: u64,
    /// The players whose cell isn't what it was the tick before, and the cell
    /// they were in, if they were in the world
    moved: Vec<(Uuid, Option<Cell>)>,
}

impl Positions {
    /// `recipes` names what the crafters are making, for [`Self::catch_up`]
    pub fn of<'a>(players: impl IntoIterator<Item = &'a PlayerState>, recipes: &HashMap<String, Recipe>) -> Self {
        let mut positions = Self::default();
        for p in players {
            let cell = cell_at(p.x, p.y);
            positions.at.insert(p.id, cell);
            match &p.action_state {
                ActionState::Idle => {}
                ActionState::Extracting { .. } => {
                    positions.doing.insert(p.id, (p.action_state.clone(), String::new()));
                }
                ActionState::Crafting { recipe_id, .. } => {
                    let name = recipes.get(recipe_id).map_or_else(|| recipe_id.clone(), |r| r.name.clone());
                    positions.doing.insert(p.id, (p.action_state.clone(), name));
                }
            }
            positions.cells.entry(cell).or_default().players.push(PlayerPosition {
                id: p.id,
                name: p.name.clone(),
                x: p.x,
                y: p.y,
                citizen: p.citizen,
            });
        }
        positions
    }

    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    /// The cell `player_id` is in, if they're in the world
    pub fn cell_of(&self, player_id: Uuid) -> Option<Cell> {
        self.at.get(&player_id).copied()
    }

    /// The buckets in sight of `cell`, in a fixed order
    fn around(&self, cell: Option<Cell>) -> impl Iterator<Item = &Bucket> {
        cell.into_iter()
            .flat_map(|(cx, cy)| (-1..=1).flat_map(move |dy| (-1..=1).map(move |dx| (cx + dx, cy + dy))))
            .filter_map(|c| self.cells.get(&c))
    }

    /// These positions as the tick after `before`: which tick they are, and
    /// who changed cell since
    pub fn after(mut self, before: Option<&Positions>) -> Self {
        self.seq = before.map_or(0, |b| b.seq + 1);
        self.moved = self
            .at
            .iter()
            .filter_map(|(id, cell)| {
                let was = before.and_then(|b| b.cell_of(*id));
                (was != Some(*cell)).then_some((*id, was))
            })
            .collect();
        self
    }

    /// Whether everyone in the world is in sight of someone in `cell`, as
    /// they are when they're all round the spawn: then there's nothing to
    /// filter, and a socket can skip [`Self::sees`]
    pub fn sees_everyone(&self, cell: Option<Cell>) -> bool {
        cell.is_some() && self.around(cell).map(|b| b.players.len()).sum::<usize>() == self.at.len()
    }

    /// Whether `player_id` is in sight of someone in `cell`
    pub fn sees(&self, cell: Option<Cell>, player_id: Uuid) -> bool {
        cell.zip(self.cell_of(player_id)).is_some_and(|(mine, theirs)| near(mine, theirs))
    }

    /// The gathering and crafting players in sight of someone now in `cell`
    /// who weren't in sight of them in `before`, the positions this socket
    /// had last, with the cell it was in then. Only the players who changed
    /// cell need looking at, unless the socket changed cell too, or missed a
    /// tick, or has no `before`.
    pub fn come_into_sight(&self, cell: Option<Cell>, before: Option<(&Positions, Option<Cell>)>) -> Vec<Uuid> {
        let Some(mine) = cell else {
            return Vec::new();
        };
        let busy_here = |id: &Uuid| self.doing.contains_key(id) && self.sees(cell, *id);
        match before {
            Some((before, was)) if before.seq + 1 == self.seq && was == cell => self
                .moved
                .iter()
                .filter(|(id, from)| busy_here(id) && !from.is_some_and(|from| near(mine, from)))
                .map(|(id, _)| *id)
                .collect(),
            _ => self
                .around(cell)
                .flat_map(|b| b.players.iter().map(|p| p.id))
                .filter(|id| busy_here(id) && !before.is_some_and(|(before, was)| before.sees(was, *id)))
                .collect(),
        }
    }

    /// What tells a socket that `player_id`, just come into sight, is part way
    /// through a gather or a craft: its `*_started` and a `*_progress`. Empty
    /// if they're idle.
    pub fn catch_up(&self, player_id: Uuid) -> Vec<ServerMessage> {
        let Some((doing, recipe_name)) = self.doing.get(&player_id) else {
            return Vec::new();
        };
        let ticks = |n: &i32| (*n).max(0) as u32;
        match doing {
            ActionState::Idle => Vec::new(),
            ActionState::Extracting { node_id, progress, duration } => vec![
                ServerMessage::ExtractionStarted { player_id, node_id: *node_id, duration_ticks: ticks(duration) },
                ServerMessage::ExtractionProgress { player_id, progress: ticks(progress), duration: ticks(duration) },
            ],
            ActionState::Crafting { operation_id, recipe_id, progress, duration } => vec![
                ServerMessage::CraftingStarted {
                    player_id,
                    operation_id: *operation_id,
                    recipe_id: recipe_id.clone(),
                    recipe_name: recipe_name.clone(),
                    duration_ticks: ticks(duration),
                },
                ServerMessage::CraftingProgress {
                    player_id,
                    operation_id: *operation_id,
                    progress: ticks(progress),
                    duration: ticks(duration),
                },
            ],
        }
    }

    /// The players in sight of someone in `cell`, none if they're nowhere
    pub fn near(&self, cell: Option<Cell>) -> Vec<PlayerPosition> {
        self.around(cell).flat_map(|b| b.players.iter().cloned()).collect()
    }

    /// The `positions` message for someone in `cell`, as its JSON text: the
    /// same text `ServerMessage::Positions { players: self.near(cell) }`
    /// serialises to, built from each cell's JSON
    pub fn message(&self, cell: Option<Cell>) -> String {
        let mut out = String::from(r#"{"type":"positions","players":["#);
        let mut first = true;
        for bucket in self.around(cell) {
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str(bucket.json());
        }
        out.push_str("]}");
        out
    }
}

/// `GameEvent` is `Serialize`; this is every player, flat, as it was before
/// the grid
impl Serialize for Positions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.cells.values().flat_map(|b| b.players.iter()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ActionState;
    use capitol_protocol::rules::INTEREST_CELL;
    use capitol_protocol::ws::ServerMessage;
    use std::time::Instant;

    fn player(name: &str, x: f64, y: f64) -> PlayerState {
        PlayerState {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            name: name.into(),
            x,
            y,
            dest_x: None,
            dest_y: None,
            speed: 50.0,
            strand_balance: 0,
            current_zone: None,
            action_state: ActionState::Idle,
            citizen: false,
        }
    }

    fn names(players: &[PlayerPosition]) -> Vec<&str> {
        let mut names: Vec<&str> = players.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        names
    }

    /// Someone in the next cell is in sight, someone two cells off isn't,
    /// and you always see yourself.
    #[test]
    fn you_see_your_cell_and_the_eight_round_it() {
        let c = INTEREST_CELL;
        let ada = player("Ada", 6000.0 + 10.0, 6000.0 + 10.0);
        let bram = player("Bram", 6000.0 + c + 10.0, 6000.0 - c + 10.0);
        let cleo = player("Cleo", 6000.0 + 2.0 * c + 10.0, 6000.0 + 10.0);
        let dot = player("Dot", 6000.0 - 1.0, 6000.0 + 10.0);
        let positions = Positions::of([&ada, &bram, &cleo, &dot], &HashMap::new());

        assert_eq!(names(&positions.near(positions.cell_of(ada.id))), ["Ada", "Bram", "Dot"]);
        assert_eq!(names(&positions.near(positions.cell_of(cleo.id))), ["Bram", "Cleo"]);
        assert_eq!(names(&positions.near(positions.cell_of(dot.id))), ["Ada", "Dot"]);
        assert!(positions.near(positions.cell_of(Uuid::new_v4())).is_empty());
        assert!(near(cell_at(ada.x, ada.y), cell_at(dot.x, dot.y)));
        assert!(!near(cell_at(dot.x, dot.y), cell_at(cleo.x, cleo.y)));
    }

    /// The text built from the cells is what serde would have sent, byte for
    /// byte, and parses back to the same players.
    #[test]
    fn the_message_is_the_positions_message_serde_would_send() {
        let c = INTEREST_CELL;
        let mut wren = player("Wren", 6000.0 + c, 6000.0);
        wren.citizen = true;
        let players = [player("Ada", 6000.0, 6000.0), wren, player("Bram", 6000.0 - c, 6000.0 + c)];
        let positions = Positions::of(&players, &HashMap::new());
        for p in &players {
            let cell = positions.cell_of(p.id);
            let expected = serde_json::to_string(&ServerMessage::Positions { players: positions.near(cell) }).unwrap();
            assert_eq!(positions.message(cell), expected);
        }
        let nowhere = serde_json::to_string(&ServerMessage::Positions { players: vec![] }).unwrap();
        assert_eq!(positions.message(None), nowhere);
    }

    /// Someone come into sight mid-gather or mid-craft is told as a start and
    /// the progress so far; someone idle, or not in the world, is told nothing.
    #[test]
    fn catch_up_is_a_start_and_the_progress_so_far() {
        let node_id = Uuid::new_v4();
        let operation_id = Uuid::new_v4();
        let mut ada = player("Ada", 6000.0, 6000.0);
        ada.action_state = ActionState::Extracting { node_id, progress: 12, duration: 30 };
        let mut bram = player("Bram", 6010.0, 6000.0);
        bram.action_state =
            ActionState::Crafting { operation_id, recipe_id: "craft_rope".into(), progress: 40, duration: 50 };
        let cleo = player("Cleo", 6020.0, 6000.0);
        let positions = Positions::of([&ada, &bram, &cleo], &HashMap::new());

        let tags = |msgs: Vec<ServerMessage>| msgs.iter().map(|m| serde_json::to_value(m).unwrap()).collect::<Vec<_>>();
        let ada_msgs = tags(positions.catch_up(ada.id));
        assert_eq!(ada_msgs[0]["type"], "extraction_started");
        assert_eq!(ada_msgs[0]["node_id"], node_id.to_string());
        assert_eq!(ada_msgs[0]["duration_ticks"], 30);
        assert_eq!(ada_msgs[1]["type"], "extraction_progress");
        assert_eq!(ada_msgs[1]["progress"], 12);

        let bram_msgs = tags(positions.catch_up(bram.id));
        assert_eq!(bram_msgs[0]["type"], "crafting_started");
        assert_eq!(bram_msgs[0]["operation_id"], operation_id.to_string());
        // No recipe on record: its id stands in for its name
        assert_eq!(bram_msgs[0]["recipe_name"], "craft_rope");
        assert_eq!(bram_msgs[1]["type"], "crafting_progress");
        assert_eq!(bram_msgs[1]["progress"], 40);
        assert_eq!(bram_msgs[1]["duration"], 50);

        assert!(positions.catch_up(cleo.id).is_empty());
        assert!(positions.catch_up(Uuid::new_v4()).is_empty());
    }

    /// Who's newly in sight and busy: a gatherer who walks into the next cell
    /// is, an idle walker isn't, nor is a gatherer who was in sight already.
    /// When you walk yourself, or miss a tick, or have nothing to go on, it
    /// looks round the cells, not just at who moved.
    #[test]
    fn come_into_sight_is_the_busy_who_werent_in_sight() {
        let c = INTEREST_CELL;
        let (x, y) = (6000.0 + 10.0, 6000.0 + 10.0);
        let gathering = ActionState::Extracting { node_id: Uuid::new_v4(), progress: 3, duration: 30 };
        let ada = player("Ada", x, y);
        let mut bram = PlayerState { action_state: gathering.clone(), ..player("Bram", x + 2.0 * c, y) };
        let cleo = PlayerState { action_state: gathering.clone(), ..player("Cleo", x + c, y) };
        let mut dot = player("Dot", x, y - 2.0 * c);
        let far = PlayerState { action_state: gathering, ..player("Far", x - 3.0 * c, y) };
        let before = Positions::of([&ada, &bram, &cleo, &dot, &far], &HashMap::new()).after(None);
        bram.x -= c;
        dot.y += c;
        let now = Positions::of([&ada, &bram, &cleo, &dot, &far], &HashMap::new()).after(Some(&before));
        let mine = now.cell_of(ada.id);
        let sorted = |mut ids: Vec<Uuid>| {
            ids.sort();
            ids
        };

        assert_eq!(now.come_into_sight(mine, Some((&before, mine))), [bram.id]);
        assert_eq!(sorted(now.come_into_sight(mine, None)), sorted(vec![bram.id, cleo.id]));
        assert!(now.come_into_sight(None, Some((&before, mine))).is_empty());
        // A tick missed: the movers aren't enough to go on, so it looks round
        let later = Positions::of([&ada, &bram, &cleo, &dot, &far], &HashMap::new()).after(Some(&now));
        assert_eq!(later.come_into_sight(mine, Some((&before, mine))), [bram.id]);
        assert!(later.come_into_sight(mine, Some((&now, mine))).is_empty());
        // Ada walks two cells west: Far comes into sight, and Cleo goes out of it
        let ada_west = PlayerState { x: x - 2.0 * c, ..ada.clone() };
        let moved = Positions::of([&ada_west, &bram, &cleo, &dot, &far], &HashMap::new()).after(Some(&now));
        assert_eq!(moved.come_into_sight(moved.cell_of(ada.id), Some((&now, mine))), [far.id]);
        assert!(moved.sees(moved.cell_of(ada.id), far.id));
        assert!(!moved.sees(moved.cell_of(ada.id), cleo.id));
        assert!(!moved.sees(None, far.id));
    }

    /// #275's numbers: what the gathering and crafting events cost the
    /// sockets with `n` players on, each with a socket, each walking and each
    /// sending four events a second (C0.3 counted up to 400 a second with 200
    /// citizens at work). Before, every socket serialised every event. After,
    /// each socket checks each event against the last `positions` and
    /// serialises it only if the actor is in sight, and ten times a second
    /// sends the catch-up for anyone busy who came into sight. Run with
    /// `cargo test --release -p capitol-backend interest::tests::activity_load -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn activity_load() {
        use capitol_protocol::fmath::StableMath;
        use capitol_protocol::rules::{CENTER_X, CENTER_Y, RURAL_OUTER_RADIUS, SPAWN_SCATTER, SPAWN_X, SPAWN_Y};
        use std::sync::Arc;
        const TICKS: u32 = 50;
        const EVENTS_PER_PLAYER_SECOND: u32 = 4;
        let mut seed = 0x275u64;
        let mut next = move || {
            seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        println!("| players | layout | before: sockets µs/s | before: kB/socket/s | after: sockets µs/s | after: kB/socket/s |");
        println!("|---|---|---|---|---|---|");
        for n in [50usize, 200] {
            for layout in ["at the spawn", "across the city"] {
                let mut players: Vec<PlayerState> = (0..n)
                    .map(|i| {
                        let (x, y) = if layout == "at the spawn" {
                            (SPAWN_X + (next() - 0.5) * 8.0 * SPAWN_SCATTER, SPAWN_Y + (next() - 0.5) * 8.0 * SPAWN_SCATTER)
                        } else {
                            let (r, a) = (RURAL_OUTER_RADIUS * next().sqrt(), std::f64::consts::TAU * next());
                            (CENTER_X + r * a.stable_cos(), CENTER_Y + r * a.stable_sin())
                        };
                        let mut p = player(&format!("Citizen{i:03} Fletcher"), x, y);
                        if i % 2 == 0 {
                            p.action_state = ActionState::Extracting { node_id: Uuid::new_v4(), progress: 12, duration: 30 };
                        }
                        p
                    })
                    .collect();
                // Half gather where they stand (a gather stops you), and half
                // walk a straight line at walking pace, 5 a tick
                let headings: Vec<f64> = players.iter().map(|_| std::f64::consts::TAU * next()).collect();
                let events: Vec<ServerMessage> = players
                    .iter()
                    .filter(|p| p.action_state != ActionState::Idle)
                    .map(|p| ServerMessage::ExtractionProgress { player_id: p.id, progress: 12, duration: 30 })
                    .collect();
                let mut seen: Vec<(Option<Arc<Positions>>, Option<Cell>)> = vec![(None, None); n];
                let mut last: Option<Arc<Positions>> = None;
                let (mut before_ns, mut before_bytes) = (0u128, 0usize);
                let (mut after_ns, mut after_bytes) = (0u128, 0usize);
                for tick in 0..TICKS {
                    for (p, a) in players.iter_mut().zip(&headings).filter(|(p, _)| p.action_state == ActionState::Idle) {
                        p.x += 5.0 * a.stable_cos();
                        p.y += 5.0 * a.stable_sin();
                    }
                    let positions = Arc::new(Positions::of(&players, &HashMap::new()).after(last.as_deref()));
                    last = Some(Arc::clone(&positions));
                    // Every player's event, on four ticks in ten
                    let sends = tick % 10 < EVENTS_PER_PLAYER_SECOND;

                    let t = Instant::now();
                    if sends {
                        for _socket in &players {
                            for event in &events {
                                before_bytes += serde_json::to_string(event).unwrap().len();
                            }
                        }
                    }
                    before_ns += t.elapsed().as_nanos();

                    let t = Instant::now();
                    for (socket, (before, was_cell)) in players.iter().zip(seen.iter_mut()) {
                        let cell = positions.cell_of(socket.id);
                        let arrived = positions.come_into_sight(cell, before.as_deref().map(|b| (b, *was_cell)));
                        for id in arrived.into_iter().filter(|id| *id != socket.id) {
                            for msg in positions.catch_up(id) {
                                after_bytes += serde_json::to_string(&msg).unwrap().len();
                            }
                        }
                        *before = Some(Arc::clone(&positions));
                        *was_cell = cell;
                        let everyone = positions.sees_everyone(cell);
                        if sends {
                            for event in &events {
                                let ServerMessage::ExtractionProgress { player_id, .. } = event else { unreachable!() };
                                if everyone || *player_id == socket.id || positions.sees(cell, *player_id) {
                                    after_bytes += serde_json::to_string(event).unwrap().len();
                                }
                            }
                        }
                    }
                    after_ns += t.elapsed().as_nanos();
                }
                let seconds = TICKS as f64 / 10.0;
                let per_second = |ns: u128| ns as f64 / seconds / 1000.0;
                let per_socket_second = |bytes: usize| bytes as f64 / seconds / n as f64 / 1000.0;
                println!(
                    "| {n} | {layout} | {:.0} | {:.1} | {:.0} | {:.1} |",
                    per_second(before_ns),
                    per_socket_second(before_bytes),
                    per_second(after_ns),
                    per_socket_second(after_bytes),
                );
            }
        }
    }

    /// C0.4's numbers: what positions cost the server with `n` players on,
    /// each with a socket, before (every socket serialises everyone) and
    /// after (each cell serialised once, each socket joins nine). Run with
    /// `cargo test --release -p capitol-backend interest::tests::positions_load -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn positions_load() {
        use capitol_protocol::fmath::StableMath;
        use capitol_protocol::rules::{CENTER_X, CENTER_Y, RURAL_OUTER_RADIUS, SPAWN_SCATTER, SPAWN_X, SPAWN_Y};
        const TICKS: u32 = 50;
        // A SplitMix64, so the layouts are the same every run
        let mut seed = 0x256u64;
        let mut next = move || {
            seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        println!("| players | layout | before: tick µs | before: sockets µs | before: B/socket/s | after: tick µs | after: sockets µs | after: B/socket/s |");
        println!("|---|---|---|---|---|---|---|---|");
        for n in [50usize, 200] {
            for layout in ["at the spawn", "across the city"] {
                let players: Vec<PlayerState> = (0..n)
                    .map(|i| {
                        let (x, y) = if layout == "at the spawn" {
                            (SPAWN_X + (next() - 0.5) * 8.0 * SPAWN_SCATTER, SPAWN_Y + (next() - 0.5) * 8.0 * SPAWN_SCATTER)
                        } else {
                            let (r, a) = (RURAL_OUTER_RADIUS * next().sqrt(), std::f64::consts::TAU * next());
                            (CENTER_X + r * a.stable_cos(), CENTER_Y + r * a.stable_sin())
                        };
                        let mut p = player(&format!("Citizen{i:03} Fletcher"), x, y);
                        p.citizen = true;
                        p
                    })
                    .collect();
                let (mut before_tick, mut before_sockets, mut before_bytes) = (0u128, 0u128, 0usize);
                let (mut after_tick, mut after_sockets, mut after_bytes) = (0u128, 0u128, 0usize);
                for _ in 0..TICKS {
                    // Before: the tick lists everyone; each socket serialises the list
                    let t = Instant::now();
                    let all: Vec<PlayerPosition> = players
                        .iter()
                        .map(|p| PlayerPosition { id: p.id, name: p.name.clone(), x: p.x, y: p.y, citizen: p.citizen })
                        .collect();
                    before_tick += t.elapsed().as_nanos();
                    let t = Instant::now();
                    for _ in &players {
                        before_bytes += serde_json::to_string(&ServerMessage::Positions { players: all.clone() }).unwrap().len();
                    }
                    before_sockets += t.elapsed().as_nanos();

                    // After: the tick grids them; each socket joins its nine cells
                    let t = Instant::now();
                    let positions = Positions::of(&players, &HashMap::new());
                    after_tick += t.elapsed().as_nanos();
                    let t = Instant::now();
                    for p in &players {
                        after_bytes += positions.message(positions.cell_of(p.id)).len();
                    }
                    after_sockets += t.elapsed().as_nanos();
                }
                let per_tick = |ns: u128| ns as f64 / TICKS as f64 / 1000.0;
                let per_socket_second = |bytes: usize| bytes as f64 / TICKS as f64 / n as f64 * 10.0;
                println!(
                    "| {n} | {layout} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |",
                    per_tick(before_tick),
                    per_tick(before_sockets),
                    per_socket_second(before_bytes),
                    per_tick(after_tick),
                    per_tick(after_sockets),
                    per_socket_second(after_bytes),
                );
            }
        }
    }
}
