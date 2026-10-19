//! The resource nodes (P4.4, E2.5): every node the server has shown us,
//! modelled by its resource type (`models`): grass tufts, trees, fallen
//! branches, rock outcrops, iron veins and clay banks.
//!
//! - **Merged per sector**, like the houses: a node at rest is baked into
//!   its sector's mesh (`nodes_r3w05`), so the woods cost a draw or two a
//!   sector however many trees they hold. A node has an entity of its own
//!   only while it changes: aimed at or worked (its tips light gold and it
//!   sways or shudders), taken (cut, felled, broken), coming back, or seen
//!   for the first time. Once it settles it goes back into its sector.
//! - **Taking one**: a tuft is cut down and stubble rises; a tree topples
//!   and its stump rises; a rock breaks down into rubble; a clay bank is
//!   dug out. Coming back, the spent look sinks and the full one grows in
//!   again: a tuft springs up past full height and settles, a tree eases up
//!   from a sapling over three seconds.
//! - **Gathering** gives off sawdust from wood, chips from stone and clods
//!   from clay (`particles/gather_*.particles.toml`) while it's worked, with
//!   a burst as it's taken. The scripts' pickup arcs and toasts are as they
//!   were (`scripts/input.rhai`, from what [`Nodes::update`] leaves in
//!   `Shared`), the bits thrown as it's taken in the kind's colour.
//! - **Placed through [`ground_at`]**, so when the wilds get their height
//!   field (W9, #202) one function lifts every node onto the land.
//!
//! Nodes never move. One the server stops listing sinks away and goes.
//! The store holds every node (`/world/nodes` with the snapshot, E2.5), so
//! the woods stand on the horizon; the nearby poll and the WebSocket keep
//! the states round you current. The nodes there at sign-in are simply
//! there; one first seen later grows out of the ground.

pub mod models;
pub mod sample;

use crate::coords::{layer, to_world};
use crate::script_api::{NodeMark, Shared};
use crate::world::mesh::MeshBuf;
use crate::world::sector::Sector;
use capitol_protocol::rest::ResourceNodeInfo;
use capitol_protocol::rules::Zone;
use flint_core::EntityId;
use flint_player::{ExtensionCtx, ExtensionGpu};
use models::{mesh_name, model, Kind, Look};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, TAU};
use uuid::Uuid;

/// A server tick, in seconds: extraction durations come in ticks.
const TICK: f32 = 0.1;
/// Seconds for a spent look to sink, and to rise.
const SINK: f32 = 0.5;
const RISE: f32 = 0.35;
/// Nodes first seen within this many seconds of the first are the world
/// as it stands at sign-in: simply there, not grown in.
const SETTLE: f64 = 3.0;
/// Below this a node has stopped swaying.
const STILL: f32 = 0.01;

/// The land's height under world (X, Z), metres: flat in the city, the
/// wilds' height field past it once it's in (W9, #202).
pub fn ground_at(at: [f32; 2]) -> f32 {
    crate::world::land::ground_y(at)
}

/// The runtime mesh for one sector's nodes at rest.
pub fn sector_mesh_name(sector: Sector) -> String {
    format!("nodes_{}", sector.tag())
}

/// Someone extracting from a node: who, and how far they've got, in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub player: Uuid,
    pub progress: u32,
    pub duration: u32,
}

/// What the scripts hear about: a node was taken, or grew back.
pub struct Changed {
    pub name: &'static str,
    pub at: [f32; 2],
    pub kind: Kind,
}

struct Node {
    id: Uuid,
    kind: Kind,
    variant: u8,
    at: [f32; 2],
    sector: Sector,
    /// Each node turned and sized a little differently, from its id, so
    /// a wood isn't one tree stamped out; and the way it falls.
    yaw: f32,
    size: f32,
    fall: f32,
    /// Its own entity, while it changes, and the mesh on it.
    entity: Option<(EntityId, String)>,
    /// The look being shown now, and whether its tips are lit.
    shown: Look,
    lit: bool,
    /// 0 to 1: how far the shown look has grown in.
    grown: f32,
    /// Whether the shown look is on its way out.
    leaving: bool,
    /// Still listed by the server. One that isn't sinks, then goes.
    present: bool,
    /// The server's state last frame, to fire changes at the scripts.
    state: String,
    /// 0 to 1: how much it's swaying.
    sway: f32,
    /// When this extraction on it was first seen, in game time.
    started: Option<f64>,
    /// The particles' emission last set on its entity, and a burst as
    /// it's taken, 0 to 1.
    fx: Option<f32>,
    burst: f32,
}

#[derive(Default)]
pub struct Nodes {
    nodes: HashMap<Uuid, Node>,
    /// The models baked into the sectors, by kind, shape and look.
    models: HashMap<(Kind, u8, Look), MeshBuf>,
    /// Sectors whose mesh is out of date, and those with something drawn.
    dirty: BTreeSet<Sector>,
    drawn: BTreeSet<Sector>,
    /// When the first nodes came, in game time.
    first: Option<f64>,
    /// Resource types with no model, logged once each.
    unknown: HashSet<String>,
}

impl Nodes {
    /// Place every node for this frame from `nodes`. `work` says who is
    /// extracting a node, `me` is the local player and `target` the node
    /// you're walking to or have asked to extract. Writes the nodes for
    /// picking and the HUD into `shared`, and returns the changes to fire
    /// at the scripts.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        ctx: &mut ExtensionCtx,
        nodes: &HashMap<Uuid, ResourceNodeInfo>,
        work: &dyn Fn(Uuid) -> Option<Work>,
        me: Option<Uuid>,
        target: Option<Uuid>,
        dt: f32,
        shared: &Shared,
    ) -> Vec<Changed> {
        if self.models.is_empty() {
            let Some(gpu) = ctx.gpu.as_mut() else {
                return Vec::new();
            };
            self.upload_models(gpu);
        }
        let now = ctx.clock.total_time;
        if !nodes.is_empty() && self.first.is_none() {
            self.first = Some(now);
        }
        let settled = self.first.is_some_and(|t| now - t < SETTLE);
        let mut changed = Vec::new();
        for node in self.nodes.values_mut() {
            node.present = false;
        }
        let mut marks = Vec::with_capacity(nodes.len());
        for (id, info) in nodes {
            let node = self.nodes.entry(*id).or_insert_with(|| {
                let kind = Kind::of(&info.resource_type);
                if kind == Kind::Other && self.unknown.insert(info.resource_type.clone()) {
                    tracing::info!(
                        "nodes: no model for {:?}, drawn as a cairn",
                        info.resource_type
                    );
                }
                let mut node = Node::new(*id, kind, info);
                if settled {
                    node.grown = 1.0;
                    self.dirty.insert(node.sector);
                }
                node
            });
            node.present = true;
            let want = look_of(&info.state);
            if node.state != info.state {
                let name = match (look_of(&node.state), want) {
                    (Look::Full, Look::Spent) => Some("node_depleted"),
                    (Look::Spent, Look::Full) => Some("node_regenerated"),
                    _ => None,
                };
                if let Some(name) = name {
                    changed.push(Changed {
                        name,
                        at: node.at,
                        kind: node.kind,
                    });
                    if name == "node_depleted" {
                        node.burst = 1.0;
                    }
                }
                node.state.clone_from(&info.state);
            }

            // Whoever is extracting it, and how far they've got. Progress
            // comes every half second, so it's carried on by the clock in
            // between, never past the next report.
            let work = work(*id);
            let progress = work.map(|w| {
                let started = *node.started.get_or_insert(now);
                let duration = w.duration.max(1) as f32;
                let told = w.progress as f32 / duration;
                let clock = (now - started) as f32 / (duration * TICK);
                clock.clamp(told, (told + 5.0 / duration).min(1.0))
            });
            if work.is_none() {
                node.started = None;
            }
            let harvesting = work.is_some() || info.state == "harvesting";
            let targeted = target == Some(*id);
            let lit = want == Look::Full && (harvesting || targeted);
            node.step(want, lit, harvesting, dt);
            marks.push(NodeMark {
                id: id.to_string(),
                at: node.at,
                kind: node.kind.name(),
                r: node.kind.ring() * node.size,
                state: match want {
                    Look::Spent => "depleted",
                    Look::Full if harvesting => "harvesting",
                    Look::Full => "available",
                },
                targeted,
                mine: work.is_some_and(|w| Some(w.player) == me),
                progress,
            });
        }

        for node in self.nodes.values_mut() {
            if !node.present {
                node.leaving = true;
                node.grown -= dt / SINK;
            }
            node.burst = (node.burst - dt / 0.35).max(0.0);
            node.reconcile(ctx, &mut self.dirty);
            if let Some((entity, _)) = &node.entity {
                let entity = *entity;
                node.particles(ctx.world, entity);
                place(ctx.world, entity, node, now);
            }
        }
        let world = &mut *ctx.world;
        self.nodes.retain(|_, node| {
            if node.present || node.grown > 0.0 {
                return true;
            }
            if let Some((entity, _)) = node.entity.take() {
                let _ = world.despawn(entity);
            }
            false
        });
        if let Some(gpu) = ctx.gpu.as_mut() {
            for name in self.upload_sectors(gpu) {
                if ctx.world.get_id(&name).is_some() {
                    continue;
                }
                let attached = ctx
                    .world
                    .spawn(name.as_str())
                    .and_then(|id| ctx.attach_mesh(id, &name));
                if let Err(e) = attached {
                    tracing::warn!("nodes: no entity for {name}: {e}");
                }
            }
        }
        if let Ok(mut shared) = shared.nodes.lock() {
            *shared = marks;
        }
        changed
    }

    /// How many nodes there are, and how many have entities of their own.
    pub fn counts(&self) -> (usize, usize) {
        let own = self.nodes.values().filter(|n| n.entity.is_some()).count();
        (self.nodes.len(), own)
    }

    /// Every model, as meshes for the nodes that change, and kept for
    /// baking into the sectors.
    fn upload_models(&mut self, gpu: &mut ExtensionGpu) {
        for kind in Kind::ALL {
            for v in 0..kind.variants() {
                for (look, lit) in [(Look::Full, false), (Look::Full, true), (Look::Spent, false)] {
                    let m = model(kind, v, look, lit);
                    m.upload(gpu, &mesh_name(kind, v, look, lit), roughness(kind));
                    if !lit {
                        self.models.insert((kind, v, look), m);
                    }
                }
            }
        }
    }

    /// The land under the nodes moved (W9): every sector is baked again.
    pub fn land_changed(&mut self) {
        self.dirty.extend(self.nodes.values().map(|n| n.sector));
    }

    /// One sector's nodes at rest, as one mesh.
    fn sector_mesh(&self, sector: Sector) -> MeshBuf {
        let mut m = MeshBuf::default();
        for node in self.nodes.values() {
            if node.sector != sector || node.entity.is_some() || !node.present {
                continue;
            }
            if let Some(model) = self.models.get(&(node.kind, node.variant, node.shown)) {
                bake(&mut m, model, node);
            }
        }
        m
    }

    /// Uploads the sectors that changed, and returns the names of those
    /// with something in them that weren't drawn before.
    fn upload_sectors(&mut self, gpu: &mut ExtensionGpu) -> Vec<String> {
        let mut fresh = Vec::new();
        for sector in std::mem::take(&mut self.dirty) {
            let name = sector_mesh_name(sector);
            let m = self.sector_mesh(sector);
            // Rocks and wood alike: the rough end of the models' range.
            m.upload(gpu, &name, 0.85);
            if m.indices.is_empty() {
                self.drawn.remove(&sector);
            } else if self.drawn.insert(sector) {
                fresh.push(name);
            }
        }
        if !fresh.is_empty() {
            let (all, own) = self.counts();
            tracing::debug!("nodes: {all}, {own} with entities of their own");
        }
        fresh
    }
}

/// How shiny a kind's own meshes are: wet clay a little, rock and wood not.
fn roughness(kind: Kind) -> f32 {
    match kind {
        Kind::Clay => 0.55,
        Kind::Vein => 0.7,
        Kind::Grass => 0.8,
        _ => 0.9,
    }
}

impl Node {
    fn new(id: Uuid, kind: Kind, info: &ResourceNodeInfo) -> Node {
        let h = id.as_u128();
        let hash = (h as u64) ^ ((h >> 64) as u64);
        let zone = Zone::from_position(info.x, info.y);
        let (lo, hi) = kind.size_range();
        Node {
            id,
            kind,
            variant: kind.variant_for(hash, zone),
            at: to_world(info.x, info.y),
            sector: Sector::of([info.x, info.y]),
            yaw: (h % 3600) as f32 / 3600.0 * TAU,
            size: lo + ((h >> 12) % 1000) as f32 / 1000.0 * (hi - lo),
            fall: ((h >> 40) % 3600) as f32 / 3600.0 * TAU,
            entity: None,
            shown: look_of(&info.state),
            lit: false,
            grown: 0.0,
            leaving: false,
            present: true,
            state: info.state.clone(),
            sway: 0.0,
            started: None,
            fx: None,
            burst: 0.0,
        }
    }

    /// Move toward the look the node should have: take down or sink what's
    /// shown, then grow in the new look.
    fn step(&mut self, want: Look, lit: bool, worked: bool, dt: f32) {
        if self.shown != want {
            // Going down, it's done with: no gold on a felled tree.
            self.leaving = true;
            self.lit = false;
            let out = match self.shown {
                Look::Full => self.kind.exit_secs(),
                Look::Spent => SINK,
            };
            self.grown -= dt / out;
            if self.grown <= 0.0 {
                self.grown = 0.0;
                self.shown = want;
                self.leaving = false;
                self.lit = lit;
            }
        } else {
            self.leaving = false;
            let grow = match self.shown {
                Look::Full => self.kind.regrow_secs(),
                Look::Spent => RISE,
            };
            self.grown = (self.grown + dt / grow).min(1.0);
            self.lit = lit;
        }
        let target = if worked && self.shown == Look::Full {
            1.0
        } else {
            0.0
        };
        self.sway += (target - self.sway) * (1.0 - (-4.0 * dt).exp());
    }

    /// Whether it's moving, lit or otherwise not what its sector would
    /// draw: then it needs an entity of its own.
    fn busy(&self) -> bool {
        !self.present
            || self.leaving
            || self.grown < 1.0
            || self.lit
            || self.sway > STILL
            || self.burst > 0.0
    }

    /// Gives it an entity of its own, or takes it back into its sector,
    /// and keeps the entity's mesh on the look shown.
    fn reconcile(&mut self, ctx: &mut ExtensionCtx, dirty: &mut BTreeSet<Sector>) {
        let busy = self.busy();
        let want = mesh_name(self.kind, self.variant, self.shown, self.lit);
        match (&mut self.entity, busy) {
            (None, true) => {
                let name = format!("node_{}", self.id.simple());
                match ctx.world.spawn(name.as_str()) {
                    Ok(entity) => {
                        if let Err(e) = ctx.attach_mesh(entity, &want) {
                            tracing::warn!("nodes: no mesh {want}: {e}");
                        }
                        self.entity = Some((entity, want));
                        self.fx = None;
                        dirty.insert(self.sector);
                    }
                    Err(e) => tracing::warn!("nodes: no entity for {name}: {e}"),
                }
            }
            (Some((entity, _)), false) => {
                let _ = ctx.world.despawn(*entity);
                self.entity = None;
                dirty.insert(self.sector);
            }
            (Some((entity, shown)), true) if *shown != want => {
                let _ = ctx.attach_mesh(*entity, &want);
                *shown = want;
            }
            _ => {}
        }
    }

    /// Sawdust, chips or clods while it's worked, and a burst as it's
    /// taken, through its entity's `particle_effect`.
    fn particles(&mut self, world: &mut flint_ecs::FlintWorld, entity: EntityId) {
        let Some(effect) = self.kind.effect() else {
            return;
        };
        let worked = if self.shown == Look::Full { self.sway } else { 0.0 };
        let want = (if worked > 0.5 { 1.0 } else { 0.0 }) + 3.0 * self.burst;
        let want = (want * 10.0).round() / 10.0;
        if self.fx == Some(want) || (self.fx.is_none() && want == 0.0) {
            return;
        }
        let mut fx = toml::Table::new();
        fx.insert("effect".into(), effect.into());
        fx.insert(
            "seed".into(),
            ((self.id.as_u128() % 1_000_000) as i64 + 1).into(),
        );
        fx.insert("emission_scale".into(), (want as f64).into());
        let _ = world.set_component(entity, "particle_effect", toml::Value::Table(fx));
        self.fx = Some(want);
    }

    /// The scale as `[across, up]`, and how far it has toppled, radians.
    fn shape(&self) -> ([f32; 2], f32) {
        let k = self.grown;
        match (self.shown, self.kind, self.leaving) {
            // A tuft springs up past full height and settles, and is cut
            // down faster than it narrows.
            (Look::Full, Kind::Grass, _) => ([0.55 + 0.45 * smooth(k), spring(k)], 0.0),
            // A tree falls over for most of its exit, and only then goes.
            (Look::Full, Kind::Tree, true) => {
                let fell = ((1.0 - k) / 0.8).clamp(0.0, 1.0);
                let s = smooth(k / 0.2);
                ([s, s], FRAC_PI_2 * 0.95 * fell * fell)
            }
            // And grows back slowly from a sapling, easing out.
            (Look::Full, Kind::Tree, false) => {
                let s = 0.12 + 0.88 * ease_out(k);
                ([s, s], 0.0)
            }
            // Rock breaks down and sinks; branches and clay are gathered up.
            (Look::Full, _, true) => ([0.7 + 0.3 * smooth(k), smooth(k)], 0.0),
            (Look::Full, _, false) => ([0.6 + 0.4 * smooth(k), ease_out(k)], 0.0),
            (Look::Spent, ..) => ([0.8 + 0.2 * smooth(k), smooth(k)], 0.0),
        }
    }
}

fn look_of(state: &str) -> Look {
    if state == "depleted" {
        Look::Spent
    } else {
        Look::Full
    }
}

/// Write the node's transform onto its entity.
fn place(world: &mut flint_ecs::FlintWorld, entity: EntityId, node: &Node, now: f64) {
    let [x, z] = node.at;
    let ([across, up], fell) = node.shape();
    let s = node.size;
    let mut t = toml::Table::new();
    t.insert("position".into(), vec3([x, ground_at(node.at), z]));
    // Turned about +Y, toppled about a level axis, then leant by the sway
    // about a level axis that wanders, so it reads as wind and work rather
    // than a metronome.
    let (amp, hz) = node.kind.sway();
    let phase = (now as f32) * hz * TAU + node.yaw;
    let lean = amp * node.sway * phase.sin();
    let axis = node.yaw + 0.6 * (phase * 0.37).sin();
    let topple = axis_angle([node.fall.cos(), 0.0, node.fall.sin()], fell);
    let q = mul(
        axis_angle([axis.cos(), 0.0, axis.sin()], lean),
        mul(topple, axis_angle([0.0, 1.0, 0.0], node.yaw)),
    );
    t.insert(
        "rotation_quat".into(),
        toml::Value::Array(q.iter().map(|v| toml::Value::Float(*v as f64)).collect()),
    );
    // Never quite zero: a zero scale has no inverse for the normals.
    let (a, u) = ((across * s).max(1e-3), (up * s).max(1e-3));
    t.insert("scale".into(), vec3([a, u, a]));
    let _ = world.set_component(entity, "transform", toml::Value::Table(t));
}

/// Adds `model` to `m` as the node at rest stands: sized, turned and in
/// its place, as `place` would put its entity.
fn bake(m: &mut MeshBuf, model: &MeshBuf, node: &Node) {
    let q = axis_angle([0.0, 1.0, 0.0], node.yaw);
    let base = [node.at[0], ground_at(node.at), node.at[1]];
    let start = m.vertices.len() as u32;
    m.vertices.extend(model.vertices.iter().map(|v| {
        let mut v = *v;
        let p = rotate(q, v.position.map(|c| c * node.size));
        v.position = [p[0] + base[0], p[1] + base[1], p[2] + base[2]];
        v.normal = rotate(q, v.normal);
        v
    }));
    m.indices.extend(model.indices.iter().map(|i| i + start));
}

fn vec3(v: [f32; 3]) -> toml::Value {
    toml::Value::Array(v.iter().map(|c| toml::Value::Float(*c as f64)).collect())
}

/// A turn of `angle` about the unit `axis`, as `[x, y, z, w]`.
fn axis_angle(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let (s, c) = (angle * 0.5).sin_cos();
    [axis[0] * s, axis[1] * s, axis[2] * s, c]
}

/// `a` after `b`.
fn mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// `v` turned by the unit quaternion `q`.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let w = q[3];
    let t = models::cross(u, v).map(|c| 2.0 * c);
    let c = models::cross(u, t);
    [
        v[0] + w * t[0] + c[0],
        v[1] + w * t[1] + c[1],
        v[2] + w * t[2] + c[2],
    ]
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}

/// Up from 0 to 1 a little past it, and back: an ease-out with overshoot.
fn spring(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c = 1.9;
    let u = t - 1.0;
    1.0 + (c + 1.0) * u * u * u + c * u * u
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(state: &str, kind: &str) -> ResourceNodeInfo {
        ResourceNodeInfo {
            id: Uuid::from_u128(7),
            resource_type: kind.into(),
            x: 3400.0,
            y: 4000.0,
            state: state.into(),
        }
    }

    #[test]
    fn a_regrowing_tuft_overshoots_and_settles() {
        assert!(spring(0.0).abs() < 1e-5);
        assert!((spring(1.0) - 1.0).abs() < 1e-5);
        let peak = (0..=100)
            .map(|i| spring(i as f32 / 100.0))
            .fold(0.0, f32::max);
        assert!(peak > 1.05 && peak < 1.25, "peak {peak}");
    }

    #[test]
    fn the_quaternions_compose_and_turn() {
        let yaw = axis_angle([0.0, 1.0, 0.0], 0.7);
        let none = axis_angle([1.0, 0.0, 0.0], 0.0);
        let q = mul(none, yaw);
        for k in 0..4 {
            assert!((q[k] - yaw[k]).abs() < 1e-6);
        }
        let two = mul(yaw, yaw);
        let want = axis_angle([0.0, 1.0, 0.0], 1.4);
        for k in 0..4 {
            assert!((two[k] - want[k]).abs() < 1e-6);
        }
        // A quarter turn about +Y takes +X to -Z, as Flint's would.
        let p = rotate(axis_angle([0.0, 1.0, 0.0], FRAC_PI_2), [1.0, 0.0, 0.0]);
        assert!(p[0].abs() < 1e-6 && (p[2] + 1.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn a_baked_node_stands_where_its_entity_would() {
        let node = Node::new(
            Uuid::from_u128(7),
            Kind::Outcrop,
            &info("available", "rock_outcrop"),
        );
        let model = model(Kind::Outcrop, node.variant, Look::Full, false);
        let mut m = MeshBuf::default();
        bake(&mut m, &model, &node);
        assert_eq!(m.vertices.len(), model.vertices.len());
        let q = axis_angle([0.0, 1.0, 0.0], node.yaw);
        for (baked, v) in m.vertices.iter().zip(&model.vertices) {
            let p = rotate(q, v.position.map(|c| c * node.size));
            let want = [p[0] + node.at[0], p[1] + layer::GROUND, p[2] + node.at[1]];
            for k in 0..3 {
                assert!((baked.position[k] - want[k]).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn a_node_is_busy_only_while_it_changes() {
        let mut node = Node::new(Uuid::from_u128(7), Kind::Tree, &info("available", "tree"));
        node.grown = 1.0;
        assert!(!node.busy(), "at rest it's in its sector");
        // Worked: lit and swaying.
        node.step(Look::Full, true, true, 0.1);
        assert!(node.busy());
        // Felled: it topples, then the stump rises.
        let mut toppled = 0.0f32;
        for _ in 0..40 {
            node.step(Look::Spent, false, false, 0.05);
            toppled = toppled.max(node.shape().1);
        }
        assert!(toppled > 1.2, "it fell over: {toppled}");
        assert_eq!(node.shown, Look::Spent);
        for _ in 0..60 {
            node.step(Look::Spent, false, false, 0.05);
        }
        assert!(!node.busy(), "the stump settles back into its sector");
        // Grows back slowly, from a sapling.
        let mut t = 0.0;
        while node.shown == Look::Spent || node.grown < 1.0 {
            node.step(Look::Full, false, false, 0.05);
            t += 0.05;
            assert!(t < 10.0);
        }
        assert!(t > 3.0, "a tree takes its time: {t}");
    }

    #[test]
    fn nodes_stand_on_one_ground_function() {
        assert_eq!(ground_at([1.0, 2.0]), layer::GROUND);
    }
}
