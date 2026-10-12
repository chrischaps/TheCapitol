//! What's left of the P0.4 spike's city: two compass markers, and the
//! water, which the terrain's rings now shape (P3.1). The water is rebuilt
//! every frame, breathing slowly like a tide, which exercises mesh
//! replacement in the live client. The ground moved to `world::ground`
//! (P3.2); P3.3 replaces the water.

use crate::coords::{polar_to_world, S};
use crate::world::mesh::MeshBuf;
use crate::world::palette::hex;
use capitol_protocol::terrain::WaterRing;
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
        marker.annulus(0.0, 30.0, 0.04, color, &[]);
        for v in &mut marker.vertices {
            v.position[0] += cx;
            v.position[2] += cz;
        }
        marker.upload(gpu, name, 0.6);
    }
}

/// Rebuild the terrain's water for game time `t`: the rings swell and ease
/// back by about a metre over a slow twelve-second tide, a hair above the
/// ground, open at each bridge.
pub fn upload_water(gpu: &mut ExtensionGpu, t: f64, rings: &[WaterRing]) {
    let tide = (t * std::f64::consts::TAU / 12.0).sin();
    // The React water colour reads too close to the Trade tint once lit,
    // so this sits between it and the highlight rgba(100, 180, 220).
    let colour = hex(0x4696c8);
    let mut water = MeshBuf::default();
    for ring in rings {
        let width = ring.width + tide * 0.5 / S as f64;
        let gaps: Vec<(f64, f64)> = ring
            .bridges
            .iter()
            .map(|b| {
                let half = ((b.width * 0.5) / ring.radius).asin().to_degrees();
                (b.angle_degrees, half)
            })
            .collect();
        water.annulus(
            ring.radius - width * 0.5,
            ring.radius + width * 0.5,
            0.02,
            colour,
            &gaps,
        );
    }
    water.upload(gpu, "spike_water", 0.25);
}
