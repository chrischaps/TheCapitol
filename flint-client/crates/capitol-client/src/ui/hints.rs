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
//! A step can lead somewhere (`onboarding::target`). The extension finds
//! the place and hands back a [`Lead`]: off screen, the card points the way
//! with the hall chip's arrow (`hud::bearing`); on screen, `input.rhai`
//! rings it in gold on the ground (`draw_lead`).

use std::time::{Duration, Instant};

use capitol_core::onboarding::{place_in_chapter, target, Did, Target, Tour};
use capitol_core::protocol::tutorial::{Chapter, Step, Steps, TutorialProgress};
use egui::{Align2, Id, RichText};

use super::shortcuts::keys_row;
use super::theme::{self, size};

/// How long a done step shows its tick before the next card.
const TICK_FOR: Duration = Duration::from_millis(900);
/// How long a card takes to fade in.
const FADE_IN: f32 = 0.35;
/// How long a chapter's close stays: a beat longer than a step's tick.
const CHAPTER_FOR: Duration = Duration::from_millis(5500);
/// How long the closing card stays, and how long it takes to go.
const CLOSING_FOR: Duration = Duration::from_secs(8);
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
            "You can gather, make things by hand and at a bench, and open with a tool what hands can't. The road north leads in."
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
        Chapter::DeedsOffice => "The land is yours, and on the city's books. Now make something of it.",
        Chapter::SettleIn => CLOSING.1,
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
        // Arrival
        Step::Walk => (
            "Find your feet",
            "Click the ground and you'll walk there, round the water by the bridges and fords.",
            "[Click] walk  ·  [W] [A] [S] [D] pan",
        ),
        Step::GatherGrass => (
            "Gather fiber",
            "The tufts of tall grass are fiber. Click one to gather it, and you'll walk over if it's far.",
            "[Click] a tuft",
        ),
        Step::OpenPack => (
            "Your pack",
            "What you gather lands in your inventory, bottom right. Drag to sort it.",
            "[I] opens and closes it",
        ),
        Step::TwistRope => (
            "Twist some rope",
            "Open crafting and press Fill to bring fiber from your pack. Two fiber twist into a rope.",
            "[C] crafting  ·  [Enter] craft",
        ),
        Step::MakeWorkbench => (
            "A workbench kit",
            "Hands only go so far. Ten fiber make a workbench kit, in the same crafting panel.",
            "[C] crafting",
        ),
        Step::PlaceWorkbench => (
            "Set it down",
            "Right-click the kit in your pack and put it on open ground. Off a lot, a workbench is the one station allowed.",
            "Right-click a kit  ·  [Click] to place",
        ),
        Step::GatherBranches => (
            "Fallen branches",
            "Branches lie under the trees. Gather them as you did the grass.",
            "[Click] the branches",
        ),
        Step::GatherStone => (
            "Loose stone",
            "Stone lies on the scree round the outcrops. Gather some.",
            "[Click] a stone pile",
        ),
        Step::MakeHandle => (
            "A handle",
            "Open the workbench: two branches and a rope make a tool handle.",
            "[O] the nearest station",
        ),
        Step::KnapStone => (
            "An edge",
            "At the same bench, knap two stones to an edge.",
            "[O] the nearest station",
        ),
        Step::MakeAxe => (
            "A stone axe",
            "Two edges, a handle and a rope make an axe. Better parts make a better axe: quality carries through.",
            "[O] the nearest station",
        ),
        Step::FellTree => (
            "Fell a tree",
            "With the axe in your pack, a tree gives logs. Hands alone won't do it.",
            "[Click] a tree",
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
            "The hall under the tower takes fiber in and pays out Strands, the city's coin, less a small fee. \
             The panel sets all three in a line. Deposit a little.",
            "[X] the exchange",
        ),
        Step::SellOrder => (
            "Offer something",
            "The order book is a list of prices, waiting. Yours waits in it until someone meets it. \
             Put up something you made on the road: a rope, a handle, an edge.",
            "[M] the market  ·  Sell",
        ),
        Step::BuyOrder => (
            "Bid for something",
            "Now bid for something you haven't got. Your Strands wait in escrow with the price, \
             and come back if you cancel.",
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
            "Beside the hall, a desk under an awning sets the zones side by side. Look in.",
            "[Click] the desk",
        ),
        Step::ClaimHome => (
            "A home",
            "Choose a lot in any home zone and claim it. The road has earned you its deed.",
            "[Alt] + [Click] a lot",
        ),
        // Settle in
        Step::PlaceAtHome => (
            "A station at home",
            "Place a station on your own lot. The room there is yours to fill.",
            "Right-click a kit",
        ),
        Step::SetAutomation => (
            "Work that runs",
            "Set a station's job, and it works while you're away.",
            "[O] a station  ·  Automation",
        ),
        Step::ReadTax => (
            "What the city asks",
            "Every deed owes tax each week. Read what yours asks, on its panel.",
            "[Alt] + [Click] your lot",
        ),
    }
}

/// The market box's card once its box is known empty (#446, plan D10):
/// nothing's filled, so it says so, and the road goes on without it.
const BOX_EMPTY: &str = "Nothing's filled yet, so your box is empty: a price can wait a day for its match. \
     Your orders stay on the book, and the road goes on without them.";

/// How long the market box's card says it's empty before it's waived.
const WAIVE_AFTER: Duration = Duration::from_secs(7);

/// A step's words on its card: [`words`], but for the market box once it's
/// known empty (`box_goods`, `Some(false)`).
fn body(step: Step, box_goods: Option<bool>) -> &'static str {
    match (step, box_goods) {
        (Step::CollectBox, Some(false)) => BOX_EMPTY,
        _ => words(step).1,
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
}

/// Which card is drawn, for remembering each one's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Step(Step),
    Chapter(Chapter),
    Closing,
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

/// The card's line on where to go.
fn lead_words(lead: Lead) -> String {
    if lead.metres < HERE {
        return "Here, ringed in gold".to_owned();
    }
    let far = if lead.metres >= 1000.0 {
        format!("{:.1} km", lead.metres / 1000.0)
    } else {
        let to = if lead.metres < 100.0 { 5.0 } else { 10.0 };
        format!("{} m", ((lead.metres / to).round() * to) as i32)
    };
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
        self.card = next_card(self.card, tour, now);
        let Some(card) = self.card else {
            return false;
        };
        let (face, since) = match card {
            Card::Step(step, at) | Card::Done(step, at) => (Face::Step(step), at),
            Card::ChapterDone(chapter, at) => (Face::Chapter(chapter), at),
            Card::Closing(at) => (Face::Closing, at),
        };
        let age = now.saturating_duration_since(since).as_secs_f32();
        let fading = |stays: Duration| {
            let left = stays.as_secs_f32() - age;
            (age / FADE_IN)
                .min(1.0)
                .min((left / FADE_OUT).clamp(0.0, 1.0))
        };
        let alpha = match card {
            Card::Step(..) => (age / FADE_IN).min(1.0),
            Card::Done(..) => 1.0,
            Card::ChapterDone(..) => fading(CHAPTER_FOR),
            Card::Closing(_) => fading(CLOSING_FOR),
        };
        let eased = 1.0 - (1.0 - alpha).powi(3);
        let mut hide = false;
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
                            let body = body(step, self.box_goods);
                            hide = step_card(ui, step, body, done, spot.compact, lead);
                        }
                        Face::Chapter(chapter) => chapter_card(ui, chapter, spot.compact),
                        Face::Closing => closing_card(ui, spot.compact),
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
                Face::Step(s) => (words(s).0.to_owned(), body(s, self.box_goods)),
                Face::Chapter(c) => (chapter_done_title(c), chapter_close(c)),
                Face::Closing => (CLOSING.0.to_owned(), CLOSING.1),
            };
            let layer = shown.response.layer_id;
            egui::show_tooltip_at_pointer(ctx, layer, Id::new("capitol_hint_words"), |ui| {
                ui.set_max_width(WIDTH);
                ui.label(theme::title(&title, size::BODY));
                ui.label(RichText::new(body).color(theme::TEXT).size(size::BODY));
            });
        }
        self.rect = Some(shown.response.rect);
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

/// A step's card: its chapter and place in it, then the step, its `body`
/// (its words, as [`body`] picks them), and where it leads if anywhere;
/// `compact` leaves out the words. True if "Hide for now" was clicked.
fn step_card(ui: &mut egui::Ui, step: Step, body: &str, done: bool, compact: bool, lead: Option<Lead>) -> bool {
    let (title, _, keys) = words(step);
    let (at, of) = place_in_chapter(step);
    let mut hide = false;
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
        keys_row(ui, keys);
    });
    hide
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

/// The closing card's title and words.
const CLOSING: (&str, &str) = (
    "Well begun",
    "You've walked the road in and made a home. The rest is yours: \
     make, trade, and make your name.",
);

/// The last card: the road is walked. `compact` leaves out its words.
fn closing_card(ui: &mut egui::Ui, compact: bool) {
    let (title, body) = CLOSING;
    let last = Chapter::SettleIn.steps().count();
    ui.label(theme::label(chapter_name(Chapter::SettleIn)));
    ui.add_space(2.0);
    theme::step_pips(ui, last, last);
    ui.add_space(4.0);
    ui.label(theme::title(title, size::HEADING + 2.0));
    if !compact {
        ui.add_space(2.0);
        ui.label(RichText::new(body).color(theme::TEXT).size(size::BODY));
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        theme::keycap(ui, "?");
        ui.label(theme::dim("shows every control, any time"));
    });
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
    let at = |done| TutorialProgress { done, dismissed: false };
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
        "hidden" | "help" => TutorialProgress { done: half, dismissed: true },
        _ => TutorialProgress::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tour(done: &[Step]) -> Tour {
        Tour::from(TutorialProgress { done: done.iter().copied().collect(), dismissed: false })
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
        hints.follow(Some(TutorialProgress { done: Steps::every(), dismissed: true }));
        assert_eq!(hints.hidden(), None);
    }

    #[test]
    fn what_the_screen_showed_is_told_once() {
        let mut hints = Hints::default();
        assert_eq!(hints.saw(Did::UsedInventory), None, "nobody to tell yet");
        hints.follow(Some(TutorialProgress::default()));
        assert_eq!(hints.saw(Did::UsedInventory), Some(Step::OpenPack));
        assert_eq!(hints.saw(Did::OpenedCrafting), None, "already on its way");
        hints.follow(Some(TutorialProgress { done: Steps::from_iter([Step::OpenPack]), dismissed: false }));
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
        assert_eq!(body(Step::CollectBox, hints.box_goods), words(Step::CollectBox).1);
        assert_eq!(hints.waive(later), None, "something waits: collect it");

        // Known empty from t1: it says so, and moves on once it's been read
        let t1 = t0 + Duration::from_secs(2);
        hints.market_box(Some(false), t1);
        hints.market_box(Some(false), t1 + Duration::from_secs(1));
        assert_eq!(body(Step::CollectBox, hints.box_goods), BOX_EMPTY);
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
        assert_eq!(body(Step::BuyOrder, Some(false)), words(Step::BuyOrder).1);
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
