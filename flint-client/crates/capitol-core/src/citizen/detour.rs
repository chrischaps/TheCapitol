//! Detours (Callings K1.5, plan D2): the unrelated things people do in work
//! hours. You go to sell ore and stop at the bazaar; you see a lake on the
//! map and walk to it; you try a recipe you've never made.
//!
//! A detour is a [`Choice::Detour`] on the stint menu and the free-time
//! menu, about a quarter hour ([`DETOUR_SECS`]) of one of five kinds
//! ([`DetourKind`]):
//! - **try:** craft a recipe it has never made, with what it holds
//!   ([`untried`]);
//! - **see:** walk out to a wild lake, a ford or a ridge top it hasn't been
//!   to, at the end of the trail to it, or to a park ([`Sight`]);
//! - **browse:** walk the bazaar lane and look in at a stall;
//! - **read:** the Gazette at the hall in the morning, the Wanted page on
//!   the hall's board after noon;
//! - **call:** at a neighbour's lot, or the lot of someone it has traded
//!   with.
//!
//! This is what's on offer ([`menu`]), pure. The rules weigh it by the
//! calling's `wander` ([`rules`](super::decide::rules)), and the run walks
//! it (`sim/run/detour.rs`). It's never on a menu mid-claim or mid-shift,
//! and its time is filed under [`Activity::Detour`](crate::sim::activity::Activity::Detour).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::decide::{Choice, Temperament};
use super::idle::{PlaceKind, Places};
use crate::catalog::Catalog;
use crate::sim::plan::Holdings;

type Point = (f64, f64);

/// How long a detour takes, in game seconds: a quarter hour, the walk
/// there and the time there together.
pub const DETOUR_SECS: f64 = 900.0;
/// How far a citizen walks to see a sight in the wilds: a game minute's
/// walk on the flat, so the farms' and the suburb's outer streets reach the
/// foothills' lakes and tops, and the city's middle doesn't.
pub const SIGHT_REACH: f64 = 3000.0;
/// How far it walks to a park it hasn't sat in: as far as in its free time.
pub const PARK_REACH: f64 = 900.0;
/// How far from its home a neighbour's lot is: a few lots down the lane.
pub const NEIGHBOUR_REACH: f64 = 600.0;
/// How far from where it stands it walks to call on someone it has traded
/// with.
pub const PARTNER_REACH: f64 = 2500.0;

/// What a detour is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetourKind {
    /// Craft a recipe it has never made.
    Try,
    /// Walk out to a lake, a ford, a ridge top or a park.
    See,
    /// Walk the bazaar lane, and look in at a stall.
    Browse,
    /// Read the Gazette, or the Wanted page.
    Read,
    /// Call on a neighbour, or someone it has traded with.
    Call,
}

impl DetourKind {
    pub const ALL: [DetourKind; 5] = [DetourKind::Try, DetourKind::See, DetourKind::Browse, DetourKind::Read, DetourKind::Call];

    /// As the wire and the log name it: `try`, `see`.
    pub fn slug(self) -> &'static str {
        match self {
            DetourKind::Try => "try",
            DetourKind::See => "see",
            DetourKind::Browse => "browse",
            DetourKind::Read => "read",
            DetourKind::Call => "call",
        }
    }

    /// How much a temperament takes to it, about 0.2 to 0.8: the bold try
    /// things (the thrifty less, since a try spends what it holds), the
    /// solitary and the bold walk out to the wilds, the sociable browse
    /// the lane and call on people, and the patient and the thrifty read
    /// the paper's prices.
    pub fn taste(self, t: &Temperament) -> f64 {
        let (thrift, risk, sociability, patience) = (t.thrift as f64, t.risk as f64, t.sociability as f64, t.patience as f64);
        match self {
            DetourKind::Try => 0.35 + 0.4 * risk - 0.15 * thrift,
            DetourKind::See => 0.25 + 0.3 * (1.0 - sociability) + 0.2 * risk,
            DetourKind::Browse => 0.25 + 0.5 * sociability,
            DetourKind::Read => 0.25 + 0.25 * patience + 0.2 * thrift,
            DetourKind::Call => 0.15 + 0.6 * sociability,
        }
    }
}

/// A detour as the run takes it: what [`Choice::Detour`] holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Detour {
    pub kind: DetourKind,
    /// What it goes to: a recipe's id (try), a sight's or a park's id
    /// (see), `lane` (browse), `gazette` or `wanted` (read), a lot's id
    /// (call).
    pub to: String,
    /// What taking it is, in a few words: `see a lake in the hills to the
    /// north`.
    pub does: String,
}

impl Detour {
    pub fn choice(&self) -> Choice {
        Choice::Detour {
            detour: self.kind,
            to: self.to.clone(),
            does: self.does.clone(),
        }
    }

    /// The detour a menu entry is, if it's one.
    pub fn of(choice: &Choice) -> Option<Detour> {
        match choice {
            Choice::Detour { detour, to, does } => Some(Detour {
                kind: *detour,
                to: to.clone(),
                does: does.clone(),
            }),
            _ => None,
        }
    }

    /// What the health line says while it's there.
    pub fn there(&self) -> String {
        match self.kind {
            DetourKind::Try => format!("trying something new: {}", self.does),
            DetourKind::See if self.does.starts_with("see ") => self.does.replacen("see ", "seeing ", 1),
            DetourKind::See => "sitting a while in a park".into(),
            DetourKind::Browse => "browsing the bazaar lane".into(),
            DetourKind::Read if self.to == WANTED => "reading the Wanted page on the hall's board".into(),
            DetourKind::Read => "reading the Gazette on the hall steps".into(),
            DetourKind::Call => self.does.replacen("call on ", "calling on ", 1),
        }
    }
}

/// What [`Detour::to`] says for the Gazette and the Wanted page.
pub const GAZETTE: &str = "gazette";
pub const WANTED: &str = "wanted";
/// And for the bazaar lane.
pub const LANE: &str = "lane";

/// What a citizen could wander off to now, from what its run knows.
#[derive(Debug, Clone)]
pub struct Open<'a> {
    pub places: &'a Places,
    /// Where it stands
    pub at: Point,
    /// The day's share, 0.5 noon
    pub time: Option<f64>,
    /// The sights and parks it has been to on a detour, by id
    pub seen: &'a BTreeSet<String>,
    /// A recipe it has never made and can make now ([`untried`]): its id
    /// and its name
    pub recipe: Option<(String, String)>,
    /// A lot to call at: its id, and whether it's someone it has traded
    /// with's rather than a neighbour's
    pub call: Option<(String, bool)>,
}

/// The detours open to a citizen now, one of each kind it can take, as
/// menu entries: an untried recipe; the nearest sight within
/// [`SIGHT_REACH`], or park within [`PARK_REACH`], it hasn't been to; the
/// bazaar lane; the Gazette (before noon) or the Wanted page; a lot to call
/// at.
pub fn menu(open: &Open) -> Vec<Choice> {
    let mut out = Vec::new();
    if let Some((id, name)) = &open.recipe {
        out.push(Detour {
            kind: DetourKind::Try,
            to: id.clone(),
            does: format!("try making {}", name.to_lowercase()),
        });
    }
    if let Some(place) = sight(open) {
        let does = match &place.kind {
            PlaceKind::Sight { sight, .. } => format!("see {}", sight.words(open.places.bearing(place.at))),
            _ => "sit a while in a park nearby".into(),
        };
        out.push(Detour {
            kind: DetourKind::See,
            to: place.id().unwrap_or_default().to_string(),
            does,
        });
    }
    if open.places.lane.is_some() {
        out.push(Detour {
            kind: DetourKind::Browse,
            to: LANE.into(),
            does: "browse the bazaar lane".into(),
        });
    }
    let wanted = open.time.is_some_and(|t| t.rem_euclid(1.0) >= 0.5);
    out.push(Detour {
        kind: DetourKind::Read,
        to: if wanted { WANTED } else { GAZETTE }.into(),
        does: if wanted { "read the Wanted page on the hall's board" } else { "read the Gazette at the hall" }.into(),
    });
    if let Some((lot, partner)) = &open.call {
        out.push(Detour {
            kind: DetourKind::Call,
            to: lot.clone(),
            does: if *partner { "call on a trading partner" } else { "call on a neighbour" }.into(),
        });
    }
    out.iter().map(Detour::choice).collect()
}

/// The nearest sight within [`SIGHT_REACH`], or park within
/// [`PARK_REACH`], it hasn't been to.
fn sight<'a>(open: &Open<'a>) -> Option<&'a super::idle::Place> {
    let places = open.places;
    let reach = |p: &super::idle::Place| match p.kind {
        PlaceKind::Sight { .. } => SIGHT_REACH,
        _ => PARK_REACH,
    };
    places
        .sights
        .iter()
        .chain(&places.parks)
        .filter(|p| p.id().is_some_and(|id| !open.seen.contains(id)))
        .filter(|p| distance(p.at, open.at) <= reach(p))
        .min_by(|a, b| distance(a.at, open.at).total_cmp(&distance(b.at, open.at)))
}

/// A recipe it has never made (`made`, by id) and can make now with what it
/// holds: its inputs in hand, its tool if it takes one, and its station
/// one of its own, or none. The least of them, by the units it takes, then
/// by id; its id and its name.
pub fn untried(catalog: &Catalog, holdings: &Holdings, made: &BTreeSet<String>) -> Option<(String, String)> {
    catalog
        .recipes()
        .iter()
        .filter(|r| !made.contains(&r.id))
        .filter(|r| r.required_station_type.as_ref().is_none_or(|s| holdings.stations.contains(s)))
        .filter(|r| r.tool_type.as_ref().is_none_or(|t| holdings.tools.contains(t)))
        .filter(|r| !r.inputs.is_empty() && r.inputs.iter().all(|i| holdings.count(&i.item_type) >= i.quantity))
        .min_by(|a, b| {
            let units = |r: &capitol_protocol::rest::RecipeInfo| r.inputs.iter().map(|i| i.quantity).sum::<i32>();
            units(a).cmp(&units(b)).then_with(|| a.id.cmp(&b.id))
        })
        .map(|r| (r.id.clone(), r.name.clone()))
}

fn distance(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::citizen::idle::Sight;
    use crate::paths::WalkGraph;
    use crate::terrain::read_terrain_file;

    fn world() -> WalkGraph {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        WalkGraph::new(&read_terrain_file(&path).unwrap())
    }

    fn open<'a>(places: &'a Places, at: Point, seen: &'a BTreeSet<String>) -> Open<'a> {
        Open {
            places,
            at,
            time: Some(0.3),
            seen,
            recipe: None,
            call: None,
        }
    }

    fn kinds(menu: &[Choice]) -> Vec<DetourKind> {
        menu.iter().filter_map(Detour::of).map(|d| d.kind).collect()
    }

    #[test]
    fn the_wilds_have_lakes_fords_and_tops_to_see() {
        let walks = world();
        let places = walks.places();
        let count = |s: Sight| places.sights.iter().filter(|p| matches!(p.kind, PlaceKind::Sight { sight, .. } if sight == s)).count();
        assert!(count(Sight::Lake) >= 20 && count(Sight::Ford) >= 5 && count(Sight::Top) >= 4, "{} lakes, {} fords, {} tops", count(Sight::Lake), count(Sight::Ford), count(Sight::Top));
        for p in &places.sights {
            // Each is at its trail's end, in the wilds, and named again
            let r = (p.at.0 - places.centre.0).hypot(p.at.1 - places.centre.1);
            assert!(r > capitol_protocol::rules::RURAL_OUTER_RADIUS, "{:?} at r{r:.0}", p.kind);
            assert_eq!(places.named(p.id().unwrap()), Some(p));
            let (spot, look) = p.spot(0.3, 0.7);
            assert_eq!(spot, p.at);
            assert!((look.0 - spot.0).hypot(look.1 - spot.1) > 1.0, "{:?} looks at itself", p.kind);
        }
        // Every lot has a middle to call at
        assert!(places.lots.len() > 2000, "{} lots", places.lots.len());
    }

    #[test]
    fn a_compass_point_is_which_way_from_the_middle() {
        let walks = world();
        let places = walks.places();
        let (cx, cy) = places.centre;
        assert_eq!(places.bearing((cx, cy - 5000.0)), "north");
        assert_eq!(places.bearing((cx, cy + 5000.0)), "south");
        assert_eq!(places.bearing((cx + 4000.0, cy - 4000.0)), "north-east");
        assert_eq!(places.bearing((cx - 5000.0, cy + 100.0)), "west");
    }

    #[test]
    fn a_detour_goes_to_what_is_near_and_unseen() {
        let walks = world();
        let places = walks.places();
        let mut seen = BTreeSet::new();
        // On a farm near the hills: a sight, the lane, the paper
        let (cx, cy) = places.centre;
        let farm = places.sights.iter().map(|p| p.at).find(|&(x, y)| (x - cx).hypot(y - cy) < 7200.0).expect("a sight in the foothills");
        let near = (farm.0 + (cx - farm.0) * 0.2, farm.1 + (cy - farm.1) * 0.2);
        let menu = menu(&open(places, near, &seen));
        assert_eq!(kinds(&menu), [DetourKind::See, DetourKind::Browse, DetourKind::Read], "{menu:?}");
        let see = Detour::of(&menu[0]).unwrap();
        assert!(see.does.starts_with("see a ") && see.does.contains(" to the "), "{}", see.does);
        // Once seen, the next nearest
        seen.insert(see.to.clone());
        let again = Detour::of(&super::menu(&open(places, near, &seen))[0]).unwrap();
        assert!(again.kind != DetourKind::See || again.to != see.to);
        // In the middle of the city, nothing of the wilds is in reach
        let hall = places.hall_steps.as_ref().unwrap().at;
        assert!(!kinds(&super::menu(&open(places, hall, &seen))).contains(&DetourKind::See));
        // The Gazette in the morning, the Wanted page after noon
        let afternoon = super::menu(&Open { time: Some(0.6), ..open(places, hall, &seen) });
        assert!(afternoon.iter().filter_map(Detour::of).any(|d| d.kind == DetourKind::Read && d.to == WANTED));
        // A recipe and a call, when the run has them
        let all = super::menu(&Open {
            recipe: Some(("rope_basic".into(), "Rope".into())),
            call: Some(("sub_1_0_3_L_2".into(), false)),
            ..open(places, near, &seen)
        });
        assert_eq!(kinds(&all), DetourKind::ALL);
        let words: Vec<String> = all.iter().filter_map(Detour::of).map(|d| d.there()).collect();
        assert!(words.iter().all(|w| !w.is_empty()), "{words:?}");
        // As the log keeps it
        let json = serde_json::to_string(&all[0]).unwrap();
        assert_eq!(json, r#"{"kind":"detour","detour":"try","to":"rope_basic","does":"try making rope"}"#);
        assert_eq!(serde_json::from_str::<Choice>(&json).unwrap(), all[0]);
    }

    #[test]
    fn an_untried_recipe_is_the_least_it_can_make_now_and_never_one_it_made() {
        let catalog = Catalog::fallback();
        let mut h = Holdings::default();
        let mut made = BTreeSet::new();
        assert_eq!(untried(&catalog, &h, &made), None, "nothing in hand, nothing to try");
        h.items.insert("fiber".into(), 50);
        h.items.insert("sticks".into(), 20);
        h.items.insert("stone".into(), 20);
        let (first, _) = untried(&catalog, &h, &made).expect("something from fiber, sticks and stone");
        let recipe = catalog.recipes().iter().find(|r| r.id == first).unwrap();
        assert!(recipe.required_station_type.is_none(), "{first} wants a station it hasn't");
        assert!(recipe.inputs.iter().all(|i| h.count(&i.item_type) >= i.quantity));
        made.insert(first.clone());
        let next = untried(&catalog, &h, &made).map(|(id, _)| id);
        assert_ne!(next.as_deref(), Some(first.as_str()));
        // A station of its own opens that station's recipes
        h.stations.insert("workbench".into());
        let all: BTreeSet<String> = catalog.recipes().iter().map(|r| r.id.clone()).collect();
        assert_eq!(untried(&catalog, &h, &all), None, "everything made");
    }
}
