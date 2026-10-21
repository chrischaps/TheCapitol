//! The exchange panel (P7.2): fiber for Strands and back, as React's
//! `ExchangePanel.tsx` has it, in two tabs. X (the `exchange` action) or
//! the header's Exchange chip opens and closes it. It's closed when you
//! arrive, as React's is.
//!
//! - **Deposit** lists the fiber stacks in your inventory. Tick some, or
//!   all, and the panel shows what goes in, the fee, what comes out and
//!   your balance afterwards.
//! - **Withdraw** takes a quantity, typed or picked (10, 50, 100, or the
//!   most you can afford), and shows the cost with its fee, the balance
//!   afterwards, and where the fiber will land.
//!
//! Every number is `capitol_core::exchange`, which uses the server's own
//! rule, so what the panel shows is what the server charges. The receipt
//! after a deposit or withdrawal says so, or says it didn't.
//!
//! The panel sits top left, beside the crafting panel while that's open
//! (where React's sits beside its own panels), and is edged in gold:
//! Strands are money, and gold is the colour of what's yours (plan §1).

use std::collections::HashSet;
use std::time::Duration;

use super::hud::grouped;
use super::icons::ItemIcons;
use super::theme::{self, size, DIM, ENOUGH, GOLD, GOLD_DIM, GOLD_WASH, SHORT, TEXT};
use capitol_core::exchange::{self, DepositQuote, Lands, Receipt, WithdrawQuote, WithdrawRefusal};
use capitol_core::state::Container;
use egui::{Align2, Id, RichText, Stroke};
use uuid::Uuid;

/// The panel's width inside its frame: React's 320 less the padding.
const WIDTH: f32 = 296.0;
/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;
/// How far it slides as it opens, in points.
const RISE: f32 = 12.0;
/// The withdraw tab's quick amounts.
const PICKS: [i64; 3] = [10, 50, 100];
/// A fiber row's icon, in points.
const ROW_ICON: f32 = 22.0;

/// Which side of the exchange the panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Deposit,
    Withdraw,
}

/// What the panel asks for this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Deposit {
        item_ids: Vec<Uuid>,
        quote: DepositQuote,
    },
    Withdraw(WithdrawQuote),
    Close,
}

/// What the panel draws from.
pub struct View<'a> {
    /// Your inventory, once fetched.
    pub inventory: Option<&'a Container>,
    /// Your balance, once known.
    pub strands: Option<i64>,
    /// A deposit or withdrawal is out.
    pub busy: bool,
    /// Why the last one was refused, while it's fresh.
    pub refused: Option<&'a str>,
    /// What the last one did, and how long ago, while it's fresh.
    pub receipt: Option<(Receipt, Duration)>,
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct ExchangePanel {
    pub open: bool,
    pub tab: Tab,
    /// The fiber stacks ticked for a deposit.
    pub selected: HashSet<Uuid>,
    /// The withdraw quantity, as typed.
    pub quantity: String,
    /// The receipt's age last frame, to tell a new receipt from the same one.
    receipt_age: Option<Duration>,
    /// Tick every fiber stack once the inventory holds some (`--exchange
    /// deposit`).
    tick_all: bool,
    /// Each fiber row's place this frame, for the tests to click.
    rows: Vec<(Uuid, egui::Rect)>,
    /// Where the panel was drawn this frame. `None` while it's closed.
    pub rect: Option<egui::Rect>,
}

impl ExchangePanel {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Opens the panel on a tab, for captures (`--exchange`): "deposit"
    /// ticks every fiber stack once the inventory holds some, and
    /// "withdraw:200" types 200.
    pub fn preset(&mut self, spec: &str) {
        let (tab, amount) = spec.split_once(':').unwrap_or((spec, ""));
        self.open = true;
        match tab {
            "withdraw" => {
                self.tab = Tab::Withdraw;
                self.quantity = amount.to_owned();
            }
            _ => {
                self.tab = Tab::Deposit;
                self.tick_all = true;
            }
        }
    }

    /// Draws the panel top left, or just right of `beside` (the crafting
    /// panel) while that's open.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        icons: &mut ItemIcons,
        view: &View,
        beside: Option<egui::Rect>,
    ) -> Option<Action> {
        self.settle(view);
        let shown = ctx.animate_bool_with_time(Id::new("capitol_exchange_fade"), self.open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            return None;
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        let top = ctx.available_rect().top();
        let left = beside.map_or(16.0, |r| r.right() + 12.0);
        let area = egui::Area::new(Id::new("capitol_exchange"))
            .anchor(Align2::LEFT_TOP, [left, top + 12.0 - RISE * (1.0 - ease)])
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(GOLD)
                    .show(ui, |ui| self.body(ui, icons, view))
                    .inner
            });
        self.rect = self.open.then_some(area.response.rect);
        area.inner
    }

    /// Keeps the selection to stacks that are still there, and clears what
    /// a new receipt has spent.
    fn settle(&mut self, view: &View) {
        if let Some(inventory) = view.inventory {
            let present: HashSet<Uuid> = exchange::fiber_stacks(inventory)
                .iter()
                .map(|i| i.item_id)
                .collect();
            self.selected.retain(|id| present.contains(id));
            if self.tick_all && !present.is_empty() {
                self.selected = present;
                self.tick_all = false;
            }
        }
        let age = view.receipt.map(|(_, age)| age);
        let fresh = match (age, self.receipt_age) {
            (Some(now), Some(before)) => now < before,
            (Some(_), None) => true,
            _ => false,
        };
        if fresh {
            if let Some((Receipt::Withdrew { .. }, _)) = view.receipt {
                self.quantity.clear();
            }
        }
        self.receipt_age = age;
    }

    fn body(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) -> Option<Action> {
        ui.set_width(WIDTH);
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(theme::heading("Fiber Exchange").color(GOLD));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.add(
                    egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM))
                        .frame(false),
                );
                if close
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Close (X)")
                    .clicked()
                {
                    action = Some(Action::Close);
                }
            });
        });
        ui.add_space(4.0);
        self.tabs(ui);
        ui.add_space(8.0);
        balance(ui, view.strands);
        if let Some(why) = view.refused {
            ui.add_space(6.0);
            theme::error_box(ui, why);
        }
        if let Some((r, age)) = view.receipt {
            ui.add_space(6.0);
            receipt(ui, &r, age);
        }
        ui.add_space(8.0);
        let asked = match self.tab {
            Tab::Deposit => self.deposit(ui, icons, view),
            Tab::Withdraw => self.withdraw(ui, view),
        };
        action.or(asked)
    }

    /// Deposit and Withdraw, the open one gold and underlined.
    fn tabs(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width() / 2.0;
        let (row, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
        ui.painter().hline(
            row.x_range(),
            row.bottom() - 0.5,
            Stroke::new(1.0_f32, GOLD_DIM.gamma_multiply(0.5)),
        );
        for (i, (tab, name)) in [(Tab::Deposit, "Deposit"), (Tab::Withdraw, "Withdraw")]
            .into_iter()
            .enumerate()
        {
            let rect = egui::Rect::from_min_size(
                row.min + egui::vec2(width * i as f32, 0.0),
                egui::vec2(width, row.height()),
            );
            let response = ui
                .interact(
                    rect,
                    Id::new(("capitol_exchange_tab", i)),
                    egui::Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let on = self.tab == tab;
            let painter = ui.painter();
            if response.hovered() && !on {
                painter.rect_filled(rect.shrink2(egui::vec2(2.0, 2.0)), 4.0, theme::WELL);
            }
            painter.text(
                rect.center() - egui::vec2(0.0, 1.0),
                Align2::CENTER_CENTER,
                name,
                theme::strong(size::BODY),
                if on { GOLD } else { DIM },
            );
            if on {
                painter.hline(
                    rect.shrink2(egui::vec2(10.0, 0.0)).x_range(),
                    rect.bottom() - 1.0,
                    Stroke::new(2.0_f32, GOLD),
                );
            }
            if response.clicked() {
                self.tab = tab;
            }
        }
    }

    fn deposit(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) -> Option<Action> {
        ui.label(theme::dim("Fiber for Strands, one for one, less 0.5%.").size(size::SMALL));
        ui.add_space(6.0);
        let Some(inventory) = view.inventory else {
            ui.vertical_centered(|ui| ui.label(theme::dim("Loading your inventory…")));
            return None;
        };
        let stacks = exchange::fiber_stacks(inventory);
        if stacks.is_empty() {
            theme::well().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    ui.label(theme::dim("No fiber in your inventory"));
                    ui.label(theme::dim("Gather it from the wild grass.").size(size::SMALL));
                });
                ui.add_space(10.0);
            });
        } else {
            theme::well().show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::list_spacing(ui);
                self.rows.clear();
                for item in &stacks {
                    let mut on = self.selected.contains(&item.item_id);
                    let row = fiber_row(ui, icons, item, &mut on);
                    self.rows.push((item.item_id, row.rect));
                    if row.clicked() {
                        if on {
                            self.selected.insert(item.item_id);
                        } else {
                            self.selected.remove(&item.item_id);
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                let all = stacks.iter().all(|i| self.selected.contains(&i.item_id));
                let (text, select) = if all {
                    ("Select none", false)
                } else {
                    ("Select all", true)
                };
                if theme::link(ui, text, size::SMALL).clicked() {
                    self.selected = if select {
                        stacks.iter().map(|i| i.item_id).collect()
                    } else {
                        HashSet::new()
                    };
                }
            });
        }
        let quote = exchange::deposit(inventory, &self.selected, view.strands);
        if let Ok(q) = &quote {
            ui.add_space(6.0);
            summary(
                ui,
                &[
                    ("Fiber to deposit", grouped(q.fiber), TEXT),
                    ("Fee (0.5%)", format!("−{}", grouped(q.fee)), DIM),
                ],
                ("You receive", format!("{} Strands", grouped(q.strands))),
                q.balance_after,
            );
        }
        ui.add_space(10.0);
        let (text, enabled) = match &quote {
            _ if view.busy => ("Depositing…".to_owned(), false),
            Ok(q) => (format!("Deposit {} Fiber", grouped(q.fiber)), true),
            Err(_) => ("Select fiber to deposit".to_owned(), false),
        };
        let clicked = theme::accent_button(ui, &text, GOLD, enabled)
            .on_hover_cursor(cursor(enabled))
            .clicked();
        match quote {
            Ok(quote) if clicked && enabled => Some(Action::Deposit {
                item_ids: exchange::deposit_ids(inventory, &self.selected),
                quote,
            }),
            _ => None,
        }
    }

    fn withdraw(&mut self, ui: &mut egui::Ui, view: &View) -> Option<Action> {
        ui.label(theme::dim("Strands for quality-50 fiber, plus 0.5%.").size(size::SMALL));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::text("Fiber").size(size::BODY));
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.quantity)
                    .hint_text("Amount")
                    .desired_width(60.0)
                    .char_limit(12),
            );
            if field.changed() {
                self.quantity.retain(|c| c.is_ascii_digit() || c == ',');
            }
            ui.add_space(4.0);
            for pick in PICKS {
                if pick_chip(ui, &pick.to_string()).clicked() {
                    self.quantity = pick.to_string();
                }
            }
            if let Some(b) = view.strands {
                let max = exchange::max_withdraw(b);
                if max > 0
                    && pick_chip(ui, "Max")
                        .on_hover_text(format!("{} fiber, all you can afford", grouped(max)))
                        .clicked()
                {
                    self.quantity = max.to_string();
                }
            }
        });
        let quantity = exchange::parse_quantity(&self.quantity).unwrap_or(0);
        let quote = exchange::withdraw(view.inventory, quantity, view.strands);
        ui.add_space(6.0);
        match &quote {
            Ok(q) => {
                summary(
                    ui,
                    &[
                        ("Fiber to receive", grouped(q.fiber), TEXT),
                        ("Fee (0.5%)", format!("+{}", grouped(q.fee)), DIM),
                    ],
                    ("Total cost", format!("{} Strands", grouped(q.strands))),
                    q.balance_after,
                );
                ui.add_space(4.0);
                ui.label(theme::dim(lands(&q.lands)).size(size::SMALL));
            }
            Err(WithdrawRefusal::NoQuantity) => {}
            Err(why) => {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(refusal(why)).color(SHORT).size(size::BODY));
                });
            }
        }
        ui.add_space(10.0);
        let (text, enabled) = match &quote {
            _ if view.busy => ("Withdrawing…".to_owned(), false),
            Ok(q) => (format!("Withdraw {} Fiber", grouped(q.fiber)), true),
            Err(WithdrawRefusal::Short { short, .. }) => {
                (format!("{} Strands short", grouped(*short)), false)
            }
            Err(_) => ("Withdraw Fiber".to_owned(), false),
        };
        let clicked = theme::accent_button(ui, &text, GOLD, enabled)
            .on_hover_cursor(cursor(enabled))
            .clicked();
        match quote {
            Ok(q) if clicked && enabled => Some(Action::Withdraw(q)),
            _ => None,
        }
    }
}

/// The balance band: React's `.exchange-balance`, gold on a gold wash.
fn balance(ui: &mut egui::Ui, strands: Option<i64>) {
    egui::Frame::none()
        .fill(GOLD_WASH)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(theme::dim("Your balance").size(size::BODY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    ui.label(
                        RichText::new("S")
                            .font(theme::display(size::LARGE))
                            .color(GOLD),
                    );
                    ui.label(
                        RichText::new(strands.map_or_else(|| "—".to_owned(), grouped))
                            .font(theme::strong(size::LARGE))
                            .color(GOLD),
                    );
                });
            });
        });
}

/// What the last deposit or withdrawal did, fading over its last second.
fn receipt(ui: &mut egui::Ui, r: &Receipt, age: Duration) {
    let fade = (6.0 - age.as_secs_f32()).clamp(0.0, 1.0);
    let text = match *r {
        Receipt::Deposited { fiber, strands, .. } => {
            format!(
                "Deposited {} fiber: +{} Strands",
                grouped(fiber),
                grouped(strands)
            )
        }
        Receipt::Withdrew { fiber, strands, .. } => {
            format!(
                "Withdrew {} fiber for {} Strands",
                grouped(fiber),
                grouped(strands)
            )
        }
    };
    ui.scope(|ui| {
        ui.multiply_opacity(fade);
        egui::Frame::none()
            .fill(ENOUGH.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0_f32, ENOUGH.gamma_multiply(0.6)))
            .rounding(4.0)
            .inner_margin(egui::Margin::symmetric(12.0, 7.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(text).color(ENOUGH).size(size::BODY));
                if !r.as_quoted() {
                    ui.label(
                        theme::dim("The server's figures differ from the preview.")
                            .size(size::SMALL),
                    );
                }
            });
    });
}

/// One fiber stack, ticked or not: the box, its icon, its name and size,
/// and its quality. The whole row is the click, and its response is
/// returned; `on` is flipped when it's clicked.
fn fiber_row(
    ui: &mut egui::Ui,
    icons: &mut ItemIcons,
    item: &capitol_core::protocol::rest::SlotItem,
    on: &mut bool,
) -> egui::Response {
    // Behind the row, filled once its size is known.
    let back = ui.painter().add(egui::Shape::Noop);
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (tick, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        check_box(ui, tick, *on);
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
    let rect = row.response.rect.expand2(egui::vec2(4.0, 1.0));
    let hit = ui
        .interact(
            rect,
            Id::new(("capitol_exchange_row", item.item_id)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if hit.hovered() || *on {
        let fill = if *on { GOLD_WASH } else { theme::SLOT_HOVER };
        ui.painter().set(
            back,
            egui::Shape::rect_filled(rect, 4.0, fill.gamma_multiply(0.5)),
        );
    }
    if hit.clicked() {
        *on = !*on;
    }
    hit
}

/// A tick box: a dim edge, gold and ticked when `on`.
fn check_box(ui: &egui::Ui, rect: egui::Rect, on: bool) {
    let painter = ui.painter();
    let r = rect.shrink(1.0);
    if on {
        painter.rect_filled(r, 3.0, GOLD);
        let a = r.left_center() + egui::vec2(3.0, 0.0);
        let b = r.center_bottom() + egui::vec2(-1.0, -4.0);
        let c = r.right_top() + egui::vec2(-3.0, 4.0);
        painter.line_segment([a, b], Stroke::new(2.0_f32, theme::BG_DARK));
        painter.line_segment([b, c], Stroke::new(2.0_f32, theme::BG_DARK));
    } else {
        painter.rect_stroke(r, 3.0, Stroke::new(1.0_f32, GOLD_DIM));
    }
}

/// The summary: a few rows, a rule, the total in gold, and the balance
/// afterwards.
fn summary(
    ui: &mut egui::Ui,
    rows: &[(&str, String, egui::Color32)],
    total: (&str, String),
    balance_after: Option<i64>,
) {
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::list_spacing(ui);
        for (label, value, ink) in rows {
            row(ui, label, RichText::new(value).color(*ink));
        }
        ui.add_space(2.0);
        theme::divider(ui);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(theme::text(total.0).font(theme::strong(size::BODY)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(total.1)
                        .font(theme::strong(size::LARGE))
                        .color(GOLD),
                );
            });
        });
        if let Some(after) = balance_after {
            row(
                ui,
                "Balance after",
                theme::dim(format!("{} Strands", grouped(after))),
            );
        }
    });
}

/// One label and value, spread across the space.
fn row(ui: &mut egui::Ui, label: &str, value: RichText) {
    ui.horizontal(|ui| {
        ui.label(theme::dim(label).size(size::BODY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value.size(size::BODY));
        });
    });
}

/// A small quick-pick chip for the withdraw amount.
fn pick_chip(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(size::SMALL).color(GOLD))
            .fill(theme::CHIP)
            .stroke(Stroke::new(1.0_f32, GOLD_DIM.gamma_multiply(0.6)))
            .rounding(4.0)
            .min_size(egui::vec2(0.0, 22.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn cursor(enabled: bool) -> egui::CursorIcon {
    if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::NotAllowed
    }
}

/// Where withdrawn fiber will land, in words.
fn lands(lands: &Lands) -> String {
    match *lands {
        Lands::OnStack {
            slot: Some(slot),
            quality_after: Some(q),
        } => format!(
            "Joins your fiber in slot {}, quality becoming {q}",
            slot + 1
        ),
        Lands::OnStack { .. } => "Joins your fiber".into(),
        Lands::NewSlot(slot) => format!("Lands in empty slot {}", slot + 1),
    }
}

/// Why a withdrawal can't go, in words.
fn refusal(why: &WithdrawRefusal) -> String {
    match why {
        WithdrawRefusal::NoQuantity => String::new(),
        WithdrawRefusal::TooMany => "More than one stack can hold".into(),
        WithdrawRefusal::Short { strands, short } => format!(
            "Costs {} Strands: {} short",
            grouped(*strands),
            grouped(*short)
        ),
        WithdrawRefusal::NoRoom => "No room: free a slot, or hold a fiber stack to join".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::protocol::rest::SlotItem;

    fn fiber(n: u128, quality: i32, quantity: i32, slot: i32) -> SlotItem {
        SlotItem::new(
            Uuid::from_u128(n),
            "fiber".into(),
            "Plant Fiber".into(),
            quality,
            quantity,
            slot,
        )
    }

    fn inventory(items: &[SlotItem]) -> Container {
        Container::new(Uuid::from_u128(100), "player_inventory", 24, 6, items)
    }

    /// Runs a few frames of the panel, clicking `click_at` on frame 5, and
    /// returns the action it asked for.
    fn run(
        panel: &mut ExchangePanel,
        view: &View,
        beside: Option<egui::Rect>,
        click_at: Option<egui::Pos2>,
    ) -> Option<Action> {
        let ctx = theme::test_context();
        let mut icons = ItemIcons::load(std::path::Path::new("no/icons/needed"));
        let mut out = None;
        for frame in 0..8 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 720.0),
                )),
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
                theme::install(ctx);
                if let Some(a) = panel.show(ctx, &mut icons, view, beside) {
                    out = Some(a);
                }
            });
        }
        out
    }

    fn view<'a>(inventory: &'a Container, strands: i64) -> View<'a> {
        View {
            inventory: Some(inventory),
            strands: Some(strands),
            busy: false,
            refused: None,
            receipt: None,
        }
    }

    /// The button is the panel's last widget, just above its margin.
    fn button(panel: &ExchangePanel) -> egui::Pos2 {
        let rect = panel.rect.expect("drawn");
        egui::pos2(rect.center().x, rect.bottom() - 30.0)
    }

    #[test]
    fn ticking_a_row_and_depositing_asks_for_that_stack() {
        let inv = inventory(&[fiber(1, 60, 150, 0), fiber(2, 40, 300, 1)]);
        let view = view(&inv, 10);
        let mut panel = ExchangePanel {
            open: true,
            ..Default::default()
        };
        assert_eq!(run(&mut panel, &view, None, None), None);
        let rect = panel.rect.unwrap();
        assert!((rect.left() - 16.0).abs() < 1.0, "{rect:?}");
        let (id, row) = panel.rows[1];
        assert_eq!(id, Uuid::from_u128(2));
        run(&mut panel, &view, None, Some(row.center()));
        assert_eq!(panel.selected, HashSet::from([id]));
        let action = {
            let at = button(&panel);
            run(&mut panel, &view, None, Some(at))
        };
        assert_eq!(
            action,
            Some(Action::Deposit {
                item_ids: vec![id],
                quote: DepositQuote {
                    fiber: 300,
                    fee: 1,
                    strands: 299,
                    balance_after: Some(309),
                },
            })
        );
    }

    #[test]
    fn withdrawing_asks_for_the_typed_amount_unless_youre_short() {
        let inv = inventory(&[]);
        let mut panel = ExchangePanel {
            open: true,
            tab: Tab::Withdraw,
            quantity: "200".into(),
            ..Default::default()
        };
        let rich = view(&inv, 500);
        run(&mut panel, &rich, None, None);
        let action = {
            let at = button(&panel);
            run(&mut panel, &rich, None, Some(at))
        };
        let Some(Action::Withdraw(q)) = action else {
            panic!("{action:?}");
        };
        assert_eq!((q.fiber, q.fee, q.strands), (200, 2, 202));
        assert_eq!(q.balance_after, Some(298));
        assert_eq!(q.lands, Lands::NewSlot(0));

        let poor = view(&inv, 201);
        run(&mut panel, &poor, None, None);
        assert_eq!(
            {
                let at = button(&panel);
                run(&mut panel, &poor, None, Some(at))
            },
            None
        );
    }

    #[test]
    fn it_sits_beside_the_crafting_panel_and_clears_after_a_withdrawal() {
        let inv = inventory(&[]);
        let crafting = egui::Rect::from_min_size(egui::pos2(16.0, 60.0), egui::vec2(300.0, 400.0));
        let mut panel = ExchangePanel {
            open: true,
            tab: Tab::Withdraw,
            quantity: "10".into(),
            ..Default::default()
        };
        run(&mut panel, &view(&inv, 50), Some(crafting), None);
        let rect = panel.rect.unwrap();
        assert!((rect.left() - 328.0).abs() < 1.0, "{rect:?}");

        let receipt = Receipt::Withdrew {
            fiber: 10,
            fee: 1,
            strands: 11,
            as_quoted: true,
        };
        let done = View {
            receipt: Some((receipt, Duration::from_millis(10))),
            ..view(&inv, 39)
        };
        run(&mut panel, &done, None, None);
        assert_eq!(panel.quantity, "");
    }

    #[test]
    fn closed_draws_nothing() {
        let inv = inventory(&[]);
        let mut panel = ExchangePanel::default();
        assert_eq!(run(&mut panel, &view(&inv, 0), None, None), None);
        assert_eq!(panel.rect, None);
    }
}
