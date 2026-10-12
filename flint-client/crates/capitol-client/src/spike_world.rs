//! What's left of the P0.4 spike's city: two compass markers for checking
//! handedness. The ground moved to `world::ground` (P3.2) and the water to
//! `world::water` (P3.3).

use crate::coords::{layer, polar_to_world};
use crate::world::mesh::MeshBuf;
use crate::world::palette::hex;
use flint_player::ExtensionGpu;

/// Upload the compass markers. Called once, on the first frame the
/// renderer exists.
pub fn upload_static(gpu: &mut ExtensionGpu) {
    // Compass markers for checking handedness: east (angle 0) in the
    // in-range orange, north (angle 270) in the text colour.
    for (name, angle, color) in [
        ("spike_marker_east", 0.0, hex(0xf97316)),
        ("spike_marker_north", 270.0, hex(0xe0e0e0)),
    ] {
        let mut marker = MeshBuf::default();
        let [cx, cz] = polar_to_world(400.0, angle);
        marker.annulus(0.0, 30.0, layer::ROAD, color, &[]);
        for v in &mut marker.vertices {
            v.position[0] += cx;
            v.position[2] += cz;
        }
        marker.upload(gpu, name, 0.6);
    }
}
