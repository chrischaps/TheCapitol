//! First-session hints (P8.3): one card at a time, top right under the
//! header (clear of the zone banner, the panels and the toasts), for
//! walking, gathering, the inventory and a first rope. Which
//! card, and what counts as done, is `capitol_core::onboarding::Tour`; this
//! keeps each account's tour in `save/hints.toml` and draws the card.
//!
//! A finished step shows its tick for a moment before the next card fades
//! in. The last one gives way to a closing card that points at `?`, and
//! then nothing, for good. "Skip the tips" ends it at once. The tour is per
//! account and per server, so a second account on this machine sees the
//! hints too, and an old account sees none (`Tour::judge`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use capitol_core::onboarding::{Did, Step, Tour};
use capitol_core::state::Container;
use egui::{Align2, Id, RichText};
use flint_runtime::PersistentStore;
use uuid::Uuid;

use super::shortcuts::keys_row;
use super::theme::{self, size};

/// How long a done step shows its tick before the next card.
const TICK_FOR: Duration = Duration::from_millis(900);
/// How long a card takes to fade in.
const FADE_IN: f32 = 0.35;
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

/// What a step's card says: a title, the words, and a line of keys.
fn words(step: Step) -> (&'static str, &'static str, &'static str) {
    match step {
        Step::Walk => (
            "Find your feet",
            "Click the ground and you'll walk there. Canals stop you; the bridges cross them.",
            "[Click] walk  ·  [W] [A] [S] [D] pan",
        ),
        Step::Gather => (
            "Gather fiber",
            "The tufts of tall grass are fiber. Click one to gather it, and you'll walk over if it's far.",
            "[Click] a tuft",
        ),
        Step::Inventory => (
            "Your pack",
            "What you gather lands in your inventory, bottom right. Drag to sort it.",
            "[I] opens and closes it",
        ),
        Step::Craft => (
            "Twist some rope",
            "Open crafting and press Fill to bring fiber from your pack. Two fiber twist into a rope.",
            "[C] crafting  ·  [Enter] craft",
        ),
    }
}

/// What the card does this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Card {
    /// The step to do, and when its card came up.
    Step(Step, Instant),
    /// A step just done, with its tick, and when it was.
    Done(Step, Instant),
    /// Every step done; when the last one was.
    Closing(Instant),
}

pub struct Hints {
    file: HintsFile,
    /// Whose tour this is: the server and the player's id.
    account: Option<(String, Uuid)>,
    tour: Tour,
    /// Whether this account's inventory has been looked at (`Tour::judge`).
    judged: bool,
    card: Option<Card>,
    /// Where the card was drawn, for panels that need to keep clear.
    pub rect: Option<egui::Rect>,
    /// Each card's size, full and compact, as last drawn: where it fits
    /// is judged from these (`place`).
    sizes: Vec<((Option<Step>, bool), egui::Vec2)>,
}

impl Hints {
    pub fn load(path: impl Into<PathBuf>) -> Hints {
        Hints {
            file: HintsFile::load(path),
            account: None,
            tour: Tour::default(),
            judged: false,
            card: None,
            rect: None,
            sizes: Vec::new(),
        }
    }

    /// Follow whoever is signed in on `server`: `me` once the server has
    /// said who, and their inventory once it's arrived. Nothing shows until
    /// both are known, so an old account never flashes a hint.
    pub fn follow(&mut self, server: &str, me: Option<Uuid>, inventory: Option<&Container>) {
        let account = me.map(|me| (server.to_owned(), me));
        if account != self.account {
            self.tour = account
                .as_ref()
                .map_or_else(Tour::default, |(s, me)| self.file.tour(s, *me));
            self.account = account;
            self.judged = false;
            self.card = None;
        }
        if self.account.is_none() || self.judged {
            return;
        }
        let Some(inventory) = inventory else {
            return;
        };
        self.judged = true;
        let raw = |t: &str| {
            capitol_core::catalog::item_type(t).is_some_and(|t| t.category == "raw_material")
        };
        if self
            .tour
            .judge(inventory.items().map(|i| i.item_type.as_str()), raw)
        {
            tracing::info!(
                "hints: judged from the inventory: {:#06b}",
                self.tour.bits()
            );
            self.save();
        }
    }

    /// The player did `did`.
    pub fn saw(&mut self, did: Did) {
        if self.account.is_some() && self.tour.saw(did) {
            tracing::info!("hints: {did:?}");
            self.save();
        }
    }

    fn save(&mut self) {
        if let Some((server, me)) = &self.account {
            self.file.set(server, *me, self.tour);
        }
    }

    /// Draw the card, if there is one. Call once a frame while in the world.
    /// It keeps clear of the open `panels` where it can (#99): see `place`.
    pub fn show(&mut self, ctx: &egui::Context, now: Instant, panels: &[egui::Rect]) {
        self.rect = None;
        if !self.judged {
            return;
        }
        self.card = next_card(self.card, self.tour, now);
        let Some(card) = self.card else {
            return;
        };
        let (step, since) = match card {
            Card::Step(step, at) => (Some(step), at),
            Card::Done(step, at) => (Some(step), at),
            Card::Closing(at) => (None, at),
        };
        let age = now.saturating_duration_since(since).as_secs_f32();
        let alpha = match card {
            Card::Step(..) => (age / FADE_IN).min(1.0),
            Card::Done(..) => 1.0,
            Card::Closing(_) => {
                let left = CLOSING_FOR.as_secs_f32() - age;
                (age / FADE_IN)
                    .min(1.0)
                    .min((left / FADE_OUT).clamp(0.0, 1.0))
            }
        };
        let eased = 1.0 - (1.0 - alpha).powi(3);
        let mut skip = false;
        // Under the header, as the panels are (anchors are screen-relative).
        let top = ctx.available_rect().top() + 12.0;
        let size = |compact| {
            self.sizes
                .iter()
                .find(|(k, _)| *k == (step, compact))
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
                    match step {
                        Some(step) => {
                            let done = matches!(card, Card::Done(..));
                            skip = step_card(ui, step, done, spot.compact);
                        }
                        None => closing_card(ui, spot.compact),
                    }
                });
            });
        let key = (step, spot.compact);
        let drawn = shown.response.rect.size();
        match self.sizes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => *v = drawn,
            None => self.sizes.push((key, drawn)),
        }
        // A compact card's words are a hover away. Anywhere on the card:
        // the area's own response isn't hovered while a child widget is.
        if spot.compact && shown.response.contains_pointer() {
            let (title, body) = step.map_or(CLOSING, |s| {
                let (title, body, _) = words(s);
                (title, body)
            });
            let layer = shown.response.layer_id;
            egui::show_tooltip_at_pointer(ctx, layer, Id::new("capitol_hint_words"), |ui| {
                ui.set_max_width(WIDTH);
                ui.label(theme::title(title, size::BODY));
                ui.label(RichText::new(body).color(theme::TEXT).size(size::BODY));
            });
        }
        self.rect = Some(shown.response.rect);
        if skip && self.tour.dismiss() {
            tracing::info!("hints: skipped");
            self.save();
            self.card = None;
        }
    }
}

/// Where the card goes this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Spot {
    /// Its inset from the screen's right edge.
    right: f32,
    /// Its words go to a hover, leaving the title, the keys and Skip.
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
        Some(Card::Closing(at)) if now.saturating_duration_since(at) < CLOSING_FOR => {
            Some(Card::Closing(at))
        }
        Some(Card::Closing(_)) => None,
        // After a tick, or with nothing up: the step at hand, or the
        // closing card if the tour was just finished here (not skipped,
        // and not finished before this session).
        last => match current {
            Some(step) => Some(Card::Step(step, now)),
            None if tour.completed() && matches!(last, Some(Card::Done(..))) => {
                Some(Card::Closing(now))
            }
            None => None,
        },
    }
}

/// A step's card; `compact` leaves out its words. True if "Skip the tips"
/// was clicked.
fn step_card(ui: &mut egui::Ui, step: Step, done: bool, compact: bool) -> bool {
    let (title, body, keys) = words(step);
    let mut skip = false;
    ui.horizontal(|ui| {
        theme::step_pips(ui, Step::ALL.len(), step.index() + usize::from(done));
        ui.add_space(4.0);
        ui.label(theme::dim(format!(
            "{} of {}",
            step.index() + 1,
            Step::ALL.len()
        )));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            skip = theme::link(ui, "Skip the tips", size::SMALL).clicked();
        });
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
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        keys_row(ui, keys);
    });
    skip
}

/// The closing card's title and words.
const CLOSING: (&str, &str) = (
    "Well begun",
    "You've walked, gathered and made rope. The rest is trade: \
     build stations, claim a plot, and make your fortune.",
);

/// The last card: you've the hang of it. `compact` leaves out its words.
fn closing_card(ui: &mut egui::Ui, compact: bool) {
    let (title, body) = CLOSING;
    theme::step_pips(ui, Step::ALL.len(), Step::ALL.len());
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

/// `save/hints.toml`: each account's tour, by server and player id, as
/// [`Tour::bits`]. Its own file, beside `save/session.toml`, because
/// extensions can't reach the scripts' store (issue #71).
struct HintsFile {
    path: PathBuf,
    store: PersistentStore,
}

impl HintsFile {
    const PATH: &'static str = "save/hints.toml";

    fn load(path: impl Into<PathBuf>) -> HintsFile {
        let path = path.into();
        let mut store = PersistentStore::new();
        if path.is_file() {
            if let Err(e) = store.load_from_file(&path) {
                tracing::warn!("hints: ignoring {}: {e}", path.display());
            }
        }
        HintsFile { path, store }
    }

    fn key(server: &str, me: Uuid) -> String {
        format!("{server} {me}")
    }

    fn tour(&self, server: &str, me: Uuid) -> Tour {
        self.store
            .get(&Self::key(server, me))
            .and_then(|v| v.as_integer())
            .and_then(|b| u8::try_from(b).ok())
            .map_or_else(Tour::default, Tour::from_bits)
    }

    fn set(&mut self, server: &str, me: Uuid, tour: Tour) {
        self.store.set(
            &Self::key(server, me),
            toml::Value::Integer(tour.bits().into()),
        );
        if let Some(dir) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!("hints: can't create {}: {e}", dir.display());
                return;
            }
        }
        match self.store.save_to_file(&self.path) {
            Ok(()) => self.store.mark_clean(),
            Err(e) => tracing::warn!("hints: can't write {}: {e}", self.path.display()),
        }
    }
}

impl Default for Hints {
    fn default() -> Self {
        Hints::load(HintsFile::PATH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("capitol-hints-{}", std::process::id()));
        dir.join(name)
    }

    #[test]
    fn a_tour_is_kept_per_account_and_server() {
        let path = temp("kept.toml");
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut file = HintsFile::load(&path);
        let mut tour = Tour::default();
        tour.saw(Did::Walked);
        file.set("http://localhost:3000", a, tour);

        let file = HintsFile::load(&path);
        assert_eq!(file.tour("http://localhost:3000", a), tour);
        assert_eq!(file.tour("http://localhost:3000", b), Tour::default());
        assert_eq!(file.tour("http://elsewhere:3000", a), Tour::default());
        let _ = std::fs::remove_file(path);
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
        let mut tour = Tour::default();
        let card = next_card(None, tour, t0);
        assert_eq!(card, Some(Card::Step(Step::Walk, t0)));
        tour.saw(Did::Walked);
        let t1 = t0 + Duration::from_secs(3);
        let card = next_card(card, tour, t1);
        assert_eq!(card, Some(Card::Done(Step::Walk, t1)));
        let card = next_card(card, tour, t1 + TICK_FOR / 2);
        assert_eq!(card, Some(Card::Done(Step::Walk, t1)));
        let t2 = t1 + TICK_FOR;
        assert_eq!(
            next_card(card, tour, t2),
            Some(Card::Step(Step::Gather, t2))
        );
    }

    #[test]
    fn the_closing_card_comes_once_and_only_after_the_last_step() {
        let t0 = Instant::now();
        let mut tour = Tour::from_bits(0b0111);
        let card = next_card(None, tour, t0);
        assert_eq!(card, Some(Card::Step(Step::Craft, t0)));
        tour.saw(Did::Crafted);
        let card = next_card(card, tour, t0);
        let t1 = t0 + TICK_FOR;
        let card = next_card(card, tour, t1);
        assert_eq!(card, Some(Card::Closing(t1)));
        assert_eq!(next_card(card, tour, t1 + CLOSING_FOR), None);
        // Finished in an earlier session: nothing at all.
        assert_eq!(next_card(None, tour, t1), None);
        // Skipped: nothing either.
        let mut skipped = Tour::default();
        skipped.dismiss();
        assert_eq!(next_card(None, skipped, t0), None);
    }
}
