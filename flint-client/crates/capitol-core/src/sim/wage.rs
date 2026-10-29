//! Wages as a citizen weighs them (Commons S2.8): what work offered is, what
//! it pays a game hour, and what the citizen's own trade earns one. Pure, so
//! it's tested without a server.
//!
//! Both hours are counted at the bench or the node: a craft's duration, a
//! gather's, and what each makes ([`pays`], [`earns`]). The walk between
//! nodes is left out of both, since a shift and a citizen's own trade walk
//! alike there. The walk *to* work offered isn't alike: its own trade is on
//! its lot, and the work may be across the city. So an offer is weighed with
//! that walk spread over the shift ([`pays_walked`], #507): far-off work
//! pays less an hour than the same work next door, and a small offer far
//! off least of all.
//!
//! **Hiring** (Callings K2.4) is the other side: the work a citizen could
//! offer ([`openings`]), at a station of its own that the job of the moment
//! doesn't work at, from inputs that job doesn't need, or on its lot,
//! gathering what grows there that the job doesn't gather; whether it can
//! afford to ([`hire_menu`], over its [`hire_line`]); the rate it pays
//! ([`hire_rate`]); and when it wants the place back ([`wants_back`]).

use std::collections::HashMap;

use capitol_protocol::rest::{CatalogResourceType, RecipeInfo};
use capitol_protocol::rules::court::court_fee;
use capitol_protocol::rules::wages::{offer_value, Work};
use capitol_protocol::rules::TICK_MS;
use uuid::Uuid;

use super::plan::Holdings;
use super::{Role, ROLES};
use crate::catalog::Catalog;
use crate::citizen::decide::{Choice, Question, Span, Temperament};

/// The fewest units worth offering work for (K2.4): two logs' planks.
pub const LEAST_HIRE: i32 = 6;
/// The least an offer is worth at the fair rate, in Strands: the court's
/// fee is at least 6, so a smaller one pays more than a tenth again in fees.
pub const LEAST_HIRE_VALUE: i64 = 50;
/// The most units a gathering offer pays for: what its escrow holds.
pub const GATHER_CAP: i32 = 60;
/// A middling citizen's line (K2.4): the Strands it keeps back before it
/// hires, unless the town's `[hiring] line` says otherwise. About what a
/// merchant's float is.
pub const HIRE_LINE: i64 = 600;

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

/// Units of `work` made a game hour, worked steadily: what a craft or a
/// gather makes on average, over its duration.
pub fn units_an_hour(catalog: &Catalog, work: &Work) -> Option<f64> {
    Some(match work {
        Work::Craft { recipe_id, .. } => {
            let r = recipe(catalog, recipe_id)?;
            crafts_an_hour(r) * made(r)
        }
        Work::Gather { resource, .. } => {
            let r = catalog.resource_type(resource)?;
            gathers_an_hour(r) * gathered(r)
        }
    })
}

/// What `work` pays a game hour at `rate` a unit, worked steadily at the
/// bench or the node: its [`units_an_hour`] at the rate.
pub fn pays(catalog: &Catalog, work: &Work, rate: i64) -> Option<i64> {
    Some((units_an_hour(catalog, work)? * rate as f64).round() as i64)
}

/// What `work` pays a game hour at `rate` a unit once the walk to it is
/// counted (#507): [`pays`], with a walk of `walk` game seconds spread over
/// the shift,
///
/// ```text
/// pays × units / (units + walk × units_an_hour / 3600)
/// ```
///
/// where `units` is the `left` to the offer's cap, as many as it makes in
/// the `secs` it has to work in once it's there (the rest of its working
/// day, or the offer's term; `None` for no end). So a walk counts for more
/// the fewer units it's for. `None` when the catalog doesn't know the work,
/// or no unit fits in the time after the walk.
pub fn pays_walked(catalog: &Catalog, work: &Work, rate: i64, left: i32, walk: f64, secs: Option<f64>) -> Option<i64> {
    let per_hour = units_an_hour(catalog, work)?;
    let walk = walk.max(0.0);
    let fits = secs.map_or(f64::INFINITY, |s| ((s - walk).max(0.0) * per_hour / 3600.0).floor());
    let units = f64::from(left.max(0)).min(fits);
    if units < 1.0 {
        return None;
    }
    let at_work = per_hour * rate as f64;
    Some((at_work * units / (units + walk * per_hour / 3600.0)).round() as i64)
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

/// Work a citizen could hire someone for (Callings K2.4).
#[derive(Debug, Clone, PartialEq)]
pub struct Opening {
    pub work: Work,
    /// The most units it could offer: what the inputs it can spare make,
    /// or a gather's [`GATHER_CAP`]
    pub most: i32,
    /// What a craft's box wants for `most`, by input: nothing for a gather
    pub stock: Vec<(String, i32)>,
}

impl Opening {
    /// What `cap` units take of each input, in whole crafts.
    pub fn stock_for(&self, catalog: &Catalog, cap: i32) -> Vec<(String, i32)> {
        let Work::Craft { recipe_id, .. } = &self.work else { return Vec::new() };
        let Some(r) = recipe(catalog, recipe_id) else { return Vec::new() };
        let crafts = crafts_for(r, cap);
        r.inputs.iter().map(|i| (i.item_type.clone(), i.quantity * crafts)).collect()
    }
}

/// Crafts of `r` that make `units`, at its least yield.
fn crafts_for(r: &RecipeInfo, units: i32) -> i32 {
    let made = r.outputs.first().map_or(1, |o| o.quantity_min.max(1));
    (units.max(0) + made - 1) / made
}

/// The goods a station of `kind` is hired for: what the jobs that work at
/// it answer commissions for or need, as made there. A sawbench saws
/// planks, a forge smelts ingots, a kiln burns charcoal and bricks, and a
/// workbench makes handles; an anvil's heads are a tool crafter's own.
fn hired_for(kind: &str) -> Vec<&'static str> {
    let mut goods: Vec<&'static str> = ROLES
        .iter()
        .filter(|r| r.crafts_at().contains(&kind))
        .flat_map(|r| r.answers().iter().chain(r.needs()).copied())
        .collect();
    goods.sort_unstable();
    goods.dedup();
    goods
}

/// Whether the job of the moment wants `work`'s place for itself: it works
/// at that station, or gathers that resource. A citizen hires out only
/// what its stint leaves idle, and takes it back when a stint wants it.
pub fn wants_back(job_now: Role, catalog: &Catalog, work: &Work) -> bool {
    match job(catalog, work) {
        Some(Job::Craft { station: Some(station), .. }) => job_now.crafts_at().contains(&station),
        Some(j @ Job::Gather { .. }) => job_now.does(&j),
        _ => false,
    }
}

/// The work a citizen at `job_now` could offer (K2.4):
/// - **at a station of its own** (`stations`, by type) that `job_now`
///   doesn't work at: the recipe there, of the goods it's hired for (and
///   at a workbench, handwork a trade answers for: rope), that `spare`
///   (what it holds that `job_now` neither needs nor makes) makes the most
///   of at their targets, at least [`LEAST_HIRE`] units' worth;
/// - **on its lot** (`lot`: its id, and the resources that grow in its
///   bounds with how many nodes of each): gathering a gatherer's good that
///   `job_now` doesn't gather, where the nodes keep a worker busy
///   ([`gather_most`]), up to a game hour's yield or [`GATHER_CAP`].
///
/// A recipe that wants a tool of the crafter is left out, since the worker
/// may not hold one.
pub fn openings(
    catalog: &Catalog,
    job_now: Role,
    stations: &[(String, Uuid)],
    spare: &HashMap<String, i32>,
    lot: Option<(Uuid, &[(String, i32)])>,
) -> Vec<Opening> {
    let mut out = Vec::new();
    for (kind, station_id) in stations {
        if job_now.crafts_at().contains(&kind.as_str()) {
            continue;
        }
        let goods = hired_for(kind);
        let best = catalog
            .recipes()
            .iter()
            .filter(|r| r.tool_type.is_none())
            .filter(|r| {
                let Some(out) = r.outputs.first() else { return false };
                match r.required_station_type.as_deref() {
                    Some(at) => at == kind && goods.contains(&out.item_type.as_str()),
                    // Handwork at the bench: rope from a fiber gatherer's
                    // fiber, which the court takes at any station
                    None => kind == "workbench" && ROLES.iter().any(|role| role.answers().contains(&out.item_type.as_str())),
                }
            })
            .filter_map(|r| {
                let crafts = r
                    .inputs
                    .iter()
                    .map(|i| spare.get(&i.item_type).copied().unwrap_or(0) / i.quantity.max(1))
                    .min()?;
                let out = r.outputs.first()?;
                let most = crafts * out.quantity_min.max(1);
                let worth = most as i64 * catalog.target_price(&out.item_type).unwrap_or(1);
                (most >= LEAST_HIRE).then_some((worth, r, most))
            })
            .max_by_key(|(worth, ..)| *worth);
        if let Some((_, r, most)) = best {
            let work = Work::Craft {
                station_id: *station_id,
                recipe_id: r.id.clone(),
            };
            let opening = Opening {
                work,
                most,
                stock: Vec::new(),
            };
            let stock = opening.stock_for(catalog, most);
            out.push(Opening { stock, ..opening });
        }
    }
    if let Some((plot_id, resources)) = lot {
        for (resource, nodes) in resources {
            let Some(r) = catalog.resource_type(resource) else { continue };
            let gathered = Job::Gather { yields: &r.yields_item };
            if !ROLES.iter().any(|role| role.does(&gathered)) || job_now.does(&gathered) {
                continue;
            }
            let most = gather_most(r, *nodes);
            if most < LEAST_HIRE {
                continue;
            }
            out.push(Opening {
                work: Work::Gather {
                    plot_id,
                    resource: resource.clone(),
                },
                most,
                stock: Vec::new(),
            });
        }
    }
    out
}

/// Game seconds a worker on a gathering shift takes between nodes, besides
/// the gather itself: a short walk across the lot.
const NODE_WALK_SECS: f64 = 10.0;

/// What `nodes` of `r` on a lot give a worker in a game hour, up to
/// [`GATHER_CAP`]: 0 unless going round them keeps it busy at least half
/// the time, since a node gathered is spent until it grows back, and a
/// worker who finds none ready three times gives the shift up. A few grass
/// tufts keep one busy; a lone vein, spent for ten minutes after each
/// pull, doesn't.
pub fn gather_most(r: &CatalogResourceType, nodes: i32) -> i32 {
    let regrow = secs(r.regeneration_ticks);
    let round = nodes.max(0) as f64 * (secs(r.extraction_duration_ticks) + NODE_WALK_SECS);
    if nodes <= 0 || round < regrow / 2.0 {
        return 0;
    }
    let pulls = (3600.0 / regrow.max(round)) * nodes as f64;
    ((pulls * gathered(r)).floor() as i32).min(GATHER_CAP)
}

/// A citizen's line (K2.4): the Strands it keeps back before it hires, the
/// town's `base` times 0.5 for the free-handed to 1.5 for the thrifty.
pub fn hire_line(base: i64, thrift: f32) -> i64 {
    (base as f64 * (0.5 + thrift as f64)).round() as i64
}

/// What a unit of `work` earns the trade that does it, at the books
/// (`worth`): a craft's goods less its inputs, a unit at the recipe's
/// least yield, or a gather's good. `None` when the books can't say. What
/// an employer pays to leave a worker no worse off than at its own trade
/// ([`earns`] counts the same).
pub fn fair(catalog: &Catalog, work: &Work, worth: impl Fn(&str) -> Option<i64>) -> Option<i64> {
    match work {
        Work::Craft { recipe_id, .. } => {
            let r = recipe(catalog, recipe_id)?;
            let out = r.outputs.first()?;
            let made = out.quantity_min.max(1) as i64;
            let mut net = made * worth(&out.item_type)?;
            for input in &r.inputs {
                net -= input.quantity as i64 * worth(&input.item_type)?;
            }
            // A unit's share, rounded up: a Strand short is a worker lost
            Some((net.max(0) + made - 1) / made)
        }
        Work::Gather { resource, .. } => worth(&catalog.resource_type(resource)?.yields_item),
    }
}

/// The rate a citizen offers (K2.4): what the work earns its trade at the
/// books ([`fair`]), else the Flint form's [`rate_for`](crate::wages::rate_for)
/// (half the good's target), times 1.2 for the free-handed to 0.8 for the
/// thrifty, rounded, and at least a Strand. At middling thrift a
/// middling worker takes it ([`rules::wage`](crate::citizen::decide::rules)).
pub fn hire_rate(rate_for: i64, fair: Option<i64>, t: &Temperament) -> i64 {
    let base = fair.filter(|f| *f > 0).unwrap_or(rate_for).max(1);
    ((base as f64 * (1.2 - 0.4 * t.thrift as f64)).round() as i64).max(1)
}

/// How many units, up to `most`, an offer at `rate` can escrow with the
/// court's fee from `budget`.
pub fn affordable(rate: i64, most: i32, budget: i64) -> i32 {
    let costs = |n: i32| {
        let value = offer_value(rate, n);
        value + court_fee(value)
    };
    if most <= 0 || rate <= 0 || costs(1) > budget {
        return 0;
    }
    // The cost rises with the units: the most that fits
    let (mut lo, mut hi) = (1, most);
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if costs(mid) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// The question and menu for hiring for `opening` (K2.4), if there's one
/// to ask: the citizen holds Strands over its `line`, enough to escrow
/// [`LEAST_HIRE`] units (or all there are) at the rate its books say is
/// fair, and [`LEAST_HIRE_VALUE`] worth of them. Offer the work (a rate up to twice that, up to `opening.most`
/// units), or pass. The rules take the rate and the units from what the
/// question carries, so a replay offers the same.
pub fn hire_menu(
    opening: &Opening,
    catalog: &Catalog,
    holdings: &Holdings,
    line: i64,
    worth: impl Fn(&str) -> Option<i64>,
) -> Option<(Question, Vec<Choice>)> {
    let rate_for = crate::wages::rate_for(&opening.work, catalog);
    let fair = fair(catalog, &opening.work, worth);
    let base = fair.filter(|f| *f > 0).unwrap_or(rate_for).max(1);
    let budget = holdings.strands - line;
    let n = affordable(base, opening.most, budget);
    if n < LEAST_HIRE.min(opening.most) || offer_value(base, n) < LEAST_HIRE_VALUE {
        return None;
    }
    let question = Question::Hire {
        work: words(catalog, &opening.work),
        rate_for,
        fair,
        line,
    };
    let menu = vec![
        Choice::Hire {
            rate: Span::new(1, base * 2),
            cap: Span::new(1, opening.most),
        },
        Choice::Pass,
    ];
    Some((question, menu))
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

    /// The walk to work is spread over the shift (#507): far-off work pays
    /// less an hour than the same work near, a small offer far off least of
    /// all, and a walk the day can't fit work after pays nothing.
    #[test]
    fn far_off_work_pays_less_an_hour_than_near() {
        let catalog = Catalog::fallback();
        let plank = craft("plank");
        let at_bench = pays(&catalog, &plank, 2).unwrap();
        let walked = |left, walk, secs| pays_walked(&catalog, &plank, 2, left, walk, secs).unwrap();
        // No walk: what the bench pays
        assert_eq!(walked(40, 0.0, None), at_bench);
        // Planks at 1800 an hour: 40 of them are 80 s at the bench, so 80 s
        // of walking halves the hour
        assert_eq!(walked(40, 80.0, None), at_bench / 2);
        // The farther, the less; and the same walk costs a small offer more
        let (near, far) = (walked(40, 60.0, None), walked(40, 600.0, None));
        assert!(at_bench > near && near > far, "{at_bench} {near} {far}");
        assert!(walked(10, 600.0, None) < far);
        // A small far offer at a better rate loses to a near one at less
        let small_far = pays_walked(&catalog, &plank, 3, 20, 600.0, None).unwrap();
        let near_cheap = pays_walked(&catalog, &plank, 2, 200, 30.0, None).unwrap();
        assert!(small_far < near_cheap, "{small_far} {near_cheap}");
        // A big offer counts only what the time left makes: an hour's
        // work after a walk is an hour's, not the whole cap's
        assert_eq!(walked(10_000, 600.0, Some(600.0 + 3600.0)), walked(1800, 600.0, None));
        // No time to work once there, or nothing left: no pay to weigh
        assert_eq!(pays_walked(&catalog, &plank, 2, 40, 600.0, Some(600.0)), None);
        assert_eq!(pays_walked(&catalog, &plank, 2, 0, 0.0, None), None);
        assert_eq!(pays_walked(&catalog, &gather("no_such"), 2, 40, 0.0, None), None);
        // A gather the same: grass at 2400 an hour, 40 tufts a minute's
        // work, and a minute's walk halves it
        let grass = pays(&catalog, &gather("grass"), 1).unwrap();
        assert_eq!(pays_walked(&catalog, &gather("grass"), 1, 40, 60.0, None), Some(grass / 2));
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

    fn spare(items: &[(&str, i32)]) -> HashMap<String, i32> {
        items.iter().map(|(i, n)| (i.to_string(), *n)).collect()
    }

    #[test]
    fn a_citizen_hires_out_what_its_stint_leaves_idle() {
        let catalog = Catalog::fallback();
        let (saw, kiln, forge) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let lot = Uuid::from_u128(9);
        let stations = vec![("sawbench".to_string(), saw), ("kiln".to_string(), kiln), ("forge".to_string(), forge)];
        let grass = [("grass".to_string(), 4), ("no_such".to_string(), 3), ("iron_vein".to_string(), 1)];
        let held = spare(&[("log", 5), ("fiber", 40)]);
        // On a fiber stint: the sawbench saws its logs (15 planks beats 10
        // charcoal), the forge has no ore, and the lot's grass is the
        // stint's own
        let fiber = openings(&catalog, Role::Gatherer(Gather::Fiber), &stations, &held, Some((lot, &grass)));
        assert_eq!(fiber.len(), 2, "{fiber:?}");
        assert_eq!(
            fiber[0].work,
            Work::Craft {
                station_id: saw,
                recipe_id: "plank".into()
            }
        );
        assert_eq!((fiber[0].most, fiber[0].stock.clone()), (15, vec![("log".to_string(), 5)]));
        assert_eq!(
            fiber[1].work,
            Work::Craft {
                station_id: kiln,
                recipe_id: "charcoal".into()
            }
        );
        assert!(!fiber.iter().any(|o| matches!(o.work, Work::Gather { .. })));
        // On a planks stint the sawbench is its own, and the grass is
        // anyone's to gather for it
        let planks = openings(&catalog, Role::Crafter(Craft::Planks), &stations, &held, Some((lot, &grass)));
        assert!(planks.iter().all(|o| crate::wages::place_of(&o.work) != saw), "{planks:?}");
        assert!(planks.iter().any(|o| o.work
            == Work::Gather {
                plot_id: lot,
                resource: "grass".into()
            }
            && o.most == GATHER_CAP));
        // At a workbench, rope twisted from its fiber
        let bench = vec![("workbench".to_string(), Uuid::from_u128(4))];
        let rope = openings(&catalog, Role::Gatherer(Gather::Wood), &bench, &held, None);
        assert_eq!(rope.len(), 1, "{rope:?}");
        assert_eq!((rope[0].most, rope[0].stock.clone()), (20, vec![("fiber".to_string(), 40)]));
        assert!(matches!(&rope[0].work, Work::Craft { recipe_id, .. } if recipe_id == "rope_basic"));
        // A lone vein, spent ten minutes after each pull, keeps nobody busy
        assert!(!planks.iter().any(|o| matches!(&o.work, Work::Gather { resource, .. } if resource == "iron_vein")));
        let vein = catalog.resource_type("iron_vein").unwrap();
        assert_eq!(gather_most(vein, 1), 0);
        assert!(gather_most(vein, 40) >= LEAST_HIRE);
        let tufts = catalog.resource_type("grass").unwrap();
        assert_eq!((gather_most(tufts, 0), gather_most(tufts, 1)), (0, 0));
        assert_eq!(gather_most(tufts, 4), GATHER_CAP);
        // Too few logs for a hire's worth: nothing at the bench
        let few = openings(&catalog, Role::Gatherer(Gather::Fiber), &stations, &spare(&[("log", 1)]), None);
        assert!(few.is_empty(), "{few:?}");
        // What it wants back follows the same line
        let saw_planks = Work::Craft {
            station_id: saw,
            recipe_id: "plank".into(),
        };
        assert!(wants_back(Role::Crafter(Craft::Planks), &catalog, &saw_planks));
        assert!(!wants_back(Role::Gatherer(Gather::Wood), &catalog, &saw_planks));
        let on_lot = gather("grass");
        assert!(wants_back(Role::Gatherer(Gather::Fiber), &catalog, &on_lot));
        assert!(!wants_back(Role::Builder, &catalog, &on_lot));
    }

    #[test]
    fn an_offer_pays_what_the_work_earns_its_trade_by_thrift_within_the_line() {
        let catalog = Catalog::fallback();
        let targets = |item: &str| catalog.target_price(item);
        let plank = craft("plank");
        let fair_plank = fair(&catalog, &plank, targets).unwrap();
        // Three planks from a log: what they fetch over the log, a plank's share
        let (p, l) = (catalog.target_price("plank").unwrap(), catalog.target_price("log").unwrap());
        assert_eq!(fair_plank, (3 * p - l + 2) / 3);
        assert_eq!(fair(&catalog, &gather("grass"), |_| None), None);
        let middling = Temperament::default();
        let thrifty = Temperament {
            thrift: 1.0,
            ..middling
        };
        let free = Temperament {
            thrift: 0.0,
            ..middling
        };
        assert_eq!(hire_rate(1, Some(fair_plank), &middling), fair_plank);
        assert!(hire_rate(1, Some(fair_plank), &thrifty) < fair_plank);
        assert!(hire_rate(1, Some(fair_plank), &free) > fair_plank);
        // With no books, the form's half the target; never under a Strand
        assert_eq!(hire_rate(7, None, &middling), 7);
        assert_eq!(hire_rate(1, None, &thrifty), 1);
        // The line by thrift
        assert_eq!((hire_line(600, 0.0), hire_line(600, 0.5), hire_line(600, 1.0)), (300, 600, 900));
        // What a budget escrows, the court's fee and all
        let n = affordable(3, 100, 200);
        let cost = |n: i32| offer_value(3, n) + court_fee(offer_value(3, n));
        assert!(cost(n) <= 200 && cost(n + 1) > 200, "{n}");
        assert_eq!(affordable(3, 10, 10_000), 10);
        assert_eq!(affordable(3, 10, 0), 0);
        assert_eq!(affordable(3, 10, -50), 0);
    }

    /// The question is asked only over the line, with enough to escrow a
    /// hire's worth at the fair rate: so nobody hires while broke.
    #[test]
    fn only_a_citizen_over_its_line_is_asked_to_hire() {
        let catalog = Catalog::fallback();
        let opening = Opening {
            work: craft("plank"),
            most: 30,
            stock: vec![("log".into(), 10)],
        };
        let books = |item: &str| catalog.target_price(item);
        let holding = |strands: i64| Holdings {
            strands,
            ..Holdings::default()
        };
        assert!(hire_menu(&opening, &catalog, &holding(0), 600, books).is_none());
        assert!(hire_menu(&opening, &catalog, &holding(600), 600, books).is_none());
        // A few Strands over isn't a hire's worth, nor are a few units
        assert!(hire_menu(&opening, &catalog, &holding(605), 600, books).is_none());
        let few = Opening { most: 6, ..opening.clone() };
        assert!(hire_menu(&few, &catalog, &holding(900), 600, books).is_none());
        let (question, menu) = hire_menu(&opening, &catalog, &holding(900), 600, books).unwrap();
        let fair_plank = fair(&catalog, &opening.work, books).unwrap();
        assert_eq!(
            question,
            Question::Hire {
                work: "craft plank at a sawbench".into(),
                rate_for: crate::wages::rate_for(&opening.work, &catalog),
                fair: Some(fair_plank),
                line: 600,
            }
        );
        assert_eq!(
            menu,
            vec![
                Choice::Hire {
                    rate: Span::new(1, fair_plank * 2),
                    cap: Span::new(1, 30)
                },
                Choice::Pass
            ]
        );
        // What a cap of planks takes from the pack
        assert_eq!(opening.stock_for(&catalog, 7), vec![("log".to_string(), 3)]);
        assert!(Opening {
            work: gather("grass"),
            most: GATHER_CAP,
            stock: Vec::new()
        }
        .stock_for(&catalog, 10)
        .is_empty());
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
