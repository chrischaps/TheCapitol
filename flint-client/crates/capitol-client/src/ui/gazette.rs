//! The Capitol Gazette (E3.9): the latest edition as a page of newsprint.
//!
//! G, the header's Gazette chip, or a click on the notice board in the
//! Exchange hall's forecourt opens it. It's the one light page in the game:
//! warm paper and dark ink (`theme::PAPER`, `theme::INK`), the masthead and
//! headline in the inscriptional capitals, then three columns: the Exchange
//! with its small price table, the finest sale and the land office, and the
//! day's busiest hands. Narrower than a broadsheet, the columns stack.
//!
//! Its numbers are the edition's own, as the server composed it
//! (`engine::gazette`). This only lays them out. The day you last opened is
//! kept in `save/gazette.toml`, by server and player, so the header's hint
//! for a new edition doesn't come back each time you sign in.

use std::path::PathBuf;

use capitol_core::gazette::Reader;
use capitol_core::protocol::rest::{Gazette, GazettePrice};
use capitol_core::protocol::rules;
use egui::{Align, Align2, Color32, Id, Layout, Margin, RichText, Sense, Stroke};
use flint_runtime::PersistentStore;
use uuid::Uuid;

use super::theme::{self, size};

/// The open and close fade, in seconds, and how far it rises as it comes.
const FADE: f32 = 0.25;
const RISE: f32 = 14.0;
/// The page's widest, and the narrowest that still takes three columns.
const PAGE_WIDTH: f32 = 780.0;
const THREE_COLUMNS: f32 = 640.0;
/// Room kept at the page's right for the scroll bar.
const SCROLL_GUTTER: f32 = 12.0;
/// The masthead's and the headline's sizes.
const MASTHEAD: f32 = 30.0;
const HEADLINE: f32 = 25.0;

/// What the page needs to show.
pub struct View<'a> {
    pub reader: &'a Reader,
    /// Signed in: the byline can be changed. Offline it's a sample.
    pub online: bool,
}

/// What the reader did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Turn back (-1) or on (+1) a day
    Turn(i64),
    /// Print your name, or keep it out
    Byline(bool),
}

/// The panel and the file that keeps what you've read.
pub struct GazettePanel {
    pub open: bool,
    was_open: bool,
    /// Open when `reading` last looked
    reading: bool,
    file: ReadFile,
    account: Option<(String, Uuid)>,
    /// Where the page was drawn, for panels that keep clear of it.
    pub rect: Option<egui::Rect>,
}

impl Default for GazettePanel {
    fn default() -> Self {
        GazettePanel::load(ReadFile::PATH)
    }
}

impl GazettePanel {
    pub fn load(path: impl Into<PathBuf>) -> Self {
        Self { open: false, was_open: false, reading: false, file: ReadFile::load(path), account: None, rect: None }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Follow whoever is signed in on `server`: tell the reader which day
    /// they last read, once the server has said who they are.
    pub fn follow(&mut self, server: &str, me: Option<Uuid>, reader: &mut Reader) {
        let account = me.map(|me| (server.to_owned(), me));
        if account != self.account {
            if let Some((server, me)) = &account {
                reader.set_read(self.file.read(server, *me));
            }
            self.account = account;
        }
    }

    /// Each frame: opened, turn to the latest; while open, count it read
    /// (it may arrive after the page opens), and keep it.
    pub fn reading(&mut self, reader: &mut Reader) {
        let fresh = self.open && !self.reading;
        self.reading = self.open;
        if !self.open {
            return;
        }
        let read = if fresh { reader.open() } else { reader.mark_read() };
        if let (Some(day), Some((server, me))) = (read, &self.account) {
            self.file.set(server, *me, day);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, view: &View) -> Option<Action> {
        let shown = ctx.animate_bool_with_time(Id::new("capitol_gazette_fade"), self.open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            self.was_open = false;
            return None;
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        let screen = ctx.available_rect();
        if self.open && !self.was_open {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, Id::new("capitol_gazette")));
        }
        self.was_open = self.open;
        let width = PAGE_WIDTH.min(screen.width() - 32.0).max(280.0);
        let height = (screen.height() - 40.0).max(200.0);
        let mut action = None;
        let area = egui::Area::new(Id::new("capitol_gazette"))
            .anchor(Align2::CENTER_TOP, [0.0, screen.top() + 14.0 - RISE * (1.0 - ease)])
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                paper().show(ui, |ui| {
                    ui.set_width(width - 40.0);
                    ui.visuals_mut().override_text_color = Some(theme::INK);
                    action = page(ui, view, &mut self.open, height - 40.0);
                });
            });
        self.rect = self.open.then_some(area.response.rect);
        action
    }
}

/// The sheet: paper, a thin deckle edge, a little shadow.
fn paper() -> egui::Frame {
    egui::Frame::none()
        .fill(theme::PAPER)
        .stroke(Stroke::new(1.0, theme::PAPER_EDGE))
        .rounding(2.0)
        .inner_margin(Margin::symmetric(20.0, 16.0))
        .shadow(egui::epaint::Shadow {
            offset: [0.0, 6.0].into(),
            blur: 24.0,
            spread: 0.0,
            color: Color32::from_black_alpha(140),
        })
}

/// The page: the arrows and the fold stay put, and everything under them
/// scrolls, so a short window still reaches the columns.
fn page(ui: &mut egui::Ui, view: &View, open: &mut bool, height: f32) -> Option<Action> {
    let mut action = None;
    let reader = view.reader;
    turns(ui, reader, open, &mut action);
    let room = (height - 36.0).max(120.0);
    egui::ScrollArea::vertical()
        .max_height(room)
        .min_scrolled_height(room)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            // A gutter for the scroll bar, so it never sits on the text
            ui.set_max_width(ui.available_width() - SCROLL_GUTTER);
            masthead(ui, reader);
            let Some(g) = reader.showing() else {
                ui.add_space(24.0);
                let line = if reader.showing_missing() {
                    "No edition was printed that day."
                } else if reader.showing_day().is_some() {
                    "The presses are rolling…"
                } else {
                    "The first edition goes to press at midnight. Come back then."
                };
                ui.vertical_centered(|ui| ui.label(ink(line, size::LARGE).italics()));
                ui.add_space(24.0);
                return;
            };
            front_page(ui, g);
            ui.add_space(8.0);
            rule(ui, 1.0);
            ui.add_space(6.0);
            if ui.available_width() >= THREE_COLUMNS {
                // Ragged right, as `columns` would justify every line
                ui.columns(3, |c| {
                    c[0].vertical(|ui| exchange(ui, g));
                    c[1].vertical(|ui| sale_and_land(ui, g));
                    c[2].vertical(|ui| hands(ui, g));
                });
            } else {
                exchange(ui, g);
                ui.add_space(10.0);
                sale_and_land(ui, g);
                ui.add_space(10.0);
                hands(ui, g);
            }
            ui.add_space(10.0);
            rule(ui, 1.0);
            byline(ui, view, &mut action);
        });
    action
}

/// The arrows to the days either side, and the fold.
fn turns(ui: &mut egui::Ui, reader: &Reader, open: &mut bool, action: &mut Option<Action>) {
    ui.horizontal(|ui| {
        let (back, on) = reader.can_turn();
        if turn_button(ui, "‹ Earlier", back).clicked() {
            *action = Some(Action::Turn(-1));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(egui::Label::new(ink("×", size::LARGE + 3.0)).sense(Sense::click()))
                .on_hover_text("Fold it away (G)")
                .clicked()
            {
                *open = false;
            }
            ui.add_space(8.0);
            if turn_button(ui, "Later ›", on).clicked() {
                *action = Some(Action::Turn(1));
            }
        });
    });
}

/// THE CAPITOL GAZETTE, with the edition's number and day.
fn masthead(ui: &mut egui::Ui, reader: &Reader) {
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new("The Capitol Gazette")
                .font(theme::display(MASTHEAD))
                .color(theme::INK)
                .extra_letter_spacing(MASTHEAD * 0.12),
        );
    });
    rule(ui, 2.0);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let day = reader.showing_day();
        let left = day.map_or_else(String::new, |d| format!("No. {}", rules::gazette_number(d)));
        ui.label(ink(&left, size::SMALL));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(ink("One Strand", size::SMALL));
            ui.with_layout(Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                let middle = day.map_or_else(String::new, |d| {
                    format!("{} · the day's trade, deeds and work", rules::weekday(d))
                });
                ui.label(dim(&middle, size::SMALL));
            });
        });
    });
    rule(ui, 1.0);
    ui.add_space(8.0);
}

fn turn_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let colour = if enabled { theme::INK } else { theme::PAPER_EDGE };
    let label = egui::Label::new(RichText::new(text).font(theme::strong(size::SMALL)).color(colour));
    ui.add_enabled(enabled, label.sense(Sense::click()))
}

/// The headline, the line under it, the story and the sky.
fn front_page(ui: &mut egui::Ui, g: &Gazette) {
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(&g.headline)
                .font(theme::display(HEADLINE))
                .color(theme::INK)
                .extra_letter_spacing(1.0),
        );
        ui.add_space(2.0);
        ui.label(dim(&g.dek, size::LARGE).italics());
    });
    ui.add_space(8.0);
    ui.label(ink(&g.lead, size::LARGE));
    ui.add_space(8.0);
    egui::Frame::none()
        .stroke(Stroke::new(1.0, theme::INK_RULE))
        .inner_margin(Margin::symmetric(10.0, 5.0))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(kicker("The Sky"));
                ui.label(ink(&g.weather, size::SMALL));
            });
        });
}

/// From the Exchange: the day's trading, the movers and every price.
fn exchange(ui: &mut egui::Ui, g: &Gazette) {
    section(ui, "From the Exchange");
    ui.label(ink(&g.markets, size::BODY));
    if !g.prices.is_empty() {
        ui.add_space(6.0);
        price_table(ui, &g.prices);
    }
}

fn price_table(ui: &mut egui::Ui, prices: &[GazettePrice]) {
    egui::Grid::new("gazette_prices")
        .num_columns(4)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            for head in ["", "Range", "Mean", "Chg"] {
                ui.label(dim(head, size::LABEL).strong());
            }
            ui.end_row();
            for p in prices {
                ui.label(ink(&p.item_name, size::SMALL));
                ui.label(dim(&format!("{}–{}", p.low, p.high), size::SMALL));
                ui.label(ink(&p.mean.to_string(), size::SMALL).strong());
                let (text, colour) = change(p);
                ui.label(RichText::new(text).font(theme::body(size::SMALL)).color(colour));
                ui.end_row();
            }
        });
}

/// A price's change from the day before, as a paper prints it: ▲ 36%, ▼ 8%,
/// "–" when it held, "new" when it didn't trade the day before.
fn change(p: &GazettePrice) -> (String, Color32) {
    match p.previous.filter(|&v| v > 0) {
        None => ("new".into(), theme::INK_DIM),
        Some(prev) => {
            let pct = (p.mean - prev) * 100 / prev;
            match pct.signum() {
                1 => (format!("▲ {pct}%"), theme::INK_UP),
                -1 => (format!("▼ {}%", -pct), theme::INK_DOWN),
                _ => ("–".into(), theme::INK_DIM),
            }
        }
    }
}

/// The finest sale, then the land office's deeds by zone.
fn sale_and_land(ui: &mut egui::Ui, g: &Gazette) {
    section(ui, "The Finest Sale");
    if let Some(sale) = &g.best_sale {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&sale.grade)
                    .font(theme::display(size::GLYPH))
                    .color(theme::INK),
            );
            ui.vertical(|ui| {
                ui.label(ink(&sale.item_name, size::BODY).strong());
                ui.label(dim(&format!("quality {} · {} Strands", sale.quality, sale.price), size::SMALL));
            });
        });
    }
    ui.label(ink(&g.best_sale_story, size::BODY));
    ui.add_space(10.0);
    section(ui, "The Land Office");
    ui.label(ink(&g.deeds_story, size::BODY));
    if !g.deeds.is_empty() {
        ui.add_space(4.0);
        egui::Grid::new("gazette_deeds").num_columns(3).spacing([8.0, 2.0]).show(ui, |ui| {
            for head in ["", "New", "Lost"] {
                ui.label(dim(head, size::LABEL).strong());
            }
            ui.end_row();
            for d in &g.deeds {
                ui.label(ink(&d.zone_name, size::SMALL));
                ui.label(ink(&d.claimed.to_string(), size::SMALL));
                let lost = RichText::new(d.foreclosed.to_string()).font(theme::body(size::SMALL));
                ui.label(if d.foreclosed > 0 { lost.color(theme::INK_DOWN) } else { lost.color(theme::INK_DIM) });
                ui.end_row();
            }
        });
    }
}

/// The day's busiest hands.
fn hands(ui: &mut egui::Ui, g: &Gazette) {
    section(ui, "Busy Hands");
    ui.label(ink(&g.producers_story, size::BODY));
    if !g.producers.is_empty() {
        ui.add_space(4.0);
        for (k, p) in g.producers.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.label(dim(&format!("{}.", k + 1), size::SMALL));
                let name = p.name.as_deref().unwrap_or("Name withheld");
                ui.label(ink(name, size::SMALL).strong());
                ui.label(dim(&format!("{} units, chiefly {}", p.units, p.item_name), size::SMALL));
            });
        }
    }
}

/// Whether the Gazette prints your name, and the switch.
fn byline(ui: &mut egui::Ui, view: &View, action: &mut Option<Action>) {
    if !view.online {
        return;
    }
    let Some(named) = view.reader.named() else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        let (now, switch, to) = if named {
            ("The Gazette may print your name.", "Keep it out", false)
        } else {
            ("The Gazette keeps your name out of its pages.", "Let it print it", true)
        };
        ui.label(dim(now, size::SMALL));
        let link = egui::Label::new(
            RichText::new(switch).font(theme::strong(size::SMALL)).color(theme::INK).underline(),
        );
        if ui
            .add(link.sense(Sense::click()))
            .on_hover_text("From the next edition. Printed ones stay as they are.")
            .clicked()
        {
            *action = Some(Action::Byline(to));
        }
    });
}

/// A section's head: small capitals over a hairline.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(kicker(title));
    rule(ui, 1.0);
    ui.add_space(2.0);
}

fn kicker(text: &str) -> RichText {
    RichText::new(text)
        .font(theme::display(size::BODY))
        .color(theme::INK)
        .extra_letter_spacing(1.5)
}

/// A rule across the page or column.
fn rule(ui: &mut egui::Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), width + 3.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(width, theme::INK_RULE));
}

fn ink(text: &str, size: f32) -> RichText {
    RichText::new(text).font(theme::body(size)).color(theme::INK)
}

fn dim(text: &str, size: f32) -> RichText {
    RichText::new(text).font(theme::body(size)).color(theme::INK_DIM)
}

/// `save/gazette.toml`: the day each account last opened, by server and
/// player id. Its own file, as `save/hints.toml` is.
struct ReadFile {
    path: PathBuf,
    store: PersistentStore,
}

impl ReadFile {
    const PATH: &'static str = "save/gazette.toml";

    fn load(path: impl Into<PathBuf>) -> ReadFile {
        let path = path.into();
        let mut store = PersistentStore::new();
        if path.is_file() {
            if let Err(e) = store.load_from_file(&path) {
                tracing::warn!("gazette: ignoring {}: {e}", path.display());
            }
        }
        ReadFile { path, store }
    }

    fn key(server: &str, me: Uuid) -> String {
        format!("{server} {me}")
    }

    fn read(&self, server: &str, me: Uuid) -> Option<i64> {
        self.store.get(&Self::key(server, me)).and_then(|v| v.as_integer())
    }

    fn set(&mut self, server: &str, me: Uuid, day: i64) {
        self.store.set(&Self::key(server, me), toml::Value::Integer(day));
        if let Some(dir) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!("gazette: can't create {}: {e}", dir.display());
                return;
            }
        }
        match self.store.save_to_file(&self.path) {
            Ok(()) => self.store.mark_clean(),
            Err(e) => tracing::warn!("gazette: can't write {}: {e}", self.path.display()),
        }
    }
}

/// `--gazette-sample`: a reader holding the protocol's fixture edition, so
/// the page can be seen offline.
pub fn sample() -> Reader {
    use capitol_core::gazette::Ask;
    let response = capitol_core::serde_json::from_str(include_str!("../../../../../protocol/tests/fixtures/rest/gazette.json"))
        .expect("the gazette fixture parses");
    let mut reader = Reader::new(None);
    reader.sent(Ask::Latest);
    reader.answered(Ask::Latest, response);
    reader
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("capitol-gazette-{}-{name}.toml", std::process::id()))
    }

    #[test]
    fn the_sample_has_an_edition() {
        let reader = sample();
        assert!(reader.showing().is_some());
        assert!(reader.unread());
    }

    #[test]
    fn opening_it_counts_the_edition_read_and_keeps_it() {
        let path = temp("read");
        let me = Uuid::from_u128(7);
        let mut reader = sample();
        let mut panel = GazettePanel::load(&path);
        panel.follow("http://s", Some(me), &mut reader);
        panel.open = true;
        panel.reading(&mut reader);
        assert!(!reader.unread());
        // Read again from the file, by a new session
        let mut again = sample();
        let mut panel = GazettePanel::load(&path);
        panel.follow("http://s", Some(me), &mut again);
        assert!(!again.unread());
        // Someone else hasn't read it
        let mut other = sample();
        panel.follow("http://s", Some(Uuid::from_u128(8)), &mut other);
        assert!(other.unread());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn changes_read_as_a_paper_prints_them() {
        let p = |mean, previous| GazettePrice { mean, previous, ..Default::default() };
        assert_eq!(change(&p(41, Some(30))).0, "▲ 36%");
        assert_eq!(change(&p(27, Some(30))).0, "▼ 10%");
        assert_eq!(change(&p(30, Some(30))).0, "–");
        assert_eq!(change(&p(30, None)).0, "new");
    }

    /// The page lays out at a broadsheet's width and a phone's, without panic.
    #[test]
    fn the_page_draws_wide_and_narrow() {
        let reader = sample();
        for width in [1600.0, 420.0] {
            let ctx = theme::test_context();
            let mut panel = GazettePanel::load(temp("draw"));
            panel.open = true;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 900.0))),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    panel.show(ctx, &View { reader: &reader, online: true });
                });
            }
            let rect = panel.rect.expect("drawn");
            assert!(rect.width() <= width, "{width}: {rect:?}");
        }
    }
}
