//! A walker stranded in a pocket of the wilds walled by crests (#522).
//!
//! The land's flood ([`wilds::Land`]) knows which cells of the wilds a walker
//! can reach from the rural edge, and since #347 no wild node is planted
//! anywhere else. A walker can still end up outside them: a regenerate that
//! moves the land under someone, or a shift of the world that moves everyone
//! (#459). There the Flint client's walk graph has no way out for it
//! (`NoWay::Stranded`), so a citizen or a click goes straight, into the crest,
//! and is stopped "Too steep", again and again.
//!
//! So when a walk past the rural edge is stopped "Too steep", the tick asks
//! [`Reach::way_out`] whether the walker can walk out from where it stands.
//! If it can't, it's moved to the nearest reached ground and its walk goes
//! on from there, as `node_placement::fit_to_terrain` moves a node off a
//! road. A walker held against a cliff it could walk away from is left
//! where it is.
//!
//! The reach takes the land's whole grid and flood, under a second in
//! release on every core, so it's worked out on a thread of its own: at
//! startup, after a regenerate, or the first time a tick wants it. Until
//! it's ready, nobody is moved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use super::node_spawn::wilds::{Land, Reach};
use crate::models::TerrainConfig;

pub use super::node_spawn::wilds::WayOut;

/// The [`Reach`] of one terrain, worked out in the background.
#[derive(Default)]
pub struct WildReach {
    started: AtomicBool,
    reach: OnceLock<Reach>,
}

impl WildReach {
    /// Starts working out `terrain`'s reach on a thread of its own, unless
    /// it's already started. `terrain` must be the one this handle is for.
    pub fn start(self: &Arc<Self>, terrain: &TerrainConfig) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let terrain = terrain.clone();
        let handle = self.clone();
        std::thread::Builder::new()
            .name("wild-reach".into())
            .spawn(move || {
                let start = std::time::Instant::now();
                let _ = handle.reach.set(Land::new(&terrain).into_reach());
                tracing::info!("The wilds' reach worked out in {:.1}s", start.elapsed().as_secs_f64());
            })
            .expect("a thread for the wilds' reach");
    }

    /// The reach, once it's worked out.
    pub fn get(&self) -> Option<&Reach> {
        self.reach.get()
    }

    /// The reach, waiting for it: for the tests.
    #[cfg(test)]
    pub fn wait(&self) -> &Reach {
        self.reach.wait()
    }
}

/// The default world's reach, worked out once for every test that wants it.
#[cfg(test)]
pub(crate) fn default_reach() -> Arc<WildReach> {
    static REACH: OnceLock<Arc<WildReach>> = OnceLock::new();
    let reach = REACH.get_or_init(Arc::default).clone();
    reach.start(TerrainConfig::default_world());
    reach.wait();
    reach
}

/// A pocket of the east-south-east massif walled by crests, where the land's
/// flood doesn't reach (`wilds::tests::a_pocket_walled_by_crests_is_not_reached`)
/// and the Flint client's walk graph has no way in or out.
#[cfg(test)]
pub(crate) const POCKET: (f64, f64) = (15329.1, 11083.0);

#[cfg(test)]
mod tests {
    use capitol_protocol::fmath::StableMath;
    use capitol_protocol::rules::{CENTER_X, CENTER_Y};

    use super::*;

    #[test]
    fn a_walker_in_a_pocket_is_lifted_to_the_nearest_reached_ground() {
        let reach = default_reach();
        let WayOut::Lift(out) = reach.wait().way_out(POCKET) else {
            panic!("{POCKET:?} is in a pocket: {:?}", reach.wait().way_out(POCKET));
        };
        let off = (out.0 - POCKET.0).stable_hypot(out.1 - POCKET.1);
        eprintln!("lifted from {POCKET:?} to {out:?}, {off:.0} away");
        assert!(off < 10.0 * super::super::node_spawn::wilds::CELL, "{off}");
        assert_eq!(reach.wait().way_out(out), WayOut::Walks, "out at {out:?}");
    }

    #[test]
    fn a_walker_who_can_walk_out_is_left_be() {
        let reach = default_reach();
        let reach = reach.wait();
        // In the city, and at wild nodes far round the crests (#521)
        for at in [(CENTER_X, CENTER_Y), (15678.2, 5647.9), (15850.2, 5970.6), (3050.8, 4418.3)] {
            assert_eq!(reach.way_out(at), WayOut::Walks, "{at:?}");
        }
        assert_eq!(reach.way_out((-50.0, 100.0)), WayOut::Lost);
    }
}
