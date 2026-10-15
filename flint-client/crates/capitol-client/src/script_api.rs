//! The `cap_*` functions scripts call into Rust (plan §4.2). Registered by
//! `CapitolExtension::register_script_api`.
//!
//! Rhai runs with its `sync` feature, so every closure here and the state
//! it captures must be `Send + Sync`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// State the Rust side writes and scripts read.
#[derive(Clone, Default)]
pub struct Shared {
    /// The inscription over the Capitol (`--note`).
    pub note: Arc<Mutex<String>>,
    /// The camera view the script frames (`--view`).
    pub view: Arc<Mutex<String>>,
    /// Mouse-wheel notches since the camera rig last took them, up = in.
    /// A workaround: Flint doesn't feed the wheel to actions (issue #67).
    pub wheel: Arc<Mutex<f32>>,
    /// The login form is up (P4.1): the camera circles the tower behind it.
    pub at_login: Arc<AtomicBool>,
    /// Where you stand in world metres (P4.2), for the camera to follow.
    /// `None` offline, and until the server has said where you are.
    pub follow: Arc<Mutex<Option<[f32; 3]>>>,
    /// The name over each avatar this frame (P4.2), for `hud.rhai`.
    pub players: Arc<Mutex<Vec<PlayerLabel>>>,
    /// Where a script last asked to walk (`cap_move`, P4.3), in server
    /// units, clamped to the world. The extension takes it next frame.
    pub move_to: Arc<Mutex<Option<[f64; 2]>>>,
    /// Where you're walking to (P4.3), in world metres, for the marker.
    /// `None` when standing, and offline.
    pub destination: Arc<Mutex<Option<[f32; 2]>>>,
    /// The zone you last entered (P4.5), for the banner in `hud.rhai`.
    pub banner: Arc<Mutex<Option<ZoneBanner>>>,
    /// Every resource node shown this frame (P4.4), as `nodes::Nodes` left
    /// them, for picking and the HUD.
    pub nodes: Arc<Mutex<Vec<NodeMark>>>,
    /// The node a script last asked to extract from (`cap_extract`). The
    /// extension takes it next frame.
    pub extract: Arc<Mutex<Option<String>>>,
    /// Every station shown this frame (P6.1), as `stations::Stations` left
    /// them, for the in-range ring.
    pub stations: Arc<Mutex<Vec<StationMark>>>,
}

/// The zone banner's words and colour, and when it started.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneBanner {
    pub name: String,
    /// sRGB, 0 to 1.
    pub colour: [f32; 3],
    /// When the banner starts; it may be a moment ahead.
    pub at: Instant,
    /// The zone you came into the world in, not one you walked into.
    pub first: bool,
}

/// One resource node as scripts see it.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeMark {
    pub id: String,
    /// Where it stands, world metres (X, Z).
    pub at: [f32; 2],
    /// "available", "harvesting" or "depleted".
    pub state: &'static str,
    /// You're walking to it to extract, or have asked to.
    pub targeted: bool,
    /// You're extracting it.
    pub mine: bool,
    /// How far the extraction on it has got, 0 to 1, if one is under way.
    pub progress: Option<f32>,
}

/// One station as scripts see it.
#[derive(Debug, Clone, PartialEq)]
pub struct StationMark {
    pub id: String,
    /// "workbench", "forge", "storage_chest", or "generic" for a type the
    /// client has no model for.
    pub kind: &'static str,
    /// Where it stands, world metres (X, Z).
    pub at: [f32; 2],
    /// The in-range ring's radius, metres: just round the model.
    pub radius: f32,
    /// You're within its interaction range (`capitol_core::station`).
    pub in_reach: bool,
    /// 0 to 1: the ring, eased toward `in_reach`.
    pub reach: f32,
}

/// How near a click must land to a node to pick it, in metres. A node's
/// tuft is about 1.2 m across; React picks within 16 server units.
pub const NODE_PICK: f32 = 1.8;

/// One avatar's name tag, as `entities::Players` placed it this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerLabel {
    pub name: String,
    /// Where the name hangs, over the head, in world metres.
    pub at: [f32; 3],
    /// Where they stand: the height of their feet, in world metres.
    pub feet_y: f32,
    /// 0 to 1 as the player fades in or out.
    pub alpha: f32,
    /// You.
    pub me: bool,
    /// How far their craft has got, 0 to 1, while they craft (P5.5).
    pub craft: Option<f32>,
}

pub fn register(engine: &mut rhai::Engine, shared: &Shared) {
    // `cap_spike_note() -> String`: the text written over the Capitol.
    let note = shared.note.clone();
    engine.register_fn("cap_spike_note", move || -> String {
        note.lock().map(|s| s.clone()).unwrap_or_default()
    });
    // `cap_view() -> String`: which view the camera rig starts on.
    let view = shared.view.clone();
    engine.register_fn("cap_view", move || -> String {
        view.lock().map(|s| s.clone()).unwrap_or_default()
    });
    // `cap_take_wheel() -> float`: wheel notches (up positive) since the
    // last call, which zeroes them. Only the camera rig should call it.
    let wheel = shared.wheel.clone();
    engine.register_fn("cap_take_wheel", move || -> f64 {
        wheel
            .lock()
            .map(|mut w| std::mem::take(&mut *w) as f64)
            .unwrap_or_default()
    });
    // `cap_at_login() -> bool`: the login form is up, so the camera rig
    // circles the tower instead of taking input.
    let at_login = shared.at_login.clone();
    engine.register_fn("cap_at_login", move || at_login.load(Ordering::Relaxed));
    // `cap_follow() -> [x, y, z] or ()`: where you stand, in world metres,
    // for the camera rig. () offline, and until the server has placed you.
    let follow = shared.follow.clone();
    engine.register_fn("cap_follow", move || -> rhai::Dynamic {
        match follow.lock().ok().and_then(|f| *f) {
            Some(p) => rhai::Dynamic::from_array(p.iter().map(|v| (*v as f64).into()).collect()),
            None => rhai::Dynamic::UNIT,
        }
    });
    // `cap_players() -> [#{ name, x, y, z, feet_y, alpha, me, craft }]`:
    // every avatar's name and where it hangs (world metres), the height of
    // its feet, how far it has faded in, and how far its craft has got
    // (0 to 1, or () when it isn't crafting).
    let players = shared.players.clone();
    engine.register_fn("cap_players", move || -> rhai::Array {
        let Ok(players) = players.lock() else {
            return rhai::Array::new();
        };
        players
            .iter()
            .map(|p| {
                let mut m = rhai::Map::new();
                m.insert("name".into(), p.name.clone().into());
                m.insert("x".into(), (p.at[0] as f64).into());
                m.insert("y".into(), (p.at[1] as f64).into());
                m.insert("z".into(), (p.at[2] as f64).into());
                m.insert("alpha".into(), (p.alpha as f64).into());
                m.insert("me".into(), p.me.into());
                m.insert("feet_y".into(), (p.feet_y as f64).into());
                m.insert(
                    "craft".into(),
                    p.craft.map_or(rhai::Dynamic::UNIT, |c| (c as f64).into()),
                );
                m.into()
            })
            .collect()
    });
    // `cap_move(x, z)`: walk to the ground point (x, z), in world metres.
    // It's clamped to the world's edge and sent next frame. Offline, and
    // before the server has placed you, it does nothing.
    let move_to = shared.move_to.clone();
    engine.register_fn("cap_move", move |x: f64, z: f64| {
        if let Ok(mut m) = move_to.lock() {
            *m = Some(clamp_to_world(x as f32, z as f32));
        }
    });
    // `cap_destination() -> [x, z] or ()`: where you're walking to, in
    // world metres. () when you're standing, or following a walk the
    // server resumed after a reconnect.
    let destination = shared.destination.clone();
    engine.register_fn("cap_destination", move || -> rhai::Dynamic {
        match destination.lock().ok().and_then(|d| *d) {
            Some(p) => rhai::Dynamic::from_array(p.iter().map(|v| (*v as f64).into()).collect()),
            None => rhai::Dynamic::UNIT,
        }
    });
    // `cap_zone_banner() -> #{ name, r, g, b, age, first } or ()`: the zone
    // you last entered, its colour (sRGB) and how many seconds ago its
    // banner started. `age` is negative while it waits to start. `first`
    // when it's the zone you came into the world in. () offline, and
    // signed out.
    let banner = shared.banner.clone();
    engine.register_fn("cap_zone_banner", move || -> rhai::Dynamic {
        let Some(b) = banner.lock().ok().and_then(|b| b.clone()) else {
            return rhai::Dynamic::UNIT;
        };
        let now = Instant::now();
        let age = if now >= b.at {
            (now - b.at).as_secs_f64()
        } else {
            -(b.at - now).as_secs_f64()
        };
        let mut m = rhai::Map::new();
        m.insert("name".into(), b.name.into());
        m.insert("r".into(), (b.colour[0] as f64).into());
        m.insert("g".into(), (b.colour[1] as f64).into());
        m.insert("b".into(), (b.colour[2] as f64).into());
        m.insert("age".into(), age.into());
        m.insert("first".into(), b.first.into());
        m.into()
    });
    // `cap_nodes() -> [#{ id, x, z, state, targeted, mine, progress }]`:
    // the resource nodes shown, at ground level in world metres. `state` is
    // "available", "harvesting" or "depleted"; `progress` is 0 to 1 while
    // someone extracts, otherwise ().
    let nodes = shared.nodes.clone();
    engine.register_fn("cap_nodes", move || -> rhai::Array {
        let Ok(nodes) = nodes.lock() else {
            return rhai::Array::new();
        };
        nodes.iter().map(|n| node_map(n).into()).collect()
    });
    // `cap_stations() -> [#{ id, kind, x, z, radius, in_reach, reach }]`:
    // the stations shown, in world metres. `in_reach` is whether you're
    // within its interaction range; `reach` eases from 0 to 1 with it.
    let stations = shared.stations.clone();
    engine.register_fn("cap_stations", move || -> rhai::Array {
        let Ok(stations) = stations.lock() else {
            return rhai::Array::new();
        };
        stations.iter().map(|s| station_map(s).into()).collect()
    });
    // `cap_node_at(x, z) -> #{ ... } or ()`: the node nearest the ground
    // point (x, z), within NODE_PICK metres, as in `cap_nodes`.
    let nodes = shared.nodes.clone();
    engine.register_fn("cap_node_at", move |x: f64, z: f64| -> rhai::Dynamic {
        let Ok(nodes) = nodes.lock() else {
            return rhai::Dynamic::UNIT;
        };
        match node_at(&nodes, x as f32, z as f32) {
            Some(n) => node_map(n).into(),
            None => rhai::Dynamic::UNIT,
        }
    });
    // `cap_extract(id)`: extract from the node `id` (from `cap_node_at`):
    // at once within range, otherwise after walking there. Sent next frame.
    let extract = shared.extract.clone();
    engine.register_fn("cap_extract", move |id: &str| {
        if let Ok(mut e) = extract.lock() {
            *e = Some(id.to_owned());
        }
    });
    // `cap_ground_y() -> float`: the land's height in metres, for picking
    // and for drawing on the ground. The sea is at 0.
    engine.register_fn("cap_ground_y", || crate::coords::layer::GROUND as f64);
    // `cap_crown_y() -> float`: just above the tip of the Capitol's needle,
    // in metres, where the inscription hangs.
    engine.register_fn("cap_crown_y", || crate::world::tower::CROWN_Y as f64);
}

fn node_map(n: &NodeMark) -> rhai::Map {
    let mut m = rhai::Map::new();
    m.insert("id".into(), n.id.clone().into());
    m.insert("x".into(), (n.at[0] as f64).into());
    m.insert("z".into(), (n.at[1] as f64).into());
    m.insert("state".into(), n.state.into());
    m.insert("targeted".into(), n.targeted.into());
    m.insert("mine".into(), n.mine.into());
    m.insert(
        "progress".into(),
        n.progress
            .map_or(rhai::Dynamic::UNIT, |p| (p as f64).into()),
    );
    m
}

fn station_map(s: &StationMark) -> rhai::Map {
    let mut m = rhai::Map::new();
    m.insert("id".into(), s.id.clone().into());
    m.insert("kind".into(), s.kind.into());
    m.insert("x".into(), (s.at[0] as f64).into());
    m.insert("z".into(), (s.at[1] as f64).into());
    m.insert("radius".into(), (s.radius as f64).into());
    m.insert("in_reach".into(), s.in_reach.into());
    m.insert("reach".into(), (s.reach as f64).into());
    m
}

/// The node nearest (x, z), within [`NODE_PICK`].
pub fn node_at(nodes: &[NodeMark], x: f32, z: f32) -> Option<&NodeMark> {
    nodes
        .iter()
        .map(|n| (n, (n.at[0] - x).hypot(n.at[1] - z)))
        .filter(|(_, d)| *d <= NODE_PICK)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(n, _)| n)
}

/// World (X, Z) in metres to the server point a move goes to: inside the
/// world, 0 to `WORLD_SIZE` on both axes, as the React client clamps it.
pub fn clamp_to_world(wx: f32, wz: f32) -> [f64; 2] {
    use capitol_protocol::rules::WORLD_SIZE;
    let [x, y] = crate::coords::to_server(wx, wz);
    [x.clamp(0.0, WORLD_SIZE), y.clamp(0.0, WORLD_SIZE)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_read_the_note() {
        let shared = Shared::default();
        *shared.note.lock().unwrap() = "Ad astra".into();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        let note: String = engine.eval("cap_spike_note()").unwrap();
        assert_eq!(note, "Ad astra");
    }

    #[test]
    fn scripts_read_where_you_stand_and_who_is_about() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>("cap_follow()").is_ok());
        *shared.follow.lock().unwrap() = Some([1.0, 0.7, -2.0]);
        shared.players.lock().unwrap().push(PlayerLabel {
            name: "Ada".into(),
            at: [1.0, 2.9, -2.0],
            feet_y: 0.65,
            alpha: 0.5,
            me: true,
            craft: Some(0.25),
        });
        let z: f64 = engine.eval("cap_follow()[2]").unwrap();
        assert_eq!(z, -2.0);
        let who: String = engine
            .eval(r#"let p = cap_players()[0]; `${p.name} ${p.alpha} ${p.me}`"#)
            .unwrap();
        assert_eq!(who, "Ada 0.5 true");
        let craft: f64 = engine.eval("cap_players()[0].craft").unwrap();
        assert_eq!(craft, 0.25);
    }

    /// Every script parses, so a typo fails here rather than in a capture.
    /// (Parsing only: the engine's own functions aren't registered.)
    #[test]
    fn the_scripts_parse() {
        let mut engine = rhai::Engine::new();
        engine.set_max_expr_depths(128, 128); // as flint-script sets it
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
        let mut parsed = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rhai") {
                if let Err(e) = engine.compile_file(path.clone()) {
                    panic!("{}: {e}", path.display());
                }
                parsed += 1;
            }
        }
        assert!(parsed >= 4, "only {parsed} scripts in {}", dir.display());
    }

    #[test]
    fn scripts_read_the_zone_banner() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>("cap_zone_banner()").is_ok());
        *shared.banner.lock().unwrap() = Some(ZoneBanner {
            name: "Trade District".into(),
            colour: [0.25, 0.5, 1.0],
            at: Instant::now() + std::time::Duration::from_secs(60),
            first: true,
        });
        let words: String = engine
            .eval(r#"let b = cap_zone_banner(); `${b.name} ${b.b} ${b.first} ${b.age < -59.0}`"#)
            .unwrap();
        assert_eq!(words, "Trade District 1.0 true true");
    }

    #[test]
    fn the_wheel_is_taken_once() {
        let shared = Shared::default();
        *shared.wheel.lock().unwrap() = 2.5;
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        let first: f64 = engine.eval("cap_take_wheel()").unwrap();
        let second: f64 = engine.eval("cap_take_wheel()").unwrap();
        assert_eq!((first, second), (2.5, 0.0));
    }

    #[test]
    fn a_move_is_clamped_to_the_world_and_the_last_one_wins() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        engine
            .run("cap_move(5.0, -2.5); cap_move(1000.0, -1000.0);")
            .unwrap();
        assert_eq!(*shared.move_to.lock().unwrap(), Some([8000.0, 0.0]));
        let [x, y] = clamp_to_world(5.0, -2.5);
        assert!((x - 4050.0).abs() < 1e-3 && (y - 3975.0).abs() < 1e-3);
    }

    #[test]
    fn a_click_picks_the_nearest_node_and_asks_to_extract_it() {
        let shared = Shared::default();
        let mark = |id: &str, x: f32| NodeMark {
            id: id.into(),
            at: [x, 0.0],
            state: "available",
            targeted: false,
            mine: false,
            progress: None,
        };
        *shared.nodes.lock().unwrap() = vec![mark("a", 0.0), mark("b", 2.0)];
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        let picked: String = engine
            .eval("let n = cap_node_at(1.2, 0.3); cap_extract(n.id); n.id")
            .unwrap();
        assert_eq!(picked, "b");
        assert_eq!(shared.extract.lock().unwrap().as_deref(), Some("b"));
        assert!(engine.eval::<()>("cap_node_at(9.0, 9.0)").is_ok());
        let p: bool = engine.eval("cap_nodes()[0].progress == ()").unwrap();
        assert!(p);
    }

    #[test]
    fn scripts_read_where_you_are_walking_to() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>("cap_destination()").is_ok());
        *shared.destination.lock().unwrap() = Some([3.0, -4.0]);
        let z: f64 = engine.eval("cap_destination()[1]").unwrap();
        assert_eq!(z, -4.0);
    }
}
