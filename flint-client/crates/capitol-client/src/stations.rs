//! The stations (P6.1): one entity per station the server has shown us,
//! a small model by type, placed from the store each frame like the
//! resource nodes (`nodes.rs`). The orange in-range ring round them is
//! `scripts/input.rhai`, from what [`Stations::update`] leaves in `Shared`.
//!
//! The models wear the React client's colours (`GameCanvas.tsx`
//! `drawStation`): a workbench of `#8b5a2b` planks on `#5d3a1a` legs, a
//! `#4a4a54` stone forge with an `#f97316` mouth and a `#fbbf24` heart,
//! and a `#8b6914` chest under an `#a67c1a` lid with a `#ffd700` lock.
//! React draws each 20 units (2 m) across, and so do these, give or take.
//!
//! E2.6 added the content's three new stations in the same hand: a
//! sawbench with its blade standing through the top, a beehive kiln of
//! brick with a glowing mouth, and an anvil on a stump. A type the client
//! doesn't know is a plank crate, so it still looks like something built.
//!
//! The forge's fire is real: the scene's `particles/forge.particles.toml`
//! (flames in the mouth, embers and a little smoke out of the chimney) on
//! its entity, and its mouth glows (`palette::glow`) bright enough to
//! bloom. The kiln has its own (`particles/kiln.particles.toml`: smoke and
//! a few sparks from its vent). Both fires burn brighter after dark:
//! [`Stations::fires`] sets their `material.emissive_scale` (H23, Flint
//! ADR 0090) from the hour, so no mesh is rebuilt.
//!
//! **At work (E4.4).** A station working on its own shows it. Running, its
//! light comes up: the forge's mouth and the kiln's door burn brighter, and
//! the workbench, the sawbench, the anvil and the crate each have a lantern
//! that's dark until then. The light swells as each craft comes due, and
//! when it's made a puff goes up (`particles/craft_sparks.particles.toml`
//! from the hot ones, `craft_wisp` from the rest) and the swell starts
//! again. A paused one sinks to embers, and over your own an ember floats
//! (`draw_station_ember` in `input.rhai`) with the reason on hover. It all
//! follows `station_automation_changed` and `automation_produced`, through
//! the same `material.emissive_scale` as the hour.
//!
//! A station that was placed just now (`station_placed`) drops into place
//! and settles. One seen for the first time, walking near it, rises out
//! of the ground instead, quickly, so it doesn't pop. One the server stops
//! listing (`station_removed`, or after a reconnect) sinks away and is
//! despawned.
//!
//! The server has no facing for them, so each turns its front (+Z: the
//! forge's mouth, the chest's lock) away from the Capitol, give or take a
//! little from its id. The camera mostly looks toward the tower, so that
//! is the side you see, and a yard of them shares a grain.
//!
//! **The ghost (P6.2).** While a kit is being placed, [`Stations::ghost`]
//! shows its station at the cursor. It's the same model, tinted toward
//! React's preview green (`#22c55e`) where it can go and red (`#ef4444`)
//! where it can't, translucent, and hovering a hand's width up. It glides
//! after the cursor rather than snapping to it. Once the click is sent it
//! holds still and breathes until the server answers. When the station
//! lands, the ghost is gone and the real one drops into its place.

use crate::coords::{to_server, to_world};
use crate::script_api::{Shared, StationMark};
use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{glow, hex, lit_glow, mix};
use capitol_core::automation::{pause_words, Work};
use capitol_core::protocol::rest::AutomationState;
use capitol_core::protocol::rules::TICK_MS;
use capitol_core::state::GameState;
use capitol_core::station::in_reach;
use flint_core::EntityId;
use flint_player::{ExtensionCtx, ExtensionGpu};
use std::collections::{HashMap, HashSet};
use std::f32::consts::{PI, TAU};
use uuid::Uuid;

/// The runtime meshes, one per type.
pub const MESH_WORKBENCH: &str = "station_workbench";
pub const MESH_FORGE: &str = "station_forge";
pub const MESH_CHEST: &str = "station_chest";
pub const MESH_SAWBENCH: &str = "station_sawbench";
pub const MESH_KILN: &str = "station_kiln";
pub const MESH_ANVIL: &str = "station_anvil";
pub const MESH_GENERIC: &str = "station_generic";

/// The forge's fire, `particles/forge.particles.toml`.
const FORGE_EFFECT: &str = "forge";
/// The kiln's smoke and sparks, `particles/kiln.particles.toml`.
const KILN_EFFECT: &str = "kiln";
/// A craft made on its own (E4.4): sparks off a hot station, a wisp of
/// dust and smoke off the others.
const SPARKS_EFFECT: &str = "craft_sparks";
const WISP_EFFECT: &str = "craft_wisp";
/// How long a craft's puff lasts, s, and how hard it blows.
const PUFF: f32 = 0.5;
const PUFF_SCALE: f32 = 4.0;
/// How fast a station's light follows its work, per second.
const LIGHT_RATE: f32 = 3.0;

/// Seconds to rise out of the ground, to drop into place, and to sink.
const RISE: f32 = 0.45;
const DROP: f32 = 0.55;
const SINK: f32 = 0.45;
/// How high a placed station drops from, in metres.
const DROP_FROM: f32 = 1.4;
/// How fast the ring eases in and out as you come into reach, per second.
const REACH_RATE: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrival {
    Rise,
    Drop,
}

struct Station {
    entity: EntityId,
    kind: &'static str,
    at: [f32; 2],
    yaw: f32,
    arrival: Arrival,
    /// 0 to 1: how far it has arrived. Falls back to 0 as it goes.
    shown: f32,
    present: bool,
    /// 0 to 1: the ring, eased toward whether you're in reach.
    reach: f32,
    /// The `emissive_scale` last set on its light ([`Stations::fires`]).
    fire: Option<f32>,
    /// What it's doing on its own (E4.4), and seconds since it began the
    /// craft under way.
    work: Work,
    since: f32,
    /// Its recipe's time, s, while it works.
    duration: f32,
    /// Its light's strength over the hour's: eased toward [`work_light`].
    light: f32,
    /// The fire's `emission_scale` last set ([`fire_rate`]).
    flame: Option<f32>,
    /// The craft's puff: an entity of its own, once it has made something,
    /// and 1 to 0 as the last one blows.
    puff: Option<EntityId>,
    blow: f32,
    blown: Option<f32>,
}

#[derive(Default)]
pub struct Stations {
    stations: HashMap<Uuid, Station>,
    meshes: bool,
    ghost: Option<Ghost>,
}

/// What the ghost should show this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GhostView {
    /// The model of the station being placed ([`kind_of`] its type).
    pub kind: &'static str,
    /// Where the cursor is on the ground, world metres (X, Z).
    pub at: [f32; 2],
    /// It can go down here.
    pub ok: bool,
    /// The click went out: hold still until the server answers.
    pub asking: bool,
}

struct Ghost {
    entity: EntityId,
    kind: &'static str,
    /// The tint its mesh wears now.
    ok: bool,
    at: [f32; 2],
    /// 0 to 1: faded in. Falls back to 0 as placement ends.
    shown: f32,
    /// Seconds shown, for its breathing.
    age: f32,
}

/// The ghost's opacity at full, its height over the ground (m), how fast
/// it fades in and out (per second), and how fast it closes on the cursor.
const GHOST_OPACITY: f32 = 0.6;
const GHOST_HOVER: f32 = 0.18;
const GHOST_FADE: f32 = 5.0;
const GHOST_FOLLOW: f32 = 22.0;

impl Stations {
    /// Place every station for this frame from `state`. `me` is where you
    /// stand in world metres, `placed` the stations placed since last
    /// frame. Writes the stations for the scripts into `shared`.
    /// `produced` are the stations that made something on their own since
    /// last frame.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        ctx: &mut ExtensionCtx,
        state: &GameState,
        me: Option<[f32; 3]>,
        placed: &HashSet<Uuid>,
        produced: &[Uuid],
        dt: f32,
        shared: &Shared,
    ) {
        if !self.meshes {
            let Some(gpu) = ctx.gpu.as_mut() else {
                return;
            };
            upload(gpu);
            self.meshes = true;
        }
        let me = me.map(|[x, _, z]| to_server(x, z));
        for station in self.stations.values_mut() {
            station.present = false;
        }
        let mut marks = Vec::with_capacity(state.stations().len());
        for (id, info) in state.stations() {
            let station = match self.stations.get_mut(id) {
                Some(s) => s,
                None => {
                    let kind = kind_of(&info.station_type);
                    let Some(entity) = spawn(ctx, *id, kind) else {
                        continue;
                    };
                    let at = to_world(info.x, info.y);
                    self.stations.entry(*id).or_insert(Station {
                        entity,
                        kind,
                        at,
                        yaw: facing(at, *id),
                        arrival: if placed.contains(id) {
                            Arrival::Drop
                        } else {
                            Arrival::Rise
                        },
                        shown: 0.0,
                        present: true,
                        reach: 0.0,
                        fire: None,
                        work: Work::Idle,
                        since: 0.0,
                        duration: 0.0,
                        light: work_light(kind, Work::Idle, 0.0),
                        flame: None,
                        puff: None,
                        blow: 0.0,
                        blown: None,
                    })
                }
            };
            station.present = true;
            let set = info.automation.as_ref();
            let work = Work::of(set);
            if work != station.work {
                station.work = work;
                station.since = 0.0;
            }
            station.since += dt;
            station.duration = set
                .and_then(|a| state.recipe(&a.recipe_id))
                .map_or(0.0, |r| r.duration_ticks.max(1) as f32 * TICK_MS as f32 / 1000.0);
            if produced.contains(id) {
                station.since = 0.0;
                station.blow = 1.0;
            }
            station.blow = (station.blow - dt / PUFF).max(0.0);
            let want = work_light(station.kind, station.work, station.progress());
            station.light += (want - station.light) * (1.0 - (-LIGHT_RATE * dt).exp());
            // An ember over your own while it waits, with why on hover
            let paused = match set.map(|a| &a.state) {
                Some(AutomationState::Paused { reason })
                    if state.me().is_some() && state.me() == Some(info.owner_id) =>
                {
                    let recipe = set.and_then(|a| state.recipe(&a.recipe_id));
                    Some(pause_words(reason, state.catalog(), recipe).what)
                }
                _ => None,
            };
            let near = me.is_some_and(|at| in_reach(at, info));
            let time = match station.arrival {
                Arrival::Rise => RISE,
                Arrival::Drop => DROP,
            };
            station.shown = (station.shown + dt / time).min(1.0);
            let want = if near { 1.0 } else { 0.0 };
            station.reach += (want - station.reach) * (1.0 - (-REACH_RATE * dt).exp());
            marks.push(StationMark {
                id: id.to_string(),
                kind: station.kind,
                at: station.at,
                radius: footprint(station.kind),
                in_reach: near,
                reach: station.reach * smooth(station.shown),
                top: height(station.kind) * smooth(station.shown),
                paused,
            });
        }

        let world = &mut *ctx.world;
        self.stations.retain(|_, station| {
            if !station.present {
                station.arrival = Arrival::Rise;
                station.shown -= dt / SINK;
                if station.shown <= 0.0 {
                    let _ = world.despawn(station.entity);
                    if let Some(puff) = station.puff {
                        let _ = world.despawn(puff);
                    }
                    return false;
                }
            }
            place(world, station);
            true
        });
        for (id, station) in &mut self.stations {
            station.flames(ctx.world, *id);
            station.puffs(ctx, *id);
        }
        if let Ok(mut shared) = shared.stations.lock() {
            *shared = marks;
        }
    }

    /// The ghost of the station being placed. `None` (placement over, or
    /// the cursor off the ground) fades it out, and it goes.
    pub fn ghost(&mut self, ctx: &mut ExtensionCtx, view: Option<GhostView>, dt: f32) {
        if !self.meshes {
            return;
        }
        let Some(view) = view else {
            if let Some(ghost) = &mut self.ghost {
                ghost.shown -= dt * GHOST_FADE;
                if ghost.shown <= 0.0 {
                    let _ = ctx.world.despawn(ghost.entity);
                    self.ghost = None;
                } else {
                    place_ghost(ctx.world, ghost, false);
                }
            }
            return;
        };
        let kind = view.kind;
        if self.ghost.as_ref().is_some_and(|g| g.kind != kind) {
            let old = self.ghost.take().expect("checked");
            let _ = ctx.world.despawn(old.entity);
        }
        if self.ghost.is_none() {
            let Some(entity) = spawn_ghost(ctx, kind, view.ok) else {
                return;
            };
            self.ghost = Some(Ghost {
                entity,
                kind,
                ok: view.ok,
                at: view.at,
                shown: 0.0,
                age: 0.0,
            });
        }
        let ghost = self.ghost.as_mut().expect("just made");
        if ghost.ok != view.ok {
            ghost.ok = view.ok;
            let _ = ctx.attach_mesh(ghost.entity, &ghost_mesh(kind, view.ok));
        }
        if !view.asking {
            let k = 1.0 - (-GHOST_FOLLOW * dt).exp();
            ghost.at[0] += (view.at[0] - ghost.at[0]) * k;
            ghost.at[1] += (view.at[1] - ghost.at[1]) * k;
        }
        ghost.age += dt;
        ghost.shown = (ghost.shown + dt * GHOST_FADE).min(1.0);
        place_ghost(ctx.world, ghost, view.asking);
    }

    /// Where the ghost is drawn, world metres, while there is one.
    pub fn ghost_at(&self) -> Option<[f32; 2]> {
        self.ghost.as_ref().map(|g| g.at)
    }

    /// Set every station's light for the hour's `scale` ([`fire_light`])
    /// and its work: the forge's and kiln's fires, and the lanterns, through
    /// their `material.emissive_scale`.
    pub fn fires(&mut self, world: &mut flint_ecs::FlintWorld, scale: f32) {
        for station in self.stations.values_mut() {
            if station.kind == "storage_chest" {
                continue;
            }
            // Stepped, so the material is only written when it moves
            let scale = (scale * station.light * 64.0).round() / 64.0;
            if station.fire == Some(scale) {
                continue;
            }
            let mut table = world
                .get_components(station.entity)
                .and_then(|c| c.get("material"))
                .and_then(|m| m.as_table())
                .cloned()
                .unwrap_or_default();
            table.insert("emissive_scale".into(), (scale as f64).into());
            match world.set_component(station.entity, "material", table.into()) {
                Ok(()) => station.fire = Some(scale),
                Err(e) => tracing::warn!("stations: no glow scale: {e}"),
            }
        }
    }
}

impl Station {
    /// 0 to 1: how near its craft is to done, while it runs.
    fn progress(&self) -> f32 {
        if self.work != Work::Running || self.duration <= 0.0 {
            return 0.0;
        }
        (self.since / self.duration).min(1.0)
    }

    /// The forge's flames and the kiln's smoke, harder while it works and
    /// low while it waits. Written only when the work changes.
    fn flames(&mut self, world: &mut flint_ecs::FlintWorld, id: Uuid) {
        let Some(effect) = fire_effect(self.kind) else {
            return;
        };
        let rate = fire_rate(self.work);
        if self.flame == Some(rate) || (self.flame.is_none() && rate == 1.0) {
            return;
        }
        let mut fx = toml::Table::new();
        fx.insert("effect".into(), effect.into());
        fx.insert("seed".into(), seed(id).into());
        fx.insert("emission_scale".into(), (rate as f64).into());
        let _ = world.set_component(self.entity, "particle_effect", toml::Value::Table(fx));
        self.flame = Some(rate);
    }

    /// The puff of a craft made on its own, from an entity of its own over
    /// the station: made at the first, and kept.
    fn puffs(&mut self, ctx: &mut ExtensionCtx, id: Uuid) {
        if self.puff.is_none() && self.blow > 0.0 {
            match ctx.world.spawn(format!("station_puff_{}", id.simple()).as_str()) {
                Ok(e) => self.puff = Some(e),
                Err(e) => tracing::warn!("stations: no puff for {id}: {e}"),
            }
        }
        let Some(entity) = self.puff else {
            return;
        };
        // Where the work shows: out of a fire's mouth, off a bench's top
        let [ox, oy, oz] = puff_at(self.kind);
        let (sin, cos) = self.yaw.sin_cos();
        let [x, z] = [
            self.at[0] + ox * cos + oz * sin,
            self.at[1] - ox * sin + oz * cos,
        ];
        let mut t = toml::Table::new();
        let top = crate::world::land::ground_y(self.at) + oy;
        t.insert("position".into(), vec3([x, top, z]));
        t.insert("rotation_quat".into(), vec4([0.0, 0.0, 0.0, 1.0]));
        t.insert("scale".into(), vec3([1.0, 1.0, 1.0]));
        let _ = ctx.world.set_component(entity, "transform", toml::Value::Table(t));
        let blow = (PUFF_SCALE * self.blow * 10.0).round() / 10.0;
        if self.blown == Some(blow) {
            return;
        }
        let mut fx = toml::Table::new();
        let effect = if hot(self.kind) { SPARKS_EFFECT } else { WISP_EFFECT };
        fx.insert("effect".into(), effect.into());
        fx.insert("seed".into(), seed(id).into());
        fx.insert("emission_scale".into(), (blow as f64).into());
        let _ = ctx
            .world
            .set_component(entity, "particle_effect", toml::Value::Table(fx));
        self.blown = Some(blow);
    }
}

/// How strong a station's light is, over the hour's, for its `work` and how
/// far its craft has got. A fire burns as built when idle; a lantern is dark.
/// Running, both come up and swell toward the craft's end; waiting, both sink
/// to embers.
pub fn work_light(kind: &str, work: Work, progress: f32) -> f32 {
    let swell = smooth(progress);
    match (fired(kind), work) {
        (true, Work::Idle) => 1.0,
        (true, Work::Running) => 1.4 + 0.35 * swell,
        (true, Work::Paused) => 0.12,
        (false, Work::Idle) => 0.0,
        (false, Work::Running) => 0.9 + 0.3 * swell,
        (false, Work::Paused) => 0.22,
    }
}

/// The fire's particles for its work: as built when idle.
fn fire_rate(work: Work) -> f32 {
    match work {
        Work::Idle => 1.0,
        Work::Running => 1.6,
        Work::Paused => 0.15,
    }
}

/// A station that throws sparks when it makes something: the fired ones
/// and the anvil.
fn hot(kind: &str) -> bool {
    fired(kind) || kind == "anvil"
}

fn fire_effect(kind: &str) -> Option<&'static str> {
    match kind {
        "forge" => Some(FORGE_EFFECT),
        "kiln" => Some(KILN_EFFECT),
        _ => None,
    }
}

/// Each station's particles flicker their own way.
fn seed(id: Uuid) -> i64 {
    (id.as_u128() % 1_000_000) as i64 + 1
}

/// Where a station's puff goes up, in its model's space (m, +Z its front):
/// just out of a fire's mouth, else over its work.
fn puff_at(kind: &str) -> [f32; 3] {
    match kind {
        "forge" => [0.0, 0.62, 0.78],
        "kiln" => [0.0, 0.5, 1.0],
        "anvil" => [0.0, 0.8, 0.0],
        "workbench" => [0.0, 1.0, 0.0],
        "sawbench" => [0.0, 0.9, 0.0],
        _ => [0.0, 1.05, 0.0],
    }
}

/// About how tall a station's model is, m: where its ember floats.
fn height(kind: &str) -> f32 {
    match kind {
        "workbench" => 1.05,
        "forge" => 2.2,
        "storage_chest" => 0.8,
        "sawbench" => 1.1,
        "kiln" => 1.8,
        "anvil" => 0.85,
        _ => 1.1,
    }
}

/// `--station-sample`: a store holding one of every station type the
/// catalog knows, and a `loom` it doesn't, in a row across the view from
/// `camera` to `target` (world metres, X and Z), [`SAMPLE_SPACING`] apart.
pub fn sample(target: [f32; 2], camera: [f32; 2]) -> GameState {
    use capitol_core::protocol::ws::ServerMessage;
    let mut state = GameState::new();
    let mut types: Vec<String> = state
        .catalog()
        .station_types()
        .iter()
        .map(|t| t.id.clone())
        .collect();
    types.push("loom".into());
    let (dx, dz) = (target[0] - camera[0], target[1] - camera[1]);
    let len = dx.hypot(dz).max(1e-3);
    let across = [dz / len, -dx / len];
    let mid = (types.len() as f32 - 1.0) / 2.0;
    for (i, t) in types.iter().enumerate() {
        let k = (i as f32 - mid) * SAMPLE_SPACING;
        let [x, y] = to_server(target[0] + across[0] * k, target[1] + across[1] * k);
        state.apply_ws(&ServerMessage::StationPlaced {
            station_id: Uuid::from_u128(i as u128 + 1),
            station_type: t.clone(),
            name: t.clone(),
            owner_id: Uuid::nil(),
            x,
            y,
            container_id: Uuid::nil(),
            quality: 60,
        });
    }
    state
}

/// How far apart `--station-sample` stands them, metres.
const SAMPLE_SPACING: f32 = 3.6;

/// `--automation-sample` (E4.4): stations at work and waiting, in a row
/// across the view as `--station-sample` stands them, all yours: a forge
/// running, one waiting for ore and one idle, then a workbench and an anvil
/// running and a kiln with its box full. [`automation_tick`] makes their
/// crafts come due.
pub fn automation_sample(target: [f32; 2], camera: [f32; 2]) -> GameState {
    use capitol_core::net::{ConnectionState, RestResponse};
    use capitol_core::protocol::rest::{AutomationMode, PauseReason, RecipesResponse, StationAutomation};
    use capitol_core::protocol::ws::ServerMessage;
    let me = Uuid::from_u128(0xca9);
    let mut state = GameState::new();
    state.apply_connection(&ConnectionState::Connected { player_id: me });
    let recipes = state.catalog().recipes().to_vec();
    state.apply_rest(&RestResponse::Recipes(RecipesResponse { recipes }));
    let running = || AutomationState::Running;
    let rows: [(&str, Option<AutomationState>); 6] = [
        ("forge", Some(running())),
        (
            "forge",
            Some(AutomationState::Paused {
                reason: PauseReason::MissingInput {
                    item_type: "iron_ore".into(),
                },
            }),
        ),
        ("forge", None),
        ("workbench", Some(running())),
        ("anvil", Some(running())),
        (
            "kiln",
            Some(AutomationState::Paused {
                reason: PauseReason::OutputFull,
            }),
        ),
    ];
    let (dx, dz) = (target[0] - camera[0], target[1] - camera[1]);
    let len = dx.hypot(dz).max(1e-3);
    let across = [dz / len, -dx / len];
    let mid = (rows.len() as f32 - 1.0) / 2.0;
    for (i, (kind, work)) in rows.iter().enumerate() {
        let id = Uuid::from_u128(i as u128 + 1);
        let k = (i as f32 - mid) * SAMPLE_SPACING;
        let [x, y] = to_server(target[0] + across[0] * k, target[1] + across[1] * k);
        state.apply_ws(&ServerMessage::StationPlaced {
            station_id: id,
            station_type: (*kind).into(),
            name: (*kind).into(),
            owner_id: me,
            x,
            y,
            container_id: Uuid::nil(),
            quality: 60,
        });
        let recipe = capitol_core::station::recipes_at(state.recipes(), kind)
            .next()
            .map(|r| r.id.clone());
        if let (Some(work), Some(recipe_id)) = (work, recipe) {
            state.apply_ws(&ServerMessage::StationAutomationChanged {
                station_id: id,
                automation: StationAutomation {
                    recipe_id,
                    mode: AutomationMode::Forever,
                    state: work.clone(),
                    done: 0,
                },
            });
        }
    }
    state
}

/// The sample's crafts: each running station makes one every time its
/// recipe's duration passes. Returns the ones that did this frame, `t`
/// seconds into the sample.
pub fn automation_tick(state: &mut GameState, t: f32, dt: f32) -> Vec<Uuid> {
    use capitol_core::protocol::rest::Produced;
    use capitol_core::protocol::ws::ServerMessage;
    let due: Vec<(Uuid, String, u32)> = state
        .stations()
        .iter()
        .filter_map(|(id, s)| {
            let a = s.automation.as_ref().filter(|a| a.state == AutomationState::Running)?;
            let r = state.recipe(&a.recipe_id)?;
            let every = r.duration_ticks.max(1) as f32 * TICK_MS as f32 / 1000.0;
            let item = r.outputs.first()?.item_type.clone();
            ((t / every).floor() > ((t - dt) / every).floor()).then_some((*id, item, a.done + 1))
        })
        .collect();
    let mut made = Vec::new();
    for (station_id, item_type, done) in due {
        let produced = Produced {
            item_type,
            quantity: 1,
            quality: 40 + (done as i32 * 7) % 50,
            at: capitol_core::chrono::Utc::now(),
        };
        state.apply_ws(&ServerMessage::AutomationProduced {
            station_id,
            produced,
            done,
        });
        made.push(station_id);
    }
    made
}

/// Whether a station of `kind` has a fire in it, whose glow follows the
/// hour.
fn fired(kind: &str) -> bool {
    matches!(kind, "forge" | "kiln")
}

/// How bright a fire's glow is at `glow`, the light on your plots: as
/// built by day, and up to 1.8 times that at night, when a lit mouth is
/// most of what you see of a yard. Stepped like the glow, so the material
/// is only written when it moves.
pub fn fire_light(glow: crate::daylight::PlotGlow) -> f32 {
    1.0 + 0.5 * glow.stepped().radiance
}

/// A ghost entity wearing `kind`'s tinted mesh.
fn spawn_ghost(ctx: &mut ExtensionCtx, kind: &'static str, ok: bool) -> Option<EntityId> {
    let entity = match ctx.world.spawn("station_ghost") {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("stations: no entity for the ghost: {e}");
            return None;
        }
    };
    if let Err(e) = ctx.attach_mesh(entity, &ghost_mesh(kind, ok)) {
        tracing::warn!("stations: no mesh for the ghost: {e}");
    }
    Some(entity)
}

/// Write the ghost's transform and opacity onto its entity. It hovers and
/// breathes. While asking it breathes faster, as if about to settle.
fn place_ghost(world: &mut flint_ecs::FlintWorld, g: &Ghost, asking: bool) {
    let [x, z] = g.at;
    let rate = if asking { 7.0 } else { 2.2 };
    let bob = 0.04 * (g.age * rate).sin();
    let k = smooth(g.shown);
    let mut t = toml::Table::new();
    t.insert(
        "position".into(),
        vec3([x, crate::world::land::ground_y([x, z]) + GHOST_HOVER * k + bob, z]),
    );
    let (sin, cos) = (facing_plain([x, z]) * 0.5).sin_cos();
    t.insert("rotation_quat".into(), vec4([0.0, sin, 0.0, cos]));
    let s = 0.9 + 0.1 * k;
    t.insert("scale".into(), vec3([s, s, s]));
    let _ = world.set_component(g.entity, "transform", toml::Value::Table(t));
    let pulse = if asking {
        0.8 + 0.2 * (g.age * 7.0).sin()
    } else {
        1.0
    };
    let mut m = toml::Table::new();
    m.insert(
        "opacity".into(),
        toml::Value::Float((GHOST_OPACITY * k * pulse).max(0.01) as f64),
    );
    let _ = world.set_component(g.entity, "material", toml::Value::Table(m));
}

/// The ghost's mesh for `kind`: tinted green where it can go, red where
/// it can't.
fn ghost_mesh(kind: &str, ok: bool) -> String {
    format!("{}_ghost_{}", mesh(kind), if ok { "ok" } else { "bad" })
}

/// The type's mesh name and look: the six in the content, or the crate
/// for anything else.
pub fn kind_of(station_type: &str) -> &'static str {
    match station_type {
        "workbench" => "workbench",
        "forge" => "forge",
        "storage_chest" => "storage_chest",
        "sawbench" => "sawbench",
        "kiln" => "kiln",
        "anvil" => "anvil",
        _ => "generic",
    }
}

fn mesh(kind: &str) -> &'static str {
    match kind {
        "workbench" => MESH_WORKBENCH,
        "forge" => MESH_FORGE,
        "storage_chest" => MESH_CHEST,
        "sawbench" => MESH_SAWBENCH,
        "kiln" => MESH_KILN,
        "anvil" => MESH_ANVIL,
        _ => MESH_GENERIC,
    }
}

/// The radius of the in-range ring, metres: React's is the station's 20
/// units plus a little, about its footprint here.
fn footprint(kind: &str) -> f32 {
    match kind {
        "workbench" => 1.45,
        "forge" => 1.95,
        "storage_chest" => 1.0,
        "sawbench" => 1.35,
        "kiln" => 1.4,
        "anvil" => 1.0,
        _ => 1.0,
    }
}

/// The yaw that turns a station at `at` (world metres) to face away from
/// the Capitol, within 15° either way by its id.
fn facing([x, z]: [f32; 2], id: Uuid) -> f32 {
    let jitter = ((id.as_u128() % 1000) as f32 / 1000.0 - 0.5) * 30f32.to_radians();
    facing_plain([x, z]) + jitter
}

/// Straight away from the Capitol, for the ghost, which has no id yet.
fn facing_plain([x, z]: [f32; 2]) -> f32 {
    x.atan2(z)
}

/// The ring round a station of `station_type`, metres, as the in-range
/// ring draws it: the ghost's ring matches.
pub fn ring_radius(station_type: &str) -> f32 {
    footprint(kind_of(station_type))
}

fn spawn(ctx: &mut ExtensionCtx, id: Uuid, kind: &'static str) -> Option<EntityId> {
    let name = format!("station_{}", id.simple());
    let entity = match ctx.world.spawn(name.as_str()) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("stations: no entity for {id}: {e}");
            return None;
        }
    };
    if let Err(e) = ctx.attach_mesh(entity, mesh(kind)) {
        tracing::warn!("stations: no mesh for {id}: {e}");
    }
    if let Some(effect) = fire_effect(kind) {
        let mut fx = toml::Table::new();
        fx.insert("effect".into(), effect.into());
        fx.insert("seed".into(), seed(id).into());
        if let Err(e) = ctx
            .world
            .set_component(entity, "particle_effect", toml::Value::Table(fx))
        {
            tracing::warn!("stations: no fire for {id}: {e}");
        }
    }
    Some(entity)
}

/// Write the station's transform onto its entity.
fn place(world: &mut flint_ecs::FlintWorld, s: &Station) {
    let [x, z] = s.at;
    let k = s.shown.clamp(0.0, 1.0);
    let (lift, [across, up]) = match s.arrival {
        Arrival::Rise => (0.0, [0.7 + 0.3 * smooth(k), smooth(k)]),
        Arrival::Drop => drop(k),
    };
    let mut t = toml::Table::new();
    let ground = crate::world::land::ground_y([x, z]);
    t.insert("position".into(), vec3([x, ground + lift, z]));
    let (sin, cos) = (s.yaw * 0.5).sin_cos();
    t.insert("rotation_quat".into(), vec4([0.0, sin, 0.0, cos]));
    // Never quite zero: a zero scale has no inverse for the normals.
    let (a, u) = (across.max(1e-3), up.max(1e-3));
    t.insert("scale".into(), vec3([a, u, a]));
    let _ = world.set_component(s.entity, "transform", toml::Value::Table(t));
}

/// A placed station's fall: down from [`DROP_FROM`] under gravity, a
/// squash as it lands, and a settle. Returns the lift and `[across, up]`.
fn drop(k: f32) -> (f32, [f32; 2]) {
    const FALL: f32 = 0.45;
    if k < FALL {
        let t = k / FALL;
        (DROP_FROM * (1.0 - t * t), [0.92, 1.06])
    } else {
        // A damped wobble about full size after the landing.
        let t = (k - FALL) / (1.0 - FALL);
        let w = (1.0 - t).powi(2) * (t * 2.5 * TAU).cos();
        (0.0, [1.0 + 0.1 * w, 1.0 - 0.16 * w])
    }
}

fn vec3(v: [f32; 3]) -> toml::Value {
    toml::Value::Array(v.iter().map(|c| toml::Value::Float(*c as f64)).collect())
}

fn vec4(v: [f32; 4]) -> toml::Value {
    toml::Value::Array(v.iter().map(|c| toml::Value::Float(*c as f64)).collect())
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ── The meshes ───────────────────────────────────────────────────────────

const WOOD: u32 = 0x8b5a2b;
const WOOD_DARK: u32 = 0x5d3a1a;
const WOOD_LIGHT: u32 = 0xa67c52;
const STONE: u32 = 0x4a4a54;
const FIRE: u32 = 0xf97316;
const FLAME: u32 = 0xfbbf24;
const CHEST: u32 = 0x8b6914;
const LID: u32 = 0xa67c1a;
const LOCK: u32 = 0xffd700;
const IRON: u32 = 0x2e2e36;
/// The kiln's brick, fired red, its courses a shade apart.
const BRICK: u32 = 0x9a4a2c;
const BRICK_LIGHT: u32 = 0xb5633a;
const BRICK_DARK: u32 = 0x6b3320;
/// Bright steel: the saw's blade, the anvil's face.
const STEEL: u32 = 0x94a3b8;
/// The Capitol's gold: a touch on each new station, as on the icons.
const GOLD: u32 = 0xc9a227;
/// Sawdust, and the pale end grain of a cut.
const SAWDUST: u32 = 0xd9b98a;
/// The crate an unknown type is drawn as.
const CRATE: u32 = 0x8a6a40;
/// React's placement preview: `#22c55e` where it can go, `#ef4444` where not.
const GHOST_OK: u32 = 0x22c55e;
const GHOST_BAD: u32 = 0xef4444;
/// How far the ghost's colours lean to its tint.
const GHOST_TINT: f32 = 0.6;

/// Every type's model, and its two ghosts.
fn upload(gpu: &mut ExtensionGpu) {
    let models = [
        ("workbench", workbench(), 0.85),
        ("forge", forge(), 0.9),
        ("storage_chest", chest(), 0.7),
        ("sawbench", sawbench(), 0.8),
        ("kiln", kiln(), 0.92),
        ("anvil", anvil(), 0.6),
        ("generic", generic(), 0.85),
    ];
    for (kind, m, roughness) in models {
        for ok in [true, false] {
            let tint = if ok { GHOST_OK } else { GHOST_BAD };
            tinted(&m, tint).upload(gpu, &ghost_mesh(kind, ok), roughness);
        }
        m.upload(gpu, mesh(kind), roughness);
    }
}

/// A copy of `m` with every colour leaning toward `tint`, and lifted a
/// little, so the ghost reads as light rather than paint.
fn tinted(m: &MeshBuf, tint: u32) -> MeshBuf {
    let tint = hex(tint);
    let mut out = MeshBuf {
        vertices: m.vertices.clone(),
        indices: m.indices.clone(),
    };
    for v in &mut out.vertices {
        let c = mix(v.color, tint, GHOST_TINT);
        v.color = [c[0] * 1.15, c[1] * 1.15, c[2] * 1.15, 1.0];
    }
    out
}

/// A box from `lo` to `hi`, without its underside.
fn cuboid(m: &mut MeshBuf, lo: [f32; 3], hi: [f32; 3], colour: [f32; 4]) {
    let [x0, y0, z0] = lo;
    let [x1, y1, z1] = hi;
    m.quad(
        [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
        UP,
        colour,
    );
    m.quad(
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        [0.0, 0.0, 1.0],
        colour,
    );
    m.quad(
        [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]],
        [0.0, 0.0, -1.0],
        colour,
    );
    m.quad(
        [[x1, y0, z0], [x1, y0, z1], [x1, y1, z1], [x1, y1, z0]],
        [1.0, 0.0, 0.0],
        colour,
    );
    m.quad(
        [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        [-1.0, 0.0, 0.0],
        colour,
    );
}

/// A four-sided frustum: `[half x, half z]` at `y0` narrowing to the
/// top's at `y1`, centred on the Y axis, with its top capped.
fn frustum(m: &mut MeshBuf, y0: f32, bottom: [f32; 2], y1: f32, top: [f32; 2], colour: [f32; 4]) {
    let at = |[hx, hz]: [f32; 2], y: f32, sx: f32, sz: f32| [hx * sx, y, hz * sz];
    let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    for i in 0..4 {
        let (ax, az) = corners[i];
        let (bx, bz) = corners[(i + 1) % 4];
        let quad = [
            at(bottom, y0, ax, az),
            at(bottom, y0, bx, bz),
            at(top, y1, bx, bz),
            at(top, y1, ax, az),
        ];
        let e = sub(quad[1], quad[0]);
        let u = sub(quad[3], quad[0]);
        let mut n = normalise(cross(e, u));
        // Outward: away from the axis.
        let mid = [
            (quad[0][0] + quad[1][0]) * 0.5,
            0.0,
            (quad[0][2] + quad[1][2]) * 0.5,
        ];
        if n[0] * mid[0] + n[2] * mid[2] < 0.0 {
            n = neg(n);
        }
        m.quad(quad, n, colour);
    }
    m.quad(
        [
            at(top, y1, -1.0, -1.0),
            at(top, y1, 1.0, -1.0),
            at(top, y1, 1.0, 1.0),
            at(top, y1, -1.0, 1.0),
        ],
        UP,
        colour,
    );
}

/// A workbench, its long side along X, 2 m by 0.9, its top at hip
/// height: four planks on four legs, a shelf below, a vise at one end
/// and a mallet left lying on it.
fn workbench() -> MeshBuf {
    let mut m = MeshBuf::default();
    let (hx, hz, top) = (1.0, 0.45, 0.88);
    // The top: four planks, alternately lighter, with hairline gaps.
    let plank = (2.0 * hz) / 4.0;
    for i in 0..4 {
        let z0 = -hz + i as f32 * plank + 0.006;
        let z1 = z0 + plank - 0.012;
        let c = if i % 2 == 0 {
            hex(WOOD)
        } else {
            mix(hex(WOOD), hex(WOOD_LIGHT), 0.45)
        };
        cuboid(&mut m, [-hx, top, z0], [hx, top + 0.07, z1], c);
    }
    // The front edge catches the light, as React's highlight strip does.
    cuboid(
        &mut m,
        [-hx, top + 0.07, hz - 0.05],
        [hx, top + 0.085, hz - 0.006],
        hex(WOOD_LIGHT),
    );
    // Legs, an apron under the top, and a shelf between the legs.
    let leg = 0.055;
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let (cx, cz) = (sx * (hx - 0.12), sz * (hz - 0.08));
        cuboid(
            &mut m,
            [cx - leg, 0.0, cz - leg],
            [cx + leg, top, cz + leg],
            hex(WOOD_DARK),
        );
    }
    let dark = mix(hex(WOOD_DARK), hex(WOOD), 0.35);
    cuboid(
        &mut m,
        [-hx + 0.1, top - 0.1, hz - 0.12],
        [hx - 0.1, top, hz - 0.07],
        dark,
    );
    cuboid(
        &mut m,
        [-hx + 0.1, top - 0.1, -hz + 0.07],
        [hx - 0.1, top, -hz + 0.12],
        dark,
    );
    cuboid(
        &mut m,
        [-hx + 0.12, 0.2, -hz + 0.1],
        [hx - 0.12, 0.25, hz - 0.1],
        mix(hex(WOOD), hex(WOOD_DARK), 0.4),
    );
    // Offcuts on the shelf.
    cuboid(
        &mut m,
        [-0.55, 0.25, -0.2],
        [-0.15, 0.33, -0.05],
        hex(WOOD_LIGHT),
    );
    cuboid(&mut m, [-0.5, 0.33, -0.15], [-0.2, 0.39, 0.02], hex(WOOD));
    // A vise at the right end: jaws and a screw handle.
    let iron = hex(IRON);
    cuboid(
        &mut m,
        [hx - 0.32, top + 0.07, hz - 0.12],
        [hx - 0.08, top + 0.2, hz + 0.02],
        iron,
    );
    cuboid(
        &mut m,
        [hx - 0.32, top - 0.06, hz + 0.02],
        [hx - 0.08, top + 0.2, hz + 0.08],
        iron,
    );
    cuboid(
        &mut m,
        [hx - 0.21, top + 0.05, hz + 0.08],
        [hx - 0.19, top + 0.07, hz + 0.26],
        iron,
    );
    cuboid(
        &mut m,
        [hx - 0.3, top + 0.05, hz + 0.24],
        [hx - 0.1, top + 0.07, hz + 0.26],
        iron,
    );
    // A mallet lying across the top.
    let y = top + 0.07;
    cuboid(
        &mut m,
        [-0.3, y, 0.05],
        [0.12, y + 0.035, 0.09],
        hex(WOOD_LIGHT),
    );
    cuboid(
        &mut m,
        [0.12, y, -0.01],
        [0.26, y + 0.1, 0.15],
        hex(WOOD_DARK),
    );
    // A lantern at the back corner, dark until it works on its own (E4.4).
    lantern(&mut m, [-hx + 0.16, y, -hz + 0.14], 0.8);
    m
}

/// A small iron lantern standing at `base`, `s` times its full 0.255 m: a
/// foot, a pane of warm glass, corner posts, a cap and a ring. The glass is
/// part lit surface, part light, so it reads as glass when dark and glows
/// when its `emissive_scale` comes up.
fn lantern(m: &mut MeshBuf, [x, y, z]: [f32; 3], s: f32) {
    let iron = hex(IRON);
    let (w, g, post) = (0.075 * s, 0.055 * s, 0.008 * s);
    let at = |h: f32| y + h * s;
    cuboid(m, [x - w, y, z - w], [x + w, at(0.025), z + w], iron);
    cuboid(
        m,
        [x - g, at(0.025), z - g],
        [x + g, at(0.17), z + g],
        lit_glow(hex(FLAME), 0.45, LANTERN_GLOW),
    );
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let (cx, cz) = (x + sx * g, z + sz * g);
        cuboid(
            m,
            [cx - post, at(0.025), cz - post],
            [cx + post, at(0.17), cz + post],
            iron,
        );
    }
    let knob = 0.02 * s;
    frustum_at(m, [x, z], at(0.17), [w, w], at(0.215), [knob, knob], iron);
    cuboid(
        m,
        [x - 0.006 * s, at(0.215), z - 0.025 * s],
        [x + 0.006 * s, at(0.255), z + 0.025 * s],
        iron,
    );
}

/// How bright a lantern's glass is at full, before its `emissive_scale`:
/// past the scene's bloom threshold (2.5), so a working one blooms at dusk.
const LANTERN_GLOW: f32 = 3.2;

/// A forge: a stone hearth narrowing as it rises from a plinth, three
/// courses of it, an arched mouth glowing at the front (+Z), a chimney,
/// and an anvil beside it.
fn forge() -> MeshBuf {
    let mut m = MeshBuf::default();
    let stone = hex(STONE);
    let light = mix(stone, hex(0x6a6a76), 0.6);
    let dark = mix(stone, hex(0x24242a), 0.5);
    // A plinth, and the body in three courses, the middle one lighter, so
    // it reads as laid stone from the street.
    cuboid(&mut m, [-0.95, 0.0, -0.75], [0.95, 0.22, 0.85], dark);
    let courses = [
        (0.22, [0.85, 0.65], 0.6, [0.75, 0.57], stone),
        (0.6, [0.75, 0.57], 0.98, [0.65, 0.49], light),
        (0.98, [0.65, 0.49], 1.32, [0.56, 0.42], stone),
    ];
    for (y0, b, y1, t, c) in courses {
        frustum(&mut m, y0, b, y1, t, c);
    }
    // The hood's lip, and the chimney with a cap.
    cuboid(&mut m, [-0.6, 1.32, -0.46], [0.6, 1.38, 0.46], dark);
    cuboid(&mut m, [-0.22, 1.38, -0.32], [0.22, 2.35, 0.12], stone);
    cuboid(&mut m, [-0.28, 2.35, -0.38], [0.28, 2.42, 0.18], dark);

    // The mouth: an arch in the front face, the fire's glow round the
    // heart of it. The face leans back as it rises, so each point is set
    // on it, a hair proud.
    let face_z = |y: f32| {
        let (y0, z0, y1, z1) = (0.22, 0.65, 0.98, 0.49);
        let t = ((y - y0) / (y1 - y0)).clamp(0.0, 1.0);
        z0 + (z1 - z0) * t
    };
    let n = normalise([0.0, 0.16, 0.76]);
    let arch = |half: f32, base: f32, rise: f32, proud: f32| {
        let mut pts = vec![[-half, base], [half, base]];
        const STEPS: usize = 10;
        for i in 0..=STEPS {
            let a = i as f32 / STEPS as f32 * PI;
            pts.push([half * a.cos(), base + rise + half * a.sin()]);
        }
        pts.push([-half, base]);
        pts.into_iter()
            .map(|[x, y]| [x, y, face_z(y) + proud])
            .collect::<Vec<_>>()
    };
    let mut lit = MeshBuf::default();
    // A dark stone surround, then the glow, then the white-hot heart.
    let surround = arch(0.36, 0.3, 0.2, 0.012);
    m.fan(
        [0.0, 0.5, face_z(0.5) + 0.012],
        &surround,
        n,
        mix(dark, hex(0x000000), 0.4),
    );
    let mouth = arch(0.28, 0.33, 0.17, 0.02);
    lit.fan(
        [0.0, 0.48, face_z(0.48) + 0.02],
        &mouth,
        n,
        glow(hex(FIRE), 1.15),
    );
    let heart = arch(0.13, 0.34, 0.03, 0.028);
    lit.fan(
        [0.0, 0.4, face_z(0.4) + 0.028],
        &heart,
        n,
        glow(hex(FLAME), 3.2),
    );
    // Coals spilling onto the plinth's lip, glowing.
    for (x, z, s, c) in [
        (-0.14, 0.72, 0.05, FIRE),
        (0.06, 0.76, 0.045, FLAME),
        (0.2, 0.7, 0.04, FIRE),
        (-0.02, 0.69, 0.035, FIRE),
    ] {
        let mut coal = MeshBuf::default();
        cuboid(
            &mut coal,
            [x - s, 0.22, z - s],
            [x + s, 0.22 + s * 1.2, z + s],
            glow(hex(c), 1.4),
        );
        lit.append(coal);
    }
    // The flue, hot at the chimney's top, so the forge shows it's lit
    // from any side, not only its mouth's.
    lit.quad(
        [
            [-0.13, 2.425, -0.23],
            [0.13, 2.425, -0.23],
            [0.13, 2.425, 0.03],
            [-0.13, 2.425, 0.03],
        ],
        UP,
        glow(mix(hex(FIRE), hex(FLAME), 0.3), 2.8),
    );
    m.append(lit);

    // An anvil to the right: a foot, a waist, a face and a horn.
    let iron = hex(IRON);
    let (ax, az) = (1.35, 0.25);
    let stump = hex(WOOD_DARK);
    frustum_at(&mut m, [ax, az], 0.0, [0.22, 0.22], 0.42, [0.2, 0.2], stump);
    cuboid(
        &mut m,
        [ax - 0.16, 0.42, az - 0.12],
        [ax + 0.16, 0.5, az + 0.12],
        iron,
    );
    cuboid(
        &mut m,
        [ax - 0.08, 0.5, az - 0.07],
        [ax + 0.08, 0.6, az + 0.07],
        iron,
    );
    cuboid(
        &mut m,
        [ax - 0.24, 0.6, az - 0.1],
        [ax + 0.2, 0.7, az + 0.1],
        mix(iron, hex(0x8a8a96), 0.3),
    );
    m.fan(
        [ax + 0.38, 0.66, az],
        &[
            [ax + 0.2, 0.62, az - 0.07],
            [ax + 0.2, 0.7, az - 0.07],
            [ax + 0.2, 0.7, az + 0.07],
            [ax + 0.2, 0.62, az + 0.07],
            [ax + 0.2, 0.62, az - 0.07],
        ],
        [1.0, 0.0, 0.0],
        iron,
    );
    m
}

/// [`frustum`], centred on `(cx, cz)` instead of the axis.
fn frustum_at(
    m: &mut MeshBuf,
    [cx, cz]: [f32; 2],
    y0: f32,
    bottom: [f32; 2],
    y1: f32,
    top: [f32; 2],
    colour: [f32; 4],
) {
    let mut f = MeshBuf::default();
    frustum(&mut f, y0, bottom, y1, top, colour);
    for v in &mut f.vertices {
        v.position[0] += cx;
        v.position[2] += cz;
    }
    m.append(f);
}

/// A storage chest, 1.1 m long: a box, a rounded lid, two iron bands
/// over both, and a gold lock at the front (+Z).
fn chest() -> MeshBuf {
    let mut m = MeshBuf::default();
    let (hx, hz, h) = (0.55, 0.32, 0.45);
    cuboid(&mut m, [-hx, 0.0, -hz], [hx, h, hz], hex(CHEST));
    // A darker plinth strip, so it sits rather than floats.
    cuboid(
        &mut m,
        [-hx - 0.02, 0.0, -hz - 0.02],
        [hx + 0.02, 0.06, hz + 0.02],
        mix(hex(CHEST), hex(WOOD_DARK), 0.6),
    );
    // The lid: half a barrel along X.
    barrel(&mut m, [-hx - 0.015, hx + 0.015], h, hz + 0.015, hex(LID));
    // Iron bands round body and lid.
    let iron = hex(IRON);
    for cx in [-0.32, 0.32] {
        cuboid(
            &mut m,
            [cx - 0.035, 0.03, hz],
            [cx + 0.035, h, hz + 0.02],
            iron,
        );
        cuboid(
            &mut m,
            [cx - 0.035, 0.03, -hz - 0.02],
            [cx + 0.035, h, -hz],
            iron,
        );
        barrel(&mut m, [cx - 0.035, cx + 0.035], h, hz + 0.03, iron);
    }
    // The lock: a gold plate where lid meets body, with a keyhole.
    let gold = hex(LOCK);
    cuboid(
        &mut m,
        [-0.08, h - 0.14, hz],
        [0.08, h + 0.05, hz + 0.03],
        gold,
    );
    cuboid(
        &mut m,
        [-0.015, h - 0.09, hz + 0.03],
        [0.015, h - 0.03, hz + 0.034],
        hex(0x1a1a1a),
    );
    m
}

/// Half a cylinder of radius `r` lying along X from `xs[0]` to `xs[1]`,
/// its flat side down at `y`, with its ends closed.
fn barrel(m: &mut MeshBuf, xs: [f32; 2], y: f32, r: f32, colour: [f32; 4]) {
    const STEPS: usize = 8;
    let squash = 0.55;
    let at = |i: usize| {
        let a = i as f32 / STEPS as f32 * PI;
        [a.cos() * r, y + a.sin() * r * squash]
    };
    for i in 0..STEPS {
        let ([z0, y0], [z1, y1]) = (at(i), at(i + 1));
        let mid = (i as f32 + 0.5) / STEPS as f32 * PI;
        let n = normalise([0.0, mid.sin() / squash, mid.cos()]);
        m.quad(
            [
                [xs[0], y0, z0],
                [xs[1], y0, z0],
                [xs[1], y1, z1],
                [xs[0], y1, z1],
            ],
            n,
            colour,
        );
    }
    for (x, nx) in [(xs[0], -1.0), (xs[1], 1.0)] {
        let rim: Vec<[f32; 3]> = (0..=STEPS).map(at).map(|[z, y]| [x, y, z]).collect();
        m.fan([x, y, 0.0], &rim, [nx, 0.0, 0.0], colour);
    }
}

/// A band round the Y axis through `(cx, cz)`, `sides` faces, from radius
/// `r0` at `y0` to `r1` at `y1`: a cylinder, a cone's slice or a dome's
/// course. No caps.
#[allow(clippy::too_many_arguments)]
fn band(
    m: &mut MeshBuf,
    [cx, cz]: [f32; 2],
    sides: usize,
    y0: f32,
    r0: f32,
    y1: f32,
    r1: f32,
    colour: [f32; 4],
) {
    let at = |i: usize, r: f32, y: f32| {
        let a = i as f32 / sides as f32 * TAU;
        [cx + a.sin() * r, y, cz + a.cos() * r]
    };
    for i in 0..sides {
        let quad = [
            at(i, r0, y0),
            at(i + 1, r0, y0),
            at(i + 1, r1, y1),
            at(i, r1, y1),
        ];
        let mut n = normalise(cross(sub(quad[1], quad[0]), sub(quad[3], quad[0])));
        let mid = (i as f32 + 0.5) / sides as f32 * TAU;
        if n[0] * mid.sin() + n[2] * mid.cos() < 0.0 {
            n = neg(n);
        }
        m.quad(quad, n, colour);
    }
}

/// A flat disc of radius `r` facing up at `y`, centred on `(cx, cz)`.
fn disc(m: &mut MeshBuf, [cx, cz]: [f32; 2], sides: usize, y: f32, r: f32, colour: [f32; 4]) {
    let rim: Vec<[f32; 3]> = (0..=sides)
        .map(|i| {
            let a = i as f32 / sides as f32 * TAU;
            [cx + a.sin() * r, y, cz + a.cos() * r]
        })
        .collect();
    m.fan([cx, y, cz], &rim, UP, colour);
}

/// A sawbench, its long side along X: a trestle table of planks, a round
/// blade standing up through a slot in the middle of the top, a crank on
/// the right end, a half-sawn log on the left, and sawdust heaped under
/// the blade. The blade's plane is the cut's, along X.
fn sawbench() -> MeshBuf {
    let mut m = MeshBuf::default();
    let (hx, hz, top) = (0.9, 0.4, 0.78);
    let wood = hex(WOOD);
    let light = mix(hex(WOOD), hex(WOOD_LIGHT), 0.5);
    let dark = hex(WOOD_DARK);
    // The top in two halves, either side of the blade's slot.
    cuboid(&mut m, [-hx, top, -hz], [hx, top + 0.06, -0.02], wood);
    cuboid(&mut m, [-hx, top, 0.02], [hx, top + 0.06, hz], light);
    // Splayed trestle legs at each end, braced, and a stretcher between.
    let leg = 0.05;
    for sx in [-1.0, 1.0] {
        let x = sx * (hx - 0.15);
        for sz in [-1.0, 1.0] {
            let z = sz * (hz - 0.07);
            cuboid(
                &mut m,
                [x - leg, 0.0, z - leg],
                [x + leg, top, z + leg],
                dark,
            );
        }
        cuboid(
            &mut m,
            [x - 0.04, 0.3, -hz + 0.05],
            [x + 0.04, 0.36, hz - 0.05],
            dark,
        );
    }
    cuboid(
        &mut m,
        [-hx + 0.15, 0.3, -0.04],
        [hx - 0.15, 0.36, 0.04],
        mix(dark, wood, 0.4),
    );
    // A housing under the slot, where the arbor turns.
    cuboid(
        &mut m,
        [-0.32, top - 0.2, -0.1],
        [0.32, top, 0.1],
        mix(dark, hex(IRON), 0.3),
    );

    // The blade: a toothed disc standing in the slot, both faces, with a
    // darker hub and a gold arbor nut.
    let (bx, by, r) = (0.05, top - 0.04, 0.3);
    const TEETH: usize = 40;
    let rim = |z: f32| {
        (0..=TEETH * 2)
            .map(|i| {
                let a = i as f32 / (TEETH * 2) as f32 * TAU;
                let rr = if i % 2 == 0 { r } else { r - 0.025 };
                [bx + a.cos() * rr, by + a.sin() * rr, z]
            })
            .collect::<Vec<_>>()
    };
    let steel = hex(STEEL);
    for (z, nz) in [(0.006, 1.0), (-0.006, -1.0)] {
        m.fan([bx, by, z], &rim(z), [0.0, 0.0, nz], steel);
        let hub: Vec<[f32; 3]> = (0..=12)
            .map(|i| {
                let a = i as f32 / 12.0 * TAU;
                [bx + a.cos() * 0.1, by + a.sin() * 0.1, z + 0.002 * nz]
            })
            .collect();
        m.fan(
            [bx, by, z + 0.002 * nz],
            &hub,
            [0.0, 0.0, nz],
            mix(steel, hex(IRON), 0.5),
        );
    }
    cuboid(
        &mut m,
        [bx - 0.03, by - 0.03, -0.03],
        [bx + 0.03, by + 0.03, 0.03],
        hex(GOLD),
    );

    // A crank on the right end: a spindle out of the housing's end, an
    // arm and a handle.
    let iron = hex(IRON);
    cuboid(
        &mut m,
        [0.32, by - 0.02, -0.02],
        [hx + 0.12, by + 0.02, 0.02],
        iron,
    );
    cuboid(
        &mut m,
        [hx + 0.12, by - 0.02, -0.02],
        [hx + 0.16, by + 0.02, 0.24],
        iron,
    );
    cuboid(
        &mut m,
        [hx + 0.12, by - 0.12, 0.2],
        [hx + 0.2, by + 0.0, 0.26],
        dark,
    );

    // A log on the left, half through: bark round, pale ends.
    let (lx0, lx1) = (-hx + 0.05, -0.32);
    let (ly, lr) = (top + 0.06 + 0.13, 0.13);
    let bark = mix(hex(WOOD_DARK), hex(0x3a2a1a), 0.3);
    const SIDES: usize = 10;
    let at = |i: usize, x: f32| {
        let a = i as f32 / SIDES as f32 * TAU;
        [x, ly + a.sin() * lr, a.cos() * lr]
    };
    for i in 0..SIDES {
        let mid = (i as f32 + 0.5) / SIDES as f32 * TAU;
        m.quad(
            [at(i, lx0), at(i + 1, lx0), at(i + 1, lx1), at(i, lx1)],
            [0.0, mid.sin(), mid.cos()],
            if i % 2 == 0 {
                bark
            } else {
                mix(bark, wood, 0.25)
            },
        );
    }
    for (x, nx) in [(lx0, -1.0), (lx1, 1.0)] {
        let rim: Vec<[f32; 3]> = (0..=SIDES).map(|i| at(i, x)).collect();
        m.fan([x, ly, 0.0], &rim, [nx, 0.0, 0.0], hex(SAWDUST));
    }
    // Two planks already cut, stacked on the right of the top.
    cuboid(
        &mut m,
        [0.38, top + 0.06, -0.3],
        [0.85, top + 0.1, -0.06],
        hex(WOOD_LIGHT),
    );
    cuboid(
        &mut m,
        [0.42, top + 0.1, -0.28],
        [0.83, top + 0.14, -0.08],
        light,
    );
    // Sawdust heaped on the ground under the blade, and a drift of it.
    band(
        &mut m,
        [0.05, 0.12],
        10,
        0.0,
        0.34,
        0.09,
        0.12,
        hex(SAWDUST),
    );
    disc(&mut m, [0.05, 0.12], 10, 0.09, 0.12, hex(SAWDUST));
    // A lantern on the far end of the top, dark until it works on its own.
    lantern(&mut m, [hx - 0.14, top + 0.06, -hz + 0.12], 0.8);
    m
}

/// A beehive kiln: a domed brick oven on a stone plinth, in courses of
/// two reds, with an arched firebox standing out at the front (+Z), its
/// mouth glowing, a vent at the crown glowing too, and firewood stacked
/// at its side.
fn kiln() -> MeshBuf {
    let mut m = MeshBuf::default();
    let o = [0.0, -0.1];
    const SIDES: usize = 16;
    let brick = hex(BRICK);
    let light = hex(BRICK_LIGHT);
    let dark = hex(BRICK_DARK);
    let stone = mix(hex(STONE), hex(0x6a6a76), 0.3);
    // The plinth, and its top.
    band(&mut m, o, SIDES, 0.0, 0.92, 0.16, 0.88, stone);
    disc(&mut m, o, SIDES, 0.16, 0.88, stone);
    // The wall, straight to the spring of the dome.
    let (r, spring) = (0.76, 0.6);
    band(&mut m, o, SIDES, 0.16, r, 0.38, r, brick);
    band(&mut m, o, SIDES, 0.38, r, spring, r, light);
    // The dome in courses, to a ring round the vent.
    const COURSES: usize = 6;
    let (crown, vent) = (1.5, 0.14);
    let course = |k: usize| {
        let a = k as f32 / COURSES as f32 * (PI / 2.0);
        let rr = vent + (r - vent) * a.cos();
        (spring + (crown - spring) * a.sin(), rr)
    };
    for k in 0..COURSES {
        let ((y0, r0), (y1, r1)) = (course(k), course(k + 1));
        let c = if k % 2 == 0 { brick } else { light };
        band(&mut m, o, SIDES, y0, r0, y1, r1, c);
    }
    // The vent's collar, standing proud of the crown.
    band(
        &mut m,
        o,
        10,
        crown - 0.02,
        vent + 0.04,
        crown + 0.12,
        vent + 0.03,
        dark,
    );
    // A band of gold-washed iron round the wall, where a hoop holds it.
    band(
        &mut m,
        o,
        SIDES,
        0.36,
        r + 0.012,
        0.42,
        r + 0.012,
        hex(GOLD),
    );

    // The firebox: a brick porch standing out of the front, its arch
    // dark, then the glow, then the white-hot heart.
    let (fz0, fz1) = (0.5, 0.92);
    cuboid(
        &mut m,
        [-0.36, 0.16, o[1] + fz0],
        [0.36, 0.78, o[1] + fz1],
        dark,
    );
    cuboid(
        &mut m,
        [-0.4, 0.78, o[1] + fz0 - 0.02],
        [0.4, 0.85, o[1] + fz1 + 0.03],
        brick,
    );
    let face = o[1] + fz1;
    let arch = |half: f32, base: f32, rise: f32, proud: f32| {
        let mut pts = vec![[-half, base], [half, base]];
        const STEPS: usize = 10;
        for i in 0..=STEPS {
            let a = i as f32 / STEPS as f32 * PI;
            pts.push([half * a.cos(), base + rise + half * a.sin()]);
        }
        pts.push([-half, base]);
        pts.into_iter()
            .map(|[x, y]| [x, y, face + proud])
            .collect::<Vec<_>>()
    };
    let front = [0.0, 0.0, 1.0];
    m.fan(
        [0.0, 0.42, face + 0.01],
        &arch(0.25, 0.2, 0.18, 0.01),
        front,
        mix(dark, hex(0x000000), 0.5),
    );
    let mut lit = MeshBuf::default();
    lit.fan(
        [0.0, 0.4, face + 0.018],
        &arch(0.19, 0.22, 0.14, 0.018),
        front,
        glow(hex(FIRE), 1.2),
    );
    lit.fan(
        [0.0, 0.3, face + 0.026],
        &arch(0.1, 0.23, 0.02, 0.026),
        front,
        glow(hex(FLAME), 3.2),
    );
    // The vent at the crown, hot: the kiln shows it's firing from behind.
    let vent_rim: Vec<[f32; 3]> = (0..=10)
        .map(|i| {
            let a = i as f32 / 10.0 * TAU;
            [
                o[0] + a.sin() * (vent + 0.03),
                crown + 0.121,
                o[1] + a.cos() * (vent + 0.03),
            ]
        })
        .collect();
    lit.fan(
        [o[0], crown + 0.121, o[1]],
        &vent_rim,
        UP,
        glow(mix(hex(FIRE), hex(FLAME), 0.4), 2.8),
    );
    m.append(lit);

    // Firewood stacked against the right side: split logs, end on.
    for (i, (y, z)) in [
        (0.0, 0.0),
        (0.0, 0.17),
        (0.0, 0.34),
        (0.15, 0.085),
        (0.15, 0.255),
    ]
    .into_iter()
    .enumerate()
    {
        let c = if i % 2 == 0 {
            hex(WOOD)
        } else {
            hex(WOOD_DARK)
        };
        cuboid(&mut m, [0.94, y, -0.35 + z], [1.28, y + 0.14, -0.21 + z], c);
        cuboid(
            &mut m,
            [1.28, y + 0.02, -0.33 + z],
            [1.285, y + 0.12, -0.23 + z],
            hex(SAWDUST),
        );
    }
    m
}

/// An anvil on a stump: a wide round of wood, pale on its cut face, the
/// anvil on it with its horn to +X and its heel to -X, a hammer laid
/// across the face, and a hooped quench bucket beside.
fn anvil() -> MeshBuf {
    let mut m = MeshBuf::default();
    let bark = mix(hex(WOOD_DARK), hex(0x3a2a1a), 0.3);
    // The stump, flaring at its foot, and its end grain with a ring.
    band(&mut m, [0.0, 0.0], 12, 0.0, 0.36, 0.08, 0.31, bark);
    band(
        &mut m,
        [0.0, 0.0],
        12,
        0.08,
        0.31,
        0.5,
        0.29,
        hex(WOOD_DARK),
    );
    disc(&mut m, [0.0, 0.0], 12, 0.5, 0.29, hex(SAWDUST));
    disc(
        &mut m,
        [0.0, 0.0],
        12,
        0.501,
        0.17,
        mix(hex(SAWDUST), hex(WOOD), 0.35),
    );
    // The anvil: a foot, a waist, the body and the face.
    let iron = hex(IRON);
    let face = mix(iron, hex(STEEL), 0.55);
    let y = 0.502;
    cuboid(&mut m, [-0.22, y, -0.15], [0.22, y + 0.07, 0.15], iron);
    cuboid(
        &mut m,
        [-0.12, y + 0.07, -0.08],
        [0.12, y + 0.18, 0.08],
        iron,
    );
    cuboid(&mut m, [-0.3, y + 0.18, -0.11], [0.24, y + 0.3, 0.11], iron);
    cuboid(&mut m, [-0.3, y + 0.3, -0.11], [0.24, y + 0.32, 0.11], face);
    // The hardy hole, in gold for the Capitol's mark.
    cuboid(
        &mut m,
        [-0.25, y + 0.32, -0.025],
        [-0.2, y + 0.325, 0.025],
        hex(GOLD),
    );
    // The horn: a cone out to +X, flat on top with the face.
    let (hx0, hx1) = (0.24, 0.52);
    let horn = [
        [hx0, y + 0.2, -0.1],
        [hx0, y + 0.32, -0.1],
        [hx0, y + 0.32, 0.1],
        [hx0, y + 0.2, 0.1],
    ];
    let tip = [hx1, y + 0.3, 0.0];
    for i in 0..4 {
        let (a, b) = (horn[i], horn[(i + 1) % 4]);
        let mut n = normalise(cross(sub(b, a), sub(tip, a)));
        let mid = [
            (a[0] + b[0]) * 0.5,
            (a[1] + b[1]) * 0.5,
            (a[2] + b[2]) * 0.5,
        ];
        let out = sub(mid, [hx0, y + 0.26, 0.0]);
        if n[0] * out[0] + n[1] * out[1] + n[2] * out[2] < 0.0 {
            n = neg(n);
        }
        m.fan(a, &[b, tip], n, if i == 1 { face } else { iron });
    }
    // A hammer across the face: haft and head.
    let top = y + 0.32;
    cuboid(
        &mut m,
        [-0.14, top, 0.02],
        [0.2, top + 0.03, 0.05],
        hex(WOOD_LIGHT),
    );
    cuboid(&mut m, [-0.2, top, -0.04], [-0.12, top + 0.07, 0.11], iron);
    // A quench bucket to the left: staves, two hoops, dark water.
    let b = [-0.6, 0.18];
    band(&mut m, b, 12, 0.0, 0.17, 0.36, 0.2, hex(WOOD));
    band(&mut m, b, 12, 0.06, 0.177, 0.1, 0.182, iron);
    band(&mut m, b, 12, 0.27, 0.193, 0.31, 0.197, iron);
    disc(&mut m, b, 12, 0.3, 0.19, hex(0x1e3a4a));
    // A lantern on the ground by the stump, dark until it works on its own.
    lantern(&mut m, [0.42, 0.0, 0.22], 1.0);
    m
}

/// A plank crate, for a type the client doesn't know: a box of boards,
/// framed at every edge, with a gold stencil band, so an unknown station
/// still looks like something built rather than a hole in the world.
fn generic() -> MeshBuf {
    let mut m = MeshBuf::default();
    let h = 0.5;
    let crate_ = hex(CRATE);
    cuboid(&mut m, [-h, 0.0, -h], [h, 2.0 * h, h], crate_);
    // The boards' seams, on each side.
    let seam = mix(crate_, hex(WOOD_DARK), 0.6);
    for y in [0.33, 0.66] {
        for (lo, hi) in [
            ([-h, y - 0.008, h], [h, y + 0.008, h + 0.004]),
            ([-h, y - 0.008, -h - 0.004], [h, y + 0.008, -h]),
            ([h, y - 0.008, -h], [h + 0.004, y + 0.008, h]),
            ([-h - 0.004, y - 0.008, -h], [-h, y + 0.008, h]),
        ] {
            cuboid(&mut m, lo, hi, seam);
        }
    }
    // The frame, on every edge.
    let f = 0.07;
    let frame = hex(WOOD_DARK);
    let e = h + 0.012;
    for sx in [-1.0, 1.0] {
        for sz in [-1.0, 1.0] {
            let (x, z) = (sx * (e - f / 2.0), sz * (e - f / 2.0));
            cuboid(
                &mut m,
                [x - f / 2.0, 0.0, z - f / 2.0],
                [x + f / 2.0, 2.0 * h + 0.012, z + f / 2.0],
                frame,
            );
        }
    }
    for y in [0.0, 2.0 * h - f + 0.012] {
        cuboid(&mut m, [-e, y, e - f], [e, y + f, e], frame);
        cuboid(&mut m, [-e, y, -e], [e, y + f, -e + f], frame);
        cuboid(&mut m, [e - f, y, -e], [e, y + f, e], frame);
        cuboid(&mut m, [-e, y, -e], [-e + f, y + f, e], frame);
    }
    // A gold band stencilled on the front, where a merchant's mark goes.
    cuboid(
        &mut m,
        [-0.2, 0.44, h + 0.012],
        [0.2, 0.56, h + 0.016],
        hex(GOLD),
    );
    // A lantern on the ground at its corner, dark until it works on its own.
    lantern(&mut m, [h + 0.14, 0.0, h - 0.12], 1.0);
    m
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(u: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

fn neg([x, y, z]: [f32; 3]) -> [f32; 3] {
    [-x, -y, -z]
}

fn normalise([x, y, z]: [f32; 3]) -> [f32; 3] {
    let len = (x * x + y * y + z * z).sqrt().max(1e-6);
    [x / len, y / len, z / len]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::mesh::is_light;

    fn bounds(m: &MeshBuf) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in &m.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v.position[k]);
                hi[k] = hi[k].max(v.position[k]);
            }
        }
        (lo, hi)
    }

    #[test]
    fn each_model_is_about_reacts_twenty_units_and_stands_on_the_ground() {
        for (name, m, tall) in [
            ("workbench", workbench(), 0.9..1.2),
            ("forge", forge(), 2.2..2.6),
            ("storage_chest", chest(), 0.55..0.8),
            ("sawbench", sawbench(), 0.95..1.2),
            ("kiln", kiln(), 1.5..1.8),
            ("anvil", anvil(), 0.8..1.0),
            ("generic", generic(), 0.9..1.1),
        ] {
            let (lo, hi) = bounds(&m);
            assert!(lo[1].abs() < 1e-4, "{name} stands at {}", lo[1]);
            assert!(tall.contains(&hi[1]), "{name} {} m tall", hi[1]);
            let across = (hi[0] - lo[0]).max(hi[2] - lo[2]);
            assert!((1.0..3.2).contains(&across), "{name} {across} m across");
            let reach = m
                .vertices
                .iter()
                .map(|v| v.position[0].hypot(v.position[2]))
                .fold(0.0, f32::max);
            let ring = footprint(kind_of(name));
            assert!(
                reach < ring && reach > ring - 0.5,
                "{name}'s ring ({ring} m) sits round it ({reach} m)"
            );
        }
    }

    #[test]
    fn every_crafting_station_has_a_light_and_the_chest_none() {
        let glows = |m: &MeshBuf| m.vertices.iter().any(is_light);
        for (kind, m) in [
            ("workbench", workbench()),
            ("forge", forge()),
            ("storage_chest", chest()),
            ("sawbench", sawbench()),
            ("kiln", kiln()),
            ("anvil", anvil()),
            ("generic", generic()),
        ] {
            assert_eq!(glows(&m), kind != "storage_chest", "{kind}");
        }
    }

    #[test]
    fn work_lights_a_station_and_a_wait_dims_it() {
        for kind in ["forge", "kiln", "workbench", "sawbench", "anvil", "generic"] {
            let idle = work_light(kind, Work::Idle, 0.0);
            let start = work_light(kind, Work::Running, 0.0);
            let due = work_light(kind, Work::Running, 1.0);
            let paused = work_light(kind, Work::Paused, 0.0);
            assert!(start > idle, "{kind}: running is brighter than idle");
            assert!(due > start, "{kind}: it swells as the craft comes due");
            assert!(paused < start, "{kind}: a wait dims it");
            if fired(kind) {
                assert!(paused < idle, "{kind}: a waiting fire sinks below a banked one");
            } else {
                assert_eq!(idle, 0.0, "{kind}: an idle lantern is dark");
            }
        }
    }

    #[test]
    fn fires_burn_brighter_at_night() {
        use crate::daylight::PlotGlow;
        let day = PlotGlow {
            surface: 1.0,
            radiance: 0.0,
        };
        let night = PlotGlow {
            surface: 0.0,
            radiance: 1.6,
        };
        assert_eq!(fire_light(day), 1.0);
        assert!(fire_light(PlotGlow::EVENING) > 1.0);
        assert!(fire_light(night) > fire_light(PlotGlow::EVENING));
    }

    #[test]
    fn the_fires_hearts_are_bright_enough_to_bloom() {
        // The scene's bloom threshold is 2.5.
        for m in [forge(), kiln()] {
            let brightest = m
                .vertices
                .iter()
                .filter(|v| is_light(v))
                .map(|v| {
                    let [r, g, b, _] = v.color;
                    r.max(g).max(b)
                })
                .fold(0.0, f32::max);
            assert!(brightest > 2.5, "{brightest}");
        }
    }

    #[test]
    fn every_face_faces_its_normal() {
        for m in [
            workbench(),
            forge(),
            chest(),
            sawbench(),
            kiln(),
            anvil(),
            generic(),
        ] {
            for t in m.indices.as_chunks::<3>().0 {
                let v = [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]);
                assert!(crate::world::mesh::facing(
                    v[0].position,
                    v[1].position,
                    v[2].position,
                    v[0].normal
                ));
            }
        }
    }

    #[test]
    fn a_station_faces_away_from_the_capitol() {
        for at in [[0.0, 300.0], [-250.0, 10.0], [120.0, -400.0]] {
            let yaw = facing(at, Uuid::from_u128(500));
            // The model's front, +Z, turned by the yaw about +Y.
            let front = [yaw.sin(), yaw.cos()];
            let out = {
                let l = (at[0] * at[0] + at[1] * at[1]).sqrt();
                [at[0] / l, at[1] / l]
            };
            let dot = front[0] * out[0] + front[1] * out[1];
            assert!(dot > 0.999, "{at:?}: {dot}");
        }
        let a = facing([0.0, 300.0], Uuid::from_u128(0));
        let b = facing([0.0, 300.0], Uuid::from_u128(999));
        assert!((a - b).abs() > 20f32.to_radians(), "jitter spreads them");
    }

    #[test]
    fn every_catalog_station_has_its_own_model_and_unknown_ones_a_crate() {
        let catalog = capitol_core::catalog::Catalog::fallback();
        for t in catalog.station_types() {
            assert_ne!(mesh(kind_of(&t.id)), MESH_GENERIC, "{} has no model", t.id);
        }
        assert_eq!(mesh(kind_of("loom")), MESH_GENERIC);
    }

    #[test]
    fn a_placed_station_lands_and_settles() {
        let (lift, _) = drop(0.0);
        assert!((lift - DROP_FROM).abs() < 1e-5);
        let (lift, [a, u]) = drop(1.0);
        assert_eq!(lift, 0.0);
        assert!((a - 1.0).abs() < 1e-3 && (u - 1.0).abs() < 1e-3);
        // Squashed just after it lands.
        let (_, [_, u]) = drop(0.46);
        assert!(u < 0.9, "{u}");
    }

    #[test]
    fn the_kilns_smoke_loads() {
        let text = include_str!("../../../particles/kiln.particles.toml");
        let fx = flint_particles::load_effect_from_str(text, "kiln").expect("parses");
        assert_eq!(fx.name, KILN_EFFECT);
        let names: Vec<_> = fx.emitters.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sparks", "smoke"]);
    }

    #[test]
    fn the_forges_fire_loads() {
        let text = include_str!("../../../particles/forge.particles.toml");
        let fx = flint_particles::load_effect_from_str(text, "forge").expect("parses");
        assert_eq!(fx.name, FORGE_EFFECT);
        let names: Vec<_> = fx.emitters.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["flames", "embers", "smoke"]);
    }

    #[test]
    fn the_crafts_puffs_load() {
        for (text, name, emitters) in [
            (
                include_str!("../../../particles/craft_sparks.particles.toml"),
                SPARKS_EFFECT,
                ["sparks", "breath"],
            ),
            (
                include_str!("../../../particles/craft_wisp.particles.toml"),
                WISP_EFFECT,
                ["dust", "wisp"],
            ),
        ] {
            let fx = flint_particles::load_effect_from_str(text, name).expect("parses");
            assert_eq!(fx.name, name);
            let names: Vec<_> = fx.emitters.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, emitters);
        }
    }
}
