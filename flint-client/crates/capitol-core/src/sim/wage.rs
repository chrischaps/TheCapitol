//! Wages as a citizen weighs them (Commons S2.8): what work offered is, what
//! it pays a game hour, and what the citizen's own trade earns one. Pure, so
//! it's tested without a server.
//!
//! Both hours are counted at the bench or the node: a craft's duration, a
//! gather's, and what each makes. Walking is left out of both, since a
//! shift and a citizen's own trade walk alike: to the bench, between nodes.
//! The walk to the work isn't weighed, so far-off work looks as good as near
//! ([#507](https://github.com/chrischaps/TheCapitol/issues/507)).

use capitol_protocol::rest::{CatalogResourceType, RecipeInfo};
use capitol_protocol::rules::wages::Work;
use capitol_protocol::rules::TICK_MS;

use super::Role;
use crate::catalog::Catalog;

/// Work offered, as a role's menu needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job<'a> {
    /// Crafting `makes` at a station of type `station`, or by hand with
    /// `None`
    Craft { station: Option<&'a str>, makes: &'a str },
    /// Gathering what yields `yields`
    Gather { yields: &'a str },
}

/// An offer of work, as a citizen's wage menu needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hire<'a> {
    pub job: Job<'a>,
    /// Units left before the cap
    pub left: i32,
    /// It holds the tool the work needs, if it needs one
    pub tooled: bool,
}

/// The job `work` is, from the catalog: `None` for a recipe or resource it
/// doesn't know.
pub fn job<'a>(catalog: &'a Catalog, work: &'a Work) -> Option<Job<'a>> {
    match work {
        Work::Craft { recipe_id, .. } => {
            let recipe = recipe(catalog, recipe_id)?;
            Some(Job::Craft {
                station: recipe.required_station_type.as_deref(),
                makes: &recipe.outputs.first()?.item_type,
            })
        }
        Work::Gather { resource, .. } => Some(Job::Gather {
            yields: &catalog.resource_type(resource)?.yields_item,
        }),
    }
}

/// The tool `work` needs from the worker, if any: a resource's required
/// tool. A craft's tool may be the station's own (E1.6), so a craft asks
/// none of the worker here; a craft that fails for want of one ends the
/// shift.
pub fn tool_needed<'a>(catalog: &'a Catalog, work: &Work) -> Option<&'a str> {
    match work {
        Work::Craft { .. } => None,
        Work::Gather { resource, .. } => catalog.resource_type(resource)?.required_tool.as_deref(),
    }
}

/// What `work` is, in a few words: `craft plank at a sawbench`, `gather
/// grass`, `twist rope by hand`.
pub fn words(catalog: &Catalog, work: &Work) -> String {
    match work {
        Work::Craft { recipe_id, .. } => {
            let Some(recipe) = recipe(catalog, recipe_id) else {
                return format!("craft {recipe_id}");
            };
            let makes = recipe.outputs.first().map_or(recipe_id.as_str(), |o| o.item_type.as_str());
            match &recipe.required_station_type {
                Some(station) => format!("craft {makes} at a {station}"),
                None => format!("craft {makes} by hand"),
            }
        }
        Work::Gather { resource, .. } => format!("gather {resource}"),
    }
}

/// What `work` pays a game hour at `rate` a unit, worked steadily: units a
/// craft or a gather makes on average, over its duration.
pub fn pays(catalog: &Catalog, work: &Work, rate: i64) -> Option<i64> {
    let per_hour = match work {
        Work::Craft { recipe_id, .. } => {
            let r = recipe(catalog, recipe_id)?;
            crafts_an_hour(r) * made(r)
        }
        Work::Gather { resource, .. } => {
            let r = catalog.resource_type(resource)?;
            gathers_an_hour(r) * gathered(r)
        }
    };
    Some((per_hour * rate as f64).round() as i64)
}

/// What `role`'s own trade earns a game hour at the books (`worth`), worked
/// steadily: its first good's worth from the quickest node, or a craft's worth less
/// its inputs' at one of its stations. `None` when the books can't put a
/// worth to the good or an input, or the role has no trade of its own.
pub fn earns(role: Role, catalog: &Catalog, worth: impl Fn(&str) -> Option<i64>) -> Option<i64> {
    let item = *role.answers().first()?;
    // From a node: the quickest of those that yield it (trees, not old growth)
    let quickest = catalog
        .resource_types()
        .iter()
        .filter(|r| r.yields_item == item)
        .map(|r| gathers_an_hour(r) * gathered(r))
        .max_by(f64::total_cmp);
    if let Some(per_hour) = quickest {
        return Some((per_hour * worth(item)? as f64).round() as i64);
    }
    let makes = |r: &&RecipeInfo| r.outputs.first().is_some_and(|o| o.item_type == item);
    let at_its_station = |r: &&RecipeInfo| r.required_station_type.as_deref().is_none_or(|s| role.crafts_at().contains(&s));
    let r = catalog.recipes().iter().filter(makes).find(at_its_station)?;
    let mut net = made(r) * worth(item)? as f64;
    for input in &r.inputs {
        net -= input.quantity as f64 * worth(&input.item_type)? as f64;
    }
    Some((crafts_an_hour(r) * net).round() as i64)
}

fn recipe<'a>(catalog: &'a Catalog, id: &str) -> Option<&'a RecipeInfo> {
    catalog.recipes().iter().find(|r| r.id == id)
}

/// Seconds of game time `ticks` take.
fn secs(ticks: i32) -> f64 {
    (ticks.max(1) as u64 * TICK_MS) as f64 / 1000.0
}

fn crafts_an_hour(r: &RecipeInfo) -> f64 {
    3600.0 / secs(r.duration_ticks)
}

fn gathers_an_hour(r: &CatalogResourceType) -> f64 {
    3600.0 / secs(r.extraction_duration_ticks)
}

/// What a craft makes of its first output, on average.
fn made(r: &RecipeInfo) -> f64 {
    r.outputs.first().map_or(0.0, |o| (o.quantity_min + o.quantity_max) as f64 / 2.0)
}

/// What a gather yields, on average: its `hand` row, or its lowest tier's.
fn gathered(r: &CatalogResourceType) -> f64 {
    (r.yield_quantity_min + r.yield_quantity_max) as f64 / 2.0
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::sim::{Craft, Gather};

    fn craft(recipe: &str) -> Work {
        Work::Craft {
            station_id: Uuid::nil(),
            recipe_id: recipe.into(),
        }
    }

    fn gather(resource: &str) -> Work {
        Work::Gather {
            plot_id: Uuid::nil(),
            resource: resource.into(),
        }
    }

    #[test]
    fn work_is_known_by_what_it_makes_and_where() {
        let catalog = Catalog::fallback();
        assert_eq!(
            job(&catalog, &craft("plank")),
            Some(Job::Craft {
                station: Some("sawbench"),
                makes: "plank"
            })
        );
        assert_eq!(
            job(&catalog, &craft("rope_basic")),
            Some(Job::Craft { station: None, makes: "rope" })
        );
        assert_eq!(job(&catalog, &gather("grass")), Some(Job::Gather { yields: "fiber" }));
        assert_eq!(job(&catalog, &craft("no_such")), None);
        assert_eq!(words(&catalog, &craft("plank")), "craft plank at a sawbench");
        assert_eq!(words(&catalog, &craft("rope_basic")), "craft rope by hand");
        assert_eq!(words(&catalog, &gather("tree")), "gather tree");
        assert_eq!(tool_needed(&catalog, &gather("tree")), Some("axe"));
        assert_eq!(tool_needed(&catalog, &gather("grass")), None);
        assert_eq!(tool_needed(&catalog, &craft("iron_axe_head")), None);
    }

    #[test]
    fn an_hour_pays_what_its_crafts_or_gathers_make_at_the_rate() {
        let catalog = Catalog::fallback();
        // Three planks every 6 s: 1800 an hour
        assert_eq!(pays(&catalog, &craft("plank"), 2), Some(3600));
        // A handle every 6 s
        assert_eq!(pays(&catalog, &craft("tool_handle"), 5), Some(3000));
        // Grass, 1 to 3 every 3 s: 2400 an hour
        assert_eq!(pays(&catalog, &gather("grass"), 1), Some(2400));
        assert_eq!(pays(&catalog, &gather("no_such"), 1), None);
    }

    #[test]
    fn a_trade_earns_its_goods_worth_less_its_inputs() {
        let catalog = Catalog::fallback();
        let books = |item: &str| match item {
            "log" => Some(6),
            "plank" => Some(4),
            "iron_ore" => Some(20),
            _ => None,
        };
        // Three planks at 4 from a log at 6, every 6 s
        assert_eq!(earns(Role::Crafter(Craft::Planks), &catalog, books), Some(3600));
        // Logs: 1 or 2 every 6 s, at 6
        assert_eq!(earns(Role::Gatherer(Gather::Wood), &catalog, books), Some(5400));
        // Ingots: nothing on the books for them
        assert_eq!(earns(Role::Crafter(Craft::Ingots), &catalog, books), None);
        // Dear logs: the planks lose money
        let dear = |item: &str| if item == "log" { Some(20) } else { books(item) };
        assert!(earns(Role::Crafter(Craft::Planks), &catalog, dear).unwrap() < 0);
        assert_eq!(earns(Role::Merchant, &catalog, books), None);
    }

    /// At the books' targets, the Flint form's default rate for planks
    /// (half the plank's target) pays a planks crafter less than its own
    /// trade, and a Strand more pays more.
    #[test]
    fn the_default_rate_for_planks_is_just_under_a_crafters_own() {
        let catalog = Catalog::fallback();
        let targets = |item: &str| catalog.target_price(item);
        let own = earns(Role::Crafter(Craft::Planks), &catalog, targets).unwrap();
        let rate = crate::wages::rate_for(&craft("plank"), &catalog);
        assert!(pays(&catalog, &craft("plank"), rate).unwrap() < own);
        assert!(pays(&catalog, &craft("plank"), rate + 1).unwrap() >= own);
    }
}
