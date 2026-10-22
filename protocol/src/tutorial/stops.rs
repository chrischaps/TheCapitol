//! The road in's stops (#445, `the-capitol-road-in.md` §4.2): one lot in
//! each home zone, by the south arterial, on the walk from the spawn to the
//! hall.
//!
//! A stop is chosen **by rule, not by id**, so a regenerated world or new
//! plot sizes can't strand it: the lot of that zone nearest a fixed point on
//! the south arterial ([`stop_point`]). It prefers a free lot within
//! [`STOP_REACH`] of the point, but an owned one teaches as well, so if every
//! lot that near is taken it's the nearest. The Flint client marks it
//! (`CapitolExtension::lead_mark`), and the citizens walking the road (#450)
//! find the same lot from the same `/terrain`.
//!
//! The arterial runs straight south from the centre (`+y`, the bridges at
//! 90°), through the spawn ([`Frame::spawn`]) and up to the hall. The points
//! are laid in the frame's own rings, so a move of the rings
//! (`rules::ZONE_OUTER_RADII`) moves the stops with them:
//! - **rural:** half way from the suburb's edge to the spawn, so the farm is
//!   the first thing up the road (r3775 on the default world);
//! - **suburban** and **urban:** the middle of their rings (r2880, r1580).
//!
//! At [`PLAYER_SPEED`](crate::rules::PLAYER_SPEED) that's about 4 s from the
//! spawn to the farm, 18 s on to the suburb, 26 s to the city's yards and
//! about 20 s more to the hall: the stops break the walk, they don't
//! lengthen it.

use crate::rules::{Frame, Zone, SPAWN_PAST_SUBURB};
use crate::terrain::TerrainPlot;

/// How far from its point on the road a free lot may be and still be
/// preferred over a nearer one that's taken, in server units. The default
/// world's stops are all well inside it.
pub const STOP_REACH: f64 = 150.0;

/// The zones with a stop, in the order the road meets them from the spawn.
pub const STOP_ZONES: [Zone; 3] = [Zone::Rural, Zone::Suburban, Zone::Urban];

/// How far out along the south arterial `zone`'s stop is, or `None` for a
/// zone with none.
pub fn stop_radius(frame: &Frame, zone: Zone) -> Option<f64> {
    match zone {
        Zone::Rural => Some(frame.suburb_edge() + SPAWN_PAST_SUBURB / 2.0),
        Zone::Suburban | Zone::Urban => {
            let (inner, outer) = frame.radii_of(zone);
            Some((inner + outer) / 2.0)
        }
        _ => None,
    }
}

/// The point on the south arterial `zone`'s stop is chosen from, server
/// units.
pub fn stop_point(frame: &Frame, zone: Zone) -> Option<(f64, f64)> {
    stop_radius(frame, zone).map(|r| (frame.center.0, frame.center.1 + r))
}

/// A stop: its lot, and whether someone owns it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop<'a> {
    pub lot: &'a TerrainPlot,
    /// The lot is someone's. Its deed reads the same, and the card says so.
    pub taken: bool,
}

/// `zone`'s stop among `lots` (a `/terrain`'s `terrain_plots`). `taken`
/// says whether a lot, by its id, is owned. `None` for a zone with no stop,
/// or a world with no lot in it.
pub fn stop<'a>(
    lots: impl IntoIterator<Item = &'a TerrainPlot>,
    frame: &Frame,
    zone: Zone,
    taken: impl Fn(&str) -> bool,
) -> Option<Stop<'a>> {
    let at = stop_point(frame, zone)?;
    // Nearest first, ties by id, so every caller picks the same lot.
    let order = |a: &(f64, &TerrainPlot), b: &(f64, &TerrainPlot)| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id));
    let zoned = lots
        .into_iter()
        .filter(|l| l.zone == zone.as_str())
        .map(|l| (distance_to_lot(at, &l.corners), l));
    let mut nearest: Option<(f64, &TerrainPlot)> = None;
    let mut free: Option<(f64, &TerrainPlot)> = None;
    for c in zoned {
        if nearest.is_none_or(|n| order(&c, &n).is_lt()) {
            nearest = Some(c);
        }
        if c.0 <= STOP_REACH && free.is_none_or(|f| order(&c, &f).is_lt()) && !taken(&c.1.id) {
            free = Some(c);
        }
    }
    let (_, lot) = free.or(nearest)?;
    Some(Stop { lot, taken: taken(&lot.id) })
}

/// How far `p` is from the quad `corners`: 0 inside it.
pub fn distance_to_lot(p: (f64, f64), corners: &[(f64, f64); 4]) -> f64 {
    if inside(p, corners) {
        return 0.0;
    }
    (0..4)
        .map(|i| distance_to_segment(p, corners[i], corners[(i + 1) % 4]))
        .fold(f64::INFINITY, f64::min)
}

fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    (qx * qx + qy * qy).sqrt()
}

/// Even-odd: whichever way the corners wind.
fn inside(p: (f64, f64), c: &[(f64, f64); 4]) -> bool {
    let mut odd = false;
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            odd = !odd;
        }
    }
    odd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 40-square lot whose near edge is `gap` east of the south arterial,
    /// at radius `r`.
    fn lot(id: &str, zone: Zone, r: f64, gap: f64) -> TerrainPlot {
        let (x, y) = (Frame::TABLE.center.0 + gap, Frame::TABLE.center.1 + r - 20.0);
        let corners = [(x, y), (x + 40.0, y), (x + 40.0, y + 40.0), (x, y + 40.0)];
        TerrainPlot {
            id: id.to_owned(),
            corners,
            street_facing_edge: (0, 3),
            area: 1600.0,
            adjacent_street_id: "arterial_2".to_owned(),
            zone: zone.as_str().to_owned(),
        }
    }

    #[test]
    fn the_stops_run_up_the_road_from_the_spawn_to_the_hall() {
        let f = Frame::TABLE;
        let r: Vec<f64> = STOP_ZONES.iter().map(|z| stop_radius(&f, *z).unwrap()).collect();
        let spawn = f.spawn().1 - f.center.1;
        assert!(r[0] < spawn && r[0] > r[1] && r[1] > r[2], "{r:?} from the spawn at {spawn}");
        for (zone, r) in STOP_ZONES.iter().zip(&r) {
            let (x, y) = stop_point(&f, *zone).unwrap();
            assert_eq!(x, f.center.0);
            assert_eq!(f.zone_at(x, y), *zone, "{zone}'s point at r{r} is in its own ring");
        }
        assert_eq!(r, [3775.0, 2880.0, 1580.0], "the default world's");
        assert_eq!(stop_radius(&f, Zone::GuildDistrict), None);
        assert_eq!(stop_radius(&f, Zone::Wilderness), None);
    }

    #[test]
    fn a_stop_is_the_nearest_lot_of_its_zone() {
        let f = Frame::TABLE;
        let lots = [
            lot("near_suburb", Zone::Suburban, 3775.0, 10.0),
            lot("far", Zone::Rural, 3775.0, 90.0),
            lot("near", Zone::Rural, 3775.0, 40.0),
            lot("up_the_road", Zone::Rural, 3500.0, 5.0),
        ];
        let s = stop(&lots, &f, Zone::Rural, |_| false).unwrap();
        assert_eq!((s.lot.id.as_str(), s.taken), ("near", false));
        assert!(stop(&lots, &f, Zone::Urban, |_| false).is_none(), "no urban lot");
        assert!(stop(&lots, &f, Zone::GuildDistrict, |_| false).is_none(), "no stop there");
    }

    #[test]
    fn it_prefers_a_free_lot_near_the_road_and_takes_a_taken_one_if_not() {
        let f = Frame::TABLE;
        let lots = [
            lot("near", Zone::Urban, 1580.0, 30.0),
            lot("next", Zone::Urban, 1580.0, 100.0),
            lot("too_far", Zone::Urban, 1580.0, 400.0),
        ];
        let s = stop(&lots, &f, Zone::Urban, |id| id == "near").unwrap();
        assert_eq!((s.lot.id.as_str(), s.taken), ("next", false), "the free one within reach");
        let s = stop(&lots, &f, Zone::Urban, |id| id != "too_far").unwrap();
        assert_eq!((s.lot.id.as_str(), s.taken), ("near", true), "taken, but nearer than a free one far off");
    }

    #[test]
    fn distance_to_a_lot_is_to_its_nearest_edge() {
        let c = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert_eq!(distance_to_lot((5.0, 5.0), &c), 0.0);
        assert_eq!(distance_to_lot((13.0, 5.0), &c), 3.0);
        assert_eq!(distance_to_lot((13.0, 14.0), &c), 5.0);
        let mut back = c;
        back.reverse();
        assert_eq!(distance_to_lot((5.0, 5.0), &back), 0.0, "either winding");
    }
}
