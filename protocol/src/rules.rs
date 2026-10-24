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
/// It grew again, to 13600, when the rural ring widened to two roads deep
/// (RD10, #308), and every stored position moved by +800. It grew to 16600
/// when every zone took Plots v2's rings (V2.0, #459), and every stored
/// position moved by +1500.
pub const WORLD_SIZE: f64 = 16600.0;
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
/// Spawn Y: [`SPAWN_PAST_SUBURB`] past the suburb's outer edge, 3950 south of
/// the centre (3350 before Plots v2, #459). It stayed put when the suburb
/// deepened (#117), so it no longer tracks the rural zone's outer edge; it
/// follows the suburb's ([`Frame::spawn`], PV2, #454).
pub const SPAWN_Y: f64 = CENTER_Y + SUBURBAN_OUTER_RADIUS + SPAWN_PAST_SUBURB;
/// How far past the suburb's outer edge new players arrive, on the south
/// arterial: just into the farms.
pub const SPAWN_PAST_SUBURB: f64 = 350.0;
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

/// The Capitol's week that day `day` falls in, Towerday to Wildday, counted
/// from the Gazette's first (commons S3.2). The leaderboards are kept by it.
pub fn week_of(day: i64) -> i64 {
    (day - GAZETTE_FIRST_DAY).div_euclid(7)
}

/// The first day (a Towerday) of week `week`.
pub fn week_first_day(week: i64) -> i64 {
    GAZETTE_FIRST_DAY + week * 7
}

/// Whether day `day` ends its week. Its edition, published at the next
/// week's first midnight, carries the week's names (S3.2).
pub fn ends_week(day: i64) -> bool {
    week_of(day + 1) != week_of(day)
}

/// How many weeks of tax a plot may owe before it's foreclosed (#131). Tax
/// weeks are the calendar's, 168 hours, charged an hour at a time, not the
/// Capitol's fourteen-hour [`WEEKDAYS`]. `PlotInfo::forecloses_at` is this
/// many weeks of the plot's tax, so a client reads a week's tax from it
/// (the road in's tax card, #449).
pub const FORECLOSE_AFTER_WEEKS: i64 = 4;

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

/// The cell of the interest grid a point is in, by column and row: the
/// server's bucket for it, and what the client's dev overlay shows (D5.2).
pub fn interest_cell(x: f64, y: f64) -> (i32, i32) {
    ((x / INTEREST_CELL).floor() as i32, (y / INTEREST_CELL).floor() as i32)
}
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

/// Fee on Strand/Fiber exchange by default, as a fraction (0.5%). It's a
/// live knob since D6.1: what's charged now is the server's
/// [`crate::tunables::Fees`], whose `deposit_quote` and `withdraw_quote` are
/// what `POST /exchange/deposit` and `/withdraw` charge.
pub const EXCHANGE_FEE_RATE: f64 = 0.005;

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

/// Walking on a heading: the `steer` message's heading, throttle and lease (T0.2).
pub mod steer;

/// Free text: a line of talk and a letter, cleaned and counted alike (S0.1).
pub mod talk;

/// The Contract Court's fee on commissions and wage offers (S0.2).
pub mod court;

/// Standing in words: only the first band's until S3.1 (D8, S0.3).
pub mod standing;

/// Guilds: names and tags, the fees, and each role's rights (D10, S4.1).
pub mod guild;
/// Piece-rate wages: an offer's bounds, its pay and why a shift ends (D7, S2.6).
pub mod wages;

/// The building that comes with a deed: its footprint, indoors and out (V3.1).
pub mod building;

/// Weather: a station's condition, its wear outdoors, and its repair (V3.2).
pub mod weather;

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
/// Outer radius of the Urban zone: five rings of blocks, the outer ring
/// street at r2125.5 (Plots v2, #455, #459; r1800 and three rings before).
pub const URBAN_OUTER_RADIUS: f64 = 2160.0;
/// Outer radius of the Suburban zone. The suburb is 1440 deep so that a
/// neighbourhood fits 5 rows of lots about 134 deep (Plots v2, #456, #459).
/// It was 1200 deep, r1800 to r3000, for 5 rows of about 106, and at 800 deep
/// every one was 3 rows (#117).
pub const SUBURBAN_OUTER_RADIUS: f64 = 3600.0;
/// Outer radius of the Rural zone. The ring is 2500 deep so that four main
/// lanes with farms of about 35k either side fit across it (Plots v2, #457,
/// #459). It was 1600 deep, r3000 to r4600, for two (rural plan RD10, #308).
pub const RURAL_OUTER_RADIUS: f64 = 6100.0;
/// Outer radius of the Wilderness, "the wilds": the edge of the world disc,
/// 2200 past the rural edge (W1, #194; moved out with it, #308, #459).
pub const WILDERNESS_OUTER_RADIUS: f64 = WORLD_SIZE / 2.0;

/// How far past the suburb's outer edge the wild water may come in: the
/// hydrology's inner rim, so the suburb stays dry.
pub const WATER_PAST_SUBURB: f64 = 150.0;
/// The hydrology's inner rim (r3750): no course or lake comes in past it.
/// Flint's `land::FROM` starts the land just inside it.
pub const WATER_INNER_RADIUS: f64 = SUBURBAN_OUTER_RADIUS + WATER_PAST_SUBURB;

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

/// One frame of radii: where the world's centre is and how far out each
/// zone's ring runs (PV2, #454). The protocol's table is [`Frame::TABLE`],
/// and it's what everything reads at runtime. The generators read a frame
/// instead, so a test can lay a world on other rings without moving the
/// default one. The walk ([`ground`]), [`Zone::from_position`] and the
/// non-wild node planner read the table (#465): Plots v2's rings are the
/// table since the flip (#459).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// The world's centre: the Capitol tower.
    pub center: (f64, f64),
    /// Each zone's outer radius, in `Zone::ALL` order. The last, the
    /// Wilderness's, is the edge of the world disc.
    pub radii: [f64; 7],
}

impl Default for Frame {
    fn default() -> Self {
        Self::TABLE
    }
}

impl Frame {
    /// The protocol's table: [`ZONE_OUTER_RADII`] round ([`CENTER_X`], [`CENTER_Y`]).
    pub const TABLE: Frame = Frame { center: (CENTER_X, CENTER_Y), radii: ZONE_OUTER_RADII };

    /// The frame of a world `world_size` across, with these zones inside the
    /// Wilderness (`Zone::ALL` order, Capitol to Rural): the centre is the
    /// square's middle, and the Wilderness runs to the disc's edge.
    pub fn new(world_size: f64, inner: [f64; 6]) -> Self {
        let half = world_size / 2.0;
        let [a, b, c, d, e, f] = inner;
        Frame { center: (half, half), radii: [a, b, c, d, e, f, half] }
    }

    /// The side of the world's square: the disc's diameter.
    pub fn world_size(&self) -> f64 {
        2.0 * self.world_edge()
    }

    /// Inner and outer radius of a zone's ring.
    pub fn radii_of(&self, zone: Zone) -> (f64, f64) {
        let i = zone.ring_order() as usize;
        let inner = if i == 0 { 0.0 } else { self.radii[i - 1] };
        (inner, self.radii[i])
    }

    /// The suburb's outer edge.
    pub fn suburb_edge(&self) -> f64 {
        self.radii[Zone::Suburban as usize]
    }

    /// The rural ring's outer edge, where the wilds start.
    pub fn rural_edge(&self) -> f64 {
        self.radii[Zone::Rural as usize]
    }

    /// The edge of the world disc.
    pub fn world_edge(&self) -> f64 {
        self.radii[Zone::Wilderness as usize]
    }

    /// The hydrology's inner rim: [`WATER_PAST_SUBURB`] past the suburb.
    pub fn water_inner_radius(&self) -> f64 {
        self.suburb_edge() + WATER_PAST_SUBURB
    }

    /// Where new players arrive: on the south arterial, [`SPAWN_PAST_SUBURB`]
    /// past the suburb.
    pub fn spawn(&self) -> (f64, f64) {
        (self.center.0, self.center.1 + self.suburb_edge() + SPAWN_PAST_SUBURB)
    }

    /// How far a point is from the centre.
    pub fn radius(&self, x: f64, y: f64) -> f64 {
        let (dx, dy) = (x - self.center.0, y - self.center.1);
        (dx * dx + dy * dy).sqrt()
    }

    /// Which zone a point falls into, as [`Zone::from_position`] reads the
    /// table.
    pub fn zone_at(&self, x: f64, y: f64) -> Zone {
        let distance = self.radius(x, y);
        Zone::ALL
            .into_iter()
            .find(|zone| distance < self.radii[zone.ring_order() as usize])
            .unwrap_or(Zone::Wilderness)
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
pub(crate) const BLOCK_BACKOFF: f64 = 2.0;

/// Test a straight move from `from` to `to` against every water ring, in order.
/// Returns the first block, or `None` if the move stays dry or crosses only at bridges.
///
/// The server's walk tests each step with [`ring_crossing`], through
/// [`ground::Ground::walk_step`]. This is the same test over a whole line,
/// for a route, or a check that the way in is open.
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
        assert_eq!((week_of(GAZETTE_FIRST_DAY), week_of(GAZETTE_FIRST_DAY + 6), week_of(GAZETTE_FIRST_DAY + 7)), (0, 0, 1));
        assert_eq!(week_of(GAZETTE_FIRST_DAY - 1), -1);
        assert_eq!(week_first_day(week_of(GAZETTE_FIRST_DAY + 9)), GAZETTE_FIRST_DAY + 7);
        assert!(ends_week(GAZETTE_FIRST_DAY + 6) && !ends_week(GAZETTE_FIRST_DAY + 5));
        assert_eq!(weekday(week_first_day(3)), "Towerday");
    }

    #[test]
    fn exchange_quotes() {
        let fees = crate::tunables::Fees::default();
        assert_eq!(fees.deposit_quote(199), (0, 199));
        assert_eq!(fees.deposit_quote(200), (1, 199));
        assert_eq!(fees.deposit_quote(450), (2, 448));
        assert_eq!(fees.withdraw_quote(1), (2, 1));
        assert_eq!(fees.withdraw_quote(100), (101, 1));
        assert_eq!(fees.withdraw_quote(199), (200, 1));
        assert_eq!(fees.withdraw_quote(200), (202, 2));
        // The float rule is the exact one, with no rounding surprises.
        for n in 1..=200_000i64 {
            assert_eq!(fees.deposit_quote(n).0, n / 200, "deposit {n}");
            assert_eq!(fees.withdraw_quote(n).0, (200 * n + 198) / 199, "withdraw {n}");
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

    /// The table's frame is the table: its zones, its spawn and its water's
    /// rim (PV2, #454).
    #[test]
    fn the_table_frame_is_the_table() {
        let f = Frame::TABLE;
        assert_eq!(f, Frame::new(WORLD_SIZE, [200.0, 600.0, 1000.0, 2160.0, 3600.0, 6100.0]));
        assert_eq!(f.world_size(), WORLD_SIZE);
        assert_eq!(f.spawn(), (SPAWN_X, SPAWN_Y));
        assert_eq!(SPAWN_Y, CENTER_Y + 3950.0);
        assert_eq!((f.water_inner_radius(), WATER_INNER_RADIUS), (3750.0, 3750.0));
        for zone in Zone::ALL {
            assert_eq!(f.radii_of(zone), zone.radii());
        }
        for i in 0..400 {
            let (x, y) = (i as f64 * 37.3 % WORLD_SIZE, i as f64 * 91.7 % WORLD_SIZE);
            assert_eq!(f.zone_at(x, y), Zone::from_position(x, y));
        }
        // RW1's world (#308), the table before Plots v2 (#459).
        let before = Frame::new(13600.0, [200.0, 600.0, 1000.0, 1800.0, 3000.0, 4600.0]);
        assert_eq!(before.center, (6800.0, 6800.0));
        assert_eq!(before.radii_of(Zone::Wilderness), (4600.0, 6800.0));
        assert_eq!(before.spawn(), (6800.0, 6800.0 + 3350.0));
        assert_eq!(before.zone_at(before.spawn().0, before.spawn().1), Zone::Rural);
    }

    /// The world is the r8300 disc: the wilds run 2200 past the rural edge,
    /// and the square's corners are out of it.
    #[test]
    fn the_world_is_the_disc() {
        assert_eq!((WORLD_SIZE, CENTER_X, CENTER_Y), (16600.0, 8300.0, 8300.0));
        assert_eq!(Zone::Urban.radii(), (1000.0, 2160.0));
        assert_eq!(Zone::Suburban.radii(), (2160.0, 3600.0));
        assert_eq!(Zone::Rural.radii(), (3600.0, 6100.0));
        assert_eq!(Zone::Wilderness.radii(), (6100.0, 8300.0));
        assert!(!Zone::Wilderness.has_plots());
        assert!(in_world(CENTER_X, CENTER_Y));
        assert!(in_world(CENTER_X, CENTER_Y + WILDERNESS_OUTER_RADIUS));
        assert!(in_world(SPAWN_X, SPAWN_Y));
        assert!(!in_world(CENTER_X + 5900.0, CENTER_Y + 5900.0), "a corner");
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
            (CENTER_X + 150.0, CENTER_Y),
            (CENTER_X + 250.0, CENTER_Y),
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
            (CENTER_X, CENTER_Y + 150.0),
            (CENTER_X, CENTER_Y + 250.0),
        )
        .expect("blocked");
        assert_eq!(block.ring_id, "moat");
        assert!((block.x - CENTER_X).abs() < 1e-9);
        // Inner edge is at radius 190; the mover stops 2 units short.
        assert!((block.y - (CENTER_Y + 188.0)).abs() < 1e-9);
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
