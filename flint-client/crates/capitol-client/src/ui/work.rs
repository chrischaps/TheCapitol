//! The notice board's Work page (Commons S2.7, plan D7): piece-rate work
//! the city offers, each a card pinned to the cork beside the Wanted page's
//! commissions.
//!
//! A card says who's hiring (a link to their card), the work (what's made
//! at their station, or gathered on their lot), the rate a unit and what's
//! left of the cap, how long the offer runs, and where it is from you. A
//! named offer says it's for you; one being worked says by whom.
//!
//! *Take* walks you to the work and starts the shift as you arrive
//! (`Session::take_work`). At a station its panel then opens on the
//! employer's recipe. While you're on a shift, a slip over the cards says
//! whose work it is and what you've earned, with *Stop*, as the header's
//! chip does. Offering work is done where the work is: the station panel's
//! Hire tab, or the plot panel of a lot of yours.
//!
//! The page reads the session's `capitol_core::wages::Wages`; what reaches
//! past it comes back as an [`Action`].

use capitol_core::catalog::Catalog;
use capitol_core::chrono::{DateTime, Utc};
use capitol_core::protocol::rest::WageOffer;
use capitol_core::protocol::rules::wages::Work;
use capitol_core::wages::{self as wg, Ask, Wages};
use egui::{Align, Id, Layout, RichText, Sense};
use uuid::Uuid;

use super::board::{ink, ink_dim, name_link, note_button, on_cork, paper_card, rule, slip, tag, GAP, ICON, PIN, TWO_COLUMNS};
use super::icons::ItemIcons;
use super::theme::{self, size};

/// Where the first card's Take was drawn this frame, kept in egui's memory
/// for the tests to click.
pub const TAKE: &str = "capitol_work_take";

/// What the page needs besides the offers.
pub struct View<'a> {
    pub me: Option<Uuid>,
    /// Where you stand, server units, for each card's bearing
    pub at: Option<[f64; 2]>,
    /// The server's clock, for how long an offer runs
    pub now: DateTime<Utc>,
    pub catalog: &'a Catalog,
    /// Walking to work you're taking
    pub walking: bool,
    /// Signed in. Offline it's a sample, and nothing is sent.
    pub online: bool,
}

/// What the page asks of the rest of the game.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Walk to this offer's work and take it
    Take(Uuid),
    /// Make this call: a stop or a withdrawal
    Act(Ask),
    /// See who someone is: their card (S0.3)
    Card(Uuid),
}

/// Which offers the page lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Show {
    #[default]
    All,
    /// Those named for you
    ForMe,
    /// Your own
    Mine,
}

/// The Work page, under the board's tabs: the filters, your shift, and the cards.
pub fn page(ui: &mut egui::Ui, wages: &Wages, view: &View, icons: &mut ItemIcons, room: f32) -> Option<Action> {
    let mut action = None;
    let id = Id::new("capitol_work_show");
    let mut show: Show = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    toolbar(ui, wages, view, &mut show);
    ui.data_mut(|d| d.insert_temp(id, show));
    ui.add_space(8.0);
    news(ui, wages);
    if let Some(a) = shift_slip(ui, wages, view) {
        action = Some(a);
    }
    egui::ScrollArea::vertical()
        .id_salt("capitol_work_cards")
        .max_height(room)
        .min_scrolled_height(room)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if let Some(a) = cards(ui, wages, view, icons, show) {
                action = Some(a);
            }
        });
    action
}

/// The offers to list, by `show`.
pub fn listed(wages: &Wages, me: Option<Uuid>, show: Show) -> Vec<&WageOffer> {
    wages
        .listed_now(me)
        .into_iter()
        .filter(|o| match show {
            Show::All => true,
            Show::ForMe => o.worker_id.is_some() && o.worker_id == me,
            Show::Mine => Some(o.employer_id) == me,
        })
        .collect()
}

fn toolbar(ui: &mut egui::Ui, wages: &Wages, view: &View, show: &mut Show) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (s, label) in [(Show::All, "Everything"), (Show::ForMe, "For me"), (Show::Mine, "Mine")] {
            let n = listed(wages, view.me, s).len();
            if tag(ui, &format!("{label} · {n}"), *show == s).clicked() {
                *show = s;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(on_cork("Offer work from a station or lot of yours").size(size::SMALL).italics());
        });
    });
}

/// What the last thing you did did, or why it didn't.
fn news(ui: &mut egui::Ui, wages: &Wages) {
    let (text, colour) = match (wages.refused(), wages.done()) {
        (Some(why), _) => (why, theme::INK_DOWN),
        (None, Some(done)) => (done, theme::INK),
        _ => return,
    };
    slip(ui, |ui| {
        ui.label(RichText::new(text).size(size::SMALL).color(colour));
    });
    ui.add_space(6.0);
}

/// Your shift, if you're on one: whose work, what you've made, and Stop.
fn shift_slip(ui: &mut egui::Ui, wages: &Wages, view: &View) -> Option<Action> {
    let (offer, shift) = wages.working()?;
    let mut action = None;
    slip(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(wg::chip_words(offer, shift)).font(theme::strong(size::SMALL)).color(theme::INK));
            ui.label(ink_dim(&wg::work_words(&offer.work, view.catalog), size::SMALL));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if note_button(ui, "Stop", view.online && !wages.busy())
                    .on_hover_text("End your shift. What you made is paid already")
                    .clicked()
                {
                    action = Some(Action::Act(Ask::Stop(offer.id)));
                }
            });
        });
    });
    ui.add_space(6.0);
    action
}

fn cards(ui: &mut egui::Ui, wages: &Wages, view: &View, icons: &mut ItemIcons, show: Show) -> Option<Action> {
    let mut action = None;
    if !wages.loaded() && view.online {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(on_cork("Reading the board…")));
        return None;
    }
    let shown: Vec<WageOffer> = listed(wages, view.me, show).into_iter().cloned().collect();
    if shown.is_empty() {
        ui.add_space(20.0);
        let line = match show {
            Show::All => "No work is offered just now. Hire from a station or a lot of yours, and the city will see it here.",
            Show::ForMe => "Nobody has offered work to you by name.",
            Show::Mine => "You offer no work. Open a station of yours, or a lot, to hire.",
        };
        ui.vertical_centered(|ui| ui.label(on_cork(line).italics()));
        ui.add_space(20.0);
        return None;
    }
    let two = ui.available_width() >= TWO_COLUMNS;
    let width = if two { (ui.available_width() - GAP) / 2.0 } else { ui.available_width() };
    let mut first = true;
    for (row, pair) in shown.chunks(if two { 2 } else { 1 }).enumerate() {
        ui.add_space(PIN + 2.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for (i, o) in pair.iter().enumerate() {
                let drop = if (row + i) % 2 == 1 { 3.0 } else { 0.0 };
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(drop);
                    if let Some(a) = card(ui, wages, view, icons, o, first) {
                        action = Some(a);
                    }
                    first = false;
                });
            }
        });
        ui.add_space(GAP - PIN);
    }
    action
}

/// What the work yields, for the card's icon.
fn yields(work: &Work, catalog: &Catalog) -> Option<String> {
    match work {
        Work::Craft { recipe_id, .. } => {
            catalog.recipes().iter().find(|r| &r.id == recipe_id).and_then(|r| r.outputs.first()).map(|o| o.item_type.clone())
        }
        Work::Gather { resource, .. } => catalog.resource_type(resource).map(|r| r.yields_item.clone()),
    }
}

/// One offer's card.
fn card(ui: &mut egui::Ui, wages: &Wages, view: &View, icons: &mut ItemIcons, o: &WageOffer, first: bool) -> Option<Action> {
    let mut action = None;
    let mine = view.me == Some(o.employer_id);
    let left = (o.cap - o.made).max(0);
    let response = paper_card(mine).show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::paper_widgets(ui);
        // The work, and the rate
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), Sense::hover());
            if let Some(item) = yields(&o.work, view.catalog) {
                icons.paint(ui, rect, &item);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(wg::work_words(&o.work, view.catalog)).font(theme::strong(size::LARGE)).color(theme::INK));
                ui.label(ink_dim(wg::place_words(&o.work), size::SMALL));
            });
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                        ui.label(RichText::new(format!("{} each", o.rate)).font(theme::strong(size::BODY)).color(theme::INK));
                        ui.label(ink_dim(&format!("{} in all", o.rate.saturating_mul(left as i64)), size::SMALL));
                    });
                });
            });
        });
        ui.add_space(4.0);
        rule(ui);
        // How many, for how long, how far
        ui.horizontal(|ui| {
            ui.label(ink(&format!("{left} of {} left", o.cap), size::BODY));
            ui.label(ink_dim("·", size::BODY));
            ui.label(ink_dim(&format!("runs {}", wg::time_left(o.ends_at, view.now)), size::BODY));
            if let Some(at) = view.at {
                ui.label(ink_dim("·", size::BODY));
                ui.label(ink_dim(&wg::whereabouts(at, [o.x, o.y]), size::BODY));
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.label(ink_dim("Offered by", size::SMALL));
            if o.employer_citizen {
                theme::citizen_mark(ui, size::SMALL, Some(theme::INK));
            }
            if mine {
                ui.label(RichText::new("you").font(theme::strong(size::SMALL)).color(theme::INK));
            } else if name_link(ui, &o.employer_name) {
                action = Some(Action::Card(o.employer_id));
            }
            if o.worker_id.is_some() {
                let whom = if o.worker_id == view.me { "you".to_owned() } else { o.worker_name.clone().unwrap_or_default() };
                ui.label(ink_dim(&format!("· for {whom} alone"), size::SMALL));
            }
        });
        if let Some(shift) = &o.shift {
            let who = if Some(shift.worker_id) == view.me { "You are".to_owned() } else { format!("{} is", shift.worker_name) };
            ui.label(RichText::new(format!("{who} working it: {} this shift", shift.made)).font(theme::body(size::SMALL)).color(theme::PENCIL).italics());
        }
        ui.add_space(4.0);
        if let Some(a) = moves(ui, wages, view, o, first) {
            action = Some(a);
        }
    });
    theme::pin(ui.painter(), egui::pos2(response.response.rect.center().x, response.response.rect.top() + 1.0), PIN);
    action
}

/// What you can do with `o`: take it, stop your shift on it, or take
/// yours down.
fn moves(ui: &mut egui::Ui, wages: &Wages, view: &View, o: &WageOffer, first: bool) -> Option<Action> {
    let mut action = None;
    let busy = wages.busy() || !view.online;
    ui.horizontal(|ui| {
        if view.me == Some(o.employer_id) {
            ui.label(ink_dim(&format!("{} Strands held", o.escrow_strands), size::SMALL));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if note_button(ui, "Take it down", !busy)
                    .on_hover_text("What's unspent comes back. The court keeps its fee")
                    .clicked()
                {
                    action = Some(Action::Act(Ask::Withdraw(o.id)));
                }
                if o.shift.is_some()
                    && note_button(ui, "Stop the shift", !busy)
                        .on_hover_text("They go, paid for what they made. The offer stays up")
                        .clicked()
                {
                    action = Some(Action::Act(Ask::Stop(o.id)));
                }
            });
            return;
        }
        if o.shift.as_ref().is_some_and(|s| Some(s.worker_id) == view.me) {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if note_button(ui, "Stop", !busy).clicked() {
                    action = Some(Action::Act(Ask::Stop(o.id)));
                }
            });
            return;
        }
        let can = wages.can_take(o, view.me);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if view.walking && wages.held() == Some(&Ask::Take(o.id)) { "Walking there…" } else { "Take" };
            let hover = match can {
                Ok(()) => "Walk there and start a shift: you're paid for each one as it's made",
                Err(why) => why,
            };
            let button = note_button(ui, label, can.is_ok() && !busy).on_hover_text(hover);
            if first {
                ui.data_mut(|d| d.insert_temp(Id::new(TAKE), button.rect));
            }
            if button.clicked() {
                action = Some(Action::Take(o.id));
            }
        });
    });
    action
}

/// Offers for `--board-sample work`: work at a bench and on a farm, one
/// named for you, one of yours with someone on it, and one being worked.
pub fn sample(me: Uuid, now: DateTime<Utc>) -> Wages {
    use capitol_core::protocol::rest::{WageShift, WagesResponse};
    use capitol_core::protocol::rules::{court, wages::OfferStatus};
    let hours = capitol_core::chrono::Duration::hours;
    let offer = |n: u128, employer: &str, citizen: bool, work: Work, rate: i64, cap: i32, made: i32, x: f64, y: f64, runs: i64| WageOffer {
        id: Uuid::from_u128(0x3930 + n),
        employer_id: Uuid::from_u128(0x4000 + n),
        employer_name: employer.into(),
        employer_citizen: citizen,
        worker_id: None,
        worker_name: None,
        work,
        x,
        y,
        rate,
        cap,
        made,
        escrow_strands: rate * (cap - made) as i64,
        fee: court::court_fee(rate * cap as i64),
        posted_at: now - hours(n as i64),
        ends_at: now + hours(runs),
        status: OfferStatus::Open,
        shift: None,
    };
    let craft = |recipe: &str| Work::Craft { station_id: Uuid::from_u128(0x5000), recipe_id: recipe.into() };
    let gather = |resource: &str| Work::Gather { plot_id: Uuid::from_u128(0x6000), resource: resource.into() };
    let (cx, cy) = (capitol_core::protocol::rules::CENTER_X, capitol_core::protocol::rules::CENTER_Y);
    let handles = offer(1, "Ada Reeder", false, craft("tool_handle"), 4, 40, 19, cx + 1200.0, cy - 900.0, 60);
    let grass = offer(2, "Citizen Wren", true, gather("grass"), 1, 120, 34, cx - 4200.0, cy + 2600.0, 150);
    let mut planks = offer(3, "Odile Fenn", false, craft("plank"), 6, 25, 0, cx + 300.0, cy + 2200.0, 30);
    planks.worker_id = Some(me);
    planks.worker_name = Some("You".into());
    let shift = |n: u128, worker: Uuid, name: &str, made: i32| WageShift {
        id: Uuid::from_u128(0x7000 + n),
        offer_id: Uuid::from_u128(0x3930 + n),
        worker_id: worker,
        worker_name: name.into(),
        worker_citizen: false,
        started_at: now - hours(1),
        ended_at: None,
        made,
        end: None,
    };
    let mut yours = offer(4, "You", false, craft("rope_basic"), 2, 60, 12, cx + 40.0, cy + 30.0, 100);
    yours.employer_id = me;
    yours.shift = Some(shift(4, Uuid::from_u128(0xb21), "Brin Ashdown", 12));
    let mut worked = offer(5, "Citizen Cy", true, gather("oak_tree"), 3, 50, 8, cx - 2600.0, cy - 3100.0, 20);
    worked.shift = Some(shift(5, Uuid::from_u128(0xc1), "Citizen Pell", 8));
    let mut wages = Wages::default();
    wages.connected();
    if let Some(ask) = wages.wants() {
        wages.sent(ask);
    }
    wages.listed(WagesResponse { offers: vec![handles, grass, planks, yours, worked], shift: None });
    wages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Uuid {
        Uuid::from_u128(0x389)
    }

    /// Draws the page `frames` times at `width`, with `events` on the last.
    fn draw(wages: &Wages, width: f32, clicks: bool) -> Option<Action> {
        let ctx = theme::test_context();
        let catalog = Catalog::fallback();
        let mut icons = ItemIcons::load(std::path::Path::new(ItemIcons::DIR));
        let view = View {
            me: Some(me()),
            at: Some([capitol_core::protocol::rules::CENTER_X, capitol_core::protocol::rules::CENTER_Y]),
            now: Utc::now(),
            catalog: &catalog,
            walking: false,
            online: true,
        };
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 900.0));
        let mut action = None;
        let mut frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(a) = page(ui, wages, &view, &mut icons, 600.0) {
                        action = Some(a);
                    }
                });
            });
        };
        frame(vec![]);
        frame(vec![]);
        if clicks {
            let at = ctx.data(|d| d.get_temp::<egui::Rect>(Id::new(TAKE))).expect("a Take was drawn").center();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(vec![egui::Event::PointerMoved(at)]);
            frame(vec![button(true)]);
            frame(vec![button(false)]);
        }
        action
    }

    #[test]
    fn the_work_page_draws_wide_and_narrow() {
        let wages = sample(me(), Utc::now());
        for width in [900.0, 380.0] {
            assert_eq!(draw(&wages, width, false), None, "at {width}");
        }
        assert_eq!(draw(&Wages::default(), 900.0, false), None, "empty");
    }

    #[test]
    fn the_work_page_filters() {
        let wages = sample(me(), Utc::now());
        let ids = |show| -> Vec<u128> { listed(&wages, Some(me()), show).iter().map(|o| o.id.as_u128() - 0x3930).collect() };
        assert_eq!(ids(Show::All), [1, 2, 3, 4, 5]);
        assert_eq!(ids(Show::ForMe), [3]);
        assert_eq!(ids(Show::Mine), [4]);
    }

    #[test]
    fn take_on_a_card_takes_that_offer() {
        let wages = sample(me(), Utc::now());
        assert_eq!(draw(&wages, 900.0, true), Some(Action::Take(Uuid::from_u128(0x3931))), "the newest, Ada's handles");
    }
}
