//! The zone card's words (#444, the road in plan D9): what living in each
//! home zone is like, said plainly from `/bureaucracy/zone-facts`. Pure, so
//! the sentences are tested without a window.
//!
//! The card tells the truth. Where a zone's lots fit fewer stations than a
//! whole workshop, or fewer than the deed's capacity says, it says so.

use capitol_protocol::rest::ZoneFacts;

/// The stations a whole workshop takes: a bench, a forge, a kiln and their
/// like, as Plots v2 sized the suburb's lots for (`the-capitol-plots-v2.md`).
pub const WORKSHOP: i32 = 6;

/// How many resources the card names.
pub const GROWS_SHOWN: usize = 3;

/// A walk of `secs` seconds, to the nearest 5 under a minute: "35 s",
/// "1 min 10 s", "2 min".
pub fn walk(secs: f64) -> String {
    let secs = ((secs / 5.0).round() * 5.0).max(5.0) as i64;
    match (secs / 60, secs % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// The walk to the hall, nearest and typical: "17 s to 35 s".
pub fn walk_range(f: &ZoneFacts) -> String {
    let (near, typical) = (walk(f.walk_nearest_secs), walk(f.walk_median_secs));
    if near == typical {
        near
    } else {
        format!("{near} to {typical}")
    }
}

/// The stations a typical lot fits, and more with floors, which any home
/// lot may build (Plots v2 §5.3): "4, more with floors", or "3 to 8, more
/// with floors" where some fit fewer.
pub fn stations(f: &ZoneFacts) -> String {
    let fits = if f.stations_least < f.stations_median {
        format!("{} to {}", f.stations_least, f.stations_median)
    } else {
        f.stations_median.to_string()
    };
    format!("{fits}, more with floors")
}

/// What the card owns up to about the room, if anything: fewer than a whole
/// workshop, or fewer than the deed's capacity reads. Since the building
/// came with the deed (V3.1, #473) the capacity is what a lot fits, so the
/// second is only for a server from before it.
pub fn room_notes(f: &ZoneFacts) -> Vec<String> {
    let mut notes = Vec::new();
    if f.stations_median < WORKSHOP {
        notes.push(format!(
            "Room for {} stations, not a whole workshop, until you build a floor.",
            f.stations_median
        ));
    }
    if f.stations_median < f.station_capacity {
        notes.push(format!(
            "The deed allows {}; a typical lot fits {}.",
            f.station_capacity, f.stations_median
        ));
    }
    notes
}

/// What grows there, densest first: "Tree, Fallen Branches and Rock Outcrop".
pub fn grows(f: &ZoneFacts) -> String {
    let names: Vec<&str> = f.grows.iter().take(GROWS_SHOWN).map(|g| g.name.as_str()).collect();
    match names.as_slice() {
        [] => "Nothing wild".to_owned(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The free lots: "1015 of 1016", or "None of 900" once they're all taken.
pub fn free(f: &ZoneFacts) -> String {
    match f.lots_free {
        0 => format!("None of {}", f.lots_total),
        n if n == f.lots_total => format!("All {n}"),
        n => format!("{n} of {}", f.lots_total),
    }
}

/// The typical lot's size, rounded: "8,600 sq", "35,000 sq".
pub fn area(f: &ZoneFacts) -> String {
    let rounded = ((f.typical_area / 100.0).round() * 100.0) as i64;
    let digits = rounded.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out} sq")
}

/// The facts for `zone`, if the server sent them.
pub fn find<'a>(zones: &'a [ZoneFacts], zone: &str) -> Option<&'a ZoneFacts> {
    zones.iter().find(|f| f.zone_id == zone)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use capitol_protocol::rest::ZoneGrowth;

    /// A zone as the default world has it: urban, a box of 85 × 100.
    pub(crate) fn urban() -> ZoneFacts {
        ZoneFacts {
            zone_id: "urban".into(),
            zone_name: "Urban".into(),
            style: "A workshop yard a step from the market.".into(),
            deed_cost: 1000,
            tax_per_week: 20,
            stations_median: 4,
            stations_least: 4,
            station_capacity: 4,
            lots_free: 1015,
            lots_total: 1016,
            walk_nearest_secs: 17.2,
            walk_median_secs: 34.0,
            grows: vec![ZoneGrowth { resource_id: "clay_bank".into(), name: "Clay Bank".into(), density: 6.0 }],
            typical_lot: [(-42.5, -50.0), (42.5, -50.0), (42.5, 50.0), (-42.5, 50.0)],
            typical_area: 8562.0,
        }
    }

    #[test]
    fn walks_read_in_seconds_then_minutes() {
        assert_eq!(walk(17.2), "15 s");
        assert_eq!(walk(34.0), "35 s");
        assert_eq!(walk(59.0), "1 min");
        assert_eq!(walk(68.0), "1 min 10 s");
        assert_eq!(walk(121.0), "2 min");
        assert_eq!(walk(0.4), "5 s", "never no walk at all");
        assert_eq!(walk_range(&urban()), "15 s to 35 s");
    }

    #[test]
    fn the_card_owns_up_to_a_lot_short_of_a_workshop_and_of_its_capacity() {
        let u = urban();
        assert_eq!(stations(&u), "4, more with floors");
        assert_eq!(room_notes(&u), ["Room for 4 stations, not a whole workshop, until you build a floor."]);
        // A server from before the building, whose capacity said more
        let old = ZoneFacts { station_capacity: 16, ..urban() };
        assert_eq!(room_notes(&old)[1], "The deed allows 16; a typical lot fits 4.");
        let farm = ZoneFacts { stations_median: 14, stations_least: 12, station_capacity: 14, ..u };
        assert_eq!(stations(&farm), "12 to 14, more with floors");
        assert!(room_notes(&farm).is_empty(), "nothing to own up to");
    }

    #[test]
    fn grows_names_the_densest_three() {
        let mut f = urban();
        assert_eq!(grows(&f), "Clay Bank");
        f.grows = ["Tree", "Fallen Branches", "Rock Outcrop", "Wild Grass"]
            .iter()
            .map(|n| ZoneGrowth { resource_id: n.to_lowercase(), name: (*n).into(), density: 1.0 })
            .collect();
        assert_eq!(grows(&f), "Tree, Fallen Branches and Rock Outcrop");
        f.grows.clear();
        assert_eq!(grows(&f), "Nothing wild");
    }

    #[test]
    fn free_lots_and_areas_read_plainly() {
        let mut f = urban();
        assert_eq!(free(&f), "1015 of 1016");
        f.lots_free = 0;
        assert_eq!(free(&f), "None of 1016");
        f.lots_free = 1016;
        assert_eq!(free(&f), "All 1016");
        assert_eq!(area(&f), "8,600 sq");
        f.typical_area = 34_911.0;
        assert_eq!(area(&f), "34,900 sq");
        f.typical_area = 640.0;
        assert_eq!(area(&f), "600 sq");
    }
}
