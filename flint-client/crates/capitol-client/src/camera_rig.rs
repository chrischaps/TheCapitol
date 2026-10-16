//! Tests for the camera rig, `scripts/camera.rhai` (P3.9). The rig's
//! geometry is pure Rhai (`rig_pose`, `rig_pitch`, `rig_distance`), so
//! these load the script into a bare engine with only the `cap_*` API, and
//! drive it across every zoom, yaw and focus to check what it promises:
//! the tilt rides the zoom, the horizon stays out of frame once the fog has
//! thinned, the map is the whole world north up, and the camera never goes
//! into the land or the tower.

use crate::atmosphere::{CLEAR_FROM, FLAT_BY};
use crate::coords::layer::GROUND;
use crate::script_api::{self, Shared};
use crate::world::tower;
use capitol_protocol::terrain::WaterRing;
use rhai::{CallFnOptions, Engine, FuncArgs, Map, Scope, AST};
use std::path::Path;

/// The player's vertical field of view in degrees (flint-player's camera;
/// the scene's `[camera]` doesn't set one).
const VFOV: f64 = 70.0;
/// The camera's near plane (scene `[camera] near`): geometry nearer than
/// this is cut away, so the camera keeps at least this far from anything.
const NEAR: f32 = 1.0;
/// The world's ground fades out by here (P3.2), in metres.
const WORLD_R: f64 = 400.0;

struct Rig {
    engine: Engine,
    ast: AST,
}

struct Pose {
    p: [f32; 3],
    t: [f32; 3],
}

impl Rig {
    fn load() -> Self {
        let mut engine = Engine::new();
        // As flint-script sets it (engine.rs), or the deeper expressions
        // here fail to parse.
        engine.set_max_expr_depths(128, 128);
        script_api::register(&mut engine, &Shared::default());
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/camera.rhai");
        let ast = engine.compile_file(path).expect("camera.rhai compiles");
        Self { engine, ast }
    }

    fn call<T: Clone + Send + Sync + 'static>(&self, name: &str, args: impl FuncArgs) -> T {
        // Only the function: the script's top level is the live rig's state.
        let options = CallFnOptions::new().eval_ast(false);
        self.engine
            .call_fn_with_options(options, &mut Scope::new(), &self.ast, name, args)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn num(&self, name: &str) -> f64 {
        self.call(name, ())
    }

    fn pitch(&self, zoom: f64) -> f64 {
        self.call("rig_pitch", (zoom,))
    }

    fn pose(&self, fx: f64, fz: f64, zoom: f64, yaw: f64) -> Pose {
        let m: Map = self.call("rig_pose", (fx, fz, zoom, yaw));
        let get = |k: &str| m[k].as_float().unwrap() as f32;
        Pose {
            p: [get("px"), get("py"), get("pz")],
            t: [get("tx"), get("ty"), get("tz")],
        }
    }
}

fn zooms() -> impl Iterator<Item = f64> {
    (0..=200).map(|i| i as f64 / 200.0)
}

fn yaws() -> impl Iterator<Item = f64> {
    (0..16).map(|i| i as f64 * std::f64::consts::TAU / 16.0 - 3.0)
}

#[test]
fn the_tilt_rides_the_zoom() {
    let rig = Rig::load();
    let street = rig.num("street_zoom");
    assert!((rig.pitch(0.0) - 15.0).abs() < 1e-9);
    assert!((rig.pitch(street) - 50.0).abs() < 1e-9);
    assert!(rig.pitch(1.0) > 89.0);
    let mut last = 0.0;
    for zoom in zooms() {
        let p = rig.pitch(zoom);
        assert!(p >= last, "the camera tilts up as it zooms out at {zoom}");
        last = p;
    }
}

#[test]
fn the_street_is_fifty_degrees_down_and_twenty_five_metres_up() {
    // P3.6's plot culling test assumed this camera.
    let rig = Rig::load();
    let pose = rig.pose(0.0, 200.0, rig.num("street_zoom"), 0.0);
    let height = pose.p[1] - GROUND;
    assert!(
        (23.0..27.0).contains(&height),
        "street camera {height} m up"
    );
}

#[test]
fn the_horizon_stays_out_of_frame_once_the_fog_thins() {
    // Above CLEAR_FROM the fog thins, and a horizon in frame would show the
    // dark ground skirt, unfogged, against the sky. The horizon is in frame when
    // the tilt is less than half the vertical field of view.
    let rig = Rig::load();
    for zoom in zooms() {
        let pose = rig.pose(0.0, 200.0, zoom, 0.0);
        if pose.p[1] - GROUND > CLEAR_FROM - 10.0 {
            let p = rig.pitch(zoom);
            assert!(
                p > VFOV / 2.0 + 5.0,
                "zoom {zoom}: {p}° down at {} m",
                pose.p[1]
            );
        }
    }
}

#[test]
fn the_map_is_the_whole_world_north_up() {
    let rig = Rig::load();
    for (fx, fz, yaw) in [(0.0, 0.0, 0.0), (300.0, -200.0, 2.0), (-50.0, 380.0, -9.0)] {
        let pose = rig.pose(fx, fz, 1.0, yaw);
        let [px, py, pz] = pose.p;
        let [tx, _, tz] = pose.t;
        assert!(
            tx.abs() < 1e-3 && tz.abs() < 1e-3,
            "the map looks at the Capitol"
        );
        let height = (py - GROUND) as f64;
        assert!(height >= FLAT_BY as f64, "the light is flat by the map");
        let half_view = height * (VFOV / 2.0).to_radians().tan();
        assert!(half_view >= WORLD_R, "the map shows the whole world");
        // Straight down, a hair to the south: north is at the top.
        assert!((px - tx).abs() < 0.01, "yaw {yaw}: not north up ({px})");
        assert!(
            pz > tz && pz - tz < 2.0,
            "yaw {yaw}: not straight down ({pz})"
        );
    }
}

/// The tower's height at each horizontal radius (0.25 m bins), from the
/// tower built on the fixture's moat. Rotationally conservative: the
/// tallest thing at that radius at any angle.
fn tower_profile() -> Vec<f32> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
    let terrain = capitol_core::terrain::read_terrain_file(&path).expect("fixture");
    let rings: &[WaterRing] = &terrain.water_rings;
    let built = tower::build(&tower::Site::new(rings));
    let mut bins = vec![GROUND; 200];
    for buf in [&built.stone, &built.glass, &built.gold, &built.light] {
        for v in &buf.vertices {
            let [x, y, z] = v.position;
            let bin = ((x * x + z * z).sqrt() / 0.25) as usize;
            if let Some(top) = bins.get_mut(bin) {
                *top = top.max(y);
            }
        }
    }
    bins
}

#[test]
fn the_camera_never_goes_into_the_land_or_the_tower() {
    let rig = Rig::load();
    let profile = tower_profile();
    // Over the Capitol, the moat and its bridge, and out to the world's edge.
    let mut focus: Vec<(f64, f64)> = Vec::new();
    for i in -15..=15 {
        for j in -15..=15 {
            focus.push((i as f64 * 2.5, j as f64 * 2.5));
        }
    }
    focus.extend([(0.0, 60.0), (300.0, 0.0), (-283.0, 283.0)]);
    // By zoom 0.65 the camera is over the needle (90 m up), clear of it all.
    for zoom in zooms().take_while(|&z| z <= 0.7).step_by(8) {
        for yaw in yaws().step_by(2) {
            for &(fx, fz) in &focus {
                let pose = rig.pose(fx, fz, zoom, yaw);
                let [x, y, z] = pose.p;
                // Above the land and a bridge's rails (1.6 m), by the near plane.
                assert!(y >= GROUND + 1.6 + NEAR, "into the land at {:?}", pose.p);
                // Clear of the tower within the near plane on every side.
                let r = (x * x + z * z).sqrt();
                let lo = ((r - NEAR) / 0.25).max(0.0) as usize;
                let hi = ((r + NEAR) / 0.25) as usize;
                let top = profile[lo.min(profile.len())..(hi + 1).min(profile.len())]
                    .iter()
                    .fold(GROUND, |a, &b| a.max(b));
                assert!(
                    y >= top + NEAR,
                    "into the tower at {:?} (r {r:.1}, tower {top:.1} m) from focus ({fx}, {fz}) zoom {zoom:.2}",
                    pose.p
                );
            }
        }
    }
}

#[test]
fn the_login_circle_keeps_the_tower_right_of_the_form() {
    // The form covers the left third of a 16:9 window (ui/login.rs), so the
    // whole tower, foot to needle, must stand right of centre and in frame,
    // all the way round, with the fog still on.
    let rig = Rig::load();
    let half_v = (VFOV / 2.0).to_radians().tan();
    let half_h = half_v * 16.0 / 9.0;
    let lap = std::f64::consts::TAU / rig.num("login_rate");
    for i in 0..64 {
        let t = lap * i as f64 / 64.0;
        let shot: Map = rig.call("login_shot", (t,));
        let v = |k: &str| -> [f64; 3] {
            let a = shot[k].clone().into_typed_array::<f64>().unwrap();
            [a[0], a[1], a[2]]
        };
        let (p, at) = (v("pos"), v("at"));
        assert!(
            p[1] - (GROUND as f64) < CLEAR_FROM as f64,
            "fogless at {p:?}"
        );
        let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let norm = |a: [f64; 3]| {
            let l = dot(a, a).sqrt();
            [a[0] / l, a[1] / l, a[2] / l]
        };
        let f = norm(sub(at, p));
        let r = norm([-f[2], 0.0, f[0]]);
        let u = [
            r[1] * f[2] - r[2] * f[1],
            r[2] * f[0] - r[0] * f[2],
            r[0] * f[1] - r[1] * f[0],
        ];
        for y in [GROUND as f64, tower::CROWN_Y as f64] {
            let d = sub([0.0, y, 0.0], p);
            let x = dot(d, r) / dot(d, f) / half_h;
            let y_ndc = dot(d, u) / dot(d, f) / half_v;
            assert!(
                (0.2..0.7).contains(&x),
                "tower at screen x {x:.2} (ndc) from {p:?} at t {t:.0}"
            );
            assert!(y_ndc.abs() < 0.95, "tower out of frame at y {y} from {p:?}");
        }
    }
}
