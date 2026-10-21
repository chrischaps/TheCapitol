//! What a sim bot does next. Pure: it sees the catalog and what the bot
//! holds, and names one step toward a goal. The runner takes the step, looks
//! again and asks again, so a plan is never more than one step long and
//! never goes stale.
//!
//! The search walks the content tree backwards from the goal, the way a new
//! player learns it:
//! - an item a resource yields is gathered, once the bot holds the tool the
//!   resource requires (a missing tool is the thing to get first)
//! - otherwise a recipe makes it: the one at a station the bot already has,
//!   else the first in the catalog. Before crafting, the bot needs the
//!   recipe's station and tool, then each input in turn
//! - a station is placed from its kit. A workbench can stand anywhere
//!   (D10); every other station needs a plot of the bot's own with room,
//!   and a plot needs Strands, which come from depositing fiber. A bot
//!   buys the deed once it holds [`FLOAT`] over its price, so it can still
//!   buy and sell after (#294)
//!
//! Since a step is taken before the next is planned, the inputs a craft uses
//! up (the rope a handle takes, say) are simply missing next time, and made
//! again.

use capitol_protocol::tunables::Fees;
use std::collections::{HashMap, HashSet};

use capitol_protocol::rest::RecipeInfo;

use crate::catalog::Catalog;
use crate::exchange::FIBER;

/// How deep the search may go. D5's longest chain (vein to iron axe, with
/// its stations and their kits) is about a dozen.
const MAX_DEPTH: usize = 24;

/// The Strands a bot wants in hand past a deed (#294): what its inputs and
/// the fees on its sales take. It claims a plot once it holds a deed's price
/// and this much more, and earns it back by fiber when it's down to
/// [`FLOOR`]. A new player's 100.
pub const FLOAT: i64 = 100;
/// The Strands a bot never spends on goods or rent, for its listing fees,
/// and under which it earns its [`FLOAT`] back.
pub const FLOOR: i64 = 10;

/// What a bot holds, as the planner needs it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Holdings {
    /// Units by item type, in the inventory and the crafting output.
    pub items: HashMap<String, i32>,
    /// Tool types the bot holds a tool of, with uses left.
    pub tools: HashSet<String>,
    /// Station types the bot owns and can reach.
    pub stations: HashSet<String>,
    /// The bot's plot, if it has one: how many more stations it takes.
    pub plot_room: Option<i32>,
    pub strands: i64,
    /// What a deed costs where the bot would buy one.
    pub deed_cost: i64,
    /// The fees the server charges now (D6.1), for what a sale, a rent or
    /// the exchange costs.
    pub fees: Fees,
}

impl Holdings {
    pub fn count(&self, item_type: &str) -> i32 {
        self.items.get(item_type).copied().unwrap_or(0)
    }
}

/// What a bot is after.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Goal {
    /// Hold at least `n` of an item.
    Hold { item: String, n: i32 },
    /// Own a station of this type.
    Station(String),
    /// Hold at least this many Strands (#294).
    Strands(i64),
}

impl Goal {
    pub fn hold(item: &str, n: i32) -> Goal {
        Goal::Hold {
            item: item.to_string(),
            n,
        }
    }

    /// As a citizen's menu names it: `hold 21 plank`, `own a forge`.
    pub fn label(&self) -> String {
        match self {
            Goal::Hold { item, n } => format!("hold {n} {item}"),
            Goal::Station(station) => format!("own a {station}"),
            Goal::Strands(n) => format!("hold {n} Strands"),
        }
    }
}

/// One thing to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Gather a node of this resource type, which yields `item`.
    Gather { resource: String, item: String },
    /// Craft this recipe, at a station of this type if it names one.
    Craft {
        recipe: String,
        station: Option<String>,
    },
    /// Put a station down from its kit, on the bot's plot or (for a type
    /// that may stand off a plot) wherever it stands.
    Place {
        kit: String,
        station: String,
        on_plot: bool,
    },
    /// Buy the deed to a plot.
    Claim,
    /// Deposit every fiber stack at the exchange, for Strands.
    Deposit,
}

impl Step {
    /// What the report files the step's time under: `gather log`,
    /// `craft iron_ingot`, `place kiln`.
    pub fn label(&self, catalog: &Catalog) -> String {
        match self {
            Step::Gather { item, .. } => format!("gather {item}"),
            Step::Craft { recipe, .. } => {
                let made = catalog
                    .recipes()
                    .iter()
                    .find(|r| &r.id == recipe)
                    .and_then(|r| r.outputs.first())
                    .map_or(recipe.as_str(), |o| o.item_type.as_str());
                format!("craft {made}")
            }
            Step::Place { station, .. } => format!("place {station}"),
            Step::Claim => "claim a plot".into(),
            Step::Deposit => "deposit fiber".into(),
        }
    }
}

/// Why no step leads to the goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stuck(pub String);

/// The next step toward `goal`, or `None` once it's met.
pub fn next(catalog: &Catalog, holdings: &Holdings, goal: &Goal) -> Result<Option<Step>, Stuck> {
    let mut planner = Planner {
        catalog,
        holdings,
        visiting: Vec::new(),
    };
    match goal {
        Goal::Hold { item, n } => planner.obtain(item, *n),
        Goal::Station(station) => planner.station(station),
        Goal::Strands(n) if holdings.strands >= *n => Ok(None),
        Goal::Strands(n) => planner.strands(n - holdings.strands).map(Some),
    }
}

struct Planner<'a> {
    catalog: &'a Catalog,
    holdings: &'a Holdings,
    /// The items being obtained further up, so a cycle is caught.
    visiting: Vec<String>,
}

impl<'a> Planner<'a> {
    /// The next step toward holding `n` of `item`.
    fn obtain(&mut self, item: &str, n: i32) -> Result<Option<Step>, Stuck> {
        if self.holdings.count(item) >= n {
            return Ok(None);
        }
        if self.visiting.iter().any(|v| v == item) || self.visiting.len() >= MAX_DEPTH {
            return Err(Stuck(format!(
                "making {item} needs {item}: {}",
                self.visiting.join(" ← ")
            )));
        }
        self.visiting.push(item.to_string());
        let step = self.obtain_one(item);
        self.visiting.pop();
        step.map(Some)
    }

    fn obtain_one(&mut self, item: &str) -> Result<Step, Stuck> {
        if let Some(resource) = self
            .catalog
            .resource_types()
            .iter()
            .find(|r| r.yields_item == item)
        {
            if let Some(tool_type) = &resource.required_tool {
                if !self.holdings.tools.contains(tool_type) {
                    return self.tool(tool_type);
                }
            }
            return Ok(Step::Gather {
                resource: resource.id.clone(),
                item: item.to_string(),
            });
        }
        let recipe = self
            .recipe_for(item)
            .ok_or_else(|| Stuck(format!("nothing yields or makes {item}")))?;
        if let Some(station) = &recipe.required_station_type {
            if !self.holdings.stations.contains(station) {
                return self.station(station)?.ok_or_else(|| {
                    Stuck(format!("the {station} is owned but not counted"))
                });
            }
        }
        if let Some(tool_type) = &recipe.tool_type {
            if !self.holdings.tools.contains(tool_type) {
                return self.tool(tool_type);
            }
        }
        for input in &recipe.inputs {
            if let Some(step) = self.obtain(&input.item_type, input.quantity)? {
                return Ok(step);
            }
        }
        Ok(Step::Craft {
            recipe: recipe.id.clone(),
            station: recipe.required_station_type.clone(),
        })
    }

    /// The recipe that makes `item`: one at a station the bot has (or by
    /// hand), else the first the catalog lists.
    fn recipe_for(&self, item: &str) -> Option<&'a RecipeInfo> {
        let makes = |r: &&'a RecipeInfo| r.outputs.iter().any(|o| o.item_type == item);
        let catalog: &'a Catalog = self.catalog;
        let recipes = catalog.recipes();
        recipes
            .iter()
            .filter(makes)
            .find(|r| {
                r.required_station_type
                    .as_ref()
                    .is_none_or(|s| self.holdings.stations.contains(s))
            })
            .or_else(|| recipes.iter().find(makes))
    }

    /// The next step toward holding a tool of `tool_type`: the weakest one,
    /// which is the bootstrap tier.
    fn tool(&mut self, tool_type: &str) -> Result<Step, Stuck> {
        let tool = self
            .catalog
            .item_types()
            .iter()
            .filter(|t| t.tool.as_ref().is_some_and(|t| t.tool_type == tool_type))
            .min_by(|a, b| {
                let power = |t: &capitol_protocol::rest::CatalogItemType| {
                    t.tool.as_ref().map_or(0.0, |t| t.base_power)
                };
                power(a).total_cmp(&power(b))
            })
            .ok_or_else(|| Stuck(format!("no tool is a {tool_type}")))?
            .id
            .clone();
        self.obtain(&tool, 1)?
            .ok_or_else(|| Stuck(format!("holding a {tool} that isn't counted as a {tool_type}")))
    }

    /// The next step toward owning a `station`, or `None` if the bot has one.
    fn station(&mut self, station: &str) -> Result<Option<Step>, Stuck> {
        if self.holdings.stations.contains(station) {
            return Ok(None);
        }
        let kind = self
            .catalog
            .station_type(station)
            .ok_or_else(|| Stuck(format!("no station type {station}")))?;
        let kit = kind
            .kit
            .clone()
            .ok_or_else(|| Stuck(format!("the {station} has no kit")))?;
        if let Some(step) = self.obtain(&kit, 1)? {
            return Ok(Some(step));
        }
        if kind.off_plot {
            return Ok(Some(Step::Place {
                kit,
                station: station.to_string(),
                on_plot: false,
            }));
        }
        match self.holdings.plot_room {
            Some(room) if room > 0 => Ok(Some(Step::Place {
                kit,
                station: station.to_string(),
                on_plot: true,
            })),
            Some(_) => Err(Stuck(format!("the plot has no room for a {station}"))),
            None if self.holdings.strands >= self.holdings.deed_cost + FLOAT => Ok(Some(Step::Claim)),
            None => self.strands(self.holdings.deed_cost + FLOAT - self.holdings.strands).map(Some),
        }
    }

    /// The next step toward `short` more Strands: fiber, then the exchange.
    fn strands(&mut self, short: i64) -> Result<Step, Stuck> {
        // The exchange keeps its fee.
        let fiber = (short as f64 / (1.0 - self.holdings.fees.exchange_fee_rate)).ceil() as i32;
        match self.obtain(FIBER, fiber)? {
            Some(step) => Ok(step),
            None => Ok(Step::Deposit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(items: &[(&str, i32)]) -> Holdings {
        Holdings {
            items: items.iter().map(|(t, n)| (t.to_string(), *n)).collect(),
            deed_cost: 250,
            ..Holdings::default()
        }
    }

    fn with(mut h: Holdings, tools: &[&str], stations: &[&str]) -> Holdings {
        h.tools.extend(tools.iter().map(|t| t.to_string()));
        h.stations.extend(stations.iter().map(|s| s.to_string()));
        h
    }

    fn plan(h: &Holdings, goal: Goal) -> Option<Step> {
        next(&Catalog::fallback(), h, &goal).unwrap()
    }

    fn gather(item: &str) -> Option<Step> {
        let catalog = Catalog::fallback();
        let resource = catalog
            .resource_types()
            .iter()
            .find(|r| r.yields_item == item)
            .unwrap()
            .id
            .clone();
        Some(Step::Gather {
            resource,
            item: item.into(),
        })
    }

    fn craft(recipe: &str, station: Option<&str>) -> Option<Step> {
        Some(Step::Craft {
            recipe: recipe.into(),
            station: station.map(str::to_owned),
        })
    }

    #[test]
    fn a_goal_held_needs_nothing() {
        assert_eq!(plan(&holding(&[("fiber", 5)]), Goal::hold("fiber", 5)), None);
    }

    #[test]
    fn rope_is_fiber_then_a_twist_by_hand() {
        assert_eq!(plan(&holding(&[]), Goal::hold("rope", 1)), gather("fiber"));
        assert_eq!(
            plan(&holding(&[("fiber", 2)]), Goal::hold("rope", 1)),
            craft("rope_basic", None)
        );
    }

    #[test]
    fn a_log_wants_an_axe_and_an_axe_wants_a_workbench() {
        let h = holding(&[]);
        // No axe, no workbench, no kit: the kit's fiber comes first.
        assert_eq!(plan(&h, Goal::hold("log", 1)), gather("fiber"));
        let h = holding(&[("fiber", 10)]);
        assert_eq!(plan(&h, Goal::hold("log", 1)), craft("craft_workbench", None));
        let h = holding(&[("workbench_kit", 1)]);
        assert_eq!(
            plan(&h, Goal::hold("log", 1)),
            Some(Step::Place {
                kit: "workbench_kit".into(),
                station: "workbench".into(),
                on_plot: false,
            }),
            "a workbench stands off a plot (D10)"
        );
        // With the bench: knapped stone first, from stone by hand.
        let h = with(holding(&[]), &[], &["workbench"]);
        assert_eq!(plan(&h, Goal::hold("log", 1)), gather("stone"));
        let h = with(
            holding(&[("knapped_stone", 2), ("tool_handle", 1), ("rope", 1)]),
            &[],
            &["workbench"],
        );
        assert_eq!(plan(&h, Goal::hold("log", 1)), craft("stone_axe", Some("workbench")));
        let h = with(holding(&[]), &["axe"], &[]);
        assert_eq!(plan(&h, Goal::hold("log", 1)), gather("log"));
    }

    #[test]
    fn the_cheap_handle_is_sticks_and_rope_until_there_is_a_sawbench() {
        let h = with(holding(&[("sticks", 2), ("rope", 1)]), &[], &["workbench"]);
        assert_eq!(
            plan(&h, Goal::hold("tool_handle", 1)),
            craft("tool_handle", Some("workbench"))
        );
        let h = with(holding(&[("plank", 1)]), &[], &["sawbench"]);
        assert_eq!(
            plan(&h, Goal::hold("tool_handle", 1)),
            craft("sawn_handle", Some("sawbench")),
            "the bot has a sawbench and no workbench"
        );
    }

    #[test]
    fn a_kiln_needs_a_plot_and_a_plot_needs_strands() {
        let kit = holding(&[("kiln_kit", 1)]);
        assert_eq!(plan(&kit, Goal::Station("kiln".into())), gather("fiber"));
        let mut rich = kit.clone();
        rich.items.insert("fiber".into(), 352);
        assert_eq!(
            plan(&rich, Goal::Station("kiln".into())),
            Some(Step::Deposit),
            "a deed and the float, 350 Strands, is 352 fiber after the fee"
        );
        // The deed alone isn't enough: it keeps its float past it (#294)
        rich.items.remove("fiber");
        rich.strands = 250;
        assert_eq!(plan(&rich, Goal::Station("kiln".into())), gather("fiber"));
        rich.strands = 250 + FLOAT;
        assert_eq!(plan(&rich, Goal::Station("kiln".into())), Some(Step::Claim));
        let mut owner = kit.clone();
        owner.plot_room = Some(3);
        assert_eq!(
            plan(&owner, Goal::Station("kiln".into())),
            Some(Step::Place {
                kit: "kiln_kit".into(),
                station: "kiln".into(),
                on_plot: true,
            })
        );
        owner.plot_room = Some(0);
        assert!(next(&Catalog::fallback(), &owner, &Goal::Station("kiln".into())).is_err());
        owner.stations.insert("kiln".into());
        assert_eq!(plan(&owner, Goal::Station("kiln".into())), None);
    }

    #[test]
    fn strands_are_fiber_then_the_exchange() {
        let mut h = holding(&[]);
        h.strands = 30;
        assert_eq!(plan(&h, Goal::Strands(FLOAT)), gather("fiber"));
        h.items.insert("fiber".into(), 71);
        assert_eq!(plan(&h, Goal::Strands(FLOAT)), Some(Step::Deposit));
        h.strands = FLOAT;
        assert_eq!(plan(&h, Goal::Strands(FLOAT)), None);
        assert_eq!(Goal::Strands(FLOAT).label(), "hold 100 Strands");
    }

    #[test]
    fn anvil_work_wants_a_hammer() {
        let h = with(holding(&[("iron_ingot", 2)]), &[], &["anvil", "workbench"]);
        assert_eq!(
            plan(&h, Goal::hold("iron_axe_head", 1)),
            gather("stone"),
            "a stone hammer, from knapped stone"
        );
        let h = with(holding(&[("iron_ingot", 2)]), &["hammer"], &["anvil"]);
        assert_eq!(
            plan(&h, Goal::hold("iron_axe_head", 1)),
            craft("iron_axe_head", Some("anvil"))
        );
    }

    /// Every item in the content can be planned for from nothing, and the
    /// plan's first step is one a new player can take.
    #[test]
    fn every_item_has_a_first_step_from_empty_hands() {
        let catalog = Catalog::fallback();
        let empty = holding(&[]);
        for item in catalog.item_types() {
            let step = next(&catalog, &empty, &Goal::hold(&item.id, 1))
                .unwrap_or_else(|e| panic!("{}: {}", item.id, e.0))
                .unwrap();
            assert!(
                matches!(
                    &step,
                    Step::Gather { .. } | Step::Craft { station: None, .. }
                ),
                "{} starts with {step:?}",
                item.id
            );
        }
    }
}
