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
//! it.

use capitol_protocol::rest::ResourceNodeInfo;
use capitol_protocol::rules::Zone;
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
}

impl Did {
    /// The step it counts for.
    pub fn step(self) -> Step {
        match self {
            Did::UsedInventory | Did::OpenedCrafting => Step::OpenPack,
            Did::ReadZoneCard => Step::ReadZoneCard,
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
}

/// Where `step` leads, if anywhere. A gather leads to the nodes the server
/// judges it by (`engine::tutorial`: fiber, sticks, stone, a log): the
/// kinds that grow round the spawn, so not the wilds' old growth, which a
/// stone axe can't fell. Each chapter's work package adds its own (#443 to
/// #449): the workbench's clear spot, the road's stops, the Deeds Office.
pub fn target(step: Step) -> Option<Target> {
    Some(match step {
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
        Tour::from(TutorialProgress { done: done.iter().copied().collect(), dismissed })
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
        let done = Tour::from(TutorialProgress { done: Steps::every(), dismissed: true });
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
        for did in [Did::UsedInventory, Did::OpenedCrafting, Did::ReadZoneCard] {
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
}
