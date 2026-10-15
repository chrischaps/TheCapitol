//! The resource nodes (P4.4): one entity per node the server has shown us,
//! a tuft of wild grass while it's there to take and a patch of cut stubble
//! once it's been taken. Placed from the store each frame, like the players
//! (`entities.rs`). The rings, the targeting line and the toasts round them
//! are `scripts/input.rhai`, from what [`Nodes::update`] leaves in `Shared`.
//!
//! The tuft wears the React client's greens (`#4a7c59` blades, `#6ba377`
//! tips, on a `#3d5e42` mound). While you're walking to one, or anyone is
//! extracting it, its tips light gold, as React's do, and someone working
//! it makes it sway. Taking it cuts it down: the blades drop and the
//! stubble rises in their place. When it regrows (`node_regenerated`), the
//! stubble sinks and the tuft springs back up, a little past full height
//! and settling. A node seen for the first time grows out of the ground.
//!
//! Nodes never move. One the server stops listing (after a reconnect) sinks
//! away and is despawned.
//!
//! The server lists nodes only within 200 units (20 m) of you, so tufts
//! appear as you walk near; growing them in keeps that from popping. A
//! workaround until the client can ask for more (issue #79).

use crate::coords::{layer, to_world};
use crate::script_api::{NodeMark, Shared};
use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{glow, hex, mix, GOLD};
use capitol_core::state::{Activity, GameState};
use flint_core::EntityId;
use flint_player::{ExtensionCtx, ExtensionGpu};
use std::collections::HashMap;
use std::f32::consts::TAU;
use uuid::Uuid;

/// The runtime meshes: a tuft, a tuft with its tips lit, and stubble.
pub const MESH_TUFT: &str = "node_tuft";
pub const MESH_TUFT_LIT: &str = "node_tuft_lit";
pub const MESH_STUBBLE: &str = "node_stubble";

/// A server tick, in seconds: extraction durations come in ticks.
const TICK: f32 = 0.1;
/// Seconds for a tuft to be cut down, and for stubble to sink.
const CUT: f32 = 0.28;
const SINK: f32 = 0.5;
/// Seconds for stubble to rise, and for a tuft to grow back.
const RISE: f32 = 0.35;
const REGROW: f32 = 1.3;
/// How far a worked tuft leans as it sways, in radians, and how fast.
const SWAY: f32 = 0.11;
const SWAY_HZ: f32 = 1.3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Tuft,
    Stubble,
}

struct Node {
    entity: EntityId,
    at: [f32; 2],
    /// Each node turned and sized a little differently, from its id, so
    /// a meadow of them isn't one tuft stamped out.
    yaw: f32,
    size: f32,
    /// The look on the entity now, and whether its tips are lit.
    shown: Look,
    lit: bool,
    /// 0 to 1: how far the shown look has grown in.
    grown: f32,
    /// Still listed by the server. One that isn't sinks, then goes.
    present: bool,
    /// The server's state last frame, to fire changes at the scripts.
    state: String,
    /// 0 to 1: how much it's swaying.
    sway: f32,
    /// When this frame's extraction on it was first seen, in game time.
    started: Option<f64>,
}

#[derive(Default)]
pub struct Nodes {
    nodes: HashMap<Uuid, Node>,
    meshes: bool,
}

/// What the scripts hear about: a node was taken, or grew back.
pub struct Changed {
    pub name: &'static str,
    pub at: [f32; 2],
}

impl Nodes {
    /// Place every node for this frame from `state`. `target` is the node
    /// you're walking to or have asked to extract. Writes the nodes for
    /// picking and the HUD into `shared`, and returns the changes to fire
    /// at the scripts.
    pub fn update(
        &mut self,
        ctx: &mut ExtensionCtx,
        state: &GameState,
        target: Option<Uuid>,
        dt: f32,
        shared: &Shared,
    ) -> Vec<Changed> {
        if !self.meshes {
            let Some(gpu) = ctx.gpu.as_mut() else {
                return Vec::new();
            };
            upload(gpu);
            self.meshes = true;
        }
        let now = ctx.clock.total_time;
        let me = state.me();
        let mut changed = Vec::new();
        for node in self.nodes.values_mut() {
            node.present = false;
        }
        let mut marks = Vec::with_capacity(state.nodes().len());
        for (id, info) in state.nodes() {
            let node = match self.nodes.get_mut(id) {
                Some(n) => n,
                None => {
                    let name = format!("node_{}", id.simple());
                    let entity = match ctx.world.spawn(name.as_str()) {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!("nodes: no entity for {id}: {e}");
                            continue;
                        }
                    };
                    let shown = look_of(&info.state);
                    if let Err(e) = ctx.attach_mesh(entity, mesh(shown, false)) {
                        tracing::warn!("nodes: no mesh for {id}: {e}");
                    }
                    let h = id.as_u128();
                    self.nodes.entry(*id).or_insert(Node {
                        entity,
                        at: to_world(info.x, info.y),
                        yaw: (h % 3600) as f32 / 3600.0 * TAU,
                        size: 0.88 + ((h >> 12) % 100) as f32 / 100.0 * 0.24,
                        shown,
                        lit: false,
                        grown: 0.0,
                        present: true,
                        state: info.state.clone(),
                        sway: 0.0,
                        started: None,
                    })
                }
            };
            node.present = true;
            if node.state != info.state {
                let name = match (look_of(&node.state), look_of(&info.state)) {
                    (Look::Tuft, Look::Stubble) => Some("node_depleted"),
                    (Look::Stubble, Look::Tuft) => Some("node_regenerated"),
                    _ => None,
                };
                if let Some(name) = name {
                    changed.push(Changed { name, at: node.at });
                }
                node.state.clone_from(&info.state);
            }

            // Whoever is extracting it, and how far they've got. Progress
            // comes every half second, so it's carried on by the clock in
            // between, never past the next report.
            let work = state.harvester(*id).and_then(|p| match state.activity(p) {
                Some(Activity::Extracting {
                    progress, duration, ..
                }) => Some((p, *progress, *duration)),
                _ => None,
            });
            let progress = work.map(|(_, progress, duration)| {
                let started = *node.started.get_or_insert(now);
                let told = progress as f32 / duration.max(1) as f32;
                let clock = (now - started) as f32 / (duration.max(1) as f32 * TICK);
                clock.clamp(told, (told + 5.0 / duration.max(1) as f32).min(1.0))
            });
            if work.is_none() {
                node.started = None;
            }
            let harvesting = work.is_some() || info.state == "harvesting";
            let targeted = target == Some(*id);
            let want = look_of(&info.state);
            let lit = want == Look::Tuft && (harvesting || targeted);
            node.step(ctx, want, lit, harvesting, dt);
            marks.push(NodeMark {
                id: id.to_string(),
                at: node.at,
                state: match want {
                    Look::Stubble => "depleted",
                    Look::Tuft if harvesting => "harvesting",
                    Look::Tuft => "available",
                },
                targeted,
                mine: work.is_some_and(|(p, ..)| Some(p) == me),
                progress,
            });
        }

        let world = &mut *ctx.world;
        self.nodes.retain(|_, node| {
            if node.present {
                place(world, node, now);
                return true;
            }
            node.grown -= dt / SINK;
            if node.grown <= 0.0 {
                let _ = world.despawn(node.entity);
                return false;
            }
            place(world, node, now);
            true
        });
        if let Ok(mut shared) = shared.nodes.lock() {
            *shared = marks;
        }
        changed
    }
}

impl Node {
    /// Move toward the look the node should have: cut down or sink what's
    /// shown, then grow in the new look.
    fn step(&mut self, ctx: &mut ExtensionCtx, want: Look, lit: bool, worked: bool, dt: f32) {
        if self.shown != want {
            let out = if self.shown == Look::Tuft { CUT } else { SINK };
            self.grown -= dt / out;
            if self.grown <= 0.0 {
                self.grown = 0.0;
                self.shown = want;
                self.lit = lit;
                let _ = ctx.attach_mesh(self.entity, mesh(want, lit));
            }
        } else {
            let grow = if self.shown == Look::Tuft {
                REGROW
            } else {
                RISE
            };
            self.grown = (self.grown + dt / grow).min(1.0);
            if self.lit != lit {
                self.lit = lit;
                let _ = ctx.attach_mesh(self.entity, mesh(self.shown, lit));
            }
        }
        let target = if worked && self.shown == Look::Tuft {
            1.0
        } else {
            0.0
        };
        self.sway += (target - self.sway) * (1.0 - (-4.0 * dt).exp());
    }

    /// The scale as `[across, up]`: a tuft springs up past full height and
    /// settles, and is cut down faster than it narrows; stubble just rises.
    fn scale(&self) -> [f32; 2] {
        let k = self.grown;
        match self.shown {
            Look::Tuft => [0.55 + 0.45 * smooth(k), spring(k)],
            Look::Stubble => [0.8 + 0.2 * smooth(k), smooth(k)],
        }
    }
}

fn look_of(state: &str) -> Look {
    if state == "depleted" {
        Look::Stubble
    } else {
        Look::Tuft
    }
}

fn mesh(look: Look, lit: bool) -> &'static str {
    match (look, lit) {
        (Look::Stubble, _) => MESH_STUBBLE,
        (Look::Tuft, false) => MESH_TUFT,
        (Look::Tuft, true) => MESH_TUFT_LIT,
    }
}

/// Write the node's transform onto its entity.
fn place(world: &mut flint_ecs::FlintWorld, node: &Node, now: f64) {
    let [x, z] = node.at;
    let [across, up] = node.scale();
    let s = node.size;
    let mut t = toml::Table::new();
    t.insert("position".into(), vec3([x, layer::GROUND, z]));
    // Turned about +Y, then leant by the sway about a horizontal axis
    // that wanders, so it reads as wind and work rather than a metronome.
    let phase = (now as f32) * SWAY_HZ * TAU + node.yaw;
    let lean = SWAY * node.sway * phase.sin();
    let axis = node.yaw + 0.6 * (phase * 0.37).sin();
    let q = mul(
        axis_angle([axis.cos(), 0.0, axis.sin()], lean),
        axis_angle([0.0, 1.0, 0.0], node.yaw),
    );
    t.insert(
        "rotation_quat".into(),
        toml::Value::Array(q.iter().map(|v| toml::Value::Float(*v as f64)).collect()),
    );
    // Never quite zero: a zero scale has no inverse for the normals.
    let (a, u) = ((across * s).max(1e-3), (up * s).max(1e-3));
    t.insert("scale".into(), vec3([a, u, a]));
    let _ = world.set_component(node.entity, "transform", toml::Value::Table(t));
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

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Up from 0 to 1 a little past it, and back: an ease-out with overshoot.
fn spring(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c = 1.9;
    let u = t - 1.0;
    1.0 + (c + 1.0) * u * u * u + c * u * u
}

// ── The meshes ───────────────────────────────────────────────────────────

const BLADE: u32 = 0x4a7c59;
const TIP: u32 = 0x6ba377;
const MOUND: u32 = 0x3d5e42;
const BLADE_DARK: u32 = 0x2d4e36;
const STRAW: u32 = 0x7a7460;
const STUBBLE_MOUND: u32 = 0x2e2c30;
const FALLEN: u32 = 0x4a4a54;

fn upload(gpu: &mut ExtensionGpu) {
    tuft(false).upload(gpu, MESH_TUFT, 0.8);
    tuft(true).upload(gpu, MESH_TUFT_LIT, 0.8);
    stubble().upload(gpu, MESH_STUBBLE, 0.9);
}

/// A little generator of numbers in [0, 1), so every tuft is the same
/// tuft and a test can rebuild it.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// A tuft of wild grass, its base at the origin: a dozen blades arching
/// out of a low mound, the middle ones tallest, and three seed stalks over
/// them. About 1.4 m across and 1.3 m to the seed heads: React's 12-unit
/// node, a little generous so it reads from the street camera.
fn tuft(lit: bool) -> MeshBuf {
    let mut m = MeshBuf::default();
    mound(&mut m, 0.55, 0.08, hex(MOUND));
    let mut rng = Rng(0x9e37_79b9);
    let mut tips = MeshBuf::default();
    let tip = if lit {
        glow(mix(hex(TIP), hex(GOLD), 0.75), 0.9)
    } else {
        hex(TIP)
    };
    const BLADES: usize = 22;
    for i in 0..BLADES {
        // Golden-angle turns, so the blades fan evenly without a pattern.
        let a = i as f32 * 2.399_963 + rng.range(-0.15, 0.15);
        let inner = i % 3 == 0;
        let r0 = if inner {
            rng.range(0.02, 0.08)
        } else {
            rng.range(0.1, 0.26)
        };
        let height = if inner {
            rng.range(0.8, 1.02)
        } else {
            rng.range(0.48, 0.8)
        };
        let lean = if inner {
            rng.range(0.1, 0.28)
        } else {
            rng.range(0.3, 0.55)
        };
        let colours = [
            hex(BLADE_DARK),
            mix(hex(BLADE_DARK), hex(BLADE), 0.6),
            hex(BLADE),
        ];
        blade(&mut m, &mut tips, a, r0, height, lean, 0.085, colours, tip);
    }
    // Two seed stalks, thin and straight-ish, with a head of seed.
    for (a, lean) in [(0.9f32, 0.14f32), (3.6, 0.2), (5.3, 0.1)] {
        let seed = mix(hex(STRAW), hex(TIP), 0.35);
        let colours = [hex(BLADE_DARK), hex(BLADE), hex(BLADE)];
        blade(&mut m, &mut tips, a, 0.06, 1.15, lean, 0.022, colours, seed);
        let [dx, dz] = [a.cos(), a.sin()];
        let top = [0.06 * dx + lean * dx, 1.15, 0.06 * dz + lean * dz];
        head(&mut tips, top, [dx, dz], if lit { tip } else { seed });
    }
    if lit {
        tips = tips.glowing();
    }
    m.append(tips);
    m
}

/// Stubble: short cut stalks, flat across the top, on a darker mound, and
/// a few blades fallen across it.
fn stubble() -> MeshBuf {
    let mut m = MeshBuf::default();
    mound(&mut m, 0.5, 0.05, hex(STUBBLE_MOUND));
    let mut rng = Rng(0x2545_f491);
    let mut tops = MeshBuf::default();
    for i in 0..11 {
        let a = i as f32 / 11.0 * TAU + rng.range(-0.25, 0.25);
        let r0 = rng.range(0.04, 0.3);
        let h = rng.range(0.1, 0.2);
        let straw = mix(hex(STRAW), hex(FALLEN), rng.next() * 0.6);
        let colours = [mix(straw, hex(STUBBLE_MOUND), 0.4), straw, straw];
        // Cut: a flat end, so the tip is as wide as the base.
        stalk(&mut m, a, r0, h, 0.03, colours);
        let [dx, dz] = [a.cos(), a.sin()];
        let c = [r0 * dx, h + 0.04, r0 * dz];
        tops.quad(
            [
                [c[0] - 0.02, c[1], c[2] - 0.02],
                [c[0] + 0.02, c[1], c[2] - 0.02],
                [c[0] + 0.02, c[1], c[2] + 0.02],
                [c[0] - 0.02, c[1], c[2] + 0.02],
            ],
            UP,
            mix(straw, hex(0xd8cfa8), 0.35),
        );
    }
    m.append(tops);
    // Fallen blades, lying almost flat over the mound.
    for a in [0.4f32, 2.3, 4.4] {
        let [dx, dz] = [a.cos(), a.sin()];
        let [sx, sz] = [-dz * 0.04, dx * 0.04];
        let (r0, r1, y) = (0.08, 0.55, 0.075);
        m.quad(
            [
                [dx * r0 - sx, y, dz * r0 - sz],
                [dx * r0 + sx, y, dz * r0 + sz],
                [dx * r1, y + 0.02, dz * r1],
                [dx * r1, y + 0.02, dz * r1],
            ],
            UP,
            hex(FALLEN),
        );
    }
    m
}

/// A low dome of earth, `r` across and `h` high, its edge just under the
/// ground so it sits in it rather than on it.
fn mound(m: &mut MeshBuf, r: f32, h: f32, colour: [f32; 4]) {
    const SIDES: usize = 12;
    let rim = |i: usize, r: f32, y: f32| {
        let a = i as f32 / SIDES as f32 * TAU;
        [r * a.cos(), y, r * a.sin()]
    };
    for i in 0..SIDES {
        let mid = (i as f32 + 0.5) / SIDES as f32 * TAU;
        let n = normalise([mid.cos() * h / r * 2.0, 1.0, mid.sin() * h / r * 2.0]);
        m.quad(
            [
                rim(i, r, -0.02),
                rim(i + 1, r, -0.02),
                rim(i + 1, r * 0.55, h * 0.8),
                rim(i, r * 0.55, h * 0.8),
            ],
            n,
            colour,
        );
    }
    let top: Vec<[f32; 3]> = (0..=SIDES).map(|i| rim(i, r * 0.55, h * 0.8)).collect();
    m.fan([0.0, h, 0.0], &top, UP, mix(colour, hex(0x000000), 0.1));
}

/// One blade of grass: a ribbon from the ground at radius `r0` out along
/// angle `a`, rising `height` and arching `lean` metres outward, narrowing
/// from `width` to a point. Seen from both sides. The last segment goes
/// in `tips`, in `tip`'s colour, so it can be lit.
#[allow(clippy::too_many_arguments)]
fn blade(
    m: &mut MeshBuf,
    tips: &mut MeshBuf,
    a: f32,
    r0: f32,
    height: f32,
    lean: f32,
    width: f32,
    colours: [[f32; 4]; 3],
    tip: [f32; 4],
) {
    let [dx, dz] = [a.cos(), a.sin()];
    let side = [-dz, 0.0, dx];
    let at = |t: f32| {
        let out = r0 + lean * t * t;
        [dx * out, height * t, dz * out]
    };
    const SEGS: usize = 4;
    for k in 0..SEGS {
        let (t0, t1) = (k as f32 / SEGS as f32, (k + 1) as f32 / SEGS as f32);
        let (p0, p1) = (at(t0), at(t1));
        let (w0, w1) = (width * (1.0 - t0 * 0.85), width * (1.0 - t1 * 0.85));
        let w1 = if k + 1 == SEGS { 0.0 } else { w1 };
        let off =
            |p: [f32; 3], w: f32, s: f32| [p[0] + side[0] * w * s, p[1], p[2] + side[2] * w * s];
        let corners = [
            off(p0, w0, -1.0),
            off(p0, w0, 1.0),
            off(p1, w1, 1.0),
            off(p1, w1, -1.0),
        ];
        let up = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let n = normalise(cross(side, up));
        let (target, colour) = if k + 1 == SEGS {
            (&mut *tips, tip)
        } else {
            (&mut *m, colours[k.min(2)])
        };
        if w1 == 0.0 {
            let tri = [corners[0], corners[1], corners[2]];
            target.fan(tri[0], &[tri[1], tri[2]], n, colour);
            target.fan(tri[0], &[tri[1], tri[2]], neg(n), colour);
        } else {
            target.quad(corners, n, colour);
            target.quad(corners, neg(n), colour);
        }
    }
}

/// A short straight stalk with a flat cut top, seen from both sides.
fn stalk(m: &mut MeshBuf, a: f32, r0: f32, h: f32, width: f32, colours: [[f32; 4]; 3]) {
    let [dx, dz] = [a.cos(), a.sin()];
    let side = [-dz, 0.0, dx];
    let tilt = 0.03;
    let p0 = [dx * r0, 0.0, dz * r0];
    let p1 = [dx * (r0 + tilt), h + 0.04, dz * (r0 + tilt)];
    let off = |p: [f32; 3], s: f32| [p[0] + side[0] * width * s, p[1], p[2] + side[2] * width * s];
    let corners = [off(p0, -1.0), off(p0, 1.0), off(p1, 1.0), off(p1, -1.0)];
    let n = normalise(cross(side, [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]]));
    m.quad(corners, n, colours[1]);
    m.quad(corners, neg(n), colours[0]);
}

/// A head of seed: a slim spindle at the top of a stalk, leaning its way.
fn head(m: &mut MeshBuf, top: [f32; 3], [dx, dz]: [f32; 2], colour: [f32; 4]) {
    let len = 0.16;
    let base = [top[0] - dx * 0.01, top[1] - 0.02, top[2] - dz * 0.01];
    let tip = [top[0] + dx * 0.05, top[1] + len, top[2] + dz * 0.05];
    let mid = [
        (base[0] + tip[0]) * 0.5,
        (base[1] + tip[1]) * 0.5,
        (base[2] + tip[2]) * 0.5,
    ];
    const SIDES: usize = 5;
    let r = 0.025;
    for i in 0..SIDES {
        let a0 = i as f32 / SIDES as f32 * TAU;
        let a1 = (i + 1) as f32 / SIDES as f32 * TAU;
        let ring = |a: f32| [mid[0] + r * a.cos(), mid[1], mid[2] + r * a.sin()];
        let mida = (a0 + a1) * 0.5;
        let out = [mida.cos(), 0.0, mida.sin()];
        for end in [base, tip] {
            let n = normalise([out[0], if end == tip { 0.4 } else { -0.4 }, out[2]]);
            m.fan(end, &[ring(a0), ring(a1)], n, colour);
        }
    }
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
    fn a_tuft_is_a_nodes_size_and_stubble_is_low() {
        for lit in [false, true] {
            let (lo, hi) = bounds(&tuft(lit));
            assert!(
                lo[1] > -0.05 && lo[1] < 0.0,
                "sits in the ground: {}",
                lo[1]
            );
            assert!((1.0..1.45).contains(&hi[1]), "{} m tall", hi[1]);
            let across = (hi[0] - lo[0]).max(hi[2] - lo[2]);
            assert!((1.0..1.7).contains(&across), "{across} m across");
        }
        let (_, hi) = bounds(&stubble());
        assert!(hi[1] < 0.3, "stubble {} m tall", hi[1]);
    }

    #[test]
    fn only_the_lit_tuft_glows() {
        let glows = |m: &MeshBuf| {
            m.vertices
                .iter()
                .any(|v| v.normal == crate::world::mesh::GLOW)
        };
        assert!(!glows(&tuft(false)));
        assert!(glows(&tuft(true)));
        assert!(!glows(&stubble()));
    }

    #[test]
    fn every_face_faces_its_normal() {
        // So both sides of a blade, each wound its own way, are drawn.
        let m = tuft(false);
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
    fn the_quaternions_compose() {
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
    }
}
