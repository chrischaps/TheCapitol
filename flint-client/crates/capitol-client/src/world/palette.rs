//! The Capitol's colour tokens (plan §4.7), shared with the React client
//! (`client/src/index.css`, `components/GameCanvas.tsx`). Tokens are sRGB
//! hex as written there; vertex colours are linear, so convert with `hex`.

use capitol_protocol::rules::Zone;
use capitol_protocol::terrain::RoadType;
use flint_render::srgb_to_linear;

pub const BACKGROUND: u32 = 0x0a0a0f;
pub const DIM_TEXT: u32 = 0x808080;
pub const GOLD: u32 = 0xc9a227;
pub const GOLD_DIM: u32 = 0x8a7019;
/// Water, `rgba(30,100,160,0.7)` in the 2D clients. In 3D the water is
/// the scene's ocean, whose colours are set in the scene file; this tints
/// the canal beds.
pub const WATER: u32 = 0x1e64a0;

/// The scene's ground ambient (`[environment] ambient_ground`), linear.
/// A glowing face (`MeshBuf::glowing`) is lit by this alone, so [`glow`]
/// divides by it. A test holds it to the scene file.
pub const AMBIENT_GROUND: [f32; 3] = [0.05, 0.05, 0.07];

/// The vertex colour that makes a glowing face give off `colour` (linear)
/// at `radiance`: 1 is about as bright as a sunlit wall, and well over 1
/// is a light source (bloom picks it up once P3.8 turns it on).
pub fn glow(colour: [f32; 4], radiance: f32) -> [f32; 4] {
    let [r, g, b, a] = colour;
    let [ar, ag, ab] = AMBIENT_GROUND;
    [r * radiance / ar, g * radiance / ag, b * radiance / ab, a]
}

/// A road's colour in the 2D clients: the greater the road, the lighter.
pub fn road(road_type: RoadType) -> u32 {
    match road_type {
        RoadType::Arterial => 0x4a4a5a,
        RoadType::Grid => 0x3a3a4a,
        RoadType::Suburban => 0x2a2a3a,
        RoadType::Trail => 0x1a1a2a,
    }
}

/// A zone's colour in the 2D clients.
pub fn zone(zone: Zone) -> u32 {
    match zone {
        Zone::Capitol => 0xc9a227,
        Zone::TradeDistrict => 0x3b82f6,
        Zone::GuildDistrict => 0x8b5cf6,
        Zone::Urban => 0x64748b,
        Zone::Suburban => 0x22c55e,
        Zone::Rural => 0x84cc16,
        Zone::Wilderness => 0x1a1a24,
    }
}

/// A hex colour token (sRGB) as linear RGBA.
pub fn hex(h: u32) -> [f32; 4] {
    let c = |shift: u32| ((h >> shift) & 0xff) as f32 / 255.0;
    srgb_to_linear([c(16), c(8), c(0), 1.0])
}

/// `a` moved toward `b` by `t` (0 = a, 1 = b), alpha included.
pub fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_linear() {
        assert_eq!(hex(0xffffff), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(hex(0x000000), [0.0, 0.0, 0.0, 1.0]);
        // sRGB mid-grey is about 0.214 linear.
        assert!((hex(0x808080)[0] - 0.2158).abs() < 1e-3);
    }

    /// Glowing faces are calibrated against the scene's ground ambient. If
    /// the scene's ambient changes, change `AMBIENT_GROUND` with it, or
    /// every light in the world dims or flares.
    #[test]
    fn glow_is_calibrated_to_the_scene_ambient() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/capitol.scene.toml");
        let scene = std::fs::read_to_string(path).expect("scene");
        let mut seen = 0;
        for line in scene.lines() {
            let Some(list) = line.trim().strip_prefix("ambient_ground = [") else {
                continue;
            };
            let values: Vec<f32> = list
                .trim_end_matches(']')
                .split(',')
                .map(|v| v.trim().parse().expect("a number"))
                .collect();
            assert_eq!(values[..3], AMBIENT_GROUND, "{line}");
            seen += 1;
        }
        assert!(seen > 0, "the scene sets no ambient_ground");
        let lit = glow([0.5, 0.25, 1.0, 1.0], 2.0);
        for i in 0..3 {
            assert!((lit[i] * AMBIENT_GROUND[i] - [1.0, 0.5, 2.0][i]).abs() < 1e-5);
        }
    }

    #[test]
    fn every_zone_has_its_own_colour() {
        for (i, a) in Zone::ALL.iter().enumerate() {
            for b in &Zone::ALL[i + 1..] {
                assert_ne!(zone(*a), zone(*b), "{a} and {b}");
            }
        }
    }
}
