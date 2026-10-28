use capitol_protocol::fmath::StableMath;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

use capitol_protocol::rules;
use capitol_protocol::land::LandParams;

use super::rural::RuralParams;
use super::urban::UrbanParams;
use super::street_plan;

// The terrain types the client sees are shared through capitol-protocol, so
// GET /terrain serves the generated terrain as-is.
pub use capitol_protocol::terrain::{
    Bridge, Ford, Lake, Landmark, LandmarkKind, RoadSegment, RoadType, TerrainPark, TerrainPlot, WaterCourse,
    WaterRing,
};

/// Configuration for suburban street generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuburbanGenParams {
    /// Inner and outer radius of the suburban zone. The generators read them
    /// only through `WorldGenParams::suburban_params`, which sets them from
    /// the world's `urban_outer_radius` and `suburban_outer_radius`, so a
    /// saved value here has no effect. They stay in the struct (and the JSON)
    /// because the generators read the resolved radii from it.
    pub inner_radius: f64,
    pub outer_radius: f64,
    /// The world's seed. Not a tweakable: `suburban_params` sets it from
    /// `WorldGenParams::seed`.
    #[serde(skip, default = "default_seed")]
    pub seed: u64,
    /// Width of the arterials the streets keep clear of. Not a tweakable:
    /// `generate_all_roads_with_params` sets it from `WorldGenParams`.
    #[serde(skip, default = "default_arterial_width")]
    pub arterial_width: f64,
    /// The neighbourhood generator's knobs (suburb v2, #103). The serde
    /// default keeps saved `world_config` rows from before #105 loading.
    ///
    /// The Legacy generator's params (the street spacing, the plot-area and
    /// frontage windows, its street widths) and the `layout` switch went
    /// with it in #113. Unknown fields are ignored, so a saved row that still
    /// has them, `"layout": "legacy"` included, loads as the neighbourhoods.
    #[serde(default)]
    pub neighbourhood: NeighbourhoodParams,
}

fn default_arterial_width() -> f64 {
    WorldGenParams::default().arterial_width
}

fn default_seed() -> u64 {
    WorldGenParams::default().seed
}

/// The street patterns a neighbourhood can take (plan §4.1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeighbourhoodPattern {
    /// (a) Near-parallel rows two lots apart, joined by T links.
    WarpedParallels,
    /// (b) A racetrack collector with combs of cul-de-sacs inside it.
    LoopsAndLollipops,
    /// (c) A flattened loop round a park, cul-de-sacs pointing out (PW2).
    CentralLoop,
}

/// How often each pattern is picked, among those that fit a neighbourhood.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternWeights {
    pub warped_parallels: f64,
    pub loops_and_lollipops: f64,
    pub central_loop: f64,
}

impl Default for PatternWeights {
    fn default() -> Self {
        Self { warped_parallels: 1.0, loops_and_lollipops: 1.0, central_loop: 1.0 }
    }
}

/// The Neighbourhood generator's parameters, in world units (plan §3, §4).
/// Every field has a default, so a partial object loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NeighbourhoodParams {
    /// Lot width along the street.
    pub lot_frontage: f64,
    /// The narrowest a lot in a run may be, at mid-depth: a run takes fewer
    /// lots rather than any narrower. 0, the default, leaves the count to
    /// `lot_frontage` alone (#456).
    pub lot_width_min: f64,
    /// Lot depth wanted, and the range a row fit may use.
    pub lot_depth: f64,
    pub lot_depth_min: f64,
    pub lot_depth_max: f64,
    /// Street widths.
    pub local_width: f64,
    pub collector_width: f64,
    /// Green strip between a neighbourhood's edge and the arterial.
    pub buffer: f64,
    /// How often a sector splits into two neighbourhoods, by angle, and the
    /// width of the greenway between them.
    pub split_chance: f64,
    pub greenway_width: f64,
    /// Spacing of the T links between parallel rows (L).
    pub link_spacing: f64,
    /// Cul-de-sac turnaround radius, and the longest stem.
    pub bulb_radius: f64,
    pub cul_de_sac_max: f64,
    /// How deep a pie round a bulb reaches from its front, at least, where
    /// the ground allows, past the rear line it would share with the lots
    /// behind. 0, the default, leaves a pie as deep as the lots beside it
    /// (#468: Plots v2's pies, which narrow to the bulb, take 6 stations
    /// only some 20 deeper).
    pub pie_depth: f64,
    /// The frame's shared radial warp: amplitude A (at most 40) and
    /// wavelength λ (at least 1200). #111 tuned them to 30 and 1200: a
    /// 40 warp costs the band 20 of depth, and the lots fall under the
    /// 6600 area floor of the time (9000 since #422).
    pub warp_amplitude: f64,
    pub warp_wavelength: f64,
    pub pattern_weights: PatternWeights,
    /// Every neighbourhood takes this pattern, if it fits. For tests.
    pub force_pattern: Option<NeighbourhoodPattern>,
}

impl Default for NeighbourhoodParams {
    /// The suburb of Plots v2 (`the-capitol-plots-v2.md` V1.2, #456; the
    /// default since the flip, #459), for the band r2160 to r3600. Measured
    /// over seeds 42, 7, 1234 and 99999: 960 to 990 lots of about 18.6k (140
    /// × 130), 92 to 93% of them taking 6 stations, a whole workshop (#468),
    /// and 54 to 59% taking 8 (#466). `the-capitol-suburb-v2.md` §4.4.
    /// - **Warp 10.** The 1440 band less the warp fits five rows of lots
    ///   134 deep. At 30 they're 130 by the fit, and about 127 as laid
    ///   (the warp tilts the rows), under the 128 a lot needs to take 8.
    /// - **Frontage 140, and none under 136.** A lot takes 8 from about
    ///   130 × 128, so a run takes one lot fewer rather than any narrower.
    /// - **Stems to 400,** so pattern (b)'s combs still reach the middle of
    ///   its band, as they did at 300 in the 1200 band.
    /// - **Pies 155 deep** (#468). A pie narrows to about 90 at its bulb,
    ///   so as deep as the lots beside it (about 120 from the bulb's edge)
    ///   it takes 5 stations, and only 2% of pies took 6. At least 155 deep
    ///   where the ground allows, 83 to 90% do. A bigger bulb doesn't help:
    ///   at a radius of 20 or more a pie's front, from the stem's last ray,
    ///   cuts the bulb's neck, and most pies are dropped.
    ///
    /// Before the flip the suburb was r1800 to r3000, with lots 100 wide
    /// (#422; 64 before, the suburb plan's 60 × 110, #108), 110 deep (95 to
    /// 130), warp 30, stems to 300 and pies as deep as the lots beside them.
    fn default() -> Self {
        Self {
            lot_frontage: 140.0,
            lot_width_min: 136.0,
            lot_depth: 134.0,
            lot_depth_min: 118.0,
            lot_depth_max: 140.0,
            local_width: 16.0,
            collector_width: 20.0,
            buffer: 20.0,
            split_chance: 0.5,
            greenway_width: 40.0,
            link_spacing: 400.0,
            bulb_radius: 13.0,
            cul_de_sac_max: 400.0,
            pie_depth: 155.0,
            warp_amplitude: 10.0,
            warp_wavelength: 1200.0,
            pattern_weights: PatternWeights::default(),
            force_pattern: None,
        }
    }
}

impl Default for SuburbanGenParams {
    fn default() -> Self {
        Self {
            inner_radius: rules::URBAN_OUTER_RADIUS,
            outer_radius: rules::SUBURBAN_OUTER_RADIUS,
            seed: 42,
            arterial_width: 60.0,
            neighbourhood: NeighbourhoodParams::default(),
        }
    }
}

/// Master world generation parameters capturing all tweakable values
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldGenParams {
    // Zone radii (concentric from center). Not tweakable: they're the
    // protocol's table, which `Zone::from_position`, the `zones` table and
    // the clients all read, so they stay off the wire and out of the saved
    // config, and the live world can't disagree with them (#118). Code can
    // still set them, for a test or `suburb-plan.ps1 -Inner -Outer`.
    #[serde(skip)]
    pub capitol_radius: f64,
    #[serde(skip)]
    pub trade_outer_radius: f64,
    #[serde(skip)]
    pub guild_outer_radius: f64,
    #[serde(skip)]
    pub urban_outer_radius: f64,
    #[serde(skip)]
    pub suburban_outer_radius: f64,
    #[serde(skip)]
    pub rural_outer_radius: f64,
    /// The side of the world's square, as the zone radii are skipped: the
    /// Wilderness runs to its disc's edge, round its middle (PV2, #454).
    #[serde(skip)]
    pub world_size: f64,
    // Water/Canals
    pub moat_width: f64,
    pub moat_bridge_width: f64,
    pub canal_inner_width: f64,
    pub canal_outer_width: f64,
    pub canal_bridge_width: f64,
    // Ring roads
    pub guild_ring_radius: f64,
    pub guild_ring_width: f64,
    pub trade_ring_radius: f64,
    pub trade_ring_width: f64,
    // Arterials. They run out to the rural edge, where the wilds' trails
    // take over: always `rural_outer_radius`, so they're not a param of
    // their own (PV2, #454).
    pub arterial_width: f64,
    // The urban grid (#421): its quay, rings of blocks and spokes.
    pub urban: UrbanParams,
    // Suburban (nested)
    pub suburban: SuburbanGenParams,
    // The wild trails past the rural edge (W5, #198).
    pub wild_trail_width: f64,
    // Rural v2 (`the-capitol-rural.md`): the farms and their roads. Its own
    // defaults, so a saved config from before it still loads. A saved config
    // that still names the old trails' params (`rural_trail_*`,
    // `rural_plot_*`, `rural.layout`) loads as the farms: serde skips them
    // (#291).
    pub rural: RuralParams,
    // Guild halls
    pub guild_plot_frontage: f64,
    // Seeds
    pub seed: u64,
    // The height of the wilds (W2, #195). Its own seed, so reseeding the
    // suburb leaves the ranges standing.
    pub land: LandParams,
}

impl WorldGenParams {
    /// The suburban params the generators use: `suburban` with its radii,
    /// seed and arterial width taken from the world. Roads and lots both read
    /// this one set (#105). Before, the lots read `suburban`'s own radii.
    pub fn suburban_params(&self) -> SuburbanGenParams {
        SuburbanGenParams {
            inner_radius: self.urban_outer_radius,
            outer_radius: self.suburban_outer_radius,
            seed: self.seed,
            arterial_width: self.arterial_width,
            ..self.suburban.clone()
        }
    }

    /// The rings these params lay the world in: their zone radii round the
    /// middle of a world `world_size` across. The land, the water, the
    /// trails and the wild nodes all read it (PV2, #454).
    pub fn frame(&self) -> rules::Frame {
        rules::Frame::new(
            self.world_size,
            [
                self.capitol_radius,
                self.trade_outer_radius,
                self.guild_outer_radius,
                self.urban_outer_radius,
                self.suburban_outer_radius,
                self.rural_outer_radius,
            ],
        )
    }

    /// The land's params in this world's frame: the foothills and the
    /// ranges past its rural edge, round its centre.
    pub fn framed_land(&self) -> LandParams {
        self.land.clone().in_frame(self.frame())
    }

    /// RW1's rings (#308), the protocol's table before Plots v2's flip
    /// (#459): urban out to r1800, the suburb to r3000, rural to r4600, in a
    /// world 13600 across, with the zones' params of the time. For tests
    /// that check that what reads the frame follows it off the table.
    #[cfg(test)]
    pub fn rw1() -> Self {
        let mut params = Self {
            urban_outer_radius: 1800.0,
            suburban_outer_radius: 3000.0,
            rural_outer_radius: 4600.0,
            world_size: 13600.0,
            urban: UrbanParams { rings: 3, outer_spokes: 0, ..UrbanParams::default() },
            rural: RuralParams {
                frontage: (120.0, 160.0),
                field_gap: (50.0, 100.0),
                depth: (230.0, 310.0),
                min_frontage: 100.0,
                min_depth: 110.0,
                room: 0,
                sway: 1.0,
                lanes_follow: false,
                mains: vec![(330.0 / 1600.0, 470.0 / 1600.0), (1130.0 / 1600.0, 1270.0 / 1600.0)],
                ..RuralParams::default()
            },
            ..Self::default()
        };
        params.suburban.neighbourhood = NeighbourhoodParams {
            lot_frontage: 100.0,
            lot_width_min: 0.0,
            lot_depth: 110.0,
            lot_depth_min: 95.0,
            lot_depth_max: 130.0,
            cul_de_sac_max: 300.0,
            pie_depth: 0.0,
            warp_amplitude: 30.0,
            ..NeighbourhoodParams::default()
        };
        params
    }

    /// The zone radii a params object names with values off the protocol's
    /// table. Deserializing ignores them (#118); this is for saying so. The
    /// arterials' old end (`arterial_end_radius`) counts too: it's always
    /// the rural edge now (PV2, #454).
    pub fn untunable_radii_in(json: &serde_json::Value) -> Vec<(&'static str, f64)> {
        let table = Self::default();
        [
            ("capitol_radius", table.capitol_radius),
            ("trade_outer_radius", table.trade_outer_radius),
            ("guild_outer_radius", table.guild_outer_radius),
            ("urban_outer_radius", table.urban_outer_radius),
            ("suburban_outer_radius", table.suburban_outer_radius),
            ("rural_outer_radius", table.rural_outer_radius),
            ("arterial_end_radius", table.rural_outer_radius),
        ]
        .into_iter()
        .filter_map(|(key, radius)| json.get(key)?.as_f64().filter(|&v| v != radius).map(|v| (key, v)))
        .collect()
    }

    /// Whether these are the default world's params, so build its terrain.
    /// Compared as the saved config is, so the radii, which aren't saved,
    /// don't count. Cheaper than building both terrains to compare versions.
    pub fn is_default(&self) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(Self::default()).ok()
    }
}

impl Default for WorldGenParams {
    fn default() -> Self {
        Self {
            // The zone radii are the protocol's table (#116).
            capitol_radius: rules::CAPITOL_RADIUS,
            trade_outer_radius: rules::TRADE_OUTER_RADIUS,
            guild_outer_radius: rules::GUILD_OUTER_RADIUS,
            urban_outer_radius: rules::URBAN_OUTER_RADIUS,
            suburban_outer_radius: rules::SUBURBAN_OUTER_RADIUS,
            rural_outer_radius: rules::RURAL_OUTER_RADIUS,
            world_size: rules::WORLD_SIZE,
            moat_width: 20.0,
            moat_bridge_width: 30.0,
            canal_inner_width: 15.0,
            canal_outer_width: 15.0,
            canal_bridge_width: 25.0,
            guild_ring_radius: 800.0,
            guild_ring_width: 30.0,
            trade_ring_radius: 400.0,
            trade_ring_width: 30.0,
            arterial_width: 60.0,
            urban: UrbanParams::default(),
            suburban: SuburbanGenParams::default(),
            wild_trail_width: 8.0,
            rural: RuralParams::default(),
            guild_plot_frontage: 150.0,
            seed: 42,
            land: LandParams::default(),
        }
    }
}

/// How far a guild hall stands back from the kerb of the roads and water
/// round it, in world units (#83).
const GUILD_CLEAR: f64 = 1.0;

/// Terrain configuration for the world
/// Defines moats, canals, bridges, roads, and suburban plots
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerrainConfig {
    pub world_size: f64,
    pub center_x: f64,
    pub center_y: f64,
    pub water_rings: Vec<WaterRing>,
    pub roads: Vec<RoadSegment>,
    pub terrain_plots: Vec<TerrainPlot>,
    /// The suburb's parks, from the open ground its lots leave (#129).
    #[serde(default)]
    pub parks: Vec<TerrainPark>,
    /// The city's public buildings: the Exchange hall (E3.4).
    #[serde(default)]
    pub landmarks: Vec<Landmark>,
    /// The params of the land's height (W2, #195), which `/terrain` carries.
    #[serde(default)]
    pub land: LandParams,
    /// The wilds' water, from the land (W3, #196): see `hydrology`.
    #[serde(default)]
    pub rivers: Vec<WaterCourse>,
    #[serde(default)]
    pub streams: Vec<WaterCourse>,
    #[serde(default)]
    pub lakes: Vec<Lake>,
    #[serde(default)]
    pub fords: Vec<Ford>,
}

/// Calculate distance from point to line segment
pub(crate) fn point_to_segment_distance(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len_sq = dx * dx + dy * dy;

    if len_sq == 0.0 {
        return ((px - x1).powi(2) + (py - y1).powi(2)).sqrt();
    }

    let t = ((px - x1) * dx + (py - y1) * dy) / len_sq;
    let t = t.clamp(0.0, 1.0);

    let proj_x = x1 + t * dx;
    let proj_y = y1 + t * dy;

    ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt()
}

impl TerrainConfig {
    /// Get the static terrain configuration for the world (uses default params)
    pub fn get_default() -> Self {
        Self::from_params(&WorldGenParams::default())
    }

    /// Where each lot is, by its street's name: lot id to "Larch Close", or
    /// "Mercer Street by Tannergate" on a ring street (#483)
    pub fn addresses(&self) -> std::collections::HashMap<String, String> {
        super::road_names::addresses(&self.roads, &self.terrain_plots)
    }

    /// The lot that `(x, y)` stands on, if any. Lots don't overlap, so there's at most one.
    pub fn lot_at(&self, x: f64, y: f64) -> Option<&TerrainPlot> {
        self.terrain_plots.iter().find(|lot| {
            let c = &lot.corners;
            let (min_x, max_x) = c.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
            let (min_y, max_y) = c.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
            x >= min_x && x <= max_x && y >= min_y && y <= max_y
                && capitol_protocol::terrain::quad_contains(c, x, y)
        })
    }

    /// A stable hash of the generated terrain, as 16 hex digits: FNV-1a over its
    /// JSON. It hashes the output rather than the params, so a generator change
    /// also changes it, and clients can key a disk cache on it.
    ///
    /// Every float is snapped to a 1e-3 grid first, so a last-bit difference
    /// between builds (the optimiser reordering a sum, a libm path) doesn't
    /// count, and debug and release servers agree on the world (#128). The grid
    /// is coarse on purpose: a value within an ulp of a grid boundary still
    /// flips, and the odds of that grow as the grid gets finer. At 1e-3, on a
    /// world 12000 across, they're about 1e-8 a float.
    pub fn version(&self) -> String {
        fn quantise(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Number(n) if n.is_f64() => {
                    let snapped = (n.as_f64().expect("is_f64") * 1000.0).round() as i64;
                    *value = snapped.into();
                }
                serde_json::Value::Array(items) => items.iter_mut().for_each(quantise),
                serde_json::Value::Object(fields) => fields.values_mut().for_each(quantise),
                _ => {}
            }
        }
        let mut tree = serde_json::to_value(self).expect("terrain serializes");
        quantise(&mut tree);
        let json = serde_json::to_vec(&tree).expect("terrain serializes");
        let hash = json.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
            (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3)
        });
        format!("{hash:016x}")
    }

    /// The default world, built once for every test that reads it: it takes
    /// a few seconds, and the performance test runs beside the others (PV2,
    /// #454; Plots v2's world since the flip, #459).
    #[cfg(test)]
    pub(crate) fn default_world() -> &'static TerrainConfig {
        static WORLD: std::sync::OnceLock<TerrainConfig> = std::sync::OnceLock::new();
        WORLD.get_or_init(Self::get_default)
    }

    /// The world `WorldGenParams::rw1()` builds, off the table, once for
    /// every test that reads it.
    #[cfg(test)]
    pub(crate) fn rw1() -> &'static TerrainConfig {
        static WORLD: std::sync::OnceLock<TerrainConfig> = std::sync::OnceLock::new();
        WORLD.get_or_init(|| Self::from_params(&WorldGenParams::rw1()))
    }

    /// The world's frame before anything is laid on it: its size, centre,
    /// water rings and land, with no roads, lots or water courses. The urban
    /// grid's v2 measure (#455) lays its own roads on it. It's laid in the
    /// params' frame (`WorldGenParams::frame`): the default's is the
    /// protocol's table.
    pub(crate) fn bare(params: &WorldGenParams) -> Self {
        let frame = params.frame();
        // The land in the world's frame, so the water, the trails and the
        // wild nodes follow its rings. A saved config's foothills and ranges
        // don't count: they're offsets past the rural edge (PV2, #454).
        let land = params.framed_land();
        TerrainConfig {
            world_size: frame.world_size(),
            center_x: frame.center.0,
            center_y: frame.center.1,
            water_rings: vec![
                // Moat: Capitol/Trade boundary
                WaterRing {
                    id: "moat".to_string(),
                    radius: params.capitol_radius,
                    width: params.moat_width,
                    bridges: vec![
                        Bridge { angle_degrees: 90.0, width: params.moat_bridge_width },
                    ],
                },
                // Canal 1: Trade/Guild boundary
                WaterRing {
                    id: "canal_inner".to_string(),
                    radius: params.trade_outer_radius,
                    width: params.canal_inner_width,
                    bridges: vec![
                        Bridge { angle_degrees: 270.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 0.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 90.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 180.0, width: params.canal_bridge_width },
                    ],
                },
                // Canal 2: Guild/Urban boundary
                WaterRing {
                    id: "canal_outer".to_string(),
                    radius: params.guild_outer_radius,
                    width: params.canal_outer_width,
                    bridges: vec![
                        Bridge { angle_degrees: 270.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 315.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 0.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 45.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 90.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 135.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 180.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 225.0, width: params.canal_bridge_width },
                    ],
                },
            ],
            roads: Vec::new(),
            terrain_plots: Vec::new(),
            parks: Vec::new(),
            landmarks: Vec::new(),
            land,
            rivers: Vec::new(),
            streams: Vec::new(),
            lakes: Vec::new(),
            fords: Vec::new(),
        }
    }

    /// Build terrain from explicit world generation parameters
    pub fn from_params(params: &WorldGenParams) -> Self {
        let mut config = Self::bare(params);

        // Generate roads using params
        config.roads = config.generate_all_roads_with_params(params);

        // The water comes from the land, and skirts the roads. The farms keep
        // clear of it (#287).
        let water = super::hydrology::generate(&config.land, &config.roads);

        // Generate terrain plots for all zones
        config.terrain_plots = config.generate_suburban_plots_internal(&params.suburban_params());
        config.terrain_plots.extend(config.generate_urban_plots(params));
        let wet = super::rural::farms::Water { rivers: &water.rivers, streams: &water.streams, lakes: &water.lakes };
        config.terrain_plots.extend(config.generate_farms(params, wet));
        config.terrain_plots.extend(config.generate_guild_plots(params));
        // Pull every lot clear of the roads and the water, and make the ids unique
        // (B7): a lot is what players claim, so it must be the ground it looks like.
        config.terrain_plots = super::lot_clearance::finish_lots(
            std::mem::take(&mut config.terrain_plots),
            &config.roads,
            &config.water_rings,
            (config.center_x, config.center_y),
        );
        // The wild trails run on from the arterials, round the water, and
        // after the lots, which keep off them anyway: none stand past the
        // rural edge. The water was laid before them, so it doesn't skirt them.
        let trails = super::trails::lay(&config.land, &water, &config.roads, &config.terrain_plots, params.wild_trail_width);
        config.roads.extend(trails);
        (config.rivers, config.streams, config.lakes, config.fords) =
            (water.rivers, water.streams, water.lakes, water.fords);

        // The parks are the ground the cleared lots leave.
        config.parks = config.generate_parks(&params.suburban_params());
        config.landmarks = vec![config.exchange_hall_site(params)];
        // The bazaar quarter (E3.12): its stalls, the mouth on the avenue,
        // the square, and the lane and the alley
        let bazaar = super::bazaar::quarter((config.center_x, config.center_y), params);
        config.landmarks.extend(bazaar.landmarks);
        config.roads.extend(bazaar.roads);
        // And now the world's laid, what people call its roads (#483)
        let sectors = super::rural::sectors_of(params, (config.center_x, config.center_y), &config.roads);
        super::road_names::name_roads(&mut config.roads, params.seed, &sectors, params.rural.mains.len());

        config
    }

    /// The landmark of `kind`, if the world has one.
    pub fn landmark(&self, kind: LandmarkKind) -> Option<&Landmark> {
        self.landmarks.iter().find(|l| l.kind == kind)
    }

    /// The Exchange hall (E3.4, plan D6): on the east side of the Capitol
    /// avenue, the stretch of the south arterial between the trade ring and the
    /// moat's one bridge. New players arrive on that same road, so the way in
    /// passes the hall's door on the last walk to the tower. It's 80 across
    /// and 100 along the avenue, set back from the kerb, the moat and the ring.
    fn exchange_hall_site(&self, params: &WorldGenParams) -> Landmark {
        const SETBACK: f64 = 15.0;
        const DEPTH: f64 = 80.0;
        const LENGTH: f64 = 100.0;
        let bearing = 90.0_f64.to_radians();
        let out = (bearing.stable_cos(), bearing.stable_sin());
        // At 90° that's east: the avenue's right, walking in.
        let side = (out.1, -out.0);
        let moat_edge = params.capitol_radius + params.moat_width / 2.0;
        let ring_edge = params.trade_ring_radius - params.trade_ring_width / 2.0;
        let mid_r = (moat_edge + ring_edge) / 2.0;
        let near = params.arterial_width / 2.0 + SETBACK;
        let at = |lateral: f64, r: f64| {
            (self.center_x + out.0 * r + side.0 * lateral, self.center_y + out.1 * r + side.1 * lateral)
        };
        let (inner_r, outer_r) = (mid_r - LENGTH / 2.0, mid_r + LENGTH / 2.0);
        Landmark {
            id: "exchange_hall".to_string(),
            kind: LandmarkKind::ExchangeHall,
            position: at(near + DEPTH / 2.0, mid_r),
            corners: [at(near, outer_r), at(near, inner_r), at(near + DEPTH, inner_r), at(near + DEPTH, outer_r)],
            entrance_edge: (0, 1),
            door: at(near, mid_r),
        }
    }

    /// The suburb's parks, for the roads and lots already in `self`.
    fn generate_parks(&self, params: &SuburbanGenParams) -> Vec<TerrainPark> {
        let center = (self.center_x, self.center_y);
        let neighbourhoods = super::suburb::plan(params, center).neighbourhoods;
        let greens = super::suburb::parks::lay(&self.roads, &self.terrain_plots, &neighbourhoods, params, center);
        super::suburb::parks::outlines(&greens)
    }

    /// Generate all road segments from params
    fn generate_all_roads_with_params(&self, params: &WorldGenParams) -> Vec<RoadSegment> {
        self.generate_roads_and_rural_plan(params).0
    }

    /// The roads, and the rural plan they were laid from: its sectors, and
    /// the farm gates, the balance pass's too.
    pub(crate) fn generate_roads_and_rural_plan(&self, params: &WorldGenParams) -> (Vec<RoadSegment>, super::rural::Plan) {
        let mut roads = Vec::new();

        // Ring roads
        roads.extend(self.generate_ring_roads_with_params(params));

        // Arterial roads
        roads.extend(self.generate_arterial_roads_with_params(params));

        // Urban grid
        let grid = self.generate_urban_grid_with_params(params, &roads);
        roads.extend(grid);

        // Suburban streets
        roads.extend(super::suburb::plan(&params.suburban_params(), (self.center_x, self.center_y)).roads);

        // The rural ring: Rural v2's lanes (#286), which the arterials cut
        // into sectors, with the balance pass's spurs (RD7), before the weld
        // and the water, so the water skirts them.
        let plan = super::rural::plan_balanced(params, (self.center_x, self.center_y), &roads);
        roads.extend(plan.roads.iter().cloned());

        // Every suburban junction becomes a vertex both roads share, so
        // clients can see where streets meet (#62).
        street_plan::weld_junctions(&mut roads, |r| r.road_type == RoadType::Suburban);
        // And every lane's: its ends onto the arterial or the main road it
        // stops at (RD5). Only the farm gates stay loose.
        street_plan::weld_junctions(&mut roads, super::rural::is_lane);
        // And the urban grid's: its ring streets cross the arterials (#436).
        street_plan::weld_junctions(&mut roads, super::urban::is_grid_street);

        (roads, plan)
    }

    /// Generate circular ring roads within districts
    pub(crate) fn generate_ring_roads_with_params(&self, params: &WorldGenParams) -> Vec<RoadSegment> {
        vec![
            self.generate_circular_road("guild_ring", params.guild_ring_radius, params.guild_ring_width),
            self.generate_circular_road("trade_ring", params.trade_ring_radius, params.trade_ring_width),
        ]
    }

    /// Generate a circular road at a given radius
    fn generate_circular_road(&self, id: &str, radius: f64, width: f64) -> RoadSegment {
        let num_segments = 64;
        let mut points = Vec::new();

        for i in 0..=num_segments {
            let angle = (i as f64 / num_segments as f64) * 2.0 * PI;
            let x = self.center_x + radius * angle.stable_cos();
            let y = self.center_y + radius * angle.stable_sin();
            points.push((x, y));
        }

        RoadSegment {
            id: id.to_string(),
            road_type: RoadType::Arterial,
            points,
            width,
            name: None,
        }
    }

    /// Generate arterial roads connecting ring roads to wilderness edge
    pub(crate) fn generate_arterial_roads_with_params(&self, params: &WorldGenParams) -> Vec<RoadSegment> {
        let mut roads = Vec::new();
        let outer_canal = self.water_rings.iter().find(|r| r.id == "canal_outer");
        let inner_canal = self.water_rings.iter().find(|r| r.id == "canal_inner");
        let moat = self.water_rings.iter().find(|r| r.id == "moat");

        let guild_ring_radius = params.guild_ring_radius;
        let trade_ring_radius = params.trade_ring_radius;

        if let Some(outer) = outer_canal {
            for (i, bridge) in outer.bridges.iter().enumerate() {
                let angle_rad = bridge.angle_degrees.to_radians();
                let angle_deg = bridge.angle_degrees;

                let start_radius = guild_ring_radius;
                // Out to the rural edge, where the wild trails go on.
                let end_radius = params.rural_outer_radius;

                let start_x = self.center_x + start_radius * angle_rad.stable_cos();
                let start_y = self.center_y + start_radius * angle_rad.stable_sin();
                let end_x = self.center_x + end_radius * angle_rad.stable_cos();
                let end_y = self.center_y + end_radius * angle_rad.stable_sin();

                roads.push(RoadSegment {
                    id: format!("arterial_{}", i),
                    road_type: RoadType::Arterial,
                    points: vec![(start_x, start_y), (end_x, end_y)],
                    width: params.arterial_width,
                    name: None,
                });

                let is_cardinal = angle_deg == 0.0 || angle_deg == 90.0 || angle_deg == 180.0 || angle_deg == 270.0;
                if is_cardinal {
                    if let Some(_inner) = inner_canal {
                        let guild_x = self.center_x + guild_ring_radius * angle_rad.stable_cos();
                        let guild_y = self.center_y + guild_ring_radius * angle_rad.stable_sin();
                        let trade_x = self.center_x + trade_ring_radius * angle_rad.stable_cos();
                        let trade_y = self.center_y + trade_ring_radius * angle_rad.stable_sin();

                        roads.push(RoadSegment {
                            id: format!("arterial_inner_{}", i),
                            road_type: RoadType::Arterial,
                            points: vec![(guild_x, guild_y), (trade_x, trade_y)],
                            width: params.arterial_width,
                            name: None,
                        });
                    }
                }
            }
        }

        // South street extends into Capitol (from trade ring to moat)
        if let Some(moat_ring) = moat {
            let south_angle = 90.0_f64.to_radians();
            let trade_x = self.center_x + trade_ring_radius * south_angle.stable_cos();
            let trade_y = self.center_y + trade_ring_radius * south_angle.stable_sin();
            let capitol_x = self.center_x + (moat_ring.inner_radius() - 10.0) * south_angle.stable_cos();
            let capitol_y = self.center_y + (moat_ring.inner_radius() - 10.0) * south_angle.stable_sin();

            roads.push(RoadSegment {
                id: "arterial_capitol".to_string(),
                road_type: RoadType::Arterial,
                points: vec![(trade_x, trade_y), (capitol_x, capitol_y)],
                width: params.arterial_width,
                name: None,
            });
        }

        roads
    }

    /// The urban grid's streets (#421): its ring streets and spokes. The
    /// arterials must be in `roads` already.
    fn generate_urban_grid_with_params(&self, params: &WorldGenParams, roads: &[RoadSegment]) -> Vec<RoadSegment> {
        super::urban::grid(params, (self.center_x, self.center_y), roads).roads()
    }

    /// The suburban lots for the already-generated roads in self.roads, as
    /// generated, before `finish_lots`. `params` is `suburban_params()`.
    pub(crate) fn generate_suburban_plots_internal(&self, params: &SuburbanGenParams) -> Vec<TerrainPlot> {
        // The plan again rather than a copy kept on self, so `version()`
        // doesn't see it. It's deterministic, and quick beside the lots.
        let center = (self.center_x, self.center_y);
        super::suburb::lots::lay(&self.roads, &super::suburb::plan(params, center), params, center)
    }

    /// The urban lots (#421), in the blocks between the grid's streets.
    fn generate_urban_plots(&self, params: &WorldGenParams) -> Vec<TerrainPlot> {
        super::urban::grid(params, (self.center_x, self.center_y), &self.roads).lots(&params.urban)
    }

    /// Guild halls: big lots either side of the guild ring road, from the inner
    /// canal to the outer one, split at every arterial that reaches the band
    /// (#83). The four cardinal arterials cross the inner band; the other four
    /// start at the ring, and their ends reach into it, so they split it too.
    ///
    /// Each band is the ground that's clear by construction: the ring road's
    /// ribbon lies inside `R + w/2` and outside `R·cos(π/n) − w/2` (it's an
    /// n-gon), and the water keeps its kerb. A lot's straight inner edge sags
    /// toward the centre, so its inner corners stand far enough out that the
    /// edge's midpoint still clears the band. A side edge runs parallel to its
    /// arterial, `GUILD_CLEAR` past the kerb. So `finish_lots` has nothing to
    /// trim, and the generator's ids and shapes are the ones that get claimed.
    fn generate_guild_plots(&self, params: &WorldGenParams) -> Vec<TerrainPlot> {
        let ring = self.roads.iter().find(|r| r.id == "guild_ring");
        let inner_canal = self.water_rings.iter().find(|r| r.id == "canal_inner");
        let outer_canal = self.water_rings.iter().find(|r| r.id == "canal_outer");
        let (Some(ring), Some(inner_canal), Some(outer_canal)) = (ring, inner_canal, outer_canal) else {
            return Vec::new();
        };
        let kerb = super::lot_clearance::WATER_KERB;
        let ring_r = params.guild_ring_radius;
        let ring_half = ring.width / 2.0;
        let sides = ring.points.len().saturating_sub(1).max(3) as f64;
        let bands = [
            ("in", inner_canal.outer_radius() + kerb + GUILD_CLEAR, ring_r * (PI / sides).stable_cos() - ring_half - GUILD_CLEAR),
            ("out", ring_r + ring_half + GUILD_CLEAR, outer_canal.inner_radius() - kerb - GUILD_CLEAR),
        ];

        let radius = |&(x, y): &(f64, f64)| ((x - self.center_x).powi(2) + (y - self.center_y).powi(2)).sqrt();
        let mut plots = Vec::new();
        for (band, lo, hi) in bands {
            if hi - lo < 20.0 {
                continue;
            }
            // (angle, half width) of each radial arterial whose ribbon reaches
            // the band. Suburban junctions weld extra vertices into the outer
            // arterials (#62), so a radial road is any whose points share a bearing.
            let bearing = |&(x, y): &(f64, f64)| (y - self.center_y).stable_atan2(x - self.center_x).rem_euclid(2.0 * PI);
            let mut dividers: Vec<(f64, f64)> = self
                .roads
                .iter()
                .filter(|r| r.road_type == RoadType::Arterial && r.points.len() >= 2)
                .filter_map(|r| {
                    let far = *r.points.iter().max_by(|a, b| radius(a).total_cmp(&radius(b)))?;
                    let a = bearing(&far);
                    let radial = r.points.iter().all(|p| {
                        let d = (bearing(p) - a).rem_euclid(2.0 * PI);
                        radius(p) < 1e-6 || d.min(2.0 * PI - d) < 1e-6
                    });
                    let h = r.width / 2.0;
                    let near = r.points.iter().map(radius).fold(f64::INFINITY, f64::min);
                    (radial && near - h < hi && radius(&far) + h > lo).then_some((a, h))
                })
                .collect();
            dividers.sort_by(|a, b| a.0.total_cmp(&b.0));
            dividers.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
            if dividers.is_empty() {
                continue;
            }

            for wedge in 0..dividers.len() {
                let (a1, h1) = dividers[wedge];
                let (mut a2, h2) = dividers[(wedge + 1) % dividers.len()];
                if a2 <= a1 {
                    a2 += 2.0 * PI;
                }
                // The wedge's edges at radius r, each GUILD_CLEAR past its arterial's kerb
                let edges = |r: f64| (a1 + ((h1 + GUILD_CLEAR) / r).stable_asin(), a2 - ((h2 + GUILD_CLEAR) / r).stable_asin());
                let (m1, m2) = edges((lo + hi) / 2.0);
                if m2 <= m1 {
                    continue;
                }
                let count = ((m2 - m1) * (lo + hi) / 2.0 / params.guild_plot_frontage.max(1.0)).ceil().max(1.0) as usize;

                // Stand the inner corners out so the chord between them clears `lo`
                let mut r_in = lo;
                for _ in 0..3 {
                    let (e1, e2) = edges(r_in);
                    r_in = lo / ((e2 - e1) / count as f64 / 2.0).stable_cos();
                }
                let r_out = hi;
                let (in1, in2) = edges(r_in);
                let (out1, out2) = edges(r_out);
                if in2 <= in1 || out2 <= out1 {
                    continue;
                }

                let at = |r: f64, a: f64| (self.center_x + r * a.stable_cos(), self.center_y + r * a.stable_sin());
                for s in 0..count {
                    let (f0, f1) = (s as f64 / count as f64, (s + 1) as f64 / count as f64);
                    let (i0, i1) = (in1 + (in2 - in1) * f0, in1 + (in2 - in1) * f1);
                    let (o0, o1) = (out1 + (out2 - out1) * f0, out1 + (out2 - out1) * f1);
                    // Corners start with the edge on the ring road, as (0, 1)
                    let corners = if band == "out" {
                        [at(r_in, i0), at(r_in, i1), at(r_out, o1), at(r_out, o0)]
                    } else {
                        [at(r_out, o1), at(r_out, o0), at(r_in, i0), at(r_in, i1)]
                    };
                    let area = TerrainPlot::calculate_area(&corners);
                    if area < 100.0 {
                        continue;
                    }
                    plots.push(TerrainPlot {
                        id: format!("guild_plot_{band}_{wedge}_{s}"),
                        corners,
                        street_facing_edge: (0, 1),
                        area,
                        adjacent_street_id: ring.id.clone(),
                        zone: "guild_district".to_string(),
                    });
                }
            }
        }
        plots
    }

    /// The rural lots as `from_params` lays them, before `finish_lots`, and
    /// the roads they're laid against: `roads`, the ones from before the
    /// water, the wild trails and the bazaar
    /// (`generate_roads_and_rural_plan`'s). For the rural plan report (#284).
    #[cfg(test)]
    pub(crate) fn rural_plots_before(&self, params: &WorldGenParams, roads: Vec<RoadSegment>) -> (Vec<TerrainPlot>, Vec<RoadSegment>) {
        let laid = TerrainConfig { roads, ..self.clone() };
        let water = super::rural::farms::Water { rivers: &self.rivers, streams: &self.streams, lakes: &self.lakes };
        (laid.generate_farms(params, water), laid.roads)
    }

    /// The rural lots: the farms (#287), clear of `water`.
    fn generate_farms(&self, params: &WorldGenParams, water: super::rural::farms::Water) -> Vec<TerrainPlot> {
        let center = (self.center_x, self.center_y);
        let sectors = super::rural::sectors_of(params, center, &self.roads);
        super::rural::farms::lay(params, center, &self.roads, &sectors, Some(water))
    }
}

impl TerrainConfig {
    /// The land, the wild water and the rings, for walking on (W4, #197;
    /// the rings since T0.1, #315). Build it once per terrain: `GameState`
    /// keeps it beside the terrain.
    pub fn ground(&self) -> capitol_protocol::rules::ground::Ground {
        capitol_protocol::rules::ground::Ground::new(
            &self.land,
            &self.rivers,
            &self.streams,
            &self.lakes,
            &self.fords,
            &self.roads,
        )
        .with_rings((self.center_x, self.center_y), &self.water_rings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet, VecDeque};

    use crate::models::geom::SpatialGrid;

    #[test]
    fn test_terrain_config_default() {
        let config = TerrainConfig::default_world();
        assert_eq!(config.world_size, 16600.0);
        assert_eq!(config.water_rings.len(), 3);
    }

    #[test]
    fn test_version_is_stable_and_follows_params() {
        let version = TerrainConfig::get_default().version();
        assert_eq!(version.len(), 16);
        // Regenerating from the same params gives the same terrain, so the same version
        assert_eq!(TerrainConfig::get_default().version(), version);

        let params = WorldGenParams { seed: 43, ..WorldGenParams::default() };
        assert_ne!(TerrainConfig::from_params(&params).version(), version);
        let params = WorldGenParams { moat_width: 70.0, ..WorldGenParams::default() };
        assert_ne!(TerrainConfig::from_params(&params).version(), version);
    }

    /// Pins the default world, so nothing moves the live world by accident:
    /// its lots are owned plots and its nodes were placed against it. Change
    /// the literal only for a change that means to move it, write the node
    /// move migration it needs (see `models::resource::tests`), and say so in
    /// the commit. It holds in debug and release alike (#128).
    #[test]
    fn default_terrain_version_is_pinned() {
        // The world last moved on purpose in #112, which made Neighbourhood
        // the default. #128 changed only how the version is hashed (floats
        // quantised), not the world. #129 added the parks: no road, lot or
        // node moved.
        //
        // #180 moved the trig off the platform's libm (`fmath::StableMath`),
        // so every platform builds the same bits. That moved the Windows
        // world (`c3a856792daad83d`) onto the one Linux already built: three
        // suburban roads and some park rings gained or lost a vertex near a
        // junction. No lot or node moved.
        //
        // W1 (#194) grew the world to 12000 and moved the centre to (6000,
        // 6000), so every coordinate moved by +2000. The generator didn't
        // change: against `4d3d9d112a4f30f1` moved by +2000, the lots and
        // their ids are the same, no corner is off by more than 0.015, and 11
        // roads gained or lost a collinear vertex without moving. The seeded
        // nodes moved with the migration, and none needs fitting.
        //
        // W2 (#195) put the land's params on the terrain (`land`), so the
        // hash covers them. No road, lot, park or node moved.
        //
        // E3.4 (#170) added the Exchange hall to `landmarks`
        // (`b31da8eabe0cb864`). No road, lot, park or node moved.
        //
        // W3 (#196) put the wilds' water on the terrain: rivers, streams,
        // lakes and fords, all past r3800. No road, lot, park or node moved,
        // and no seeded node stands in the water
        // (`no_seeded_node_stands_in_the_wild_water`). It landed as
        // `5c38bf6ff1ffa2a0`; then channels side by side were gathered and
        // the meanders kept apart, so no two courses cross (#220,
        // `dbb6436950a157b5`).
        //
        // #222 let the water run on into the rural ring, round the roads, to
        // end in ponds or peter out, instead of meres on one circle. No road,
        // lot, park or seeded node moved. Some of E2.3's planned nodes stand
        // in the new water, and startup's `fit_to_terrain` moves them
        // (`no_node_stands_in_the_wild_water`). The rural lots don't know the
        // water yet (#223).
        //
        // W5 (#198) ran the arterials on past r3800 as wild trails, round the
        // water: the `wild_*` trail roads. Nothing else moved, and no seeded
        // or planned node stands on one
        // (`the_default_terrain_moves_no_planned_node`).
        //
        // E3.10 (#213) added the bazaar and its 13 stalls to `landmarks`,
        // across the avenue from the hall. No road, lot, park or node moved.
        //
        // E3.12 (#277) made the bazaar a quarter: 41 stalls along the trade
        // ring, a lane in from the avenue and an alley to a square
        // (`models::bazaar`), with the lane and the alley as two new roads.
        // No other road, lot, park or node moved.
        //
        // RW1 (#308) widened the rural ring to r4600 and grew the world to
        // 13600, centred on (6800, 6800), so every coordinate moved by +800
        // (rural plan RD10). Against `8067c60f47cb8c7c` moved by +800: every
        // urban, guild and suburban lot keeps its id, 33 suburban lots in
        // four neighbourhoods by the rural edge move by 0.02 to 2.4 (with a
        // collector, three entrances and three cul-de-sacs there, and two
        // parks; most likely the old rural trails, which start at the
        // suburb's edge and now run twice as far), and nothing else in the
        // city moves. The arterials run on to r4600. The old trails stretch
        // with the ring, so it has 121 rural lots, not 52 (32 of the old ids
        // kept). Past r3800 the land, the water (53 rivers, 200 streams, 67
        // lakes, 43 fords) and the wild trails are new; the plain's tilt is
        // measured from its edge, a course out of a lake rises well inside
        // it, and a trail never crosses itself. The planned nodes are
        // re-planned whole, clear of the wild water
        // (`post-content/20261122_001_rural_widens_nodes.sql`).
        //
        // R7 (#290) flipped the rural ring to the farms (rural plan §7):
        // `RuralParams::layout` defaults to `Farms`. The 12 trails and their
        // 121 lots give way to each sector's two main roads, the roads between
        // them and the gate roads, the balance pass's spurs (29 `lane_*`
        // roads in all) and 117 `farm_*` lots. Each arterial gains the lanes'
        // junctions as vertices, between r3290 and r4350, and doesn't move.
        // With the trails gone from the suburb's edge, 136 suburban lots in
        // seven neighbourhoods there grow or shift by 0.02 to 11.3 and keep
        // their ids, and 8 parks change. The plain's water runs round the new
        // roads, so inside r4800 its courses, ponds and fords move (62 lakes,
        // not 67); past r4800 the water is the same, though some ids shift.
        // Owned trail lots are released by startup's sync
        // (`plot_sync::RETIRED_LOTS`). Two seeded grass nodes move
        // (`20261126_001_rural_farms.sql`), and the planned nodes the farms
        // move are re-planned (`post-content/20261126_001_rural_farms_nodes.sql`).
        //
        // #423 made the farms smaller (plots plan §5.3): long lots, narrower
        // and closer along the same lanes, 377 of about 36k for 117 of about
        // 117k. Only the rural lots moved: no road, no water, no other lot.
        // Old farm rows are renamed `oldfarm_` and released by startup's sync
        // (`20261201_001_rural_smaller_farms.sql`). No seeded grass moves, and
        // the planned nodes the farms move are re-planned
        // (`post-content/20261201_001_rural_smaller_farms_nodes.sql`).
        //
        // #422 made the suburb's lots wider (plots plan §5.2): frontage 100,
        // not 64, and 2 or 3 pies round a bulb, not 4 or 5. 1192 lots of
        // about 10.3k for 1905 of about 6.7k, 90% taking 4 stations. The
        // streets didn't move; the parks are the open ground the lots leave,
        // so they did (116, not 128). Nothing outside the suburb moved. Old
        // suburban rows are renamed `oldsub_` and released by startup's sync
        // (`20261205_001_suburb_wider_lots.sql`). No seeded grass moves, and
        // the planned nodes the lots move are re-planned
        // (`post-content/20261205_001_suburb_wider_lots_nodes.sql`).
        //
        // #421 laid a new urban grid (plots plan §5.1): a canal bank and a
        // quay, then three rings of back-to-back blocks between four ring
        // streets, cut by 16 spokes and the 8 arterials. 520 lots of about
        // 8.6k (`urb_…`), every one taking 4 stations, for 3008 of about 1.4k
        // (`urban_plot_…`, now in `RETIRED_LOTS`, so startup releases an
        // owned one). The old grid's spokes ran on to r1800, into the
        // suburb's first row, so 123 suburban lots on 19 streets at its
        // inner edge grow or re-space, one is gone, and the parks shift;
        // startup keeps an owned one as it was. The arterials, the farms and
        // the water didn't move. No seeded grass moves, and the planned nodes
        // the grid moves (the urban clay, now on the canal bank, and the
        // suburb's branches and clay planned after it) are re-planned
        // (`post-content/20261207_001_urban_grid_nodes.sql`).
        //
        // Plots v2's flip (#459, `the-capitol-plots-v2.md`) moved the rings
        // out and gave every zone its v2 layout: urban to r2160 with five
        // rings of blocks and 32 bearings past r1800, the suburb to r3600 with
        // lots of about 140 x 134 and pies 155 deep, the rural ring to r6100
        // with four main lanes and farms that take 12, and the world to 16600,
        // centred on (8300, 8300), so every coordinate moved by +1500. Against
        // `9d6cec09d529c7dd` moved by +1500, the 520 urban lots and the 72
        // guild halls keep their ids and their shapes; the two new rings of
        // blocks add 496 urban lots. The suburb is new (977 lots for 1191,
        // 121 parks for 116), and so are the farms (901 for 377, on 69 lanes),
        // the land (the foothills from r6200, the ranges full height at
        // r7900), the water (72 rivers, 266 streams, 73 lakes, 78 fords; its
        // rim at r3750) and the wild trails. 404 roads, 2966 lots. Old
        // suburban and rural rows are renamed `rw2_` and released by
        // startup's sync; the seeded grass moves out with the rings, and every
        // planned node is re-planned (`20261220_001_plots_v2_flip.sql` and
        // `post-content/20261220_001_plots_v2_flip_nodes.sql`).
        //
        // #467 made the station packing rule monotone, with rows along each
        // side of a lot. The farms are laid as small as still takes 12, so
        // some take it smaller: 911 farms for 900 (18 gone, 115 reshaped,
        // 29 new ids), 2977 lots. Nothing outside the rural ring moved. The
        // reshaped and dropped farms' rows are renamed `repack_` and
        // released by startup's sync, and the rural ring's planned nodes are
        // re-planned (`20270106_001_packing_rows.sql` and its post-content
        // nodes).
        //
        // #436 welded the urban grid into the arterials: each of the eight
        // gains a vertex on its own line where each of the six ring streets
        // crosses it, and the rings' vertices there settle by 2e-12. No lot,
        // park, water or node moved (7 farms by 4e-12, under the hash's
        // quantum), so there's no migration.
        //
        // #483 named the roads (`road_names`): every road but the wild spurs
        // has a `name`, which the hash covers. No road, lot, park, water or
        // node moved, and no id changed.
        assert_eq!(TerrainConfig::default_world().version(), "415fdfde7a60fb1d");
    }

    /// Writes the default world as JSON to `$TERRAIN_DUMP`, to diff one
    /// platform's world against another's (#180):
    /// `TERRAIN_DUMP=win.json cargo test dump_default_terrain -- --ignored`.
    #[test]
    #[ignore]
    fn dump_default_terrain() {
        let path = std::env::var("TERRAIN_DUMP").expect("set TERRAIN_DUMP to the output path");
        let terrain = TerrainConfig::default_world();
        std::fs::write(&path, serde_json::to_string_pretty(&terrain).unwrap()).unwrap();
        println!("{} -> {path}", terrain.version());
    }

    /// A last-bit float difference, the kind the optimiser makes, leaves the
    /// version alone; a real move doesn't (#128).
    #[test]
    fn version_ignores_last_bit_float_noise() {
        let terrain = TerrainConfig::default_world();
        let mut nudged = terrain.clone();
        let x = &mut nudged.terrain_plots[0].corners[0].0;
        *x = f64::from_bits(x.to_bits() + 1);
        let radius = &mut nudged.water_rings[0].radius;
        *radius = f64::from_bits(radius.to_bits() - 1);
        assert_eq!(nudged.version(), terrain.version());

        nudged.terrain_plots[0].corners[0].0 += 0.01;
        assert_ne!(nudged.version(), terrain.version());
    }

    /// Saved `world_config` rows and admin PUTs from before #105 have no
    /// `neighbourhood`, and ones from before #113 carry Legacy's params and a
    /// `layout`. They all load, as the neighbourhoods with their defaults.
    #[test]
    fn world_params_load_old_and_new() {
        let mut json = serde_json::to_value(WorldGenParams::default()).unwrap();
        let suburban = json["suburban"].as_object_mut().unwrap();
        assert!(!suburban.contains_key("layout") && !suburban.contains_key("seed"), "{suburban:?}");
        suburban.remove("neighbourhood");
        let old: WorldGenParams = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old.suburban.neighbourhood, NeighbourhoodParams::default());

        // A row from before #113, with Legacy named and its params.
        json["suburban"]["layout"] = "legacy".into();
        json["suburban"]["min_street_spacing"] = 100.0.into();
        json["suburban"]["local_street_width"] = 10.0.into();
        json["suburban"]["seed"] = 7.into();
        let legacy: WorldGenParams = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(legacy.suburban.neighbourhood, NeighbourhoodParams::default());
        assert_eq!(legacy.suburban_params().seed, legacy.seed);

        // A partial neighbourhood: the rest takes its defaults.
        json["suburban"]["neighbourhood"] = serde_json::json!({ "lot_depth": 120.0, "force_pattern": "central_loop" });
        let new: WorldGenParams = serde_json::from_value(json).unwrap();
        assert_eq!(new.suburban.neighbourhood.lot_depth, 120.0);
        assert_eq!(new.suburban.neighbourhood.lot_frontage, 140.0);
        assert_eq!(new.suburban.neighbourhood.force_pattern, Some(NeighbourhoodPattern::CentralLoop));
    }

    /// The default world with pattern (a) in every neighbourhood, for the
    /// suburban street tests (#107). The mixed default is covered by
    /// `suburb::tests`. Built once: several tests share it, and the
    /// performance test runs beside them.
    fn warped_parallels() -> &'static TerrainConfig {
        static WORLD: std::sync::OnceLock<TerrainConfig> = std::sync::OnceLock::new();
        WORLD.get_or_init(|| {
            let mut params = WorldGenParams::default();
            params.suburban.neighbourhood.force_pattern = Some(NeighbourhoodPattern::WarpedParallels);
            TerrainConfig::from_params(&params)
        })
    }

    /// The default world lays suburban streets and lots (#108), and the rest
    /// of the world round them.
    #[test]
    fn the_default_world_has_a_suburb() {
        let t = TerrainConfig::default_world();
        assert!(t.roads.iter().any(|r| r.road_type == RoadType::Suburban));
        assert!(t.terrain_plots.iter().filter(|l| l.zone == "suburban").count() > 900);
        assert!(t.terrain_plots.iter().any(|l| l.zone == "urban"));
    }

    /// Roads and lots read one set of suburban radii, from the zone radii,
    /// whatever `suburban`'s own copies say (#105).
    #[test]
    fn suburban_params_take_the_zone_radii() {
        let mut params = WorldGenParams { urban_outer_radius: 1500.0, suburban_outer_radius: 2500.0, seed: 9, ..Default::default() };
        params.suburban.inner_radius = 1.0;
        params.suburban.outer_radius = 2.0;
        let s = params.suburban_params();
        assert_eq!((s.inner_radius, s.outer_radius, s.seed), (1500.0, 2500.0, 9));
        let mut stale = WorldGenParams::default();
        stale.suburban.inner_radius = 1.0;
        stale.suburban.outer_radius = 2.0;
        assert_eq!(TerrainConfig::from_params(&stale).version(), TerrainConfig::default_world().version());
    }

    /// The zone radii aren't tweakable (#118): params sent or saved with
    /// other radii still build the default world, so the lots' zones agree
    /// with `Zone::from_position`, and the params never carry them out.
    #[test]
    fn zone_radii_stay_the_protocol_table() {
        let json = serde_json::json!({ "seed": 42, "suburban_outer_radius": 2800.0, "rural_outer_radius": 6100.0 });
        let params: WorldGenParams = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(
            [params.capitol_radius, params.trade_outer_radius, params.guild_outer_radius, params.urban_outer_radius, params.suburban_outer_radius, params.rural_outer_radius],
            rules::ZONE_OUTER_RADII[..6]
        );
        assert_eq!(TerrainConfig::from_params(&params).version(), TerrainConfig::default_world().version());
        // Only the one off the table is named.
        assert_eq!(WorldGenParams::untunable_radii_in(&json), [("suburban_outer_radius", 2800.0)]);
        let out = serde_json::to_value(WorldGenParams::default()).unwrap();
        assert!(WorldGenParams::untunable_radii_in(&out).is_empty());
        for key in ["capitol_radius", "trade_outer_radius", "guild_outer_radius", "urban_outer_radius", "suburban_outer_radius", "rural_outer_radius"] {
            assert!(out.get(key).is_none(), "the params carry {key}");
        }

        // Each lot stands in the zone it was laid for.
        for lot in &TerrainConfig::from_params(&params).terrain_plots {
            let x = lot.corners.iter().map(|c| c.0).sum::<f64>() / 4.0;
            let y = lot.corners.iter().map(|c| c.1).sum::<f64>() / 4.0;
            assert_eq!(rules::Zone::from_position(x, y).as_str(), lot.zone, "{} at ({x}, {y})", lot.id);
        }
    }

    #[test]
    fn test_moat_has_one_bridge() {
        let config = TerrainConfig::default_world();
        let moat = config.water_rings.iter().find(|r| r.id == "moat").unwrap();
        assert_eq!(moat.bridges.len(), 1);
        assert_eq!(moat.bridges[0].angle_degrees, 90.0); // South (90° in screen coords where Y+ is down)
    }

    #[test]
    fn test_inner_canal_has_four_bridges() {
        let config = TerrainConfig::default_world();
        let canal = config.water_rings.iter().find(|r| r.id == "canal_inner").unwrap();
        assert_eq!(canal.bridges.len(), 4);
    }

    #[test]
    fn test_outer_canal_has_eight_bridges() {
        let config = TerrainConfig::default_world();
        let canal = config.water_rings.iter().find(|r| r.id == "canal_outer").unwrap();
        assert_eq!(canal.bridges.len(), 8);
    }

    #[test]
    fn test_bridge_contains_angle() {
        let bridge = Bridge { angle_degrees: 0.0, width: 25.0 };
        assert!(bridge.contains_angle(0.0, 600.0));
        assert!(bridge.contains_angle(1.0, 600.0));
        // Bridge is ~2.4 degrees wide at radius 600, so 5 degrees should be outside
        assert!(!bridge.contains_angle(5.0, 600.0));
    }

    #[test]
    fn test_bridge_angle_wraparound() {
        let bridge = Bridge { angle_degrees: 0.0, width: 25.0 };
        // Should handle wraparound at 0/360
        assert!(bridge.contains_angle(359.0, 600.0));
        assert!(bridge.contains_angle(-1.0, 600.0));
    }

    #[test]
    fn the_rings_stop_a_walk_but_their_bridges_let_it_over() {
        let ground = TerrainConfig::default_world().ground();
        // Within the Capitol, inside the moat: a whole step.
        let step = ground.step((8300.0, 8300.0), (8350.0, 8350.0), 5.0);
        assert_eq!(step.blocked, None);
        // East, square into the moat, where there's no bridge.
        let mut at = (8450.0, 8300.0);
        let blocked = loop {
            let step = ground.step(at, (8550.0, 8300.0), 5.0);
            at = (step.x, step.y);
            if let Some(blocked) = step.blocked {
                break blocked;
            }
            assert!(at.0 < 8550.0, "walked over the moat");
        };
        assert_eq!(blocked.water_id(), "moat");
        // South, over the moat's bridge.
        let mut at = (8300.0, 8450.0);
        while at != (8300.0, 8550.0) {
            let step = ground.step(at, (8300.0, 8550.0), 5.0);
            assert_eq!(step.blocked, None, "at {at:?}");
            at = (step.x, step.y);
        }
    }

    #[test]
    fn test_arterial_roads_generated() {
        let config = TerrainConfig::default_world();
        let arterials: Vec<_> = config.roads.iter()
            .filter(|r| r.road_type == RoadType::Arterial)
            .collect();
        // 8 main arterials + 4 inner NSEW + 1 capitol + 2 ring roads = 15
        assert!(arterials.len() >= 13, "Expected at least 13 arterial roads, got {}", arterials.len());
    }

    #[test]
    fn test_ring_roads_generated() {
        let config = TerrainConfig::default_world();
        let ring_roads: Vec<_> = config.roads.iter()
            .filter(|r| r.id.contains("ring") && !super::super::urban::is_grid_street(r))
            .collect();
        assert_eq!(ring_roads.len(), 2, "Expected 2 ring roads (guild and trade)");
        // And the urban grid's: the quay and five more (#421; five rings of
        // blocks since Plots v2, #455, #459).
        assert_eq!(config.roads.iter().filter(|r| r.id.starts_with("urb_ring_")).count(), 6);
    }

    #[test]
    fn test_suburban_streets_smooth() {
        let config = warped_parallels();
        let suburban: Vec<_> = config.roads.iter()
            .filter(|r| r.road_type == RoadType::Suburban)
            .collect();
        // Each suburban street should have many points for smooth curves
        for road in &suburban {
            assert!(road.points.len() >= 10, "Suburban road {} should have smooth curves (10+ points), got {}", road.id, road.points.len());
        }
    }

    #[test]
    fn test_suburban_gen_params_default() {
        let params = SuburbanGenParams::default();
        assert_eq!(params.inner_radius, rules::URBAN_OUTER_RADIUS);
        assert_eq!(params.outer_radius, rules::SUBURBAN_OUTER_RADIUS);
        assert_eq!(params.neighbourhood, NeighbourhoodParams::default());
    }

    #[test]
    fn test_suburban_collector_roads_generated() {
        let config = warped_parallels();
        let collectors: Vec<_> = config.roads.iter()
            .filter(|r| r.id.starts_with("collector_"))
            .collect();
        // Should have collectors for each of 8 superblocks (at least 8)
        assert!(collectors.len() >= 8, "Expected at least 8 collector roads, got {}", collectors.len());
    }

    #[test]
    fn test_suburban_cul_de_sacs_generated() {
        let config = warped_parallels();
        let culdesacs: Vec<_> = config.roads.iter()
            .filter(|r| r.id.starts_with("culdesac_"))
            .collect();
        // Should have some cul-de-sacs
        assert!(!culdesacs.is_empty(), "Expected at least 1 cul-de-sac road");
    }

    #[test]
    fn test_suburban_streets_within_zone() {
        let config = warped_parallels();
        let params = SuburbanGenParams::default();

        // Check that all suburban street points are within the suburban zone
        for road in config.roads.iter().filter(|r| r.road_type == RoadType::Suburban) {
            for (x, y) in &road.points {
                let dist = ((x - config.center_x).powi(2) + (y - config.center_y).powi(2)).sqrt();
                // Allow some tolerance for curves near boundaries
                let tolerance = 50.0;
                assert!(
                    dist >= params.inner_radius - tolerance && dist <= params.outer_radius + tolerance,
                    "Point ({}, {}) in road {} is outside suburban zone (dist={}, expected {}..{})",
                    x, y, road.id, dist, params.inner_radius, params.outer_radius
                );
            }
        }
    }

    #[test]
    fn test_suburban_connectivity() {
        let config = TerrainConfig::default_world();
        let params = SuburbanGenParams::default();

        // Build adjacency graph and verify all suburban streets can reach arterials
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        let mut endpoints: HashMap<String, Vec<(f64, f64)>> = HashMap::new();

        // Add arterials
        for i in 0..8 {
            let id = format!("arterial_{}", i);
            graph.insert(id.clone(), Vec::new());
            let angle = i as f64 * PI / 4.0;
            endpoints.insert(id, vec![
                (config.center_x + params.inner_radius * angle.stable_cos(), config.center_y + params.inner_radius * angle.stable_sin()),
                (config.center_x + params.outer_radius * angle.stable_cos(), config.center_y + params.outer_radius * angle.stable_sin()),
            ]);
        }

        // Add suburban streets
        for road in config.roads.iter().filter(|r| r.road_type == RoadType::Suburban) {
            graph.insert(road.id.clone(), Vec::new());
            let mut eps = Vec::new();
            if let Some(first) = road.points.first() {
                eps.push(*first);
            }
            if let Some(last) = road.points.last() {
                eps.push(*last);
            }
            endpoints.insert(road.id.clone(), eps);
        }

        // Build edges based on proximity
        let connection_threshold = 50.0;
        let road_ids: Vec<String> = graph.keys().cloned().collect();

        for i in 0..road_ids.len() {
            for j in (i + 1)..road_ids.len() {
                let id1 = &road_ids[i];
                let id2 = &road_ids[j];

                if let (Some(eps1), Some(eps2)) = (endpoints.get(id1), endpoints.get(id2)) {
                    let mut connected = false;
                    for &(x1, y1) in eps1 {
                        for &(x2, y2) in eps2 {
                            let dist = ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt();
                            if dist < connection_threshold {
                                connected = true;
                                break;
                            }
                        }
                        if connected { break; }
                    }

                    if connected {
                        graph.get_mut(id1).unwrap().push(id2.clone());
                        graph.get_mut(id2).unwrap().push(id1.clone());
                    }
                }
            }
        }

        // BFS from each suburban street to check connectivity
        let arterial_ids: HashSet<String> = (0..8).map(|i| format!("arterial_{}", i)).collect();

        for road in config.roads.iter().filter(|r| r.road_type == RoadType::Suburban) {
            // BFS to find path to arterial
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
            queue.push_back(road.id.clone());
            visited.insert(road.id.clone());

            let mut found_arterial = false;
            while let Some(current) = queue.pop_front() {
                if arterial_ids.contains(&current) {
                    found_arterial = true;
                    break;
                }

                if let Some(neighbors) = graph.get(&current) {
                    for neighbor in neighbors {
                        if !visited.contains(neighbor) {
                            visited.insert(neighbor.clone());
                            queue.push_back(neighbor.clone());
                        }
                    }
                }
            }

            // Note: Not all streets may be connected due to collision avoidance
            // This is informational rather than a hard failure
            if !found_arterial {
                println!("Warning: Road {} may not be connected to arterial network", road.id);
            }
        }
    }

    #[test]
    fn test_suburban_plot_area_calculation() {
        // Test the shoelace formula for area calculation
        let square = [
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (0.0, 100.0),
        ];
        let area = TerrainPlot::calculate_area(&square);
        assert!((area - 10_000.0).abs() < 0.01, "Expected area 10000, got {}", area);

        // Test a rectangle
        let rect = [
            (0.0, 0.0),
            (200.0, 0.0),
            (200.0, 100.0),
            (0.0, 100.0),
        ];
        let rect_area = TerrainPlot::calculate_area(&rect);
        assert!((rect_area - 20_000.0).abs() < 0.01, "Expected area 20000, got {}", rect_area);
    }

    /// The street plan's invariants (#62), at a few seeds: no repeated
    /// points, no hairpins outside a bulb, no road over itself, roads meeting
    /// only at shared vertices, and every street end on a road or closed on
    /// its own bulb.
    #[test]
    fn suburban_street_plan_holds_its_invariants() {
        for seed in [42, 7, 1234, 99_999] {
            let t = TerrainConfig::from_params(&WorldGenParams { seed, ..Default::default() });
            let audit = street_plan::audit(&t.roads, |r| r.road_type == RoadType::Suburban);
            assert!(audit.is_clean(), "seed {seed}: {audit:#?}");

            let suburban: Vec<_> = t.roads.iter().filter(|r| r.road_type == RoadType::Suburban).collect();
            let collectors = suburban.iter().filter(|r| r.id.starts_with("collector_")).count();
            eprintln!("seed {seed}: {} suburban roads, {collectors} collectors", suburban.len());
            assert!(suburban.len() >= 80 && collectors >= 10, "seed {seed}: a sparse plan");

            let mut ids = HashSet::new();
            assert!(t.roads.iter().all(|r| ids.insert(r.id.as_str())), "seed {seed}: repeated road ids");

            // Off the arterials' ribbons, except near a junction at either end.
            for art in t.roads.iter().filter(|r| r.id.starts_with("arterial_") && r.road_type == RoadType::Arterial) {
                for street in &suburban {
                    let ends = [street.points[0], *street.points.last().unwrap()];
                    let joins: Vec<_> = ends
                        .into_iter()
                        .filter(|e| t.roads.iter().any(|o| o.id != street.id && o.points.contains(e)))
                        .collect();
                    let clearance = (art.width + street.width) / 2.0;
                    let exempt = art.width / 2.0 + 2.0 * street.width;
                    assert!(
                        street_plan::keeps_clear(&street.points, &art.points, clearance, &joins, exempt),
                        "seed {seed}: {} runs into {}", street.id, art.id
                    );
                }
            }
        }
    }

    #[test]
    fn test_polyline_length() {
        use crate::models::geom::polyline_length;
        let line = vec![(0.0, 0.0), (100.0, 0.0)];
        assert!((polyline_length(&line) - 100.0).abs() < 0.01);

        let square = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)];
        assert!((polyline_length(&square) - 400.0).abs() < 0.01);
    }

    #[test]
    fn test_no_roads_terminate_at_outer_boundary() {
        // This test verifies that no suburban roads have endpoints at the outer boundary
        // of the suburban zone (which would indicate erroneous emergency connectors)
        let config = warped_parallels();
        let params = SuburbanGenParams::default();

        let outer_boundary_tolerance = 20.0; // Within 20 units of outer boundary

        // Calculate the 8 "corner" positions at the outer boundary (where arterials intersect)
        let corner_positions: Vec<(f64, f64)> = (0..8)
            .map(|i| {
                let angle = i as f64 * PI / 4.0;
                (
                    config.center_x + params.outer_radius * angle.stable_cos(),
                    config.center_y + params.outer_radius * angle.stable_sin(),
                )
            })
            .collect();

        let mut problematic_roads = Vec::new();

        for road in config.roads.iter().filter(|r| r.road_type == RoadType::Suburban) {
            // Check first and last points
            let endpoints = [road.points.first(), road.points.last()];

            for endpoint_opt in endpoints.iter() {
                if let Some(&(ex, ey)) = *endpoint_opt {
                    // Check if this endpoint is near the outer boundary
                    let dist_from_center = ((ex - config.center_x).powi(2) + (ey - config.center_y).powi(2)).sqrt();
                    let dist_from_outer = (dist_from_center - params.outer_radius).abs();

                    if dist_from_outer < outer_boundary_tolerance {
                        // Check if it's near one of the arterial corner positions
                        for (i, &(cx, cy)) in corner_positions.iter().enumerate() {
                            let dist_to_corner = ((ex - cx).powi(2) + (ey - cy).powi(2)).sqrt();
                            if dist_to_corner < 50.0 {
                                problematic_roads.push((road.id.clone(), i, ex, ey));
                            }
                        }
                    }
                }
            }
        }

        // Report any problematic roads
        for (road_id, corner_idx, x, y) in &problematic_roads {
            println!(
                "WARNING: Road {} has endpoint ({:.0}, {:.0}) near outer boundary corner {}",
                road_id, x, y, corner_idx
            );
        }

        assert!(
            problematic_roads.is_empty(),
            "Found {} roads with endpoints at outer boundary corners: {:?}",
            problematic_roads.len(),
            problematic_roads.iter().map(|(id, _, _, _)| id.clone()).collect::<Vec<_>>()
        );
    }

    /// D6: the Exchange hall stands in the Trade District beside the Capitol
    /// avenue, its entrance on the avenue, clear of every road and the water.
    #[test]
    fn the_exchange_hall_stands_on_the_capitol_avenue() {
        let params = WorldGenParams::default();
        let config = TerrainConfig::default_world();
        let hall = config.landmark(LandmarkKind::ExchangeHall).expect("a hall");
        let center = (config.center_x, config.center_y);
        let r = |(x, y): (f64, f64)| (x - center.0).stable_hypot(y - center.1);
        for &corner in &hall.corners {
            assert_eq!(rules::Zone::from_position(corner.0, corner.1), rules::Zone::TradeDistrict, "{corner:?}");
        }
        assert!(TerrainPlot::calculate_area(&hall.corners) >= 8000.0);
        assert!(capitol_protocol::terrain::quad_contains(&hall.corners, hall.position.0, hall.position.1));

        // Clear of the water and every road, kerbs and all
        let edges = |i: usize| (hall.corners[i], hall.corners[(i + 1) % 4]);
        for ring in &config.water_rings {
            for &c in &hall.corners {
                assert!(r(c) > ring.outer_radius() + 10.0 || r(c) < ring.inner_radius() - 10.0, "{} at {c:?}", ring.id);
            }
        }
        for road in &config.roads {
            for w in road.points.windows(2) {
                for i in 0..4 {
                    let (a, b) = edges(i);
                    // Sample the hall's edge against the road's centre line
                    for k in 0..=20 {
                        let t = k as f64 / 20.0;
                        let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                        let d = point_to_segment_distance(p.0, p.1, w[0].0, w[0].1, w[1].0, w[1].1);
                        assert!(d >= road.width / 2.0 + 10.0, "{} comes within {d} of the hall", road.id);
                    }
                }
            }
        }
        assert!(config.lot_at(hall.position.0, hall.position.1).is_none());

        // The door is on the avenue's kerb side, and the avenue's crown in
        // front of it is within range
        let avenue = config.roads.iter().find(|r| r.id == "arterial_capitol").expect("the avenue");
        let (a, b) = (avenue.points[0], avenue.points[1]);
        let to_crown = point_to_segment_distance(hall.door.0, hall.door.1, a.0, a.1, b.0, b.1);
        assert!((to_crown - (params.arterial_width / 2.0 + 15.0)).abs() < 1e-6, "{to_crown}");
        assert!(capitol_protocol::rules::market::at_the_hall(hall, center.0, hall.door.1));
        assert!(!capitol_protocol::rules::market::at_the_hall(hall, center.0, center.1 + 600.0));
    }

    /// D6 (E3.12): the bazaar quarter on the default world. Every stall is in
    /// the Trade District, clear of the water, of every road but by its
    /// street's gap, and of every lot; and the quarter's own tests
    /// (`models::bazaar`) hold for it.
    #[test]
    fn the_bazaar_quarter_stands_in_the_trade_district_clear_of_the_rest() {
        let config = TerrainConfig::default_world();
        let center = (config.center_x, config.center_y);
        let stalls: Vec<&Landmark> = config.landmarks.iter().filter(|l| l.kind == LandmarkKind::Stall).collect();
        assert!(stalls.len() >= 34, "{} stalls", stalls.len());
        assert!(config.landmark(LandmarkKind::Bazaar).is_some());
        assert!(config.landmark(LandmarkKind::Square).is_some());
        for road in ["bazaar_lane", "bazaar_alley"] {
            assert!(config.roads.iter().any(|r| r.id == road), "{road}");
        }
        for stall in &stalls {
            for &c in &stall.corners {
                assert_eq!(rules::Zone::from_position(c.0, c.1), rules::Zone::TradeDistrict, "{} at {c:?}", stall.id);
                for ring in &config.water_rings {
                    let r = (c.0 - center.0).stable_hypot(c.1 - center.1);
                    assert!(r > ring.outer_radius() + 5.0 || r < ring.inner_radius() - 5.0, "{} in the {}", stall.id, ring.id);
                }
            }
            for road in &config.roads {
                for w in road.points.windows(2) {
                    for &c in &stall.corners {
                        let d = point_to_segment_distance(c.0, c.1, w[0].0, w[0].1, w[1].0, w[1].1);
                        assert!(d >= road.width / 2.0 + 2.0, "{} comes within {d} of {}", stall.id, road.id);
                    }
                }
            }
            assert!(config.lot_at(stall.position.0, stall.position.1).is_none());
        }
    }

    #[test]
    fn guild_halls_fill_the_guild_district_untrimmed() {
        // The defaults and the wider canals the dev DB commits (#83).
        let wide = WorldGenParams {
            canal_inner_width: 50.0,
            canal_outer_width: 50.0,
            ..WorldGenParams::default()
        };
        for params in [WorldGenParams::default(), wide] {
            let config = TerrainConfig::from_params(&params);
            let generated = config.generate_guild_plots(&params);
            let kept: Vec<_> = config.terrain_plots.iter().filter(|p| p.zone == "guild_district").collect();
            // finish_lots had nothing to slide or drop
            assert_eq!(kept.len(), generated.len());
            for (k, g) in kept.iter().zip(&generated) {
                assert_eq!(k.id, g.id);
                assert_eq!(k.corners, g.corners, "{} was trimmed", g.id);
            }
            let (inner, outer) = (kept.iter().filter(|p| p.id.contains("_in_")).count(), kept.len());
            assert!(inner >= 24 && outer - inner >= 32, "{inner} inner, {} outer", outer - inner);
            for p in &kept {
                assert!(p.area > 10_000.0, "{} is only {} square", p.id, p.area);
                for &(x, y) in &p.corners {
                    let r = ((x - config.center_x).powi(2) + (y - config.center_y).powi(2)).sqrt();
                    assert!(r > params.trade_outer_radius && r < params.guild_outer_radius, "{} reaches r {r}", p.id);
                }
            }
        }
    }

    #[test]
    fn test_terrain_generation_determinism() {
        // Generate terrain twice with the same params and verify identical output
        let params = WorldGenParams::default();
        let config1 = TerrainConfig::from_params(&params);
        let config2 = TerrainConfig::from_params(&params);

        // Same number of roads
        assert_eq!(config1.roads.len(), config2.roads.len(),
            "Road count differs between runs: {} vs {}", config1.roads.len(), config2.roads.len());

        // Same road IDs and points
        for (r1, r2) in config1.roads.iter().zip(config2.roads.iter()) {
            assert_eq!(r1.id, r2.id, "Road IDs differ");
            assert_eq!(r1.points.len(), r2.points.len(),
                "Road {} point count differs: {} vs {}", r1.id, r1.points.len(), r2.points.len());
            for (i, (p1, p2)) in r1.points.iter().zip(r2.points.iter()).enumerate() {
                assert!((p1.0 - p2.0).abs() < 1e-10 && (p1.1 - p2.1).abs() < 1e-10,
                    "Road {} point {} differs: ({},{}) vs ({},{})", r1.id, i, p1.0, p1.1, p2.0, p2.1);
            }
        }

        // Same plots
        assert_eq!(config1.terrain_plots.len(), config2.terrain_plots.len(),
            "Plot count differs: {} vs {}", config1.terrain_plots.len(), config2.terrain_plots.len());

        for (p1, p2) in config1.terrain_plots.iter().zip(config2.terrain_plots.iter()) {
            assert_eq!(p1.id, p2.id, "Plot IDs differ");
            for (i, (c1, c2)) in p1.corners.iter().zip(p2.corners.iter()).enumerate() {
                assert!((c1.0 - c2.0).abs() < 1e-10 && (c1.1 - c2.1).abs() < 1e-10,
                    "Plot {} corner {} differs", p1.id, i);
            }
        }
    }

    /// The default world's parks are on `/terrain` (#129), and they're ground
    /// no lot took: no lot's centre is in one, so none can be claimed or
    /// built on. Their rings hold the raster's park squares to within 2%,
    /// and they're 8–18% of the suburb (14% when written).
    #[test]
    fn the_parks_are_on_the_terrain_and_off_the_lots() {
        let params = WorldGenParams::default();
        let t = TerrainConfig::from_params(&params);
        let (sp, center) = (params.suburban_params(), (t.center_x, t.center_y));
        let neighbourhoods = super::super::suburb::plan(&sp, center).neighbourhoods;
        let greens = super::super::suburb::parks::lay(&t.roads, &t.terrain_plots, &neighbourhoods, &sp, center);
        let squares: f64 = greens.iter().flat_map(|g| &g.parks).map(|p| p.area).sum();
        assert!(t.parks.len() > 20, "{} parks", t.parks.len());
        let mut ids: Vec<_> = t.parks.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), t.parks.len(), "park ids are unique");
        for lot in &t.terrain_plots {
            let c = lot.corners.iter().fold((0.0, 0.0), |m, p| (m.0 + p.0 / 4.0, m.1 + p.1 / 4.0));
            assert!(!t.parks.iter().any(|p| p.contains(c.0, c.1)), "lot {} is in a park", lot.id);
        }
        let area: f64 = t.parks.iter().map(|p| p.area).sum();
        let (inner, outer) = (rules::URBAN_OUTER_RADIUS, rules::SUBURBAN_OUTER_RADIUS);
        let share = area / (PI * (outer * outer - inner * inner));
        println!("{} parks, {:.0} square units, {:.1}% of the suburb", t.parks.len(), area, share * 100.0);
        assert!((area - squares).abs() <= 0.02 * squares, "traced {area:.0}, squares {squares:.0}");
        assert!((0.08..=0.18).contains(&share), "parks are {:.1}% of the suburb", share * 100.0);
    }

    /// Plots v2's rings, the table since the flip (#459), build (PV2, #454):
    /// the land is flat out to the rural edge + 100 and rises past it, the
    /// water keeps out past the suburb, and the arterials and the wild
    /// trails meet at the rural edge.
    #[test]
    fn the_default_world_follows_the_table() {
        use capitol_protocol::land::height_at;
        let params = WorldGenParams::default();
        let frame = params.frame();
        assert_eq!(frame, rules::Frame::TABLE);
        let t = TerrainConfig::default_world();

        assert_eq!((t.world_size, t.center_x, t.center_y), (16600.0, 8300.0, 8300.0));
        assert_eq!(t.land.frame, frame);
        assert_eq!((t.land.foothill_radius, t.land.range_radius), (6200.0, 7900.0));
        let at = |r: f64, d: f64| {
            let a = d.to_radians();
            (t.center_x + r * a.stable_cos(), t.center_y + r * a.stable_sin())
        };
        // Flat out to r6200, just short of it (cos and sin can stand a hair past).
        for step in 0..=124 {
            let r = (step as f64 * 50.0).min(6199.99);
            for d in 0..72 {
                let (x, y) = at(r, d as f64 * 5.0);
                assert_eq!(height_at(x, y, &t.land), 0.0, "r{r} at {}°", d * 5);
            }
        }
        let peak = (0..360).map(|d| height_at(at(7600.0, d as f64).0, at(7600.0, d as f64).1, &t.land)).fold(0.0, f64::max);
        assert!(peak > 100.0, "the grown ranges stand up: {peak}");

        // The water keeps out past the grown suburb, and reaches the grown edge.
        let inner = frame.water_inner_radius();
        assert_eq!(inner, 3750.0);
        let courses: Vec<&WaterCourse> = t.rivers.iter().chain(&t.streams).collect();
        assert!(!t.rivers.is_empty() && !t.lakes.is_empty(), "{} rivers, {} lakes", t.rivers.len(), t.lakes.len());
        for c in &courses {
            for p in &c.points {
                let r = frame.radius(p.x, p.y);
                assert!(r > inner - super::super::hydrology::CELL && r <= frame.world_edge() + 0.01, "{} at r{r:.0}", c.id);
            }
        }
        let past_old_edge = courses.iter().flat_map(|c| &c.points).filter(|p| frame.radius(p.x, p.y) > 6800.0).count();
        assert!(past_old_edge > 0, "the water runs out past RW1's edge, r6800");

        // The arterials end at the grown rural edge, and the trails start there.
        for road in t.roads.iter().filter(|r| r.id.strip_prefix("arterial_").is_some_and(|k| k.parse::<u32>().is_ok())) {
            let end = *road.points.last().unwrap();
            assert!((frame.radius(end.0, end.1) - 6100.0).abs() < 1e-6, "{} ends at r{:.1}", road.id, frame.radius(end.0, end.1));
        }
        let trails: Vec<&RoadSegment> = t.roads.iter().filter(|r| r.id.starts_with("wild_trail_")).collect();
        assert_eq!(trails.len(), 8, "a valley trail up from each arterial");
        for trail in &trails {
            let r = frame.radius(trail.points[0].0, trail.points[0].1);
            assert!((r - 6100.0).abs() < 1.0, "{} starts at r{r:.0}", trail.id);
            assert!(trail.points.iter().all(|p| frame.radius(p.0, p.1) < frame.world_edge()), "{} leaves the world", trail.id);
        }
    }

    /// A fixed workload shaped like generation: libm trig through `StableMath`,
    /// pushes, a spatial hash and a sort. It never changes, so how long it takes
    /// measures the machine, not the code. Don't make it faster or slower.
    fn reference_workload() -> f64 {
        use std::collections::HashMap;
        const N: usize = 1_000_000;
        let mut points: Vec<(f64, f64)> = Vec::with_capacity(N);
        let mut cells: HashMap<(i32, i32), u32> = HashMap::new();
        for i in 0..N {
            let (a, r) = (i as f64 * 0.618_034, (i as f64).sqrt());
            let p = (r * a.stable_cos(), r * a.stable_sin());
            *cells.entry(((p.0 / 8.0) as i32, (p.1 / 8.0) as i32)).or_insert(0) += 1;
            points.push(p);
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        points.iter().map(|p| p.0.stable_hypot(p.1)).sum::<f64>() + cells.len() as f64
    }

    /// One world's generation, timed against the reference workload: its CPU
    /// time, the reference's, and the ratio.
    fn generation_ratio(params: &WorldGenParams) -> (std::time::Duration, std::time::Duration, f64) {
        use cpu_time::ThreadTime;
        use std::hint::black_box;

        // CPU time on this thread, not wall time: generation is single-threaded, and in a
        // parallel `cargo test` (or beside other agents' builds) the wall clock counts the
        // time other work held the core. Wall time hit 14.5s that way (#91).
        // The thread's CPU clock ticks every 15.6ms on Windows, and in release one
        // run of the reference is about 60ms, so one reading was ±25% (#464). So
        // it runs until a quarter second has passed, and the time is the mean.
        let reference = || {
            let start = ThreadTime::now();
            let mut runs = 0;
            while runs == 0 || start.elapsed() < std::time::Duration::from_millis(250) {
                black_box(reference_workload());
                runs += 1;
            }
            start.elapsed() / runs
        };
        let before = reference();
        let start = ThreadTime::now();
        black_box(TerrainConfig::from_params(params));
        let elapsed = start.elapsed();
        // CPU time still varies with the machine and with what shares its cores: a fixed
        // 6s limit passed at ~5.4s on one CI runner and failed at ~7.4s on another with
        // the same code, and on a desktop the full parallel suite nearly doubles it
        // (#236). So the limit is in units of the reference workload, timed on this
        // thread either side of generation (the faster of the two, to shed a hiccup).
        let machine = before.min(reference());
        (elapsed, machine, elapsed.as_secs_f64() / machine.as_secs_f64())
    }

    /// The default world generates within [`LIMIT`] times the reference
    /// workload.
    #[test]
    fn test_terrain_generation_performance() {
        // The history, in debug: pre-optimization was ~115s, about 200×. With
        // `capitol_protocol` and libm optimised even in debug (the root
        // Cargo.toml, #221) it was ~2.7s, 4.5–4.8× on a Windows desktop and 3.7× on
        // a CI runner. The farms (R7, #290) took it to 5.8–6.9× alone and 7.5× in the
        // full suite, so the limit went from 7× to 8×. The grown world (PV2, #454)
        // read 8.5× (5.3s), and beside its tests the default went over 8× too.
        //
        // #458 made the lots' clearing, the junctions' weld and the land's noise
        // cheaper, all to the bit (the pin holds). On a desktop busy with other
        // agents' builds, the default world went from 3.3–3.9s (5.9–6.7×) to
        // 2.3–2.8s (4.1–4.8×), and the grown one from 4.8–5.9s (8.4–9.2×) to 3.4s
        // (5.8×). Most of what's left is the suburb's lots and parks (~0.95s, 1.3s
        // grown), the wild trails (~0.65s, 0.85s), the water (~0.3s, 0.5s) and the
        // rural plan (~0.2s, 0.4s). The limit still fails a doubling of the default.
        //
        // In release (#464) the reference speeds up far more than generation does
        // (it's hashing and sorting in the backend, which debug doesn't optimise,
        // where generation leans on the protocol, optimised in both), so a world
        // reads about 2.3× higher. Release has its own limit: the default went
        // from 0.9–1.0s (10.9–13.2×) to 0.63s (9.1–9.6×), and the grown one from
        // 1.1–1.5s (15.6–18×) to 0.89s (13.4–14.2×). It too fails a doubling.
        //
        // Plots v2's flip (#459) made the grown world the default, with every
        // zone's v2 layout on it, so the test times one world again. Its farms
        // ask every walk's farm whether it takes 12 stations, so the default
        // now reads 7.6-8.0x in debug (4.5-4.6s; the farms 1.4s of it, the
        // wild trails 0.85s, the suburb's lots 0.7s, the roads and the water
        // 0.5s each, the parks 0.45s) and 16.2-20x in release (1.13-1.19s;
        // the trails and the farms 0.33s each). Both were at their limits of
        // 8x and 18x, so they rose to 11x and 26x: a doubling, about 16x and
        // 36x, still fails.
        const LIMIT: f64 = if cfg!(debug_assertions) { 11.0 } else { 26.0 };
        // A run beside other tests' threads can read high once: they share the
        // caches and the core's turbo, which the CPU clock doesn't shed. A real
        // slowdown reads high every time, so a world gets three tries (#458).
        const TRIES: usize = 3;
        for (name, params) in [("default", WorldGenParams::default())] {
            let mut best = f64::INFINITY;
            for _ in 0..TRIES {
                let (elapsed, machine, ratio) = generation_ratio(&params);
                println!("The {name} world took {elapsed:?} of CPU, {ratio:.1}× the reference ({machine:?})");
                best = best.min(ratio);
                if best < LIMIT {
                    break;
                }
            }
            assert!(
                best < LIMIT,
                "The {name} world took too long: at best {best:.1}× the reference workload in {TRIES} tries \
                 (expected under {LIMIT}×)"
            );
        }
    }

    #[test]
    fn test_spatial_grid_basic() {
        let mut grid = SpatialGrid::new(0.0, 0.0, 100.0, 10.0);

        // Insert a horizontal line segment
        grid.insert(0, &[(-50.0, 0.0), (50.0, 0.0)]);

        // Point near the line should find it
        let results = grid.query_point(0.0, 0.0, 5.0);
        assert!(results.contains(&0), "Should find segment near origin");

        // Point far away should not find it
        let results = grid.query_point(0.0, 80.0, 5.0);
        assert!(!results.contains(&0), "Should not find segment far away");
    }

    #[test]
    fn test_spatial_grid_multiple_segments() {
        let mut grid = SpatialGrid::new(0.0, 0.0, 100.0, 10.0);

        grid.insert(0, &[(-50.0, -50.0), (-40.0, -50.0)]);
        grid.insert(1, &[(40.0, 40.0), (50.0, 50.0)]);

        // Query near first segment
        let near_first = grid.query_point(-45.0, -50.0, 5.0);
        assert!(near_first.contains(&0));
        assert!(!near_first.contains(&1));

        // Query near second segment
        let near_second = grid.query_point(45.0, 45.0, 5.0);
        assert!(!near_second.contains(&0));
        assert!(near_second.contains(&1));
    }

    /// The road in's stops (#445): on the default world each resolves to a
    /// lot of its own zone within 150 of the south arterial, free or taken,
    /// and they come in order up the road from the spawn to the hall.
    #[test]
    fn the_road_ins_stops_are_lots_of_their_zone_by_the_south_road() {
        use capitol_protocol::tutorial::stops::{self, STOP_ZONES};
        let world = TerrainConfig::default_world();
        let frame = WorldGenParams::default().frame();
        let (cx, cy) = frame.center;
        let road = world
            .roads
            .iter()
            .find(|r| {
                r.road_type == RoadType::Arterial
                    && r.points.iter().all(|p| (p.0 - cx).abs() < 1e-6 && p.1 > cy)
                    && r.points.iter().any(|p| p.1 - cy > frame.suburb_edge())
            })
            .expect("the south arterial runs out past the suburb");
        // Straight south, however many junctions split it
        let lo = road.points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let hi = road.points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        let off_road = |lot: &TerrainPlot| {
            lot.corners
                .iter()
                .map(|p| {
                    let y = p.1.clamp(lo, hi);
                    ((p.0 - cx).powi(2) + (p.1 - y).powi(2)).sqrt()
                })
                .fold(f64::INFINITY, f64::min)
        };
        let mut last = frame.spawn().1;
        for zone in STOP_ZONES {
            let lots = &world.terrain_plots;
            let free = stops::stop(lots, &frame, zone, |_| false).expect("a stop");
            let all_taken = stops::stop(lots, &frame, zone, |_| true).expect("a stop");
            let next = stops::stop(lots, &frame, zone, |id| id == free.lot.id).expect("a stop");
            assert_eq!(free.lot.id, all_taken.lot.id, "{zone}: all taken, still the nearest");
            assert!(all_taken.taken && !free.taken);
            assert_ne!(next.lot.id, free.lot.id, "{zone}: a free one near the road instead");
            assert!(!next.taken, "{zone}: there's a free one within reach");
            for lot in [free.lot, next.lot] {
                assert_eq!(lot.zone, zone.as_str(), "{zone}: {}", lot.id);
                let off = off_road(lot);
                assert!(off <= 150.0, "{zone}: {} is {off:.0} off the south road", lot.id);
            }
            let (_, y) = stops::stop_point(&frame, zone).unwrap();
            assert!(y < last, "{zone} is further up the road");
            println!(
                "{zone}: {} ({:.0} off the road; next {} at {:.0}), {:.0} s on from the last",
                free.lot.id,
                off_road(free.lot),
                next.lot.id,
                off_road(next.lot),
                (last - y) / rules::PLAYER_SPEED
            );
            last = y;
        }
    }

    /// Lots follow a road's shape, not how many points draw it (#436): an
    /// arterial with extra vertices on its straight line, as the urban grid's
    /// weld gives it, lays the same suburban lots and farms.
    #[test]
    fn a_collinear_vertex_on_an_arterial_moves_no_lot() {
        let params = WorldGenParams::default();
        let bare = TerrainConfig::bare(&params);
        let center = (bare.center_x, bare.center_y);
        let (roads, _) = bare.generate_roads_and_rural_plan(&params);
        let mut split = roads.clone();
        let mut added = 0;
        for road in split.iter_mut().filter(|r| r.id.strip_prefix("arterial_").is_some_and(|n| n.parse::<u32>().is_ok())) {
            let mut points = Vec::with_capacity(road.points.len() * 3);
            for w in road.points.windows(2) {
                points.push(w[0]);
                for t in [0.29, 0.61] {
                    points.push((w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t));
                    added += 1;
                }
            }
            points.push(*road.points.last().unwrap());
            road.points = points;
        }
        assert!(added > 0);
        let lay = |roads: Vec<RoadSegment>| {
            let t = TerrainConfig { roads, ..bare.clone() };
            let mut lots = t.generate_suburban_plots_internal(&params.suburban_params());
            let sectors = super::super::rural::sectors_of(&params, center, &t.roads);
            lots.extend(super::super::rural::farms::lay(&params, center, &t.roads, &sectors, None));
            lots
        };
        let (before, after) = (lay(roads), lay(split));
        let by_id: HashMap<&str, &TerrainPlot> = after.iter().map(|l| (l.id.as_str(), l)).collect();
        let mut moved = Vec::new();
        for lot in &before {
            match by_id.get(lot.id.as_str()) {
                None => moved.push(format!("{} dropped", lot.id)),
                Some(other) => {
                    let off = lot.corners.iter().zip(&other.corners).map(|(a, b)| (a.0 - b.0).stable_hypot(a.1 - b.1)).fold(0.0, f64::max);
                    if off > 1e-6 {
                        moved.push(format!("{} moved {off:.3}", lot.id));
                    }
                }
            }
        }
        moved.extend(after.iter().filter(|l| !before.iter().any(|b| b.id == l.id)).map(|l| format!("{} new", l.id)));
        assert!(moved.is_empty(), "{} of {} lots changed: {:?}", moved.len(), before.len(), &moved[..moved.len().min(40)]);
    }
}
