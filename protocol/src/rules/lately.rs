//! What someone has been busy with lately (callings plan D8, K3.1): the line
//! on their card, in words.
//!
//! A person learns what someone does by watching them, or from the paper.
//! The card says it for them: what they've made over the last [`DAYS`] game
//! days (`production_tally`, the Gazette's Busy Hands), and where they've
//! sold (the Exchange's fills, the bazaar's sales). Never a number: "felling
//! trees and sawing planks, and selling at the bazaar".
//!
//! - **Activities, not items.** Each item belongs to one activity
//!   ([`ACTIVITIES`]): logs are "felling trees", every tool head is
//!   "forging tool heads". An item the table doesn't know is "making" it.
//! - **Most first, by value.** An activity's weight is its units at each
//!   item's target price, as the maker board counts them (S3.2), so an axe
//!   outweighs a handful of fiber. Up to [`MOST`] activities show, and none
//!   under [`MIN_SHARE`] of the whole, but the first always does.
//! - **The opt-out** is the server's: someone who keeps their name out of
//!   the papers (`players.gazette_named`) has no `lately` on anyone else's
//!   card. Their own always does.
//!
//! The server builds a [`Lately`] with [`from_work`]; the client draws it
//! with [`words`]. Citizens read the [`Lately::doing`] activities, and
//! [`items_of`] turns them back into goods (K3.2): a citizen knows only
//! what a person could read.

use serde::{Deserialize, Serialize};

/// How many game days back the card looks, today among them.
pub const DAYS: i64 = 3;
/// The most activities a card names.
pub const MOST: usize = 3;
/// The least share of the whole an activity needs to be named, after the first.
pub const MIN_SHARE: f64 = 0.1;

/// Each activity, in words, and the goods it makes. An item is in one at most.
pub const ACTIVITIES: &[(&str, &[&str])] = &[
    ("cutting grass", &["fiber"]),
    ("gathering sticks", &["sticks"]),
    ("digging clay", &["clay"]),
    ("quarrying stone", &["stone"]),
    ("felling trees", &["log"]),
    ("cutting reeds", &["reeds"]),
    ("mining ore", &["iron_ore", "copper_ore", "tin_ore"]),
    ("twisting rope", &["rope"]),
    ("knapping stone", &["knapped_stone"]),
    ("making handles", &["tool_handle"]),
    ("sawing planks", &["plank"]),
    ("burning charcoal", &["charcoal"]),
    ("firing bricks", &["brick"]),
    ("smelting metal", &["iron_ingot", "bronze_ingot"]),
    ("making nails", &["iron_nails"]),
    (
        "forging tool heads",
        &[
            "iron_sickle_blade",
            "iron_axe_head",
            "iron_pickaxe_head",
            "iron_hammer_head",
            "bronze_sickle_blade",
            "bronze_axe_head",
            "bronze_pickaxe_head",
            "bronze_hammer_head",
        ],
    ),
    (
        "making tools",
        &[
            "stone_sickle",
            "stone_axe",
            "stone_pickaxe",
            "stone_hammer",
            "iron_sickle",
            "iron_axe",
            "iron_pickaxe",
            "iron_hammer",
            "bronze_sickle",
            "bronze_axe",
            "bronze_pickaxe",
            "bronze_hammer",
        ],
    ),
    (
        "building workshops",
        &["workbench_kit", "chest_kit", "forge_kit", "kiln_kit", "sawbench_kit", "anvil_kit"],
    ),
];

/// Where someone sells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Venue {
    /// The bazaar's stalls (E3.10)
    Bazaar,
    /// The Exchange: a sell order filled (E3.1)
    Exchange,
}

impl Venue {
    /// "at the bazaar", "on the Exchange"
    pub fn words(self) -> &'static str {
        match self {
            Venue::Bazaar => "at the bazaar",
            Venue::Exchange => "on the Exchange",
        }
    }
}

/// `PlayerCard::lately`: what they've been busy with these last [`DAYS`]
/// game days, in words.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Lately {
    /// What they've made, most first: up to [`MOST`] activities, each an
    /// [`ACTIVITIES`] name ("felling trees") or "making" an item it doesn't know
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doing: Vec<String>,
    /// Where they've sold, the bazaar before the Exchange
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sells: Vec<Venue>,
}

impl Lately {
    /// Nothing to say: they've made nothing and sold nothing.
    pub fn is_empty(&self) -> bool {
        self.doing.is_empty() && self.sells.is_empty()
    }
}

/// The activity an item belongs to, if the table knows it.
pub fn activity_of(item_type: &str) -> Option<&'static str> {
    ACTIVITIES.iter().find(|(_, items)| items.contains(&item_type)).map(|(name, _)| *name)
}

/// The goods an activity makes: none for one the table doesn't know.
pub fn items_of(activity: &str) -> &'static [&'static str] {
    ACTIVITIES.iter().find(|(name, _)| *name == activity).map(|(_, items)| *items).unwrap_or(&[])
}

/// The card's `lately` from what someone made, each `(item_type, item name,
/// value)` with the value the units at their price, and where they sold.
/// `None` when there's nothing to say.
pub fn from_work<'a>(made: impl IntoIterator<Item = (&'a str, &'a str, f64)>, sells: &[Venue]) -> Option<Lately> {
    // Summed by activity, in first-seen order, so ties keep the caller's order
    let mut by: Vec<(String, f64)> = Vec::new();
    for (item, name, value) in made {
        if value.is_nan() || value <= 0.0 {
            continue;
        }
        let activity = match activity_of(item) {
            Some(a) => a.to_string(),
            None => format!("making {}", name.trim().to_lowercase()),
        };
        match by.iter_mut().find(|(a, _)| *a == activity) {
            Some((_, v)) => *v += value,
            None => by.push((activity, value)),
        }
    }
    let total: f64 = by.iter().map(|(_, v)| v).sum();
    by.sort_by(|a, b| b.1.total_cmp(&a.1));
    let doing = by
        .into_iter()
        .enumerate()
        .filter(|(i, (_, v))| *i == 0 || *v >= total * MIN_SHARE)
        .take(MOST)
        .map(|(_, (a, _))| a)
        .collect();
    let mut sells = sells.to_vec();
    sells.sort();
    sells.dedup();
    let lately = Lately { doing, sells };
    (!lately.is_empty()).then_some(lately)
}

/// The card's line, after "Lately": "felling trees and sawing planks, and
/// selling at the bazaar".
pub fn words(lately: &Lately) -> String {
    let doing = list(&lately.doing.iter().map(String::as_str).collect::<Vec<_>>());
    let venues: Vec<&str> = lately.sells.iter().map(|v| v.words()).collect();
    let selling = (!venues.is_empty()).then(|| format!("selling {}", list(&venues)));
    match (doing.is_empty(), selling) {
        (_, None) => doing,
        (true, Some(selling)) => selling,
        // A comma once the list already has an "and" of its own
        (false, Some(selling)) if lately.doing.len() > 1 => format!("{doing}, and {selling}"),
        (false, Some(selling)) => format!("{doing} and {selling}"),
    }
}

/// "a", "a and b", "a, b and c"
fn list(parts: &[&str]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_is_in_one_activity_at_most() {
        let mut seen = std::collections::HashSet::new();
        for (name, items) in ACTIVITIES {
            assert!(!items.is_empty(), "{name}");
            assert_eq!(items_of(name), *items);
            for item in *items {
                assert!(seen.insert(*item), "{item} is in two activities");
                assert_eq!(activity_of(item), Some(*name));
            }
        }
        assert_eq!(items_of("juggling"), &[] as &[&str]);
    }

    #[test]
    fn activities_come_most_first_by_value_and_group_their_goods() {
        let made = [("fiber", "Plant Fiber", 30.0), ("plank", "Plank", 60.0), ("log", "Log", 80.0), ("iron_axe_head", "Iron Axe Head", 0.0)];
        let lately = from_work(made, &[]).unwrap();
        assert_eq!(lately.doing, ["felling trees", "sawing planks", "cutting grass"]);
        // Three ores are one activity
        let made = [("iron_ore", "Iron Ore", 40.0), ("tin_ore", "Tin Ore", 40.0), ("log", "Log", 60.0)];
        assert_eq!(from_work(made, &[]).unwrap().doing, ["mining ore", "felling trees"]);
    }

    #[test]
    fn no_more_than_three_and_nothing_slight_but_the_first() {
        let made = [("log", "", 50.0), ("plank", "", 40.0), ("brick", "", 30.0), ("clay", "", 20.0)];
        assert_eq!(from_work(made, &[]).unwrap().doing.len(), MOST);
        let made = [("log", "", 95.0), ("fiber", "", 5.0)];
        assert_eq!(from_work(made, &[]).unwrap().doing, ["felling trees"]);
        let made = [("fiber", "", 1.0)];
        assert_eq!(from_work(made, &[]).unwrap().doing, ["cutting grass"]);
    }

    #[test]
    fn an_item_the_table_doesnt_know_is_making_it() {
        let made = [("steel_ingot", "Steel Ingot", 10.0)];
        assert_eq!(from_work(made, &[]).unwrap().doing, ["making steel ingot"]);
    }

    #[test]
    fn nothing_made_or_sold_is_nothing_to_say() {
        assert_eq!(from_work([], &[]), None);
        assert_eq!(from_work([("log", "Log", 0.0)], &[]), None);
        let sold = from_work([], &[Venue::Exchange, Venue::Bazaar, Venue::Exchange]).unwrap();
        assert_eq!(sold.sells, [Venue::Bazaar, Venue::Exchange]);
    }

    #[test]
    fn the_line_reads_as_a_sentence() {
        let l = |doing: &[&str], sells: &[Venue]| {
            words(&Lately { doing: doing.iter().map(|s| s.to_string()).collect(), sells: sells.to_vec() })
        };
        assert_eq!(l(&["felling trees"], &[]), "felling trees");
        assert_eq!(l(&["felling trees", "sawing planks"], &[]), "felling trees and sawing planks");
        assert_eq!(
            l(&["felling trees", "sawing planks"], &[Venue::Bazaar]),
            "felling trees and sawing planks, and selling at the bazaar"
        );
        assert_eq!(l(&["felling trees"], &[Venue::Exchange]), "felling trees and selling on the Exchange");
        assert_eq!(
            l(&["felling trees", "sawing planks", "quarrying stone"], &[]),
            "felling trees, sawing planks and quarrying stone"
        );
        assert_eq!(l(&[], &[Venue::Bazaar, Venue::Exchange]), "selling at the bazaar and on the Exchange");
    }
}
