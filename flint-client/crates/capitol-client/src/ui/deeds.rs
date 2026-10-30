//! The Deeds Office's panel (#448, `the-capitol-road-in.md` §4.5): where you
//! choose a home and claim it with the deed the road in earned.
//!
//! It opens from the desk in the hall's forecourt (`input.rhai`'s
//! `open_office`), in the middle of the screen, on two pages:
//!
//! - **The zones side by side:** the zone card's three columns (#444), every
//!   plan at one scale, each with a link to its free lots.
//! - **Free lots** in one zone, sorted by the walk to the hall or by room
//!   (the stations each fits, D9). Each has "Show me", which marks it on
//!   the ground and points the way, so you can walk out and look first, and
//!   while your deed is open, "Claim with your deed", asked twice.
//!
//! Under both, **your deed**: a slip of the Gazette's paper with the
//! registry's seal, saying what it is and where it stands. It's the
//! bureaucracy's voice (plan D1): a desk, not a clerk. The claim needs no
//! trip here (D8): once you've looked in, any free home lot's plot panel
//! offers the deed too. The rules and words are `capitol_core::deeds`.

use capitol_core::deeds::{self, Sort, HOME_ZONES};
use capitol_core::protocol::rest::{FreeLot, FreeLotsResponse, ZoneFacts};
use capitol_core::protocol::rules::Zone;
use capitol_core::protocol::tutorial::Grant;
use capitol_core::zone_card as words;
use egui::{Align2, Id, Pos2, Rect, RichText, Sense, Stroke};
use uuid::Uuid;

use super::pad_nav::FollowFocus;
use super::theme::{self, size, DIM, GOLD, TEXT};
use super::zone_card::{columns_width, Plans};

/// The open and close fade, in seconds, and how far it rises as it comes.
const FADE: f32 = 0.22;
const RISE: f32 = 12.0;
/// One free lot's row.
const ROW: f32 = 46.0;
/// Room kept for the header, the tabs, the slip and the foot, in points.
const CHROME: f32 = 236.0;

/// The office's two pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Compare,
    Lots,
}

/// A lot the office marked for you to walk out and look at.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub terrain_id: String,
    pub plot_id: Uuid,
    pub zone: Zone,
    /// Its middle, server units
    pub x: f64,
    pub y: f64,
}

/// What the reader did.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Close,
    /// List this zone's free lots: ask the server for them.
    List(Zone),
    ShowMe(Shown),
    StopShowing,
    /// Claim this plot with the deed, asked twice.
    Claim(Uuid),
}

/// What the panel draws from.
pub struct View<'a> {
    /// The zone card's facts, `None` until the server answers.
    pub facts: Option<&'a [ZoneFacts]>,
    /// The free lots last listed, of whichever zone.
    pub lots: Option<&'a FreeLotsResponse>,
    pub grant: Grant,
    /// The lot shown on the ground, if any (its terrain id).
    pub shown: Option<&'a str>,
    /// A claim is out.
    pub claiming: bool,
    /// Why the last claim was refused, while it's fresh.
    pub refused: Option<&'a str>,
}

/// The panel and what it keeps between frames.
#[derive(Default)]
pub struct DeedsOffice {
    open: bool,
    /// Open last frame, so it fades out after it closes.
    was_open: bool,
    page: Page,
    /// The zone whose free lots are listed.
    zone: Option<Zone>,
    sort: Sort,
    /// A claim asked for once, waiting for "Yes".
    confirm: Option<Uuid>,
    plans: Plans,
    /// Where it was drawn this frame, `None` while it's closed.
    pub rect: Option<Rect>,
    /// Each row's buttons as drawn this frame, for the tests to click.
    hits: Vec<(Uuid, Hit, Rect)>,
}

/// A row's button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Show,
    Claim,
    Yes,
    NotYet,
}

impl DeedsOffice {
    /// Opens it on the zones side by side. The caller asks for the facts.
    pub fn open(&mut self) {
        self.open = true;
        self.page = Page::Compare;
        self.confirm = None;
    }

    /// Opens it on one zone's free lots: the caller asks for them.
    pub fn open_on(&mut self, zone: Zone) {
        self.open = true;
        self.page = Page::Lots;
        self.zone = Some(zone);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.confirm = None;
    }

    /// Ask once to claim `plot` with the deed, as its first click does
    /// (for `--deeds-sample confirm`).
    pub fn ask_to_claim(&mut self, plot: Uuid) {
        self.confirm = Some(plot);
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Draws it in the middle of the screen.
    pub fn show(&mut self, ctx: &egui::Context, view: &View) -> Option<Action> {
        let shown = ctx.animate_bool_with_time(Id::new("capitol_deeds_fade"), self.open, FADE);
        self.rect = None;
        self.hits.clear();
        if shown <= 0.0 {
            self.was_open = false;
            return None;
        }
        self.was_open = self.open;
        let ease = 1.0 - (1.0 - shown).powi(3);
        let screen = ctx.available_rect();
        let mut action = None;
        let area = egui::Area::new(Id::new("capitol_deeds"))
            .order(egui::Order::Middle)
            .anchor(Align2::CENTER_CENTER, [0.0, RISE * (1.0 - ease)])
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                // Clear of the header strip above and the toasts below
                theme::panel_edged(GOLD).show(ui, |ui| {
                    action = self.body(ui, view, screen.height() - 120.0);
                });
            });
        self.rect = self.open.then_some(area.response.rect);
        if action == Some(Action::Close) {
            self.close();
        }
        action
    }

    fn body(&mut self, ui: &mut egui::Ui, view: &View, room: f32) -> Option<Action> {
        let width = columns_width(3);
        ui.set_width(width);
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(theme::title("The Deeds Office", size::HEADING + 4.0).color(GOLD));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.add(egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM)).frame(false));
                if close.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    action = Some(Action::Close);
                }
            });
        });
        ui.label(theme::dim("Where the city's land is recorded. Compare the zones, choose a lot, and claim it."));
        ui.add_space(6.0);
        let mut page = self.page;
        ui.horizontal(|ui| {
            theme::segmented(ui, &mut page, &[(Page::Compare, "The zones side by side"), (Page::Lots, "Free lots")]);
        });
        if page != self.page {
            self.page = page;
            if page == Page::Lots {
                let zone = *self.zone.get_or_insert(Zone::Urban);
                action = Some(Action::List(zone));
            }
        }
        ui.add_space(6.0);
        theme::divider(ui);
        ui.add_space(8.0);
        let inner = (room - CHROME).max(150.0);
        match self.page {
            Page::Compare => {
                if let Some(a) = self.compare(ui, view, inner) {
                    action = Some(a);
                }
            }
            Page::Lots => {
                if let Some(a) = self.lots(ui, view, inner) {
                    action = Some(a);
                }
            }
        }
        ui.add_space(8.0);
        slip(ui, view, self.taxes(view));
        if let Some(why) = view.refused {
            ui.add_space(6.0);
            theme::error_box(ui, why);
        }
        action
    }

    /// The zone card's three columns, and a way from each to its lots.
    fn compare(&mut self, ui: &mut egui::Ui, view: &View, room: f32) -> Option<Action> {
        let Some(all) = view.facts.filter(|f| !f.is_empty()) else {
            waiting(ui, view.facts.is_some(), "Fetching the records…");
            return None;
        };
        let columns: Vec<&ZoneFacts> = all.iter().collect();
        self.plans.show(ui, "capitol_deeds_compare", &columns, all, room - 30.0);
        ui.add_space(6.0);
        let mut action = None;
        // Each under its column: the columns' width and gap
        let (row, _) = ui.allocate_exact_size(egui::vec2(columns_width(columns.len()), 18.0), Sense::hover());
        let step = columns_width(2) - columns_width(1);
        for (i, f) in columns.iter().enumerate() {
            let Some(zone) = Zone::from_str(&f.zone_id) else { continue };
            let at = Rect::from_min_size(row.min + egui::vec2(step * i as f32, 0.0), egui::vec2(step, row.height()));
            let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(at).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let text = format!("{} free lots →", words::free(f));
            if theme::link(&mut cell, &text, size::LABEL).clicked() {
                self.page = Page::Lots;
                self.zone = Some(zone);
                action = Some(Action::List(zone));
            }
        }
        action
    }

    /// One zone's free lots, sorted, each to show or claim.
    fn lots(&mut self, ui: &mut egui::Ui, view: &View, room: f32) -> Option<Action> {
        let mut action = None;
        let mut zone = self.zone.unwrap_or(Zone::Urban);
        ui.horizontal(|ui| {
            let choices: Vec<(Zone, &str)> = HOME_ZONES.iter().map(|z| (*z, short_name(*z))).collect();
            theme::segmented(ui, &mut zone, &choices);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut sort = self.sort;
                // Right to left, so the choices go in backwards
                let choices: Vec<(Sort, &str)> = Sort::ALL.iter().rev().map(|s| (*s, s.label())).collect();
                ui.horizontal(|ui| theme::segmented(ui, &mut sort, &choices));
                self.sort = sort;
                ui.add_space(6.0);
                ui.label(theme::dim("Sort").size(size::SMALL));
            });
        });
        if Some(zone) != self.zone {
            self.zone = Some(zone);
            self.confirm = None;
            action = Some(Action::List(zone));
        }
        ui.add_space(6.0);
        let listed = view.lots.filter(|l| Zone::from_str(&l.zone_id) == Some(zone));
        let Some(listed) = listed else {
            waiting(ui, false, "Fetching the register…");
            return action;
        };
        if listed.lots.is_empty() {
            waiting(ui, true, "Every lot in this zone is taken.");
            return action;
        }
        let rows = deeds::sorted(&listed.lots, self.sort);
        let fact = view.facts.and_then(|f| words::find(f, zone.as_str()));
        let claims = view.grant.is_open();
        ui.label(theme::dim(format!(
            "{} free{}.",
            rows.len(),
            fact.map(|f| format!(", deed {} Strands, or your deed", f.deed_cost)).unwrap_or_default()
        )).size(size::SMALL));
        ui.add_space(2.0);
        ui.scope(|ui| {
            ui.spacing_mut().scroll.dormant_handle_opacity = 0.5;
            // Rows out of view aren't laid out, but for one past each edge,
            // which the pad's ring steps on to and the list scrolls in (#517)
            egui::ScrollArea::vertical()
                .id_salt("capitol_deeds_lots")
                .max_height(room - 44.0)
                .drag_to_scroll(false)
                .show_rows_following(ui, ROW, rows.len(), |ui, i| {
                    if let Some(a) = self.row(ui, rows[i], zone, view, claims) {
                        action = Some(a);
                    }
                });
        });
        action
    }

    /// One free lot: where it is and what it holds, "Show me", and the claim.
    fn row(&mut self, ui: &mut egui::Ui, lot: &FreeLot, zone: Zone, view: &View, claims: bool) -> Option<Action> {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW), Sense::hover());
        let shown = view.shown == Some(lot.terrain_id.as_str());
        let painter = ui.painter_at(rect);
        let ink = theme::zone(zone);
        let band = Rect::from_min_size(rect.min + egui::vec2(0.0, 6.0), egui::vec2(3.0, ROW - 12.0));
        painter.rect_filled(band, 1.5, if shown { GOLD } else { ink });
        if shown {
            painter.rect_filled(rect.shrink2(egui::vec2(0.0, 2.0)), 4.0, theme::GOLD_WASH);
        }
        // Its address, short of the buttons: a ring street's crossing goes
        // when there isn't room for it ("A yard at 112 Mercer Street")
        let room = width - 300.0 - 12.0;
        let place = |address: Option<&str>| {
            let place = deeds::whereabouts(zone, address, lot.x, lot.y);
            painter.layout_no_wrap(format!("{}{}", place[..1].to_uppercase(), &place[1..]), theme::strong(size::BODY), TEXT)
        };
        let mut galley = place(lot.address.as_deref());
        if galley.size().x > room {
            galley = place(lot.address.as_deref().map(deeds::street_only));
        }
        painter.galley(rect.min + egui::vec2(12.0, 8.0), galley, TEXT);
        let detail = format!(
            "{} stations  ·  {} sq  ·  {} to the hall",
            lot.fits,
            grouped(lot.area),
            words::walk(lot.walk_secs)
        );
        painter.text(rect.min + egui::vec2(12.0, 26.0), Align2::LEFT_TOP, detail, theme::body(size::SMALL), DIM);
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, theme::BORDER));
        // The buttons, right to left from the edge
        let mut action = None;
        let buttons = Rect::from_min_max(Pos2::new(rect.right() - 300.0, rect.top()), rect.right_bottom() - egui::vec2(8.0, 0.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(buttons)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let asking = self.confirm == Some(lot.plot_id);
        let id = lot.plot_id;
        if asking {
            let no = small(&mut child, "Not yet", DIM, true);
            self.hits.push((id, Hit::NotYet, no.rect));
            if no.clicked() {
                self.confirm = None;
            }
            let yes = small(&mut child, "Yes, claim it", GOLD, !view.claiming);
            self.hits.push((id, Hit::Yes, yes.rect));
            if yes.clicked() {
                self.confirm = None;
                action = Some(Action::Claim(lot.plot_id));
            }
            child.label(RichText::new("Spend your deed here?").color(theme::GOLD_LIGHT).size(size::SMALL));
        } else {
            if claims {
                let claim = small(&mut child, "Claim with your deed", GOLD, !view.claiming);
                self.hits.push((id, Hit::Claim, claim.rect));
                if claim.clicked() {
                    self.confirm = Some(lot.plot_id);
                }
            }
            let (text, ink) = if shown { ("Shown ✓", theme::GOLD_LIGHT) } else { ("Show me", TEXT) };
            let show = small(&mut child, text, ink, true);
            self.hits.push((id, Hit::Show, show.rect));
            if show
                .on_hover_text(if shown {
                    "Ringed in gold on the ground. Click to stop showing it"
                } else {
                    "Mark it on the ground and point the way, to walk out and look first"
                })
                .clicked()
            {
                action = Some(if shown {
                    Action::StopShowing
                } else {
                    Action::ShowMe(Shown { terrain_id: lot.terrain_id.clone(), plot_id: lot.plot_id, zone, x: lot.x, y: lot.y })
                });
            }
        }
        action
    }

    /// A week's tax on each zone's deed, for the slip: the listed zone's on
    /// its lots, every zone's beside the comparison.
    fn taxes(&self, view: &View) -> Vec<(Zone, i64)> {
        let Some(facts) = view.facts else { return Vec::new() };
        let tax = |zone: Zone| words::find(facts, zone.as_str()).map(|f| (zone, f.tax_per_week));
        match (self.page, self.zone) {
            (Page::Lots, Some(zone)) => tax(zone).into_iter().collect(),
            _ => HOME_ZONES.iter().filter_map(|z| tax(*z)).collect(),
        }
    }
}

/// A zone's name, short: "Urban", "Suburban", "Rural".
fn short_name(zone: Zone) -> &'static str {
    match zone {
        Zone::Urban => "Urban",
        Zone::Suburban => "Suburban",
        Zone::Rural => "Rural",
        _ => zone.display_name(),
    }
}

/// A row's small button, in `ink`.
pub(super) fn small(ui: &mut egui::Ui, text: &str, ink: egui::Color32, enabled: bool) -> egui::Response {
    let button = egui::Button::new(RichText::new(text).size(size::SMALL).color(ink))
        .fill(if ink == GOLD { theme::GOLD_WASH } else { theme::FIELD })
        .stroke(Stroke::new(1.0_f32, if ink == GOLD { GOLD } else { theme::BORDER }))
        .rounding(5.0)
        .min_size(egui::vec2(0.0, 24.0));
    ui.add_enabled(enabled, button).on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::NotAllowed
    })
}

/// A word in the middle while there's nothing to show.
fn waiting(ui: &mut egui::Ui, answered: bool, text: &str) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| ui.label(theme::dim(if answered { text } else { "Fetching the records…" })));
    ui.add_space(24.0);
}

/// "35,100": an area rounded to the hundred, in thousands.
fn grouped(area: f64) -> String {
    let rounded = ((area / 100.0).round() * 100.0) as i64;
    let digits = rounded.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Your deed: a slip of paper with the registry's seal, its terms and
/// where it stands. The seal is wax while it's yours to spend, pressed and
/// faded once it's spent, and only an outline before the road is walked.
fn slip(ui: &mut egui::Ui, view: &View, taxes: Vec<(Zone, i64)>) {
    egui::Frame::none()
        .fill(theme::PAPER)
        .stroke(Stroke::new(1.0_f32, theme::PAPER_EDGE))
        .rounding(4.0)
        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (seal, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), Sense::hover());
                match view.grant {
                    Grant::Open => theme::wax_seal(ui.painter(), seal.center(), 15.0),
                    Grant::Spent => {
                        ui.painter().circle_filled(seal.center(), 15.0, theme::INK_DOWN.gamma_multiply(0.35));
                        theme::tick(ui.painter(), seal.shrink(9.0), theme::PAPER);
                    }
                    Grant::Unearned => {
                        ui.painter().circle_stroke(seal.center(), 14.0, Stroke::new(1.0_f32, theme::INK_DIM.gamma_multiply(0.6)));
                    }
                }
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new("Your deed").font(theme::display(size::HEADING)).color(theme::INK));
                    ui.add(egui::Label::new(RichText::new(deeds::grant_terms(&taxes)).color(theme::INK).size(size::SMALL)).wrap());
                    ui.add(
                        egui::Label::new(
                            RichText::new(deeds::grant_state(view.grant, true)).italics().color(theme::INK_DIM).size(size::SMALL),
                        )
                        .wrap(),
                    );
                });
            });
        });
}

/// A sample register for `--deeds-sample`: the default world's lots of
/// `zone` near the hall, as `/bureaucracy/free-lots?zone=…` lists them, and
/// what each tab is answered with offline (#574). The streets are the
/// default world's nearest the hall (#483), the numbers made up.
pub fn sample_lots(zone: Zone) -> FreeLotsResponse {
    use capitol_core::protocol::rules::{CENTER_X, CENTER_Y};
    let (r, fits, area, walk0) = match zone {
        Zone::Urban => (1900.0, 4, 8_600.0, 16.0),
        Zone::Suburban => (2900.0, 7, 18_600.0, 42.0),
        _ => (4300.0, 14, 35_000.0, 72.0),
    };
    let streets: &[&str] = match zone {
        Zone::Urban => &["Canal Quay by Coopergate", "Canal Quay by Pilgrims' Road", "Canal Quay by Shearergate", "Spurrier Street by Wrightgate"],
        Zone::Suburban => &["Lupin Close", "Ivy Court", "Elderberry Road", "Redstart End", "Goldfinch Drive"],
        _ => &["Pilgrims' Road", "Quarry Road", "Dimmock Line", "Flax Road", "Scarth Line", "Hollow Lane"],
    };
    let lots = (0..40)
        .map(|k| {
            let a = 1.2 + k as f64 * 0.157;
            let (x, y) = (CENTER_X + r * a.cos(), CENTER_Y + r * a.sin());
            let n = k as usize;
            let (number, street) = (2 * (n / streets.len()) + 1 + n % 2, streets[n % streets.len()]);
            let address = match street.split_once(" by ") {
                Some((street, by)) => format!("{number} {street} by {by}"),
                None => format!("{number} {street}"),
            };
            FreeLot {
                plot_id: Uuid::from_u128(4480 + k as u128),
                terrain_id: format!("sample_{}_{k}", zone.as_str()),
                x,
                y,
                address: Some(address),
                area: area + 1_300.0 * ((k * 7) % 5) as f64,
                fits: fits + (k * 3) % 4 - 1,
                capacity: if zone == Zone::Urban { 16 } else { 25 },
                walk_secs: walk0 + 2.5 * k as f64,
            }
        })
        .collect();
    FreeLotsResponse { zone_id: zone.as_str().to_owned(), lots, deed_cost: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::zone_card::sample;

    /// Runs a few frames of the panel and returns the actions it asked for
    /// and its rect, clicking at `click` on the fifth frame.
    fn run(office: &mut DeedsOffice, view: &View, screen: egui::Vec2, click: Option<Pos2>) -> (Vec<Action>, Option<Rect>) {
        let ctx = theme::test_context();
        let mut out = (Vec::new(), None);
        for frame in 0..8 {
            let mut raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)),
                ..Default::default()
            };
            if let (Some(p), 5) = (click, frame) {
                raw.events.push(egui::Event::PointerMoved(p));
                for pressed in [true, false] {
                    raw.events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    });
                }
            }
            let _ = ctx.run(raw, |ctx| {
                theme::install(ctx);
                if let Some(a) = office.show(ctx, view) {
                    out.0.push(a);
                }
                out.1 = office.rect;
            });
        }
        out
    }

    fn view<'a>(facts: &'a [ZoneFacts], lots: Option<&'a FreeLotsResponse>, grant: Grant) -> View<'a> {
        View { facts: Some(facts), lots, grant, shown: None, claiming: false, refused: None }
    }

    #[test]
    fn it_opens_on_the_comparison_and_fits_the_smallest_screen() {
        let facts = sample();
        let mut office = DeedsOffice::default();
        office.open();
        // 1280x720 at 150% is 853x480 points
        let (actions, rect) = run(&mut office, &view(&facts, None, Grant::Open), egui::vec2(853.0, 480.0), None);
        assert!(actions.is_empty());
        let rect = rect.expect("drawn");
        assert!(rect.width() <= 853.0 && rect.height() <= 480.0, "{rect:?}");
    }

    /// Where `hit` was drawn on `plot`'s row last frame
    fn at(office: &DeedsOffice, plot: Uuid, hit: Hit) -> Pos2 {
        office
            .hits
            .iter()
            .find(|(id, h, _)| *id == plot && *h == hit)
            .map(|(.., r)| r.center())
            .unwrap_or_else(|| panic!("{hit:?} on {plot} isn't drawn: {:?}", office.hits))
    }

    #[test]
    fn the_free_lots_list_shows_a_lot_on_the_ground() {
        let facts = sample();
        let lots = sample_lots(Zone::Rural);
        let mut office = DeedsOffice::default();
        office.open_on(Zone::Rural);
        let screen = egui::vec2(1280.0, 800.0);
        let v = view(&facts, Some(&lots), Grant::Unearned);
        run(&mut office, &v, screen, None);
        let first = deeds::sorted(&lots.lots, Sort::NearestHall)[0];
        // Unearned, the deed isn't offered: only "Show me"
        assert!(office.hits.iter().all(|(_, h, _)| *h == Hit::Show));
        let p = at(&office, first.plot_id, Hit::Show);
        let (actions, _) = run(&mut office, &v, screen, Some(p));
        let Some(Action::ShowMe(shown)) = actions.into_iter().find(|a| matches!(a, Action::ShowMe(_))) else {
            panic!("no Show me");
        };
        assert_eq!((shown.zone, shown.terrain_id.as_str()), (Zone::Rural, first.terrain_id.as_str()));
        // Shown, the same button stops showing it
        let v = View { shown: Some(&first.terrain_id), ..v };
        run(&mut office, &v, screen, None);
        let p = at(&office, first.plot_id, Hit::Show);
        let (actions, _) = run(&mut office, &v, screen, Some(p));
        assert!(actions.contains(&Action::StopShowing));
    }

    #[test]
    fn a_claim_with_the_deed_is_asked_twice() {
        let facts = sample();
        let lots = sample_lots(Zone::Urban);
        let mut office = DeedsOffice::default();
        office.open_on(Zone::Urban);
        let screen = egui::vec2(1280.0, 800.0);
        let v = view(&facts, Some(&lots), Grant::Open);
        run(&mut office, &v, screen, None);
        let first = deeds::sorted(&lots.lots, Sort::NearestHall)[0].plot_id;
        let p = at(&office, first, Hit::Claim);
        let (actions, _) = run(&mut office, &v, screen, Some(p));
        assert!(!actions.iter().any(|a| matches!(a, Action::Claim(_))), "never on the first click");
        assert_eq!(office.confirm, Some(first), "asked once");
        // Not yet takes it back; asked again, yes claims it
        let p = at(&office, first, Hit::NotYet);
        let (actions, _) = run(&mut office, &v, screen, Some(p));
        assert!(actions.is_empty() && office.confirm.is_none());
        let p = at(&office, first, Hit::Claim);
        run(&mut office, &v, screen, Some(p));
        let p = at(&office, first, Hit::Yes);
        let (actions, _) = run(&mut office, &v, screen, Some(p));
        assert!(actions.contains(&Action::Claim(first)), "{actions:?}");
    }

    #[test]
    fn a_zone_chosen_asks_for_its_lots() {
        let facts = sample();
        let mut office = DeedsOffice::default();
        office.open();
        let screen = egui::vec2(1280.0, 800.0);
        let v = view(&facts, None, Grant::Open);
        let (_, rect) = run(&mut office, &v, screen, None);
        let rect = rect.unwrap();
        // "Free lots", the second tab, under the lede at the top left
        let mut asked = None;
        for y in (rect.top() as i32 + 40..rect.top() as i32 + 100).step_by(3) {
            let (actions, _) = run(&mut office, &v, screen, Some(Pos2::new(rect.left() + 190.0, y as f32)));
            if let Some(a) = actions.into_iter().find(|a| matches!(a, Action::List(_))) {
                asked = Some(a);
                break;
            }
        }
        assert_eq!(asked, Some(Action::List(Zone::Urban)), "the city out: urban first");
    }

    #[test]
    fn the_slip_taxes_by_the_zone_listed_or_every_zone() {
        let facts = sample();
        let mut office = DeedsOffice::default();
        let v = view(&facts, None, Grant::Open);
        assert_eq!(office.taxes(&v).len(), 3);
        office.open_on(Zone::Suburban);
        let taxes = office.taxes(&v);
        assert_eq!(taxes.len(), 1);
        assert_eq!(taxes[0].0, Zone::Suburban);
    }

    #[test]
    fn areas_read_in_thousands() {
        assert_eq!(grouped(35_123.0), "35,100");
        assert_eq!(grouped(8_640.0), "8,600");
    }
}
