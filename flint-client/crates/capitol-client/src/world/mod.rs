//! The world built from `/terrain` (plan P3): each piece is a runtime mesh
//! (H4) rebuilt when the terrain changes. So far that's the ground (P3.2),
//! the water (P3.3), the roads (P3.4), the bridges (P3.5) and the plots
//! (P3.6) and the Capitol tower (P3.7). A layer with thousands of pieces is
//! split into `sector`s, one mesh each, so each part culls on its own.
//!
//! When the terrain changes while the game runs (P3.10), [`Rebuild`] says
//! which layers to build again: only those whose inputs changed.

pub mod bridges;
pub mod geom;
pub mod ground;
pub mod mesh;
pub mod palette;
pub mod plots;
pub mod roads;
pub mod sector;
pub mod tower;
pub mod water;

use capitol_core::protocol::terrain::TerrainResponse;

/// Which layers a new terrain changes. Each layer reads:
///
/// | Layer | Inputs |
/// |---|---|
/// | ground | world size, water rings (its holes) |
/// | water, bridges, tower | water rings |
/// | roads | roads, water rings (they stop at the kerbs) |
/// | plots | plots, roads, water rings (lots are clipped clear of both) |
///
/// The centre isn't an input: `coords` places the world on the protocol's
/// fixed `CENTER_X`/`CENTER_Y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rebuild {
    pub ground: bool,
    /// The water, the bridges and the tower.
    pub rings: bool,
    pub roads: bool,
    pub plots: bool,
}

impl Rebuild {
    pub const ALL: Rebuild = Rebuild {
        ground: true,
        rings: true,
        roads: true,
        plots: true,
    };

    /// What to build for `new`, when the world was last built from `old`.
    pub fn between(old: Option<&TerrainResponse>, new: &TerrainResponse) -> Rebuild {
        let Some(old) = old else {
            return Rebuild::ALL;
        };
        let rings = old.water_rings != new.water_rings;
        let roads = rings || old.roads != new.roads;
        Rebuild {
            ground: rings || old.world_size != new.world_size,
            rings,
            roads,
            plots: roads || old.terrain_plots != new.terrain_plots,
        }
    }
}

impl std::fmt::Display for Rebuild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let layers = [
            (self.ground, "ground"),
            (self.rings, "water, bridges, tower"),
            (self.roads, "roads"),
            (self.plots, "plots"),
        ];
        let names: Vec<_> = layers
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, n)| *n)
            .collect();
        if names.is_empty() {
            f.write_str("nothing")
        } else {
            f.write_str(&names.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::terrain::read_terrain_file;
    use std::path::Path;

    fn fixture() -> TerrainResponse {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        read_terrain_file(&path).unwrap()
    }

    const NONE: Rebuild = Rebuild {
        ground: false,
        rings: false,
        roads: false,
        plots: false,
    };

    #[test]
    fn the_first_terrain_builds_everything() {
        assert_eq!(Rebuild::between(None, &fixture()), Rebuild::ALL);
    }

    #[test]
    fn the_same_terrain_builds_nothing() {
        let old = fixture();
        let mut new = old.clone();
        new.version = Some("another".into());
        let rebuild = Rebuild::between(Some(&old), &new);
        assert_eq!(rebuild, NONE);
        assert_eq!(rebuild.to_string(), "nothing");
    }

    #[test]
    fn each_input_rebuilds_only_what_reads_it() {
        let old = fixture();

        let mut new = old.clone();
        new.terrain_plots.pop();
        let plots_only = Rebuild {
            plots: true,
            ..NONE
        };
        assert_eq!(Rebuild::between(Some(&old), &new), plots_only);

        let mut new = old.clone();
        new.roads[0].width += 1.0;
        let roads = Rebuild {
            roads: true,
            plots: true,
            ..NONE
        };
        assert_eq!(Rebuild::between(Some(&old), &new), roads);
        assert_eq!(roads.to_string(), "roads, plots");

        let mut new = old.clone();
        new.world_size += 100.0;
        let ground = Rebuild {
            ground: true,
            ..NONE
        };
        assert_eq!(Rebuild::between(Some(&old), &new), ground);

        let mut new = old.clone();
        new.water_rings[0].width += 1.0;
        assert_eq!(Rebuild::between(Some(&old), &new), Rebuild::ALL);
    }
}
