//! Tests for the camera rig, `scripts/camera.rhai` (P3.9). The rig's
//! geometry is pure Rhai (`rig_pose`, `rig_pitch`, `rig_distance`), so
//! these load the script into a bare engine with only the `cap_*` API, and
//! drive it across every zoom, yaw and focus to check what it promises:
//! the tilt rides the zoom, the near end stands over your shoulder (E3.13),
//! the horizon stays out of frame once the fog has thinned, the map is the
//! whole world north up, and the camera never goes into the land or the
//! tower; and close in it pulls in out of walls, and the bazaar takes it
//! down to the shoulder and gives it back.

use crate::atmosphere::{CLEAR_FROM, FLAT_BY};
use crate::coords::layer::GROUND;
use crate::script_api::{self, Shared};
use crate::world::tower;
use capitol_protocol::rules::Zone;
use capitol_protocol::terrain::WaterRing;
use rhai::{CallFnOptions, Engine, FuncArgs, Map, Scope, AST};
use std::path::Path;

/// The player's vertical field of view in degrees (flint-player's camera;
/// the scene's `[camera]` doesn't set one).
const VFOV: f64 = 70.0;
/// The camera's near plane (scene `[camera] near`): geometry nearer than
/// this is cut away, so the camera keeps at least this far from anything.
const NEAR: f32 = 1.0;
/// The world's edge (r6800 since #308), in metres.
const WORLD_R: f64 = capitol_protocol::rules::WILDERNESS_OUTER_RADIUS * crate::coords::S as f64;

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
        Self::load_on(None)
    }

    /// The rig on the fixture's land (W9): `cap_ground_at` reads `field`
    /// rather than the game's land, which tests leave unset.
    fn load_on(field: Option<&'static flint_player::GroundField>) -> Self {
        let mut engine = Engine::new();
        // As flint-script sets it (engine.rs), or the deeper expressions
        // here fail to parse.
        engine.set_max_expr_depths(128, 128);
        script_api::register(&mut engine, &Shared::default());
        if let Some(field) = field {
            engine.register_fn("cap_ground_at", move |x: f64, z: f64| {
                let rise = crate::world::land::rise_on(field, [x as f32, z as f32]);
                (GROUND + rise) as f64
            });
        }
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
    let shoulder = rig.num("shoulder_zoom");
    assert!((rig.pitch(0.0) - 5.0).abs() < 1e-9, "nearly level at the shoulder");
    assert!((rig.pitch(shoulder) - 15.0).abs() < 1e-9, "as the dial used to end, at 11 m");
    assert!((rig.pitch(street) - 50.0).abs() < 1e-9);
    assert!(shoulder > 0.0 && shoulder < street);
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
    let shoulder = rig.num("shoulder_zoom");
    // By zoom 0.75 the camera is over the needle (90 m up), clear of it all.
    for zoom in zooms().take_while(|&z| z <= 0.75).step_by(8) {
        for yaw in yaws().step_by(2) {
            for &(fx, fz) in &focus {
                let pose = rig.pose(fx, fz, zoom, yaw);
                let [x, y, z] = pose.p;
                // Above the land and a bridge's rails (1.6 m), by the near
                // plane; at the shoulder (E3.13), at standing height, over
                // the rails beside you and looking along the deck.
                let least = if zoom < shoulder { 1.8 } else { 1.6 + NEAR };
                assert!(y >= GROUND + least, "into the land at {:?} zoom {zoom:.2}", pose.p);
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
fn in_the_wilds_the_camera_keeps_over_the_land_and_sees_the_point() {
    // W9: across the ranges, at every zoom up to the district and every
    // yaw, the camera stands clear of the ground under it, looks at the
    // ground under the point, and no hill stands in the way.
    let field = crate::world::land::tests::fixture_field();
    let rig = Rig::load_on(Some(field));
    let ground = |x: f32, z: f32| GROUND + crate::world::land::rise_on(field, [x, z]);
    let mut checked = 0;
    let mut risen = 0;
    // Across the wilds, from the foothills out to near the world's edge.
    let (inner, outer) = Zone::Wilderness.radii();
    for share in [0.2, 0.4, 0.6, 0.8, 0.95] {
        let r = (inner + share * (outer - inner)) * crate::coords::S as f64;
        for k in 0..24 {
            let a = k as f64 * std::f64::consts::TAU / 24.0;
            let (fx, fz) = (r * a.cos(), r * a.sin());
            for zoom in zooms().take_while(|&z| z <= 0.6).step_by(10) {
                for yaw in yaws().step_by(4) {
                    let pose = rig.pose(fx, fz, zoom, yaw);
                    let ([px, py, pz], [tx, ty, tz]) = (pose.p, pose.t);
                    // Past the shoulder's reach it looks at the ground; at
                    // the shoulder (E3.13), at chest height ahead of you
                    if zoom >= rig.num("shoulder_zoom") {
                        assert!((ty - ground(tx, tz)).abs() < 1e-3, "looks at the ground");
                    } else {
                        assert!(ty >= ground(fx as f32, fz as f32) - 1e-3, "looks over the ground");
                    }
                    assert!(py >= ground(px, pz) + 1.4, "into the land at {:?}", pose.p);
                    // The line from the camera to the point, a metre at a time.
                    let len = ((px - tx).powi(2) + (pz - tz).powi(2)).sqrt();
                    let n = (len as usize).max(2);
                    for i in 1..n {
                        let t = i as f32 / n as f32;
                        let (x, z) = (tx + (px - tx) * t, tz + (pz - tz) * t);
                        let y = ty + (py - ty) * t;
                        // Between the dozen places the rig checks, a little slack.
                        assert!(y >= ground(x, z) - 0.5, "a hill in the way from {:?}", pose.p);
                    }
                    checked += 1;
                    risen += (ty > GROUND + 5.0) as usize;
                }
            }
        }
    }
    assert!(risen * 3 > checked, "{risen} of {checked} on the hills");
}

#[test]
fn the_wild_shots_stand_on_their_ridge_and_see_the_tower() {
    // W11's shots, moved out with the land (#309): each stands a few metres
    // over the ground under it, not over the plain the ranges left, and
    // from-wilderness sees the tower over the land between.
    let field = crate::world::land::tests::fixture_field();
    let rig = Rig::load_on(Some(field));
    let ground = |x: f64, z: f64| (GROUND + crate::world::land::rise_on(field, [x as f32, z as f32])) as f64;
    for (view, over) in [("from-wilderness", 2.0..6.0), ("range", 10.0..20.0)] {
        let shot: Map = rig.call("view_shot", (view.to_string(),));
        let pos = shot["pos"].clone().into_typed_array::<f64>().unwrap();
        let rise = ground(pos[0], pos[2]) - GROUND as f64;
        assert!(rise > 40.0, "{view}: on the ranges, not the plain ({rise:.1} m up)");
        let stands = pos[1] - ground(pos[0], pos[2]);
        assert!(over.contains(&stands), "{view}: {stands:.1} m over the ground");
    }
    let shot: Map = rig.call("view_shot", ("from-wilderness".to_string(),));
    let [p, at] = ["pos", "at"].map(|k| shot[k].clone().into_typed_array::<f64>().unwrap());
    for i in 1..200 {
        let t = i as f64 / 200.0;
        let (x, z) = (p[0] + (at[0] - p[0]) * t, p[2] + (at[2] - p[2]) * t);
        let y = p[1] + (at[1] - p[1]) * t;
        assert!(y > ground(x, z), "a hill hides the tower at ({x:.0}, {z:.0})");
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

#[test]
fn a_shot_lets_go_with_what_it_framed_still_in_frame() {
    // Issue #92: the first camera input after a fixed shot hands over to
    // the rig, and what the shot was looking at must stay on screen, near
    // the middle, so a capture can orbit or zoom on from it. The same
    // hand-over ends a --camera-path.
    let rig = Rig::load();
    assert!(rig.num("framing_angle") < VFOV / 2.0 - 10.0);
    let views = [
        "bridge",
        "suburb",
        "urban",
        "capitol",
        "crown",
        "from-urban",
        "from-suburban",
        "from-rural",
        "from-wilderness",
        "range",
    ];
    let mut off = Vec::new();
    for view in views {
        let shot: Map = rig.call("view_shot", (view.to_string(),));
        let at = shot["at"].clone().into_typed_array::<f64>().unwrap();
        let mut state = rhai::Dynamic::from_map(Map::from_iter([("shot".into(), shot.into())]));
        let options = CallFnOptions::new()
            .eval_ast(false)
            .bind_this_ptr(&mut state);
        rig.engine
            .call_fn_with_options::<()>(
                options,
                &mut Scope::new(),
                &rig.ast,
                "take_over_from_shot",
                (true,),
            )
            .unwrap();
        let m = state.cast::<Map>();
        let f = |k: &str| m[k].as_float().unwrap();
        let pose = rig.pose(f("target_x"), f("target_z"), f("zoom"), f("yaw"));
        let p = pose.p.map(|x| x as f64);
        let look = [0, 1, 2].map(|i| pose.t[i] as f64 - p[i]);
        let to = [0, 1, 2].map(|i| at[i] - p[i]);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let angle = (dot(look, to) / (dot(look, look) * dot(to, to)).sqrt()).acos();
        off.push((view, angle.to_degrees().round()));
    }
    assert!(
        off.iter()
            .all(|&(_, a)| a <= rig.num("framing_angle") + 0.5),
        "degrees off the rig's centre: {off:?}"
    );
}

/// E3.13: at the near end the camera stands about 3 m behind you and 2 m
/// up, nearly level, a little to your right, looking past you at chest
/// height. You're in frame, left of the middle, and the street ahead fills
/// the rest.
#[test]
fn the_near_end_stands_over_your_shoulder() {
    let rig = Rig::load();
    for yaw in yaws() {
        let (fx, fz) = (0.0, 200.0);
        let pose = rig.pose(fx, fz, 0.0, yaw);
        let [px, py, pz] = pose.p;
        let [tx, ty, tz] = pose.t;
        let height = py - GROUND;
        assert!((1.8..2.4).contains(&height), "{height} m up");
        let behind = ((px - fx as f32).powi(2) + (pz - fz as f32).powi(2)).sqrt();
        assert!((2.6..3.6).contains(&behind), "{behind} m from you");
        // It looks ahead of you: further from the camera than you are
        let to_look = ((tx - px).powi(2) + (tz - pz).powi(2)).sqrt();
        assert!(to_look > behind, "looks past you");
        assert!((ty - GROUND - 1.6).abs() < 0.01, "at chest height");
        // You, at chest height, in frame and left of the middle
        let d = [fx as f32 - px, GROUND + 1.0 - py, fz as f32 - pz];
        let f = {
            let v = [tx - px, ty - py, tz - pz];
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            v.map(|x| x / l)
        };
        let r = {
            let v = [-f[2], 0.0, f[0]];
            let l = (v[0] * v[0] + v[2] * v[2]).sqrt();
            v.map(|x| x / l)
        };
        let along = d[0] * f[0] + d[1] * f[1] + d[2] * f[2];
        let half_h = (VFOV / 2.0).to_radians().tan() as f32 * 16.0 / 9.0;
        let x = (d[0] * r[0] + d[2] * r[2]) / along / half_h;
        assert!(along > NEAR + 1.0, "you're past the near plane: {along}");
        assert!((-0.6..-0.02).contains(&x), "you at screen x {x:.2}, yaw {yaw:.2}");
    }
}

#[test]
fn the_dial_runs_smoothly_through_the_shoulder() {
    let rig = Rig::load();
    let mut last: Option<Pose> = None;
    for zoom in zooms().take_while(|&z| z <= 0.6) {
        let pose = rig.pose(0.0, 200.0, zoom, 0.4);
        if let Some(prev) = &last {
            let step = (0..3).map(|i| (pose.p[i] - prev.p[i]).powi(2)).sum::<f32>().sqrt();
            let look = (0..3).map(|i| (pose.t[i] - prev.t[i]).powi(2)).sum::<f32>().sqrt();
            let d = (0..3).map(|i| (pose.p[i] - pose.t[i]).powi(2)).sum::<f32>().sqrt();
            assert!(step < 0.06 * d.max(4.0), "the camera jumps {step} m at zoom {zoom:.3}");
            assert!(look < 0.2, "the look jumps {look} m at zoom {zoom:.3}");
        }
        last = Some(pose);
    }
}

/// A rig of the script's, as the live one is, for the functions that keep
/// state (`this`).
fn with_this(rig: &Rig, state: &mut rhai::Dynamic, name: &str, args: impl FuncArgs) {
    let options = CallFnOptions::new().eval_ast(false).bind_this_ptr(state);
    rig.engine
        .call_fn_with_options::<()>(options, &mut Scope::new(), &rig.ast, name, args)
        .unwrap_or_else(|e| panic!("{name}: {e}"));
}

#[test]
fn a_wall_brings_the_camera_in_along_its_line_but_not_onto_you() {
    let rig = Rig::load();
    let pose = Map::from_iter([
        ("px".into(), 0.0_f64.into()),
        ("py".into(), 2.0_f64.into()),
        ("pz".into(), 4.0_f64.into()),
        ("tx".into(), 0.0_f64.into()),
        ("ty".into(), 2.0_f64.into()),
        ("tz".into(), 0.0_f64.into()),
    ]);
    let pulled = |share: f64| -> f64 {
        let m: Map = rig.call("pulled", (pose.clone(), share));
        m["pz"].as_float().unwrap()
    };
    assert_eq!(pulled(1.0), 4.0);
    assert!((pulled(0.5) - 2.0).abs() < 1e-9, "half way in");
    assert!((pulled(0.0) - rig.num("pull_least")).abs() < 1e-9, "never onto you");
    // Far out, walls don't pull it: 1 whatever stands about
    let head: rhai::Array = vec![0.0_f64.into(), 1.6_f64.into(), 0.0_f64.into()];
    let far: f64 = rig.call("clear_share", (pose.clone(), 40.0_f64, head));
    assert_eq!(far, 1.0);
    // Lifted, it looks at the same point from higher up
    let up: Map = rig.call("lifted", (pose.clone(), 1.5_f64));
    assert_eq!((up["py"].as_float().unwrap(), up["ty"].as_float().unwrap()), (3.5, 2.0));
}

/// The rig with `raycast_meshes` answering from `scene`, as Flint's does
/// from the meshes drawn as ray targets (ADR 0105).
fn with_walls(scene: flint_player::RayScene) -> Rig {
    let mut rig = Rig::load();
    rig.engine.register_fn(
        "raycast_meshes",
        move |ox: f64, oy: f64, oz: f64, dx: f64, dy: f64, dz: f64, max: f64| -> rhai::Dynamic {
            let p = |x: f64, y: f64, z: f64| [x as f32, y as f32, z as f32];
            match scene.cast(p(ox, oy, oz), p(dx, dy, dz), max as f32) {
                Some(hit) => rhai::Dynamic::from_map(Map::from_iter([("distance".into(), (hit.distance as f64).into())])),
                None => rhai::Dynamic::UNIT,
            }
        },
    );
    rig
}

/// A wall 2.2 m high across x = 1..1.5, as a stall's back wall stands.
fn a_wall() -> flint_player::RayScene {
    let (lo, hi) = ([1.0f32, 0.0, -5.0], [1.5f32, 2.2, 5.0]);
    let corner = |k: usize| {
        [
            if k & 1 == 0 { lo[0] } else { hi[0] },
            if k & 2 == 0 { lo[1] } else { hi[1] },
            if k & 4 == 0 { lo[2] } else { hi[2] },
        ]
    };
    let positions: Vec<[f32; 3]> = (0..8).map(corner).collect();
    let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let indices: Vec<u32> = quads.iter().flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]]).collect();
    let mut scene = flint_player::RayScene::new();
    let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let mesh = std::sync::Arc::new(flint_player::RayMesh::new(&positions, &indices));
    scene.add(flint_core::EntityId::from_raw(1), mesh, identity);
    scene
}

#[test]
fn the_camera_meets_a_wall_by_its_mesh_and_rises_over_it() {
    let rig = with_walls(a_wall());
    let point = |x: f64, y: f64, z: f64| -> rhai::Array { vec![x.into(), y.into(), z.into()] };
    let head = point(0.0, 1.6, 0.0);
    // Through the wall at its face, a quarter of the way out
    let share: f64 = rig.call("clear_line", (head.clone(), point(4.0, 1.6, 0.0)));
    assert!((share - 0.25).abs() < 1e-6, "{share}");
    // Short of it, over it, or along it, clear
    for to in [point(0.9, 1.6, 0.0), point(4.0, 6.5, 0.0), point(0.0, 1.6, 4.0)] {
        let share: f64 = rig.call("clear_line", (head.clone(), to.clone()));
        assert_eq!(share, 1.0, "{to:?}");
    }
    // Behind the wall, the camera rises until its line clears the top
    let pose = Map::from_iter([
        ("px".into(), 4.0_f64.into()),
        ("py".into(), 2.0_f64.into()),
        ("pz".into(), 0.0_f64.into()),
        ("tx".into(), 0.0_f64.into()),
        ("ty".into(), 1.6_f64.into()),
        ("tz".into(), 0.0_f64.into()),
    ]);
    let h: f64 = rig.call("rise_needed", (pose.clone(), head.clone()));
    let most = rig.num("rise_most");
    assert!(h > 0.0 && h <= most, "{h}");
    let cleared: f64 = rig.call("clear_line", (head.clone(), point(4.0, 2.0 + h, 0.0)));
    assert_eq!(cleared, 1.0, "risen {h} m");
    // And close in, the share of the way that's clear is the wall's
    let share: f64 = rig.call("clear_share", (pose, 4.0_f64, head));
    assert!((share - 0.25).abs() < 0.01, "{share}");
}

#[test]
fn walking_swings_the_camera_behind_you_close_in_only() {
    let rig = Rig::load();
    // As the script reads a look direction: camera at (sin, cos) from you
    let behind = |vx: f64, vz: f64| -> f64 { (-vx).atan2(-vz) };
    let step = |yaw: f64, zoom: f64, since: f64| -> f64 { rig.call("follow_yaw", (yaw, -1.5_f64, 0.0_f64, zoom, since, 0.1_f64)) };
    // Walking west at the shoulder: the yaw moves toward behind you
    let target = behind(-1.5, 0.0);
    let mut yaw = 0.0;
    for _ in 0..200 {
        yaw = step(yaw, 0.0, 10.0);
    }
    assert!((yaw - target).abs() < 0.01, "{yaw} toward {target}");
    // Not at street zoom, not just after you've turned it, not standing still
    assert_eq!(step(0.0, rig.num("street_zoom"), 10.0), 0.0);
    assert_eq!(step(0.0, 0.0, 0.5), 0.0);
    let still: f64 = rig.call("follow_yaw", (0.0_f64, 0.1_f64, 0.0_f64, 0.0_f64, 10.0_f64, 0.1_f64));
    assert_eq!(still, 0.0);
}

#[test]
fn the_bazaar_takes_the_view_down_and_gives_it_back() {
    let rig = Rig::load();
    let street = rig.num("street_zoom");
    let state = |zoom: f64| {
        rhai::Dynamic::from_map(Map::from_iter([
            ("zoom_to".into(), zoom.into()),
            ("in_bazaar".into(), false.into()),
            ("before".into(), rhai::Dynamic::UNIT),
            ("zoomed".into(), false.into()),
            ("arriving".into(), false.into()),
        ]))
    };
    let zoom_to = |s: &rhai::Dynamic| s.clone().cast::<Map>()["zoom_to"].as_float().unwrap();

    // In: down to the shoulder; out: back to the street
    let mut s = state(street);
    with_this(&rig, &mut s, "bazaar_ease", (true,));
    assert_eq!(zoom_to(&s), 0.0);
    with_this(&rig, &mut s, "bazaar_ease", (true,));
    assert_eq!(zoom_to(&s), 0.0, "once, as you come in");
    with_this(&rig, &mut s, "bazaar_ease", (false,));
    assert!((zoom_to(&s) - street).abs() < 1e-9);

    // A wheel turned inside counts: leaving keeps it
    let mut s = state(street);
    with_this(&rig, &mut s, "bazaar_ease", (true,));
    let mut m = s.cast::<Map>();
    m.insert("zoomed".into(), true.into());
    m.insert("zoom_to".into(), 0.12.into());
    let mut s = rhai::Dynamic::from_map(m);
    with_this(&rig, &mut s, "bazaar_ease", (false,));
    assert!((zoom_to(&s) - 0.12).abs() < 1e-9);

    // Already close in, it's left alone, coming and going
    let mut s = state(0.02);
    with_this(&rig, &mut s, "bazaar_ease", (true,));
    with_this(&rig, &mut s, "bazaar_ease", (false,));
    assert!((zoom_to(&s) - 0.02).abs() < 1e-9);
}
