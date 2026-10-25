//! What a bot's time goes on (Callings K0.1, `the-capitol-callings.md` D2):
//! the eight job activities, the Commons, detours, the road in and free
//! time. Every [`Entry::Spent`](super::report::Entry::Spent) carries one,
//! set from the step and the goal it was for, so a report can say how a
//! citizen spends its day and how many things it does. Pure.

use serde::{Deserialize, Serialize};

use super::plan::{Goal, Step};
use super::{Craft, Gather, Role};
use crate::catalog::Catalog;

/// What a stretch of a bot's time was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    /// Grass, banked at the exchange: the faucet
    Fiber,
    /// Trees and old growth
    Wood,
    Ore,
    /// The sawbench
    Planks,
    /// The forge and kiln
    Ingots,
    /// The workbench and anvil
    Tools,
    /// Stations and bricks
    Building,
    /// Market making and stalls
    Trading,
    /// The hall's board, wages and hiring: claims, deliveries, posts, shifts
    Commons,
    /// The unrelated things people do in work hours (K1.5): nothing yet
    Detour,
    /// The tutorial's walk up the south road (#450)
    RoadIn,
    /// Time with nothing to do, and market hours with nothing to trade
    #[default]
    Free,
}

impl Activity {
    /// Every activity, in the order a report's columns go.
    pub const ALL: [Activity; 12] = [
        Activity::Fiber,
        Activity::Wood,
        Activity::Ore,
        Activity::Planks,
        Activity::Ingots,
        Activity::Tools,
        Activity::Building,
        Activity::Trading,
        Activity::Commons,
        Activity::Detour,
        Activity::RoadIn,
        Activity::Free,
    ];

    /// The eight a citizen leans over (D2): one for each job, and trading.
    pub const JOBS: [Activity; 8] = [
        Activity::Fiber,
        Activity::Wood,
        Activity::Ore,
        Activity::Planks,
        Activity::Ingots,
        Activity::Tools,
        Activity::Building,
        Activity::Trading,
    ];

    /// As a report's JSON keys it: `fiber`, `road_in`.
    pub fn slug(self) -> &'static str {
        match self {
            Activity::Fiber => "fiber",
            Activity::Wood => "wood",
            Activity::Ore => "ore",
            Activity::Planks => "planks",
            Activity::Ingots => "ingots",
            Activity::Tools => "tools",
            Activity::Building => "building",
            Activity::Trading => "trading",
            Activity::Commons => "commons",
            Activity::Detour => "detour",
            Activity::RoadIn => "road_in",
            Activity::Free => "free",
        }
    }

    /// The activity a [`slug`](Self::slug) names.
    pub fn from_slug(slug: &str) -> Option<Activity> {
        Activity::ALL.into_iter().find(|a| a.slug() == slug)
    }

    /// As a report's column heads it: `Fiber`, `Road in`.
    pub fn title(self) -> &'static str {
        match self {
            Activity::Fiber => "Fiber",
            Activity::Wood => "Wood",
            Activity::Ore => "Ore",
            Activity::Planks => "Planks",
            Activity::Ingots => "Ingots",
            Activity::Tools => "Tools",
            Activity::Building => "Building",
            Activity::Trading => "Trading",
            Activity::Commons => "Commons",
            Activity::Detour => "Detour",
            Activity::RoadIn => "Road in",
            Activity::Free => "Free",
        }
    }

    /// One of the eight [`JOBS`](Self::JOBS).
    pub fn is_job(self) -> bool {
        Activity::JOBS.contains(&self)
    }

    /// Work time: everything but [`Free`](Activity::Free).
    pub fn is_work(self) -> bool {
        self != Activity::Free
    }

    /// The job a role works at all its life, as things stand.
    pub fn of_role(role: Role) -> Activity {
        match role {
            Role::Gatherer(Gather::Fiber) => Activity::Fiber,
            Role::Gatherer(Gather::Wood) => Activity::Wood,
            Role::Gatherer(Gather::Ore) => Activity::Ore,
            Role::Crafter(Craft::Planks) => Activity::Planks,
            Role::Crafter(Craft::Ingots) => Activity::Ingots,
            Role::Crafter(Craft::Tools) => Activity::Tools,
            Role::Builder => Activity::Building,
            Role::Merchant => Activity::Trading,
        }
    }

    /// The job an item is the product of, if it's one's: a tool is the
    /// tool crafter's, a kit the builder's. Stone, clay, charcoal and the
    /// like are inputs, gathered on the way to something (D2), so `None`.
    pub fn of_item(catalog: &Catalog, item: &str) -> Option<Activity> {
        Some(match item {
            "fiber" => Activity::Fiber,
            "log" | "sticks" => Activity::Wood,
            "iron_ore" => Activity::Ore,
            "plank" => Activity::Planks,
            "iron_ingot" => Activity::Ingots,
            "tool_handle" => Activity::Tools,
            "brick" => Activity::Building,
            _ if catalog.item_type(item).is_some_and(|t| t.tool.is_some()) => Activity::Tools,
            _ if catalog.station_for_kit(item).is_some() => Activity::Building,
            _ => return None,
        })
    }

    /// What working toward `goal` is, for a bot of `role`: banking (no
    /// goal) and earning a float are fiber, a station is building, and an
    /// item is its job's, or the role's own when it's an input.
    pub fn of_goal(catalog: &Catalog, goal: Option<&Goal>, role: Role) -> Activity {
        match goal {
            None | Some(Goal::Strands(_)) => Activity::Fiber,
            Some(Goal::Station(_)) => Activity::Building,
            Some(Goal::Hold { item, .. }) => Activity::of_item(catalog, item).unwrap_or(Activity::of_role(role)),
        }
    }
}

impl std::fmt::Display for Activity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

/// Whether `step` is setup on the way to `goal` (§10's measure): a deed, a
/// station put down, or a tool or kit made, when that isn't what the goal
/// is after. A builder's own station and a tool crafter's own tool are its
/// work, not setup.
pub fn is_setup(catalog: &Catalog, goal: Option<&Goal>, step: &Step) -> bool {
    match step {
        Step::Claim => true,
        Step::Place { station, .. } => goal != Some(&Goal::Station(station.clone())),
        Step::Craft { recipe, .. } => {
            let Some(made) = catalog
                .recipes()
                .iter()
                .find(|r| &r.id == recipe)
                .and_then(|r| r.outputs.first())
                .map(|o| o.item_type.as_str())
            else {
                return false;
            };
            let capital = catalog.item_type(made).is_some_and(|t| t.tool.is_some()) || catalog.station_for_kit(made).is_some();
            let wanted = match goal {
                Some(Goal::Hold { item, .. }) => item == made,
                Some(Goal::Station(station)) => catalog.kit_for_station(station) == Some(made),
                _ => false,
            };
            capital && !wanted
        }
        Step::Gather { .. } | Step::Deposit => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::ROLES;

    #[test]
    fn every_activity_has_a_slug_that_names_it_again() {
        for a in Activity::ALL {
            assert_eq!(Activity::from_slug(a.slug()), Some(a));
            assert_eq!(serde_json::to_string(&a).unwrap(), format!("\"{}\"", a.slug()));
        }
        assert_eq!(Activity::default(), Activity::Free);
        assert!(Activity::JOBS.iter().all(|a| a.is_job() && a.is_work()));
        assert!(!Activity::Commons.is_job() && Activity::Commons.is_work() && !Activity::Free.is_work());
        // Each role works at a job of its own
        let jobs: std::collections::BTreeSet<Activity> = ROLES.iter().map(|r| Activity::of_role(*r)).collect();
        assert_eq!(jobs.len(), ROLES.len());
    }

    #[test]
    fn a_goal_is_its_items_job_and_an_input_its_roles() {
        let catalog = Catalog::fallback();
        let wood = Role::Gatherer(Gather::Wood);
        assert_eq!(Activity::of_goal(&catalog, None, wood), Activity::Fiber, "banking");
        assert_eq!(Activity::of_goal(&catalog, Some(&Goal::Strands(40)), wood), Activity::Fiber, "a float");
        assert_eq!(Activity::of_goal(&catalog, Some(&Goal::hold("plank", 3)), wood), Activity::Planks, "a claim on planks");
        assert_eq!(Activity::of_goal(&catalog, Some(&Goal::hold("stone", 3)), wood), Activity::Wood, "an input");
        assert_eq!(Activity::of_goal(&catalog, Some(&Goal::hold("stone_axe", 1)), Role::Builder), Activity::Tools);
        assert_eq!(Activity::of_goal(&catalog, Some(&Goal::Station("kiln".into())), wood), Activity::Building);
    }

    #[test]
    fn setup_is_capital_on_the_way_to_something_else() {
        let catalog = Catalog::fallback();
        let axe = catalog
            .recipes()
            .iter()
            .find(|r| r.outputs.first().is_some_and(|o| o.item_type == "stone_axe"))
            .expect("a stone axe recipe")
            .id
            .clone();
        let craft_axe = Step::Craft { recipe: axe, station: None };
        let logs = Goal::hold("log", 5);
        assert!(is_setup(&catalog, Some(&logs), &craft_axe), "an axe for logs");
        assert!(!is_setup(&catalog, Some(&Goal::hold("stone_axe", 1)), &craft_axe), "the tool crafter's own axe");
        let place = Step::Place {
            kit: "kiln_kit".into(),
            station: "kiln".into(),
            on_plot: true,
        };
        assert!(!is_setup(&catalog, Some(&Goal::Station("kiln".into())), &place), "the builder's own kiln");
        assert!(is_setup(&catalog, Some(&Goal::hold("brick", 2)), &place));
        assert!(is_setup(&catalog, Some(&logs), &Step::Claim));
        let gather = Step::Gather {
            resource: "tree".into(),
            item: "log".into(),
        };
        assert!(!is_setup(&catalog, Some(&logs), &gather));
        assert!(!is_setup(&catalog, None, &Step::Deposit));
    }
}
