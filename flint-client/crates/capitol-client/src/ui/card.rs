//! A person's card (S0.3): who someone is, top right under the header.
//!
//! A click on a person opens it (`input.rhai`, after the trade's
//! shift-click), and so does a name in the Gazette's columns; the chat
//! log's names will too (S1.3). The × closes it. Not Esc: Flint's player
//! quits on Esc.
//!
//! It says what anyone in the city could know: the name, with the lozenge
//! for a citizen; how long they've been here; where they live; what's said
//! of them (standing in words, D8: "new in town" until S3.1) and their guild
//! once there are guilds (S4.1). Then Trade, as T or a shift-click asks.
//! Write (S1.5) and Mute (S1.4) join it when those land.
//!
//! The card is edged in the light its person carries: a person's amber
//! lantern, a citizen's pale one, and gold for your own, the colour of
//! what's yours. The words are `capitol_core::card`'s.

use capitol_core::card::{home_words, time_in_city, Hint, Shown};
use capitol_core::protocol::rest::PlayerCard;
use egui::{Align2, Color32, Id, RichText};
use uuid::Uuid;

use super::theme::{self, size, DIM, GOLD, TEXT};

/// The card's width inside its frame.
const WIDTH: f32 = 236.0;
/// How long it takes to open or close, in seconds, and how far it rises.
const FADE: f32 = 0.18;
const RISE: f32 = 10.0;
/// The name's size, in the inscriptional capitals.
const NAME: f32 = 19.0;

/// What the card asks for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Ask them to trade.
    Trade(Uuid),
    Close,
}

/// What the card draws from.
pub struct View<'a> {
    pub shown: Shown<'a>,
    /// Today's game day (`rules::day_number`), for how long they've been here.
    pub today: i64,
    /// Signed in, so Trade can ask. Offline it's a sample.
    pub online: bool,
}

/// What the card shows, kept so it can fade out after the session lets go.
#[derive(Debug, Clone, PartialEq)]
enum Drawn {
    Asking(Option<Hint>),
    Card(PlayerCard),
    Missing,
    Failed,
}

/// What the card keeps between frames.
#[derive(Default)]
pub struct CardPanel {
    last: Option<Drawn>,
    /// Where it was drawn this frame, for the hints to keep clear of. `None`
    /// while it's closed.
    pub rect: Option<egui::Rect>,
}

impl CardPanel {
    pub fn show(&mut self, ctx: &egui::Context, view: &View) -> Option<Action> {
        let now = match view.shown {
            Shown::Closed => None,
            Shown::Asking(hint) => Some(Drawn::Asking(hint.cloned())),
            Shown::Card(card) => Some(Drawn::Card(card.clone())),
            Shown::Missing => Some(Drawn::Missing),
            Shown::Failed => Some(Drawn::Failed),
        };
        let open = now.is_some();
        if now.is_some() {
            self.last = now;
        }
        let shown = ctx.animate_bool_with_time(Id::new("capitol_card_fade"), open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            self.last = None;
            return None;
        }
        let drawn = self.last.as_ref()?;
        let ease = 1.0 - (1.0 - shown).powi(3);
        // Under the header, as the panels are (anchors are screen-relative)
        let top = ctx.available_rect().top() + 12.0;
        let area = egui::Area::new(Id::new("capitol_card"))
            .anchor(Align2::RIGHT_TOP, [-16.0, top - RISE * (1.0 - ease)])
            .interactable(open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(edge(drawn)).show(ui, |ui| body(ui, drawn, view)).inner
            });
        self.rect = open.then_some(area.response.rect);
        area.inner
    }
}

/// The light its person carries: gold for yours.
fn edge(drawn: &Drawn) -> Color32 {
    let (citizen, mine) = match drawn {
        Drawn::Card(card) => (card.citizen, card.mine),
        Drawn::Asking(Some(hint)) => (hint.citizen, false),
        _ => (false, false),
    };
    match (mine, citizen) {
        (true, _) => GOLD,
        (_, true) => theme::CITIZEN_LANTERN,
        _ => theme::LANTERN,
    }
}

fn body(ui: &mut egui::Ui, drawn: &Drawn, view: &View) -> Option<Action> {
    ui.set_width(WIDTH);
    let mut action = None;
    let (name, citizen) = match drawn {
        Drawn::Card(card) => (Some(card.name.as_str()), card.citizen),
        Drawn::Asking(Some(hint)) => (Some(hint.name.as_str()), hint.citizen),
        _ => (None, false),
    };
    ui.horizontal(|ui| {
        if citizen {
            theme::citizen_mark(ui, NAME, None);
        }
        let name = name.unwrap_or("Someone");
        ui.label(RichText::new(name).font(theme::display(NAME)).color(TEXT).extra_letter_spacing(0.6));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui.add(egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM)).frame(false));
            if close.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                action = Some(Action::Close);
            }
        });
    });
    ui.add_space(2.0);
    theme::rule_in(ui, edge(drawn).gamma_multiply(0.45));
    ui.add_space(6.0);
    match drawn {
        Drawn::Card(card) => {
            if let Some(a) = known(ui, card, view) {
                action = Some(a);
            }
        }
        Drawn::Asking(_) => {
            ui.label(theme::dim("Asking after them…").italics());
        }
        Drawn::Missing => {
            ui.label(theme::dim("No one in the city goes by that name.").italics());
        }
        Drawn::Failed => {
            ui.label(theme::dim("The city's records can't be reached just now.").italics());
        }
    }
    action
}

/// A card the server answered with.
fn known(ui: &mut egui::Ui, card: &PlayerCard, view: &View) -> Option<Action> {
    theme::list_spacing(ui);
    if card.citizen {
        ui.label(RichText::new("A citizen of the Capitol").font(theme::body(size::SMALL)).color(theme::CITIZEN_LANTERN).italics());
    }
    ui.label(theme::text(time_in_city(card.since, view.today)));
    ui.label(theme::text(home_words(card.home.as_deref(), card.mine)));
    if let Some(guild) = &card.guild {
        ui.label(theme::text(format!("Of the {guild}")));
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(theme::dim("Spoken of as"));
        ui.label(RichText::new(&card.standing).font(theme::body(size::BODY)).color(GOLD).italics());
    });
    if card.mine {
        ui.add_space(6.0);
        ui.label(theme::dim("This is how the city sees you.").size(size::SMALL));
        return None;
    }
    ui.add_space(8.0);
    // Write (S1.5, #385) and Mute (S1.4, #384) stand beside it once they land
    let trade = ui.horizontal(|ui| theme::accent_button(ui, "Trade", theme::TRADE, view.online)).inner;
    trade
        .on_hover_text("Ask them to trade (or shift-click them)")
        .clicked()
        .then_some(Action::Trade(card.id))
}

/// `--card-sample [citizen|person|mine]`: what a click on that one shows,
/// the server's answer already in.
pub fn sample(which: &str) -> (capitol_core::card::Cards, i64) {
    use capitol_core::card::{Cards, Who};
    use capitol_core::protocol::rules;
    let today = rules::GAZETTE_FIRST_DAY + 30;
    let mut card: PlayerCard = capitol_core::serde_json::from_str(include_str!(
        "../../../../../protocol/tests/fixtures/rest/player_card.json"
    ))
    .expect("the card fixture parses");
    match which {
        "person" => {
            card = PlayerCard {
                id: Uuid::from_u128(0xada),
                name: "Ada Reeder".into(),
                citizen: false,
                since: today - 3,
                home: Some("suburban".into()),
                ..card
            }
        }
        "mine" => card = PlayerCard { name: "You".into(), citizen: false, home: None, since: today, mine: true, ..card },
        _ => {}
    }
    let mut cards = Cards::default();
    let who = Who::Id(card.id);
    cards.open(who.clone(), None);
    cards.sent(who);
    cards.answered(card);
    (cards, today)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::card::{Cards, Who};

    /// Runs a few frames of the card, clicking at `click_at` on the fifth,
    /// and returns any action and where it was drawn.
    fn run(cards: &Cards, today: i64, width: f32, click_at: Option<egui::Pos2>) -> (Option<Action>, Option<egui::Rect>) {
        let ctx = theme::test_context();
        let mut panel = CardPanel::default();
        let mut out = (None, None);
        for frame in 0..8 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 720.0))),
                ..Default::default()
            };
            if let (Some(p), 5) = (click_at, frame) {
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
                if let Some(a) = panel.show(ctx, &View { shown: cards.shown(), today, online: true }) {
                    out.0 = Some(a);
                }
                out.1 = panel.rect;
            });
        }
        out
    }

    fn sample_card(which: &str) -> PlayerCard {
        match sample(which).0.shown() {
            Shown::Card(card) => card.clone(),
            other => panic!("{which}: {other:?}"),
        }
    }

    #[test]
    fn every_sample_card_draws_top_right_on_a_small_screen() {
        for which in ["citizen", "person", "mine"] {
            let (cards, today) = sample(which);
            let (action, rect) = run(&cards, today, 853.0, None);
            let rect = rect.expect("drawn");
            assert!((853.0 - rect.right() - 16.0).abs() < 1.0 && rect.width() < 300.0, "{which}: {rect:?}");
            assert_eq!(action, None);
        }
    }

    #[test]
    fn trade_on_someone_elses_card_asks_them() {
        let (cards, today) = sample("person");
        let (_, rect) = run(&cards, today, 1280.0, None);
        let rect = rect.expect("drawn");
        // The button stands at the card's foot, on the left
        let at = egui::pos2(rect.left() + 34.0, rect.bottom() - 24.0);
        let (action, _) = run(&cards, today, 1280.0, Some(at));
        assert_eq!(action, Some(Action::Trade(Uuid::from_u128(0xada))));
    }

    #[test]
    fn the_card_is_edged_in_its_persons_light() {
        let card = |citizen, mine| Drawn::Card(PlayerCard { citizen, mine, ..sample_card("citizen") });
        assert_eq!(edge(&card(true, false)), theme::CITIZEN_LANTERN);
        assert_eq!(edge(&card(false, false)), theme::LANTERN);
        assert_eq!(edge(&card(false, true)), GOLD);
        assert_eq!(edge(&Drawn::Asking(Some(Hint { name: "Cy".into(), citizen: true }))), theme::CITIZEN_LANTERN);
    }

    #[test]
    fn a_card_still_asking_shows_the_name_the_click_knew() {
        let mut cards = Cards::default();
        cards.open(Who::Id(Uuid::from_u128(5)), Some(Hint { name: "Bea".into(), citizen: false }));
        let (_, rect) = run(&cards, 0, 1280.0, None);
        assert!(rect.is_some());
    }
}
