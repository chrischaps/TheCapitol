//! Hiring (Commons S2.7, plan D7): piece-rate work offered at a station of
//! yours, from the station panel's Hire tab, or on a lot of yours, from the
//! plot panel. Both draw this.
//!
//! - **Your offers there**, each in a well: whether someone's working it
//!   and who, how many of the cap are made and what that's paid, what's
//!   still held, and how long it runs. *Stop the shift* sends the worker
//!   home, paid for what they made; *Take it down* withdraws the offer and
//!   the unspent escrow comes back.
//! - **The form** under them: the work (a recipe here, or a resource that
//!   grows on the lot), the rate a unit, the most units, the term, and
//!   whether it's open to anyone or for one person by name. The name is
//!   looked up as you leave the field (`GET /players/by-name`).
//! - **The court's numbers**, before you commit: the escrow is
//!   `rules::wages::offer_value`, the fee `rules::court::court_fee` on it,
//!   through `capitol_core::wages::Draft`, so the preview is what's charged.
//!
//! The panel keeps the [`Draft`]; the offers and the name's lookup are the
//! session's `Wages`. What reaches past the panel comes back as an [`Act`].

use capitol_core::catalog::Catalog;
use capitol_core::chrono::{DateTime, Utc};
use capitol_core::protocol::rest::WageOffer;
use capitol_core::protocol::rules::wages::{self as rules, Work};
use capitol_core::wages::{self as wg, Ask, Draft, Named, Wages};
use egui::{Color32, Id, RichText};
use uuid::Uuid;

use super::theme::{self, size, DIM, EMBER, ENOUGH, PILL, SHORT, TEXT};

/// Where the form's button was drawn this frame, kept in egui's memory for
/// the tests to click.
pub const GO: &str = "capitol_hire_go";

/// What the form asks of the rest of the game.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Make this call: an offer, a withdrawal or a stop
    Ask(Ask),
    /// Look up who this draft's name is
    LookUp(Draft),
}

/// What the form draws from besides the draft.
pub struct View<'a> {
    pub wages: &'a Wages,
    pub me: Option<Uuid>,
    /// Your Strands, once known
    pub strands: Option<i64>,
    pub catalog: &'a Catalog,
    /// The server's clock, for how long an offer runs
    pub now: DateTime<Utc>,
    /// Signed in. Offline it's a sample, and nothing is sent.
    pub online: bool,
    /// The panel's accent, for the button
    pub accent: Color32,
}

/// The court's numbers for a draft, as the form shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preview {
    pub escrow: i64,
    pub fee: i64,
    pub total: i64,
}

pub fn preview(draft: &Draft) -> Preview {
    Preview { escrow: draft.value(), fee: draft.fee(), total: draft.total() }
}

/// Your offers at `place`, then the form for another. `choices` are what
/// the work may be: (id, name) of the recipes at the station, or of the
/// resources on the lot.
pub fn show(ui: &mut egui::Ui, place: Uuid, draft: &mut Draft, choices: &[(String, String)], view: &View) -> Option<Act> {
    let mut act = None;
    let mine = view.wages.mine_at(view.me, place);
    for offer in &mine {
        if let Some(a) = running(ui, offer, view) {
            act = Some(a);
        }
        ui.add_space(6.0);
    }
    if let Some(done) = view.wages.done() {
        ui.label(RichText::new(done).size(size::SMALL).color(ENOUGH));
        ui.add_space(4.0);
    }
    if !mine.is_empty() {
        ui.label(theme::label("Offer more work"));
    }
    if let Some(a) = form(ui, draft, choices, view) {
        act = Some(a);
    }
    act
}

/// One offer of yours: who's on it, how far it's got, and Stop and Take it down.
fn running(ui: &mut egui::Ui, offer: &WageOffer, view: &View) -> Option<Act> {
    let mut act = None;
    let busy = view.wages.busy() || !view.online;
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let t = ui.input(|i| i.time) as f32;
            match &offer.shift {
                Some(_) => {
                    ui.ctx().request_repaint();
                    theme::pill(ui, "Working", view.accent.gamma_multiply(0.45 + 0.15 * (t * 2.4).sin()));
                }
                None => {
                    theme::pill(ui, "Open", PILL);
                }
            }
            ui.label(theme::text(wg::work_words(&offer.work, view.catalog)).size(size::SMALL));
        });
        ui.add_space(2.0);
        let paid = offer.rate.saturating_mul(offer.made as i64);
        ui.label(
            theme::text(format!("{} of {} made · {paid} Strands paid", offer.made, offer.cap)).size(size::SMALL),
        );
        ui.label(
            theme::dim(format!(
                "{} each · {} held · {} left to run",
                offer.rate,
                offer.escrow_strands,
                wg::time_left(offer.ends_at, view.now)
            ))
            .size(size::LABEL),
        );
        match (&offer.shift, &offer.worker_name) {
            (Some(shift), _) => {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    if shift.worker_citizen {
                        theme::citizen_mark(ui, size::SMALL, None);
                    }
                    ui.label(RichText::new(&shift.worker_name).font(theme::strong(size::SMALL)).color(TEXT));
                    ui.label(theme::dim(format!("is working: {} this shift", shift.made)).size(size::SMALL));
                });
            }
            (None, Some(name)) => {
                ui.label(theme::dim(format!("For {name} alone")).size(size::LABEL));
            }
            (None, None) => {
                ui.label(theme::dim("Open to anyone; nobody's on it yet").size(size::LABEL));
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if offer.shift.is_some()
                && ui
                    .add_enabled(!busy, egui::Button::new(theme::text("Stop the shift").size(size::SMALL)))
                    .on_hover_text("They go, paid for what they made. The offer stays up for another")
                    .clicked()
            {
                act = Some(Act::Ask(Ask::Stop(offer.id)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !busy && theme::link(ui, "Take it down", size::LABEL)
                    .on_hover_text("What's unspent comes back. The court keeps its fee")
                    .clicked()
                {
                    act = Some(Act::Ask(Ask::Withdraw(offer.id)));
                }
            });
        });
    });
    act
}

/// The form for an offer: the work, the terms, who it's for, the court's
/// numbers, and the button.
fn form(ui: &mut egui::Ui, draft: &mut Draft, choices: &[(String, String)], view: &View) -> Option<Act> {
    let mut act = None;
    if choices.is_empty() {
        ui.label(theme::dim(match draft.work {
            Work::Craft { .. } => "Nothing is made here yet",
            Work::Gather { .. } => "Nothing grows on this lot to gather",
        }));
        return None;
    }
    let kind = draft.work.kind();
    let (verb, picked) = match &mut draft.work {
        Work::Craft { recipe_id, .. } => ("Makes:", recipe_id),
        Work::Gather { resource, .. } => ("Gathers:", resource),
    };
    if !choices.iter().any(|(id, _)| id == picked) {
        *picked = choices[0].0.clone();
    }
    let shown = choices.iter().find(|(id, _)| id == picked).map_or_else(String::new, |(_, n)| n.clone());
    let mut chose = None;
    ui.horizontal(|ui| {
        ui.label(theme::dim(verb));
        egui::ComboBox::from_id_salt(("capitol_hire_work", kind))
            .width(ui.available_width())
            .selected_text(theme::text(shown))
            .show_ui(ui, |ui| {
                for (id, name) in choices {
                    if ui.selectable_label(id == picked, name).clicked() {
                        chose = Some(id.clone());
                    }
                }
            });
    });
    if let Some(id) = chose.filter(|id| id != picked) {
        *picked = id;
        draft.rate = wg::rate_for(&draft.work, view.catalog);
    }
    ui.add_space(4.0);
    egui::Grid::new(("capitol_hire_terms", draft.work.kind())).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label(theme::dim("Pay"));
        ui.add(
            egui::DragValue::new(&mut draft.rate)
                .range(1..=1_000_000)
                .speed(0.2)
                .custom_formatter(|n, _| if n as i64 == 1 { "1 Strand each".into() } else { format!("{} Strands each", n as i64) }),
        );
        ui.end_row();
        ui.label(theme::dim("Up to"));
        ui.add(egui::DragValue::new(&mut draft.cap).range(1..=10_000).speed(0.25).suffix(" units"));
        ui.end_row();
        ui.label(theme::dim("For"));
        ui.add(
            egui::DragValue::new(&mut draft.term_days)
                .range(rules::MIN_TERM_DAYS..=rules::MAX_TERM_DAYS)
                .speed(0.05)
                .custom_formatter(|d, _| if d as i64 == 1 { "1 day".into() } else { format!("{} days", d as i64) }),
        );
        ui.end_row();
        ui.label(theme::dim("Open to"));
        ui.horizontal(|ui| {
            theme::segmented(ui, &mut draft.named, &[(false, "Anyone"), (true, "Someone")]);
        });
        ui.end_row();
    });
    if draft.named {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(theme::dim("Name:"));
            let field = ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(120.0).hint_text("who it's for"));
            if field.lost_focus() || (field.changed() && draft.name.ends_with(' ')) {
                act = Some(Act::LookUp(draft.clone()));
            }
            match view.wages.named() {
                Named::Found { asked, name, citizen, .. } if asked == draft.name() => {
                    if *citizen {
                        theme::citizen_mark(ui, size::SMALL, None);
                    }
                    ui.label(RichText::new(format!("✓ {name}")).size(size::SMALL).color(ENOUGH));
                }
                Named::Unknown(n) if n == draft.name() => {
                    ui.label(RichText::new("no one by that name").size(size::SMALL).color(SHORT));
                }
                Named::Asking(n) if n == draft.name() => {
                    ui.label(theme::dim("looking…").size(size::SMALL));
                }
                _ if !draft.name().is_empty() && theme::link(ui, "Check", size::LABEL).clicked() => {
                    act = Some(Act::LookUp(draft.clone()));
                }
                _ => {}
            }
        });
    }
    ui.add_space(6.0);
    let p = preview(draft);
    egui::Grid::new(("capitol_hire_sums", draft.work.kind())).num_columns(2).spacing([14.0, 2.0]).show(ui, |ui| {
        for (what, n) in [("Held in escrow", p.escrow), ("The court's fee", p.fee)] {
            ui.label(theme::dim(what).size(size::SMALL));
            ui.label(theme::text(n.to_string()).size(size::SMALL));
            ui.end_row();
        }
        ui.label(RichText::new("It takes").font(theme::strong(size::BODY)).color(TEXT));
        ui.label(RichText::new(format!("{} Strands", p.total)).font(theme::strong(size::BODY)).color(TEXT));
        ui.end_row();
        if let Some(s) = view.strands {
            ui.label(theme::dim("You'd have").size(size::SMALL));
            ui.label(theme::dim((s - p.total).to_string()).size(size::SMALL));
            ui.end_row();
        }
    });
    ui.label(
        theme::dim("Each unit is paid as it's made, out of the escrow. What's unspent comes back; the fee doesn't.")
            .size(size::LABEL),
    );
    ui.add_space(6.0);
    let stop = view.wages.stopped_by(draft, view.strands, view.me);
    if let Some(why) = view.wages.refused().or(stop.as_deref()) {
        ui.label(RichText::new(why).size(size::SMALL).color(if stop.is_some() { DIM } else { EMBER }));
        ui.add_space(4.0);
    }
    let label = if view.wages.busy() { "Offering…" } else { "Offer the work" };
    let ready = view.online && stop.is_none() && !view.wages.busy();
    let button = theme::accent_button(ui, label, view.accent, ready);
    ui.data_mut(|d| d.insert_temp(Id::new(GO), button.rect));
    if button.clicked() {
        if let Some(ask) = view.wages.post(draft, view.strands, view.me) {
            act = Some(Act::Ask(ask));
        }
    }
    act
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::protocol::rules::court;

    fn bench() -> Work {
        Work::Craft { station_id: Uuid::from_u128(0x235), recipe_id: "tool_handle".into() }
    }

    /// The form's preview is the court's: what the server charges.
    #[test]
    fn the_hire_preview_is_the_courts() {
        for (rate, cap) in [(1, 1), (4, 40), (12, 250), (900, 10_000)] {
            let mut d = Draft::new(bench(), rate);
            d.cap = cap;
            let p = preview(&d);
            assert_eq!(p.escrow, rules::offer_value(rate, cap));
            assert_eq!(p.fee, court::court_fee(rules::offer_value(rate, cap)));
            assert_eq!(p.total, p.escrow + p.fee);
        }
    }

    /// Draws the form on `wages` and clicks its button: what it asks.
    fn click_go(wages: &Wages, draft: &mut Draft) -> Option<Act> {
        let ctx = theme::test_context();
        let catalog = Catalog::fallback();
        let view = View {
            wages,
            me: Some(Uuid::from_u128(1)),
            strands: Some(1000),
            catalog: &catalog,
            now: Utc::now(),
            online: true,
            accent: theme::STATION,
        };
        let choices = vec![("tool_handle".to_owned(), "Tool Handle".to_owned())];
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 800.0));
        let mut act = None;
        let mut frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                theme::install(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(a) = show(ui, Uuid::from_u128(0x235), draft, &choices, &view) {
                        act = Some(a);
                    }
                });
            });
        };
        frame(vec![]);
        frame(vec![]);
        let at = ctx.data(|d| d.get_temp::<egui::Rect>(Id::new(GO))).expect("the button was drawn").center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![button(true)]);
        frame(vec![button(false)]);
        act
    }

    #[test]
    fn the_hire_form_offers_the_draft() {
        let mut wages = Wages::default();
        wages.connected();
        wages.sent(Ask::List);
        wages.listed(Default::default());
        let mut draft = Draft::new(bench(), 4);
        draft.cap = 30;
        let Some(Act::Ask(Ask::Offer(offer))) = click_go(&wages, &mut draft) else { panic!("an offer") };
        assert_eq!(offer, draft.request(None));
        assert_eq!((offer.rate, offer.cap, offer.worker_id), (4, 30, None));
        // Named, but not yet looked up: the button won't go
        draft.named = true;
        draft.name = "Brin".into();
        assert_eq!(click_go(&wages, &mut draft), None);
    }
}
