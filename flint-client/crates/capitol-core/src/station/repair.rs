//! A station's condition and its repair (V3.2, V3.5, #477): what the
//! station panel says of the weather and what mending it takes.
//!
//! - **The bill** is the server's (`engine::weather::repair_bill`): a share
//!   of each input of the recipe that makes the station's kit, the first such
//!   recipe by id, by `rules::weather::repair_bill`. The client has only the
//!   condition shown (whole, rounded up), so at a whole number's edge the
//!   bill can come out one under the server's; the server's refusal says
//!   so, and the panel shows it.
//! - **Who may repair:** anyone the station admits, in reach. The panel
//!   offers it to everyone who has it open: the server judges the rest.

use capitol_protocol::rest::{RecipeInfo, StationInfo};
use capitol_protocol::rules::weather;

use crate::catalog::Catalog;
use crate::plot::Need;
use crate::state::Container;

/// What a repair of a `station_type` at `condition` takes, by item type, from
/// `recipes`: nothing at full, or for a kit no recipe makes.
pub fn bill(catalog: &Catalog, recipes: &[RecipeInfo], station_type: &str, condition: i32) -> Vec<(String, i32)> {
    let Some(kit) = catalog.kit_for_station(station_type) else {
        return Vec::new();
    };
    let recipe = recipes
        .iter()
        .filter(|r| r.outputs.iter().any(|o| o.item_type == kit))
        .min_by(|a, b| a.id.cmp(&b.id));
    let Some(recipe) = recipe else {
        return Vec::new();
    };
    recipe
        .inputs
        .iter()
        .map(|i| (i.item_type.clone(), weather::repair_bill(i.quantity, condition as f64)))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// [`bill`] against your `pack`.
pub fn needs(catalog: &Catalog, recipes: &[RecipeInfo], station: &StationInfo, pack: Option<&Container>) -> Vec<Need> {
    bill(catalog, recipes, &station.station_type, station.condition)
        .into_iter()
        .map(|(t, n)| Need::of(&t, n, pack, catalog))
        .collect()
}

/// Where a station stands, as far as the weather goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelter {
    /// Inside its lot's building, on this storey: the weather never reaches it
    Indoors { floor: i32 },
    /// Out on a plot: it wears slowly, and never falls
    OnAPlot,
    /// Outdoors, on ground the client can't place: someone else's
    Outdoors,
    /// Out on open ground: it wears fast, and falls down at 0
    OpenGround,
}

impl Shelter {
    /// Where `station` stands: `on_a_plot` is whether a plot covers it, or
    /// `None` where that isn't known.
    pub fn of(station: &StationInfo, on_a_plot: Option<bool>) -> Shelter {
        match (station.indoors, on_a_plot) {
            (true, _) => Shelter::Indoors { floor: station.floor },
            (false, Some(true)) => Shelter::OnAPlot,
            (false, Some(false)) => Shelter::OpenGround,
            (false, None) => Shelter::Outdoors,
        }
    }

    /// The station panel's line: where it stands and what the weather does.
    pub fn words(self) -> String {
        match self {
            Shelter::Indoors { floor: 0 } => "Indoors, out of the weather".into(),
            Shelter::Indoors { floor } => {
                format!("Upstairs on {}, out of the weather", capitol_protocol::rules::floors::storey_name(floor))
            }
            Shelter::OnAPlot => "Outdoors: the weather wears it slowly".into(),
            Shelter::Outdoors => "Outdoors: the weather wears it".into(),
            Shelter::OpenGround => "Out on open ground: the weather wears it fast, and at 0 it falls down".into(),
        }
    }
}

/// How a station's condition reads: what it does at this condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// In good repair, or worn but working
    Sound,
    /// Under 25: it doesn't work on its own, only by hand
    NeedsRepair,
    /// At 0: out of use until it's repaired
    WornOut,
}

impl State {
    pub fn of(condition: i32) -> State {
        if condition <= 0 {
            State::WornOut
        } else if condition < weather::NEEDS_REPAIR {
            State::NeedsRepair
        } else {
            State::Sound
        }
    }

    /// What it means, when it's anything but sound.
    pub fn words(self) -> Option<&'static str> {
        match self {
            State::Sound => None,
            State::NeedsRepair => Some("Needs repair: it won't work on its own until it's mended"),
            State::WornOut => Some("Worn out: nothing can be made here until it's repaired"),
        }
    }
}

/// The toast as a station of yours wears past 25 or to 0: "Your workbench
/// needs repair", or "is worn out". `name` is its type's name.
pub fn worn_words(name: &str, condition: i32) -> (String, String) {
    let name = name.to_lowercase();
    if condition <= 0 {
        (
            format!("Your {name} is worn out"),
            "The weather has worn it to nothing. Open it and repair it to use it again".into(),
        )
    } else {
        (
            format!("Your {name} needs repair"),
            format!("The weather has worn it to {condition}%. It won't work on its own until it's repaired"),
        )
    }
}

/// The toast as your repair is done: "Workbench repaired", "As good as
/// new, for 3 Fiber". `used` is what it took, already counted in words.
pub fn repaired_words(name: &str, used: &str) -> (String, String) {
    let body = if used.is_empty() { "As good as new".to_string() } else { format!("As good as new, for {used}") };
    (format!("{name} repaired"), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipes(catalog: &Catalog) -> Vec<RecipeInfo> {
        catalog.recipes().to_vec()
    }

    /// The server's bill (V3.2): a workbench from 0 is 3 fiber, an anvil 2
    /// ingots and a plank
    #[test]
    fn a_repair_takes_a_quarter_of_the_kit_from_nothing() {
        let catalog = Catalog::fallback();
        let recipes = recipes(&catalog);
        let kit_inputs = |station: &str| {
            let kit = catalog.kit_for_station(station).unwrap();
            let r = recipes.iter().filter(|r| r.outputs.iter().any(|o| o.item_type == kit)).min_by(|a, b| a.id.cmp(&b.id)).unwrap();
            r.inputs.iter().map(|i| (i.item_type.clone(), i.quantity)).collect::<Vec<_>>()
        };
        let worn = bill(&catalog, &recipes, "workbench", 0);
        let expect: Vec<_> = kit_inputs("workbench").into_iter().map(|(t, q)| (t, weather::repair_bill(q, 0.0))).collect();
        assert_eq!(worn, expect);
        assert!(!worn.is_empty());
        assert_eq!(bill(&catalog, &recipes, "workbench", 100), Vec::<(String, i32)>::new(), "nothing at full");
        // Less wear, less to mend, but any wear costs one of each
        let little = bill(&catalog, &recipes, "workbench", 99);
        assert!(little.iter().all(|(_, n)| *n == 1), "{little:?}");
        assert_eq!(bill(&catalog, &recipes, "no_such_station", 0), Vec::<(String, i32)>::new());
    }

    #[test]
    fn a_condition_reads_as_what_it_does() {
        assert_eq!(State::of(100), State::Sound);
        assert_eq!(State::of(25), State::Sound);
        assert_eq!(State::of(24), State::NeedsRepair);
        assert_eq!(State::of(0), State::WornOut);
        assert!(State::Sound.words().is_none());
        assert_eq!(Shelter::Indoors { floor: 2 }.words(), "Upstairs on the second floor, out of the weather");
        assert_eq!(worn_words("Workbench", 24).0, "Your workbench needs repair");
        assert_eq!(worn_words("Workbench", 0).0, "Your workbench is worn out");
        assert_eq!(repaired_words("Workbench", "3 Fiber").1, "As good as new, for 3 Fiber");
    }
}
