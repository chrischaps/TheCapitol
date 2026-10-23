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

// The houses on claimed plots (P8.6b). Each zone has its own materials,
// a little lighter than the lots under them, so the city reads as built
// up from its ground. Each house picks one of each and shades it.

/// The urban terraces: brick, buff brick and old render.
pub const URBAN_WALLS: [u32; 6] = [0x8e5b45, 0x7d4f3d, 0x9b7a5c, 0xa48a6c, 0xc4bcae, 0xa9a39a];
/// Slate, for the city and the guilds.
pub const URBAN_ROOFS: [u32; 4] = [0x3a3d46, 0x2f333b, 0x44464d, 0x3b3532];
/// The post-war suburb: cream, sage, pale blue-grey and sand render,
/// and some brick.
pub const SUBURBAN_WALLS: [u32; 7] = [
    0xd8cfbf, 0xc9c2b0, 0xb9c4bb, 0xbfc6cf, 0xd6c3a5, 0xcfb9a8, 0x9a6a52,
];
/// Brown and red tile, and grey.
pub const SUBURBAN_ROOFS: [u32; 5] = [0x5a4a44, 0x6e4636, 0x7a4a38, 0x464a55, 0x4f4b45];
/// The cottages: whitewash, field stone and ochre.
pub const RURAL_WALLS: [u32; 4] = [0xe0dacb, 0xd2c9b4, 0xa8a090, 0xc8b48a];
/// Thatch, slate and old red tile.
pub const RURAL_ROOFS: [u32; 4] = [0x8a7550, 0x7d6a48, 0x3e414a, 0x7a3e2e];
/// The guild halls' dressed stone.
pub const GUILD_WALLS: [u32; 3] = [0xb5aa94, 0xa69c88, 0xc2b9a6];
/// Front doors: post-war paints and plain wood.
pub const DOORS: [u32; 6] = [0x5b2a2a, 0x2a3f5b, 0x2f4a33, 0x6b5a2a, 0x3a2a22, 0x4a4f58];
pub const GARAGE_DOOR: u32 = 0xb8b2a6;
/// Barn boards: oxblood and weathered timber.
pub const BARNS: [u32; 3] = [0x6e3a2e, 0x5e3328, 0x5a4a3a];
pub const BARN_ROOF: u32 = 0x45474c;
pub const BARN_DOOR: u32 = 0x3a2a22;
/// A farm's drive: packed earth up to the farmhouse door (R6).
pub const FARM_TRACK: u32 = 0x7a6a4e;
/// New timber, on a house going up.
pub const TIMBER: u32 = 0xb08a5c;
pub const CHIMNEY: u32 = 0x6e4a3c;
/// A building opened as you come close (V3.4), drawn as an architect's
/// section: limewashed plaster inside, scrubbed boards underfoot, and the
/// cut through the walls and floors in a dark umber, so the cut reads.
pub const PLASTER: u32 = 0xe6dccb;
pub const BOARDS: u32 = 0xa07c55;
pub const CUT: u32 = 0x3b2f2a;
/// A shop's carriage arch, in shadow, and its shop window's frame.
pub const ARCH: u32 = 0x231d1a;
pub const SHOPFRONT: u32 = 0x2f3a33;
/// Scaffolding while a floor goes up: galvanised poles, timber boards.
pub const SCAFFOLD: u32 = 0x8d9296;
/// A station left out in the weather (V3.4): what its colours fade toward
/// as its condition falls, greyed and greened with moss.
pub const WEATHERED: [f32; 3] = [0.7, 0.76, 0.62];
/// Window glass by day, and the lamp-light in it at night.
pub const GLASS: u32 = 0x27313d;
pub const LAMP: u32 = 0xffb766;

/// The grass fields (P8.6c): a deep olive at the root, a muted take on the
/// rural zone's lime at the tip, and straw for the dry blades.
pub const GRASS_ROOT: u32 = 0x3e5a2a;
pub const GRASS_TIP: u32 = 0x93ae48;
pub const GRASS_DRY: u32 = 0xa8955a;

/// The wilds' ground (W9): hill meadow, bare rock on the steeps, scree
/// on the high slopes and snow on the tops. Like the zones' tokens, each
/// is pulled most of the way toward the background on the ground
/// (`world::land_mesh`), so the ranges stand in the same dusk as the city.
pub const WILD_MEADOW: u32 = 0x5c7a34;
pub const WILD_ROCK: u32 = 0x6e6c74;
pub const WILD_SCREE: u32 = 0x8c806c;
pub const WILD_SNOW: u32 = 0xe6ebf2;
/// The water's ground (W10): dark silt in the beds, moss on the wet banks
/// and pale gravel where a ford runs shallow.
pub const WILD_SILT: u32 = 0x3a3428;
pub const WILD_MOSS: u32 = 0x2f4a26;
pub const WILD_GRAVEL: u32 = 0xa49a84;
/// The valleys' mist at dawn and dusk (W11): a pale, faintly blue white.
pub const WILD_MIST: u32 = 0xd6dce6;

/// `c` darker (`k` < 1) or lighter, alpha kept.
pub fn shade(c: [f32; 4], k: f32) -> [f32; 4] {
    [c[0] * k, c[1] * k, c[2] * k, c[3]]
}

/// The vertex colour of a face that gives off `colour` (linear) as light
/// at `radiance`: 1 is about as bright as a sunlit wall, and over the
/// scene's bloom threshold (2.5) it blooms. Alpha 0 marks it as light
/// rather than surface (`ImportedMaterial::vertex_emission`, Flint ADR
/// 0076, which `MeshBuf::upload_material` turns on), so it glows the same
/// from every side and under any light.
pub fn glow(colour: [f32; 4], radiance: f32) -> [f32; 4] {
    let [r, g, b, _] = colour;
    [r * radiance, g * radiance, b * radiance, 0.0]
}

/// The vertex colour of a face that is `surface` of a lit `colour` and
/// also gives off `colour` as light at `radiance`. The engine lights
/// `rgb × alpha` and gives off `rgb × (1 − alpha)`, so any share of each
/// fits in one vertex: surface 0 is [`glow`], radiance 0 a plain surface.
/// Day/night (P8.6a) slides the gold on your plots between the two.
pub fn lit_glow(colour: [f32; 4], surface: f32, radiance: f32) -> [f32; 4] {
    let total = surface + radiance;
    if total <= 0.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let [r, g, b, _] = colour;
    [r * total, g * total, b * total, surface / total]
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

/// A zone's colour where it's named rather than painted: the React
/// client's zone indicator (`ZoneIndicator.tsx`), the header strip and the
/// zone banner. The same as [`zone`], except the wilderness, whose ground
/// colour is too dark to read on a dark panel.
pub fn zone_mark(zone: Zone) -> u32 {
    match zone {
        Zone::Wilderness => 0x6b7280,
        zone => self::zone(zone),
    }
}

/// An sRGB hex token as `[r, g, b]` from 0 to 1, for the script HUD, whose
/// colours are sRGB.
pub fn srgb(h: u32) -> [f32; 3] {
    let c = |shift: u32| ((h >> shift) & 0xff) as f32 / 255.0;
    [c(16), c(8), c(0)]
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

    #[test]
    fn glow_is_light_at_its_radiance() {
        assert_eq!(glow([0.5, 0.25, 1.0, 1.0], 2.0), [1.0, 0.5, 2.0, 0.0]);
    }

    #[test]
    fn lit_glow_spans_surface_to_light() {
        let gold = [0.5, 0.25, 0.0, 1.0];
        assert_eq!(lit_glow(gold, 0.0, 2.0), glow(gold, 2.0));
        assert_eq!(lit_glow(gold, 1.0, 0.0), gold);
        // Half each: the engine lights rgb × a and gives off rgb × (1 − a).
        let [r, _, _, a] = lit_glow(gold, 1.0, 1.0);
        assert_eq!((r * a, r * (1.0 - a)), (0.5, 0.5));
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
