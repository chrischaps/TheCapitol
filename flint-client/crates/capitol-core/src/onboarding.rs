//! The road in, as the client shows it (#441, `the-capitol-road-in.md`).
//!
//! The server keeps the tour and judges each step from what the player did
//! (#438, #439), so a [`Tour`] is only a view of the `tutorial` and
//! `tutorial_step` it sent (`GameState::tutorial`): which step is at hand,
//! where it sits in its chapter, and whether the card is hidden. Nothing is
//! kept on this machine, and nothing is judged from what the pack holds:
//! every account walks the road (plan D3).
//!
//! A few steps are only things on screen, which the server can't see:
//! opening the pack, reading a zone card. The client says it saw them
//! ([`Did`], sent as `tutorial_seen`), and the server takes that only for
//! the steps `protocol::tutorial` lets the client report (plan D7).
//!
//! Hiding the card is for now, never for good (plan D2): it's `dismissed`
//! on the server, so it holds on every device, and the tour waits.
//!
//! A step can lead you somewhere ([`Target`], #442): the card points the
//! way and a gold ring marks the place. A step names a kind of place, never
//! one place, so the client finds it from what it holds (the nearest live
//! node of a kind, [`nearest_node`]) and a regenerated world can't strand
//! it. The workbench's place is found from the ground near you
//! ([`clear_spot`], #443).
//!
//! Arrival's quality card (#443) reads its numbers from the player's own
//! pack ([`QualityLesson`]). Settle in's cards (#449) read theirs from the
//! player's own lot ([`Home`]): what it fits, and what it's taxed.

use capitol_protocol::rest::{PlotInfo, RecipeInfo, ResourceNodeInfo, SlotItem, StationInfo};
use capitol_protocol::rules::{self, quality, Zone};
use capitol_protocol::terrain::{quad_contains, TerrainResponse};
use capitol_protocol::tutorial::{Chapter, Step, TutorialProgress};
use capitol_protocol::ws::PlayerPosition;
use uuid::Uuid;

/// Something only the screen shows, which the server takes for a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Did {
    /// Opened or closed the pack, or moved something in it.
    UsedInventory,
    /// Opened the crafting panel, which is past looking in the pack.
    OpenedCrafting,
    /// Opened a zone card (#444).
    ReadZoneCard,
    /// Looked in at the Deeds Office (#448).
    OpenedOffice,
    /// Read the tax card, and said so with its "Noted" (#449).
    ReadTax,
}

impl Did {
    /// The step it counts for.
    pub fn step(self) -> Step {
        match self {
            Did::UsedInventory | Did::OpenedCrafting => Step::OpenPack,
            Did::ReadZoneCard => Step::ReadZoneCard,
            Did::OpenedOffice => Step::OpenOffice,
            Did::ReadTax => Step::ReadTax,
        }
    }
}

/// Where one account is on the road in, as the server last said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tour {
    progress: TutorialProgress,
}

impl From<TutorialProgress> for Tour {
    fn from(progress: TutorialProgress) -> Tour {
        Tour { progress }
    }
}

impl Tour {
    pub fn progress(self) -> TutorialProgress {
        self.progress
    }

    /// The step at hand: the first not done, in the tour's order. Done out
    /// of order still counts. `None` once the tour is finished. Hidden or
    /// not, this is where the player is.
    pub fn current(self) -> Option<Step> {
        self.progress.done.current()
    }

    pub fn done(self, step: Step) -> bool {
        self.progress.done.has(step)
    }

    pub fn finished(self) -> bool {
        self.progress.finished()
    }

    /// Every step of `chapter` is done, in whatever order.
    pub fn chapter_done(self, chapter: Chapter) -> bool {
        self.progress.done.chapter_done(chapter)
    }

    /// The card is hidden for now, and there's still a road to walk.
    pub fn hidden(self) -> bool {
        self.progress.dismissed && !self.finished()
    }

    /// The step to tell the server `did` showed, if it isn't done yet.
    pub fn report(self, did: Did) -> Option<Step> {
        let step = did.step();
        (!self.done(step)).then_some(step)
    }
}

/// Where a step's card leads you (#442).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// A point on the ground, server units.
    Point([f64; 2]),
    /// The nearest live node of one of these resource types ([`nearest_node`]).
    Node(&'static [&'static str]),
    /// The road in's stop in this zone (#445): the lot
    /// `protocol::tutorial::stops::stop` picks by the south arterial, free
    /// if one near is, so a regenerated world moves it with its lots.
    Lot(Zone),
    /// The Exchange hall's door.
    Hall,
    /// The nearest stall in the bazaar.
    Stall,
    /// The notice board by the hall's door, where the Gazette is pinned (#446).
    Board,
    /// Someone else: the nearest person or citizen in sight
    /// ([`nearest_person`], #447). Nobody near, it leads nowhere.
    Person,
    /// The Deeds Office's desk, in the hall's forecourt (#448).
    Office,
    /// Open ground near you where a workbench may stand (#443): off every
    /// lot, on dry ground, clear of every station, and beside the road
    /// rather than on it ([`clear_spot`]). The extension keeps the spot it
    /// found while it stays clear, so the ring doesn't follow you about.
    ClearSpot,
    /// Your home (#449): the lot your deed claimed ([`home_plot`]), its
    /// edge traced.
    Home,
    /// A station of yours that can be set to work (#449): a crafting
    /// station with a box, the one on your home lot first, else the
    /// nearest ([`my_station`]).
    MyStation,
}

/// Where `step` leads, if anywhere. A gather leads to the nodes the server
/// judges it by (`engine::tutorial`: fiber, sticks, stone, a log): the
/// kinds that grow round the spawn, so not the wilds' old growth, which a
/// stone axe can't fell. Each chapter's work package adds its own (#443 to
/// #449): the workbench's clear spot, the road's stops, the Deeds Office.
pub fn target(step: Step) -> Option<Target> {
    Some(match step {
        Step::PlaceWorkbench => Target::ClearSpot,
        Step::GatherGrass => Target::Node(&["grass"]),
        Step::GatherBranches => Target::Node(&["fallen_branches"]),
        Step::GatherStone => Target::Node(&["rock_outcrop"]),
        Step::FellTree => Target::Node(&["tree"]),
        Step::SeeRuralLot => Target::Lot(Zone::Rural),
        Step::SeeSuburbanLot => Target::Lot(Zone::Suburban),
        Step::SeeUrbanLot => Target::Lot(Zone::Urban),
        // Left till after the three lots, you're at the last stop
        Step::ReadZoneCard => Target::Lot(Zone::Urban),
        Step::Deposit | Step::SellOrder | Step::BuyOrder | Step::CollectBox => Target::Hall,
        Step::VisitStall => Target::Stall,
        Step::ReadGazette => Target::Board,
        Step::OpenCard => Target::Person,
        // The claim leads to the lot the office showed you, if any: the
        // client's to say (#448)
        Step::OpenOffice => Target::Office,
        // Settle in (#449): your own land, then a station of yours on it.
        // The tax card asks nothing of the world, so it leads nowhere.
        Step::PlaceAtHome => Target::Home,
        Step::SetAutomation => Target::MyStation,
        _ => return None,
    })
}

/// The nearest node to `from` (server units) that's one of `kinds` and
/// there to take: not spent, and not being worked by someone else.
pub fn nearest_node<'a>(
    nodes: impl IntoIterator<Item = &'a ResourceNodeInfo>,
    kinds: &[&str],
    from: [f64; 2],
) -> Option<&'a ResourceNodeInfo> {
    nodes
        .into_iter()
        .filter(|n| n.state == "available" && kinds.contains(&n.resource_type.as_str()))
        .min_by(|a, b| {
            let d = |n: &ResourceNodeInfo| (n.x - from[0]).powi(2) + (n.y - from[1]).powi(2);
            d(a).total_cmp(&d(b))
        })
}

/// The nearest player to `from` (server units) who isn't `me`: a person or
/// a citizen, whoever's card there is to open.
pub fn nearest_person<'a>(
    players: impl IntoIterator<Item = &'a PlayerPosition>,
    me: Option<Uuid>,
    from: [f64; 2],
) -> Option<&'a PlayerPosition> {
    players
        .into_iter()
        .filter(|p| Some(p.id) != me)
        .min_by(|a, b| {
            let d = |p: &PlayerPosition| (p.x - from[0]).powi(2) + (p.y - from[1]).powi(2);
            d(a).total_cmp(&d(b))
        })
}

/// Your home, as Settle in's cards say it (#449): the lot [`home_plot`]
/// picks, what it fits, and what it's taxed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    pub plot_id: Uuid,
    pub zone: Zone,
    /// The stations it fits on its own, empty, at the packing #444's zone
    /// card counts by (`sim::lot::layout`, which the server's
    /// `station_room` copies), capped by its capacity
    pub fits: usize,
    /// What its deed allows (`station_capacity`)
    pub capacity: i32,
    /// The stations on it now
    pub placed: i32,
    /// What it's assessed at: what the deed cost, or the zone's price for a
    /// granted one (plan D4)
    pub assessed: i32,
    /// A week's tax, at the live rate; `None` where it's untaxed
    pub weekly_tax: Option<i64>,
    /// Tax owed and unpaid
    pub owed: i64,
}

impl Home {
    /// `plot` as a home. `before` is the last one worked out: on the same
    /// lot with the same capacity, its room is kept rather than packed
    /// again, since it's asked every frame Settle in is open. `None` for a
    /// lot outside the home zones.
    pub fn of(plot: &PlotInfo, before: Option<&Home>) -> Option<Home> {
        let zone = Zone::from_str(&plot.zone_id).filter(|z| crate::deeds::HOME_ZONES.contains(z))?;
        let fits = match before {
            Some(b) if b.plot_id == plot.id && b.capacity == plot.station_capacity => b.fits,
            _ => crate::sim::lot::layout(crate::sim::lot::corners(plot), plot.station_capacity.max(0) as usize).len(),
        };
        Some(Home {
            plot_id: plot.id,
            zone,
            fits,
            capacity: plot.station_capacity,
            placed: plot.station_count,
            assessed: plot.assessed_value,
            weekly_tax: plot.forecloses_at.filter(|f| *f > 0).map(|f| f / rules::FORECLOSE_AFTER_WEEKS),
            owed: plot.tax_owed,
        })
    }

    /// The debt that forecloses it: four weeks of its tax.
    pub fn forecloses_at(&self) -> Option<i64> {
        self.weekly_tax.map(|w| w * rules::FORECLOSE_AFTER_WEEKS)
    }
}

/// Your home among your `plots`: a lot in the rural, suburban or urban
/// rings, the one claimed last if you hold more than one (a deed bought
/// before the grant was spent, say: plan D6). Ties go by id.
pub fn home_plot(plots: &[PlotInfo]) -> Option<&PlotInfo> {
    plots
        .iter()
        .filter(|p| p.owner_id.is_some())
        .filter(|p| Zone::from_str(&p.zone_id).is_some_and(|z| crate::deeds::HOME_ZONES.contains(&z)))
        .max_by(|a, b| a.claimed_at.cmp(&b.claimed_at).then_with(|| b.id.cmp(&a.id)))
}

/// A station of `me`'s that can be set to work (E4.2's rule: a crafting
/// station with a box), for the automation step: one on `home` first, the
/// nearest to `from` (server units) among those, else the nearest anywhere,
/// such as Arrival's workbench by the road.
pub fn my_station<'a>(
    stations: impl IntoIterator<Item = &'a StationInfo>,
    me: Option<Uuid>,
    home: Option<&PlotInfo>,
    from: [f64; 2],
) -> Option<&'a StationInfo> {
    let me = me?;
    let at_home = |s: &StationInfo| home.is_some_and(|h| h.contains(s.x, s.y));
    let d = |s: &StationInfo| (s.x - from[0]).powi(2) + (s.y - from[1]).powi(2);
    stations
        .into_iter()
        .filter(|s| s.owner_id == me && s.category == "crafting" && s.container_id.is_some())
        .min_by(|a, b| at_home(b).cmp(&at_home(a)).then(d(a).total_cmp(&d(b))))
}

/// How far from you [`clear_spot`] looks, in server units: inside the 75 a
/// station goes down from, so you can place it from where you stand.
pub const SPOT_REACH: f64 = 60.0;

/// How far apart [`clear_spot`] tries spots, along each ring and between
/// rings.
const SPOT_STEP: f64 = 5.0;

/// How near you [`clear_spot`] looks first: a step or so off, so the ring
/// lies beside you rather than under your feet, and the bench doesn't go
/// down on top of you.
pub const SPOT_NEAR: f64 = 15.0;

/// The room a clear spot keeps from a lot's edge: a bench's footprint (a
/// ring of 1.45 m, 14.5 units, `stations::footprint`) and a little, so the
/// whole bench stands off the lot, not only its middle.
pub const SPOT_LOT_GAP: f64 = 16.0;

/// The room it keeps from a road's edge, where one would do: the same, so
/// the bench stands beside the road, not half on it.
pub const SPOT_ROAD_GAP: f64 = 16.0;

/// The room it keeps past the stations' least spacing (50), so a station
/// nudged a little since doesn't make it too close.
pub const SPOT_STATION_GAP: f64 = 2.0;

/// The ground round one place, gathered once to test many spots on it:
/// the lots, roads and stations near it.
pub struct Ground<'a> {
    terrain: &'a TerrainResponse,
    lots: Vec<&'a [(f64, f64); 4]>,
    /// Each road segment near, and the road's half width.
    roads: Vec<([f64; 2], [f64; 2], f64)>,
    stations: Vec<[f64; 2]>,
}

impl<'a> Ground<'a> {
    /// The ground within `reach` of `from`, from the terrain and the
    /// stations the client knows.
    pub fn near(
        from: [f64; 2],
        reach: f64,
        terrain: &'a TerrainResponse,
        stations: impl IntoIterator<Item = &'a StationInfo>,
    ) -> Self {
        let near = |p: [f64; 2], pad: f64| (p[0] - from[0]).hypot(p[1] - from[1]) <= reach + pad;
        let lots = terrain
            .terrain_plots
            .iter()
            .filter(|lot| {
                // A quad's far corner can be a long way off: a farm is about
                // 400 long, so take any lot whose box comes within reach.
                let xs = lot.corners.iter().map(|c| c.0);
                let ys = lot.corners.iter().map(|c| c.1);
                let (x0, x1) = xs.fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(x), b.max(x)));
                let (y0, y1) = ys.fold((f64::MAX, f64::MIN), |(a, b), y| (a.min(y), b.max(y)));
                let gap = SPOT_LOT_GAP + reach;
                from[0] >= x0 - gap && from[0] <= x1 + gap && from[1] >= y0 - gap && from[1] <= y1 + gap
            })
            .map(|lot| &lot.corners)
            .collect();
        let roads = terrain
            .roads
            .iter()
            .flat_map(|road| {
                road.points.windows(2).map(move |w| ([w[0].0, w[0].1], [w[1].0, w[1].1], road.width / 2.0))
            })
            .filter(|&(a, b, half)| to_segment(from, a, b) <= reach + half + SPOT_ROAD_GAP)
            .collect();
        let stations = stations
            .into_iter()
            .map(|s| [s.x, s.y])
            .filter(|&p| near(p, rules::STATION_MIN_SPACING + SPOT_STATION_GAP))
            .collect();
        Ground { terrain, lots, roads, stations }
    }

    /// Whether a workbench may stand at `at`, with room to spare: off every
    /// lot by [`SPOT_LOT_GAP`], out of the water, and clear of every
    /// station by the least spacing and [`SPOT_STATION_GAP`]. The server's
    /// own tests (`station::check`) are these without the room.
    pub fn clear(&self, at: [f64; 2]) -> bool {
        let [x, y] = at;
        let off_lots = self.lots.iter().all(|corners| {
            !quad_contains(corners, x, y)
                && (0..4).all(|i| {
                    let (a, b) = (corners[i], corners[(i + 1) % 4]);
                    to_segment(at, [a.0, a.1], [b.0, b.1]) >= SPOT_LOT_GAP
                })
        });
        let spaced = self
            .stations
            .iter()
            .all(|s| (s[0] - x).hypot(s[1] - y) >= rules::STATION_MIN_SPACING + SPOT_STATION_GAP);
        let centre = (self.terrain.center_x, self.terrain.center_y);
        off_lots
            && spaced
            && rules::in_water(centre, &self.terrain.water_rings, (x, y)).is_none()
            && rules::ground::water_at(self.terrain, x, y).is_none()
    }

    /// Whether `at` is on a road, or within [`SPOT_ROAD_GAP`] of its edge.
    pub fn on_road(&self, at: [f64; 2]) -> bool {
        self.roads.iter().any(|&(a, b, half)| to_segment(at, a, b) < half + SPOT_ROAD_GAP)
    }
}

/// The nearest spot to `from` (server units) where a workbench may stand,
/// [`SPOT_NEAR`] to [`SPOT_REACH`] off: off every lot, on dry ground and clear of every
/// station (`Ground::clear`), and off the road where any such spot is
/// (`Ground::on_road`), so it's beside the road rather than on it. `None`
/// when there's nowhere: a farm's middle, say, where every spot near is
/// the farm's.
pub fn clear_spot<'a>(
    from: [f64; 2],
    terrain: &'a TerrainResponse,
    stations: impl IntoIterator<Item = &'a StationInfo>,
) -> Option<[f64; 2]> {
    let ground = Ground::near(from, SPOT_REACH, terrain, stations);
    let rings = (SPOT_REACH / SPOT_STEP) as usize;
    let spots = ((SPOT_NEAR / SPOT_STEP) as usize..=rings).flat_map(|ring| {
        let r = ring as f64 * SPOT_STEP;
        let n = ((std::f64::consts::TAU * r / SPOT_STEP).ceil() as usize).max(1);
        (0..n).map(move |k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            [from[0] + r * a.cos(), from[1] + r * a.sin()]
        })
    });
    let mut on_road = None;
    for at in spots {
        let road = ground.on_road(at);
        if road && on_road.is_some() {
            continue;
        }
        if !ground.clear(at) {
            continue;
        }
        if !road {
            return Some(at);
        }
        on_road = Some(at);
    }
    on_road
}

/// The distance from `p` to the segment `a`–`b`.
fn to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) };
    (a[0] + t * dx - p[0]).hypot(a[1] + t * dy - p[1])
}

/// The quality card's numbers (#443, plan §4.1), from the player's own
/// pack: the rope they twisted, and what their best and worst fiber would
/// twist to, by the server's own cascade (`rules::quality`) over
/// `rope_basic`'s weights, as the crafting panel's preview reckons it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityLesson {
    /// Each quality the ropes held come in at, best first, at most two.
    pub ropes: Vec<i32>,
    /// The worst and the best unit of fiber held, if any is.
    pub fiber: Option<(i32, i32)>,
    /// The range two of the worst would twist to, then two of the best:
    /// what's left to the roll. `None` without fiber, or the recipe.
    pub twists: Option<((i32, i32), (i32, i32))>,
}

impl QualityLesson {
    /// The numbers from `items` (the pack, and crafting's inputs and its
    /// output, where a rope just twisted waits), for the rope `recipe`.
    /// `None` while there's no rope to talk about.
    pub fn of<'a>(items: impl IntoIterator<Item = &'a SlotItem>, recipe: Option<&RecipeInfo>) -> Option<Self> {
        let mut ropes: Vec<i32> = Vec::new();
        let mut fiber: Option<(i32, i32)> = None;
        for item in items {
            match item.item_type.as_str() {
                "rope" => ropes.extend(item.units.pairs().map(|(q, _)| q)),
                "fiber" => {
                    if let (Some(lo), Some(hi)) = (item.units.min(), item.units.max()) {
                        fiber = Some(fiber.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
                    }
                }
                _ => {}
            }
        }
        ropes.sort_unstable_by(|a, b| b.cmp(a));
        ropes.dedup();
        ropes.truncate(2);
        if ropes.is_empty() {
            return None;
        }
        let twists = fiber.zip(recipe).and_then(|((lo, hi), recipe)| {
            let input = recipe.inputs.first()?;
            let twist = |q: i32| {
                let mut inputs = quality::Inputs::default();
                inputs.add(&input.item_type, input.quantity, q);
                quality::output_range(&inputs, quality::NO_STATION, 0, &recipe.quality_weights)
            };
            Some((twist(lo), twist(hi)))
        });
        Some(QualityLesson { ropes, fiber, twists })
    }
}

/// Where `step` sits in its chapter: its place from 0, and how many steps
/// the chapter has. A chapter's pips are drawn from these.
pub fn place_in_chapter(step: Step) -> (usize, usize) {
    let chapter = step.chapter();
    let at = chapter.steps().position(|s| s == step).unwrap_or(0);
    (at, chapter.steps().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::tutorial::{Chapter, Steps};

    fn tour(done: &[Step], dismissed: bool) -> Tour {
        Tour::from(TutorialProgress { done: done.iter().copied().collect(), dismissed, ..Default::default() })
    }

    #[test]
    fn a_new_account_starts_at_the_first_step() {
        let new = Tour::default();
        assert_eq!(new.current(), Some(Step::Walk));
        assert!(!new.finished() && !new.hidden());
    }

    #[test]
    fn the_step_at_hand_is_the_first_not_done_whatever_the_order() {
        // Clicking grass from afar walks there and gathers: both count.
        let ran_ahead = tour(&[Step::GatherGrass, Step::Deposit], false);
        assert_eq!(ran_ahead.current(), Some(Step::Walk));
        let half = tour(&[Step::Walk, Step::GatherGrass], false);
        assert_eq!(half.current(), Some(Step::OpenPack));
    }

    #[test]
    fn hidden_is_for_now_and_keeps_the_place() {
        let hidden = tour(&[Step::Walk], true);
        assert!(hidden.hidden());
        assert_eq!(hidden.current(), Some(Step::GatherGrass), "the tour waits");
        // Finished, there's nothing to hide
        let done = Tour::from(TutorialProgress { done: Steps::every(), dismissed: true, ..Default::default() });
        assert!(done.finished() && !done.hidden());
        assert_eq!(done.current(), None);
    }

    #[test]
    fn only_what_isnt_done_is_reported() {
        let new = Tour::default();
        assert_eq!(new.report(Did::UsedInventory), Some(Step::OpenPack));
        assert_eq!(new.report(Did::OpenedCrafting), Some(Step::OpenPack));
        assert_eq!(new.report(Did::ReadZoneCard), Some(Step::ReadZoneCard));
        let looked = tour(&[Step::OpenPack], false);
        assert_eq!(looked.report(Did::UsedInventory), None);
    }

    /// The server takes `tutorial_seen` only for steps it can't see, so
    /// the client never reports one it judges
    #[test]
    fn every_report_is_one_the_server_takes() {
        for did in [Did::UsedInventory, Did::OpenedCrafting, Did::ReadZoneCard, Did::OpenedOffice, Did::ReadTax] {
            assert!(did.step().judge().client_reports(), "{did:?}");
        }
    }

    #[test]
    fn a_chapter_is_done_when_its_steps_are_in_any_order() {
        let mut steps: Vec<Step> = Chapter::Arrival.steps().collect();
        steps.reverse();
        let (last, rest) = steps.split_last().unwrap();
        assert!(!tour(rest, false).chapter_done(Chapter::Arrival));
        let mut all = rest.to_vec();
        all.push(*last);
        assert!(tour(&all, false).chapter_done(Chapter::Arrival));
        assert!(!tour(&all, false).chapter_done(Chapter::RoadIn));
    }

    #[test]
    fn the_steps_that_lead_somewhere() {
        assert_eq!(target(Step::Walk), None, "anywhere will do");
        assert_eq!(target(Step::GatherGrass), Some(Target::Node(&["grass"])));
        assert_eq!(target(Step::FellTree), Some(Target::Node(&["tree"])));
        assert_eq!(target(Step::SeeSuburbanLot), Some(Target::Lot(Zone::Suburban)));
        assert_eq!(target(Step::VisitStall), Some(Target::Stall));
        // Everything at the Exchange leads to the hall; the Gazette reads anywhere
        for step in Chapter::Hall.steps().filter(|s| !matches!(s, Step::VisitStall | Step::ReadGazette)) {
            assert_eq!(target(step), Some(Target::Hall), "{step:?}");
        }
        assert_eq!(target(Step::ReadGazette), Some(Target::Board));
        // The Deeds Office's desk; the claim's lot is the one it showed you
        assert_eq!(target(Step::OpenOffice), Some(Target::Office));
        assert_eq!(target(Step::ClaimHome), None);
    }

    /// Each stop of the road in leads to its zone's lot, and the zone card,
    /// if it's still to read, to the last of them (#445)
    #[test]
    fn the_road_in_leads_to_its_stops() {
        let stops: Vec<_> = Chapter::RoadIn.steps().map(target).collect();
        let lot = |z| Some(Target::Lot(z));
        assert_eq!(stops, [lot(Zone::Rural), lot(Zone::Suburban), lot(Zone::Urban), lot(Zone::Urban)]);
    }

    /// Every node a step leads to is a kind the content has, and the
    /// fallback catalog's yields are the items the server judges the
    /// gathers by
    #[test]
    fn every_node_target_is_a_kind_that_yields_what_the_step_wants() {
        let catalog = crate::catalog::Catalog::fallback();
        for (step, item) in [
            (Step::GatherGrass, "fiber"),
            (Step::GatherBranches, "sticks"),
            (Step::GatherStone, "stone"),
            (Step::FellTree, "log"),
        ] {
            let Some(Target::Node(kinds)) = target(step) else {
                panic!("{step:?} leads to a node");
            };
            for kind in kinds {
                let t = catalog.resource_type(kind).unwrap_or_else(|| panic!("{kind} is in the catalog"));
                assert_eq!(t.yields_item, item, "{kind}");
            }
        }
    }

    #[test]
    fn the_nearest_live_node_of_the_kind() {
        let node = |kind: &str, x: f64, state: &str| ResourceNodeInfo {
            id: uuid::Uuid::from_u128(u128::from(x.to_bits())),
            resource_type: kind.into(),
            x,
            y: 0.0,
            state: state.into(),
        };
        let nodes = [
            node("grass", 30.0, "available"),
            node("tree", 5.0, "available"),
            node("grass", 8.0, "depleted"),
            node("grass", -12.0, "harvesting"),
            node("grass", -20.0, "available"),
        ];
        let near = nearest_node(&nodes, &["grass"], [0.0, 0.0]).unwrap();
        assert_eq!(near.x, -20.0, "passing over the spent, the worked and the trees");
        assert_eq!(nearest_node(&nodes, &["grass"], [28.0, 0.0]).unwrap().x, 30.0);
        assert!(nearest_node(&nodes, &["rock_outcrop"], [0.0, 0.0]).is_none());
    }

    fn bench(x: f64, y: f64) -> StationInfo {
        StationInfo {
            id: uuid::Uuid::nil(),
            station_type: "workbench".into(),
            name: "Workbench".into(),
            category: "crafting".into(),
            icon: None,
            x,
            y,
            owner_id: uuid::Uuid::nil(),
            container_id: None,
            interaction_range: 75.0,
            quality: 50,
            tool: None,
            automation: None,
            production: Vec::new(),
        }
    }

    /// A road 20 wide along y = 1000, a lot north of it from 1015, and
    /// nothing south of it, round (4000, 4000)'s far side
    fn roadside() -> TerrainResponse {
        use capitol_protocol::terrain::{RoadSegment, RoadType, TerrainPlot};
        TerrainResponse {
            world_size: 8000.0,
            center_x: 4000.0,
            center_y: 4000.0,
            water_rings: vec![],
            roads: vec![RoadSegment {
                id: "lane".into(),
                road_type: RoadType::Arterial,
                points: vec![(800.0, 1000.0), (1200.0, 1000.0)],
                width: 20.0,
            }],
            terrain_plots: vec![TerrainPlot {
                id: "farm".into(),
                corners: [(800.0, 1015.0), (1200.0, 1015.0), (1200.0, 1300.0), (800.0, 1300.0)],
                street_facing_edge: (0, 1),
                area: 114_000.0,
                adjacent_street_id: "lane".into(),
                zone: "rural".into(),
            }],
            parks: vec![],
            landmarks: vec![],
            land: Default::default(),
            rivers: vec![],
            streams: vec![],
            lakes: vec![],
            fords: vec![],
            version: None,
        }
    }

    /// The server would take a bench there: `station::check`'s own tests,
    /// off every plot
    fn the_server_takes(at: [f64; 2], stations: &[StationInfo], terrain: &TerrainResponse) -> bool {
        crate::station::check(at, at, uuid::Uuid::nil(), true, stations, &[], Some(terrain)) == Ok(None)
    }

    #[test]
    fn the_workbench_goes_on_a_clear_spot_beside_the_road() {
        assert_eq!(target(Step::PlaceWorkbench), Some(Target::ClearSpot));
        let terrain = roadside();
        let none: [StationInfo; 0] = [];
        // On the road: the nearest spot off it, which is south, away from the farm
        let spot = clear_spot([1000.0, 1000.0], &terrain, &none).expect("a spot");
        assert!(the_server_takes(spot, &none, &terrain), "{spot:?}");
        assert!(spot[1] < 1000.0 - 10.0 - SPOT_ROAD_GAP + 1e-9, "beside the road, not on it or the farm: {spot:?}");
        assert!((spot[0] - 1000.0).hypot(spot[1] - 1000.0) <= 10.0 + SPOT_ROAD_GAP + 5.0, "the nearest such: {spot:?}");

        // Someone's bench there already: the next is clear of it
        let theirs = [bench(spot[0], spot[1])];
        let next = clear_spot([1000.0, 1000.0], &terrain, &theirs).expect("another");
        assert!(the_server_takes(next, &theirs, &terrain));
        assert!((next[0] - spot[0]).hypot(next[1] - spot[1]) >= 50.0 + SPOT_STATION_GAP);

        // In the farm's middle, every spot near is the farm's
        assert_eq!(clear_spot([1000.0, 1200.0], &terrain, &none), None);
        // At its edge, the spot is out on the road's verge
        let verge = clear_spot([1000.0, 1030.0], &terrain, &none).expect("the verge");
        assert!(the_server_takes(verge, &none, &terrain));
    }

    /// On the default world, from the spawn: a spot the server takes, off
    /// the arterial (#443)
    #[test]
    fn a_newcomer_has_a_clear_spot_by_the_spawn() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = crate::terrain::read_terrain_file(&path).expect("the fixture");
        let spawn = [rules::SPAWN_X, rules::SPAWN_Y];
        let none: [StationInfo; 0] = [];
        let spot = clear_spot(spawn, &terrain, &none).expect("a spot by the spawn");
        assert!(the_server_takes(spot, &none, &terrain), "{spot:?}");
        assert!(!Ground::near(spawn, SPOT_REACH, &terrain, &none).on_road(spot), "{spot:?}");
    }

    #[test]
    fn the_quality_card_reads_the_players_own_pack() {
        use capitol_protocol::rules::stacking::Units;
        let catalog = crate::catalog::Catalog::fallback();
        let rope = catalog.recipes().iter().find(|r| r.id == "rope_basic");
        assert!(rope.is_some(), "the rope recipe is in the catalog");
        let item = |kind: &str, units: &[(i32, i32)]| {
            SlotItem::new(uuid::Uuid::nil(), kind.into(), kind.into(), 0, 0, 0).with_units(Units::from_pairs(units.iter().copied()))
        };
        // No rope yet: nothing to say
        assert_eq!(QualityLesson::of(&[item("fiber", &[(50, 4)])], rope), None);

        let pack = [item("fiber", &[(41, 2), (58, 1)]), item("rope", &[(52, 1)]), item("fiber", &[(66, 3)]), item("sticks", &[(90, 1)])];
        let lesson = QualityLesson::of(&pack, rope).unwrap();
        assert_eq!(lesson.ropes, vec![52]);
        assert_eq!(lesson.fiber, Some((41, 66)), "across every stack, and only fiber");
        // The rope is fiber and the roll (rope_basic: ±3)
        assert_eq!(lesson.twists, Some(((38, 44), (63, 69))));

        // Two ropes of two qualities, best first; the fiber all spent
        let two = [item("rope", &[(44, 1), (58, 1)])];
        let lesson = QualityLesson::of(&two, rope).unwrap();
        assert_eq!(lesson.ropes, vec![58, 44]);
        assert_eq!((lesson.fiber, lesson.twists), (None, None));
    }

    #[test]
    fn a_card_to_open_is_the_nearest_someone_else() {
        assert_eq!(target(Step::OpenCard), Some(Target::Person));
        assert_eq!(target(Step::SayLocal), None, "said anywhere");
        assert_eq!(target(Step::ReadLetter), None, "read anywhere");
        let at = |n: u128, x: f64, citizen: bool| PlayerPosition { id: Uuid::from_u128(n), name: format!("P{n}"), x, y: 0.0, citizen };
        let me = Uuid::from_u128(1);
        let players = [at(1, 0.0, false), at(2, 40.0, false), at(3, -25.0, true)];
        assert_eq!(nearest_person(&players, Some(me), [0.0, 0.0]).unwrap().id, Uuid::from_u128(3), "a citizen's card will do");
        assert_eq!(nearest_person(&players, Some(me), [38.0, 0.0]).unwrap().id, Uuid::from_u128(2));
        assert!(nearest_person(&players[..1], Some(me), [0.0, 0.0]).is_none(), "nobody but you");
    }

    #[test]
    fn a_step_knows_its_place_in_its_chapter() {
        assert_eq!(place_in_chapter(Step::Walk), (0, Chapter::Arrival.steps().count()));
        assert_eq!(place_in_chapter(Step::FellTree), (11, 12));
        assert_eq!(place_in_chapter(Step::SeeRuralLot), (0, 4));
        assert_eq!(place_in_chapter(Step::ClaimHome), (1, 2));
    }

    /// A farm-sized lot, 120 by 300, of `zone`, owned by `owner`
    fn lot(n: u128, zone: &str, owner: Option<Uuid>, claimed: i64) -> PlotInfo {
        use capitol_protocol::rest::Bounds;
        PlotInfo {
            id: Uuid::from_u128(n),
            zone_id: zone.into(),
            world_x: 60.0,
            world_y: 150.0,
            bounds: Bounds::new(0.0, 0.0, 120.0, 300.0),
            size_category: "large".into(),
            plot_type: "residential".into(),
            owner_id: owner,
            owner_name: None,
            owner_citizen: false,
            claimed_at: chrono::DateTime::from_timestamp(claimed, 0),
            assessed_value: 250,
            tax_owed: 0,
            forecloses_at: owner.map(|_| 20),
            deed_cost: Some(250),
            station_count: 0,
            station_capacity: 25,
            terrain_id: None,
            corners: Some([(0.0, 0.0), (120.0, 0.0), (120.0, 300.0), (0.0, 300.0)]),
        }
    }

    /// Settle in leads to your own land, then to a station of yours on it,
    /// and the tax card leads nowhere: it asks nothing of the world (#449)
    #[test]
    fn settling_in_leads_home() {
        assert_eq!(target(Step::PlaceAtHome), Some(Target::Home));
        assert_eq!(target(Step::SetAutomation), Some(Target::MyStation));
        assert_eq!(target(Step::ReadTax), None);
        assert_eq!(Tour::default().report(Did::ReadTax), Some(Step::ReadTax));
        assert!(Did::ReadTax.step().judge().client_reports(), "the server takes the tax card's word");
    }

    /// Home is the home lot claimed last; a guild plot isn't one
    #[test]
    fn home_is_the_home_lot_claimed_last() {
        let me = Some(Uuid::from_u128(1));
        let plots = [lot(2, "rural", me, 100), lot(3, "suburban", me, 200), lot(4, "guild", me, 300)];
        assert_eq!(home_plot(&plots).unwrap().id, Uuid::from_u128(3));
        assert_eq!(home_plot(&plots[2..]), None, "a guild plot isn't a home");
        assert_eq!(home_plot(&[]), None);
    }

    /// Its room is #444's count, worked out once a lot; its tax a quarter of
    /// the debt that forecloses it
    #[test]
    fn a_home_says_what_it_fits_and_what_it_owes() {
        let me = Some(Uuid::from_u128(1));
        let mut plot = lot(2, "rural", me, 100);
        let home = Home::of(&plot, None).expect("a farm is a home");
        let fits = crate::sim::lot::layout(crate::sim::lot::corners(&plot), 25).len();
        assert!(fits > 4 && fits <= 25, "{fits}");
        assert_eq!((home.zone, home.fits, home.capacity, home.placed), (Zone::Rural, fits, 25, 0));
        assert_eq!((home.weekly_tax, home.forecloses_at(), home.assessed), (Some(5), Some(20), 250));
        // A station placed and a debt run up: the room is kept, the rest follows
        plot.station_count = 1;
        plot.tax_owed = 3;
        let kept = Home { fits: 99, ..home.clone() };
        let after = Home::of(&plot, Some(&kept)).unwrap();
        assert_eq!((after.fits, after.placed, after.owed), (99, 1, 3));
        // Another lot is packed afresh
        let other = lot(3, "rural", me, 100);
        assert_eq!(Home::of(&other, Some(&kept)).unwrap().fits, fits);
        // Untaxed, it says so; a guild plot isn't a home
        plot.forecloses_at = None;
        assert_eq!(Home::of(&plot, None).unwrap().weekly_tax, None);
        assert_eq!(Home::of(&lot(4, "guild", me, 0), None), None);
    }

    /// The automation step leads to a crafting station of yours with a box:
    /// at home first, else the nearest (Arrival's bench by the road)
    #[test]
    fn the_job_goes_on_a_station_of_yours_at_home_first() {
        let me = Uuid::from_u128(1);
        let home = lot(2, "rural", Some(me), 100);
        let mine = |n: u128, x: f64, y: f64| StationInfo {
            id: Uuid::from_u128(n),
            owner_id: me,
            container_id: Some(Uuid::from_u128(n + 100)),
            ..bench(x, y)
        };
        let road = mine(10, -40.0, 0.0);
        let at_home = mine(11, 60.0, 250.0);
        let chest = StationInfo { category: "storage".into(), station_type: "storage_chest".into(), ..mine(12, 60.0, 20.0) };
        let theirs = StationInfo { owner_id: Uuid::from_u128(7), ..mine(13, 60.0, 40.0) };
        let stations = [road.clone(), at_home.clone(), chest, theirs];
        let from = [-30.0, 0.0];
        assert_eq!(my_station(&stations, Some(me), Some(&home), from).unwrap().id, at_home.id);
        assert_eq!(my_station(&stations, Some(me), None, from).unwrap().id, road.id, "no home: the nearest");
        assert_eq!(my_station(&stations[2..], Some(me), Some(&home), from), None, "a chest makes nothing");
        assert_eq!(my_station(&stations, None, Some(&home), from), None);
    }
}
