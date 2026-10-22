//! The road in (`the-capitol-road-in.md`): the tutorial's steps and
//! chapters, shared by the server, which keeps and judges them, and the
//! clients, which show them.
//!
//! A step's number is its bit in [`Steps`], stored in
//! `tutorial_progress.steps`, so a number is never reused or changed: a new
//! step takes the next free one and goes wherever it belongs in
//! [`Step::ALL`], which is the order the tour shows them in. On the wire
//! [`Steps`] is a list of the steps' names, and a name this build doesn't
//! know is passed over, so a client a step behind still reads the rest.
//!
//! Each step's words are the client's, not the protocol's.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod stops;

/// How far a walk has to end from where it began to count for
/// [`Step::Walk`]: judged where the server sees the walk end (#439)
pub const WALK_DISTANCE: f64 = 20.0;

/// A chapter of the road in (plan §4), in the order they're walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chapter {
    /// Making things among the farms, at the spawn (#443)
    Arrival,
    /// A stop in each home zone on the walk to the hall (#445)
    RoadIn,
    /// The Exchange, the order book, the bazaar and the Gazette (#446)
    Hall,
    /// A word, a card and the Registrar's letter (#447)
    People,
    /// Comparing the zones, and the claim with the grant (#448)
    DeedsOffice,
    /// A station at home, a job that runs, and what tax asks (#449)
    SettleIn,
}

impl Chapter {
    pub const ALL: [Chapter; 6] = [
        Chapter::Arrival,
        Chapter::RoadIn,
        Chapter::Hall,
        Chapter::People,
        Chapter::DeedsOffice,
        Chapter::SettleIn,
    ];

    /// The chapter's steps, in the order they're shown
    pub fn steps(self) -> impl Iterator<Item = Step> {
        Step::ALL.into_iter().filter(move |s| s.chapter() == self)
    }
}

/// Who says a step is done (plan D7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judge {
    /// The server, at the point the action succeeds (#439)
    Server,
    /// The client, with `tutorial_seen`: the step is only something on
    /// screen, which the server can't see. No reward hangs on these alone.
    Client,
    /// The server when it happens, or the client when it can't: a step
    /// that needs someone else, as an order filling, is taught and then
    /// waived with words (plan D10)
    Waivable,
}

impl Judge {
    /// Whether a client's `tutorial_seen` counts for the step: one only the
    /// screen shows, or one waived with words when nobody else came (D10).
    /// Never a step the server judges alone.
    pub fn client_reports(self) -> bool {
        matches!(self, Judge::Client | Judge::Waivable)
    }
}

/// One step of the road in. The number is the step's bit: never reuse or
/// change one (see the module's notes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Step {
    // Arrival (#443)
    /// Walk somewhere
    Walk = 0,
    /// Gather grass by hand
    GatherGrass = 1,
    /// Open your pack
    OpenPack = 2,
    /// Twist rope (`rope_basic`)
    TwistRope = 3,
    /// Make a workbench kit (`craft_workbench`)
    MakeWorkbench = 4,
    /// Place it, off any lot
    PlaceWorkbench = 5,
    /// Gather fallen branches by hand
    GatherBranches = 6,
    /// Gather stone by hand
    GatherStone = 7,
    /// Make a handle at the workbench (`tool_handle`)
    MakeHandle = 8,
    /// Knap stone at the workbench (`knapped_stone`)
    KnapStone = 9,
    /// Make a stone axe (`stone_axe`)
    MakeAxe = 10,
    /// Fell a tree, which needs the axe
    FellTree = 11,
    // The road in (#445)
    /// Look at a rural lot (a farm)
    SeeRuralLot = 12,
    /// Look at a suburban lot
    SeeSuburbanLot = 13,
    /// Look at an urban lot
    SeeUrbanLot = 14,
    /// Open a zone card (#444)
    ReadZoneCard = 15,
    // The hall (#446)
    /// Deposit fiber at the Exchange
    Deposit = 16,
    /// Place a sell order
    SellOrder = 17,
    /// Place a buy order
    BuyOrder = 18,
    /// Collect from the market box
    CollectBox = 19,
    /// Look at a bazaar stall's prices
    VisitStall = 20,
    /// Read the Gazette
    ReadGazette = 21,
    // People (#447)
    /// Say a line in local talk
    SayLocal = 22,
    /// Open someone's card
    OpenCard = 23,
    /// Read the Registrar's letter
    ReadLetter = 24,
    // The Deeds Office (#448)
    /// Open the Deeds Office's panel
    OpenOffice = 25,
    /// Claim a home
    ClaimHome = 26,
    // Settle in (#449)
    /// Place a station on your own plot
    PlaceAtHome = 27,
    /// Set a station's automation job
    SetAutomation = 28,
    /// Read what tax asks of you
    ReadTax = 29,
}

impl Step {
    /// Every step, in the order the tour shows them: chapter by chapter
    pub const ALL: [Step; 30] = [
        Step::Walk,
        Step::GatherGrass,
        Step::OpenPack,
        Step::TwistRope,
        Step::MakeWorkbench,
        Step::PlaceWorkbench,
        Step::GatherBranches,
        Step::GatherStone,
        Step::MakeHandle,
        Step::KnapStone,
        Step::MakeAxe,
        Step::FellTree,
        Step::SeeRuralLot,
        Step::SeeSuburbanLot,
        Step::SeeUrbanLot,
        Step::ReadZoneCard,
        Step::Deposit,
        Step::SellOrder,
        Step::BuyOrder,
        Step::CollectBox,
        Step::VisitStall,
        Step::ReadGazette,
        Step::SayLocal,
        Step::OpenCard,
        Step::ReadLetter,
        Step::OpenOffice,
        Step::ClaimHome,
        Step::PlaceAtHome,
        Step::SetAutomation,
        Step::ReadTax,
    ];

    /// The step's bit in [`Steps`]
    pub const fn bit(self) -> u64 {
        1 << self as u8
    }

    pub fn chapter(self) -> Chapter {
        use Step::*;
        match self {
            Walk | GatherGrass | OpenPack | TwistRope | MakeWorkbench | PlaceWorkbench | GatherBranches
            | GatherStone | MakeHandle | KnapStone | MakeAxe | FellTree => Chapter::Arrival,
            SeeRuralLot | SeeSuburbanLot | SeeUrbanLot | ReadZoneCard => Chapter::RoadIn,
            Deposit | SellOrder | BuyOrder | CollectBox | VisitStall | ReadGazette => Chapter::Hall,
            SayLocal | OpenCard | ReadLetter => Chapter::People,
            OpenOffice | ClaimHome => Chapter::DeedsOffice,
            PlaceAtHome | SetAutomation | ReadTax => Chapter::SettleIn,
        }
    }

    pub fn judge(self) -> Judge {
        use Step::*;
        match self {
            OpenPack | ReadZoneCard | OpenOffice | ReadTax => Judge::Client,
            CollectBox => Judge::Waivable,
            _ => Judge::Server,
        }
    }

    /// The step's name on the wire
    pub fn name(self) -> &'static str {
        use Step::*;
        match self {
            Walk => "walk",
            GatherGrass => "gather_grass",
            OpenPack => "open_pack",
            TwistRope => "twist_rope",
            MakeWorkbench => "make_workbench",
            PlaceWorkbench => "place_workbench",
            GatherBranches => "gather_branches",
            GatherStone => "gather_stone",
            MakeHandle => "make_handle",
            KnapStone => "knap_stone",
            MakeAxe => "make_axe",
            FellTree => "fell_tree",
            SeeRuralLot => "see_rural_lot",
            SeeSuburbanLot => "see_suburban_lot",
            SeeUrbanLot => "see_urban_lot",
            ReadZoneCard => "read_zone_card",
            Deposit => "deposit",
            SellOrder => "sell_order",
            BuyOrder => "buy_order",
            CollectBox => "collect_box",
            VisitStall => "visit_stall",
            ReadGazette => "read_gazette",
            SayLocal => "say_local",
            OpenCard => "open_card",
            ReadLetter => "read_letter",
            OpenOffice => "open_office",
            ClaimHome => "claim_home",
            PlaceAtHome => "place_at_home",
            SetAutomation => "set_automation",
            ReadTax => "read_tax",
        }
    }

    pub fn from_name(name: &str) -> Option<Step> {
        Step::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// The steps a player has done: a bit each ([`Step::bit`]). The tour is
/// finished when every step is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Steps(u64);

impl Steps {
    pub const NONE: Steps = Steps(0);

    /// Every step this build knows
    pub const fn every() -> Steps {
        let mut bits = 0;
        let mut i = 0;
        while i < Step::ALL.len() {
            bits |= Step::ALL[i].bit();
            i += 1;
        }
        Steps(bits)
    }

    /// From the stored bits. A bit this build has no step for is dropped.
    pub fn from_bits(bits: u64) -> Steps {
        Steps(bits & Steps::every().0)
    }

    pub fn bits(self) -> u64 {
        self.0
    }

    pub fn has(self, step: Step) -> bool {
        self.0 & step.bit() != 0
    }

    pub fn with(self, step: Step) -> Steps {
        Steps(self.0 | step.bit())
    }

    /// The steps done, in the order the tour shows them
    pub fn iter(self) -> impl Iterator<Item = Step> {
        Step::ALL.into_iter().filter(move |s| self.has(*s))
    }

    /// The first step not done, in the tour's order: done out of order
    /// still counts. `None` when the tour is finished.
    pub fn current(self) -> Option<Step> {
        Step::ALL.into_iter().find(|s| !self.has(*s))
    }

    pub fn chapter_done(self, chapter: Chapter) -> bool {
        chapter.steps().all(|s| self.has(s))
    }

    /// Every chapter before `chapter` is done, in whatever order: the tour
    /// has come to it. People opening sends the Registrar's letter (#447).
    pub fn chapter_open(self, chapter: Chapter) -> bool {
        Chapter::ALL.into_iter().take_while(|c| *c != chapter).all(|c| self.chapter_done(c))
    }

    /// Every chapter before the Deeds Office is done, which opens the deed
    /// grant (#440): the road in has been walked, and the claim is next
    pub fn deed_earned(self) -> bool {
        Chapter::ALL.into_iter().take_while(|c| *c != Chapter::DeedsOffice).all(|c| self.chapter_done(c))
    }

    pub fn finished(self) -> bool {
        self == Steps::every()
    }
}

impl FromIterator<Step> for Steps {
    fn from_iter<I: IntoIterator<Item = Step>>(steps: I) -> Steps {
        steps.into_iter().fold(Steps::NONE, Steps::with)
    }
}

impl Serialize for Steps {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter().map(Step::name))
    }
}

impl<'de> Deserialize<'de> for Steps {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Steps, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        Ok(names.iter().filter_map(|n| Step::from_name(n)).collect())
    }
}

/// Where a player is on the road in: `GET /tutorial`, and what `tutorial`
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TutorialProgress {
    /// The steps done, by name, in the tour's order
    #[cfg_attr(feature = "backend", schema(value_type = Vec<String>))]
    pub done: Steps,
    /// The card is hidden for now, on every device (plan D2). The tour
    /// waits; it can't be skipped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dismissed: bool,
}

impl TutorialProgress {
    pub fn finished(&self) -> bool {
        self.done.finished()
    }
}

/// `PUT /tutorial/dismissed`: hide the card for now, or bring it back
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct SetDismissed {
    pub dismissed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_step_has_its_own_bit_below_the_sign() {
        let bits: HashSet<u8> = Step::ALL.iter().map(|s| *s as u8).collect();
        assert_eq!(bits.len(), Step::ALL.len(), "two steps share a bit");
        // Stored as a BIGINT, so bit 63 would read back negative
        assert!(bits.iter().all(|b| *b < 63));
    }

    #[test]
    fn each_name_reads_back_as_its_step() {
        let names: HashSet<&str> = Step::ALL.iter().map(|s| s.name()).collect();
        assert_eq!(names.len(), Step::ALL.len());
        for step in Step::ALL {
            assert_eq!(Step::from_name(step.name()), Some(step));
            // The name is serde's, so a step alone and in a list agree
            assert_eq!(serde_json::to_string(&step).unwrap(), format!("\"{}\"", step.name()));
        }
    }

    #[test]
    fn the_chapters_come_in_order_and_each_has_steps() {
        let order: Vec<Chapter> = Step::ALL.iter().map(|s| s.chapter()).collect();
        let mut chapters = order.clone();
        chapters.dedup();
        assert_eq!(chapters, Chapter::ALL, "a chapter's steps are together, in chapter order");
    }

    #[test]
    fn the_current_step_is_the_first_not_done() {
        let done = Steps::from_iter([Step::Walk, Step::OpenPack]);
        assert_eq!(done.current(), Some(Step::GatherGrass));
        assert!(!done.chapter_done(Chapter::Arrival));
        let arrival: Steps = Chapter::Arrival.steps().collect();
        assert!(arrival.chapter_done(Chapter::Arrival));
        assert_eq!(arrival.current(), Some(Step::SeeRuralLot));
        assert!(!arrival.finished());
        assert_eq!(Steps::every().current(), None);
        assert!(Steps::every().finished());
    }

    #[test]
    fn the_deed_is_earned_by_every_chapter_before_the_office() {
        let before: Steps = [Chapter::Arrival, Chapter::RoadIn, Chapter::Hall, Chapter::People]
            .into_iter()
            .flat_map(Chapter::steps)
            .collect();
        assert!(before.deed_earned());
        // One step short, wherever it is, and it isn't
        for step in before.iter() {
            assert!(!Steps::from_bits(before.bits() & !step.bit()).deed_earned(), "{step:?}");
        }
        // The office and settling in don't count toward it
        let after: Steps = Chapter::DeedsOffice.steps().chain(Chapter::SettleIn.steps()).collect();
        assert!(!after.deed_earned());
    }

    #[test]
    fn a_chapter_opens_once_every_one_before_it_is_done() {
        assert!(Steps::NONE.chapter_open(Chapter::Arrival), "the first is open from the start");
        let before: Steps = [Chapter::Arrival, Chapter::RoadIn, Chapter::Hall].into_iter().flat_map(Chapter::steps).collect();
        assert!(before.chapter_open(Chapter::People));
        assert!(!before.chapter_open(Chapter::DeedsOffice));
        for step in before.iter() {
            assert!(!Steps::from_bits(before.bits() & !step.bit()).chapter_open(Chapter::People), "{step:?}");
        }
        // Its own steps, done ahead, don't open it
        let ahead = Steps::from_bits(before.bits() & !Step::Deposit.bit()).with(Step::SayLocal);
        assert!(!ahead.chapter_open(Chapter::People));
    }

    #[test]
    fn steps_go_out_in_the_tours_order_and_unknown_names_are_passed_over() {
        let done = Steps::from_iter([Step::Deposit, Step::Walk]);
        assert_eq!(serde_json::to_string(&done).unwrap(), r#"["walk","deposit"]"#);
        let back: Steps = serde_json::from_str(r#"["deposit","a_step_from_later","walk"]"#).unwrap();
        assert_eq!(back, done);
    }

    #[test]
    fn stored_bits_with_no_step_are_dropped() {
        let bits = Step::Walk.bit() | 1 << 62;
        assert_eq!(Steps::from_bits(bits), Steps::from_iter([Step::Walk]));
    }

    #[test]
    fn only_screen_steps_are_the_clients() {
        let client: Vec<Step> = Step::ALL.into_iter().filter(|s| s.judge() == Judge::Client).collect();
        assert_eq!(client, [Step::OpenPack, Step::ReadZoneCard, Step::OpenOffice, Step::ReadTax]);
        // The claim, the reward's own step, is always the server's
        assert_eq!(Step::ClaimHome.judge(), Judge::Server);
        assert!(!Step::ClaimHome.judge().client_reports());
        assert!(Step::OpenPack.judge().client_reports() && Step::CollectBox.judge().client_reports());
    }
}
