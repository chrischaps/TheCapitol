//! Trading (P7.3): React's `TradeRequestNotification.tsx` and
//! `TradeWindow.tsx`, as one panel that shows whichever the trade needs.
//!
//! - **A request** is a small card under the header. Sent to you, it names
//!   who's asking, with Accept and Decline. Sent by you, it says you're
//!   waiting, with Withdraw: the server never says a request arrived, and
//!   keeps it until it's answered. While you walk over to ask someone, it
//!   says that.
//! - **The trade window** is two sides, yours and theirs. Drag stacks in
//!   from the inventory and type the Strands; set how many of a stack with
//!   its number, and take it out with ×. Your side is a
//!   `capitol_core::trading::Draft`, sent once you stop changing it, so
//!   there's no Update button. Every change the server takes resets both
//!   accepts, and the window says so. Accept waits until the server has
//!   confirmed your side as you see it.
//!
//! It's edged in the Trade District's blue. The card is centred under the
//! header; the window stands bottom centre, between the controls and the
//! inventory you drag from, so the two of you stay in view above it.

use std::time::Duration;

use super::hud::grouped;
use super::icons::ItemIcons;
use super::slots;
use super::theme::{self, size, DIM, ENOUGH, GOLD, SHORT, TEXT, TRADE};
use capitol_core::protocol::rest::SlotItem;
use capitol_core::protocol::ws::TradeItemInfo;
use capitol_core::state::{Container, OpenTrade, Trade, TradeAction};
use capitol_core::trading::{Draft, Problem};
use egui::{Align2, Id, RichText, Stroke};
use uuid::Uuid;

/// One side of the window, inside the frame.
const SIDE: f32 = 250.0;
/// The gap between the sides.
const GUTTER: f32 = 14.0;
/// The request card's width inside its frame.
const CARD: f32 = 300.0;
/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;
/// How far it drops as it opens, in points.
const RISE: f32 = 10.0;
/// Below the top of the space the header leaves.
const TOP: f32 = 14.0;
/// Above the bottom of the window, for the trade window.
const BOTTOM: f32 = 16.0;
/// How long "the offer changed" stays in the status line.
const CHANGED_FOR: Duration = Duration::from_secs(4);
/// How tall each side's list stands, at least, in points.
const WELL_HEIGHT: f32 = 76.0;
/// An offered stack's icon, in points.
const ROW_ICON: f32 = 24.0;

/// What the panel asks for this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Accept (true) or decline the request you were sent.
    Answer(bool),
    /// Forget the request you sent.
    Withdraw,
    /// Offer this stack, all of it.
    Add(SlotItem),
    /// Offer this many of a stack of `stack`.
    Quantity {
        item_id: Uuid,
        quantity: i32,
        stack: i32,
    },
    Remove(Uuid),
    Strands(i64),
    Accept,
    Cancel,
}

/// What the panel draws from.
pub struct View<'a> {
    pub trade: &'a Trade,
    pub draft: &'a Draft,
    pub inventory: Option<&'a Container>,
    pub strands: Option<i64>,
    /// The trade command waiting on the server.
    pub waiting: Option<TradeAction>,
    /// The last change to an offer: whether it was theirs, and how long ago.
    pub offer_changed: Option<(bool, Duration)>,
    /// The player you're walking to, to ask.
    pub walking_to: Option<&'a str>,
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct TradePanel {
    /// The Strands field, as typed.
    strands: String,
    /// The trade the window was last drawn for.
    trade_id: Option<Uuid>,
    /// Where the window or the card was drawn this frame. `None` when
    /// there's nothing to show.
    pub rect: Option<egui::Rect>,
    /// The drop well on your side, this frame, for the tests.
    well: Option<egui::Rect>,
    /// The Accept button, this frame, for the tests.
    accept: Option<egui::Rect>,
}

impl TradePanel {
    /// A trade window is open.
    pub fn is_open(&self) -> bool {
        self.trade_id.is_some()
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        icons: &mut ItemIcons,
        view: &View,
    ) -> Option<Action> {
        self.rect = None;
        self.well = None;
        self.accept = None;
        let open = match view.trade {
            Trade::Open(open) => Some(open),
            _ => None,
        };
        let id = open.map(|o| o.trade_id);
        if id != self.trade_id {
            self.trade_id = id;
            self.strands.clear();
        }
        let showing = match view.trade {
            Trade::Idle => view.walking_to.is_some(),
            _ => true,
        };
        let shown = ctx.animate_bool_with_time(Id::new("capitol_trade_fade"), showing, FADE);
        if shown <= 0.0 || !showing {
            return None;
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        // The card hangs under the header. The window stands at the bottom,
        // so the two of you, and the strand between you, stay in view.
        let (align, offset) = if open.is_some() {
            (Align2::CENTER_BOTTOM, [0.0, -BOTTOM + RISE * (1.0 - ease)])
        } else {
            let top = ctx.available_rect().top() + TOP - RISE * (1.0 - ease);
            (Align2::CENTER_TOP, [0.0, top])
        };
        let area = egui::Area::new(Id::new(("capitol_trade", open.is_some())))
            .anchor(align, offset)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(TRADE)
                    .show(ui, |ui| match (view.trade, open) {
                        (_, Some(open)) => self.window(ui, icons, view, open),
                        (Trade::Incoming { name, .. }, _) => incoming(ui, name, view),
                        (Trade::Requested { name, .. }, _) => asking(ui, name),
                        _ => walking(ui, view.walking_to.unwrap_or_default()),
                    })
                    .inner
            });
        self.rect = Some(area.response.rect);
        area.inner
    }

    fn window(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
        open: &OpenTrade,
    ) -> Option<Action> {
        ui.set_width(SIDE * 2.0 + GUTTER);
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(theme::heading("Trading with").color(DIM));
            ui.label(theme::heading(&open.partner_name).color(TRADE));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.add(
                    egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM))
                        .frame(false),
                );
                if close
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Cancel the trade")
                    .clicked()
                {
                    action = Some(Action::Cancel);
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GUTTER;
            ui.vertical(|ui| {
                ui.set_width(SIDE);
                if let Some(a) = self.mine(ui, icons, view, open) {
                    action = Some(a);
                }
            });
            ui.vertical(|ui| {
                ui.set_width(SIDE);
                theirs(ui, icons, open);
            });
        });
        ui.add_space(6.0);
        let line = status(view, open);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(line.0).color(line.1).size(size::BODY));
        });
        ui.add_space(8.0);
        let both_empty = view.draft.is_empty() && open.theirs == Default::default();
        let ready = !open.i_accepted
            && !view.draft.unsent()
            && view.draft.confirmed(&open.mine)
            && view.draft.problem(view.strands).is_none()
            && view.waiting.is_none()
            && !both_empty;
        let text = if open.i_accepted {
            "Accepted"
        } else if view.waiting == Some(TradeAction::Accept) {
            "Accepting…"
        } else {
            "Accept trade"
        };
        ui.columns(2, |cols| {
            if cols[0]
                .vertical_centered_justified(|ui| {
                    ui.add_sized(
                        [ui.available_width(), 38.0],
                        egui::Button::new(RichText::new("Cancel").color(TEXT)),
                    )
                })
                .inner
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                action = Some(Action::Cancel);
            }
            let accept =
                theme::accent_button(&mut cols[1], text, ENOUGH, ready).on_hover_cursor(if ready {
                    egui::CursorIcon::PointingHand
                } else {
                    egui::CursorIcon::NotAllowed
                });
            self.accept = Some(accept.rect);
            if accept.clicked() && ready {
                action = Some(Action::Accept);
            }
        });
        action
    }

    /// Your side: the drop well, each stack with its number and ×, the
    /// Strands, and whether you've accepted.
    fn mine(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
        open: &OpenTrade,
    ) -> Option<Action> {
        let mut action = None;
        side_title(ui, "Your offer", open.i_accepted);
        ui.add_space(4.0);
        let dragging = slots::dragging_item(ui.ctx());
        let inner = theme::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(WELL_HEIGHT);
            theme::list_spacing(ui);
            if view.draft.lines().is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    ui.label(theme::dim("Drag items here").size(size::BODY));
                    ui.label(theme::dim("from your inventory").size(size::SMALL));
                });
            }
            for line in view.draft.lines() {
                let Some(item) = view.inventory.and_then(|i| i.find(line.item_id)) else {
                    continue;
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (rect, _) = ui
                        .allocate_exact_size(egui::vec2(ROW_ICON, ROW_ICON), egui::Sense::hover());
                    icons.paint(ui, rect, &item.item_type);
                    ui.label(theme::text(item.item_name.clone()).size(size::BODY))
                        .on_hover_text(format!("Quality {}", item.quality));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let remove =
                            ui.add(egui::Button::new(RichText::new("×").color(DIM)).frame(false));
                        if remove
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("Take it out")
                            .clicked()
                        {
                            action = Some(Action::Remove(line.item_id));
                        }
                        if item.quantity > 1 {
                            ui.label(theme::dim(format!("of {}", item.quantity)).size(size::SMALL));
                            let mut quantity = line.quantity;
                            let field = ui.add(
                                egui::DragValue::new(&mut quantity)
                                    .range(1..=item.quantity)
                                    .speed(0.1),
                            );
                            if field.changed() {
                                action = Some(Action::Quantity {
                                    item_id: line.item_id,
                                    quantity,
                                    stack: item.quantity,
                                });
                            }
                        } else {
                            ui.label(theme::dim("×1").size(size::SMALL));
                        }
                    });
                });
            }
        });
        let well = inner.response.rect;
        self.well = Some(well);
        let drop = ui.interact(well, Id::new("capitol_trade_drop"), egui::Sense::hover());
        if dragging {
            let over = slots::item_over(&drop);
            let edge = if over {
                TRADE
            } else {
                TRADE.gamma_multiply(0.4)
            };
            ui.painter()
                .rect_stroke(well, 6.0, Stroke::new(if over { 2.0 } else { 1.0 }, edge));
            if over {
                ui.painter()
                    .rect_filled(well, 6.0, TRADE.gamma_multiply(0.08));
            }
        }
        if let Some(item) = slots::dropped_item(&drop) {
            action = Some(Action::Add(item));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::dim("Strands").size(size::BODY));
            if self.strands.is_empty() && view.draft.strands() > 0 {
                self.strands = view.draft.strands().to_string();
            }
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.strands)
                    .hint_text("0")
                    .desired_width(70.0)
                    .char_limit(12),
            );
            if field.changed() {
                self.strands.retain(|c| c.is_ascii_digit() || c == ',');
                let typed = self.strands.replace(',', "").parse::<i64>().unwrap_or(0);
                action = Some(Action::Strands(typed));
            }
            if let Some(b) = view.strands {
                ui.label(theme::dim(format!("of {}", grouped(b))).size(size::SMALL));
            }
        });
        if let Some(Problem::Short { short, .. }) = view.draft.problem(view.strands) {
            ui.label(
                RichText::new(format!("{} Strands short", grouped(short)))
                    .color(SHORT)
                    .size(size::SMALL),
            );
        }
        action
    }

    /// The window's Accept button and your drop well this frame, for tests.
    #[cfg(test)]
    fn parts(&self) -> (Option<egui::Rect>, Option<egui::Rect>) {
        (self.well, self.accept)
    }
}

/// Their side: what they offer, as the server last said, and whether
/// they've accepted.
fn theirs(ui: &mut egui::Ui, icons: &mut ItemIcons, open: &OpenTrade) {
    side_title(
        ui,
        &format!("{}'s offer", open.partner_name),
        open.they_accepted,
    );
    ui.add_space(4.0);
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(WELL_HEIGHT);
        theme::list_spacing(ui);
        if open.theirs.items.is_empty() {
            ui.add_space(22.0);
            ui.vertical_centered(|ui| ui.label(theme::dim("No items yet").size(size::BODY)));
        }
        for item in &open.theirs.items {
            their_row(ui, icons, item);
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(theme::dim("Strands").size(size::BODY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let ink = if open.theirs.strands > 0 { GOLD } else { DIM };
            ui.label(
                RichText::new("S")
                    .font(theme::display(size::LARGE))
                    .color(ink),
            );
            ui.label(
                RichText::new(grouped(open.theirs.strands))
                    .font(theme::strong(size::LARGE))
                    .color(ink),
            );
        });
    });
}

fn their_row(ui: &mut egui::Ui, icons: &mut ItemIcons, item: &TradeItemInfo) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ROW_ICON, ROW_ICON), egui::Sense::hover());
        icons.paint(ui, rect, &item.item_type);
        ui.label(theme::text(item.item_name.clone()).size(size::BODY));
        ui.label(
            RichText::new(format!("×{}", grouped(item.quantity as i64)))
                .font(theme::strong(size::BODY))
                .color(TEXT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            theme::quality_badge(ui, item.quality);
        });
    });
}

/// A side's name, and whether it has accepted: a green tick, or a quiet
/// "not yet".
fn side_title(ui: &mut egui::Ui, name: &str, accepted: bool) {
    ui.horizontal(|ui| {
        ui.label(theme::label(name));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if accepted {
                ui.label(
                    RichText::new("✓ Accepted")
                        .font(theme::strong(size::SMALL))
                        .color(ENOUGH),
                );
            } else {
                ui.label(theme::dim("Not yet").size(size::SMALL));
            }
        });
    });
}

/// The line under both sides: where the deal stands.
fn status(view: &View, open: &OpenTrade) -> (String, egui::Color32) {
    let them = &open.partner_name;
    if view.draft.problem(view.strands).is_some() {
        return (
            "Offer more Strands than you hold, and it can't be sent".into(),
            SHORT,
        );
    }
    if view.draft.unsent() || view.waiting == Some(TradeAction::Offer) {
        return ("Sending your offer…".into(), DIM);
    }
    if let Some((theirs, age)) = view.offer_changed {
        if age < CHANGED_FOR && !open.i_accepted && !open.they_accepted {
            let who = if theirs {
                format!("{them} changed their")
            } else {
                "You changed your".into()
            };
            return (format!("{who} offer, so both accepts were reset"), GOLD);
        }
    }
    match (open.i_accepted, open.they_accepted) {
        (true, true) => ("Both accepted. Trading…".into(), ENOUGH),
        (true, false) => (format!("Waiting for {them} to accept"), DIM),
        (false, true) => (format!("{them} has accepted. Your turn"), ENOUGH),
        (false, false) => ("Accept when the deal looks right".into(), DIM),
    }
}

/// Someone asked you.
fn incoming(ui: &mut egui::Ui, name: &str, view: &View) -> Option<Action> {
    ui.set_width(CARD);
    let mut action = None;
    ui.vertical_centered(|ui| {
        ui.horizontal(|ui| {
            ui.label(
                theme::text(name)
                    .font(theme::strong(size::LARGE))
                    .color(TRADE),
            );
            ui.label(theme::text("wants to trade with you").size(size::BODY));
        });
    });
    ui.add_space(8.0);
    let busy = view.waiting == Some(TradeAction::AcceptRequest);
    ui.columns(2, |cols| {
        let accept = theme::accent_button(
            &mut cols[0],
            if busy { "Opening…" } else { "Accept" },
            ENOUGH,
            !busy,
        );
        if accept.clicked() && !busy {
            action = Some(Action::Answer(true));
        }
        if cols[1]
            .vertical_centered_justified(|ui| {
                ui.add_sized(
                    [ui.available_width(), 38.0],
                    egui::Button::new(RichText::new("Decline").color(TEXT)),
                )
            })
            .inner
            .clicked()
        {
            action = Some(Action::Answer(false));
        }
    });
    ui.add_space(2.0);
    ui.vertical_centered(|ui| ui.label(theme::dim("T accepts").size(size::TINY)));
    action
}

/// You asked them.
fn asking(ui: &mut egui::Ui, name: &str) -> Option<Action> {
    ui.set_width(CARD);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(size::SPINNER).color(TRADE));
        ui.vertical(|ui| {
            ui.label(theme::text(format!("Asking {name} to trade")).size(size::BODY));
            ui.label(theme::dim("They'll see it until they answer").size(size::SMALL));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::link(ui, "Withdraw", size::SMALL).clicked() {
                action = Some(Action::Withdraw);
            }
        });
    });
    action
}

/// You're walking over to ask them.
fn walking(ui: &mut egui::Ui, name: &str) -> Option<Action> {
    ui.set_width(CARD);
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(size::SPINNER).color(TRADE));
        ui.label(theme::text(format!("Walking over to {name}…")).size(size::BODY));
    });
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::state::Offer;
    use std::time::Instant;

    const ROPE: Uuid = Uuid::from_u128(60);

    fn inventory() -> Container {
        let rope = SlotItem::new(ROPE, "rope".into(), "Rope".into(), 72, 3, 0);
        Container::new(Uuid::from_u128(100), "player_inventory", 24, 6, &[rope])
    }

    fn open() -> OpenTrade {
        OpenTrade {
            trade_id: Uuid::from_u128(50),
            partner_id: Uuid::from_u128(2),
            partner_name: "Bram".into(),
            mine: Offer::default(),
            theirs: Offer {
                items: vec![],
                strands: 40,
            },
            i_accepted: false,
            they_accepted: false,
        }
    }

    /// Runs a few frames, clicking at `click` on frame 5. Returns every
    /// action asked for.
    fn run(panel: &mut TradePanel, view: &View, click: Option<egui::Pos2>) -> Vec<Action> {
        let ctx = theme::test_context();
        let mut icons = ItemIcons::load(std::path::Path::new("no/icons/needed"));
        let mut out = Vec::new();
        for frame in 0..10 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 720.0),
                )),
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
                out.extend(panel.show(ctx, &mut icons, view));
            });
        }
        out
    }

    fn view<'a>(trade: &'a Trade, draft: &'a Draft, inv: &'a Container) -> View<'a> {
        View {
            trade,
            draft,
            inventory: Some(inv),
            strands: Some(100),
            waiting: None,
            offer_changed: None,
            walking_to: None,
        }
    }

    #[test]
    fn idle_shows_nothing() {
        let (inv, draft) = (inventory(), Draft::default());
        let mut panel = TradePanel::default();
        assert!(run(&mut panel, &view(&Trade::Idle, &draft, &inv), None).is_empty());
        assert_eq!(panel.rect, None);
    }

    #[test]
    fn a_request_is_accepted_from_its_card() {
        let (inv, draft) = (inventory(), Draft::default());
        let trade = Trade::Incoming {
            from: Uuid::from_u128(2),
            name: "Bram".into(),
        };
        let mut panel = TradePanel::default();
        let v = view(&trade, &draft, &inv);
        run(&mut panel, &v, None);
        let card = panel.rect.expect("drawn");
        assert!((card.center().x - 640.0).abs() < 2.0, "{card:?}");
        // Accept is the left half of the button row.
        let at = egui::pos2(card.left() + card.width() * 0.3, card.bottom() - 40.0);
        assert_eq!(run(&mut panel, &v, Some(at)), vec![Action::Answer(true)]);
    }

    #[test]
    fn accept_waits_for_the_draft_to_be_confirmed() {
        let inv = inventory();
        let trade = Trade::Open(open());
        let mut draft = Draft::default();
        let t0 = Instant::now();
        draft.sync(trade_open(&trade), Some(&inv), t0);
        draft.add(inv.find(ROPE).unwrap(), t0);
        let mut panel = TradePanel::default();
        let v = view(&trade, &draft, &inv);
        run(&mut panel, &v, None);
        let (_, accept) = panel.parts();
        let at = accept.expect("drawn").center();
        assert!(run(&mut panel, &v, Some(at)).is_empty(), "unsent");

        // Sent and echoed: now it can be accepted.
        draft.due(t0 + Duration::from_secs(1), Some(100));
        let mut confirmed = open();
        confirmed.mine = Offer {
            items: vec![TradeItemInfo {
                item_id: ROPE,
                item_type: "rope".into(),
                item_name: "Rope".into(),
                quantity: 3,
                quality: 72,
            }],
            strands: 0,
        };
        let trade = Trade::Open(confirmed);
        let v = view(&trade, &draft, &inv);
        assert_eq!(run(&mut panel, &v, Some(at)), vec![Action::Accept]);
    }

    fn trade_open(trade: &Trade) -> Option<&OpenTrade> {
        match trade {
            Trade::Open(o) => Some(o),
            _ => None,
        }
    }

    #[test]
    fn the_status_says_whose_offer_changed() {
        let inv = inventory();
        let trade = Trade::Open(open());
        let draft = {
            let mut d = Draft::default();
            d.sync(trade_open(&trade), Some(&inv), Instant::now());
            d
        };
        let mut v = view(&trade, &draft, &inv);
        v.offer_changed = Some((true, Duration::from_secs(1)));
        let (line, _) = status(&v, trade_open(&trade).unwrap());
        assert_eq!(line, "Bram changed their offer, so both accepts were reset");
        v.offer_changed = Some((true, Duration::from_secs(9)));
        let (line, _) = status(&v, trade_open(&trade).unwrap());
        assert_eq!(line, "Accept when the deal looks right");
    }
}
