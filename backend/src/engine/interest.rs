//! Positions by interest (C0.4).
//!
//! Every tick the players are put in a grid of [`INTEREST_CELL`] squares, and
//! each socket is sent the players in its own player's cell and the eight
//! round it. A cell's players are serialised once, the first time a socket
//! asks for them, however many sockets see that cell, so the cost per tick
//! grows with the players, not with players times sockets.
//!
//! Before this, every socket serialised every player ten times a second:
//! with 200 citizens on, that's 200 copies of a 15 kB list a tick.

use std::collections::HashMap;
use std::sync::OnceLock;

use capitol_protocol::rules::INTEREST_CELL;
use serde::{Serialize, Serializer};
use uuid::Uuid;

use crate::models::{PlayerPosition, PlayerState};

/// A square of the interest grid, by column and row
pub type Cell = (i32, i32);

/// The cell a point is in
pub fn cell_at(x: f64, y: f64) -> Cell {
    ((x / INTEREST_CELL).floor() as i32, (y / INTEREST_CELL).floor() as i32)
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

/// Every player's position on one tick, by cell
#[derive(Debug, Default)]
pub struct Positions {
    cells: HashMap<Cell, Bucket>,
    at: HashMap<Uuid, Cell>,
}

impl Positions {
    pub fn of<'a>(players: impl IntoIterator<Item = &'a PlayerState>) -> Self {
        let mut positions = Self::default();
        for p in players {
            let cell = cell_at(p.x, p.y);
            positions.at.insert(p.id, cell);
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
        let positions = Positions::of([&ada, &bram, &cleo, &dot]);

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
        let positions = Positions::of(&players);
        for p in &players {
            let cell = positions.cell_of(p.id);
            let expected = serde_json::to_string(&ServerMessage::Positions { players: positions.near(cell) }).unwrap();
            assert_eq!(positions.message(cell), expected);
        }
        let nowhere = serde_json::to_string(&ServerMessage::Positions { players: vec![] }).unwrap();
        assert_eq!(positions.message(None), nowhere);
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
                    let positions = Positions::of(&players);
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
