//! A bazaar stall's shop (E3.11), opened at its stall: click the stall
//! (`input.rhai`, `capitol_core::bazaar::Visit`) or its row in the market
//! panel's Bazaar tab, and you walk there, in at the gate and up the lane,
//! and it opens as you reach the counter. It stands top left, beside the
//! crafting panel while that's open (as the exchange does), so the
//! inventory (bottom right) stays clear for stocking.
//!
//! - **Someone's shop:** its name and keeper, and every listing: the item,
//!   each grade it holds and how many (never an average, D2), the price a
//!   unit, and Buy n. Buying takes the best units first, as the server
//!   does; click a grade to buy only that grade's. Prices are seen at the
//!   stall (D6): from afar the panel says how far away you are, and Buy
//!   waits until you're back.
//! - **Your shop:** when the rent runs out, Pay to keep it (up to a week
//!   ahead) and a rename; your listings with Unlist; and Stock, from a stack
//!   in your inventory at a price a unit.
//! - **A free stall:** what it costs (`rules::bazaar`), a name and the days,
//!   and Rent. One stall a merchant: if you keep another, it says which.
//!
//! The numbers are `capitol_core::bazaar`'s (`buy`, `stock`, `rent`), the
//! server's rules, so what Buy and Rent say is what they cost. Sales at your
//! stall and its closing come as toasts (`market_note`), panel or no panel.

use std::collections::HashMap;

use capitol_core::bazaar::{self, Refusal};
use capitol_core::catalog::Catalog;
use capitol_core::chrono::{DateTime, Utc};
use capitol_core::protocol::rest::{BazaarListing, BazaarShop, BuyListing, RentStall, ShopSign, StockListing};
use capitol_core::protocol::rules::bazaar as rules;
use capitol_core::state::Container;
use egui::{Align, Align2, Id, Layout, RichText, Sense, Stroke};
use uuid::Uuid;

use super::icons::ItemIcons;
use super::theme::{self, size, BAZAAR, DIM, GOLD, SHORT, TEXT};

/// The panel's width inside its frame.
const WIDTH: f32 = 340.0;
/// How tall the listings may stand before they scroll.
const LISTINGS_HEIGHT: f32 = 250.0;
const ICON: f32 = 28.0;
/// How long the panel takes to open or close, and how far it slides.
const FADE: f32 = 0.18;
const RISE: f32 = 12.0;

/// What the panel asks for this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Buy { listing_id: Uuid, request: BuyListing },
    Rent { stall_id: String, request: RentStall },
    Stock(StockListing),
    Unlist(Uuid),
    /// Walk to your own stall, from a free one
    GoTo(String),
    Close,
}

/// What the panel draws from.
pub struct View<'a> {
    pub stall_id: &'a str,
    /// The shop as last seen at the stall, with its prices
    pub shop: Option<&'a BazaarShop>,
    /// The stall as the directory has it: kept or free, from anywhere
    pub sign: Option<&'a ShopSign>,
    /// The directory has answered
    pub listed: bool,
    /// Your own shop, at whichever stall
    pub mine: Option<&'a BazaarShop>,
    pub me: Option<Uuid>,
    /// How far you are from the stall, when that's further than its range
    pub away: Option<f64>,
    pub inventory: Option<&'a Container>,
    pub strands: Option<i64>,
    pub catalog: &'a Catalog,
    pub busy: bool,
    pub refused: Option<&'a str>,
    pub now: DateTime<Utc>,
}

/// One listing's Buy: how many, and of which grade (all, best first, when
/// none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pick {
    n: i32,
    grade: Option<&'static str>,
}

impl Default for Pick {
    fn default() -> Self {
        Pick { n: 1, grade: None }
    }
}

/// The stock form, as typed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StockForm {
    pub stack: Option<Uuid>,
    pub quantity: String,
    pub price: String,
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct ShopPanel {
    /// The stall it's open on
    pub stall: Option<String>,
    picks: HashMap<Uuid, Pick>,
    pub stock: StockForm,
    /// A new shop's name, or your shop's new one, as typed
    pub name: String,
    pub days: i64,
    /// Where things were drawn this frame, for the tests to click
    buys: Vec<(Uuid, egui::Rect)>,
    rent_button: Option<egui::Rect>,
    /// Where the panel was drawn this frame. `None` while it's closed.
    pub rect: Option<egui::Rect>,
}

impl ShopPanel {
    /// Opens the panel on `stall_id`.
    pub fn open_on(&mut self, stall_id: &str) {
        if self.stall.as_deref() != Some(stall_id) {
            self.picks.clear();
            self.days = 0;
        }
        self.stall = Some(stall_id.to_owned());
    }

    pub fn close(&mut self) {
        self.stall = None;
    }

    /// Draws the panel top left, or just right of `beside` (the crafting
    /// panel) while that's open.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        icons: &mut ItemIcons,
        view: Option<&View>,
        beside: Option<egui::Rect>,
    ) -> Option<Action> {
        let open = view.is_some() && self.stall.is_some();
        let shown = ctx.animate_bool_with_time(Id::new("capitol_shop_fade"), open, FADE);
        self.rect = None;
        let view = view?;
        if shown <= 0.0 {
            return None;
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        let top = ctx.available_rect().top();
        let area = egui::Area::new(Id::new("capitol_shop"))
            .anchor(Align2::LEFT_TOP, [beside.map_or(12.0, |r| r.right() + 12.0), top + 12.0 - RISE * (1.0 - ease)])
            .interactable(open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(BAZAAR).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    self.body(ui, icons, view)
                })
                .inner
            });
        self.rect = open.then_some(area.response.rect);
        area.inner
    }

    fn body(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) -> Option<Action> {
        let mut action = None;
        let yours = view.mine.filter(|m| m.stall_id == view.stall_id);
        let number = stall_number(view.stall_id);
        // The heading: the shop's name, or the stall's
        ui.horizontal(|ui| {
            let title = match (yours, view.shop, view.sign) {
                (Some(m), _, _) => m.name.clone(),
                (None, Some(s), _) => s.name.clone(),
                (None, None, Some(sign)) => sign.name.clone(),
                _ => format!("Stall {number}"),
            };
            ui.label(theme::title(&title, size::LARGE).color(BAZAAR));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let close = ui.add(egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM)).frame(false));
                if close.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Close").clicked() {
                    action = Some(Action::Close);
                }
            });
        });
        let kept = yours.is_some() || view.shop.is_some() || view.sign.is_some();
        match (yours, kept) {
            (Some(shop), _) => {
                ui.label(theme::dim(format!("Your shop · stall {number} · {}", paid_words(shop.paid_until, view.now))).size(size::SMALL));
                theme::gold_rule(ui);
                if let Some(a) = self.yours(ui, icons, view, shop) {
                    action = Some(a);
                }
            }
            (None, true) => {
                let (keeper, citizen) = match (view.shop, view.sign) {
                    (Some(s), _) => (s.owner_name.as_str(), s.owner_citizen),
                    (None, Some(s)) => (s.owner_name.as_str(), s.owner_citizen),
                    _ => ("", false),
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(theme::dim(format!("Kept by {keeper}")).size(size::SMALL));
                    if citizen {
                        ui.label(RichText::new("◆ citizen").size(size::LABEL).color(DIM))
                            .on_hover_text("A citizen: one of the city's own merchants, who earns all they sell");
                    }
                    ui.label(theme::dim(format!("· stall {number}")).size(size::SMALL));
                });
                theme::gold_rule(ui);
                if let Some(a) = self.theirs(ui, icons, view) {
                    action = Some(a);
                }
            }
            (None, false) => {
                ui.label(theme::dim(format!("To let · {} Strands a day", rules::RENT_PER_DAY)).size(size::SMALL));
                theme::gold_rule(ui);
                if let Some(a) = self.free_stall(ui, view) {
                    action = Some(a);
                }
            }
        }
        if let Some(why) = view.refused {
            ui.add_space(4.0);
            theme::error_box(ui, why);
        }
        action
    }

    /// Where you stand, if not at the stall: what needs it waits.
    fn away_note(ui: &mut egui::Ui, view: &View, what: &str) {
        if let Some(d) = view.away {
            ui.label(theme::dim(format!("You're {:.0} m away: {what}", d / 10.0)).size(size::SMALL).italics());
            ui.add_space(4.0);
        }
    }

    // -- Someone's shop ----------------------------------------------------

    fn theirs(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) -> Option<Action> {
        let Some(shop) = view.shop else {
            let line = match view.away {
                Some(_) => "Its prices are seen at the stall. Walk there to browse.",
                None => "Asking the stall…",
            };
            ui.label(theme::dim(line).size(size::SMALL));
            if let Some(sign) = view.sign.filter(|s| !s.item_types.is_empty()) {
                ui.add_space(4.0);
                goods_line(ui, icons, view.catalog, &sign.item_types);
            }
            return None;
        };
        Self::away_note(ui, view, "these are its prices as you last saw them. Walk back to buy.");
        if shop.listings.is_empty() {
            ui.label(theme::dim("Its shelves are bare today.").size(size::SMALL));
            return None;
        }
        self.buys.clear();
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("capitol_shop_listings")
            .max_height(LISTINGS_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for listing in &shop.listings {
                    if let Some(a) = self.buy_row(ui, icons, view, shop, listing) {
                        action = Some(a);
                    }
                    ui.add_space(4.0);
                }
            });
        action
    }

    /// One listing to buy from: icon, name and price; its grades, each a
    /// chip to buy only that grade; and Buy n.
    fn buy_row(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
        shop: &BazaarShop,
        listing: &BazaarListing,
    ) -> Option<Action> {
        let mut action = None;
        let pick = self.picks.entry(listing.id).or_default();
        theme::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), Sense::hover());
                icons.paint(ui, r, &listing.item_type);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.horizontal(|ui| {
                        ui.label(theme::text(view.catalog.item_name(&listing.item_type)).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("{} each", listing.price)).color(GOLD).strong());
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for (g, n) in bazaar::grades(&listing.units) {
                            let on = pick.grade == Some(g);
                            if grade_chip(ui, g, n, on).clicked() {
                                pick.grade = if on { None } else { Some(g) };
                                pick.n = 1;
                            }
                        }
                        if let Some(d) = listing.durability {
                            ui.label(theme::dim(format!("· wear {d}")).size(size::LABEL));
                        }
                    });
                });
            });
            let quote = bazaar::buy(listing, pick.n, pick.grade, shop.owner_id, view.me, view.strands);
            let has = match pick.grade {
                Some(g) => bazaar::grades(&listing.units).iter().find(|(x, _)| *x == g).map_or(0, |(_, n)| *n),
                None => listing.units.count() as i32,
            };
            ui.horizontal(|ui| {
                stepper(ui, &mut pick.n, 1, has.max(1));
                let what = match pick.grade {
                    Some(g) => format!("{g} only"),
                    None => "best first".to_owned(),
                };
                ui.label(theme::dim(what).size(size::LABEL));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let cost = listing.price * i64::from(pick.n);
                    let can = quote.is_ok() && view.away.is_none() && !view.busy;
                    let label = format!("Buy {} · {cost}", pick.n);
                    let button = small_accent(ui, &label, can);
                    let button = match (&quote, view.away) {
                        (Err(why), _) => button.on_disabled_hover_text(why.message()),
                        (Ok(_), Some(_)) => button.on_disabled_hover_text("Walk to the stall to buy"),
                        _ => button.on_hover_text("Paid to the merchant at once; it goes to your pack"),
                    };
                    self.buys.push((listing.id, button.rect));
                    if button.clicked() && can {
                        if let Ok(p) = &quote {
                            action = Some(Action::Buy { listing_id: listing.id, request: p.request.clone() });
                        }
                    }
                });
            });
        });
        action
    }

    // -- Your shop ---------------------------------------------------------

    fn yours(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View, shop: &BazaarShop) -> Option<Action> {
        let mut action = None;
        // Paying ahead, and a rename
        let most = bazaar::days_payable(Some(shop), view.now);
        ui.horizontal(|ui| {
            ui.label(theme::dim("Name").size(size::SMALL));
            ui.add(egui::TextEdit::singleline(&mut self.name).hint_text(shop.name.as_str()).desired_width(WIDTH - 60.0).char_limit(rules::MAX_SHOP_NAME));
        });
        if most > 0 {
            self.days = self.days.clamp(1, most);
            ui.horizontal(|ui| {
                stepper_days(ui, &mut self.days, most);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let quote = bazaar::rent(self.days, &self.name, Some(shop), view.strands, view.now);
                    let can = quote.is_ok() && view.away.is_none() && !view.busy;
                    let b = small_accent(ui, &format!("Pay {} · {}", day_words(self.days), rules::rent(self.days)), can);
                    let b = match (&quote, view.away) {
                        (Err(why), _) => b.on_disabled_hover_text(why.message()),
                        (Ok(_), Some(_)) => b.on_disabled_hover_text("Pay at your stall"),
                        _ => b.on_hover_text("Keeps the shop open longer, up to a week from now"),
                    };
                    if b.clicked() && can {
                        if let Ok(request) = quote {
                            action = Some(Action::Rent { stall_id: shop.stall_id.clone(), request });
                        }
                    }
                });
            });
        } else {
            ui.label(theme::dim("Paid a week ahead, the most there is.").size(size::SMALL));
        }
        ui.add_space(6.0);
        Self::away_note(ui, view, "stocking and paying happen at your stall.");
        ui.label(theme::label(&format!("On the counter · {} of {}", shop.listings.len(), rules::MAX_LISTINGS)));
        ui.add_space(2.0);
        egui::ScrollArea::vertical()
            .id_salt("capitol_shop_mine")
            .max_height(LISTINGS_HEIGHT * 0.7)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if shop.listings.is_empty() {
                    ui.label(theme::dim("Nothing yet. Stock it from your pack below.").size(size::SMALL));
                }
                for listing in &shop.listings {
                    theme::well().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::hover());
                            icons.paint(ui, r, &listing.item_type);
                            ui.label(theme::text(view.catalog.item_name(&listing.item_type)));
                            ui.spacing_mut().item_spacing.x = 3.0;
                            for (g, n) in bazaar::grades(&listing.units) {
                                grade_chip(ui, g, n, false);
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let b = ui.add_enabled(!view.busy && view.away.is_none(), egui::Button::new(RichText::new("Unlist").size(size::SMALL)));
                                if b.on_hover_text("Back to your pack").clicked() {
                                    action = Some(Action::Unlist(listing.id));
                                }
                                ui.label(RichText::new(format!("{}", listing.price)).color(GOLD));
                            });
                        });
                    });
                    ui.add_space(2.0);
                }
            });
        ui.add_space(6.0);
        if let Some(a) = self.stock_form(ui, view, shop) {
            action = Some(a);
        }
        action
    }

    /// Stock: a stack from your pack, how many, and the price a unit.
    fn stock_form(&mut self, ui: &mut egui::Ui, view: &View, shop: &BazaarShop) -> Option<Action> {
        let mut action = None;
        ui.label(theme::label("Stock from your pack"));
        let stacks: Vec<_> = view.inventory.map(|c| c.items().collect()).unwrap_or_default();
        if stacks.is_empty() {
            ui.label(theme::dim("Your pack is empty.").size(size::SMALL));
            return None;
        }
        if !self.stock.stack.is_some_and(|id| stacks.iter().any(|s| s.item_id == id)) {
            self.stock.stack = stacks.first().map(|s| s.item_id);
        }
        let chosen = stacks.iter().find(|s| Some(s.item_id) == self.stock.stack).copied();
        let describe = |s: &capitol_core::protocol::rest::SlotItem| {
            format!("{} ×{} ({})", s.item_name, s.quantity, bazaar::grades(&s.units).iter().map(|(g, n)| format!("{g} {n}")).collect::<Vec<_>>().join(", "))
        };
        egui::ComboBox::from_id_salt("capitol_shop_stack")
            .width(WIDTH - 16.0)
            .selected_text(chosen.map(describe).unwrap_or_default())
            .show_ui(ui, |ui| {
                for s in &stacks {
                    ui.selectable_value(&mut self.stock.stack, Some(s.item_id), describe(s));
                }
            });
        ui.horizontal(|ui| {
            ui.label(theme::dim("How many").size(size::SMALL));
            let hint = chosen.map(|s| s.quantity.to_string()).unwrap_or_default();
            number_field(ui, &mut self.stock.quantity, &hint, 52.0);
            ui.label(theme::dim("Price each").size(size::SMALL));
            number_field(ui, &mut self.stock.price, "", 60.0);
        });
        let quantity = self.stock.quantity.parse::<i32>().ok().or(chosen.map(|s| s.quantity)).unwrap_or(0);
        let price = self.stock.price.parse::<i64>().unwrap_or(0);
        let quote = chosen
            .ok_or(Refusal::NotHeld { has: 0 })
            .and_then(|s| bazaar::stock(s, quantity, price, shop.listings.len()));
        let can = quote.is_ok() && view.away.is_none() && !view.busy;
        let label = if price > 0 && quantity > 0 {
            format!("Stock {quantity} at {price} · {} if all sell", i64::from(quantity) * price)
        } else {
            "Stock".to_owned()
        };
        ui.add_space(2.0);
        let b = ui.add_enabled_ui(can, |ui| theme::accent_button(ui, &label, BAZAAR, can)).inner;
        let b = match (&quote, view.away) {
            (Err(_), _) if price == 0 => b.on_disabled_hover_text("Name a price a unit"),
            (Err(why), _) => b.on_disabled_hover_text(why.message()),
            (Ok(_), Some(_)) => b.on_disabled_hover_text("Stock it at your stall"),
            _ => b.on_hover_text("No fee: the rent is the bazaar's only charge"),
        };
        if b.clicked() && can {
            if let Ok(request) = quote {
                action = Some(Action::Stock(request));
                self.stock.quantity.clear();
            }
        }
        action
    }

    // -- A free stall ------------------------------------------------------

    fn free_stall(&mut self, ui: &mut egui::Ui, view: &View) -> Option<Action> {
        if !view.listed {
            ui.label(theme::dim("Asking the bazaar…").size(size::SMALL));
            return None;
        }
        if let Some(mine) = view.mine {
            ui.label(theme::text(format!("You keep {} at stall {}. A merchant keeps one stall at a time.", mine.name, stall_number(&mine.stall_id))).size(size::SMALL));
            ui.add_space(4.0);
            if theme::button(ui, "Walk to your stall", true).clicked() {
                return Some(Action::GoTo(mine.stall_id.clone()));
            }
            return None;
        }
        ui.label(
            theme::text(format!(
                "{} Strands a day, paid up to a week ahead. Stock it from your pack and set your prices; no fee on a sale. What hasn't sold when the rent runs out goes to your market box.",
                rules::RENT_PER_DAY
            ))
            .size(size::SMALL),
        );
        ui.add_space(6.0);
        Self::away_note(ui, view, "rent it at the stall.");
        ui.horizontal(|ui| {
            ui.label(theme::dim("Shop name").size(size::SMALL));
            ui.add(egui::TextEdit::singleline(&mut self.name).hint_text("Name your shop").desired_width(WIDTH - 80.0).char_limit(rules::MAX_SHOP_NAME));
        });
        self.days = self.days.clamp(1, rules::MAX_RENT_DAYS);
        let mut action = None;
        ui.horizontal(|ui| {
            stepper_days(ui, &mut self.days, rules::MAX_RENT_DAYS);
            let cost = rules::rent(self.days);
            let short = view.strands.is_some_and(|s| s < cost);
            ui.label(RichText::new(format!("= {cost} Strands")).color(if short { SHORT } else { GOLD }));
        });
        ui.add_space(4.0);
        let quote = bazaar::rent(self.days, &self.name, None, view.strands, view.now);
        let can = quote.is_ok() && view.away.is_none() && !view.busy;
        let b = ui.add_enabled_ui(can, |ui| theme::accent_button(ui, &format!("Rent stall {}", stall_number(view.stall_id)), BAZAAR, can)).inner;
        self.rent_button = Some(b.rect);
        let b = match (&quote, view.away) {
            (Err(why), _) => b.on_disabled_hover_text(why.message()),
            (Ok(_), Some(_)) => b.on_disabled_hover_text("Rent it at the stall"),
            _ => b,
        };
        if b.clicked() && can {
            if let Ok(request) = quote {
                action = Some(Action::Rent { stall_id: view.stall_id.to_owned(), request });
                self.name.clear();
            }
        }
        action
    }
}

/// The number on a stall's id: `bazaar_stall_7` is 7.
pub fn stall_number(stall_id: &str) -> &str {
    stall_id.rsplit('_').next().unwrap_or(stall_id)
}

/// "paid 2 days 4 hours ahead", "rent runs out within the hour", or "rent
/// run out".
fn paid_words(until: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let left = (until - now).num_minutes();
    match left {
        l if l <= 0 => "the rent has run out".to_owned(),
        l if l < 60 => "the rent runs out within the hour".to_owned(),
        l => {
            let (days, hours) = (l / (24 * 60), l / 60 % 24);
            match (days, hours) {
                (0, h) => format!("paid {h} h ahead"),
                (d, 0) => format!("paid {} ahead", day_words(d)),
                (d, h) => format!("paid {} {h} h ahead", day_words(d)),
            }
        }
    }
}

fn day_words(days: i64) -> String {
    if days == 1 {
        "1 day".to_owned()
    } else {
        format!("{days} days")
    }
}

/// The kinds of goods a shop has, as the directory names them: icons in a
/// row, their names on hover.
pub fn goods_line(ui: &mut egui::Ui, icons: &mut ItemIcons, catalog: &Catalog, item_types: &[String]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for t in item_types {
            let (r, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
            icons.paint(ui, r, t);
            response.on_hover_text(catalog.item_name(t));
        }
    });
}

/// A grade and how many of it: filled when it's the one chosen.
fn grade_chip(ui: &mut egui::Ui, grade: &str, n: i32, on: bool) -> egui::Response {
    let colour = theme::grade(grade);
    ui.add(
        egui::Button::new(
            RichText::new(format!("{grade} {n}"))
                .font(theme::strong(size::LABEL))
                .color(if on { theme::BG_DARK } else { colour }),
        )
        .fill(if on { colour } else { colour.gamma_multiply(0.15) })
        .stroke(Stroke::new(1.0_f32, colour.gamma_multiply(if on { 1.0 } else { 0.45 })))
        .rounding(3.0)
        .min_size(egui::vec2(0.0, 18.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .on_hover_text(if on { "Buying only these: click again for the best of any grade" } else { "Buy only this grade" })
}

/// − n +, from `lo` to `hi`.
fn stepper(ui: &mut egui::Ui, n: &mut i32, lo: i32, hi: i32) {
    ui.spacing_mut().item_spacing.x = 2.0;
    if ui.add_enabled(*n > lo, egui::Button::new("−").min_size(egui::vec2(20.0, 20.0))).clicked() {
        *n -= 1;
    }
    ui.add_sized([28.0, 20.0], egui::Label::new(RichText::new(n.to_string()).color(TEXT).strong()));
    if ui.add_enabled(*n < hi, egui::Button::new("+").min_size(egui::vec2(20.0, 20.0))).clicked() {
        *n += 1;
    }
    *n = (*n).clamp(lo, hi.max(lo));
}

/// Days, 1 to `most`.
fn stepper_days(ui: &mut egui::Ui, days: &mut i64, most: i64) {
    let mut n = *days as i32;
    ui.label(theme::dim("Days").size(size::SMALL));
    stepper(ui, &mut n, 1, most.max(1) as i32);
    *days = i64::from(n);
}

/// A field for a whole number.
fn number_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) {
    let field = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width).char_limit(10));
    if field.changed() {
        text.retain(|c| c.is_ascii_digit());
    }
}

/// The panel's small call to action: amber, on a row.
fn small_accent(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).font(theme::strong(size::SMALL)).color(BAZAAR))
            .fill(BAZAAR.gamma_multiply(0.16))
            .stroke(Stroke::new(1.5_f32, BAZAAR.gamma_multiply(0.8)))
            .rounding(4.0)
            .min_size(egui::vec2(0.0, 22.0)),
    )
}

// ---------------------------------------------------------------------------
// The offline sample (`--bazaar-sample`)
// ---------------------------------------------------------------------------

/// Shops to show offline: the directory, three kept stalls with their
/// prices, and yours, so the panel and the bazaar can be seen without a
/// server.
pub struct Sample {
    pub directory: capitol_core::protocol::rest::BazaarDirectory,
    pub shops: Vec<BazaarShop>,
    pub inventory: Container,
    pub me: Uuid,
}

impl Sample {
    pub fn new() -> Sample {
        use capitol_core::protocol::rest::{BazaarDirectory, BazaarStall, SlotItem};
        use capitol_core::protocol::rules::stacking::Units;
        let now = Utc::now();
        let me = Uuid::from_u128(0x5a);
        let listing = |n: u128, item: &str, price: i64, units: Units| BazaarListing {
            id: Uuid::from_u128(0x1000 + n),
            item_type: item.into(),
            price,
            units,
            durability: None,
            listed_at: now,
        };
        let shop = |stall: u32, name: &str, owner: &str, owner_id: u128, citizen: bool, listings: Vec<BazaarListing>| BazaarShop {
            stall_id: format!("bazaar_stall_{stall}"),
            name: name.into(),
            owner_id: Uuid::from_u128(owner_id),
            owner_name: owner.into(),
            owner_citizen: citizen,
            paid_until: now + capitol_core::chrono::Duration::hours(52),
            listings,
        };
        let shops = vec![
            shop(4, "Ada's Ropes", "ada", 1, false, vec![
                listing(1, "rope", 7, Units::from_pairs([(58, 3), (77, 2), (91, 1)])),
                listing(2, "fiber", 2, Units::from_pairs([(40, 12), (66, 8)])),
            ]),
            shop(9, "The Ironmonger", "Wren Hale", 2, true, vec![
                listing(3, "iron_ingot", 31, Units::from_pairs([(70, 4), (82, 2)])),
                listing(4, "nails", 3, Units::uniform(40, 60)),
            ]),
            shop(2, "Bram & Daughters", "bram", 3, false, vec![listing(5, "plank", 6, Units::from_pairs([(55, 10), (75, 6)]))]),
            shop(11, "Kiln & Hearth", "Juno", 4, true, vec![listing(6, "brick", 4, Units::uniform(30, 64))]),
            shop(6, "Mira's Mill", "mira", me.as_u128(), false, vec![listing(7, "plank", 8, Units::from_pairs([(62, 4), (79, 3)]))]),
        ];
        let stalls = (1..=13)
            .map(|k| {
                let id = format!("bazaar_stall_{k}");
                let shop = shops.iter().find(|s| s.stall_id == id).map(|s| ShopSign {
                    name: s.name.clone(),
                    owner_id: s.owner_id,
                    owner_name: s.owner_name.clone(),
                    owner_citizen: s.owner_citizen,
                    item_types: {
                        let mut t: Vec<String> = s.listings.iter().map(|l| l.item_type.clone()).collect();
                        t.sort();
                        t.dedup();
                        t
                    },
                });
                BazaarStall { stall_id: id, shop }
            })
            .collect();
        let rope = SlotItem::new(Uuid::from_u128(0x2001), "rope".into(), "Rope".into(), 72, 9, 0);
        let plank = SlotItem::new(Uuid::from_u128(0x2002), "plank".into(), "Plank".into(), 68, 14, 1);
        Sample {
            directory: BazaarDirectory { stalls },
            shops,
            inventory: Container::new(Uuid::from_u128(0x3001), "player_inventory", 4, 4, &[rope, plank]),
            me,
        }
    }

    pub fn shop(&self, stall_id: &str) -> Option<&BazaarShop> {
        self.shops.iter().find(|s| s.stall_id == stall_id)
    }

    pub fn mine(&self) -> Option<&BazaarShop> {
        self.shops.iter().find(|s| s.owner_id == self.me)
    }

    pub fn sign(&self, stall_id: &str) -> Option<&ShopSign> {
        self.directory.stalls.iter().find(|s| s.stall_id == stall_id).and_then(|s| s.shop.as_ref())
    }

    /// The panel's view of `stall_id`, as if you stood at it.
    pub fn view<'a>(&'a self, stall_id: &'a str, catalog: &'a Catalog) -> View<'a> {
        View {
            stall_id,
            shop: self.shop(stall_id),
            sign: self.sign(stall_id),
            listed: true,
            mine: self.mine(),
            me: Some(self.me),
            away: None,
            inventory: Some(&self.inventory),
            strands: Some(240),
            catalog,
            busy: false,
            refused: None,
            now: Utc::now(),
        }
    }
}

impl Default for Sample {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(panel: &mut ShopPanel, view: &View, click_at: Option<egui::Pos2>) -> Option<Action> {
        let ctx = theme::test_context();
        let mut icons = ItemIcons::load(std::path::Path::new(ItemIcons::DIR));
        let mut action = None;
        // A few frames for the fade, then the click on the last
        for frame in 0..12 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(853.0, 480.0))),
                ..Default::default()
            };
            input.time = Some(frame as f64 * 0.05);
            if let (Some(p), 11) = (click_at, frame) {
                input.events.push(egui::Event::PointerMoved(p));
                input.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
                input.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
            } else if let (Some(p), 10) = (click_at, frame) {
                input.events.push(egui::Event::PointerMoved(p));
            }
            let _ = ctx.run(input, |ctx| {
                if let Some(a) = panel.show(ctx, &mut icons, Some(view), None) {
                    action = Some(a);
                }
            });
        }
        action
    }

    #[test]
    fn someone_elses_shop_buys_the_best_first_and_fits_a_small_window() {
        let sample = Sample::new();
        let catalog = Catalog::fallback();
        let view = sample.view("bazaar_stall_4", &catalog);
        let mut panel = ShopPanel::default();
        panel.open_on("bazaar_stall_4");
        assert_eq!(run(&mut panel, &view, None), None);
        let rect = panel.rect.expect("drawn");
        assert!(rect.height() <= 480.0 - 24.0, "{rect:?} fits 1280×720 at 150%");
        let (id, button) = panel.buys[0];
        assert_eq!(id, Uuid::from_u128(0x1001));
        let action = run(&mut panel, &view, Some(button.center()));
        assert_eq!(action, Some(Action::Buy { listing_id: id, request: BuyListing { quantity: 1, qualities: None } }));
    }

    #[test]
    fn nothing_is_bought_from_afar() {
        let sample = Sample::new();
        let catalog = Catalog::fallback();
        let mut view = sample.view("bazaar_stall_4", &catalog);
        view.away = Some(340.0);
        let mut panel = ShopPanel::default();
        panel.open_on("bazaar_stall_4");
        run(&mut panel, &view, None);
        let (_, button) = panel.buys[0];
        assert_eq!(run(&mut panel, &view, Some(button.center())), None);
    }

    #[test]
    fn a_free_stall_rents_with_a_name_unless_you_keep_one() {
        let sample = Sample::new();
        let catalog = Catalog::fallback();
        let mut view = sample.view("bazaar_stall_7", &catalog);
        view.mine = None;
        let mut panel = ShopPanel::default();
        panel.open_on("bazaar_stall_7");
        panel.name = "Wren's Wares".into();
        run(&mut panel, &view, None);
        let rent = panel.rent_button.expect("a Rent button");
        let action = run(&mut panel, &view, Some(rent.center()));
        assert_eq!(
            action,
            Some(Action::Rent { stall_id: "bazaar_stall_7".into(), request: RentStall { days: 1, name: Some("Wren's Wares".into()) } })
        );

        // Keeping one already, there's no Rent, only the way to yours
        let view = sample.view("bazaar_stall_7", &catalog);
        let mut panel = ShopPanel::default();
        panel.open_on("bazaar_stall_7");
        run(&mut panel, &view, None);
        assert!(panel.rent_button.is_none());
    }

    #[test]
    fn your_own_shop_draws_its_stock_form_and_closed_draws_nothing() {
        let sample = Sample::new();
        let catalog = Catalog::fallback();
        let view = sample.view("bazaar_stall_6", &catalog);
        let mut panel = ShopPanel::default();
        panel.open_on("bazaar_stall_6");
        run(&mut panel, &view, None);
        assert!(panel.rect.is_some());
        assert_eq!(panel.stock.stack, Some(Uuid::from_u128(0x2001)), "the first stack is chosen");
        panel.close();
        run(&mut panel, &view, None);
        assert!(panel.rect.is_none());
    }

    #[test]
    fn paid_words_read_plainly() {
        let now = Utc::now();
        let h = |n: i64| now + capitol_core::chrono::Duration::hours(n);
        assert_eq!(paid_words(h(52), now), "paid 2 days 4 h ahead");
        assert_eq!(paid_words(h(24), now), "paid 1 day ahead");
        assert_eq!(paid_words(h(5), now), "paid 5 h ahead");
        assert_eq!(paid_words(now, now), "the rent has run out");
        assert_eq!(stall_number("bazaar_stall_12"), "12");
    }
}
