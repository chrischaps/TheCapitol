//! Server space <-> Flint world space (plan §4.6). Every conversion goes
//! through here.
//!
//! Server space runs 0..8000 on both axes, centre (4000, 4000), y pointing
//! down the screen (south), angle 0 = east and 90 = south.
//!
//! Flint is right-handed and Y-up: its look-at builds `right = forward × up`,
//! so a camera looking toward -Z has +X on its right (verified in P0.4,
//! `flint-render/src/camera.rs` `view_matrix`). Mapping server x to +X and
//! server y to +Z therefore keeps the map unmirrored: seen from above with
//! north (-Z) at the top of the screen, east is on the right, exactly as in
//! the 2D clients.

use capitol_protocol::rules::{CENTER_X, CENTER_Y};

/// World metres per server unit. The world is 800 m across, a player
/// (radius 8) is about 1.6 m wide, and a 60-unit arterial is 6 m.
pub const S: f32 = 0.1;

/// Server (x, y) to world (X, Z) on the ground plane.
pub fn to_world(x: f64, y: f64) -> [f32; 2] {
    [((x - CENTER_X) as f32) * S, ((y - CENTER_Y) as f32) * S]
}

/// World (X, Z) back to server (x, y). Unused until clicks map to the ground (P1 H5).
#[allow(dead_code)]
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
        assert!(close(to_world(4000.0, 4000.0), [0.0, 0.0]));
    }

    #[test]
    fn east_is_plus_x_and_south_is_plus_z() {
        assert!(close(polar_to_world(200.0, 0.0), [20.0, 0.0]));
        assert!(close(polar_to_world(200.0, 90.0), [0.0, 20.0]));
        assert!(close(polar_to_world(200.0, 270.0), [0.0, -20.0]));
    }

    #[test]
    fn round_trips() {
        let [wx, wz] = to_world(1234.5, 6789.0);
        let [x, y] = to_server(wx, wz);
        assert!((x - 1234.5).abs() < 1e-2 && (y - 6789.0).abs() < 1e-2);
    }
}
