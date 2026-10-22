//! The players (P4.2): one avatar entity per player, placed each frame
//! from `capitol_core::motion`'s poses. This is the hot path, so it's Rust,
//! not Rhai (plan §4.2). The names over their heads are drawn by
//! `scripts/hud.rhai` from what [`Players::update`] leaves in `Shared`.
//!
//! An avatar is a small cloaked figure carrying a lantern, which glows at
//! dusk and shows which way they face. Other players wear the React
//! client's green (`#4a7c59`, edged `#2d4e36`). You wear the Capitol's
//! gold, edged in the white React outlines you with, and stand in a gold
//! ring: gold means you. The figure faces where it walks, eased so a turn
//! is a turn and not a snap, and bobs a little with each stride.
//!
//! A player who joins fades in; one who leaves fades out where they last
//! stood, and only then is despawned. The fade is the entity's
//! `material.opacity`.
//!
//! A citizen (C0.2, Citizens D3) wears the same green but carries another
//! light. A person's lantern is an amber candle that gutters a little
//! ([`flicker`], through `material.emissive_scale`); a citizen's burns pale
//! and steady, like an oil lamp turned low, and a small lozenge stands by
//! their name (`labels.rs`). Both are there to be seen if you look, and
//! quiet enough that the city still feels lived in.

use crate::coords::{layer, to_world};
use crate::script_api::{PlayerLabel, Shared};
use crate::ui::crafting::Fill;
use crate::world::bridges::Decks;
use crate::world::land;
use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{glow, hex, mix};
use capitol_core::motion::Pose;
use capitol_core::protocol::terrain::{RoadSegment, RoadType, WaterRing};
use capitol_core::state::{Activity, GameState};
use flint_core::EntityId;
use flint_player::{ExtensionCtx, ExtensionGpu};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use uuid::Uuid;

/// The runtime meshes: everyone else, a citizen, and you. Your gold ring
/// is a mesh of its own, so your lantern's flicker leaves it be.
pub const MESH_OTHER: &str = "avatar";
pub const MESH_CITIZEN: &str = "avatar_citizen";
pub const MESH_SELF: &str = "avatar_self";
pub const MESH_RING: &str = "avatar_ring";

/// Where feet stand off a bridge: level with the arterials and the bailey,
/// a hair under the lots' pavement, so nobody floats over a road or sinks
/// into a lot.
fn stand() -> f32 {
    layer::road(RoadType::Arterial)
}

/// Seconds to fade in on joining, and out on leaving.
const FADE: f32 = 0.6;
/// How fast the figure turns to face its heading: 1/e in 1/k seconds.
const TURN_K: f32 = 12.0;
/// How fast the stride's bob comes and goes as a walk starts and stops.
const WALK_K: f32 = 8.0;
/// Server units per stride (one bob), and the bob's height in metres.
const STRIDE: f32 = 9.0;
const BOB: f32 = 0.035;
/// Where a name hangs: this far over the feet, in metres.
pub const LABEL_Y: f32 = 2.25;

struct Avatar {
    entity: EntityId,
    /// Your gold ring, for you alone.
    ring: Option<EntityId>,
    name: String,
    /// An AI player (C0.1): a steady lantern and a glyph by the name.
    citizen: bool,
    /// Where in its flicker this lantern starts, so a crowd's candles
    /// don't gutter together.
    seed: f32,
    /// 0 to 1: how far faded in.
    shown: f32,
    /// In this frame's poses. An avatar that isn't is fading out.
    present: bool,
    /// Where it was last placed, in world metres (feet).
    at: [f32; 3],
    /// The direction drawn, in world radians about +Y (see [`yaw_quat`]).
    yaw: f32,
    /// The stride's phase, in radians, and how much of a walk is showing.
    stride: f32,
    walk: f32,
    /// Server position last frame, to measure the stride.
    last: Option<(f64, f64)>,
    /// Their craft's progress as drawn, eased between updates (P5.5).
    craft: Fill,
}

#[derive(Default)]
pub struct Players {
    avatars: HashMap<Uuid, Avatar>,
    meshes: bool,
    decks: Decks,
    /// Counts spawns, so a returning player's entity never clashes by name
    /// with the one still fading out.
    spawned: u64,
    /// Seconds since the first frame: the candles' clock.
    time: f32,
}

impl Players {
    /// The water rings changed: the bridges' decks are where they are now.
    pub fn set_rings(&mut self, rings: &[WaterRing]) {
        self.decks = Decks::new(rings);
    }

    /// The land is in, with the trails' bridges on it (W10).
    pub fn set_trail_decks(&mut self, spans: Vec<crate::world::bridges::Span>) {
        self.decks.set_trails(spans);
    }

    /// Place every avatar for this frame. `poses` are this frame's, the
    /// local player's first. Writes the names to draw, and the point the
    /// camera follows, into `shared`.
    pub fn update(
        &mut self,
        ctx: &mut ExtensionCtx,
        poses: &[Pose],
        state: &GameState,
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
        let me = state.me();
        self.time += dt;
        for avatar in self.avatars.values_mut() {
            avatar.present = false;
        }
        for pose in poses {
            let mine = Some(pose.id) == me;
            let avatar = match self.avatars.get_mut(&pose.id) {
                Some(a) => a,
                None => {
                    self.spawned += 1;
                    let name = format!("player_{}_{}", pose.id.simple(), self.spawned);
                    // A player's kind never changes (C0.1), so it's read once.
                    let citizen = state.player(pose.id).is_some_and(|p| p.citizen);
                    let mesh = match (mine, citizen) {
                        (true, _) => MESH_SELF,
                        (false, true) => MESH_CITIZEN,
                        (false, false) => MESH_OTHER,
                    };
                    let entity = match ctx.world.spawn(name.as_str()) {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::warn!("players: no entity for {}: {e}", pose.id);
                            continue;
                        }
                    };
                    if let Err(e) = ctx.attach_mesh(entity, mesh) {
                        tracing::warn!("players: no mesh for {}: {e}", pose.id);
                    }
                    let ring = if mine {
                        ctx.world.spawn(format!("{name}_ring").as_str()).ok()
                    } else {
                        None
                    };
                    if let Some(ring) = ring {
                        if let Err(e) = ctx.attach_mesh(ring, MESH_RING) {
                            tracing::warn!("players: no ring for {}: {e}", pose.id);
                        }
                    }
                    self.avatars.entry(pose.id).or_insert(Avatar {
                        entity,
                        ring,
                        name: String::new(),
                        citizen,
                        seed: (pose.id.as_u128() % 9973) as f32 * 0.37,
                        shown: 0.0,
                        present: true,
                        at: [0.0; 3],
                        yaw: yaw_of(pose.facing),
                        stride: 0.0,
                        walk: 0.0,
                        last: None,
                        craft: Fill::default(),
                    })
                }
            };
            avatar.present = true;
            if let Some(p) = state.player(pose.id) {
                if avatar.name != p.name {
                    avatar.name.clone_from(&p.name);
                }
            }
            avatar.step(pose, &self.decks, dt);
            avatar.craft.update(state.activity(pose.id), dt);
        }

        let mut labels = shared.players.lock().ok();
        if let Some(labels) = labels.as_deref_mut() {
            labels.clear();
        }
        let world = &mut *ctx.world;
        let time = self.time;
        self.avatars.retain(|id, avatar| {
            let fade = dt / FADE;
            avatar.shown = if avatar.present {
                (avatar.shown + fade).min(1.0)
            } else {
                (avatar.shown - fade).max(0.0)
            };
            if !avatar.present && avatar.shown <= 0.0 {
                let _ = world.despawn(avatar.entity);
                if let Some(ring) = avatar.ring {
                    let _ = world.despawn(ring);
                }
                return false;
            }
            place(world, avatar, time);
            if let Some(labels) = labels.as_deref_mut() {
                let [x, y, z] = avatar.at;
                let crafting = matches!(state.activity(*id), Some(Activity::Crafting { .. }));
                labels.push(PlayerLabel {
                    id: *id,
                    name: avatar.name.clone(),
                    at: [x, y + LABEL_Y, z],
                    feet_y: y,
                    alpha: smooth(avatar.shown),
                    me: Some(*id) == me,
                    citizen: avatar.citizen,
                    craft: crafting.then_some(avatar.craft.shown),
                });
            }
            true
        });
        drop(labels);

        // The camera follows you: the avatar once it's placed, or before
        // the first `positions`, where `/player/me` says you are.
        let follow = me
            .and_then(|id| self.avatars.get(&id).filter(|a| a.present).map(|a| a.at))
            .or_else(|| {
                state.local_player().map(|p| {
                    let [x, z] = to_world(p.x, p.y);
                    let y = self.decks.top(p.x, p.y);
                    [x, y.unwrap_or_else(|| stand() + land::rise([x, z])), z]
                })
            });
        if let Ok(mut f) = shared.follow.lock() {
            *f = follow;
        }
    }
}

impl Avatar {
    fn step(&mut self, pose: &Pose, decks: &Decks, dt: f32) {
        let [x, z] = to_world(pose.x, pose.y);
        // On a bridge, its deck; on the land, the land.
        let y = decks
            .top(pose.x, pose.y)
            .unwrap_or_else(|| stand() + land::rise([x, z]));
        self.at = [x, y, z];

        let turn = wrap(yaw_of(pose.facing) - self.yaw);
        self.yaw = wrap(self.yaw + turn * (1.0 - (-TURN_K * dt).exp()));

        let walked = self
            .last
            .map_or(0.0, |(lx, ly)| (pose.x - lx).hypot(pose.y - ly) as f32);
        self.last = Some((pose.x, pose.y));
        // A teleport isn't a stride.
        if walked < 20.0 {
            self.stride = (self.stride + walked / STRIDE * PI) % TAU;
        }
        let target = if pose.moving { 1.0 } else { 0.0 };
        self.walk += (target - self.walk) * (1.0 - (-WALK_K * dt).exp());
    }
}

/// Write the avatar's transform, opacity and lantern onto its entity, and
/// its ring's onto the ring's.
fn place(world: &mut flint_ecs::FlintWorld, avatar: &Avatar, time: f32) {
    let [x, y, z] = avatar.at;
    let bob = BOB * avatar.stride.sin().abs() * avatar.walk;
    // Rising out of the ground a little as it fades in.
    let rise = (1.0 - smooth(avatar.shown)) * -0.25;
    let opacity = toml::Value::Float(smooth(avatar.shown) as f64);
    // A citizen's lamp is steady, so it's never written.
    if !avatar.citizen {
        let _ = world.set_field(
            avatar.entity,
            "material",
            "emissive_scale",
            toml::Value::Float(flicker(time, avatar.seed) as f64),
        );
    }
    if let Some(ring) = avatar.ring {
        // On the ground: the ring doesn't bob with the stride.
        let mut t = toml::Table::new();
        t.insert("position".into(), vec3(x, y + rise, z));
        let _ = world.set_component(ring, "transform", toml::Value::Table(t));
        let _ = world.set_field(ring, "material", "opacity", opacity.clone());
    }
    let mut t = toml::Table::new();
    t.insert("position".into(), vec3(x, y + bob + rise, z));
    let q = yaw_quat(avatar.yaw);
    t.insert(
        "rotation_quat".into(),
        toml::Value::Array(q.iter().map(|v| toml::Value::Float(*v as f64)).collect()),
    );
    let _ = world.set_component(avatar.entity, "transform", toml::Value::Table(t));
    let _ = world.set_field(avatar.entity, "material", "opacity", opacity);
}

/// A person's candle at `time` seconds: about 1, guttering a little. Three
/// unrelated waves, so it never visibly repeats, and now and then a dip
/// where a slow draught takes the flame.
pub fn flicker(time: f32, seed: f32) -> f32 {
    let t = time + seed;
    let waver =
        0.07 * (5.3 * t).sin() + 0.05 * (8.9 * t + 1.7).sin() + 0.03 * (15.1 * t + 0.4).sin();
    // Mostly nothing; now and then the two slow waves meet in a dip.
    let draught = ((0.71 * t).sin() * (0.29 * t + 2.0).sin()).max(0.0).powi(3) * 0.35;
    (1.0 + waver - draught).clamp(0.55, 1.2)
}

fn vec3(x: f32, y: f32, z: f32) -> toml::Value {
    toml::Value::Array(vec![
        toml::Value::Float(x as f64),
        toml::Value::Float(y as f64),
        toml::Value::Float(z as f64),
    ])
}

/// The yaw that turns the mesh's front (+X) toward a server heading. Server
/// headings are 0 east and π/2 south; south is world +Z, and a turn of θ
/// about +Y takes +X to (cos θ, 0, -sin θ), so θ is minus the heading.
fn yaw_of(facing: f64) -> f32 {
    wrap(-(facing as f32))
}

/// A turn of `yaw` radians about +Y, as `[x, y, z, w]`.
fn yaw_quat(yaw: f32) -> [f32; 4] {
    let h = yaw * 0.5;
    [0.0, h.sin(), 0.0, h.cos()]
}

/// An angle brought into (-π, π].
fn wrap(a: f32) -> f32 {
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI {
        w + TAU
    } else {
        w
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The street a stroll keeps to (#349), in world metres: a point on its
/// centreline, the unit way along it, and how far either side of the
/// centre a walker's middle may go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Street {
    pub centre: [f32; 2],
    pub along: [f32; 2],
    pub half: f32,
}

impl Street {
    /// How far a walker keeps from a street's edge: a figure's half width
    /// and a little air, so nobody brushes a stall
    const MARGIN: f32 = 0.5;

    /// The street under `at`: the nearest road segment whose width covers
    /// it, with `at` brought onto its centreline. `None` off every road.
    pub fn at(roads: &[RoadSegment], at: [f32; 2]) -> Option<Street> {
        let mut best: Option<(f32, Street)> = None;
        for road in roads {
            let half = road.width as f32 * crate::coords::S / 2.0;
            for w in road.points.windows(2) {
                let (a, b) = (to_world(w[0].0, w[0].1), to_world(w[1].0, w[1].1));
                let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                if len < 1e-3 {
                    continue;
                }
                let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
                let s = ((at[0] - a[0]) * u[0] + (at[1] - a[1]) * u[1]).clamp(0.0, len);
                let p = [a[0] + u[0] * s, a[1] + u[1] * s];
                let off = (at[0] - p[0]).hypot(at[1] - p[1]);
                if off <= half && best.is_none_or(|(d, _)| off < d) {
                    let street = Street { centre: p, along: u, half: (half - Self::MARGIN).max(0.0) };
                    best = Some((off, street));
                }
            }
        }
        best.map(|(_, s)| s)
    }
}

/// `--citizen-sample stroll` (#346): people and citizens walking up and
/// down the way the camera looks, `t` seconds after they set off, as if
/// along a street through where it looks. Each keeps a lane of its own
/// across the view, and one lingers. Mostly citizens, since they're what
/// it's for. Over ten seconds they all stay within fourteen metres of
/// where it looks; nobody wraps round, so a walker who leaves stays out.
/// On a `street` they walk along it instead, its lanes squeezed to fit
/// inside it, so nobody walks through what lines it (#349).
pub fn stroll(target: [f32; 2], camera: [f32; 2], street: Option<Street>, t: f32) -> (GameState, Vec<Pose>) {
    use capitol_core::net::ConnectionState;
    use capitol_core::protocol::ws::{PlayerPosition, ServerMessage};
    // Name, citizen, lane across the view (m, + to the right, for a street
    // LANES either side), where along it they start (m past where the
    // camera looks) and their pace along it (m/s, negative toward the camera)
    const LANES: f32 = 2.0;
    const WALKERS: [(&str, bool, f32, f32, f32); 7] = [
        ("Corin", true, -1.6, -3.0, 1.1),
        ("Elsbeth", true, 1.3, 9.0, -1.0),
        ("Wren Ashby", false, -0.5, 1.0, 0.9),
        ("Oswin", true, 1.9, -6.0, 1.2),
        ("Juno", true, 0.6, 13.0, -1.1),
        ("Tam Hollis", false, -2.0, 6.0, -0.8),
        ("Hale", true, 0.2, 3.0, 0.0),
    ];
    let mut state = GameState::new();
    state.apply_connection(&ConnectionState::Connected { player_id: Uuid::from_u128(0xc0_03_ff) });
    let (dx, dz) = (target[0] - camera[0], target[1] - camera[1]);
    let len = dx.hypot(dz).max(1e-3);
    let (origin, away, squeeze) = match street {
        // Along the street, whichever way runs away from the camera
        Some(s) => {
            let sign = if s.along[0] * dx + s.along[1] * dz < 0.0 { -1.0 } else { 1.0 };
            (s.centre, [s.along[0] * sign, s.along[1] * sign], (s.half / LANES).min(1.0))
        }
        None => (target, [dx / len, dz / len], 1.0),
    };
    let right = [-away[1], away[0]];
    let mut poses = Vec::new();
    for (i, (name, citizen, lane, from, pace)) in WALKERS.into_iter().enumerate() {
        let along = from + pace * t;
        let lane = lane * squeeze;
        let [x, y] = crate::coords::to_server(
            origin[0] + away[0] * along + right[0] * lane,
            origin[1] + away[1] * along + right[1] * lane,
        );
        let id = Uuid::from_u128(0xc0_03_00 + i as u128);
        state.apply_ws(&ServerMessage::PlayerJoined {
            player: PlayerPosition { id, name: name.into(), x, y, citizen },
        });
        // The server's heading: east 0, south (+Z) π/2. The one who lingers
        // looks across the street.
        let facing = match pace {
            p if p > 0.0 => f64::from(away[1]).atan2(f64::from(away[0])),
            p if p < 0.0 => f64::from(-away[1]).atan2(f64::from(-away[0])),
            _ => f64::from(right[1]).atan2(f64::from(right[0])),
        };
        poses.push(Pose { id, x, y, facing, moving: pace != 0.0 });
    }
    (state, poses)
}

/// `--citizen-sample` (C0.2): people and citizens side by side, in a row
/// across the view where the camera looks, turned a little toward it,
/// with you in the middle, for captures of the mark.
pub fn sample(target: [f32; 2], camera: [f32; 2]) -> (GameState, Vec<Pose>) {
    use capitol_core::net::ConnectionState;
    use capitol_core::protocol::ws::{PlayerPosition, ServerMessage};
    const ROW: [(&str, bool); 5] = [
        ("Wren Ashby", false),
        ("Corin", true),
        ("You", false),
        ("Tam Hollis", false),
        ("Elsbeth", true),
    ];
    const SPACING: f32 = 1.6;
    let me = Uuid::from_u128(0xc0_02_00 + 2);
    let mut state = GameState::new();
    state.apply_connection(&ConnectionState::Connected { player_id: me });
    let mut poses = Vec::new();
    let (dx, dz) = (camera[0] - target[0], camera[1] - target[1]);
    let len = dx.hypot(dz).max(1e-3);
    let across = [dz / len, -dx / len];
    // The server's heading toward the camera: east 0, south (+Z) π/2.
    let toward = f64::from(dz).atan2(f64::from(dx));
    let mid = (ROW.len() as f32 - 1.0) / 2.0;
    for (i, (name, citizen)) in ROW.into_iter().enumerate() {
        let k = (i as f32 - mid) * SPACING;
        let [x, y] = crate::coords::to_server(target[0] + across[0] * k, target[1] + across[1] * k);
        let id = Uuid::from_u128(0xc0_02_00 + i as u128);
        state.apply_ws(&ServerMessage::PlayerJoined {
            player: PlayerPosition {
                id,
                name: name.into(),
                x,
                y,
                citizen,
            },
        });
        // Each turned a little toward the middle of the row.
        let facing = toward + f64::from(k) * 0.25;
        poses.push(Pose {
            id,
            x,
            y,
            facing,
            moving: false,
        });
    }
    (state, poses)
}

// ── The figure ───────────────────────────────────────────────────────────

/// The colours of one figure.
struct Livery {
    cloak: u32,
    trim: u32,
    /// The lantern's light, and how bright it burns.
    lantern: u32,
    radiance: f32,
}

const OTHER: Livery = Livery {
    cloak: 0x4a7c59,
    trim: 0x2d4e36,
    lantern: LANTERN,
    radiance: 1.1,
};
/// A citizen: the same cloak as anyone else's, and a cooler, steadier lamp.
const CITIZEN: Livery = Livery {
    lantern: CITIZEN_LANTERN,
    ..OTHER
};
const SELF: Livery = Livery {
    cloak: 0xc9a227,
    trim: 0xf2ead2,
    ..OTHER
};

/// A person's lantern: warm, like the windows of the Capitol at dusk.
pub const LANTERN: u32 = 0xffa94a;
/// A citizen's: as strong, but the pale of lamp oil a little toward the
/// moon, so beside the amber it reads as another flame and not a cold one.
/// (A neutral pale, #e8e4d4, read as paper at dusk, not light.) The glyph
/// by their name (`hud.rhai`) is the same colour.
pub const CITIZEN_LANTERN: u32 = 0xd6e4ea;

fn upload(gpu: &mut ExtensionGpu) {
    figure(&OTHER).upload(gpu, MESH_OTHER, 0.85);
    figure(&CITIZEN).upload(gpu, MESH_CITIZEN, 0.85);
    figure(&SELF).upload(gpu, MESH_SELF, 0.6);
    ring().upload(gpu, MESH_RING, 0.6);
}

/// Sides round the cloak and the head.
const SIDES: usize = 10;

/// One figure, feet at the origin, facing +X. About 1.75 m tall and 0.9 m
/// across the hem, a player's 8 server units of radius.
fn figure(livery: &Livery) -> MeshBuf {
    let cloak = hex(livery.cloak);
    let trim = hex(livery.trim);
    let hood = mix(cloak, hex(0x0a0a0f), 0.35);
    let mut m = MeshBuf::default();

    // The cloak: a hem band, then a long taper to the shoulders.
    let rings = [(0.0, 0.45), (0.14, 0.43), (1.22, 0.2)];
    for (k, w) in rings.windows(2).enumerate() {
        let colour = if k == 0 { trim } else { cloak };
        frustum(&mut m, w[0], w[1], colour);
    }
    // Shoulders: a shallow cone up to the neck.
    let rim: Vec<[f32; 3]> = (0..=SIDES).map(|i| round(i, 0.2, 1.22)).collect();
    m.fan([0.0, 1.3, 0.0], &rim, UP, cloak);

    // The head, hooded: a low-poly ball.
    sphere(&mut m, [0.0, 1.5, 0.0], 0.2, hood);

    // The lantern, held out front at the hip, on a dark bail.
    let at = [0.42, 0.8, 0.14];
    cuboid(
        &mut m,
        [at[0], at[1] + 0.15, at[2]],
        [0.015, 0.06, 0.015],
        hood,
    );
    let mut light = MeshBuf::default();
    cuboid(
        &mut light,
        at,
        [0.075, 0.095, 0.075],
        glow(hex(livery.lantern), livery.radiance),
    );
    m.append(light);
    m
}

/// The gold ring you stand in, about your feet.
fn ring() -> MeshBuf {
    let mut ring = MeshBuf::default();
    annulus(&mut ring, 0.62, 0.72, 0.03, glow(hex(0xc9a227), 1.4));
    ring
}

/// The point on a ring of `SIDES` at index `i`, radius `r`, height `y`.
fn round(i: usize, r: f32, y: f32) -> [f32; 3] {
    let a = i as f32 / SIDES as f32 * TAU;
    [r * a.cos(), y, r * a.sin()]
}

/// A tapered tube between two `(height, radius)` rings, flat-shaded and
/// facing out.
fn frustum(m: &mut MeshBuf, (y0, r0): (f32, f32), (y1, r1): (f32, f32), colour: [f32; 4]) {
    for i in 0..SIDES {
        let mid = (i as f32 + 0.5) / SIDES as f32 * TAU;
        // Out from the axis, tipped up by the taper.
        let slope = (r0 - r1) / (y1 - y0);
        let n = normalise([mid.cos(), slope, mid.sin()]);
        m.quad(
            [
                round(i, r0, y0),
                round(i + 1, r0, y0),
                round(i + 1, r1, y1),
                round(i, r1, y1),
            ],
            n,
            colour,
        );
    }
}

/// A flat-shaded ball of `SIDES` around and half as many down.
fn sphere(m: &mut MeshBuf, c: [f32; 3], r: f32, colour: [f32; 4]) {
    let rows = SIDES / 2;
    let p = |i: usize, j: usize| {
        let lat = i as f32 / rows as f32 * PI;
        let lon = j as f32 / SIDES as f32 * TAU;
        [
            c[0] + r * lat.sin() * lon.cos(),
            c[1] + r * lat.cos(),
            c[2] + r * lat.sin() * lon.sin(),
        ]
    };
    for i in 0..rows {
        for j in 0..SIDES {
            let quad = [p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j)];
            let mid = quad.iter().fold([0.0; 3], |s, q| {
                [s[0] + q[0] - c[0], s[1] + q[1] - c[1], s[2] + q[2] - c[2]]
            });
            let n = normalise(mid);
            // Each triangle on its own, so a pole's collapsed edge doesn't
            // decide the winding of the other half.
            for [a, b, d] in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
                if area(a, b, d) > 1e-8 {
                    m.fan(a, &[b, d], n, colour);
                }
            }
        }
    }
}

/// A box about `c` with half-extents `h`.
fn cuboid(m: &mut MeshBuf, c: [f32; 3], h: [f32; 3], colour: [f32; 4]) {
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let corner = |su: f32, sv: f32| {
                let mut p = c;
                p[axis] += sign * h[axis];
                p[u] += su * h[u];
                p[v] += sv * h[v];
                p
            };
            let mut n = [0.0; 3];
            n[axis] = sign;
            m.quad(
                [
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                ],
                n,
                colour,
            );
        }
    }
}

/// A flat ring about the feet, facing up.
fn annulus(m: &mut MeshBuf, r_in: f32, r_out: f32, y: f32, colour: [f32; 4]) {
    const SEGMENTS: usize = 32;
    for i in 0..SEGMENTS {
        let a0 = i as f32 / SEGMENTS as f32 * TAU;
        let a1 = (i + 1) as f32 / SEGMENTS as f32 * TAU;
        let at = |r: f32, a: f32| [r * a.cos(), y, r * a.sin()];
        m.quad(
            [at(r_in, a0), at(r_out, a0), at(r_out, a1), at(r_in, a1)],
            UP,
            colour,
        );
    }
}

fn normalise([x, y, z]: [f32; 3]) -> [f32; 3] {
    let len = (x * x + y * y + z * z).sqrt().max(1e-6);
    [x / len, y / len, z / len]
}

fn area(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let x = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::mesh::facing;
    use flint_core::{Transform, Vec3};

    /// Where the mesh's front (+X) points once the entity is turned by
    /// `yaw_quat`, through the engine's own transform maths.
    fn front(yaw: f32) -> [f32; 2] {
        let t = Transform {
            position: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            rotation_quat: Some(yaw_quat(yaw)),
        };
        let m = t.to_matrix();
        // Column-major: the first column is where +X goes.
        [m[0][0], m[0][2]]
    }

    #[test]
    fn the_figure_faces_its_heading() {
        // Server headings east, south, west and north, each a step in
        // server space whose world direction the front must take.
        for (facing, step) in [
            (0.0, [1.0, 0.0]),
            (std::f64::consts::FRAC_PI_2, [0.0, 1.0]),
            (std::f64::consts::PI, [-1.0, 0.0]),
            (-std::f64::consts::FRAC_PI_2, [0.0, -1.0]),
        ] {
            let got = front(yaw_of(facing));
            let [wx, wz] = to_world(
                capitol_core::protocol::rules::CENTER_X + step[0] * 10.0,
                capitol_core::protocol::rules::CENTER_Y + step[1] * 10.0,
            );
            let want = [wx, wz].map(|v| v / 1.0f32.max((wx * wx + wz * wz).sqrt()));
            assert!(
                (got[0] - want[0]).abs() < 1e-4 && (got[1] - want[1]).abs() < 1e-4,
                "heading {facing}: front {got:?}, want {want:?}"
            );
        }
    }

    #[test]
    fn turning_takes_the_short_way() {
        assert!((wrap(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-5);
        assert!((wrap(-3.0 * PI / 2.0) - PI / 2.0).abs() < 1e-5);
        assert!((wrap(PI) - PI).abs() < 1e-5);
    }

    #[test]
    fn every_face_of_the_figure_faces_out() {
        for m in [&OTHER, &CITIZEN, &SELF]
            .map(figure)
            .into_iter()
            .chain([ring()])
        {
            assert!(!m.indices.is_empty());
            for t in m.indices.as_chunks::<3>().0 {
                let v = [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]);
                assert!(
                    facing(v[0].position, v[1].position, v[2].position, v[0].normal),
                    "{:?}",
                    v.map(|v| v.position)
                );
            }
        }
    }

    #[test]
    fn a_persons_candle_gutters_and_stays_lit() {
        let samples: Vec<f32> = (0..2000).map(|i| flicker(i as f32 / 60.0, 3.1)).collect();
        let (lo, hi) = samples
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(lo >= 0.55 && hi <= 1.2, "{lo}..{hi}");
        // It moves, but gently: never darker than a draught, never a strobe.
        assert!(hi - lo > 0.2, "barely moves: {lo}..{hi}");
        let jump = samples
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(jump < 0.05, "jumps {jump} in a frame");
        // Two candles side by side don't keep time.
        assert_ne!(flicker(1.0, 0.0), flicker(1.0, 5.0));
    }

    /// The light given off by a figure's lantern: its emissive vertices.
    fn lantern_light(livery: &Livery) -> [f32; 3] {
        let m = figure(livery);
        let v = m
            .vertices
            .iter()
            .find(|v| v.color[3] == 0.0)
            .expect("a lantern");
        [v.color[0], v.color[1], v.color[2]]
    }

    #[test]
    fn a_citizens_lantern_is_cooler_and_only_the_lantern_differs() {
        let [r, g, b] = lantern_light(&OTHER);
        let [cr, cg, cb] = lantern_light(&CITIZEN);
        // Bluer for its red: cooler.
        assert!(
            cb / cr > 2.0 * (b / r),
            "{:?} against {:?}",
            [cr, cg, cb],
            [r, g, b]
        );
        // As strong a flame, and so not much brighter to the eye: quiet.
        let luma = |r: f32, g: f32, b: f32| 0.2126 * r + 0.7152 * g + 0.0722 * b;
        assert!(luma(cr, cg, cb) <= 1.6 * luma(r, g, b));
        let (a, c) = (figure(&OTHER), figure(&CITIZEN));
        assert_eq!(a.vertices.len(), c.vertices.len());
        for (a, c) in a.vertices.iter().zip(&c.vertices) {
            assert_eq!(a.position, c.position);
            if a.color[3] != 0.0 {
                assert_eq!(a.color, c.color, "the cloak is the same");
            }
        }
    }

    #[test]
    fn the_sample_stands_people_and_citizens_where_the_camera_looks() {
        let (state, poses) = sample([10.0, 20.0], [10.0, 40.0]);
        assert_eq!(poses.len(), 5);
        assert!(state
            .me()
            .is_some_and(|me| poses.iter().any(|p| p.id == me)));
        let citizens = poses
            .iter()
            .filter(|p| state.player(p.id).is_some_and(|p| p.citizen))
            .count();
        assert_eq!(citizens, 2);
        for p in &poses {
            let [x, z] = to_world(p.x, p.y);
            assert!(
                (z - 20.0).abs() < 1e-3 && (x - 10.0).abs() < 4.0,
                "{x}, {z}"
            );
        }
    }

    #[test]
    fn on_a_narrow_street_the_stroll_keeps_inside_it() {
        // The bazaar's lane: 2.4 m across, running west along z 29.75,
        // and a camera looking down it a little askew
        let ends = [crate::coords::to_server(0.0, 29.75), crate::coords::to_server(-26.7, 29.75)];
        let lane = RoadSegment {
            id: "bazaar_lane".into(),
            road_type: RoadType::Grid,
            points: ends.iter().map(|&[x, y]| (x, y)).collect(),
            width: 24.0,
        };
        let street = Street::at(std::slice::from_ref(&lane), [-16.0, 30.1]).expect("on the lane");
        assert!((street.centre[1] - 29.75).abs() < 1e-3, "brought onto the centreline");
        assert!((street.half - 0.7).abs() < 1e-3);
        assert_eq!(Street::at(std::slice::from_ref(&lane), [-16.0, 32.0]), None, "off it");
        for t in 0..=10 {
            let (_, poses) = stroll([-16.0, 30.1], [-3.0, 31.0], Some(street), t as f32);
            for p in &poses {
                let [_, z] = to_world(p.x, p.y);
                assert!((z - 29.75).abs() <= 0.7 + 1e-3, "t {t}: {z} leaves the lane");
                if p.moving {
                    // Along the lane: west (π) or east (0), never across it
                    assert!(p.facing.sin().abs() < 1e-3, "{}", p.facing);
                }
            }
        }
    }

    #[test]
    fn the_stroll_walks_up_and_down_the_way_the_camera_looks() {
        // The camera at z 40 looks north (-Z) to z 20: walking away is -Z
        let at = |t: f32| stroll([10.0, 20.0], [10.0, 40.0], None, t);
        let (state, start) = at(0.0);
        let (_, later) = at(5.0);
        assert!(start.iter().filter(|p| state.player(p.id).is_some_and(|p| p.citizen)).count() >= 4);
        for (i, (a, b)) in start.iter().zip(&later).enumerate() {
            let ([ax, az], [bx, bz]) = (to_world(a.x, a.y), to_world(b.x, b.y));
            assert!((ax - bx).abs() < 1e-3, "each keeps its lane");
            assert!((ax - 10.0).abs() < 3.0, "the lanes run near where it looks");
            if !b.moving {
                assert!((az - bz).abs() < 1e-3, "the one who lingers stays put");
                continue;
            }
            // Facing the way it goes: away is -Z, a server heading of -π/2
            let want = if bz < az { -std::f64::consts::FRAC_PI_2 } else { std::f64::consts::FRAC_PI_2 };
            assert!((b.facing - want).abs() < 1e-3, "{}", b.facing);
            // Within fourteen metres of where it looks for a ten-second clip
            for t in 0..=10 {
                let p = &at(t as f32).1[i];
                let [_, z] = to_world(p.x, p.y);
                assert!((z - 20.0).abs() < 14.0, "t {t}: {z}");
            }
        }
    }

    #[test]
    fn the_figure_is_a_players_size_and_stands_on_its_feet() {
        let m = figure(&OTHER);
        let (mut lo, mut hi, mut r) = (f32::MAX, f32::MIN, 0.0f32);
        for v in &m.vertices {
            let [x, y, z] = v.position;
            lo = lo.min(y);
            hi = hi.max(y);
            r = r.max(x.hypot(z));
        }
        assert!(lo.abs() < 1e-4, "feet at {lo}");
        assert!((1.6..1.8).contains(&hi), "{hi} m tall");
        // A player's radius is 8 server units: 0.8 m, lantern and all.
        assert!(r <= 0.8, "{r} m out");
        assert!(LABEL_Y > hi + 0.3, "the name clears the head");
    }
}
