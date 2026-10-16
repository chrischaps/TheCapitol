//! The stations (P6.1): one entity per station the server has shown us,
//! a small model by type, placed from the store each frame like the
//! resource nodes (`nodes.rs`). The orange in-range ring round them is
//! `scripts/input.rhai`, from what [`Stations::update`] leaves in `Shared`.
//!
//! The models wear the React client's colours (`GameCanvas.tsx`
//! `drawStation`): a workbench of `#8b5a2b` planks on `#5d3a1a` legs, a
//! `#4a4a54` stone forge with an `#f97316` mouth and a `#fbbf24` heart,
//! and a `#8b6914` chest under an `#a67c1a` lid with a `#ffd700` lock. A
//! type the client doesn't know is React's grey block. React draws each
//! 20 units (2 m) across, and so do these, give or take.
//!
//! The forge's fire is real: the scene's `particles/forge.particles.toml`
//! (flames in the mouth, embers and a little smoke out of the chimney) on
//! its entity, and its mouth glows (`palette::glow`) bright enough to
//! bloom.
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

use crate::coords::{layer, to_server, to_world};
use crate::script_api::{Shared, StationMark};
use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{glow, hex, mix};
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
pub const MESH_GENERIC: &str = "station_generic";

/// The forge's fire, `particles/forge.particles.toml`.
const FORGE_EFFECT: &str = "forge";

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
    /// The station type being placed.
    pub station_type: &'static str,
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
    pub fn update(
        &mut self,
        ctx: &mut ExtensionCtx,
        state: &GameState,
        me: Option<[f32; 3]>,
        placed: &HashSet<Uuid>,
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
                    })
                }
            };
            station.present = true;
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
            });
        }

        let world = &mut *ctx.world;
        self.stations.retain(|_, station| {
            if !station.present {
                station.arrival = Arrival::Rise;
                station.shown -= dt / SINK;
                if station.shown <= 0.0 {
                    let _ = world.despawn(station.entity);
                    return false;
                }
            }
            place(world, station);
            true
        });
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
        let kind = kind_of(view.station_type);
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
        vec3([x, layer::GROUND + GHOST_HOVER * k + bob, z]),
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

/// The type's mesh name and look: the three the server seeds, or
/// anything else.
pub fn kind_of(station_type: &str) -> &'static str {
    match station_type {
        "workbench" => "workbench",
        "forge" => "forge",
        "storage_chest" => "storage_chest",
        _ => "generic",
    }
}

fn mesh(kind: &str) -> &'static str {
    match kind {
        "workbench" => MESH_WORKBENCH,
        "forge" => MESH_FORGE,
        "storage_chest" => MESH_CHEST,
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
        _ => 1.2,
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
    if kind == "forge" {
        let mut fx = toml::Table::new();
        fx.insert("effect".into(), FORGE_EFFECT.into());
        // Each forge flickers its own way.
        fx.insert(
            "seed".into(),
            ((id.as_u128() % 1_000_000) as i64 + 1).into(),
        );
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
    t.insert("position".into(), vec3([x, layer::GROUND + lift, z]));
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
const GENERIC: u32 = 0x5a5a6a;
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
        ("generic", generic(), 0.8),
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
    m
}

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

/// React's grey block, for a type the client doesn't know.
fn generic() -> MeshBuf {
    let mut m = MeshBuf::default();
    cuboid(&mut m, [-0.7, 0.0, -0.7], [0.7, 1.0, 0.7], hex(GENERIC));
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
    fn only_the_forge_glows() {
        let glows = |m: &MeshBuf| m.vertices.iter().any(is_light);
        assert!(glows(&forge()));
        assert!(!glows(&workbench()));
        assert!(!glows(&chest()));
    }

    #[test]
    fn the_forges_heart_is_bright_enough_to_bloom() {
        // The scene's bloom threshold is 2.5.
        let brightest = forge()
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

    #[test]
    fn every_face_faces_its_normal() {
        for m in [workbench(), forge(), chest(), generic()] {
            for t in m.indices.chunks_exact(3) {
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
    fn unknown_types_get_the_grey_block() {
        assert_eq!(mesh(kind_of("workbench")), MESH_WORKBENCH);
        assert_eq!(mesh(kind_of("storage_chest")), MESH_CHEST);
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
    fn the_forges_fire_loads() {
        let text = include_str!("../../../particles/forge.particles.toml");
        let fx = flint_particles::load_effect_from_str(text, "forge").expect("parses");
        assert_eq!(fx.name, FORGE_EFFECT);
        let names: Vec<_> = fx.emitters.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["flames", "embers", "smoke"]);
    }
}
