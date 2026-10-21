//! Server space <-> Flint world space (plan §4.6). Every conversion goes
//! through here.
//!
//! Server space runs 0..13600 on both axes, centre (6800, 6800), y pointing
//! down the screen (south), angle 0 = east and 90 = south.
//!
//! Flint is right-handed and Y-up: its look-at builds `right = forward × up`,
//! so a camera looking toward -Z has +X on its right (verified in P0.4,
//! `flint-render/src/camera.rs` `view_matrix`). Mapping server x to +X and
//! server y to +Z therefore keeps the map unmirrored: seen from above with
//! north (-Z) at the top of the screen, east is on the right, exactly as in
//! the 2D clients.

use capitol_protocol::rules::{CENTER_X, CENTER_Y};

/// World metres per server unit. The world is 1360 m across, a player
/// (radius 8) is about 1.6 m wide, and a 60-unit arterial is 6 m.
pub const S: f32 = 0.1;

/// Heights of the flat layers in metres (plan §4.6). Everything flat lies
/// a hair above the layer under it so nothing z-fights. Bridge decks (P3.5)
/// rise from the roads over the kerbs to `DECK`, on arches that spring from
/// `ARCH_SPRING`, under the waves.
///
/// The land stands above the sea. The water is Flint's Gerstner ocean
/// (the one Float drifts on), an endless sheet at y = 0 that follows the
/// camera. The ground hides it everywhere except where the canals are cut
/// through, so a canal is a stone-walled channel with the sea showing at
/// its bottom: a water table. Steps between layers are at least 2 cm,
/// which the depth buffer resolves from the map view (plan §4.6).
pub mod layer {
    use capitol_protocol::terrain::RoadType;

    /// The ocean's rest level, fixed by the engine.
    pub const SEA: f32 = 0.0;
    /// The floor of a canal, under the water and seen through it.
    pub const CHANNEL_BED: f32 = -1.6;
    /// The land, high enough above the sea that no wave reaches it.
    pub const GROUND: f32 = 0.6;
    /// The lowest road layer; see [`road`].
    pub const ROAD: f32 = GROUND + 0.04;
    /// The top of a lot (P3.6), a step above the highest road, like a
    /// pavement. The server clears lots off the roads (B7), so they never
    /// overlap; each has a curb a few centimetres higher round its edge.
    pub const PLOT: f32 = GROUND + 0.12;
    /// Where the scripts lay marks on the ground (rings, paths, the
    /// reticle) now that they're depth-tested world lines (#86): a step
    /// over the lots, so roads, lots and the gold on your own don't hide
    /// them, and under every curb's lip, so a ring crossing one passes
    /// behind it as paint would. Over bare land it floats 16 cm up.
    pub const MARK: f32 = PLOT + 0.04;
    /// The paving inside the moat (P3.7), level with the arterials, so the
    /// moat bridge's ramp lands on it 2 cm up, as it does on a road.
    pub const BAILEY: f32 = ROAD + 0.06;
    /// The top of the stone kerb that rims every canal.
    pub const BANK: f32 = GROUND + 0.3;
    /// The crown of a bridge's deck (P3.5), high enough over the kerbs
    /// that its ramps clear them.
    pub const DECK: f32 = BANK + 0.4;
    /// Where a bridge's arch springs from the channel walls, just under the
    /// waves so no trough shows a gap.
    pub const ARCH_SPRING: f32 = SEA - 0.2;

    /// A road's height: trail < suburban < grid < arterial, so where two
    /// cross, the greater road is drawn on top. The ranks are 2 cm apart,
    /// like every other layer: 5 mm is finer than the depth buffer resolves
    /// from the map view. Arterials top out 10 cm above the land, well
    /// under the kerb.
    pub fn road(road_type: RoadType) -> f32 {
        let rank = match road_type {
            RoadType::Trail => 0.0,
            RoadType::Suburban => 1.0,
            RoadType::Grid => 2.0,
            RoadType::Arterial => 3.0,
        };
        ROAD + rank * 0.02
    }
}

/// Server (x, y) to world (X, Z) on the ground plane.
pub fn to_world(x: f64, y: f64) -> [f32; 2] {
    [((x - CENTER_X) as f32) * S, ((y - CENTER_Y) as f32) * S]
}

/// World (X, Z) back to server (x, y), for a click on the ground (P4.3).
pub fn to_server(wx: f32, wz: f32) -> [f64; 2] {
    [(wx / S) as f64 + CENTER_X, (wz / S) as f64 + CENTER_Y]
}

/// A server polar position (radius in server units, angle in degrees, both
/// about the world centre) to world (X, Z).
pub fn polar_to_world(radius: f64, angle_degrees: f64) -> [f32; 2] {
    let a = angle_degrees.to_radians();
    to_world(CENTER_X + radius * a.cos(), CENTER_Y + radius * a.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn centre_is_origin() {
        assert!(close(to_world(CENTER_X, CENTER_Y), [0.0, 0.0]));
    }

    #[test]
    fn east_is_plus_x_and_south_is_plus_z() {
        assert!(close(polar_to_world(200.0, 0.0), [20.0, 0.0]));
        assert!(close(polar_to_world(200.0, 90.0), [0.0, 20.0]));
        assert!(close(polar_to_world(200.0, 270.0), [0.0, -20.0]));
    }

    #[test]
    fn a_server_step_south_or_east_is_a_world_step_along_plus_z_or_plus_x() {
        let [x0, z0] = to_world(CENTER_X, CENTER_Y);
        let [x1, z1] = to_world(CENTER_X + 10.0, CENTER_Y); // east
        let [x2, z2] = to_world(CENTER_X, CENTER_Y + 10.0); // south (server y down)
        assert!(close([x1 - x0, z1 - z0], [10.0 * S, 0.0]));
        assert!(close([x2 - x0, z2 - z0], [0.0, 10.0 * S]));
        // And back: +X is east, -Z is north.
        let [x, y] = to_server(5.0, -5.0);
        assert!(x > CENTER_X && y < CENTER_Y);
    }

    #[test]
    fn polar_round_trips_through_server_space() {
        for (r, deg) in [(0.0, 0.0), (600.0, 45.0), (2400.0, 200.0), (capitol_protocol::rules::WILDERNESS_OUTER_RADIUS - 1.0, 359.0)] {
            let [wx, wz] = polar_to_world(r, deg);
            let [x, y] = to_server(wx, wz);
            let (dx, dy) = (x - CENTER_X, y - CENTER_Y);
            assert!((dx.hypot(dy) - r).abs() < 0.05, "r {r}");
        }
    }

    #[test]
    fn layers_stack_sea_ground_roads_by_rank_plots_then_bank_then_deck() {
        use capitol_protocol::terrain::RoadType;
        let roads = [
            RoadType::Trail,
            RoadType::Suburban,
            RoadType::Grid,
            RoadType::Arterial,
        ]
        .map(layer::road);
        assert!(layer::GROUND < roads[0]);
        assert!(roads.windows(2).all(|w| w[0] < w[1]));
        assert!(roads[3] + 0.02 <= layer::PLOT);
        assert_eq!(layer::BAILEY, roads[3]);
        const { assert!(layer::PLOT + 0.02 < layer::MARK && layer::MARK < layer::BANK) };
        const { assert!(layer::PLOT < layer::BANK && layer::BANK < layer::DECK) };
        const { assert!(layer::CHANNEL_BED < layer::SEA && layer::SEA < layer::GROUND) };
        const { assert!(layer::CHANNEL_BED < layer::ARCH_SPRING && layer::ARCH_SPRING < layer::SEA) };
    }

    #[test]
    fn round_trips() {
        let [wx, wz] = to_world(1234.5, 6789.0);
        let [x, y] = to_server(wx, wz);
        assert!((x - 1234.5).abs() < 1e-2 && (y - 6789.0).abs() < 1e-2);
    }
}
