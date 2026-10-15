//! The `cap_*` functions scripts call into Rust (plan §4.2). Registered by
//! `CapitolExtension::register_script_api`.
//!
//! Rhai runs with its `sync` feature, so every closure here and the state
//! it captures must be `Send + Sync`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
}

/// One avatar's name tag, as `entities::Players` placed it this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerLabel {
    pub name: String,
    /// Where the name hangs, over the head, in world metres.
    pub at: [f32; 3],
    /// 0 to 1 as the player fades in or out.
    pub alpha: f32,
    /// You.
    pub me: bool,
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
    // `cap_players() -> [#{ name, x, y, z, alpha, me }]`: every avatar's
    // name and where it hangs (world metres), with how far it has faded in.
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
    // `cap_ground_y() -> float`: the land's height in metres, for picking
    // and for drawing on the ground. The sea is at 0.
    engine.register_fn("cap_ground_y", || crate::coords::layer::GROUND as f64);
    // `cap_crown_y() -> float`: just above the tip of the Capitol's needle,
    // in metres, where the inscription hangs.
    engine.register_fn("cap_crown_y", || crate::world::tower::CROWN_Y as f64);
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
            alpha: 0.5,
            me: true,
        });
        let z: f64 = engine.eval("cap_follow()[2]").unwrap();
        assert_eq!(z, -2.0);
        let who: String = engine
            .eval(r#"let p = cap_players()[0]; `${p.name} ${p.alpha} ${p.me}`"#)
            .unwrap();
        assert_eq!(who, "Ada 0.5 true");
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
