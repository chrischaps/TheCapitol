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
//!   (D10); every other station needs a plot of the bot's own with room.
//!   A citizen's first plot is the deed the road in earns (#450,
//!   [`Holdings::road_deed`]), so it never saves for one: it claims once
//!   the deed is open, and until then the station waits for the road. A
//!   bot not on the road (the sim's swarm) buys its plot with Strands,
//!   which come from depositing fiber, once it holds [`FLOAT`] over the
//!   deed's price, so it can still buy and sell after (#294)
//!
//! Since a step is taken before the next is planned, the inputs a craft uses
//! up (the rope a handle takes, say) are simply missing next time, and made
//! again.
//!
//! **Setup (Callings D5, #529).** [`setup`] looks further: it plays the steps
//! out against a copy of the holdings, to the goal, and counts what was only
//! on the way. That's the stations, kits, tools and plot still missing, and
//! the steps they take. Taking up a job that isn't your favourite can be all
//! prerequisites, and a citizen choosing its evening's work charges it.

use capitol_protocol::tunables::Fees;
use capitol_protocol::tutorial::Grant;
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
    /// How much of [`plot_room`](Self::plot_room) its role doesn't want for
    /// its own stations: a station that may stand off a plot goes on the
    /// plot only into this (#499).
    pub plot_spare: i32,
    pub strands: i64,
    /// What a deed costs where the bot would buy one.
    pub deed_cost: i64,
    /// The fees the server charges now (D6.1), for what a sale, a rent or
    /// the exchange costs.
    pub fees: Fees,
    /// The deed the road in earns, for a bot that walks it (#450): its
    /// first plot comes from it, not from saving. `None` for a bot that
    /// doesn't walk the road, which buys its plot as before, and a spent
    /// deed means the next plot is bought too.
    pub road_deed: Option<Grant>,
    /// What its claims on commissions still want of it that it doesn't hold
    /// at their grade, by item (Commons S2.5): its work makes them first.
    pub owed: Vec<(String, i32)>,
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
    /// that may stand off a plot, when the plot has no room or there's
    /// none) wherever it stands.
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
    Ok(step(catalog, holdings, goal)?.map(|(step, _)| step))
}

/// The next step toward `goal`, and whether it's setup: a step toward a
/// station, a tool or a plot the goal's own work needs, not that work.
fn step(catalog: &Catalog, holdings: &Holdings, goal: &Goal) -> Result<Option<(Step, bool)>, Stuck> {
    let mut planner = Planner {
        catalog,
        holdings,
        visiting: Vec::new(),
        setting_up: false,
    };
    let step = match goal {
        Goal::Hold { item, n } => planner.obtain(item, *n),
        Goal::Station(station) => planner.station(station),
        Goal::Strands(n) if holdings.strands >= *n => Ok(None),
        Goal::Strands(n) => planner.strands(n - holdings.strands).map(Some),
    }?;
    Ok(step.map(|step| (step, planner.setting_up)))
}

/// The most steps [`setup`] plays out before it calls a goal out of reach.
/// A builder's whole chain from empty hands, the deed's fiber among it, is
/// well under it.
pub const SETUP_HORIZON: u32 = 4000;

/// The room [`setup`] gives a plot it claims: a builder's.
pub const CLAIMED_ROOM: i32 = 4;

/// What a goal still wants set up before its own work (Callings D5): the
/// stations, kits and tools still missing on the way, and the plot, with a
/// rough cost in steps. A step is one planner step: a gather, a craft, a
/// placing, a claim or a deposit. Walking isn't counted.
///
/// What the goal is itself isn't setup: a builder's `own a kiln` counts the
/// kiln's kit as its work, and the workbench the kit is made at as setup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Setup {
    /// Stations it would place on the way, in order.
    pub stations: Vec<String>,
    /// Kits it would make for them.
    pub kits: Vec<String>,
    /// Tools it would make on the way (item types), in order.
    pub tools: Vec<String>,
    /// It would claim a plot on the way.
    pub plot: bool,
    /// Steps the setup takes.
    pub steps: u32,
    /// Steps to the goal, the setup's among them.
    pub total: u32,
}

impl Setup {
    /// Nothing is missing: the goal's own work can start.
    pub fn is_ready(&self) -> bool {
        self.steps == 0
    }
}

/// What `goal` still wants set up (Callings D5, #529). Pure: it plays the
/// planner's steps out against a copy of `holdings`, to the goal. A gather
/// yields its least, a craft makes its least, tools don't wear and a plot it
/// claims has [`CLAIMED_ROOM`], so it's an estimate, not a forecast. What the
/// bot owns already is met, so going back to a job picks up where it left
/// off. `Err` when no step leads there, or not within [`SETUP_HORIZON`].
pub fn setup(catalog: &Catalog, holdings: &Holdings, goal: &Goal) -> Result<Setup, Stuck> {
    let mut h = holdings.clone();
    let mut setup = Setup::default();
    while setup.total < SETUP_HORIZON {
        let Some((step, setting_up)) = step(catalog, &h, goal)? else {
            return Ok(setup);
        };
        setup.total += 1;
        if setting_up {
            setup.steps += 1;
            match &step {
                Step::Place { station, .. } => setup.stations.push(station.clone()),
                Step::Claim => setup.plot = true,
                Step::Craft { recipe, .. } => {
                    let made = catalog.recipes().iter().filter(|r| &r.id == recipe).flat_map(|r| &r.outputs);
                    for made in made {
                        if catalog.station_for_kit(&made.item_type).is_some() {
                            setup.kits.push(made.item_type.clone());
                        } else if catalog.item_type(&made.item_type).is_some_and(|t| t.tool.is_some()) {
                            setup.tools.push(made.item_type.clone());
                        }
                    }
                }
                Step::Gather { .. } | Step::Deposit => {}
            }
        }
        take(catalog, &mut h, &step);
    }
    Err(Stuck(format!("{} is more than {SETUP_HORIZON} steps off", goal.label())))
}

/// What taking `step` does to `h`, as [`setup`] reckons it.
fn take(catalog: &Catalog, h: &mut Holdings, step: &Step) {
    let add = |h: &mut Holdings, item: &str, n: i32| {
        *h.items.entry(item.to_string()).or_insert(0) += n;
    };
    match step {
        Step::Gather { resource, item } => {
            let n = catalog.resource_type(resource).map_or(1, |r| r.yield_quantity_min.max(1));
            add(h, item, n);
        }
        Step::Craft { recipe, .. } => {
            let Some(recipe) = catalog.recipes().iter().find(|r| &r.id == recipe) else {
                return;
            };
            for input in &recipe.inputs {
                add(h, &input.item_type, -input.quantity);
            }
            for output in &recipe.outputs {
                add(h, &output.item_type, output.quantity_min.max(1));
                if let Some(tool) = catalog.item_type(&output.item_type).and_then(|t| t.tool.as_ref()) {
                    h.tools.insert(tool.tool_type.clone());
                }
            }
        }
        Step::Place { kit, station, on_plot } => {
            add(h, kit, -1);
            h.stations.insert(station.clone());
            if *on_plot {
                h.plot_room = h.plot_room.map(|room| room - 1);
                h.plot_spare = (h.plot_spare - 1).max(0);
            }
        }
        Step::Claim => {
            if h.road_deed == Some(Grant::Open) {
                h.road_deed = Some(Grant::Spent);
            } else {
                h.strands -= h.deed_cost;
            }
            h.plot_room = Some(CLAIMED_ROOM);
            h.plot_spare = 0;
        }
        Step::Deposit => {
            let fiber = h.items.remove(FIBER).unwrap_or(0);
            h.strands += h.fees.deposit_quote(fiber as i64).1;
        }
    }
}

struct Planner<'a> {
    catalog: &'a Catalog,
    holdings: &'a Holdings,
    /// The items being obtained further up, so a cycle is caught.
    visiting: Vec<String>,
    /// The step found is setup: it's toward a station, tool or plot that
    /// something further up needs (D5).
    setting_up: bool,
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
                    let step = self.tool(tool_type);
                    return self.set_up(step);
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
                let step = self.station(station)?.ok_or_else(|| {
                    Stuck(format!("the {station} is owned but not counted"))
                });
                return self.set_up(step);
            }
        }
        if let Some(tool_type) = &recipe.tool_type {
            if !self.holdings.tools.contains(tool_type) {
                let step = self.tool(tool_type);
                return self.set_up(step);
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

    /// Marks a step found as setup (D5).
    fn set_up(&mut self, step: Result<Step, Stuck>) -> Result<Step, Stuck> {
        self.setting_up |= step.is_ok();
        step
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
        // One that may stand off a plot still goes on its own lot while
        // there's room its role doesn't want, where it stays: off it, a
        // bench was left wherever the day's work found it, and the spawn
        // filled with them (#499)
        let spare = self.holdings.plot_room.is_some_and(|room| room > 0) && self.holdings.plot_spare > 0;
        if kind.off_plot && !spare {
            return Ok(Some(Step::Place {
                kit,
                station: station.to_string(),
                on_plot: false,
            }));
        }
        // The plot is setup, even for a goal that's the station itself
        if self.holdings.plot_room.is_none() {
            self.setting_up = true;
        }
        match self.holdings.plot_room {
            Some(room) if room > 0 => Ok(Some(Step::Place {
                kit,
                station: station.to_string(),
                on_plot: true,
            })),
            Some(_) => Err(Stuck(format!("the plot has no room for a {station}"))),
            // The road in's deed is its first plot (#450): nothing saved for it
            None if self.holdings.road_deed == Some(Grant::Open) => Ok(Some(Step::Claim)),
            None if self.holdings.road_deed == Some(Grant::Unearned) => Err(Stuck(format!(
                "the {station} waits for a plot, and the plot for the road in's deed"
            ))),
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
        // With a lot of its own and room its role doesn't want, the bench
        // goes there (#499)
        let mut home = holding(&[("workbench_kit", 1)]);
        home.plot_room = Some(1);
        home.plot_spare = 1;
        assert_eq!(
            plan(&home, Goal::hold("log", 1)),
            Some(Step::Place {
                kit: "workbench_kit".into(),
                station: "workbench".into(),
                on_plot: true,
            })
        );
        // Room its role wants for its own stations, or a full lot, leaves it
        // standing where it is
        home.plot_spare = 0;
        assert_eq!(
            plan(&home, Goal::hold("log", 1)),
            Some(Step::Place {
                kit: "workbench_kit".into(),
                station: "workbench".into(),
                on_plot: false,
            })
        );
        home.plot_room = Some(0);
        home.plot_spare = 1;
        assert_eq!(
            plan(&home, Goal::hold("log", 1)),
            Some(Step::Place {
                kit: "workbench_kit".into(),
                station: "workbench".into(),
                on_plot: false,
            })
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
    fn a_bot_on_the_road_in_waits_for_its_deed_rather_than_save_for_one() {
        let goal = || Goal::Station("kiln".into());
        let mut h = holding(&[("kiln_kit", 1)]);
        h.road_deed = Some(Grant::Unearned);
        // Rich or poor, it doesn't save: the road in earns its deed (#450)
        assert!(next(&Catalog::fallback(), &h, &goal()).is_err());
        h.strands = 250 + FLOAT;
        assert!(next(&Catalog::fallback(), &h, &goal()).is_err());
        // Once the road has earned it, it claims, with nothing saved
        h.strands = 0;
        h.road_deed = Some(Grant::Open);
        assert_eq!(plan(&h, goal()), Some(Step::Claim));
        // The deed spent and the plot gone, the next is bought as before
        h.road_deed = Some(Grant::Spent);
        assert_eq!(plan(&h, goal()), gather("fiber"));
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

    // Setup and the dabble (Callings D5, #529)

    use crate::sim::{Commitment, Craft, Gather, Role, BUILD, ROLES, TOOLS};

    fn set_up(h: &Holdings, goal: Goal) -> Setup {
        setup(&Catalog::fallback(), h, &goal).unwrap()
    }

    /// A builder's full set: every station, a tool of each type, and a lot
    /// with no room left.
    fn builders_set() -> Holdings {
        let mut h = with(holding(&[]), &["axe", "pickaxe", "hammer", "sickle"], &BUILD);
        h.plot_room = Some(0);
        h
    }

    #[test]
    fn setup_counts_what_is_only_on_the_way() {
        // A log from empty hands: a workbench from its kit, a stone axe at it
        let s = set_up(&holding(&[]), Goal::hold("log", 1));
        assert_eq!(s.stations, ["workbench"]);
        assert_eq!(s.kits, ["workbench_kit"]);
        assert_eq!(s.tools, ["stone_axe"]);
        assert!(!s.plot);
        assert_eq!(s.total, s.steps + 1, "the gather is the work");
        // With the bench it's only the axe; with the axe, nothing
        let bench = set_up(&with(holding(&[]), &[], &["workbench"]), Goal::hold("log", 1));
        assert!(bench.stations.is_empty() && bench.kits.is_empty());
        assert_eq!(bench.tools, ["stone_axe"]);
        assert!(bench.steps < s.steps);
        let axe = set_up(&with(holding(&[]), &["axe"], &[]), Goal::hold("log", 1));
        assert!(axe.is_ready());
        assert_eq!(axe.total, 1);
        // The planner's own step is unchanged by the counting
        assert_eq!(plan(&holding(&[]), Goal::hold("log", 1)), gather("fiber"));
    }

    #[test]
    fn a_goal_met_needs_no_setup() {
        assert_eq!(set_up(&holding(&[("fiber", 5)]), Goal::hold("fiber", 5)), Setup::default());
        let kiln = with(holding(&[]), &[], &["kiln"]);
        assert_eq!(set_up(&kiln, Goal::Station("kiln".into())), Setup::default());
    }

    #[test]
    fn a_station_is_its_own_work_and_its_plot_is_setup() {
        let goal = || Goal::Station("kiln".into());
        // Its kit is a builder's work; the plot it stands on isn't
        let kit = holding(&[("kiln_kit", 1)]);
        let s = set_up(&kit, goal());
        assert!(s.plot);
        assert!(s.stations.is_empty() && s.kits.is_empty());
        assert_eq!(s.total, s.steps + 1, "placing it is the work");
        // On a lot with room, it's ready
        let mut owner = kit.clone();
        owner.plot_room = Some(3);
        assert_eq!(set_up(&owner, goal()), Setup { total: 1, ..Setup::default() });
        // The road in's deed is a claim, with nothing saved for it
        let mut road = kit;
        road.road_deed = Some(Grant::Open);
        let s = set_up(&road, goal());
        assert!(s.plot);
        assert_eq!((s.steps, s.total), (1, 2));
        // Waiting on the road's deed, no step leads there
        road.road_deed = Some(Grant::Unearned);
        assert!(setup(&Catalog::fallback(), &road, &goal()).is_err());
    }

    #[test]
    fn setup_for_every_job_from_empty_hands() {
        let catalog = Catalog::fallback();
        let empty = holding(&[]);
        let of = |role: Role| {
            role.setup(&catalog, &empty, Commitment::Full)
                .unwrap_or_else(|e| panic!("{role}: {}", e.0))
        };
        // Fiber is by hand, and a builder's first workbench is its own work
        for role in [Role::Gatherer(Gather::Fiber), Role::Merchant, Role::Builder] {
            assert!(of(role).is_ready(), "{role}: {:?}", of(role));
        }
        // Wood and ore want a bench and a stone tool
        assert_eq!(of(Role::Gatherer(Gather::Wood)).tools, ["stone_axe"]);
        assert_eq!(of(Role::Gatherer(Gather::Ore)).tools, ["stone_pickaxe"]);
        // A tool crafter's first axe wants the bench, not another tool
        let tools = of(Role::Crafter(Craft::Tools));
        assert_eq!(tools.stations, ["workbench"]);
        assert!(tools.tools.is_empty());
        // Planks and ingots want a plot and the chain to the forge
        let wood = of(Role::Gatherer(Gather::Wood)).steps;
        for role in [Role::Crafter(Craft::Planks), Role::Crafter(Craft::Ingots)] {
            let s = of(role);
            assert!(s.plot, "{role}");
            assert!(s.stations.iter().any(|s| s == "forge"), "{role}: {:?}", s.stations);
            assert!(s.steps > wood, "{role}");
        }
        assert!(of(Role::Crafter(Craft::Planks)).stations.iter().any(|s| s == "sawbench"));
    }

    #[test]
    fn setup_for_every_job_from_a_builders_full_set() {
        let catalog = Catalog::fallback();
        let h = builders_set();
        for role in ROLES {
            for commitment in [Commitment::Full, Commitment::Dabble] {
                let s = role.setup(&catalog, &h, commitment).unwrap();
                assert!(s.is_ready(), "{role} {commitment:?}: {s:?}");
                assert!(s.total > 0, "{role}: there's still the work");
            }
        }
    }

    #[test]
    fn a_dabbling_tool_crafter_plans_a_handle_not_the_iron_pickaxe() {
        let catalog = Catalog::fallback();
        let tools = Role::Crafter(Craft::Tools);
        let empty = holding(&[]);
        assert_eq!(tools.work(&catalog, &empty, Commitment::Full).len(), TOOLS.len() + 1);
        assert_eq!(tools.work(&catalog, &empty, Commitment::Dabble), [Goal::hold("tool_handle", 1)]);
        // Going back picks up where it left off: the bench stayed, so the
        // handle is nearer
        let bench = with(holding(&[]), &[], &["workbench"]);
        let there = tools.setup(&catalog, &bench, Commitment::Dabble).unwrap();
        assert!(there.is_ready());
        assert!(there.total < tools.setup(&catalog, &empty, Commitment::Dabble).unwrap().total);
        // Its promises and its float still come first
        let mut owing = bench;
        owing.owed = vec![("rope".into(), 2)];
        let goals = tools.goals_at(&catalog, &owing, Commitment::Dabble);
        assert_eq!(goals, [Goal::hold("rope", 2), Goal::Strands(FLOAT), Goal::hold("tool_handle", 1)]);
    }

    #[test]
    fn a_dabbling_builder_plans_bricks_at_its_kiln() {
        let catalog = Catalog::fallback();
        let mut h = with(holding(&[]), &["axe"], &["workbench", "kiln"]);
        h.plot_room = Some(2);
        assert_eq!(Role::Builder.work(&catalog, &h, Commitment::Full)[0], Goal::Station("forge".into()));
        assert_eq!(Role::Builder.work(&catalog, &h, Commitment::Dabble), [Goal::hold("brick", 1)]);
        // Without an axe for the charcoal's log, a chest of fiber is nearer
        h.tools.clear();
        assert_eq!(Role::Builder.work(&catalog, &h, Commitment::Dabble), [Goal::Station("storage_chest".into())]);
        // And with no room for one, the bricks again
        h.plot_room = Some(0);
        assert_eq!(Role::Builder.work(&catalog, &h, Commitment::Dabble), [Goal::hold("brick", 1)]);
        // From empty hands, the workbench is nearest
        let empty = holding(&[]);
        assert_eq!(Role::Builder.work(&catalog, &empty, Commitment::Dabble), [Goal::Station("workbench".into())]);
    }

    #[test]
    fn a_dabble_plans_no_longer_than_full() {
        let catalog = Catalog::fallback();
        let mut cases = 0;
        for role in ROLES {
            for stations in 0..=BUILD.len() {
                for (strands, room) in [(0, None), (400, None), (100, Some(4)), (FLOAT, Some(0))] {
                    let mut h = with(holding(&[("fiber", 12)]), &[], &BUILD[..stations]);
                    h.strands = strands;
                    h.plot_room = room;
                    let full = role.work(&catalog, &h, Commitment::Full);
                    let dabble = role.work(&catalog, &h, Commitment::Dabble);
                    let case = format!("{role} with {stations} stations, {strands} Strands, room {room:?}");
                    assert!(!dabble.is_empty() && dabble.len() <= full.len(), "{case}: {dabble:?}");
                    let total = |goals: &[Goal]| setup(&catalog, &h, &goals[0]).map(|s| s.total);
                    if let Ok(full) = total(&full) {
                        assert!(total(&dabble).unwrap() <= full, "{case}");
                    }
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, ROLES.len() * (BUILD.len() + 1) * 4);
    }

    /// Full commitment is the ladder every role worked before callings, so
    /// the swarm's tests and today's citizens hold.
    #[test]
    fn full_plans_todays_ladder() {
        let catalog = Catalog::fallback();
        let empty = holding(&[]);
        let full = |role: Role, h: &Holdings| role.work(&catalog, h, Commitment::Full);
        let tools: Vec<Goal> = TOOLS.iter().map(|t| Goal::hold(t, 1)).chain([Goal::hold("tool_handle", 1)]).collect();
        assert_eq!(full(Role::Crafter(Craft::Tools), &empty), tools);
        let build: Vec<Goal> = BUILD.iter().map(|s| Goal::Station(s.to_string())).chain([Goal::hold("brick", 1)]).collect();
        assert_eq!(full(Role::Builder, &empty), build);
        assert_eq!(full(Role::Builder, &builders_set()), [Goal::hold("brick", 1)]);
        for (role, item) in [
            (Role::Gatherer(Gather::Fiber), "fiber"),
            (Role::Merchant, "fiber"),
            (Role::Gatherer(Gather::Wood), "log"),
            (Role::Gatherer(Gather::Ore), "iron_ore"),
            (Role::Crafter(Craft::Planks), "plank"),
            (Role::Crafter(Craft::Ingots), "iron_ingot"),
        ] {
            assert_eq!(full(role, &empty), [Goal::hold(item, 1)], "{role}");
        }
        // What every caller reads today is full's
        let mut owing = builders_set();
        owing.owed = vec![("plank".into(), 3)];
        owing.strands = 500;
        for role in ROLES {
            for h in [&empty, &builders_set(), &owing] {
                assert_eq!(role.goals(&catalog, h), role.goals_at(&catalog, h, Commitment::Full), "{role}");
                assert_eq!(role.errands(&catalog, h), role.errands_at(&catalog, h, Commitment::Full), "{role}");
                assert_eq!(role.goal(&catalog, h), role.goals_at(&catalog, h, Commitment::Full)[0], "{role}");
            }
        }
    }
}
