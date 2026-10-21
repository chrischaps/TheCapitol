//! The market panel (E3.5): one book for the whole Capitol, in three
//! columns over your orders. M (the `market` action) or the header's Market
//! chip opens and closes it.
//!
//! - **Items** lists everything with activity (`/market/summary`): its icon,
//!   last price, the change on the day and the day's volume, busiest first.
//!   Under them, the items nobody trades yet, so you can open their book.
//!   A search field narrows both.
//! - **The ladder** is the selected item's book as the server keeps it
//!   (E3.3's `market_depth`): asks above, the cheapest nearest the spread,
//!   bids below, the best nearest. Each level is coloured by its grade and
//!   backed by a bar as deep as its quantity. Units keep their own quality
//!   (D2), so one lot of blended units shows at several grades. A buy on
//!   **every unit** wears its grade filled in; one on **average**, outlined.
//!   Click a level to take it: an ask fills the form as a buy at its price,
//!   a bid as a sell.
//! - **The form**: side, quantity, price, and on a buy the lowest grade and
//!   how it reads. The preview is `capitol_core::market::quote`: the fee is
//!   `rules::market::listing_fee`, the one the server charges, and "fills
//!   now" walks the visible book less your own orders, as the matcher would.
//! - **Your orders**, with a cancel on each, and the market box: what you
//!   bought waits there, and Collect brings it to your inventory.
//! - **Book or Chart** (E3.6), beside the item's name: the chart takes the
//!   ladder's and the form's room for the price history, its volume, your
//!   fills in gold and the latest trades (`market_chart`). A trade clicked
//!   there goes back to the book with its price in the form.
//!
//! - **The bazaar** (E3.11), the third tab: the directory of the bazaar's
//!   stalls across the avenue (`GET /bazaar`), from anywhere: each shop's
//!   name, its keeper and the kinds of goods it has, and which stalls are
//!   to let. No prices, quantities or grades: those are seen at the stall
//!   (D6). The search finds shops by name, keeper or goods. A click on a
//!   row walks you to the stall and opens its shop (`ui::shop`).
//!
//! It's edged in gold, as the exchange is: Strands are money. Fills arrive
//! as toasts (`market_note`) whether the panel is open or not.

use capitol_core::catalog::Catalog;
use capitol_core::market::{self, Book, ChartKey, Draft, FormRefusal, History, OrderQuote};
use capitol_core::protocol::rest::{
    BazaarDirectory, DepthLevel, MarketHistoryResponse, MarketItemSummary, MarketOrder, MarketTrade, MyFill, PlaceOrder,
    SlotItem,
};
use capitol_core::protocol::rules::market::{GradeRule, OrderSide};
use capitol_core::state::Container;
use egui::{Align, Align2, Color32, Id, Layout, RichText, Sense, Stroke};
use uuid::Uuid;

use super::hud::grouped;
use super::icons::ItemIcons;
use super::market_chart::{self as chart, ChartState};
use super::theme::{self, size, DIM, ENOUGH, GOLD, GOLD_DIM, GOLD_WASH, SHORT, TEXT};

/// The columns' widths inside the frame.
const ITEMS_WIDTH: f32 = 200.0;
const LADDER_WIDTH: f32 = 252.0;
const FORM_WIDTH: f32 = 252.0;
const GUTTER: f32 = 14.0;
/// How tall the item list and the ladder stand.
const COLUMN_HEIGHT: f32 = 300.0;
/// Levels shown on each side of the spread.
const LEVELS: usize = 5;
const LEVEL_HEIGHT: f32 = 20.0;
/// Where a level's depth bar may reach to, from the row's left: just past
/// its grade.
const BAR_FROM: f32 = 108.0;
/// And where it stops short of the row's right, clear of the order count.
const BAR_SHORT: f32 = 30.0;
/// How long the panel takes to open or close, and how far it slides.
const FADE: f32 = 0.18;
const RISE: f32 = 12.0;
/// The grades a buy can ask for, best first.
const GRADES: [&str; 5] = ["A", "B", "C", "D", "F"];

/// What the panel asks for this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Place(PlaceOrder),
    Cancel(Uuid),
    /// Bring the market box to the inventory
    Collect,
    /// Walk to a bazaar stall and open its shop (E3.11)
    VisitStall(String),
    Close,
}

/// What the panel draws from.
pub struct View<'a> {
    pub catalog: &'a Catalog,
    /// Every item with activity, once fetched
    pub summary: Option<&'a [MarketItemSummary]>,
    /// The selected item's book, once its whole book has arrived
    pub book: Option<&'a Book>,
    /// The chart's history, once it has arrived. It's shown only while its
    /// key is the panel's [`MarketPanel::charting`].
    pub history: Option<&'a History>,
    /// Your open orders, newest first
    pub orders: Vec<&'a MarketOrder>,
    pub inventory: Option<&'a Container>,
    pub market_box: Option<&'a Container>,
    pub strands: Option<i64>,
    /// A place, cancel or collect is out
    pub busy: bool,
    /// Why the last one was refused, while it's fresh
    pub refused: Option<&'a str>,
    /// How far you are from the Exchange hall, when that's further than
    /// `HALL_RANGE`: placing and collecting happen only there (D6, E3.4)
    pub away_from_hall: Option<f64>,
    /// The bazaar's directory, once fetched (E3.11)
    pub bazaar: Option<&'a BazaarDirectory>,
    /// Your stall, if you keep one, and the one you're walking to
    pub my_stall: Option<&'a str>,
    pub walking_to: Option<&'a str>,
}

/// The order form, as typed.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub side: OrderSide,
    pub quantity: String,
    pub price: String,
    pub min_grade: &'static str,
    pub grade_rule: GradeRule,
    /// A sell's stack; the first of the item's without one
    pub stack: Option<Uuid>,
}

impl Default for Form {
    fn default() -> Self {
        Form {
            side: OrderSide::Buy,
            quantity: String::new(),
            price: String::new(),
            min_grade: "F",
            grade_rule: GradeRule::Average,
            stack: None,
        }
    }
}

/// What the left column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Items,
    Orders,
    /// The bazaar's directory (E3.11)
    Bazaar,
}

/// What the right of the panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    /// The ladder and the order form
    #[default]
    Book,
    /// The price history and the tape (E3.6)
    Chart,
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct MarketPanel {
    pub open: bool,
    /// The item whose book is shown
    pub selected: Option<String>,
    pub tab: Tab,
    pub pane: Pane,
    /// The chart's bucket, grade and style, kept across items
    pub chart: chart::Settings,
    chart_state: ChartState,
    pub form: Form,
    tab_rects: Vec<egui::Rect>,
    /// The item list's search
    pub search: String,
    /// Where things were drawn this frame, for the tests to click
    item_rows: Vec<(String, egui::Rect)>,
    level_rows: Vec<(DepthLevel, egui::Rect)>,
    cancels: Vec<(Uuid, egui::Rect)>,
    stall_rows: Vec<(String, egui::Rect)>,
    place: Option<egui::Rect>,
    /// Where the panel was drawn this frame. `None` while it's closed.
    pub rect: Option<egui::Rect>,
    /// Open last frame: one just opened comes to the front.
    was_open: bool,
}

impl MarketPanel {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// The item to watch: the selected one, while the panel is open.
    pub fn watching(&self) -> Option<&str> {
        self.open.then_some(self.selected.as_deref()).flatten()
    }

    /// The chart to fetch: the selected item's, while the chart is shown.
    pub fn charting(&self) -> Option<ChartKey> {
        let item = self.watching().filter(|_| self.pane == Pane::Chart)?;
        Some(ChartKey {
            item_type: item.to_owned(),
            bucket: self.chart.bucket,
            grade: self.chart.grade.map(str::to_owned),
        })
    }

    /// Show `item_type`'s book, keeping the form's numbers.
    pub fn select(&mut self, item_type: &str) {
        if self.selected.as_deref() != Some(item_type) {
            self.selected = Some(item_type.to_owned());
            self.form.stack = None;
        }
    }

    /// Opens the panel on `item_type` with the form filled in, for captures
    /// (`--market-sample`, `--market`): "plank" opens its book, and
    /// "plank:buy:10@41:B" fills a buy of 10 at 41, B or better. Anywhere
    /// after the item, "chart" shows the chart, and "1d", "line" or
    /// "grade-B" (or "grade-all") set it up; "bazaar" opens the bazaar's
    /// directory (E3.11).
    pub fn preset(&mut self, spec: &str) {
        self.open = true;
        let mut kept = Vec::new();
        for part in spec.split(':') {
            match part {
                "chart" => self.pane = Pane::Chart,
                "bazaar" => self.tab = Tab::Bazaar,
                "1h" => self.chart.bucket = market::Bucket::Hour,
                "1d" => self.chart.bucket = market::Bucket::Day,
                "line" => self.chart.style = chart::Style::Line,
                "candles" => self.chart.style = chart::Style::Candles,
                _ => match part.strip_prefix("grade-") {
                    // "grade-all" finds none, so it's every grade
                    Some(g) => self.chart.grade = GRADES.iter().find(|x| **x == g).copied(),
                    None => kept.push(part),
                },
            }
        }
        let mut parts = kept.into_iter();
        if let Some(item) = parts.next().filter(|s| !s.is_empty()) {
            self.select(item);
        }
        if let Some(side) = parts.next() {
            self.form.side = if side == "sell" {
                OrderSide::Sell
            } else {
                OrderSide::Buy
            };
        }
        if let Some((quantity, price)) = parts.next().and_then(|s| s.split_once('@')) {
            self.form.quantity = quantity.to_owned();
            self.form.price = price.to_owned();
        }
        if let Some(grade) = parts.next().and_then(|g| GRADES.iter().find(|x| **x == g)) {
            self.form.min_grade = grade;
        }
    }

    /// Draws the panel centred under the header.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        icons: &mut ItemIcons,
        view: &View,
    ) -> Option<Action> {
        let shown = ctx.animate_bool_with_time(Id::new("capitol_market_fade"), self.open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            return None;
        }
        if self.selected.is_none() {
            self.selected = first_item(view);
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        let top = ctx.available_rect().top();
        // Opened, it comes in front of the panels already up (they come
        // back in front when clicked, as any area does)
        if self.open && !self.was_open {
            ctx.move_to_top(egui::LayerId::new(
                egui::Order::Middle,
                Id::new("capitol_market"),
            ));
        }
        self.was_open = self.open;
        let area = egui::Area::new(Id::new("capitol_market"))
            .anchor(Align2::CENTER_TOP, [0.0, top + 12.0 - RISE * (1.0 - ease)])
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

    fn body(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(theme::heading("The Market").color(GOLD));
            ui.label(theme::dim("one book for the whole Capitol").size(size::SMALL));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let close = ui.add(
                    egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM))
                        .frame(false),
                );
                if close
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Close (M)")
                    .clicked()
                {
                    action = Some(Action::Close);
                }
            });
        });
        ui.add_space(4.0);
        if self.tab == Tab::Bazaar {
            self.tabs(ui, view.orders.len());
            ui.add_space(6.0);
            if let Some(asked) = self.bazaar(ui, icons, view) {
                action = Some(asked);
            }
            ui.add_space(6.0);
            if let Some(collect) = market_box(ui, view) {
                action = Some(collect);
            }
            return action;
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GUTTER;
            ui.vertical(|ui| {
                ui.set_width(ITEMS_WIDTH);
                self.tabs(ui, view.orders.len());
                ui.add_space(6.0);
                match self.tab {
                    Tab::Items => self.items(ui, icons, view),
                    Tab::Orders => {
                        if let Some(asked) = self.orders(ui, icons, view) {
                            action = Some(asked);
                        }
                    }
                    Tab::Bazaar => {}
                }
            });
            match self.pane {
                Pane::Book => {
                    ui.vertical(|ui| {
                        ui.set_width(LADDER_WIDTH);
                        self.ladder(ui, icons, view);
                    });
                    ui.vertical(|ui| {
                        ui.set_width(FORM_WIDTH);
                        if let Some(placed) = self.form_column(ui, view) {
                            action = Some(placed);
                        }
                    });
                }
                Pane::Chart => {
                    ui.vertical(|ui| {
                        ui.set_width(LADDER_WIDTH + GUTTER + FORM_WIDTH);
                        self.chart_pane(ui, icons, view);
                    });
                }
            }
        });
        ui.add_space(6.0);
        if let Some(collect) = market_box(ui, view) {
            action = Some(collect);
        }
        action
    }

    /// Items, Orders and Bazaar, the open one gold and underlined.
    fn tabs(&mut self, ui: &mut egui::Ui, orders: usize) {
        let (row, _) = ui.allocate_exact_size(egui::vec2(ITEMS_WIDTH, 26.0), Sense::hover());
        ui.painter().hline(
            row.x_range(),
            row.bottom() - 0.5,
            Stroke::new(1.0_f32, GOLD_DIM.gamma_multiply(0.5)),
        );
        let width = row.width() / 3.0;
        let mine = format!("Orders · {orders}");
        for (i, (tab, name)) in [(Tab::Items, "Items"), (Tab::Orders, mine.as_str()), (Tab::Bazaar, "Bazaar")]
            .into_iter()
            .enumerate()
        {
            let rect = egui::Rect::from_min_size(
                row.min + egui::vec2(width * i as f32, 0.0),
                egui::vec2(width, row.height()),
            );
            let response = ui
                .interact(rect, Id::new(("capitol_market_tab", i)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            self.tab_rects.truncate(i);
            self.tab_rects.push(rect);
            let on = self.tab == tab;
            let painter = ui.painter();
            if response.hovered() && !on {
                painter.rect_filled(rect.shrink2(egui::vec2(2.0, 2.0)), 4.0, theme::WELL);
            }
            painter.text(
                rect.center() - egui::vec2(0.0, 1.0),
                Align2::CENTER_CENTER,
                name,
                theme::strong(size::SMALL),
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

    // -- The bazaar (E3.11) ---------------------------------------------------

    /// The bazaar's directory across the panel: each kept stall's shop, its
    /// keeper and its kinds of goods, then the stalls to let. A click walks
    /// you there.
    fn bazaar(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) -> Option<Action> {
        let width = ITEMS_WIDTH + LADDER_WIDTH + FORM_WIDTH + 2.0 * GUTTER;
        let mut action = None;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Find a shop, a keeper or goods…")
                    .desired_width(ITEMS_WIDTH + 60.0)
                    .char_limit(24),
            );
            ui.label(
                theme::dim("Across the avenue from the hall. Prices are seen at the stalls.")
                    .size(size::SMALL),
            );
        });
        ui.add_space(6.0);
        let needle = self.search.trim().to_lowercase();
        self.stall_rows.clear();
        theme::well().show(ui, |ui| {
            ui.set_width(width - 16.0);
            egui::ScrollArea::vertical()
                .id_salt("capitol_market_bazaar")
                .max_height(COLUMN_HEIGHT - 32.0)
                .min_scrolled_height(COLUMN_HEIGHT - 32.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let Some(directory) = view.bazaar else {
                        ui.label(theme::dim("Asking the bazaar…").size(size::SMALL));
                        return;
                    };
                    let matches = |stall: &capitol_core::protocol::rest::BazaarStall| {
                        let Some(shop) = &stall.shop else { return needle.is_empty() };
                        needle.is_empty()
                            || shop.name.to_lowercase().contains(&needle)
                            || shop.owner_name.to_lowercase().contains(&needle)
                            || shop
                                .item_types
                                .iter()
                                .any(|t| view.catalog.item_name(t).to_lowercase().contains(&needle))
                    };
                    let kept: Vec<_> = directory.stalls.iter().filter(|s| s.shop.is_some() && matches(s)).collect();
                    if kept.is_empty() {
                        let line = if needle.is_empty() { "No one keeps a stall yet." } else { "No shop has that." };
                        ui.label(theme::dim(line).size(size::SMALL));
                    }
                    for stall in kept {
                        if self.stall_row(ui, icons, view, stall) {
                            action = Some(Action::VisitStall(stall.stall_id.clone()));
                        }
                    }
                    let free: Vec<&str> = directory
                        .stalls
                        .iter()
                        .filter(|s| s.shop.is_none() && needle.is_empty())
                        .map(|s| s.stall_id.as_str())
                        .collect();
                    if !free.is_empty() {
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(theme::label("To let"));
                            ui.label(
                                theme::dim(format!(
                                    "{} Strands a day ·",
                                    capitol_core::protocol::rules::bazaar::RENT_PER_DAY
                                ))
                                .size(size::SMALL),
                            );
                            for id in free {
                                let n = super::shop::stall_number(id);
                                if chip(ui, &format!("Stall {n}")).on_hover_text("Walk there to rent it").clicked() {
                                    action = Some(Action::VisitStall(id.to_owned()));
                                }
                            }
                        });
                    }
                });
        });
        action
    }

    /// One kept stall: its number, the shop's name, its keeper, and its
    /// kinds of goods as icons. True if it was clicked.
    fn stall_row(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
        stall: &capitol_core::protocol::rest::BazaarStall,
    ) -> bool {
        let Some(shop) = &stall.shop else { return false };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(
            if view.walking_to == Some(stall.stall_id.as_str()) { "On your way" } else { "Walk there and browse" },
        );
        self.stall_rows.push((stall.stall_id.clone(), rect));
        let painter = ui.painter();
        let mine = view.my_stall == Some(stall.stall_id.as_str());
        if response.hovered() || mine {
            painter.rect_filled(rect, 4.0, if mine { GOLD_WASH } else { theme::WELL });
        }
        let y = rect.center().y;
        painter.text(
            egui::pos2(rect.left() + 6.0, y),
            Align2::LEFT_CENTER,
            super::shop::stall_number(&stall.stall_id),
            theme::strong(size::SMALL),
            DIM,
        );
        painter.text(
            egui::pos2(rect.left() + 32.0, y),
            Align2::LEFT_CENTER,
            &shop.name,
            theme::strong(size::BODY),
            if mine { GOLD } else { TEXT },
        );
        let keeper = match (mine, shop.owner_citizen) {
            (true, _) => "yours".to_owned(),
            (false, true) => format!("◆ {}", shop.owner_name),
            (false, false) => shop.owner_name.clone(),
        };
        painter.text(
            egui::pos2(rect.left() + 250.0, y),
            Align2::LEFT_CENTER,
            keeper,
            theme::body(size::SMALL),
            DIM,
        );
        let mut x = rect.left() + 380.0;
        for t in shop.item_types.iter().take(12) {
            let r = egui::Rect::from_center_size(egui::pos2(x + 10.0, y), egui::vec2(20.0, 20.0));
            icons.paint_on(painter, r, t, 1.0);
            x += 23.0;
        }
        if shop.item_types.is_empty() {
            painter.text(egui::pos2(x, y), Align2::LEFT_CENTER, "nothing out yet", theme::body(size::SMALL), DIM);
        }
        let arrow = if view.walking_to == Some(stall.stall_id.as_str()) { "walking…" } else { "›" };
        painter.text(
            egui::pos2(rect.right() - 8.0, y),
            Align2::RIGHT_CENTER,
            arrow,
            theme::strong(size::BODY),
            if response.hovered() { GOLD } else { DIM },
        );
        response.clicked()
    }

    // -- Items ---------------------------------------------------------------

    fn items(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) {
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Find an item…")
                .desired_width(ITEMS_WIDTH)
                .char_limit(24),
        );
        ui.add_space(6.0);
        let needle = self.search.trim().to_lowercase();
        let matches = |id: &str| {
            needle.is_empty() || view.catalog.item_name(id).to_lowercase().contains(&needle)
        };
        let mut active: Vec<&MarketItemSummary> = view
            .summary
            .unwrap_or_default()
            .iter()
            .filter(|s| matches(&s.item_type))
            .collect();
        active.sort_by(|a, b| {
            b.volume_24h
                .cmp(&a.volume_24h)
                .then_with(|| a.item_type.cmp(&b.item_type))
        });
        let quiet: Vec<&str> = view
            .catalog
            .item_types()
            .iter()
            .map(|t| t.id.as_str())
            .filter(|id| {
                matches(id)
                    && !view
                        .summary
                        .unwrap_or_default()
                        .iter()
                        .any(|s| s.item_type == *id)
            })
            .collect();
        self.item_rows.clear();
        theme::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("capitol_market_items")
                .max_height(COLUMN_HEIGHT - 66.0)
                .min_scrolled_height(COLUMN_HEIGHT - 66.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if view.summary.is_none() {
                        ui.label(theme::dim("Asking the market…").size(size::SMALL));
                    } else if active.is_empty() && needle.is_empty() {
                        ui.label(theme::dim("Nothing traded this week").size(size::SMALL));
                    }
                    for s in &active {
                        if self.item_row(ui, icons, view.catalog, &s.item_type, Some(s)) {
                            self.select(&s.item_type);
                        }
                    }
                    if !quiet.is_empty() {
                        ui.add_space(6.0);
                        ui.label(theme::label("Not trading yet"));
                        ui.add_space(2.0);
                        for id in quiet {
                            if self.item_row(ui, icons, view.catalog, id, None) {
                                self.select(id);
                            }
                        }
                    }
                });
        });
    }

    /// One item: icon and name, and with activity, its last price, the
    /// change on the day and the volume. True if it was clicked.
    fn item_row(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        catalog: &Catalog,
        item_type: &str,
        summary: Option<&MarketItemSummary>,
    ) -> bool {
        let on = self.selected.as_deref() == Some(item_type);
        let height = if summary.is_some() { 38.0 } else { 24.0 };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        self.item_rows.push((item_type.to_owned(), rect));
        let painter = ui.painter();
        if on {
            painter.rect_filled(rect, 4.0, GOLD_WASH.gamma_multiply(1.6));
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
                1.0,
                GOLD,
            );
        } else if response.hovered() {
            painter.rect_filled(rect, 4.0, theme::SLOT_HOVER.gamma_multiply(0.6));
        }
        let icon = if summary.is_some() { 22.0 } else { 18.0 };
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 8.0 + icon / 2.0, rect.center().y),
            egui::vec2(icon, icon),
        );
        icons.paint(ui, icon_rect, item_type);
        let painter = ui.painter();
        let x = icon_rect.right() + 7.0;
        let name = catalog.item_name(item_type);
        let ink = if summary.is_some() || on { TEXT } else { DIM };
        match summary {
            Some(s) => {
                painter.text(
                    egui::pos2(x, rect.top() + 3.0),
                    Align2::LEFT_TOP,
                    &name,
                    theme::body(size::BODY),
                    ink,
                );
                let (change, colour) = change(s.change_24h);
                painter.text(
                    egui::pos2(x, rect.bottom() - 4.0),
                    Align2::LEFT_BOTTOM,
                    format!("{change}  ·  {} today", grouped(s.volume_24h)),
                    theme::body(size::LABEL),
                    colour,
                );
                painter.text(
                    egui::pos2(rect.right() - 8.0, rect.top() + 4.0),
                    Align2::RIGHT_TOP,
                    s.last_price.map_or_else(|| "—".to_owned(), grouped),
                    theme::strong(size::BODY),
                    GOLD,
                );
            }
            None => {
                painter.text(
                    egui::pos2(x, rect.center().y),
                    Align2::LEFT_CENTER,
                    &name,
                    theme::body(size::SMALL),
                    ink,
                );
            }
        }
        response.clicked()
    }

    // -- The ladder ----------------------------------------------------------

    /// The item's icon, name and last price, with Book and Chart at the
    /// right.
    fn item_head(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View, item: &str) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), Sense::hover());
            icons.paint(ui, rect, item);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(
                    theme::text(view.catalog.item_name(item)).font(theme::strong(size::LARGE)),
                );
                let line = match view
                    .summary
                    .and_then(|s| s.iter().find(|s| s.item_type == item))
                {
                    Some(MarketItemSummary {
                        last_price: Some(p),
                        change_24h,
                        ..
                    }) => {
                        let (c, colour) = change(*change_24h);
                        RichText::new(format!("last {}  {c}", grouped(*p))).color(colour)
                    }
                    _ => theme::dim("no trades this week"),
                };
                ui.label(line.size(size::SMALL));
            });
            // Book and Chart against the right edge, in reading order: a
            // right-to-left layout would turn them round
            const PANES: [(Pane, &str); 2] = [(Pane::Book, "Book"), (Pane::Chart, "Chart")];
            let pad = ui.spacing().button_padding.x * 2.0;
            let width: f32 = PANES
                .iter()
                .map(|(_, name)| {
                    let galley = ui.painter().layout_no_wrap(
                        (*name).to_owned(),
                        egui::FontId::proportional(size::SMALL),
                        TEXT,
                    );
                    (galley.size().x + pad).max(36.0)
                })
                .sum();
            let gap = ui.spacing().item_spacing.x;
            ui.add_space((ui.available_width() - width - gap - 1.0).max(0.0));
            ui.scope(|ui| {
                theme::segmented(ui, &mut self.pane, &PANES);
            });
        });
        ui.add_space(4.0);
    }

    fn ladder(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) {
        self.level_rows.clear();
        let Some(item) = self.selected.clone() else {
            ui.label(theme::dim("Choose an item"));
            return;
        };
        self.item_head(ui, icons, view, &item);
        let Some(book) = view.book else {
            theme::well().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(COLUMN_HEIGHT - 52.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.label(theme::dim("Opening the book…"));
                });
            });
            return;
        };
        let asks: Vec<&DepthLevel> = book.side(OrderSide::Sell).take(LEVELS).collect();
        let bids: Vec<&DepthLevel> = book.side(OrderSide::Buy).take(LEVELS).collect();
        let deepest = asks
            .iter()
            .chain(&bids)
            .map(|l| l.quantity)
            .max()
            .unwrap_or(1)
            .max(1);
        let mine = |l: &DepthLevel| {
            view.orders.iter().any(|o| {
                let at = o.item_type == item && o.side == l.side && o.price == l.price;
                at && match o.side {
                    OrderSide::Buy => {
                        o.min_grade.as_deref() == Some(l.grade.as_str())
                            && o.grade_rule == l.grade_rule
                    }
                    OrderSide::Sell => units_by_grade(&o.units).iter().any(|(g, _)| *g == l.grade),
                }
            })
        };
        theme::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;
            columns_head(ui);
            // Asks stack up from the spread: the cheapest sits lowest
            for _ in asks.len()..LEVELS {
                ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), LEVEL_HEIGHT),
                    Sense::hover(),
                );
            }
            for level in asks.iter().rev() {
                if let Some(clicked) = self.level_row(ui, level, deepest, mine(level)) {
                    self.take(clicked);
                }
            }
            spread(ui, book);
            for level in &bids {
                if let Some(clicked) = self.level_row(ui, level, deepest, mine(level)) {
                    self.take(clicked);
                }
            }
            for _ in bids.len()..LEVELS {
                ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), LEVEL_HEIGHT),
                    Sense::hover(),
                );
            }
        });
    }

    /// One level: its depth bar in its grade's colour, the price in its
    /// side's, the grade, the quantity and the orders. A gold dot marks a
    /// level you have an order at. Returns the level if it was clicked.
    fn level_row(
        &mut self,
        ui: &mut egui::Ui,
        level: &DepthLevel,
        deepest: i64,
        mine: bool,
    ) -> Option<DepthLevel> {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), LEVEL_HEIGHT),
            Sense::click(),
        );
        self.level_rows.push((level.clone(), rect));
        let grade = theme::grade(&level.grade);
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(rect, 3.0, theme::SLOT_HOVER.gamma_multiply(0.7));
        }
        // The bar grows from the right edge, behind the quantity, as far
        // as the grade
        let right = rect.right() - BAR_SHORT;
        let room = right - rect.left() - BAR_FROM;
        let share = (level.quantity as f32 / deepest as f32).clamp(0.03, 1.0);
        let bar = egui::Rect::from_min_max(
            egui::pos2(right - room * share, rect.top() + 2.0),
            egui::pos2(right, rect.bottom() - 2.0),
        );
        painter.rect_filled(bar, 2.0, grade.gamma_multiply(0.16));
        painter.rect_filled(
            egui::Rect::from_min_max(bar.left_top(), egui::pos2(bar.left() + 2.0, bar.bottom())),
            1.0,
            grade.gamma_multiply(0.55),
        );
        let side = match level.side {
            OrderSide::Buy => ENOUGH,
            OrderSide::Sell => SHORT,
        };
        let y = rect.center().y;
        if mine {
            painter.circle_filled(egui::pos2(rect.left() + 5.0, y), 2.5, GOLD);
        }
        painter.text(
            egui::pos2(rect.left() + 62.0, y),
            Align2::RIGHT_CENTER,
            grouped(level.price),
            theme::strong(size::BODY),
            side,
        );
        grade_pill(
            painter,
            egui::pos2(rect.left() + 82.0, y),
            &level.grade,
            level.grade_rule,
        );
        painter.text(
            egui::pos2(rect.left() + 160.0, y),
            Align2::RIGHT_CENTER,
            grouped(level.quantity),
            theme::body(size::BODY),
            TEXT,
        );
        painter.text(
            egui::pos2(rect.right() - 8.0, y),
            Align2::RIGHT_CENTER,
            level.orders.to_string(),
            theme::body(size::LABEL),
            DIM,
        );
        let hover = match (level.side, level.grade_rule) {
            (OrderSide::Sell, _) => format!(
                "{} at {} each, grade {}. Click to buy at this price",
                grouped(level.quantity),
                grouped(level.price),
                level.grade
            ),
            (OrderSide::Buy, rule) => format!(
                "Wanted: {} at {} each, {} {} or better. Click to sell at this price",
                grouped(level.quantity),
                grouped(level.price),
                if rule == Some(GradeRule::EveryUnit) {
                    "every unit"
                } else {
                    "on average"
                },
                level.grade
            ),
        };
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(hover);
        response.clicked().then(|| level.clone())
    }

    // -- The chart (E3.6) ----------------------------------------------------

    /// The price history beside the tape, under the chart's controls.
    fn chart_pane(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) {
        self.chart_state.tape_rows.clear();
        let Some(item) = self.selected.clone() else {
            ui.label(theme::dim("Choose an item"));
            return;
        };
        self.item_head(ui, icons, view, &item);
        chart::controls(ui, &mut self.chart);
        ui.add_space(6.0);
        let key = self.charting();
        let history = view.history.filter(|h| key.as_ref() == Some(&h.key));
        let mut taken = None;
        theme::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let tape_width = 150.0;
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() - tape_width - 10.0);
                    match history {
                        Some(h) if !h.candles.is_empty() => {
                            chart::plots(ui, &mut self.chart_state, h, &self.chart, chrono::Utc::now());
                        }
                        Some(_) => {
                            let grade = self.chart.grade.map(|g| format!(" of grade {g}")).unwrap_or_default();
                            let when = match self.chart.bucket {
                                market::Bucket::Hour => "this week",
                                market::Bucket::Day => "in four months",
                            };
                            chart::empty(ui, &format!("No trades{grade} {when}"), chart::HEIGHT);
                        }
                        None => chart::empty(ui, "Fetching the history…", chart::HEIGHT),
                    }
                });
                ui.vertical(|ui| {
                    ui.set_width(tape_width);
                    if let Some(h) = history {
                        taken = chart::tape(ui, &mut self.chart_state, h, chart::HEIGHT);
                    }
                });
            });
        });
        // A trade clicked: back to the book with its price
        if let Some(trade) = taken {
            self.form.price = trade.price.to_string();
            self.pane = Pane::Book;
        }
    }

    /// A level clicked: take the other side of it at its price.
    fn take(&mut self, level: DepthLevel) {
        self.form.price = level.price.to_string();
        self.form.side = match level.side {
            OrderSide::Sell => OrderSide::Buy,
            OrderSide::Buy => OrderSide::Sell,
        };
        if level.side == OrderSide::Sell && self.form.quantity.is_empty() {
            self.form.quantity = level.quantity.min(1000).to_string();
        }
    }

    // -- The form ------------------------------------------------------------

    fn form_column(&mut self, ui: &mut egui::Ui, view: &View) -> Option<Action> {
        let item = self.selected.clone()?;
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            theme::segmented(
                ui,
                &mut self.form.side,
                &[(OrderSide::Buy, "Buy"), (OrderSide::Sell, "Sell")],
            );
        });
        ui.add_space(6.0);
        let stacks = stacks_of(view, &item);
        if self
            .form
            .stack
            .is_none_or(|id| !stacks.iter().any(|s| s.item_id == id))
        {
            self.form.stack = stacks.first().map(|s| s.item_id);
        }
        let stack = self
            .form
            .stack
            .and_then(|id| stacks.iter().find(|s| s.item_id == id))
            .copied();
        let book = view.book;
        theme::gold_fields(ui);
        field_row(ui, "Quantity", &mut self.form.quantity, |ui, quantity| {
            if self.form.side == OrderSide::Sell {
                if let Some(s) = stack {
                    if chip(ui, "All")
                        .on_hover_text("Every unit in the stack")
                        .clicked()
                    {
                        *quantity = s.quantity.to_string();
                    }
                }
            }
        });
        field_row(ui, "Price each", &mut self.form.price, |ui, price| {
            let best = book.and_then(|b| match self.form.side {
                OrderSide::Buy => b.best_ask().map(|p| ("Ask", p)),
                OrderSide::Sell => b.best_bid().map(|p| ("Bid", p)),
            });
            if let Some((name, p)) = best {
                let hover = format!("The best {} now: fills at once", name.to_lowercase());
                if chip(ui, &format!("{name} {}", grouped(p)))
                    .on_hover_text(hover)
                    .clicked()
                {
                    *price = p.to_string();
                }
            }
        });
        match self.form.side {
            OrderSide::Buy => {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    grade_picker(ui, &mut self.form.min_grade);
                    ui.add_space(4.0);
                    theme::segmented(
                        ui,
                        &mut self.form.grade_rule,
                        &[(GradeRule::Average, "Avg"), (GradeRule::EveryUnit, "Each")],
                    )
                    .on_hover_text(
                        "Avg: a blend whose mean meets the grade will do. Each: every unit must meet it.",
                    );
                });
            }
            OrderSide::Sell => {
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_sized(
                        [64.0, 22.0],
                        egui::Label::new(theme::dim("From").size(size::SMALL)),
                    );
                    if stacks.is_empty() {
                        ui.label(
                            RichText::new("You hold none")
                                .size(size::SMALL)
                                .color(SHORT),
                        );
                    }
                    for s in &stacks {
                        let on = self.form.stack == Some(s.item_id);
                        let text = format!("×{} · {} {}", s.quantity, s.quality, s.quality_grade);
                        let r = ui
                            .add(
                                egui::Button::new(RichText::new(text).size(size::SMALL).color(
                                    if on {
                                        theme::grade(&s.quality_grade)
                                    } else {
                                        DIM
                                    },
                                ))
                                .fill(if on { GOLD_WASH } else { theme::FIELD })
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    if on {
                                        GOLD
                                    } else {
                                        GOLD_DIM.gamma_multiply(0.4)
                                    },
                                ))
                                .rounding(4.0),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if r.clicked() {
                            self.form.stack = Some(s.item_id);
                        }
                    }
                });
            }
        }
        let draft = Draft {
            side: self.form.side,
            item_type: item.clone(),
            price: amount(&self.form.price),
            quantity: amount(&self.form.quantity).min(i32::MAX as i64) as i32,
            min_grade: self.form.min_grade.to_owned(),
            grade_rule: self.form.grade_rule,
        };
        let quote = market::quote(
            &draft,
            stack.map(|s| &s.units),
            book,
            &view.orders,
            view.strands,
        );
        ui.add_space(6.0);
        preview(ui, &quote);
        if let Some(why) = view.refused {
            ui.add_space(4.0);
            theme::error_box(ui, why);
        }
        ui.add_space(8.0);
        let side = match draft.side {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        };
        if let Some(away) = view.away_from_hall {
            ui.add_space(4.0);
            ui.label(
                theme::dim(format!(
                    "Orders are placed at the Exchange hall, on the Capitol avenue. You're {} away.",
                    grouped(away.round() as i64)
                ))
                .size(size::SMALL),
            );
        }
        let (text, enabled) = match &quote {
            _ if view.busy => ("Sending…".to_owned(), false),
            _ if view.away_from_hall.is_some() => ("At the Exchange hall".to_owned(), false),
            Ok(q) => (
                format!(
                    "Place {side}: {} for {}",
                    grouped(draft.quantity as i64),
                    grouped(q.value)
                ),
                true,
            ),
            Err(FormRefusal::Short { short, .. }) => {
                (format!("{} Strands short", grouped(*short)), false)
            }
            Err(_) => (format!("Place {side}"), false),
        };
        let button = theme::accent_button(ui, &text, GOLD, enabled);
        self.place = Some(button.rect);
        let clicked = button.on_hover_cursor(cursor(enabled)).clicked();
        match quote {
            Ok(_) if clicked && enabled => Some(Action::Place(draft.to_place(self.form.stack))),
            _ => None,
        }
    }

    // -- Your orders ---------------------------------------------------------

    fn orders(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) -> Option<Action> {
        let mut action = None;
        self.cancels.clear();
        theme::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("capitol_market_orders")
                .max_height(COLUMN_HEIGHT - 32.0)
                .min_scrolled_height(COLUMN_HEIGHT - 32.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    if view.orders.is_empty() {
                        ui.label(theme::dim("None open.").size(size::SMALL));
                        ui.label(
                            theme::dim("An order rests here until it fills, you cancel it, or seven days pass.")
                                .size(size::SMALL),
                        );
                    }
                    for order in &view.orders {
                        match self.order_row(ui, icons, view, order) {
                            Some(true) if !view.busy => action = Some(Action::Cancel(order.id)),
                            Some(false) => self.select(&order.item_type),
                            _ => {}
                        }
                    }
                });
        });
        action
    }

    /// One of your orders, in two lines: its side, item and cancel, then
    /// what's left, the price and its grades. `Some(true)` if its cancel was
    /// clicked, `Some(false)` if the rest of it was (show its book).
    fn order_row(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
        order: &MarketOrder,
    ) -> Option<bool> {
        let (rect, row) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
        let (side, colour) = match order.side {
            OrderSide::Buy => ("BUY", ENOUGH),
            OrderSide::Sell => ("SELL", SHORT),
        };
        if row.hovered() {
            ui.painter()
                .rect_filled(rect, 4.0, theme::SLOT_HOVER.gamma_multiply(0.6));
        }
        let top = rect.top() + 10.0;
        let tag = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 4.0, top - 7.0),
            egui::vec2(30.0, 14.0),
        );
        icons.paint(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(tag.right() + 12.0, top),
                egui::vec2(16.0, 16.0),
            ),
            &order.item_type,
        );
        let painter = ui.painter();
        painter.rect_filled(tag, 3.0, colour.gamma_multiply(0.18));
        painter.text(
            tag.center(),
            Align2::CENTER_CENTER,
            side,
            theme::strong(size::TINY),
            colour,
        );
        painter.text(
            egui::pos2(tag.right() + 24.0, top),
            Align2::LEFT_CENTER,
            view.catalog.item_name(&order.item_type),
            theme::body(size::SMALL),
            TEXT,
        );
        let bottom = rect.bottom() - 9.0;
        let left = if order.quantity_remaining < order.quantity_total {
            format!(
                "{} of {}",
                grouped(order.quantity_remaining as i64),
                grouped(order.quantity_total as i64)
            )
        } else {
            grouped(order.quantity_total as i64)
        };
        let price = format!("{left} at {}", grouped(order.price));
        let galley = painter.layout_no_wrap(price, theme::strong(size::LABEL), GOLD);
        let w = galley.size().x;
        painter.galley(
            egui::pos2(rect.left() + 6.0, bottom - galley.size().y / 2.0),
            galley,
            GOLD,
        );
        let terms = match (order.side, &order.min_grade) {
            (OrderSide::Buy, Some(g)) if order.grade_rule == Some(GradeRule::EveryUnit) => {
                format!("every unit {g}+")
            }
            (OrderSide::Buy, Some(g)) => format!("{g}+ on average"),
            (OrderSide::Sell, _) => units_by_grade(&order.units)
                .into_iter()
                .map(|(g, n)| format!("{n} {g}"))
                .collect::<Vec<_>>()
                .join(" · "),
            _ => String::new(),
        };
        painter.text(
            egui::pos2(rect.left() + 14.0 + w, bottom),
            Align2::LEFT_CENTER,
            terms,
            theme::body(size::LABEL),
            DIM,
        );
        let cancel_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 12.0, top),
            egui::vec2(20.0, 18.0),
        );
        self.cancels.push((order.id, cancel_rect));
        let cancel = ui
            .interact(
                cancel_rect,
                Id::new(("capitol_market_cancel", order.id)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Cancel: what's left comes back; the fee doesn't");
        let ink = if cancel.hovered() { SHORT } else { DIM };
        ui.painter().text(
            cancel_rect.center(),
            Align2::CENTER_CENTER,
            "×",
            theme::strong(size::HEADING),
            ink,
        );
        if cancel.clicked() {
            Some(true)
        } else if row
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            Some(false)
        } else {
            None
        }
    }
}

/// The market box, along the foot of the panel: what waits there, and
/// Collect. Returns `Collect` if it was clicked.
fn market_box(ui: &mut egui::Ui, view: &View) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        let stacks: Vec<&SlotItem> = view
            .market_box
            .map_or_else(Vec::new, |b| b.items().collect());
        ui.label(theme::label("Market box"));
        if stacks.is_empty() {
            ui.label(theme::dim("empty. What you buy waits here.").size(size::SMALL));
            return;
        }
        let what: Vec<String> = stacks
            .iter()
            .take(4)
            .map(|s| {
                format!(
                    "{} {}",
                    grouped(s.quantity as i64),
                    view.catalog.item_name(&s.item_type)
                )
            })
            .collect();
        let more = if stacks.len() > 4 {
            format!(" and {} more", stacks.len() - 4)
        } else {
            String::new()
        };
        ui.label(
            RichText::new(format!("{}{more}", what.join(", ")))
                .size(size::SMALL)
                .color(GOLD),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let at_hall = view.away_from_hall.is_none();
            let collect = theme::button(ui, "Collect", at_hall).on_hover_text(if at_hall {
                "Bring it all to your inventory, as far as it fits"
            } else {
                "Collect it at the Exchange hall, on the Capitol avenue"
            });
            if collect.clicked() && at_hall && !view.busy {
                action = Some(Action::Collect);
            }
        });
    });
    action
}

/// The first item to show: the busiest, else the first in the catalog.
fn first_item(view: &View) -> Option<String> {
    view.summary
        .and_then(|s| s.iter().max_by_key(|s| s.volume_24h))
        .map(|s| s.item_type.clone())
        .or_else(|| view.catalog.item_types().first().map(|t| t.id.clone()))
}

/// The stacks of `item_type` you can sell from: your inventory's, then your
/// market box's, each in slot order.
fn stacks_of<'a>(view: &View<'a>, item_type: &str) -> Vec<&'a SlotItem> {
    let mut stacks = Vec::new();
    for container in [view.inventory, view.market_box].into_iter().flatten() {
        let mut here: Vec<&SlotItem> = container
            .items()
            .filter(|i| i.item_type == item_type)
            .collect();
        here.sort_by_key(|i| i.slot_index);
        stacks.extend(here);
    }
    stacks
}

/// How many of `units` fall in each grade, best first.
fn units_by_grade(
    units: &capitol_core::protocol::rules::stacking::Units,
) -> Vec<(&'static str, i64)> {
    let mut out: Vec<(&'static str, i64)> = Vec::new();
    for (q, c) in units.pairs().rev() {
        let g = capitol_core::protocol::rules::quality_grade(q);
        match out.last_mut() {
            Some((last, n)) if *last == g => *n += c as i64,
            _ => out.push((g, c as i64)),
        }
    }
    out
}

/// The day's change, with an arrow, and its colour.
fn change(change: Option<i64>) -> (String, Color32) {
    match change {
        Some(c) if c > 0 => (format!("▲ {}", grouped(c)), ENOUGH),
        Some(c) if c < 0 => (format!("▼ {}", grouped(-c)), SHORT),
        Some(_) => ("— 0".to_owned(), DIM),
        None => ("—".to_owned(), DIM),
    }
}

/// The ladder's column heads.
fn columns_head(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::hover());
    let painter = ui.painter();
    let font = theme::body(size::TINY);
    let y = rect.center().y;
    painter.text(
        egui::pos2(rect.left() + 62.0, y),
        Align2::RIGHT_CENTER,
        "PRICE",
        font.clone(),
        DIM,
    );
    painter.text(
        egui::pos2(rect.left() + 82.0, y),
        Align2::LEFT_CENTER,
        "GRADE",
        font.clone(),
        DIM,
    );
    painter.text(
        egui::pos2(rect.left() + 160.0, y),
        Align2::RIGHT_CENTER,
        "QTY",
        font.clone(),
        DIM,
    );
    painter.text(
        egui::pos2(rect.right() - 8.0, y),
        Align2::RIGHT_CENTER,
        "ORD",
        font,
        DIM,
    );
}

/// The line between asks and bids: the spread, or what's missing.
fn spread(ui: &mut egui::Ui, book: &Book) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::hover());
    let painter = ui.painter();
    let y = rect.center().y;
    let text = match (book.best_bid(), book.best_ask()) {
        (Some(b), Some(a)) => format!("spread {}", grouped(a - b)),
        (Some(_), None) => "no one selling".to_owned(),
        (None, Some(_)) => "no one buying".to_owned(),
        (None, None) => "the book is empty".to_owned(),
    };
    let galley = painter.layout_no_wrap(text, theme::body(size::LABEL), GOLD_DIM);
    let w = galley.size().x;
    let gap = w / 2.0 + 8.0;
    let rule = Stroke::new(1.0_f32, GOLD_DIM.gamma_multiply(0.45));
    painter.hline((rect.left() + 4.0)..=(rect.center().x - gap), y, rule);
    painter.hline((rect.center().x + gap)..=(rect.right() - 4.0), y, rule);
    painter.galley(
        egui::pos2(rect.center().x - w / 2.0, y - galley.size().y / 2.0),
        galley,
        GOLD_DIM,
    );
}

/// A grade on the ladder: filled for a buy on every unit, outlined for one
/// on average, plain for a sell's units.
fn grade_pill(
    painter: &egui::Painter,
    left_center: egui::Pos2,
    grade: &str,
    rule: Option<GradeRule>,
) {
    let colour = theme::grade(grade);
    let r = egui::Rect::from_min_size(left_center - egui::vec2(0.0, 7.0), egui::vec2(20.0, 14.0));
    match rule {
        Some(GradeRule::EveryUnit) => {
            painter.rect_filled(r, 3.0, colour.gamma_multiply(0.85));
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                grade,
                theme::strong(size::LABEL),
                theme::BG_DARK,
            );
        }
        Some(GradeRule::Average) => {
            painter.rect_stroke(r, 3.0, Stroke::new(1.0_f32, colour.gamma_multiply(0.8)));
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                grade,
                theme::strong(size::LABEL),
                colour,
            );
        }
        None => {
            painter.rect_filled(r, 3.0, colour.gamma_multiply(0.2));
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                grade,
                theme::strong(size::LABEL),
                colour,
            );
        }
    }
}

/// The lowest grade a buy takes: five pips, each in its grade's colour.
fn grade_picker(ui: &mut egui::Ui, value: &mut &'static str) {
    ui.spacing_mut().item_spacing.x = 3.0;
    for grade in GRADES {
        let on = *value == grade;
        let colour = theme::grade(grade);
        let r = ui
            .add(
                egui::Button::new(
                    RichText::new(grade)
                        .font(theme::strong(size::SMALL))
                        .color(if on { theme::BG_DARK } else { colour }),
                )
                .fill(if on {
                    colour
                } else {
                    colour.gamma_multiply(0.12)
                })
                .stroke(Stroke::new(
                    1.0_f32,
                    colour.gamma_multiply(if on { 1.0 } else { 0.5 }),
                ))
                .rounding(4.0)
                .min_size(egui::vec2(28.0, 22.0)),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(match grade {
                "F" => "At least F: any quality".to_owned(),
                g => format!(
                    "At least {g}: quality {} or better",
                    capitol_core::protocol::rules::market::grade_floor(g).unwrap_or(0)
                ),
            });
        if r.clicked() {
            *value = grade;
        }
    }
}

/// A labelled number field, with `extra` (quick picks) after it.
fn field_row(
    ui: &mut egui::Ui,
    label: &str,
    text: &mut String,
    extra: impl FnOnce(&mut egui::Ui, &mut String),
) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [64.0, 22.0],
            egui::Label::new(theme::dim(label).size(size::SMALL)),
        );
        let field = ui.add(
            egui::TextEdit::singleline(text)
                .desired_width(78.0)
                .char_limit(13),
        );
        if field.changed() {
            text.retain(|c| c.is_ascii_digit() || c == ',');
        }
        extra(ui, text);
    });
}

/// What the order costs and does, before it's placed.
fn preview(ui: &mut egui::Ui, quote: &Result<OrderQuote, FormRefusal>) {
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::list_spacing(ui);
        match quote {
            Err(FormRefusal::Incomplete) => {
                ui.label(theme::dim("Set a quantity and a price each.").size(size::SMALL));
                ui.label(theme::dim("Placing costs 1% of the order's value.").size(size::SMALL));
            }
            Err(why) => {
                ui.label(RichText::new(refusal(why)).color(SHORT).size(size::SMALL));
            }
            Ok(q) => {
                row(ui, "Value", theme::text(grouped(q.value)));
                row(ui, "Fee (1%)", theme::dim(grouped(q.fee).to_string()));
                ui.add_space(2.0);
                theme::divider(ui);
                ui.add_space(2.0);
                let fills = &q.fills;
                let fills_text = match fills.mean_price() {
                    None if fills.maybe_more => "Maybe some, in a blend".to_owned(),
                    None => "None: it rests on the book".to_owned(),
                    Some(mean) => {
                        let more = if fills.maybe_more { " or more" } else { "" };
                        format!("{}{more} at {} avg", grouped(fills.quantity), grouped(mean))
                    }
                };
                let ink = if fills.quantity > 0 { ENOUGH } else { DIM };
                row(ui, "Fills now", RichText::new(fills_text).color(ink));
                if let Some(after) = q.balance_after {
                    row(
                        ui,
                        "Balance after",
                        RichText::new(format!("{} S", grouped(after))).color(GOLD),
                    );
                }
                if let Some(left) = q.left_in_stack {
                    row(ui, "Left in the stack", theme::dim(grouped(left)));
                }
            }
        }
    });
}

/// Why the form can't place, in words.
fn refusal(why: &FormRefusal) -> String {
    match why {
        FormRefusal::Incomplete => String::new(),
        FormRefusal::Bad(e) => e.message().to_owned(),
        FormRefusal::NoStack => "You hold none to sell".to_owned(),
        FormRefusal::NotHeld { held } => format!("The stack holds {}", grouped(*held)),
        FormRefusal::Short { debit, short } => {
            format!(
                "Costs {} Strands: {} short",
                grouped(*debit),
                grouped(*short)
            )
        }
    }
}

/// One label and value, spread across the space.
fn row(ui: &mut egui::Ui, label: &str, value: RichText) {
    ui.horizontal(|ui| {
        ui.label(theme::dim(label).size(size::SMALL));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(value.size(size::SMALL));
        });
    });
}

/// A small quick-pick chip.
fn chip(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(size::SMALL).color(GOLD))
            .fill(theme::CHIP)
            .stroke(Stroke::new(1.0_f32, GOLD_DIM.gamma_multiply(0.6)))
            .rounding(4.0)
            .min_size(egui::vec2(0.0, 22.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A typed amount, 0 while there's none.
fn amount(text: &str) -> i64 {
    capitol_core::exchange::parse_quantity(text).unwrap_or(0)
}

fn cursor(enabled: bool) -> egui::CursorIcon {
    if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::NotAllowed
    }
}

// ---------------------------------------------------------------------------
// The offline sample (`--market-sample`)
// ---------------------------------------------------------------------------

/// A market to show with no server: planks and a few others trading, a
/// plank book with blended lots and both grade rules, a few orders of
/// yours, and an inventory to sell from.
pub struct Sample {
    pub catalog: Catalog,
    pub summary: Vec<MarketItemSummary>,
    pub book: Book,
    pub orders: Vec<MarketOrder>,
    pub inventory: Container,
    pub market_box: Container,
    pub strands: i64,
    /// Four months of plank trades, the last week busiest, some of them
    /// yours: what the chart draws from
    pub trades: Vec<MarketTrade>,
    pub mine: Vec<MyFill>,
}

impl Sample {
    pub fn new() -> Sample {
        use capitol_core::protocol::rules::market::OrderStatus;
        use capitol_core::protocol::rules::stacking::Units;
        let summary = |item: &str, last: i64, change: i64, volume: i64| MarketItemSummary {
            item_type: item.into(),
            last_price: Some(last),
            change_24h: Some(change),
            volume_24h: volume,
            best_bid: None,
            best_ask: None,
        };
        let level = |side, price, grade: &str, rule, quantity, orders| DepthLevel {
            side,
            price,
            grade: grade.into(),
            grade_rule: rule,
            quantity,
            orders,
        };
        use GradeRule::{Average, EveryUnit};
        use OrderSide::{Buy, Sell};
        let mut levels = vec![
            level(Sell, 41, "B", None, 12, 2),
            level(Sell, 41, "D", None, 6, 1),
            level(Sell, 42, "A", None, 4, 1),
            level(Sell, 43, "C", None, 30, 3),
            level(Sell, 44, "B", None, 18, 2),
            level(Sell, 46, "A", None, 9, 1),
            level(Sell, 48, "C", None, 55, 4),
            level(Sell, 52, "A", None, 20, 1),
            level(Buy, 39, "C", Some(Average), 25, 3),
            level(Buy, 39, "B", Some(EveryUnit), 8, 1),
            level(Buy, 38, "F", Some(Average), 40, 2),
            level(Buy, 37, "A", Some(EveryUnit), 6, 1),
            level(Buy, 35, "D", Some(Average), 70, 5),
            level(Buy, 33, "F", Some(Average), 120, 3),
        ];
        capitol_core::protocol::rules::market::sort_levels(&mut levels);
        let at = capitol_core::chrono::DateTime::<capitol_core::chrono::Utc>::UNIX_EPOCH;
        let order =
            |n: u128, side, item: &str, price, total, left, grade: Option<&str>, rule, units| {
                MarketOrder {
                    id: Uuid::from_u128(n),
                    player_id: Uuid::nil(),
                    side,
                    item_type: item.into(),
                    price,
                    quantity_total: total,
                    quantity_remaining: left,
                    status: OrderStatus::Open,
                    min_grade: grade.map(Into::into),
                    grade_rule: rule,
                    units,
                    escrow_strands: 0,
                    fee: 1,
                    created_at: at,
                    expires_at: at,
                }
            };
        let inventory = Container::new(
            Uuid::from_u128(100),
            "player_inventory",
            24,
            6,
            &[
                SlotItem::new(
                    Uuid::from_u128(101),
                    "plank".into(),
                    "Plank".into(),
                    71,
                    14,
                    0,
                ),
                SlotItem::new(Uuid::from_u128(102), "rope".into(), "Rope".into(), 58, 6, 1),
            ],
        );
        let market_box = Container::new(
            Uuid::from_u128(200),
            "market_box",
            48,
            8,
            &[SlotItem::new(
                Uuid::from_u128(201),
                "iron_ingot".into(),
                "Iron Ingot".into(),
                80,
                3,
                0,
            )],
        );
        let (trades, mine) = sample_trades(capitol_core::chrono::Utc::now());
        Sample {
            catalog: Catalog::fallback(),
            summary: vec![
                summary("plank", 41, 3, 184),
                summary("rope", 12, -1, 96),
                summary("iron_ingot", 118, 9, 31),
                summary("log", 6, 0, 240),
                summary("brick", 9, -2, 60),
            ],
            book: Book { seq: 1, levels },
            orders: vec![
                order(
                    1,
                    Buy,
                    "plank",
                    39,
                    10,
                    8,
                    Some("B"),
                    Some(EveryUnit),
                    Units::new(),
                ),
                order(
                    2,
                    Sell,
                    "rope",
                    14,
                    10,
                    4,
                    None,
                    None,
                    Units::from_pairs([(48, 1), (62, 3)]),
                ),
                order(
                    3,
                    Sell,
                    "plank",
                    44,
                    6,
                    6,
                    None,
                    None,
                    Units::from_pairs([(80, 6)]),
                ),
            ],
            inventory,
            market_box,
            strands: 1250,
            trades,
            mine,
        }
    }

    /// The chart `key` asks for, from [`Sample::trades`], as the server
    /// would answer it.
    pub fn history(&self, key: &ChartKey) -> History {
        let mut candles = Vec::new();
        let since = capitol_core::chrono::Utc::now() - key.bucket.span();
        let shown: Vec<&MarketTrade> = self.trades.iter().filter(|t| key.takes(t) && t.at >= since).collect();
        for trade in &shown {
            market::fold_trade(&mut candles, key.bucket.secs(), trade);
        }
        let response = MarketHistoryResponse {
            item_type: key.item_type.clone(),
            bucket_secs: key.bucket.secs(),
            candles,
            recent: shown.iter().rev().take(market::RECENT_TRADES).map(|t| (*t).clone()).collect(),
            mine: self
                .mine
                .iter()
                .filter(|f| key.takes(&f.trade) && f.trade.at >= since)
                .cloned()
                .collect(),
        };
        History::from_response(key.clone(), &response)
    }

    /// The panel's view of it, with the plank book when `selected` is plank.
    #[cfg(test)]
    pub fn view(&self, selected: Option<&str>) -> View<'_> {
        self.view_with(selected, None)
    }

    /// The panel's view, with a chart's history.
    pub fn view_with<'a>(&'a self, selected: Option<&str>, history: Option<&'a History>) -> View<'a> {
        View {
            catalog: &self.catalog,
            summary: Some(&self.summary),
            book: (selected == Some("plank")).then_some(&self.book),
            history,
            orders: self.orders.iter().collect(),
            inventory: Some(&self.inventory),
            market_box: Some(&self.market_box),
            strands: Some(self.strands),
            busy: false,
            refused: None,
            away_from_hall: None,
            bazaar: None,
            my_stall: None,
            walking_to: None,
        }
    }
}

/// Plank trading for four months, about one an hour and then three an hour in the
/// last week, its price walking up from about 30 to the summary's 41: a
/// seeded walk, the same every run, moved to end on 41. Every 131st trade
/// is yours.
fn sample_trades(now: capitol_core::chrono::DateTime<capitol_core::chrono::Utc>) -> (Vec<MarketTrade>, Vec<MyFill>) {
    let mut seed: u64 = 0x5eed_cafe;
    let mut next = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as f64 / (1u64 << 31) as f64
    };
    let day = 86_400i64;
    let end = now.timestamp();
    let mut t = end - 120 * day;
    let mut price = 30.0f64;
    let (mut trades, mut mine) = (Vec::new(), Vec::new());
    let mut n = 0u128;
    while t < end {
        let age = (end - t) as f64 / day as f64;
        let target = 30.0 + 11.0 * (1.0 - age / 120.0) + 2.5 * (age * 0.9).sin();
        price += (target - price) * 0.08 + (next() - 0.5) * 2.4;
        let quality = (40.0 + next() * 55.0) as i32;
        let grade = capitol_core::protocol::rules::quality_grade(quality);
        // Better grades fetch more
        let premium = (quality as f64 - 65.0) * 0.06;
        n += 1;
        let trade = MarketTrade {
            id: Uuid::from_u128(0x7000_0000 + n),
            buy_order_id: Uuid::from_u128(0x8000_0000 + n),
            sell_order_id: Uuid::from_u128(0x9000_0000 + n),
            item_type: "plank".into(),
            grade: grade.into(),
            quality,
            min_quality: (quality - (next() * 6.0) as i32).max(0),
            price: (price + premium).round().max(1.0) as i64,
            quantity: 1 + (next() * next() * 24.0) as i32,
            at: capitol_core::chrono::DateTime::from_timestamp(t, 0).unwrap_or(now),
        };
        if n.is_multiple_of(131) {
            let side = if n.is_multiple_of(2) { OrderSide::Buy } else { OrderSide::Sell };
            mine.push(MyFill { side, trade: trade.clone() });
        }
        trades.push(trade);
        // A few a day, then a few an hour
        let gap = if age > 7.0 { 3600.0 } else { 3600.0 / 3.0 };
        t += (gap * (0.3 + next() * 1.4)) as i64;
    }
    // End on the summary's last price
    let shift = trades.last().map_or(0, |t| 41 - t.price);
    for t in trades.iter_mut().map(Some).chain(mine.iter_mut().map(|f| Some(&mut f.trade))).flatten() {
        t.price = (t.price + shift).max(1);
    }
    mine.reverse();
    (trades, mine)
}

impl Default for Sample {
    fn default() -> Self {
        Sample::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a few frames of the panel, clicking `click_at` on frame 5, and
    /// returns the action it asked for.
    fn run(panel: &mut MarketPanel, view: &View, click_at: Option<egui::Pos2>) -> Option<Action> {
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
                if let Some(a) = panel.show(ctx, &mut icons, view) {
                    out = Some(a);
                }
            });
        }
        out
    }

    fn open() -> MarketPanel {
        MarketPanel {
            open: true,
            ..Default::default()
        }
    }

    #[test]
    fn it_opens_on_the_busiest_item_and_fits_a_small_window() {
        let sample = Sample::new();
        let mut panel = open();
        run(&mut panel, &sample.view(None), None);
        assert_eq!(panel.selected.as_deref(), Some("log"));
        let rect = panel.rect.unwrap();
        assert!(
            rect.width() < 1280.0 - 32.0 && rect.bottom() < 720.0,
            "{rect:?}"
        );
        // Busiest first
        let items: Vec<&str> = panel
            .item_rows
            .iter()
            .take(3)
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(items, ["log", "plank", "rope"]);
    }

    #[test]
    fn clicking_an_ask_fills_a_buy_at_its_price_and_placing_sends_it() {
        let sample = Sample::new();
        let mut panel = open();
        panel.select("plank");
        run(&mut panel, &sample.view(Some("plank")), None);
        // Five levels each side at most, asks cheapest nearest the spread
        let asks: Vec<i64> = panel
            .level_rows
            .iter()
            .filter(|(l, _)| l.side == OrderSide::Sell)
            .map(|(l, _)| l.price)
            .collect();
        assert_eq!(asks, [44, 43, 42, 41, 41]);
        let (ask, at) = panel
            .level_rows
            .iter()
            .find(|(l, _)| l.side == OrderSide::Sell && l.price == 42)
            .cloned()
            .unwrap();
        run(&mut panel, &sample.view(Some("plank")), Some(at.center()));
        assert_eq!(
            (panel.form.side, panel.form.price.as_str()),
            (OrderSide::Buy, "42")
        );
        assert_eq!(panel.form.quantity, ask.quantity.to_string());
        let place = panel.place.unwrap().center();
        let action = run(&mut panel, &sample.view(Some("plank")), Some(place));
        let Some(Action::Place(order)) = action else {
            panic!("{action:?}");
        };
        assert_eq!(
            (order.side, order.price, order.quantity),
            (OrderSide::Buy, 42, 4)
        );
        assert_eq!(
            (order.min_grade.as_deref(), order.grade_rule),
            (Some("F"), Some(GradeRule::Average))
        );
    }

    #[test]
    fn a_sell_names_its_stack_and_a_short_buy_cannot_be_placed() {
        let sample = Sample::new();
        let mut panel = open();
        panel.preset("plank:sell:5@40");
        run(&mut panel, &sample.view(Some("plank")), None);
        let place = panel.place.unwrap().center();
        let Some(Action::Place(order)) = run(&mut panel, &sample.view(Some("plank")), Some(place))
        else {
            panic!("no sell");
        };
        assert_eq!(
            (order.side, order.item_id, order.quantity),
            (OrderSide::Sell, Some(Uuid::from_u128(101)), 5)
        );

        panel.preset("plank:buy:100@41:B");
        assert_eq!(panel.form.min_grade, "B");
        run(&mut panel, &sample.view(Some("plank")), None);
        let place = panel.place.unwrap().center();
        assert_eq!(
            run(&mut panel, &sample.view(Some("plank")), Some(place)),
            None
        );
    }

    #[test]
    fn your_orders_cancel_and_open_their_book() {
        let sample = Sample::new();
        let mut panel = open();
        panel.select("plank");
        panel.tab = Tab::Orders;
        run(&mut panel, &sample.view(Some("plank")), None);
        let (id, at) = panel.cancels[1];
        assert_eq!(
            run(&mut panel, &sample.view(Some("plank")), Some(at.center())),
            Some(Action::Cancel(id))
        );
        // The rest of a row shows its item's book: the second is rope
        let row = at.center() - egui::vec2(100.0, 0.0);
        assert_eq!(
            run(&mut panel, &sample.view(Some("plank")), Some(row)),
            None
        );
        assert_eq!(panel.selected.as_deref(), Some("rope"));
    }

    #[test]
    fn the_chart_marks_your_fills_and_a_trade_clicked_takes_its_price() {
        let sample = Sample::new();
        let mut panel = open();
        panel.preset("plank:chart:grade-B");
        let key = panel.charting().unwrap();
        assert_eq!((key.item_type.as_str(), key.grade.as_deref()), ("plank", Some("B")));
        // A week of hourly candles, only B trades
        let bs = sample.history(&key);
        assert!(bs.candles.len() > 50, "{}", bs.candles.len());
        assert!(bs.recent.iter().all(|t| t.grade == "B"));
        panel.preset("plank:chart:grade-all");
        let history = sample.history(&panel.charting().unwrap());
        assert!(history.recent.len() == market::RECENT_TRADES && !history.mine.is_empty());
        run(&mut panel, &sample.view_with(Some("plank"), Some(&history)), None);
        let rect = panel.rect.unwrap();
        assert!(rect.width() < 1280.0 - 32.0 && rect.bottom() < 720.0, "{rect:?}");
        let (id, at) = panel.chart_state.tape_rows[2];
        let price = history.recent.iter().find(|t| t.id == id).unwrap().price;
        run(&mut panel, &sample.view_with(Some("plank"), Some(&history)), Some(at.center()));
        assert_eq!((panel.pane, panel.form.price.clone()), (Pane::Book, price.to_string()));
        // On the book, nothing is charted
        assert_eq!(panel.charting(), None);
    }

    #[test]
    fn a_chart_for_another_key_is_not_drawn() {
        let sample = Sample::new();
        let mut panel = open();
        panel.preset("plank:chart");
        let day = ChartKey {
            bucket: market::Bucket::Day,
            ..panel.charting().unwrap()
        };
        let history = sample.history(&day);
        run(&mut panel, &sample.view_with(Some("plank"), Some(&history)), None);
        assert!(panel.chart_state.tape_rows.is_empty());
    }

    #[test]
    fn closed_draws_nothing_and_watches_nothing() {
        let sample = Sample::new();
        let mut panel = MarketPanel {
            selected: Some("plank".into()),
            ..Default::default()
        };
        assert_eq!(run(&mut panel, &sample.view(None), None), None);
        assert_eq!((panel.rect, panel.watching()), (None, None));
    }
}
