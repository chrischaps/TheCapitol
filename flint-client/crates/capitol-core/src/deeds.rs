//! The Deeds Office (#448, `the-capitol-road-in.md` §4.5): where you choose
//! a home and claim it with the deed the road in earned. Pure, so the rules
//! and the words are tested without a window; the panel is the client's
//! `ui/deeds.rs`.
//!
//! - **The grant** is one free claim on any free lot in the rural, suburban
//!   or urban rings, once an account (plan D6). The server says where it
//!   stands (`TutorialProgress::grant`). It's taxed as any deed is (D4).
//! - **The claim needs no trip to the office** (D8): once the office has
//!   been looked in at, any free home lot's plot panel offers "Claim with
//!   your deed" beside its price, which stays for anyone who'd rather pay
//!   and keep the grant ([`offer`]).
//! - **Free lots** come from `/bureaucracy/free-lots`, one zone at a time,
//!   and sort by the walk to the hall or by room ([`Sort`]).
//!
//! Roads have no names yet, so a lot is placed by its bearing from the
//! tower ([`whereabouts`], #483).

use capitol_protocol::rest::{FreeLot, PlotInfo};
use capitol_protocol::rules::{self, Zone};
use capitol_protocol::tutorial::{Grant, Step, TutorialProgress};

/// The zones the grant claims in, from the city out (D6).
pub const HOME_ZONES: [Zone; 3] = [Zone::Urban, Zone::Suburban, Zone::Rural];

/// What the grant offers on one plot's panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeedOffer {
    /// Nothing: the grant isn't open, or the plot isn't a free home lot.
    None,
    /// The grant is open, but the office hasn't been looked in at yet: the
    /// panel says where your deed waits (D8).
    AtTheOffice,
    /// "Claim with your deed".
    Claim,
}

impl DeedOffer {
    pub fn can_claim(self) -> bool {
        self == DeedOffer::Claim
    }
}

/// What the grant offers on `plot`, for a player on the road in at `tour`
/// (`None` before the server says).
pub fn offer(plot: &PlotInfo, tour: Option<TutorialProgress>) -> DeedOffer {
    let Some(tour) = tour.filter(|t| t.grant.is_open()) else {
        return DeedOffer::None;
    };
    let home = Zone::from_str(&plot.zone_id).is_some_and(|z| HOME_ZONES.contains(&z));
    if plot.owner_id.is_some() || plot.deed_cost.is_none() || !home {
        return DeedOffer::None;
    }
    if tour.done.has(Step::OpenOffice) {
        DeedOffer::Claim
    } else {
        DeedOffer::AtTheOffice
    }
}

/// How the office lists a zone's free lots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// The shortest walk to the hall first, as the server sends them.
    #[default]
    NearestHall,
    /// The most stations first, then the nearer.
    MostRoom,
}

impl Sort {
    pub const ALL: [Sort; 2] = [Sort::NearestHall, Sort::MostRoom];

    pub fn label(self) -> &'static str {
        match self {
            Sort::NearestHall => "Nearest the hall",
            Sort::MostRoom => "Most room",
        }
    }
}

/// `lots` in `sort`'s order. Ties keep the nearer first, then the id, so the
/// list never shuffles between frames.
pub fn sorted(lots: &[FreeLot], sort: Sort) -> Vec<&FreeLot> {
    let mut out: Vec<&FreeLot> = lots.iter().collect();
    let near = |a: &FreeLot, b: &FreeLot| a.walk_secs.total_cmp(&b.walk_secs).then_with(|| a.terrain_id.cmp(&b.terrain_id));
    match sort {
        Sort::NearestHall => out.sort_by(|a, b| near(a, b)),
        Sort::MostRoom => out.sort_by(|a, b| b.fits.cmp(&a.fits).then_with(|| near(a, b))),
    }
    out
}

/// A home in `zone`, as people say it: "A farm", "A house", "A yard".
pub fn home_noun(zone: Zone) -> &'static str {
    match zone {
        Zone::Rural => "farm",
        Zone::Suburban => "house",
        Zone::Urban => "yard",
        _ => "lot",
    }
}

/// The eight-point bearing of the server point (x, y) from the tower (y
/// points south): "south-west".
pub fn bearing(x: f64, y: f64) -> &'static str {
    const POINTS: [&str; 8] = ["east", "north-east", "north", "north-west", "west", "south-west", "south", "south-east"];
    let turn = (-(y - rules::CENTER_Y)).atan2(x - rules::CENTER_X) / std::f64::consts::TAU;
    POINTS[((turn * 8.0).round() as i64).rem_euclid(8) as usize]
}

/// Where a lot in `zone` at the server point (x, y) is, as the Gazette says
/// it: "a farm out to the south-west", "a house on a street to the east", "a
/// yard north of the tower". Roads have no names yet (#483).
pub fn whereabouts(zone: Zone, x: f64, y: f64) -> String {
    let b = bearing(x, y);
    match zone {
        Zone::Rural => format!("a farm out to the {b}"),
        Zone::Suburban => format!("a house on a street to the {b}"),
        Zone::Urban => format!("a yard {b} of the tower"),
        _ => format!("a lot to the {b}"),
    }
}

/// What the grant's slip says it is (D4, D6), with a week's tax on a deed
/// in each of `taxes`' zones: one zone's ("5 a week"), or each in turn.
pub fn grant_terms(taxes: &[(Zone, i64)]) -> String {
    let tax = match taxes {
        [] => "the zone's weekly rate".to_owned(),
        [(_, n)] => format!("{n} a week"),
        many => {
            let each: Vec<String> = many
                .iter()
                .enumerate()
                .map(|(i, (zone, n))| {
                    let noun = home_noun(*zone);
                    if i == 0 { format!("{n} a week on a {noun}") } else { format!("{n} on a {noun}") }
                })
                .collect();
            match each.split_last() {
                Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
                _ => each.concat(),
            }
        }
    };
    format!("One deed, any lot in the rural, suburban or urban rings. Taxed as any deed is: {tax}.")
}

/// Where the grant stands, in a line under its terms.
pub fn grant_state(grant: Grant, office_seen: bool) -> &'static str {
    match grant {
        Grant::Unearned => "Earned at the end of the road in. Walk it, and the office will honour it.",
        Grant::Open if office_seen => "Yours to spend: choose a lot here, or claim from any lot's panel.",
        Grant::Open => "Yours to spend: choose a lot here.",
        Grant::Spent => "Spent. The land is yours, and on the city's books.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::Bounds;
    use capitol_protocol::tutorial::{Chapter, Steps};
    use uuid::Uuid;

    fn plot(zone: &str, owner: Option<Uuid>) -> PlotInfo {
        PlotInfo {
            guild: None,
            id: Uuid::from_u128(9),
            zone_id: zone.into(),
            world_x: 0.0,
            world_y: 0.0,
            bounds: Bounds::new(0.0, 0.0, 10.0, 10.0),
            size_category: "farm".into(),
            plot_type: "standard".into(),
            owner_id: owner,
            owner_name: None,
            claimed_at: None,
            assessed_value: 0,
            tax_owed: 0,
            forecloses_at: None,
            deed_cost: Some(250),
            station_count: 0,
            station_capacity: 25,
            terrain_id: None,
            corners: None,
            indoor_room: None,
            yard_room: None,
            owner_citizen: false,
        }
    }

    fn walked(grant: Grant, office: bool) -> Option<TutorialProgress> {
        let mut done: Steps = [Chapter::Arrival, Chapter::RoadIn, Chapter::Hall, Chapter::People]
            .into_iter()
            .flat_map(Chapter::steps)
            .collect();
        if office {
            done = done.with(Step::OpenOffice);
        }
        Some(TutorialProgress { done, grant, ..Default::default() })
    }

    #[test]
    fn the_deed_is_offered_on_free_home_lots_once_the_office_is_seen() {
        let farm = plot("rural", None);
        assert_eq!(offer(&farm, walked(Grant::Open, true)), DeedOffer::Claim);
        assert!(offer(&farm, walked(Grant::Open, true)).can_claim());
        // Before the office: the panel says where it waits (D8)
        assert_eq!(offer(&farm, walked(Grant::Open, false)), DeedOffer::AtTheOffice);
        // Not earned, or spent: nothing
        assert_eq!(offer(&farm, walked(Grant::Unearned, true)), DeedOffer::None);
        assert_eq!(offer(&farm, walked(Grant::Spent, true)), DeedOffer::None);
        assert_eq!(offer(&farm, None), DeedOffer::None);
        // Taken, or not a home zone (D6)
        assert_eq!(offer(&plot("rural", Some(Uuid::from_u128(2))), walked(Grant::Open, true)), DeedOffer::None);
        assert_eq!(offer(&plot("guild_district", None), walked(Grant::Open, true)), DeedOffer::None);
        let mut wilds = plot("rural", None);
        wilds.deed_cost = None;
        assert_eq!(offer(&wilds, walked(Grant::Open, true)), DeedOffer::None);
    }

    fn lot(id: &str, fits: i32, walk_secs: f64) -> FreeLot {
        FreeLot {
            plot_id: Uuid::from_u128(id.len() as u128),
            terrain_id: id.into(),
            x: 0.0,
            y: 0.0,
            area: 35_000.0,
            fits,
            capacity: 25,
            walk_secs,
        }
    }

    #[test]
    fn free_lots_sort_by_the_walk_or_by_room() {
        let lots = [lot("b", 12, 80.0), lot("a", 15, 95.0), lot("c", 15, 70.0), lot("d", 12, 80.0)];
        let ids = |sort| sorted(&lots, sort).iter().map(|l| l.terrain_id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids(Sort::NearestHall), ["c", "b", "d", "a"]);
        assert_eq!(ids(Sort::MostRoom), ["c", "a", "b", "d"]);
    }

    #[test]
    fn a_lot_is_placed_by_its_bearing_from_the_tower() {
        let (cx, cy) = (rules::CENTER_X, rules::CENTER_Y);
        assert_eq!(bearing(cx, cy + 3000.0), "south", "y points south");
        assert_eq!(bearing(cx - 2000.0, cy + 2000.0), "south-west");
        assert_eq!(bearing(cx + 2000.0, cy - 100.0), "east");
        assert_eq!(whereabouts(Zone::Rural, cx - 3000.0, cy + 3000.0), "a farm out to the south-west");
        assert_eq!(whereabouts(Zone::Urban, cx, cy - 1500.0), "a yard north of the tower");
        assert_eq!(whereabouts(Zone::Suburban, cx + 2500.0, cy), "a house on a street to the east");
        assert_eq!(home_noun(Zone::Suburban), "house");
    }

    #[test]
    fn the_grant_says_what_it_is_and_where_it_stands() {
        assert_eq!(
            grant_terms(&[(Zone::Rural, 5)]),
            "One deed, any lot in the rural, suburban or urban rings. Taxed as any deed is: 5 a week."
        );
        assert!(grant_terms(&[(Zone::Urban, 20), (Zone::Suburban, 10), (Zone::Rural, 5)])
            .ends_with("Taxed as any deed is: 20 a week on a yard, 10 on a house and 5 on a farm."));
        assert!(grant_terms(&[]).ends_with("the zone's weekly rate."));
        assert!(grant_state(Grant::Open, true).contains("any lot's panel"));
        assert!(!grant_state(Grant::Open, false).contains("panel"));
        assert!(grant_state(Grant::Spent, true).starts_with("Spent"));
        assert!(grant_state(Grant::Unearned, false).contains("road in"));
    }
}
