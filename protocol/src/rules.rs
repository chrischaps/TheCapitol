//! Game rules both sides need to agree on: world geometry, speeds, ranges, zones,
//! quality grades, the quality cascade, the water-crossing test and moving
//! on the land ([`ground`]).
//!
//! The server is authoritative. Clients use these to predict (move the local player,
//! grey out an out-of-range node, grade an item from its numeric quality) without
//! waiting for a round trip.

use crate::fmath::StableMath;
use std::fmt;

use crate::terrain::WaterRing;

// ---------------------------------------------------------------------------
// World and timing
// ---------------------------------------------------------------------------

/// Coordinates run 0..WORLD_SIZE on both axes, and y points down (south). The
/// world itself is the disc of radius `WILDERNESS_OUTER_RADIUS` inside that
/// square ([`in_world`]); the corners are out of it. It grew from 8000 to
/// 12000 for the wilds (W1, #194), and every stored position moved by +2000.
pub const WORLD_SIZE: f64 = 12000.0;
/// World centre X; the Capitol tower stands here.
pub const CENTER_X: f64 = WORLD_SIZE / 2.0;
/// World centre Y.
pub const CENTER_Y: f64 = WORLD_SIZE / 2.0;

/// Whether a position is in the world: on the disc of radius
/// `WILDERNESS_OUTER_RADIUS` round the centre, edge included. A move to a
/// destination off it is refused.
pub fn in_world(x: f64, y: f64) -> bool {
    let (dx, dy) = (x - CENTER_X, y - CENTER_Y);
    dx * dx + dy * dy <= WILDERNESS_OUTER_RADIUS * WILDERNESS_OUTER_RADIUS
}

/// Where new players arrive: on the south arterial, out in the rural zone. The
/// road runs straight north to the moat's one bridge, so the first thing a
/// player sees is the way in, with the tower at the end of it.
pub const SPAWN_X: f64 = CENTER_X;
/// Spawn Y: 3350 south of the centre, 350 past the suburb's outer edge. It
/// stayed put when the suburb deepened (#117), so it no longer tracks the
/// rural zone's outer edge.
pub const SPAWN_Y: f64 = CENTER_Y + 3350.0;
/// New players land up to this far either side of `SPAWN_X`, so a crowd of
/// arrivals doesn't stack on one point. It keeps them on the 60-wide arterial.
pub const SPAWN_SCATTER: f64 = 20.0;

/// Length of one server tick in milliseconds.
pub const TICK_MS: u64 = 100;
/// Length of one server tick in seconds.
pub const TICK_SECS: f64 = TICK_MS as f64 / 1000.0;

/// One in-game day, in real seconds: two hours, so a play session sees dusk
/// come. A real 24 h day would mean most players only ever saw one phase. It
/// divides 24 h, so in-game midnight falls on every UTC midnight too.
pub const DAY_LENGTH_SECS: i64 = 2 * 60 * 60;

/// The time of day at `unix_ms` (the server's clock, Unix milliseconds), from
/// 0.0 (midnight) through 0.5 (noon) to just under 1.0. Every client works
/// it out from the same clock, so they all see the same hour.
pub fn time_of_day(unix_ms: i64) -> f64 {
    let day_ms = DAY_LENGTH_SECS * 1000;
    unix_ms.rem_euclid(day_ms) as f64 / day_ms as f64
}

/// The in-game day `unix_ms` falls in: whole days since the Unix epoch. The
/// Gazette (E3.9) is filed by it.
pub fn day_number(unix_ms: i64) -> i64 {
    unix_ms.div_euclid(DAY_LENGTH_SECS * 1000)
}

/// The Unix milliseconds of day `day`'s midnight.
pub fn day_start_ms(day: i64) -> i64 {
    day * DAY_LENGTH_SECS * 1000
}

/// The day of the Gazette's first edition, No. 1: 2026-10-19's first in-game
/// day (UTC midnight).
pub const GAZETTE_FIRST_DAY: i64 = 248_940;

/// The Gazette's number for the edition about day `day`.
pub fn gazette_number(day: i64) -> i64 {
    day - GAZETTE_FIRST_DAY + 1
}

/// The Capitol's week, named for the city from the tower outward. An
/// in-game day is two real hours, so a week is fourteen of them.
pub const WEEKDAYS: [&str; 7] =
    ["Towerday", "Moatday", "Canalday", "Guildday", "Marketday", "Fieldday", "Wildday"];

/// The name of day `day`'s weekday.
pub fn weekday(day: i64) -> &'static str {
    WEEKDAYS[(day - GAZETTE_FIRST_DAY).rem_euclid(7) as usize]
}

// ---------------------------------------------------------------------------
// Speeds and ranges (world units)
// ---------------------------------------------------------------------------

/// Default player speed in units per second. New players are created with it.
pub const PLAYER_SPEED: f64 = 50.0;
/// A move that ends within this distance of the destination snaps onto it.
pub const ARRIVAL_EPSILON: f64 = 0.5;
/// Maximum distance from a resource node to start extracting.
pub const EXTRACTION_RANGE: f64 = 50.0;
/// Fallback station interaction range. Each station type carries its own
/// `interaction_range`; this applies when the type doesn't set one.
pub const STATION_INTERACTION_RANGE: f64 = 75.0;
/// Maximum distance between two players to trade.
pub const TRADE_RANGE: f64 = 75.0;
/// Seconds a trade request waits for an answer before the server drops it and
/// sends both players `trade_request_expired`.
pub const TRADE_REQUEST_TTL_SECS: u64 = 30;
/// The side of a cell of the interest grid (C0.4). `positions` lists the
/// players in your cell and the eight round it: everyone within this
/// distance of you, and nobody further than twice it on either axis. A
/// player who leaves those cells is out of sight, not gone: only
/// `player_left` means they've left the world.
pub const INTEREST_CELL: f64 = 300.0;
/// Minimum distance between two placed stations.
pub const STATION_MIN_SPACING: f64 = 50.0;
/// `GET /world/nearby` returns resource nodes within this radius of the player,
/// unless the request asks for another `radius`.
pub const NEARBY_NODE_RANGE: f64 = 200.0;
/// The widest `radius` `GET /world/nearby` honours. A larger one is cut to this.
/// It covers the Flint client's district camera (#79).
pub const NEARBY_NODE_RANGE_MAX: f64 = 800.0;
/// `GET /stations/nearby` returns stations within this many units of the player on
/// each axis (a square, not a circle).
pub const NEARBY_STATION_HALF_EXTENT: f64 = 500.0;

/// Fee on Strand/Fiber exchange, as a fraction (0.5%).
pub const EXCHANGE_FEE_RATE: f64 = 0.005;

/// What depositing `fiber` units at the exchange gives: `(fee, strands)`,
/// the fee rounded down. `POST /exchange/deposit` charges exactly this, so
/// a client's preview is the server's answer.
pub fn deposit_quote(fiber: i64) -> (i64, i64) {
    let fee = ((fiber as f64) * EXCHANGE_FEE_RATE).floor() as i64;
    (fee, fiber - fee)
}

/// What withdrawing `fiber` units costs: `(strands, fee)`, where `strands`
/// is `ceil(fiber / 0.995)` and includes the fee. `POST /exchange/withdraw`
/// charges exactly this.
pub fn withdraw_quote(fiber: i64) -> (i64, i64) {
    let strands = ((fiber as f64) / (1.0 - EXCHANGE_FEE_RATE)).ceil() as i64;
    (strands, strands - fiber)
}

// ---------------------------------------------------------------------------
// Quality
// ---------------------------------------------------------------------------

/// The cascade: a craft's output quality from its inputs, station and tool.
pub mod quality;

/// Hand tools: effective power, durability and the wear curve.
pub mod tools;

/// Stacks: blending, `min_quality`, and which merges the server makes unasked.
pub mod stacking;

/// The market: the listing fee, order bounds and the grade test (E3.1).
pub mod market;

/// The bazaar: stall rent, the stall's range, shop names and listings (E3.10).
pub mod bazaar;

/// Resource types: yield by tool tier, spawn tables and cluster shapes.
pub mod resources;

/// Moving on the land: slope, cliffs, and the wild water (W4).
pub mod ground;

/// Letter grade for an item quality (0-100): A >= 90, B >= 75, C >= 55, D >= 35,
/// otherwise F. WebSocket events carry only the number, so clients grade it here.
pub fn quality_grade(quality: i32) -> &'static str {
    match quality {
        90..=100 => "A",
        75..=89 => "B",
        55..=74 => "C",
        35..=54 => "D",
        _ => "F",
    }
}

// ---------------------------------------------------------------------------
// Zones
// ---------------------------------------------------------------------------

// The zone radii. This is the one table: the terrain generator's defaults, the
// DB `zones` rows (synced on server startup), `/bureaucracy/zones` and the
// clients' zone rings all come from here. Each zone's inner radius is the
// outer radius of the zone inside it.

/// Outer radius of the Capitol, inside the moat.
pub const CAPITOL_RADIUS: f64 = 200.0;
/// Outer radius of the Trade District.
pub const TRADE_OUTER_RADIUS: f64 = 600.0;
/// Outer radius of the Guild District.
pub const GUILD_OUTER_RADIUS: f64 = 1000.0;
/// Outer radius of the Urban zone.
pub const URBAN_OUTER_RADIUS: f64 = 1800.0;
/// Outer radius of the Suburban zone. The suburb is 1200 deep so that a
/// neighbourhood fits 5 rows of streets. At 800 deep every one was 3 rows (#117).
pub const SUBURBAN_OUTER_RADIUS: f64 = 3000.0;
/// Outer radius of the Rural zone.
pub const RURAL_OUTER_RADIUS: f64 = 3800.0;
/// Outer radius of the Wilderness, "the wilds": the edge of the world disc,
/// 2200 past the rural edge (W1, #194).
pub const WILDERNESS_OUTER_RADIUS: f64 = WORLD_SIZE / 2.0;

/// Each zone's outer radius, in `Zone::ALL` order.
pub const ZONE_OUTER_RADII: [f64; 7] = [
    CAPITOL_RADIUS,
    TRADE_OUTER_RADIUS,
    GUILD_OUTER_RADIUS,
    URBAN_OUTER_RADIUS,
    SUBURBAN_OUTER_RADIUS,
    RURAL_OUTER_RADIUS,
    WILDERNESS_OUTER_RADIUS,
];

/// Zones are concentric rings around the Capitol at (CENTER_X, CENTER_Y).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Zone {
    Capitol,
    TradeDistrict,
    GuildDistrict,
    Urban,
    Suburban,
    Rural,
    Wilderness,
}

impl Zone {
    pub const CENTER_X: f64 = CENTER_X;
    pub const CENTER_Y: f64 = CENTER_Y;
    pub const WORLD_SIZE: f64 = WORLD_SIZE;

    /// All zones, from the centre outward.
    pub const ALL: [Zone; 7] = [
        Zone::Capitol,
        Zone::TradeDistrict,
        Zone::GuildDistrict,
        Zone::Urban,
        Zone::Suburban,
        Zone::Rural,
        Zone::Wilderness,
    ];

    /// Which zone a position falls into, by distance from the centre. It reads
    /// the static table, not an admin regenerate's radii; see #118.
    pub fn from_position(x: f64, y: f64) -> Self {
        let dx = x - CENTER_X;
        let dy = y - CENTER_Y;
        let distance = (dx * dx + dy * dy).sqrt();
        Self::ALL
            .into_iter()
            .find(|zone| distance < zone.radii().1)
            .unwrap_or(Zone::Wilderness)
    }

    /// The zone ID used in the database and on the wire (`zone_changed`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Zone::Capitol => "capitol",
            Zone::TradeDistrict => "trade_district",
            Zone::GuildDistrict => "guild_district",
            Zone::Urban => "urban",
            Zone::Suburban => "suburban",
            Zone::Rural => "rural",
            Zone::Wilderness => "wilderness",
        }
    }

    /// Parse a zone ID.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|zone| zone.as_str() == s)
    }

    /// The zone's display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Zone::Capitol => "The Capitol",
            Zone::TradeDistrict => "Trade District",
            Zone::GuildDistrict => "Guild District",
            Zone::Urban => "Urban Zone",
            Zone::Suburban => "Suburban Zone",
            Zone::Rural => "Rural Zone",
            Zone::Wilderness => "Wilderness",
        }
    }

    /// Whether the zone has private plots.
    pub fn has_plots(&self) -> bool {
        matches!(
            self,
            Zone::GuildDistrict | Zone::Urban | Zone::Suburban | Zone::Rural
        )
    }

    /// Ring order: 0 is the centre, higher is further out.
    pub fn ring_order(&self) -> u8 {
        *self as u8
    }

    /// Inner and outer radius of the zone's ring.
    pub fn radii(&self) -> (f64, f64) {
        let i = self.ring_order() as usize;
        let inner = if i == 0 { 0.0 } else { ZONE_OUTER_RADII[i - 1] };
        (inner, ZONE_OUTER_RADII[i])
    }
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Water crossing
// ---------------------------------------------------------------------------

/// Where a straight move was stopped by water.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterBlock {
    /// Where the mover stops: just short of the water's edge.
    pub x: f64,
    pub y: f64,
    /// ID of the ring that blocked it (`moat`, `canal_inner`, ...).
    pub ring_id: String,
}

/// A water's name, for people: `canal_outer` is "the outer canal", and the
/// wilds' `river_3` is "the river" (W4). An id it doesn't know reads as
/// itself, underscores as spaces. The server's `movement_blocked` reason
/// says it, and a client can say it from the message's `water_id`.
pub fn water_name(ring_id: &str) -> String {
    match ring_id {
        "moat" => "the moat".into(),
        "canal_inner" => "the inner canal".into(),
        "canal_outer" => "the outer canal".into(),
        "" => "the water".into(),
        wild if wild.starts_with("river_") => "the river".into(),
        wild if wild.starts_with("stream_") => "the stream".into(),
        wild if wild.starts_with("lake_") => "the lake".into(),
        other => format!("the {}", other.replace('_', " ")),
    }
}

/// How far short of the water's edge a blocked move stops.
const BLOCK_BACKOFF: f64 = 2.0;

/// Test a straight move from `from` to `to` against every water ring, in order.
/// Returns the first block, or `None` if the move stays dry or crosses only at bridges.
///
/// This is the server's movement test. A client can run it to predict a
/// `movement_blocked` before the server sends one.
pub fn check_water_crossing(
    center: (f64, f64),
    rings: &[WaterRing],
    from: (f64, f64),
    to: (f64, f64),
) -> Option<WaterBlock> {
    rings.iter().find_map(|ring| {
        ring_crossing(center, ring, from, to).map(|(x, y)| WaterBlock {
            x,
            y,
            ring_id: ring.id.clone(),
        })
    })
}

/// The ring whose water `at` stands in, if any. A bridge is over the water,
/// so a point on one is in it too: nothing is built on a bridge.
pub fn in_water(center: (f64, f64), rings: &[WaterRing], at: (f64, f64)) -> Option<&WaterRing> {
    let d = distance(center, at);
    rings
        .iter()
        .find(|ring| d > ring.inner_radius() && d < ring.outer_radius())
}

/// Test a straight move against one ring. Returns the stopping point if it's blocked.
pub fn ring_crossing(
    center: (f64, f64),
    ring: &WaterRing,
    from: (f64, f64),
    to: (f64, f64),
) -> Option<(f64, f64)> {
    let (from_x, from_y) = from;
    let (to_x, to_y) = to;
    let from_dist = distance(center, from);
    let to_dist = distance(center, to);

    let inner = ring.inner_radius();
    let outer = ring.outer_radius();

    // Does the path touch the ring at all?
    let crosses_inward = from_dist >= outer && to_dist <= inner;
    let crosses_outward = from_dist <= inner && to_dist >= outer;
    let enters_from_outside = from_dist >= outer && to_dist > inner && to_dist < outer;
    let enters_from_inside = from_dist <= inner && to_dist > inner && to_dist < outer;
    let exits_to_outside = from_dist > inner && from_dist < outer && to_dist >= outer;
    let exits_to_inside = from_dist > inner && from_dist < outer && to_dist <= inner;

    if !crosses_inward
        && !crosses_outward
        && !enters_from_outside
        && !enters_from_inside
        && !exits_to_outside
        && !exits_to_inside
    {
        return None;
    }

    // Where the path meets the inner and outer edges. Each must be on a bridge.
    let intersections = [inner, outer]
        .into_iter()
        .filter_map(|radius| line_circle_intersection(center, from, to, radius));

    for (ix, iy) in intersections {
        let angle = angle_from_center(center, ix, iy);
        let on_bridge = ring
            .bridges
            .iter()
            .any(|b| b.contains_angle(angle, ring.radius));

        if !on_bridge {
            // Blocked: stop just before the water.
            let dx = to_x - from_x;
            let dy = to_y - from_y;
            let len = (dx * dx + dy * dy).sqrt();
            if len > 0.0 {
                return Some((
                    ix - (dx / len) * BLOCK_BACKOFF,
                    iy - (dy / len) * BLOCK_BACKOFF,
                ));
            }
            return Some((ix, iy));
        }
    }

    None
}

/// The first point where the segment `from -> to` meets the circle of `radius`
/// around `center`, if any.
fn line_circle_intersection(
    center: (f64, f64),
    from: (f64, f64),
    to: (f64, f64),
    radius: f64,
) -> Option<(f64, f64)> {
    let x1 = from.0 - center.0;
    let y1 = from.1 - center.1;
    let x2 = to.0 - center.0;
    let y2 = to.1 - center.1;

    let dx = x2 - x1;
    let dy = y2 - y1;

    let a = dx * dx + dy * dy;
    let b = 2.0 * (x1 * dx + y1 * dy);
    let c = x1 * x1 + y1 * y1 - radius * radius;

    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }

    let sqrt_disc = discriminant.sqrt();
    let t1 = (-b - sqrt_disc) / (2.0 * a);
    let t2 = (-b + sqrt_disc) / (2.0 * a);

    [t1, t2]
        .into_iter()
        .find(|t| (0.0..=1.0).contains(t))
        .map(|t| (from.0 + dx * t, from.1 + dy * t))
}

fn distance(center: (f64, f64), p: (f64, f64)) -> f64 {
    let dx = p.0 - center.0;
    let dy = p.1 - center.1;
    (dx * dx + dy * dy).sqrt()
}

/// Angle of a point around `center`, in degrees: 0 = East, 90 = South (y is down).
pub fn angle_from_center(center: (f64, f64), x: f64, y: f64) -> f64 {
    (y - center.1).stable_atan2(x - center.0).to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::Bridge;

    #[test]
    fn time_of_day_runs_midnight_to_midnight() {
        let day = DAY_LENGTH_SECS * 1000;
        assert_eq!(time_of_day(0), 0.0);
        assert_eq!(time_of_day(day / 4), 0.25);
        assert_eq!(time_of_day(day / 2), 0.5);
        // It wraps: the next midnight is 0 again, and just before it is just under 1.
        assert_eq!(time_of_day(day), 0.0);
        assert_eq!(time_of_day(7 * day + day / 2), 0.5);
        assert!(time_of_day(day - 1) < 1.0 && time_of_day(day - 1) > 0.999);
        // Before 1970 still lands in 0..1.
        assert_eq!(time_of_day(-day / 4), 0.75);
        // In-game midnight falls on UTC midnight (2026-10-17T00:00:00Z).
        assert_eq!(time_of_day(1_792_195_200_000), 0.0);
    }

    #[test]
    fn days_are_numbered_from_midnight() {
        let day = DAY_LENGTH_SECS * 1000;
        assert_eq!(day_number(0), 0);
        assert_eq!(day_number(day - 1), 0);
        assert_eq!(day_number(day), 1);
        assert_eq!(day_number(-1), -1);
        assert_eq!(day_start_ms(day_number(5 * day + 17)), 5 * day);
        // 2026-10-19T00:00:00Z is the Gazette's first day, a Towerday
        assert_eq!(day_number(1_792_368_000_000), GAZETTE_FIRST_DAY);
        assert_eq!(gazette_number(GAZETTE_FIRST_DAY), 1);
        assert_eq!(weekday(GAZETTE_FIRST_DAY), "Towerday");
        assert_eq!(weekday(GAZETTE_FIRST_DAY + 4), "Marketday");
        assert_eq!(weekday(GAZETTE_FIRST_DAY - 1), "Wildday");
    }

    #[test]
    fn exchange_quotes() {
        assert_eq!(deposit_quote(199), (0, 199));
        assert_eq!(deposit_quote(200), (1, 199));
        assert_eq!(deposit_quote(450), (2, 448));
        assert_eq!(withdraw_quote(1), (2, 1));
        assert_eq!(withdraw_quote(100), (101, 1));
        assert_eq!(withdraw_quote(199), (200, 1));
        assert_eq!(withdraw_quote(200), (202, 2));
        // The float rule is the exact one, with no rounding surprises.
        for n in 1..=200_000i64 {
            assert_eq!(deposit_quote(n).0, n / 200, "deposit {n}");
            assert_eq!(withdraw_quote(n).0, (200 * n + 198) / 199, "withdraw {n}");
        }
    }

    #[test]
    fn the_water_is_named_for_people() {
        assert_eq!(water_name("canal_outer"), "the outer canal");
        assert_eq!(water_name("canal_inner"), "the inner canal");
        assert_eq!(water_name("moat"), "the moat");
        assert_eq!(water_name("canal_far"), "the canal far");
        assert_eq!(water_name(""), "the water");
        assert_eq!(water_name("river_12"), "the river");
        assert_eq!(water_name("stream_0"), "the stream");
        assert_eq!(water_name("lake_3"), "the lake");
    }

    #[test]
    fn grades_match_server_thresholds() {
        assert_eq!(quality_grade(100), "A");
        assert_eq!(quality_grade(90), "A");
        assert_eq!(quality_grade(89), "B");
        assert_eq!(quality_grade(75), "B");
        assert_eq!(quality_grade(74), "C");
        assert_eq!(quality_grade(55), "C");
        assert_eq!(quality_grade(54), "D");
        assert_eq!(quality_grade(35), "D");
        assert_eq!(quality_grade(34), "F");
        assert_eq!(quality_grade(0), "F");
    }

    #[test]
    fn zones_by_distance() {
        assert_eq!(Zone::from_position(CENTER_X, CENTER_Y), Zone::Capitol);
        // Each ring runs from its inner radius up to, not including, its outer.
        for zone in Zone::ALL {
            let (inner, outer) = zone.radii();
            assert_eq!(Zone::from_position(CENTER_X + inner, CENTER_Y), zone, "{zone} inner edge");
            assert_eq!(Zone::from_position(CENTER_X, CENTER_Y - (outer - 1.0)), zone, "{zone} outer edge");
        }
        // Past the world disc, in the corners, is still Wilderness.
        assert_eq!(Zone::from_position(0.0, 0.0), Zone::Wilderness);
    }

    #[test]
    fn zone_rings_tile_the_world_disc() {
        let mut inner = 0.0;
        for zone in Zone::ALL {
            let (zone_inner, outer) = zone.radii();
            assert_eq!(zone_inner, inner, "{zone} starts where the ring inside it ends");
            assert!(outer > zone_inner, "{zone} has width");
            inner = outer;
        }
        assert_eq!(inner, WORLD_SIZE / 2.0);
        assert_eq!(Zone::from_position(SPAWN_X, SPAWN_Y), Zone::Rural);
    }

    /// The world is the r6000 disc: the wilds run 2200 past the rural edge,
    /// and the square's corners are out of it.
    #[test]
    fn the_world_is_the_disc() {
        assert_eq!((WORLD_SIZE, CENTER_X, CENTER_Y), (12000.0, 6000.0, 6000.0));
        assert_eq!(Zone::Wilderness.radii(), (3800.0, 6000.0));
        assert!(!Zone::Wilderness.has_plots());
        assert!(in_world(CENTER_X, CENTER_Y));
        assert!(in_world(CENTER_X, CENTER_Y + WILDERNESS_OUTER_RADIUS));
        assert!(in_world(SPAWN_X, SPAWN_Y));
        assert!(!in_world(CENTER_X + 4300.0, CENTER_Y + 4300.0), "a corner");
        assert!(!in_world(0.0, 0.0));
        assert!(!in_world(CENTER_X - WILDERNESS_OUTER_RADIUS - 0.01, CENTER_Y));
    }

    #[test]
    fn zone_ids_round_trip() {
        for zone in Zone::ALL {
            assert_eq!(Zone::from_str(zone.as_str()), Some(zone));
        }
        assert_eq!(Zone::from_str("atlantis"), None);
        assert_eq!(Zone::Wilderness.ring_order(), 6);
    }

    fn moat() -> WaterRing {
        WaterRing {
            id: "moat".into(),
            radius: 200.0,
            width: 20.0,
            bridges: vec![Bridge {
                angle_degrees: 0.0,
                width: 30.0,
            }],
        }
    }

    #[test]
    fn crossing_at_a_bridge_is_allowed() {
        let rings = [moat()];
        // Due east, straight over the bridge at 0 degrees.
        let block = check_water_crossing(
            (CENTER_X, CENTER_Y),
            &rings,
            (6150.0, 6000.0),
            (6250.0, 6000.0),
        );
        assert_eq!(block, None);
    }

    #[test]
    fn crossing_away_from_a_bridge_is_blocked_short_of_the_water() {
        let rings = [moat()];
        // Due south: no bridge there.
        let block = check_water_crossing(
            (CENTER_X, CENTER_Y),
            &rings,
            (6000.0, 6150.0),
            (6000.0, 6250.0),
        )
        .expect("blocked");
        assert_eq!(block.ring_id, "moat");
        assert!((block.x - 6000.0).abs() < 1e-9);
        // Inner edge is at radius 190; the mover stops 2 units short.
        assert!((block.y - 6188.0).abs() < 1e-9);
    }

    #[test]
    fn the_water_is_between_a_rings_edges_bridges_and_all() {
        let rings = [moat()];
        let at = |x: f64, y: f64| in_water((CENTER_X, CENTER_Y), &rings, (CENTER_X + x, CENTER_Y + y)).map(|r| r.id.as_str());
        // The moat's edges are at radius 190 and 210.
        assert_eq!(at(0.0, 189.0), None, "the inner bank");
        assert_eq!(at(0.0, 200.0), Some("moat"));
        assert_eq!(at(0.0, 211.0), None, "the outer bank");
        assert_eq!(at(200.0, 0.0), Some("moat"), "on the east bridge");
    }

    #[test]
    fn moves_inside_a_ring_never_touch_it() {
        let rings = [moat()];
        let block = check_water_crossing(
            (CENTER_X, CENTER_Y),
            &rings,
            (6000.0, 6000.0),
            (6050.0, 6050.0),
        );
        assert_eq!(block, None);
    }
}
