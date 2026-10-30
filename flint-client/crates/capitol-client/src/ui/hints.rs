//! The road in's card (#441, `the-capitol-road-in.md`): one at a time, top
//! right under the header (clear of the zone banner, the panels and the
//! toasts), for the step at hand. It's P8.3's hint card, grown to the
//! server's tour: which step, and what counts as done, is the server's, as
//! `capitol_core::onboarding::Tour` reads it. Nothing is kept here.
//!
//! The card is the chapter's (#442): its name, the step's words, and pips
//! for that chapter's steps only. A finished step shows its tick for a
//! moment before the next card fades in. A finished chapter has a card of
//! its own, a beat longer ("Arrival: done", what you can now do, and what's
//! next), and the last gives way to a closing card that points at `?`.
//! "Hide for now" hides the card on every device (`dismissed`, plan D2), and
//! the header's road-in chip or a line in the `?` overlay brings it back.
//! The tour can't be skipped: the deed comes only with the finish.
//!
//! Settle in (#449) speaks of your own lot ([`Hints::set_home`]): what it
//! fits by the zone card's count, and its week's tax. The tax card asks
//! nothing of the world, so its "Noted" is the step ([`Hints::noted`]). The
//! closing card ("Well begun") has the city's seal, and points at the
//! Gazette and at `?`. Once the tour is finished nothing shows: no card, no
//! header chip, no line in the `?` overlay.
//!
//! A step can lead somewhere (`onboarding::target`). The extension finds
//! the place and hands back a [`Lead`]: off screen, the card points the way
//! with the hall chip's arrow (`hud::bearing`); on screen, `input.rhai`
//! rings it in gold on the ground (`draw_lead`).

use std::borrow::Cow;
use std::time::{Duration, Instant};

use capitol_core::onboarding::{place_in_chapter, target, Did, Home, QualityLesson, Target, Tour};
use capitol_core::protocol::rest::PlotInfo;
use capitol_core::protocol::tutorial::{Chapter, Grant, Step, Steps, TutorialProgress};
use egui::{Align2, Id, RichText};

use super::shortcuts::{keys_row, on_the_pad};
use super::theme::{self, size};

/// How long a done step shows its tick before the next card.
const TICK_FOR: Duration = Duration::from_millis(900);
/// How long a card takes to fade in.
const FADE_IN: f32 = 0.35;
/// How long a chapter's close stays: a beat longer than a step's tick.
const CHAPTER_FOR: Duration = Duration::from_millis(5500);
/// How long the closing card stays, and how long it takes to go: long
/// enough to read its words and its two ways on (#449).
const CLOSING_FOR: Duration = Duration::from_secs(12);
const FADE_OUT: f32 = 0.6;
/// The card's width inside its frame.
const WIDTH: f32 = 252.0;
/// How far it slides as it fades in, in points.
const RISE: f32 = 6.0;
/// Its inset from the screen's right edge, in its own corner.
const INSET: f32 = 16.0;
/// The room it keeps from a panel it steps round.
const GAP: f32 = 12.0;
/// How long it takes to step aside, or back, in seconds.
const SLIDE: f32 = 0.25;

/// A chapter's name, as its card and the header's chip say it.
pub fn chapter_name(chapter: Chapter) -> &'static str {
    match chapter {
        Chapter::Arrival => "Arrival",
        Chapter::RoadIn => "The road in",
        Chapter::Hall => "The hall",
        Chapter::People => "People",
        Chapter::DeedsOffice => "The Deeds Office",
        Chapter::SettleIn => "Settle in",
    }
}

/// What a chapter's close says you can now do: one line, in the same
/// voice. The last chapter's close is the closing card ([`CLOSING`]).
fn chapter_close(chapter: Chapter) -> &'static str {
    match chapter {
        Chapter::Arrival => {
            "You can gather by hand, make things at a bench of your own, and work with a tool what hands can't. \
             Your bench returns to your inventory whole, to take home. The road runs north into the city."
        }
        Chapter::RoadIn => {
            "You've read the deeds of a farm, a house and a yard, and what each zone is like. The hall is ahead, under the tower."
        }
        Chapter::Hall => {
            "You can change fiber for Strands, leave a price on the book, shop the stalls and read what the city made yesterday."
        }
        Chapter::People => {
            "You can speak to whoever's near, read anyone's card and tell a citizen by their mark. Letters wait for you while you're away."
        }
        Chapter::DeedsOffice => {
            "The land is yours, on the city's books and taxed as any deed is. The deed is spent; the room is yours to fill."
        }
        Chapter::SettleIn => CLOSING.1,
    }
}

/// Find your feet over the shoulder (T1.6): WASD walks, so the card says
/// so. Its title is the one [`words`] gives.
const DIRECT_WALK: (&str, &str) = (
    "You've come in on the south road, among the farms. Walk with WASD, the way you face; the mouse turns you. \
     Round the water by the bridges and fords.",
    "[W] [A] [S] [D] walk  ·  Mouse looks",
);

/// The same with a pad in your hands (T2.2): the sticks walk and look.
const PAD_WALK: (&str, &str) = (
    "You've come in on the south road, among the farms. Walk with the left stick, the way you face; the right stick \
     turns you. Round the water by the bridges and fords.",
    "{LS} walk  ·  {RS} looks",
);

/// How you're playing, which the cards speak of: point and walk, over the
/// shoulder on the keys (T1.6), or over the shoulder with a pad (T2.2).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Play {
    #[default]
    Click,
    Keys,
    Pad,
}

/// A step's words as [`words`], but as you're playing (`play`): over the
/// shoulder the walk step says WASD walks, or the stick does.
fn words_in(step: Step, play: Play) -> (&'static str, &'static str, &'static str) {
    match (step, play) {
        (Step::Walk, Play::Keys) => (words(step).0, DIRECT_WALK.0, DIRECT_WALK.1),
        (Step::Walk, Play::Pad) => (words(step).0, PAD_WALK.0, PAD_WALK.1),
        _ => words(step),
    }
}

/// The chapter after `chapter`, if there is one.
fn next_chapter(chapter: Chapter) -> Option<Chapter> {
    let at = Chapter::ALL.iter().position(|c| *c == chapter)?;
    Chapter::ALL.get(at + 1).copied()
}

/// What a step's card says: a title, the words, and a line of keys. It's
/// the bureaucracy's voice (plan D1): plain, a little dry, kindly meant.
/// Each chapter's own work package may reword its steps (#443 to #449).
fn words(step: Step) -> (&'static str, &'static str, &'static str) {
    match step {
        // Arrival (#443): making things among the farms, at the spawn
        Step::Walk => (
            "Find your feet",
            "You've come in on the south road, among the farms. Click the ground and you'll walk there, round the water by the bridges and fords.",
            "[Click] walk  ·  [W] [A] [S] [D] pan",
        ),
        Step::GatherGrass => (
            "Gather fiber",
            "Everything here starts as something growing. The tall grass is fiber: click a tuft to pull some, and you'll walk over if it's far.",
            "[Click] a tuft",
        ),
        Step::OpenPack => (
            "Your pack",
            "What you gather goes in your pack. Hover the fiber: each tuft had a quality of its own, and the stack keeps every one.",
            "[I] opens and closes it",
        ),
        Step::TwistRope => (
            "Twist some rope",
            "Open crafting, pick Basic Rope and press Fill: two fiber from your pack twist into a rope. Hands are enough for this.",
            "[C] crafting  ·  [Enter] craft",
        ),
        Step::MakeWorkbench => (
            "A workbench kit",
            "Hands only go so far. Ten fiber make a workbench kit, in the same panel. Pull more grass if you're short.",
            "[C] crafting",
        ),
        Step::PlaceWorkbench => (
            "Set it down",
            "Off the lots, a workbench is the one thing anyone may build. Right-click the kit in your pack, then click the clear spot ringed beside the road.",
            "Right-click the kit  ·  [Click] to place",
        ),
        Step::GatherBranches => (
            "Fallen branches",
            "Branches lie under the trees in the copse. Gather two by hand, as you did the grass.",
            "[Click] the branches",
        ),
        Step::GatherStone => (
            "Loose stone",
            "Stone lies round the outcrops, one at a pull by hand. You'll want four.",
            "[Click] an outcrop",
        ),
        Step::MakeHandle => (
            "A handle",
            "Open your workbench: two branches and a rope bind into a tool handle.",
            "[O] the nearest station",
        ),
        Step::KnapStone => (
            "An edge",
            "At the same bench, strike stone on stone: two make a knapped edge, and an axe takes two edges.",
            "[O] the nearest station",
        ),
        Step::MakeAxe => (
            "A stone axe",
            "Two edges, the handle and a second rope make an axe. The edges count for most of it: good stone, a good axe.",
            "[O] the nearest station",
        ),
        Step::FellTree => (
            "Fell a tree",
            "Your hands couldn't take a tree; an axe can. With it in your pack, fell one in the copse for its logs. \
             The bench's work here is done: open it and press its recycling bin, and it returns to your inventory \
             as a kit, whole, to set up on your own land.",
            "[Click] a tree  ·  [O] the bench  ·  the recycling bin",
        ),
        // The road in (#445): a stop in each zone up the south road, its
        // lot `protocol::tutorial::stops` picks
        Step::SeeRuralLot => (
            "The farms",
            "Farms have the most room, and the longest walk to market. Read the deed of the farm ringed by the road, and its zone's card. Then on up the road to the suburb.",
            "[Alt] + [Click] the lot  ·  About the zone",
        ),
        Step::SeeSuburbanLot => (
            "The suburb",
            "A house and garden on a quiet street, with room for a trade and a shorter walk in. Read this lot's deed, and its zone's card. Then on to the city's blocks.",
            "[Alt] + [Click] the lot  ·  About the zone",
        ),
        Step::SeeUrbanLot => (
            "The city",
            "A small yard a step from the market: little room, the shortest walk. Read this lot's deed, and its zone's card. Then up the road to the hall, under the tower.",
            "[Alt] + [Click] the lot  ·  About the zone",
        ),
        Step::ReadZoneCard => (
            "What a zone is like",
            "A lot's panel opens its zone's card: what fits, the price and the tax, the walk to the hall, what grows near. Read one.",
            "[Alt] + [Click] a lot  ·  About the zone",
        ),
        // The hall (#446): no step costs more than an order's escrow, which comes back, and a fee
        Step::Deposit => (
            "The Exchange",
            "The Exchange takes fiber in and pays out Strands, the city's coin, less a small fee. \
             It's a window, not a place: open it wherever you stand. It sets all three in a line. Deposit a little.",
            "[X] the Exchange  ·  Deposit",
        ),
        Step::SellOrder => (
            "Offer something",
            "The order book is a list of prices, waiting. Yours waits in it until someone meets it. \
             Put up something you made on the road: a rope, a handle, an edge.",
            "[M] the market  ·  Sell",
        ),
        // A bid needs no seller (#494): the book may be empty (plan D10),
        // so the card says so, and the market opens on stone ([`BID_FOR`])
        Step::BuyOrder => (
            "Bid for something",
            "A bid needs no seller: you name a price, and it waits on the book until someone meets it, \
             even for an item not trading yet. Try stone: a workbench at home knaps it into edges. \
             Your Strands wait in escrow, and come back if you cancel.",
            "[M] the market  ·  Buy",
        ),
        Step::CollectBox => (
            "Your market box",
            "What a buy order gets you waits in your market box at the hall, with anything that comes back. \
             Open the market and collect it.",
            "[M] the market  ·  Collect",
        ),
        Step::VisitStall => (
            "The bazaar",
            "Across the avenue, people rent stalls and keep shops. Walk the lane and stand at a counter: \
             its prices show from there.",
            "[Click] a stall",
        ),
        Step::ReadGazette => (
            "The Gazette",
            "The Gazette is pinned to the board by the hall's door: yesterday's prices, and who made what. \
             Read the latest.",
            "[Click] the board  ·  [G] the Gazette",
        ),
        // People
        Step::SayLocal => (
            "A word",
            "Press Enter and say something on the Local tab. Whoever's within a few paces (3 m) hears it, \
             over your head; the Capitol tab is heard by the whole city.",
            "[Enter] talk",
        ),
        Step::OpenCard => (
            "Someone's card",
            "Click anyone to read their card. A pale lozenge by a name marks a citizen: they live and work \
             here too, and are marked so you always know. To trade, Shift-click someone or press T; \
             nobody has to.",
            "[Click] a person  ·  [Shift] + [Click] trade",
        ),
        Step::ReadLetter => (
            "The Registrar's letter",
            "The Registrar has written to you. It waits in your letters, sealed: open it.",
            "[L] your letters",
        ),
        // The Deeds Office
        Step::OpenOffice => (
            "The Deeds Office",
            "The road has earned you a deed. It waits at the desk under the red awning, beside the Gazette's board, where the zones are set side by side.",
            "[Click] the desk",
        ),
        Step::ClaimHome => (
            "Choose a home",
            "Any free lot in the three rings is yours for the deed. \"Show me\" marks one to walk out and see; claim it at the desk, or from the lot's own panel.",
            "[Alt] + [Click] a lot  ·  Claim with your deed",
        ),
        // Settle in (#449): the first hour at home. The station's and the
        // tax's words are your own lot's ([`home_words`]); these are for a
        // home the client hasn't listed yet.
        Step::PlaceAtHome => (
            "A station at home",
            "The room on your own lot is yours to fill. Set down the bench you brought from the road, or make \
             a kit by hand (a storage chest takes 8 fiber, a workbench 10). A workbench at home can be set to work, next.",
            "[C] crafting  ·  Right-click the kit",
        ),
        Step::SetAutomation => (
            "Work that runs",
            "A workbench of yours can work without you. Open it, and on its Automation tab choose \
             Knapped Stone and Forever, then Start: it takes stone from its own box and leaves the edges there. \
             It waits while you're away and goes on when you're back; a subscription keeps it working.",
            "[O] the nearest station  ·  Automation",
        ),
        Step::ReadTax => (
            "What the city asks",
            "Every deed owes a week's tax, a share of what it's assessed at, taken from your Strands a little \
             each hour. An hour you can't pay is owed: Alt-click your lot to see the debt and pay it. Four weeks \
             unpaid and the lot is foreclosed: its stations come down and back to you, and the land goes back to the city.",
            "[Alt] + [Click] your lot",
        ),
    }
}

/// Settle in's words for your own lot (#449), where the card has one: what
/// it fits, by #444's count, and what it's taxed. `None` for any other step,
/// or with no home listed yet ([`words`]' then).
fn home_words(step: Step, home: Option<&Home>) -> Option<String> {
    let home = home?;
    let noun = capitol_core::deeds::home_noun(home.zone);
    match step {
        Step::PlaceAtHome => {
            // The deed's capacity reads more than fits on most lots (plan D9)
            let allows = if home.fits < home.capacity.max(0) as usize {
                format!(", though its deed allows {}", home.capacity)
            } else {
                String::new()
            };
            let stations = if home.fits == 1 { "station" } else { "stations" };
            Some(format!(
                "Your {noun} fits {} {stations}{allows}: the room is yours to fill. Set down the bench you \
                 brought from the road, or make a kit by hand (a storage chest takes 8 fiber, a workbench 10). \
                 A workbench at home can be set to work, next.",
                home.fits
            ))
        }
        Step::ReadTax => {
            let Some(week) = home.weekly_tax else {
                return Some(format!(
                    "Every deed owes tax, a share of what it's assessed at, taken a little each hour. \
                     The rate stands at nothing just now, so your {noun} owes none. Alt-click it to see \
                     a debt and pay it, if one comes."
                ));
            };
            let owes = if home.owed > 0 {
                format!(" It owes {} now.", home.owed)
            } else {
                String::new()
            };
            Some(format!(
                "Your {noun} is assessed at {} Strands, and its tax is {week} a week, taken from your Strands \
                 a little each hour. An hour you can't pay is owed: Alt-click your {noun} to see the debt and pay it.{owes} \
                 Four weeks unpaid ({} Strands) and it's foreclosed: its stations come down and back to you, \
                 and the land goes back to the city. A bought deed is refunded less the debt; the road in's, nothing.",
                home.assessed,
                week * capitol_core::protocol::rules::FORECLOSE_AFTER_WEEKS,
            ))
        }
        _ => None,
    }
}

/// The tax card's "Noted": it asks nothing of the world, so reading it is
/// the step (`read_tax`, a client-reported one).
const NOTED: &str = "Noted";

/// The market box's card once its box is known empty (#446, plan D10):
/// nothing's filled, so it says so, and the road goes on without it.
const BOX_EMPTY: &str = "Nothing's filled yet, so your box is empty: a price can wait a day for its match. \
     Your orders stay on the book, and the road goes on without them.";

/// How long the market box's card says it's empty before it's waived.
const WAIVE_AFTER: Duration = Duration::from_secs(7);

/// What the bid step leads the market to (#494): stone, which Settle in's
/// workbench knaps at home ([`Step::SetAutomation`]). An empty book still
/// takes a bid, so the step never waits on a seller (plan D10).
pub const BID_FOR: &str = "stone";

/// The claim step's words while the city grants no deeds (`Grant::Off`,
/// #451): a home is bought, and the step is done by any claim.
const CLAIM_BOUGHT: &str = "The city isn't granting deeds just now, so a home is bought: Alt-click a free lot in the three rings and pay its deed from the lot's panel.";

/// A step's words on its card: [`words_in`] how you play (`play`), but for
/// the market box once it's known empty (`box_goods`, `Some(false)`), the
/// claim while deeds aren't granted (`grant_off`), and Settle in's for your
/// own lot (`home`, [`home_words`]).
fn body(step: Step, box_goods: Option<bool>, home: Option<&Home>, play: Play, grant_off: bool) -> Cow<'static, str> {
    match (step, box_goods) {
        (Step::CollectBox, Some(false)) => BOX_EMPTY.into(),
        (Step::ClaimHome, _) if grant_off => CLAIM_BOUGHT.into(),
        _ => home_words(step, home).map_or(words_in(step, play).1.into(), Cow::Owned),
    }
}

/// What the card does this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Card {
    /// The step to do, and when its card came up.
    Step(Step, Instant),
    /// A step just done, with its tick, and when it was.
    Done(Step, Instant),
    /// A chapter just finished, and when its card came up.
    ChapterDone(Chapter, Instant),
    /// Every step done; when the last one was.
    Closing(Instant),
    /// The quality card (#443), in the bench kit's place until it's read,
    /// and when it came up.
    Lesson(Instant),
}

/// Which card is drawn, for remembering each one's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Step(Step),
    Chapter(Chapter),
    Closing,
    Lesson,
}

/// Where the step at hand's place is, as the extension found it this frame
/// (`onboarding::target`), for the card's line on where to go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lead {
    /// How far it is from you, in metres.
    pub metres: f32,
    /// Which way it lies on screen while it's off it, as `hud::bearing`
    /// gives it. `None` while it's on screen, where its ring marks it.
    pub off_screen: Option<f32>,
    /// It's a lot, and someone owns it: the road in's stop prefers a free
    /// one, but one that's taken teaches as well (#445).
    pub taken: bool,
}

/// The card's line under the lead for a stop that's taken (#445).
const TAKEN: &str = "This one's taken; most near the road are. Its deed reads the same.";

/// Inside this, you're there (metres).
const HERE: f32 = 6.0;

/// A card to open with nobody in sight (#447): a name in print opens one too.
/// A world with nobody in it at all can't do the step
/// ([#484](https://github.com/chrischaps/TheCapitol/issues/484)).
const NOBODY_NEAR: &str = "Nobody in sight. A name in the Gazette opens a card too.";

/// How far, as the card says it: "15 m", "370 m", "1.2 km".
pub fn far(metres: f32) -> String {
    if metres >= 1000.0 {
        format!("{:.1} km", metres / 1000.0)
    } else {
        let to = if metres < 100.0 { 5.0 } else { 10.0 };
        format!("{} m", ((metres / to).round() * to) as i32)
    }
}

/// The card's line on where to go.
fn lead_words(lead: Lead) -> String {
    if lead.metres < HERE {
        return "Here, ringed in gold".to_owned();
    }
    let far = far(lead.metres);
    match lead.off_screen {
        Some(_) => format!("{far} that way"),
        None => format!("{far} off, ringed in gold"),
    }
}

/// Where the road in stands while its card is hidden, for the header's
/// chip: the chapter, and the step at hand's place in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hidden {
    pub chapter: Chapter,
    /// The step at hand's place in its chapter, from 0
    pub at: usize,
    /// How many steps the chapter has
    pub of: usize,
}

#[derive(Default)]
pub struct Hints {
    /// The tour as the server last said: `None` until it has, and from a
    /// server older than the road in, which shows no card.
    tour: Option<Tour>,
    /// The steps told to the server this session ([`Hints::saw`]), so each
    /// is told once while its answer is on the way.
    told: Steps,
    /// Hidden (or brought back) as asked, until the server says so too.
    asked_hidden: Option<bool>,
    card: Option<Card>,
    /// `--hints-sample <chapter>-done`: that chapter's close, shown from
    /// the first frame drawn.
    sample_close: Option<Chapter>,
    /// What your market box holds, as last fetched (#446): `Some(true)`
    /// something waits, `Some(false)` it's empty, `None` not fetched.
    box_goods: Option<bool>,
    /// Since when it's been known empty, for the waiver.
    box_empty_since: Option<Instant>,
    /// Where the card was drawn, for panels that need to keep clear.
    pub rect: Option<egui::Rect>,
    /// Each card's size, full and compact, as last drawn: where it fits
    /// is judged from these (`place`).
    sizes: Vec<((Face, bool), egui::Vec2)>,
    /// The quality card's numbers, from the pack this frame
    /// ([`Hints::set_lesson`]): `None` while there's no rope to talk about.
    lesson: Option<QualityLesson>,
    /// The quality card was read this session ("Go on").
    lesson_read: bool,
    /// Your home, for Settle in's words (#449, [`Hints::set_home`]).
    home: Option<Home>,
    /// The tax card's "Noted" was clicked: [`Hints::noted`] tells it.
    noted: bool,
    /// The bid step has led the market to [`BID_FOR`] ([`Hints::bid_lead`]).
    bid_led: bool,
    /// Over the shoulder (T1.6, [`Hints::set_direct`]): the walk step says WASD.
    direct: bool,
    /// With a pad in your hands (T2.2, [`Hints::set_pad`]): the cards say its buttons.
    pad: bool,
}

impl Hints {
    /// Follow the tour as the server has it (`GameState::tutorial`): `None`
    /// signed out, or before it's said.
    pub fn follow(&mut self, progress: Option<TutorialProgress>) {
        let tour = progress.map(Tour::from);
        if tour.is_none() && self.tour.is_some() {
            // Signed out: the next account starts clean
            *self = Hints { sizes: std::mem::take(&mut self.sizes), ..Hints::default() };
        }
        if let (Some(tour), Some(asked)) = (tour, self.asked_hidden) {
            if tour.progress().dismissed == asked {
                self.asked_hidden = None;
            }
        }
        self.tour = tour;
    }

    /// The screen showed `did`. The step to tell the server
    /// (`tutorial_seen`), the first time it's worth telling.
    pub fn saw(&mut self, did: Did) -> Option<Step> {
        let step = self.tour?.report(did)?;
        if self.told.has(step) {
            return None;
        }
        self.told = self.told.with(step);
        tracing::info!("hints: saw {}", step.name());
        Some(step)
    }

    /// What your market box holds, as last fetched (`hall::goods_waiting`
    /// of `GameState::market_box`): `None` before it's fetched. Call once a
    /// frame, before [`Hints::waive`].
    pub fn market_box(&mut self, goods: Option<bool>, now: Instant) {
        match goods {
            Some(false) => {
                self.box_empty_since.get_or_insert(now);
            }
            _ => self.box_empty_since = None,
        }
        self.box_goods = goods;
    }

    /// The market box is waived with words (#446, plan D10): its card is up
    /// and shown, the box is known empty, and the card has said so for
    /// [`WAIVE_AFTER`]. The step to tell the server (`tutorial_seen`), once.
    /// The server looks in the box too, so a fill on its way is collected,
    /// not waived.
    pub fn waive(&mut self, now: Instant) -> Option<Step> {
        let Some(Card::Step(Step::CollectBox, up)) = self.card else {
            return None;
        };
        let empty = self.box_empty_since?;
        let said = now.saturating_duration_since(up.max(empty));
        if self.is_hidden() || said < WAIVE_AFTER || self.told.has(Step::CollectBox) {
            return None;
        }
        self.told = self.told.with(Step::CollectBox);
        tracing::info!("hints: nothing in the market box, so it's waived");
        Some(Step::CollectBox)
    }

    /// `--hints-sample`: follow the sample tour `which` names ([`sample`]),
    /// and for `<chapter>-done`, open on that chapter's close.
    /// `collect_box-empty` is the market box's card with the box empty.
    pub fn with_sample(&mut self, which: &str) {
        self.follow(Some(sample(which)));
        if which == "collect_box-empty" {
            self.market_box(Some(false), Instant::now());
        }
        self.sample_close = which
            .strip_suffix("-done")
            .and_then(|key| Chapter::ALL.into_iter().find(|c| chapter_key(*c) == key));
        // At the bench kit, the quality card first, on a sample pack
        self.lesson = super::quality_card::sample();
        // Settling in, on a sample farm
        let settling = self.tour.and_then(Tour::current).is_some_and(|s| s.chapter() == Chapter::SettleIn);
        self.home = settling.then(sample_home);
    }

    /// The quality card's numbers (#443), from the pack and crafting's
    /// slots this frame (`QualityLesson::of`). Call before [`Hints::show`].
    /// While the card is up it keeps its last numbers through a frame with
    /// none (the rope between crafting's output and the pack), so it doesn't
    /// blink to the kit's card and back under the pointer.
    pub fn set_lesson(&mut self, lesson: Option<QualityLesson>) {
        if lesson.is_some() || !matches!(self.card, Some(Card::Lesson(_))) {
            self.lesson = lesson;
        }
    }

    /// The control mode in play, over the shoulder or not (T1.6): the
    /// cards speak of its keys. Call once a frame.
    pub fn set_direct(&mut self, direct: bool) {
        self.direct = direct;
    }

    /// Whether the pad was the last thing used (T2.2): over the shoulder
    /// the cards then speak of its sticks and buttons. Call once a frame.
    pub fn set_pad(&mut self, pad: bool) {
        self.pad = pad;
    }

    /// How you're playing, as the cards speak of it.
    fn play(&self) -> Play {
        match (self.direct, self.pad) {
            (false, _) => Play::Click,
            (true, false) => Play::Keys,
            (true, true) => Play::Pad,
        }
    }

    /// Your home lot (`onboarding::home_plot` of your plots), for Settle
    /// in's words: what it fits and what it's taxed (#449). Call once a
    /// frame; the room is packed once a lot, not every frame.
    pub fn set_home(&mut self, plot: Option<&PlotInfo>) {
        self.home = plot.and_then(|p| Home::of(p, self.home.as_ref()));
    }

    /// The tax card's "Noted" was clicked since last asked: the step to tell
    /// the server (`tutorial_seen { read_tax }`), the first time.
    pub fn noted(&mut self) -> Option<Step> {
        if !std::mem::take(&mut self.noted) {
            return None;
        }
        self.saw(Did::ReadTax)
    }

    /// The quality card is still to show: there's a rope to talk about, and
    /// it hasn't been read this session.
    fn lesson_due(&self) -> bool {
        self.lesson.is_some() && !self.lesson_read
    }

    /// The card to show now: [`next_card`]'s, but with the quality card in
    /// the bench kit's place, between twisting the rope and the kit (plan
    /// §4.1), until it's read or the kit is made.
    fn card_now(&self, tour: Tour, now: Instant) -> Option<Card> {
        let at_kit = tour.current() == Some(Step::MakeWorkbench);
        match self.card {
            Some(Card::Lesson(at)) if at_kit && self.lesson_due() => Some(Card::Lesson(at)),
            Some(Card::Lesson(_)) => next_card(None, tour, now),
            card => match next_card(card, tour, now) {
                Some(Card::Step(Step::MakeWorkbench, _)) if self.lesson_due() => Some(Card::Lesson(now)),
                next => next,
            },
        }
    }

    /// What to open the market on for the bid step (#494): [`BID_FOR`],
    /// once a session, while the step's card is up. Call while the market
    /// is open, so the lead lands where it's seen.
    pub fn bid_lead(&mut self) -> Option<&'static str> {
        if self.bid_led || self.leading() != Some(Step::BuyOrder) {
            return None;
        }
        self.bid_led = true;
        Some(BID_FOR)
    }

    /// The tour as the server last said, if it has.
    pub fn progress(&self) -> Option<TutorialProgress> {
        self.tour.map(Tour::progress)
    }

    /// Whether the city grants no deeds now (#451), so the claim is bought
    fn grant_off(&self) -> bool {
        self.progress().is_some_and(|p| p.grant == Grant::Off)
    }

    /// The step whose card is up, shown and not yet done: what the card
    /// may lead to, when the extension has a place for it that
    /// `onboarding::target` can't name (the lot the Deeds Office showed
    /// you, #448).
    pub fn leading(&self) -> Option<Step> {
        match self.card {
            Some(Card::Step(step, _)) if !self.is_hidden() => Some(step),
            _ => None,
        }
    }

    /// Where the card at hand leads, if anywhere: only a step's card, shown,
    /// and not yet done. The extension finds the place and marks it.
    pub fn target(&self) -> Option<Target> {
        match self.card {
            Some(Card::Step(step, _)) if !self.is_hidden() => target(step),
            _ => None,
        }
    }

    /// The card is hidden for now, as asked here or by any device.
    fn is_hidden(&self) -> bool {
        self.tour
            .is_some_and(|t| !t.finished() && self.asked_hidden.unwrap_or(t.progress().dismissed))
    }

    /// Where the road in stands while its card is hidden; `None` while it
    /// shows, once it's finished, or before the server says.
    pub fn hidden(&self) -> Option<Hidden> {
        if !self.is_hidden() {
            return None;
        }
        let step = self.tour?.current()?;
        let (at, of) = place_in_chapter(step);
        Some(Hidden { chapter: step.chapter(), at, of })
    }

    /// Hide the card here at once; the caller asks the server.
    pub fn hide(&mut self) {
        self.asked_hidden = Some(true);
        self.card = None;
    }

    /// Bring the card back here at once; the caller asks the server.
    pub fn bring_back(&mut self) {
        self.asked_hidden = Some(false);
    }

    /// Draw the card, if there is one. Call once a frame while in the world.
    /// It keeps clear of the open `panels` where it can (#99): see `place`.
    /// `lead` is where the step's target is, if it has one and it was found.
    /// True if "Hide for now" was clicked: the card is hidden here, and the
    /// caller tells the server (`set_tutorial_dismissed`).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        panels: &[egui::Rect],
        lead: Option<Lead>,
    ) -> bool {
        self.rect = None;
        let Some(tour) = self.tour else {
            return false;
        };
        if self.is_hidden() {
            self.card = None;
            return false;
        }
        if let Some(chapter) = self.sample_close.take() {
            self.card = Some(match next_chapter(chapter) {
                Some(_) => Card::ChapterDone(chapter, now),
                None => Card::Closing(now),
            });
        }
        self.card = self.card_now(tour, now);
        let Some(card) = self.card else {
            return false;
        };
        let (face, since) = match card {
            Card::Step(step, at) | Card::Done(step, at) => (Face::Step(step), at),
            Card::ChapterDone(chapter, at) => (Face::Chapter(chapter), at),
            Card::Closing(at) => (Face::Closing, at),
            Card::Lesson(at) => (Face::Lesson, at),
        };
        let age = now.saturating_duration_since(since).as_secs_f32();
        let fading = |stays: Duration| {
            let left = stays.as_secs_f32() - age;
            (age / FADE_IN)
                .min(1.0)
                .min((left / FADE_OUT).clamp(0.0, 1.0))
        };
        let alpha = match card {
            Card::Step(..) | Card::Lesson(_) => (age / FADE_IN).min(1.0),
            Card::Done(..) => 1.0,
            Card::ChapterDone(..) => fading(CHAPTER_FOR),
            Card::Closing(_) => fading(CLOSING_FOR),
        };
        let eased = 1.0 - (1.0 - alpha).powi(3);
        let mut hide = false;
        let mut read = false;
        let mut noted = false;
        // Under the header, as the panels are (anchors are screen-relative).
        let top = ctx.available_rect().top() + 12.0;
        let size = |compact| {
            self.sizes
                .iter()
                .find(|(k, _)| *k == (face, compact))
                .map(|(_, v)| *v)
        };
        let spot = place(ctx.screen_rect(), top, [size(false), size(true)], panels);
        let right = ctx.animate_value_with_time(Id::new("capitol_hint_inset"), spot.right, SLIDE);
        let shown = egui::Area::new(Id::new("capitol_hint"))
            .anchor(Align2::RIGHT_TOP, [-right, top - RISE * (1.0 - eased)])
            .order(if spot.in_front {
                egui::Order::Foreground
            } else {
                egui::Order::Middle
            })
            .show(ctx, |ui| {
                ui.multiply_opacity(eased);
                theme::panel_edged(theme::GOLD).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    match face {
                        Face::Step(step) => {
                            let done = matches!(card, Card::Done(..));
                            let lead = lead.filter(|_| !done);
                            let body = body(step, self.box_goods, self.home.as_ref(), self.play(), self.grant_off());
                            let asked = step_card(ui, step, &body, done, spot.compact, lead, self.play());
                            hide = asked.hide;
                            noted = asked.noted;
                        }
                        Face::Chapter(chapter) => chapter_card(ui, chapter, spot.compact),
                        Face::Closing => closing_card(ui, spot.compact),
                        Face::Lesson => {
                            if let Some(lesson) = &self.lesson {
                                read = super::quality_card::show(ui, lesson, spot.compact);
                            }
                        }
                    }
                });
            });
        let key = (face, spot.compact);
        let drawn = shown.response.rect.size();
        match self.sizes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => *v = drawn,
            None => self.sizes.push((key, drawn)),
        }
        // A compact card's words are a hover away. Anywhere on the card:
        // the area's own response isn't hovered while a child widget is.
        if spot.compact && shown.response.contains_pointer() {
            let (title, body) = match face {
                Face::Step(s) => (words(s).0.to_owned(), body(s, self.box_goods, self.home.as_ref(), self.play(), self.grant_off())),
                Face::Chapter(c) => (chapter_done_title(c), chapter_close(c).into()),
                Face::Closing => (CLOSING.0.to_owned(), CLOSING.1.into()),
                Face::Lesson => (super::quality_card::TITLE.to_owned(), super::quality_card::WORDS.into()),
            };
            let layer = shown.response.layer_id;
            egui::show_tooltip_at_pointer(ctx, layer, Id::new("capitol_hint_words"), |ui| {
                ui.set_max_width(WIDTH);
                ui.label(theme::title(&title, size::BODY));
                ui.label(RichText::new(body).color(theme::TEXT).size(size::BODY));
            });
        }
        self.rect = Some(shown.response.rect);
        if read {
            // On to the bench kit's card, next frame
            self.lesson_read = true;
        }
        if noted {
            // The caller tells the server ([`Hints::noted`])
            self.noted = true;
        }
        if hide {
            tracing::info!("hints: hidden for now");
            self.hide();
        }
        hide
    }
}

/// Where the card goes this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Spot {
    /// Its inset from the screen's right edge.
    right: f32,
    /// Its words go to a hover, leaving the title, the keys and Hide.
    compact: bool,
    /// Drawn over the panels, because no spot was clear of them.
    in_front: bool,
}

/// Where the card goes, at the top of the screen, given its `sizes` (full
/// and compact, as last drawn; `None` until drawn) and the open `panels`.
/// The first spot clear of every panel wins: its corner, then just left of
/// a panel that reaches up into the corner, full and then compact. A small
/// screen with the crafting panel and the inventory both open has no such
/// spot (1280×720 at 150% is 853×480 points), so there the compact card
/// takes its corner, in front of the inventory rather than hidden by it.
fn place(
    screen: egui::Rect,
    top: f32,
    sizes: [Option<egui::Vec2>; 2],
    panels: &[egui::Rect],
) -> Spot {
    let at = |right: f32, size: egui::Vec2| {
        egui::Rect::from_min_size(egui::pos2(screen.right() - right - size.x, top), size)
    };
    // Within GAP of it; only touching at GAP is clear.
    let meets = |r: egui::Rect, p: &egui::Rect| r.expand(GAP).intersect(*p).is_positive();
    let clear =
        |r: egui::Rect| r.left() >= screen.left() + INSET && !panels.iter().any(|p| meets(r, p));
    for (compact, size) in [false, true].into_iter().zip(sizes) {
        // Not drawn yet: the corner, to learn its size.
        let Some(size) = size else {
            return Spot {
                right: INSET,
                compact,
                in_front: false,
            };
        };
        let corner = at(INSET, size);
        let beside = panels
            .iter()
            .filter(|p| meets(corner, p))
            .map(|p| screen.right() - p.left() + GAP);
        if let Some(right) = std::iter::once(INSET)
            .chain(beside)
            .find(|&right| clear(at(right, size)))
        {
            return Spot {
                right,
                compact,
                in_front: false,
            };
        }
    }
    Spot {
        right: INSET,
        compact: true,
        in_front: true,
    }
}

/// The card to show now, given the last one and the tour.
fn next_card(card: Option<Card>, tour: Tour, now: Instant) -> Option<Card> {
    let current = tour.current();
    match card {
        // A step's card stays until it's done, then shows its tick.
        Some(Card::Step(step, at)) if current == Some(step) => Some(Card::Step(step, at)),
        Some(Card::Step(step, _)) if tour.done(step) => Some(Card::Done(step, now)),
        Some(Card::Done(step, at)) if now.saturating_duration_since(at) < TICK_FOR => {
            Some(Card::Done(step, at))
        }
        Some(Card::ChapterDone(chapter, at)) if now.saturating_duration_since(at) < CHAPTER_FOR => {
            Some(Card::ChapterDone(chapter, at))
        }
        Some(Card::Closing(at)) if now.saturating_duration_since(at) < CLOSING_FOR => {
            Some(Card::Closing(at))
        }
        Some(Card::Closing(_)) => None,
        // After a tick, or with nothing up: the chapter's close if that
        // tick finished one, else the step at hand; or the closing card if
        // the tour was just finished here (not finished before this
        // session).
        last => match (current, last) {
            (Some(_), Some(Card::Done(done, _))) if tour.chapter_done(done.chapter()) => {
                Some(Card::ChapterDone(done.chapter(), now))
            }
            (Some(step), _) => Some(Card::Step(step, now)),
            (None, Some(Card::Done(..) | Card::ChapterDone(..))) if tour.finished() => {
                Some(Card::Closing(now))
            }
            (None, _) => None,
        },
    }
}

/// What a step's card was asked this frame.
#[derive(Debug, Clone, Copy, Default)]
struct Asked {
    /// "Hide for now"
    hide: bool,
    /// The tax card's "Noted" (#449)
    noted: bool,
}

/// A step's card: its chapter and place in it, then the step, its `body`
/// (its words, as [`body`] picks them), and where it leads if anywhere;
/// `compact` leaves out the words. The tax card, which asks nothing of the
/// world, ends with "Noted", which is the step. Its keys are the mode's
/// (`play`, [`words_in`]), and with a pad, the pad's where it has them
/// ([`on_the_pad`]).
fn step_card(ui: &mut egui::Ui, step: Step, body: &str, done: bool, compact: bool, lead: Option<Lead>, play: Play) -> Asked {
    let (title, _, keys) = words_in(step, play);
    let keys: Cow<str> = if play == Play::Pad { on_the_pad(keys).into() } else { keys.into() };
    let (at, of) = place_in_chapter(step);
    let mut hide = false;
    let mut noted = false;
    ui.horizontal(|ui| {
        ui.label(theme::label(chapter_name(step.chapter())));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            hide = theme::link(ui, "Hide for now", size::SMALL)
                .on_hover_text("The road in waits: the header's chip brings it back")
                .clicked();
        });
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        theme::step_pips(ui, of, at + usize::from(done));
        ui.add_space(4.0);
        ui.label(theme::dim(format!("{} of {of}", at + 1)));
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let title = ui.label(theme::title(title, size::HEADING + 2.0));
        if done {
            let h = title.rect.height() * 0.8;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(h, h), egui::Sense::hover());
            theme::tick(ui.painter(), rect, theme::GOLD_LIGHT);
        }
    });
    if !compact {
        ui.add_space(2.0);
        ui.label(RichText::new(body).color(theme::TEXT).size(size::BODY));
    }
    if let Some(lead) = lead {
        ui.add_space(4.0);
        lead_row(ui, lead);
        if lead.taken {
            ui.label(theme::dim(TAKEN));
        }
    } else if !done && target(step) == Some(Target::Person) {
        // Nobody in sight to lead to (#447): there are other ways to a card
        ui.add_space(4.0);
        ui.label(theme::dim(NOBODY_NEAR).italics());
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        keys_row(ui, &keys);
    });
    if step == Step::ReadTax && !done {
        ui.add_space(8.0);
        noted = theme::accent_button(ui, NOTED, theme::GOLD, true)
            .on_hover_text("Read and understood: on to the last card")
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked();
    }
    Asked { hide, noted }
}

/// Where the step leads: the hall chip's arrow, turned the way it lies,
/// while it's off screen; a small gold ring, as on the ground, while it's
/// on; and how far.
fn lead_row(ui: &mut egui::Ui, lead: Lead) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        match lead.off_screen {
            Some(angle) => super::hud::arrow(ui.painter(), rect.center(), angle, theme::GOLD),
            None => {
                let t = ui.input(|i| i.time) as f32;
                let r = 4.2 + 0.5 * (t * 1.6).sin();
                ui.painter()
                    .circle_stroke(rect.center(), r, egui::Stroke::new(1.5_f32, theme::GOLD));
                ui.ctx().request_repaint();
            }
        }
        ui.label(
            RichText::new(lead_words(lead))
                .font(theme::strong(size::SMALL))
                .color(theme::GOLD_LIGHT),
        );
    });
}

/// A chapter's close: its title.
fn chapter_done_title(chapter: Chapter) -> String {
    format!("{}: done", chapter_name(chapter))
}

/// A chapter's close: which of the six it was, every pip gold and a seal
/// pressed beside the title, as a clerk stamps a finished form; what you
/// can now do, and the chapter that comes next. `compact` leaves out the
/// line.
fn chapter_card(ui: &mut egui::Ui, chapter: Chapter, compact: bool) {
    let steps = chapter.steps().count();
    let n = Chapter::ALL.iter().position(|c| *c == chapter).unwrap_or(0) + 1;
    ui.label(theme::label(&format!("Chapter {n} of {}", Chapter::ALL.len())));
    ui.add_space(2.0);
    theme::step_pips(ui, steps, steps);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let title = ui.label(theme::title(&chapter_done_title(chapter), size::HEADING + 2.0));
        let h = title.rect.height();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(h, h), egui::Sense::hover());
        theme::wax_seal(ui.painter(), rect.center(), h * 0.42);
        theme::tick(ui.painter(), rect.shrink(h * 0.3), theme::PAPER);
    });
    if !compact {
        ui.add_space(2.0);
        ui.label(RichText::new(chapter_close(chapter)).color(theme::TEXT).size(size::BODY));
    }
    if let Some(next) = next_chapter(chapter) {
        ui.add_space(6.0);
        ui.label(theme::dim(format!("Next: {}", chapter_name(next))));
    }
}

/// The closing card's title and words (#449).
const CLOSING: (&str, &str) = (
    "Well begun",
    "You've walked the road in and made a home, and the city has you on its books. \
     The rest is yours: make, trade, and make your name.",
);

/// The closing card's way on: where the city says what it's doing, and
/// what it wants (the board's two pages, S2.3). G opens the Gazette's page,
/// and the Wanted tab is beside it.
const CLOSING_ON: &str = "the Gazette each morning, and Wanted: work people will pay for";

/// The last card: the road is walked, the seal pressed, and where to look
/// from here. `compact` leaves out its words.
fn closing_card(ui: &mut egui::Ui, compact: bool) {
    let (title, body) = CLOSING;
    // A pip a chapter now, every one gold
    let chapters = Chapter::ALL.len();
    ui.label(theme::label(&format!("The road in · {chapters} of {chapters}")));
    ui.add_space(2.0);
    theme::step_pips(ui, chapters, chapters);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let title = ui.label(theme::title(title, size::HEADING + 2.0));
        // The city's seal, as on the Registrar's letter: the road's last stamp
        let h = title.rect.height();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(h, h), egui::Sense::hover());
        theme::capitol_seal(ui.painter(), rect.center(), h * 0.46);
    });
    if !compact {
        ui.add_space(2.0);
        ui.label(RichText::new(body).color(theme::TEXT).size(size::BODY));
    }
    ui.add_space(6.0);
    for (key, what) in [("G", CLOSING_ON), ("?", "every control, any time")] {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            theme::keycap(ui, key);
            ui.label(theme::dim(what));
        });
    }
}

/// A chapter's name in `--hints-sample`: the protocol's, `road_in`.
fn chapter_key(chapter: Chapter) -> &'static str {
    match chapter {
        Chapter::Arrival => "arrival",
        Chapter::RoadIn => "road_in",
        Chapter::Hall => "hall",
        Chapter::People => "people",
        Chapter::DeedsOffice => "deeds_office",
        Chapter::SettleIn => "settle_in",
    }
}

/// `--hints-sample`'s home for Settle in: a farm as the default world's
/// typically are since Plots v2 (12 stations of a deed's 25, #459), on a
/// granted deed assessed at the home zones' one price, 500, so 10 a week
/// (#292).
fn sample_home() -> Home {
    Home {
        plot_id: uuid::Uuid::nil(),
        zone: capitol_core::protocol::rules::Zone::Rural,
        fits: 12,
        capacity: 25,
        placed: 0,
        assessed: 500,
        weekly_tax: Some(10),
        owed: 0,
    }
}

/// Every step before `step`, in the tour's order.
fn up_to(step: Step) -> Steps {
    Step::ALL.into_iter().take_while(|s| *s != step).collect()
}

/// `--hints-sample`: the card on a fresh tour (`new`, the default), half
/// way up the road (`half`), or hidden there, for the header's chip and
/// the `?` overlay's line (`hidden`, `help`). A step's name (`gather_stone`)
/// stands the tour at that step, and a chapter's (`hall`) at its first.
/// `<chapter>-done` (`arrival-done`) stands it just past that chapter,
/// for its close ([`Hints::with_sample`]).
pub fn sample(which: &str) -> TutorialProgress {
    let at = |done: Steps| TutorialProgress { done, dismissed: false, grant: Grant::Unearned.after(done) };
    let chapter = |key: &str| Chapter::ALL.into_iter().find(|c| chapter_key(*c) == key);
    let first = |c: Chapter| c.steps().next().expect("a chapter has steps");
    if let Some(step) = Step::from_name(which) {
        return at(up_to(step));
    }
    if let Some(c) = chapter(which) {
        return at(up_to(first(c)));
    }
    if let Some(c) = which.strip_suffix("-done").and_then(chapter) {
        return at(match next_chapter(c) {
            Some(next) => up_to(first(next)),
            None => Steps::every(),
        });
    }
    let half = up_to(Step::SellOrder);
    match which {
        "collect_box-empty" => at(up_to(Step::CollectBox)),
        "half" => at(half),
        "hidden" | "help" => TutorialProgress { done: half, dismissed: true, ..Default::default() },
        _ => TutorialProgress::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tour(done: &[Step]) -> Tour {
        Tour::from(TutorialProgress { done: done.iter().copied().collect(), dismissed: false, ..Default::default() })
    }

    #[test]
    fn the_card_keeps_clear_of_the_panels_where_it_can() {
        let spot = |right, compact, in_front| Spot {
            right,
            compact,
            in_front,
        };
        let rect = |l, t, r, b| egui::Rect::from_min_max(egui::pos2(l, t), egui::pos2(r, b));
        // 1280×720 at 150%, in points, as measured from a live capture.
        let screen = rect(0.0, 0.0, 853.0, 480.0);
        let top = 60.0;
        let sizes = [
            Some(egui::vec2(277.0, 167.0)),
            Some(egui::vec2(277.0, 125.0)),
        ];
        let inventory = rect(485.0, 173.0, 834.0, 460.0);
        let crafting = rect(15.0, 60.0, 317.0, 390.0);

        // Nothing open: its corner.
        assert_eq!(place(screen, top, sizes, &[]), spot(INSET, false, false));
        // The inventory on a taller screen stays below it: its corner.
        let big = rect(0.0, 0.0, 1280.0, 720.0);
        let low = inventory.translate(egui::vec2(427.0, 240.0));
        assert_eq!(place(big, top, sizes, &[low]), spot(INSET, false, false));
        // The inventory reaching up into the corner: just left of it.
        let beside = 853.0 - inventory.left() + GAP;
        assert_eq!(
            place(screen, top, sizes, &[inventory]),
            spot(beside, false, false)
        );
        // Crafting open too: nowhere is clear, so compact, in front.
        assert_eq!(
            place(screen, top, sizes, &[inventory, crafting]),
            spot(INSET, true, true)
        );
        // A compact card that clears the corner sits there, behind nothing.
        let shorter = rect(485.0, 210.0, 834.0, 460.0);
        assert_eq!(
            place(screen, top, sizes, &[shorter, crafting]),
            spot(INSET, true, false)
        );
        // Not drawn yet: the corner, to measure it; then compact, likewise.
        assert_eq!(
            place(screen, top, [None, None], &[inventory]),
            spot(INSET, false, false)
        );
        assert_eq!(
            place(screen, top, [sizes[0], None], &[inventory, crafting]),
            spot(INSET, true, false)
        );
    }

    #[test]
    fn a_done_step_ticks_then_gives_way() {
        let t0 = Instant::now();
        let card = next_card(None, tour(&[]), t0);
        assert_eq!(card, Some(Card::Step(Step::Walk, t0)));
        // The server says the walk is done
        let walked = tour(&[Step::Walk]);
        let t1 = t0 + Duration::from_secs(3);
        let card = next_card(card, walked, t1);
        assert_eq!(card, Some(Card::Done(Step::Walk, t1)));
        let card = next_card(card, walked, t1 + TICK_FOR / 2);
        assert_eq!(card, Some(Card::Done(Step::Walk, t1)));
        let t2 = t1 + TICK_FOR;
        assert_eq!(
            next_card(card, walked, t2),
            Some(Card::Step(Step::GatherGrass, t2))
        );
    }

    #[test]
    fn the_closing_card_comes_once_and_only_after_the_last_step() {
        let t0 = Instant::now();
        let (last, rest) = Step::ALL.split_last().unwrap();
        let card = next_card(None, tour(rest), t0);
        assert_eq!(card, Some(Card::Step(*last, t0)));
        let finished = tour(&Step::ALL);
        let card = next_card(card, finished, t0);
        let t1 = t0 + TICK_FOR;
        let card = next_card(card, finished, t1);
        assert_eq!(card, Some(Card::Closing(t1)));
        assert_eq!(next_card(card, finished, t1 + CLOSING_FOR), None);
        // Finished in an earlier session: nothing at all.
        assert_eq!(next_card(None, finished, t1), None);
    }

    #[test]
    fn a_finished_chapter_has_its_close_a_beat_longer_then_the_next() {
        let t0 = Instant::now();
        let arrival: Vec<Step> = Chapter::Arrival.steps().collect();
        let (last, rest) = arrival.split_last().unwrap();
        let card = next_card(None, tour(rest), t0);
        assert_eq!(card, Some(Card::Step(*last, t0)));
        let walked = tour(&arrival);
        let card = next_card(card, walked, t0);
        assert_eq!(card, Some(Card::Done(*last, t0)));
        let t1 = t0 + TICK_FOR;
        let card = next_card(card, walked, t1);
        assert_eq!(card, Some(Card::ChapterDone(Chapter::Arrival, t1)));
        assert!(CHAPTER_FOR > TICK_FOR * 3, "a beat longer");
        let card = next_card(card, walked, t1 + CHAPTER_FOR / 2);
        assert_eq!(card, Some(Card::ChapterDone(Chapter::Arrival, t1)));
        let t2 = t1 + CHAPTER_FOR;
        assert_eq!(next_card(card, walked, t2), Some(Card::Step(Step::SeeRuralLot, t2)));

        // A step that doesn't finish its chapter goes straight on
        let card = next_card(None, tour(&[]), t0);
        let card = next_card(card, tour(&[Step::Walk]), t0);
        assert_eq!(next_card(card, tour(&[Step::Walk]), t1), Some(Card::Step(Step::GatherGrass, t1)));

        // The last chapter's close is the closing card
        let card = Some(Card::Done(Step::ReadTax, t0));
        assert_eq!(next_card(card, tour(&Step::ALL), t1), Some(Card::Closing(t1)));
    }

    /// The quality card comes between the rope and the bench kit (plan
    /// §4.1), in the kit's place, once, and only with a rope to talk about
    #[test]
    fn the_quality_card_stands_between_the_rope_and_the_kit() {
        let t0 = Instant::now();
        let mut hints = Hints::default();
        hints.with_sample("twist_rope");
        let rope = tour(&up_to(Step::MakeWorkbench).iter().collect::<Vec<_>>());
        // The rope's tick, then the card instead of the kit's
        hints.card = Some(Card::Done(Step::TwistRope, t0));
        let t1 = t0 + TICK_FOR;
        assert_eq!(hints.card_now(rope, t1), Some(Card::Lesson(t1)));
        hints.card = hints.card_now(rope, t1);
        assert_eq!(hints.target(), None, "it leads nowhere");
        let t2 = t1 + Duration::from_secs(30);
        assert_eq!(hints.card_now(rope, t2), Some(Card::Lesson(t1)), "it waits to be read");
        // Read: the kit's card
        hints.lesson_read = true;
        assert_eq!(hints.card_now(rope, t2), Some(Card::Step(Step::MakeWorkbench, t2)));

        // No rope in the pack (sold, say): straight to the kit
        let mut hints = Hints::default();
        hints.follow(Some(TutorialProgress { done: up_to(Step::MakeWorkbench), dismissed: false, ..Default::default() }));
        hints.set_lesson(None);
        assert_eq!(hints.card_now(rope, t0), Some(Card::Step(Step::MakeWorkbench, t0)));
        // A session that starts at the kit has it first
        hints.set_lesson(super::super::quality_card::sample());
        assert_eq!(hints.card_now(rope, t0), Some(Card::Lesson(t0)));
        // The kit made before it's read: on, without it
        hints.card = Some(Card::Lesson(t0));
        let kit = tour(&up_to(Step::PlaceWorkbench).iter().collect::<Vec<_>>());
        assert_eq!(hints.card_now(kit, t1), Some(Card::Step(Step::PlaceWorkbench, t1)));
    }

    #[test]
    fn the_bench_leads_to_a_clear_spot() {
        let mut hints = Hints::default();
        hints.with_sample("place_workbench");
        hints.card = next_card(None, hints.tour.unwrap(), Instant::now());
        assert_eq!(hints.target(), Some(Target::ClearSpot));
    }

    #[test]
    fn only_a_step_card_shown_leads_anywhere() {
        let mut hints = Hints::default();
        hints.with_sample("gather_stone");
        assert_eq!(hints.target(), None, "nothing until the card is up");
        hints.card = next_card(None, hints.tour.unwrap(), Instant::now());
        assert_eq!(hints.target(), Some(Target::Node(&["rock_outcrop"])));
        hints.hide();
        assert_eq!(hints.target(), None, "hidden is quiet");
        hints.bring_back();
        hints.card = Some(Card::ChapterDone(Chapter::Arrival, Instant::now()));
        assert_eq!(hints.target(), None);
    }

    #[test]
    fn the_samples_stand_where_they_say() {
        assert_eq!(Tour::from(sample("gather_stone")).current(), Some(Step::GatherStone));
        assert_eq!(Tour::from(sample("hall")).current(), Some(Step::Deposit));
        assert_eq!(Tour::from(sample("arrival-done")).current(), Some(Step::SeeRuralLot));
        assert!(Tour::from(sample("arrival-done")).chapter_done(Chapter::Arrival));
        assert!(Tour::from(sample("settle_in-done")).finished());
        let mut hints = Hints::default();
        hints.with_sample("people-done");
        assert_eq!(hints.sample_close, Some(Chapter::People));
        hints.with_sample("new");
        assert_eq!(hints.sample_close, None);
    }

    #[test]
    fn the_way_is_said_plainly() {
        let lead = |metres, off_screen| lead_words(Lead { metres, off_screen, taken: false });
        assert_eq!(lead(3.0, None), "Here, ringed in gold");
        assert_eq!(lead(3.0, Some(1.0)), "Here, ringed in gold");
        assert_eq!(lead(42.0, None), "40 m off, ringed in gold");
        assert_eq!(lead(143.0, Some(0.5)), "140 m that way");
        assert_eq!(lead(3480.0, Some(0.5)), "3.5 km that way");
    }

    #[test]
    fn hiding_holds_until_brought_back_and_the_server_agrees() {
        let mut hints = Hints::default();
        assert_eq!(hints.hidden(), None, "nothing before the server says");
        let half = sample("half");
        hints.follow(Some(half));
        assert_eq!(hints.hidden(), None);

        // Hidden here at once, before the server's answer
        hints.hide();
        let hidden = hints.hidden().expect("hidden");
        assert_eq!((hidden.chapter, hidden.at, hidden.of), (Chapter::Hall, 1, 6));
        hints.follow(Some(half));
        assert!(hints.hidden().is_some(), "an old word from the server doesn't undo it");
        hints.follow(Some(sample("hidden")));
        assert!(hints.hidden().is_some());

        // Brought back, as on another device
        hints.bring_back();
        assert_eq!(hints.hidden(), None);
        hints.follow(Some(half));
        hints.follow(Some(sample("hidden")));
        assert!(hints.hidden().is_some(), "once agreed, the server's word is the word");

        // A finished tour has nothing to hide
        hints.follow(Some(TutorialProgress { done: Steps::every(), dismissed: true, ..Default::default() }));
        assert_eq!(hints.hidden(), None);
    }

    #[test]
    fn what_the_screen_showed_is_told_once() {
        let mut hints = Hints::default();
        assert_eq!(hints.saw(Did::UsedInventory), None, "nobody to tell yet");
        hints.follow(Some(TutorialProgress::default()));
        assert_eq!(hints.saw(Did::UsedInventory), Some(Step::OpenPack));
        assert_eq!(hints.saw(Did::OpenedCrafting), None, "already on its way");
        hints.follow(Some(TutorialProgress { done: Steps::from_iter([Step::OpenPack]), dismissed: false, ..Default::default() }));
        assert_eq!(hints.saw(Did::UsedInventory), None, "done");
        // Signed out and in again: told afresh
        hints.follow(None);
        hints.follow(Some(TutorialProgress::default()));
        assert_eq!(hints.saw(Did::UsedInventory), Some(Step::OpenPack));
    }

    /// Nothing filled: the card says so, and after a read the step is
    /// waived (#446, plan D10). Not before the box is known, not with
    /// something in it, not while hidden, and once.
    #[test]
    fn the_market_box_is_waived_with_words_when_nothing_has_filled() {
        let t0 = Instant::now();
        let mut hints = Hints::default();
        hints.with_sample("collect_box");
        hints.card = next_card(None, hints.tour.unwrap(), t0);
        assert_eq!(hints.card, Some(Card::Step(Step::CollectBox, t0)));
        let later = t0 + WAIVE_AFTER * 3;

        hints.market_box(None, t0);
        assert_eq!(hints.waive(later), None, "not fetched: nothing's known");
        hints.market_box(Some(true), t0);
        assert_eq!(body(Step::CollectBox, hints.box_goods, None, Play::Click, false), words(Step::CollectBox).1);
        assert_eq!(hints.waive(later), None, "something waits: collect it");

        // Known empty from t1: it says so, and moves on once it's been read
        let t1 = t0 + Duration::from_secs(2);
        hints.market_box(Some(false), t1);
        hints.market_box(Some(false), t1 + Duration::from_secs(1));
        assert_eq!(body(Step::CollectBox, hints.box_goods, None, Play::Click, false), BOX_EMPTY);
        assert_eq!(hints.waive(t1 + WAIVE_AFTER - Duration::from_millis(10)), None, "still being read");
        hints.hide();
        hints.card = Some(Card::Step(Step::CollectBox, t0));
        assert_eq!(hints.waive(later), None, "hidden waits");
        hints.bring_back();
        assert_eq!(hints.waive(t1 + WAIVE_AFTER), Some(Step::CollectBox));
        assert_eq!(hints.waive(later), None, "told once");

        // Only the market box's card waives anything
        let mut hints = Hints::default();
        hints.with_sample("collect_box-empty");
        assert_eq!(hints.tour.unwrap().current(), Some(Step::CollectBox));
        assert_eq!(hints.box_goods, Some(false));
        hints.with_sample("buy_order");
        hints.card = next_card(None, hints.tour.unwrap(), t0);
        hints.market_box(Some(false), t0);
        assert_eq!(hints.waive(later), None);
        assert_eq!(body(Step::BuyOrder, Some(false), None, Play::Click, false), words(Step::BuyOrder).1);
        // While the city grants no deeds (#451), the claim is bought
        assert_eq!(body(Step::ClaimHome, None, None, Play::Click, true), CLAIM_BOUGHT);
        assert_eq!(body(Step::ClaimHome, None, None, Play::Click, false), words(Step::ClaimHome).1);
    }

    /// The bid step leads the market to stone once (#494), and says a bid
    /// needs no seller, since the book may be empty (D10). No other step
    /// leads it, and a hidden card doesn't.
    #[test]
    fn the_bid_step_leads_the_market_to_stone_once() {
        let t0 = Instant::now();
        let mut hints = Hints::default();
        hints.with_sample("sell_order");
        hints.card = next_card(None, hints.tour.unwrap(), t0);
        assert_eq!(hints.bid_lead(), None, "the sell step leads nowhere");

        let mut hints = Hints::default();
        hints.with_sample("buy_order");
        hints.asked_hidden = Some(true);
        hints.card = next_card(None, hints.tour.unwrap(), t0);
        assert_eq!(hints.bid_lead(), None, "hidden");
        hints.asked_hidden = Some(false);
        assert_eq!(hints.bid_lead(), Some(BID_FOR));
        assert_eq!(hints.bid_lead(), None, "once");
        assert!(words(Step::BuyOrder).1.contains("needs no seller"));
        assert!(words(Step::BuyOrder).1.contains("Try stone"));
    }

    /// Settle in speaks of your own lot (#449): what it fits against what
    /// its deed allows, and its tax in Strands. With no home listed, the
    /// plain words; nothing else changes.
    #[test]
    fn settling_in_speaks_of_your_own_lot() {
        let farm = sample_home();
        let station = body(Step::PlaceAtHome, None, Some(&farm), Play::Click, false);
        assert!(station.contains("Your farm fits 12 stations, though its deed allows 25"), "{station}");
        let tax = body(Step::ReadTax, None, Some(&farm), Play::Click, false);
        assert!(tax.contains("assessed at 500 Strands") && tax.contains("10 a week"), "{tax}");
        assert!(tax.contains("Four weeks unpaid (40 Strands)") && !tax.contains("owes"), "{tax}");
        let owing = Home { owed: 3, ..farm.clone() };
        assert!(body(Step::ReadTax, None, Some(&owing), Play::Click, false).contains("It owes 3 now."));
        let untaxed = Home { weekly_tax: None, ..farm.clone() };
        assert!(body(Step::ReadTax, None, Some(&untaxed), Play::Click, false).contains("owes none"));
        // A yard that fits all its deed allows says nothing of the deed
        let yard = Home { zone: capitol_core::protocol::rules::Zone::Urban, fits: 4, capacity: 4, ..farm.clone() };
        let yard = body(Step::PlaceAtHome, None, Some(&yard), Play::Click, false);
        assert!(yard.contains("Your yard fits 4 stations:") && !yard.contains("allows"), "{yard}");
        // No home yet: the plain words; other steps keep theirs
        assert_eq!(body(Step::ReadTax, None, None, Play::Click, false), words(Step::ReadTax).1);
        assert_eq!(body(Step::SetAutomation, None, Some(&farm), Play::Click, false), words(Step::SetAutomation).1);
        // The settle_in samples stand on a farm, the others on none
        let mut hints = Hints::default();
        hints.with_sample("read_tax");
        assert_eq!(hints.home, Some(sample_home()));
        hints.with_sample("hall");
        assert_eq!(hints.home, None);
    }

    /// The tax card's "Noted" is the step, told once (#449)
    #[test]
    fn the_tax_card_is_read_when_noted() {
        let mut hints = Hints::default();
        hints.with_sample("read_tax");
        assert_eq!(hints.noted(), None, "nothing clicked");
        hints.noted = true;
        assert_eq!(hints.noted(), Some(Step::ReadTax));
        hints.noted = true;
        assert_eq!(hints.noted(), None, "told once");
        // It leads nowhere; the two before it lead home
        hints.card = next_card(None, hints.tour.unwrap(), Instant::now());
        assert_eq!(hints.target(), None);
        hints.with_sample("place_at_home");
        hints.card = next_card(None, hints.tour.unwrap(), Instant::now());
        assert_eq!(hints.target(), Some(Target::Home));
    }

    /// Finished, nothing of the road in shows: no card, chip or line
    #[test]
    fn a_finished_road_leaves_nothing_behind() {
        let mut hints = Hints::default();
        hints.with_sample("settle_in-done");
        assert!(hints.tour.unwrap().finished());
        assert_eq!(hints.hidden(), None, "no chip, no line in the overlay");
        assert_eq!(next_card(None, hints.tour.unwrap(), Instant::now()), None);
        assert!(CLOSING_FOR > CHAPTER_FOR, "the last card stays longest");
    }

    /// Over the shoulder the walk step says WASD walks (T1.6), or with a
    /// pad the sticks do (T2.2); the rest, and point and walk, keep their
    /// words
    #[test]
    fn the_walk_step_speaks_of_the_mode() {
        assert!(body(Step::Walk, None, None, Play::Keys, false).contains("Walk with WASD"));
        assert!(words_in(Step::Walk, Play::Keys).2.contains("[W]"));
        assert!(body(Step::Walk, None, None, Play::Pad, false).contains("Walk with the left stick"));
        assert!(words_in(Step::Walk, Play::Pad).2.contains("{LS}"));
        assert_eq!(body(Step::Walk, None, None, Play::Click, false), words(Step::Walk).1);
        for play in [Play::Keys, Play::Pad] {
            assert_eq!(words_in(Step::Walk, play).0, words(Step::Walk).0);
            for step in Step::ALL.into_iter().filter(|s| *s != Step::Walk) {
                assert_eq!(words_in(step, play), words(step), "{step:?}");
            }
        }
        let mut hints = Hints::default();
        hints.set_pad(true);
        assert_eq!(hints.play(), Play::Click, "the pad does nothing in point and walk");
        hints.set_direct(true);
        assert_eq!(hints.play(), Play::Pad);
    }

    #[test]
    fn every_step_has_words_and_every_chapter_a_name() {
        for step in Step::ALL {
            let (title, body, keys) = words(step);
            assert!(!title.is_empty() && !body.is_empty() && !keys.is_empty(), "{step:?}");
        }
        let names: std::collections::HashSet<_> = Chapter::ALL.into_iter().map(chapter_name).collect();
        assert_eq!(names.len(), Chapter::ALL.len());
        for chapter in Chapter::ALL {
            assert!(!chapter_close(chapter).is_empty(), "{chapter:?}");
        }
    }
}
