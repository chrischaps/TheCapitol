//! `--building-sample` (V3.4): offline, the home lots nearest where the
//! camera first looks, claimed and worked in, to see their buildings open
//! as the camera comes close (`world::houses`).
//!
//! - **The lots:** the [`LOTS`] nearest with a building of the rules' own,
//!   each claimed. The nearest is yours, so it opens from 25 m, and the
//!   rest from 10.
//! - **Floors:** an urban lot has none to three, by its id, and the second
//!   urban lot has one under way in scaffolding, which finishes [`FINISH`]
//!   seconds in and rises.
//! - **Stations:** four indoors on each lot, in a square inside the walls,
//!   a kiln or forge among them working, so there's a chimney over it; and
//!   on a lot with a yard, two out in the weather at falling conditions,
//!   the last needing repair.
//! - **Yours, for `--place-sample` (V3.5):** the nearest lot is in the
//!   store as a plot of yours, with the floors it was dealt, 4 indoors a
//!   storey and a yard as its zone's median holds (none in the city, 4 in
//!   the suburb, 10 on a farm), and you stand in its building, so a ghost
//!   placed there judges its room as the server would.

use crate::world::plots::Lot;
use capitol_core::net::{ConnectionState, RestResponse};
use capitol_core::protocol::rest::{
    AutomationMode, AutomationState, Bounds, ListPlotsResponse, NearbyStationsResponse, PlotInfo, RecipesResponse,
    StationAutomation, StationInfo,
};
use capitol_core::protocol::ws::{PlayerPosition, ServerMessage};
use capitol_core::protocol::rules::{building, Zone};
use capitol_core::state::GameState;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// How many lots it claims.
pub const LOTS: usize = 8;
/// When the floor under way finishes, seconds after the sample begins.
pub const FINISH: f32 = 4.0;

/// The stations indoors, by turns, from each lot's own start.
const KINDS: [&str; 6] = ["kiln", "workbench", "forge", "anvil", "sawbench", "storage_chest"];
/// The conditions of the stations out in the yards, by turns.
const CONDITIONS: [i32; 4] = [85, 50, 22, 8];

type P = (f64, f64);

pub struct Buildings {
    /// The lots claimed, and the one that's yours.
    pub claimed: Vec<String>,
    pub mine: HashSet<String>,
    /// Floors built, and the next under way, by lot.
    pub floors: Vec<(String, u8, bool)>,
    /// Their stations.
    pub state: GameState,
    time: f32,
}

impl Buildings {
    /// The lots nearest `focus` (server units), claimed and worked in.
    pub fn new(focus: P, lots: &HashMap<String, Lot>) -> Self {
        let me = Uuid::from_u128(0xca9);
        let someone = Uuid::from_u128(0xbee);
        let mut near: Vec<(f64, &String, [P; 4], [P; 4])> = lots
            .iter()
            .filter_map(|(id, lot)| {
                let corners = lot.corners.map(|[x, y]| (x, y));
                let f = building::footprint(&corners, lot.zone)?;
                let c = ((f[0].0 + f[2].0) / 2.0, (f[0].1 + f[2].1) / 2.0);
                Some(((c.0 - focus.0).hypot(c.1 - focus.1), id, corners, f))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
        near.truncate(LOTS);
        let mut state = GameState::new();
        state.apply_connection(&ConnectionState::Connected { player_id: me });
        let recipes = state.catalog().recipes().to_vec();
        state.apply_rest(&RestResponse::Recipes(RecipesResponse { recipes }));
        let mut stations = Vec::new();
        let mut floors = Vec::new();
        let mut urban = 0;
        let mut n = 0u128;
        let mut yard_k = 0;
        for (k, (_, id, corners, f)) in near.iter().enumerate() {
            let owner = if k == 0 { me } else { someone };
            if lots[*id].zone == Zone::Urban {
                let built = (crate::world::houses::Seed::of(id).unit() * 4.0) as u8;
                floors.push(((*id).clone(), built, urban == 1));
                urban += 1;
            }
            // Four indoors, in a square inside the walls.
            let at = |s: f64, t: f64| {
                (
                    f[0].0 + (f[1].0 - f[0].0) * s + (f[3].0 - f[0].0) * t,
                    f[0].1 + (f[1].1 - f[0].1) * s + (f[3].1 - f[0].1) * t,
                )
            };
            for (j, (s, t)) in [(0.27, 0.3), (0.73, 0.3), (0.27, 0.7), (0.73, 0.7)].into_iter().enumerate() {
                n += 1;
                let kind = KINDS[(k + j) % KINDS.len()];
                stations.push(station(&state, n, kind, at(s, t), owner, true, 100));
            }
            // Two in the yard, if there's room behind the building.
            let back = mid(f[2], f[3]);
            let lot_back = mid(corners[2], corners[3]);
            let room = (lot_back.0 - back.0).hypot(lot_back.1 - back.1);
            if room > 45.0 {
                let y = mid(back, lot_back);
                let along = unit((f[1].0 - f[0].0, f[1].1 - f[0].1));
                for side in [-1.0, 1.0] {
                    n += 1;
                    let p = (y.0 + along.0 * 28.0 * side, y.1 + along.1 * 28.0 * side);
                    let kind = ["workbench", "sawbench"][yard_k % 2];
                    let condition = CONDITIONS[yard_k % CONDITIONS.len()];
                    yard_k += 1;
                    stations.push(station(&state, n, kind, p, owner, false, condition));
                }
            }
        }
        state.apply_rest(&RestResponse::NearbyStations(NearbyStationsResponse { stations }));
        // The nearest is yours: in the store as your plot, and you in it
        if let Some((_, id, corners, f)) = near.first() {
            let zone = lots[*id].zone;
            let built = floors.iter().find(|(l, _, _)| l == *id).map_or(0, |(_, k, _)| *k as i32);
            let yard = match zone {
                Zone::Urban => 0,
                Zone::Suburban => 4,
                _ => 10,
            };
            let xs = corners.map(|c| c.0);
            let ys = corners.map(|c| c.1);
            let min = |v: [f64; 4]| v.into_iter().fold(f64::MAX, f64::min);
            let max = |v: [f64; 4]| v.into_iter().fold(f64::MIN, f64::max);
            let centre = mid(mid(f[0], f[1]), mid(f[2], f[3]));
            let plot = PlotInfo {
                guild: None,
                id: Uuid::from_u128(0x5a_1000),
                zone_id: zone.as_str().into(),
                world_x: centre.0,
                world_y: centre.1,
                bounds: Bounds::new(min(xs), min(ys), max(xs), max(ys)),
                size_category: "medium".into(),
                plot_type: "standard".into(),
                owner_id: Some(me),
                owner_name: Some("You".into()),
                claimed_at: None,
                assessed_value: 1000,
                tax_owed: 0,
                forecloses_at: None,
                deed_cost: Some(1000),
                station_count: 6,
                station_capacity: 4 * (1 + built) + yard,
                terrain_id: Some((*id).clone()),
                corners: Some(*corners),
                indoor_room: Some(4),
                yard_room: Some(yard),
                owner_citizen: false,
                floor_ready_at: None,
                floors: built,
                next_floor: None,
            };
            state.apply_rest(&RestResponse::Plots(ListPlotsResponse { plots: vec![plot], total: 1 }));
            state.apply_ws(&ServerMessage::Positions {
                players: vec![PlayerPosition { id: me, name: "You".into(), x: centre.0, y: centre.1, citizen: false, tag: None }],
            });
        }
        Self {
            claimed: near.iter().map(|(_, id, _, _)| (*id).clone()).collect(),
            mine: near.first().map(|(_, id, _, _)| (*id).clone()).into_iter().collect(),
            floors,
            state,
            time: 0.0,
        }
    }

    /// Your lot with at least one floor, for `--place-sample` to go up to
    /// (V3.5): in the drawing and in the store.
    pub fn give_yours_a_floor(&mut self) {
        let Some(id) = self.mine.iter().next().cloned() else { return };
        match self.floors.iter_mut().find(|(l, _, _)| *l == id) {
            Some((_, built, _)) => *built = (*built).max(1),
            None => self.floors.push((id.clone(), 1, false)),
        }
        let built = self.floors.iter().find(|(l, _, _)| *l == id).map_or(1, |(_, k, _)| *k as i32);
        let mut plots = self.state.plots().to_vec();
        for p in plots.iter_mut().filter(|p| p.terrain_id.as_deref() == Some(id.as_str())) {
            p.station_capacity += 4 * (built - p.floors);
            p.floors = built;
        }
        let total = plots.len() as i64;
        self.state.apply_rest(&RestResponse::Plots(ListPlotsResponse { plots, total }));
    }

    /// Moves on by `dt`: the floor under way finishes at [`FINISH`].
    pub fn tick(&mut self, dt: f32) {
        self.time += dt;
        if self.time >= FINISH {
            for (_, built, under_way) in &mut self.floors {
                if *under_way {
                    *built += 1;
                    *under_way = false;
                }
            }
        }
    }
}

fn mid(a: P, b: P) -> P {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn unit(a: P) -> P {
    let l = a.0.hypot(a.1).max(1e-9);
    (a.0 / l, a.1 / l)
}

/// A station of `kind` at `at`, the `n`th, working on its own if it can.
fn station(state: &GameState, n: u128, kind: &str, at: P, owner: Uuid, indoors: bool, condition: i32) -> StationInfo {
    let automation = capitol_core::station::recipes_at(state.recipes(), kind)
        .next()
        .map(|r| StationAutomation {
            recipe_id: r.id.clone(),
            mode: AutomationMode::Forever,
            state: AutomationState::Running,
            done: 0,
        });
    StationInfo {
        id: Uuid::from_u128(0x5a_0000 + n),
        station_type: kind.into(),
        name: kind.into(),
        category: if kind == "storage_chest" { "storage" } else { "crafting" }.into(),
        icon: None,
        x: at.0,
        y: at.1,
        owner_id: owner,
        container_id: None,
        interaction_range: 75.0,
        quality: 60,
        tool: None,
        automation,
        production: Vec::new(),
        indoors,
        condition,
        floor: 0,
    }
}
