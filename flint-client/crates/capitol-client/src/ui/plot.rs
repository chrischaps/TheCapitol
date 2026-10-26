//! The plot panel (P6.4), bottom left as React's `PlotPanel.tsx` has it,
//! or beside the crafting panel while that's open.
//! Alt-click a plot to open it (`input.rhai`, click priority 4). The ×
//! closes it, and so does an alt-click on ground with no plot. Not Esc:
//! Flint's player quits on Esc.
//!
//! It has React's rows: zone, size, type, stations used of capacity,
//! owner, and the assessed value once owned. A plot for sale shows its
//! deed cost with your balance under it, and the Claim button. The deed
//! cost is the plot's `deed_cost` (`capitol_core::plot::Offer`), as React
//! shows it too.
//!
//! A plot of yours that owes tax shows the debt, a bar of how near it is to
//! foreclosure, and a button that pays what your strands cover (#138).
//!
//! Under the zone, a home zone's plot offers "About the <zone>", which
//! opens the zone card on it (#444).
//!
//! While the deed the road in earned is open (#448), a free home lot offers
//! "Claim with your deed" under the Strands price, which stays for anyone
//! who'd rather pay and keep the deed (D6). It asks once more before it
//! spends it. Before the Deeds Office has been looked in at, a line says
//! the deed waits there instead (D8; `capitol_core::deeds::offer`).
//!
//! On a plot of yours, *Hire a gatherer* opens the hire form (Commons S2.7,
//! `ui::hire`): piece-rate work gathering one of the resources that grow
//! on it, with your offers there over the form.
//!
//! On a plot of yours with a building, *Building* says its storeys and the
//! room indoors, and *Build upward…* opens the next floor's bill: the
//! permit, the goods against your pack, the time it takes, and the button
//! (V3.5, #477; `capitol_core::plot::upward`). While a floor goes up it says
//! so instead, with a bar of the time left.
//!
//! The panel is edged in gold, the colour of what's yours (plan §1), and
//! the claimed plot's gold edge in the world answers it.

use super::hire;
use super::theme::{self, size, DIM, ENOUGH, GOLD, SHORT, TEXT};
use capitol_core::deeds::DeedOffer;
use capitol_core::plot::{self as plots, words, Offer, Upward};
use capitol_core::protocol::rest::PlotInfo;
use capitol_core::protocol::rules::wages::Work;
use capitol_core::protocol::rules::Zone;
use capitol_core::wages::{self as wg, Wages};
use egui::{Align2, Id, RichText};

/// The panel's width inside its frame: React's 280 less the padding.
const WIDTH: f32 = 256.0;
/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;
/// How far it slides as it opens, in points.
const RISE: f32 = 12.0;

/// What the panel asks for this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Claim(uuid::Uuid),
    /// Claim it with the deed the road in earned (#448).
    ClaimWithDeed(uuid::Uuid),
    /// Pay this many strands of the plot's tax debt.
    PayTax(uuid::Uuid, i64),
    /// Open the zone card on this zone (#444)
    AboutZone(String),
    /// Something on the hire form (S2.7)
    Wages(hire::Act),
    /// Build the next floor (V3.5)
    BuildFloor(uuid::Uuid),
    Close,
}

/// What the panel draws from.
pub struct View<'a> {
    /// The picked plot, or `None` while nothing is picked.
    pub plot: Option<&'a PlotInfo>,
    pub offer: Offer,
    /// Your balance, once known.
    pub strands: Option<i64>,
    /// A claim is out.
    pub claiming: bool,
    /// A tax payment is out.
    pub paying: bool,
    /// Why the last claim or payment was refused, while it's fresh.
    pub refused: Option<&'a str>,
    /// What the road in's deed offers here (#448).
    pub deed: DeedOffer,
    /// Hiring gatherers here, on a plot of yours (S2.7). `None`: no hiring.
    pub hire: Option<Hire<'a>>,
    /// Building upward here, on a plot of yours with a building (V3.5).
    pub upward: Option<Upward>,
    /// A floor is being bought.
    pub building: bool,
}

/// What the hire form on a plot of yours draws from.
pub struct Hire<'a> {
    pub wages: &'a Wages,
    pub me: Option<uuid::Uuid>,
    /// The resources that grow on the lot, (id, name)
    pub resources: Vec<(String, String)>,
    pub catalog: &'a capitol_core::catalog::Catalog,
    pub now: capitol_core::chrono::DateTime<capitol_core::chrono::Utc>,
    pub online: bool,
}

/// The next floor's bill on the plot panel (V3.5): open or not, and to be
/// scrolled into view the frame after it opens, Build and all.
#[derive(Debug, Default, Clone, Copy)]
pub struct Bill {
    pub open: bool,
    scroll: bool,
}

impl Bill {
    /// Open it, Build in view.
    pub fn open(&mut self) {
        self.open = true;
        self.scroll = true;
    }
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct PlotPanel {
    /// The plot last shown, so it can fade out after the store lets go.
    last: Option<PlotInfo>,
    /// "Claim with your deed" was clicked once on this plot: it asks again.
    confirm: Option<uuid::Uuid>,
    /// The hire form is open, and what's written in it, for the plot it's for.
    hiring: bool,
    hire: Option<wg::Draft>,
    /// The next floor's bill (V3.5).
    pub bill: Bill,
    /// Where the panel was drawn this frame, for the controls hint to keep
    /// clear of. `None` while it's closed.
    pub rect: Option<egui::Rect>,
}

impl PlotPanel {
    /// Draws the panel bottom left, or just right of `beside` (the crafting
    /// panel, which takes the top left) while that's open, so neither
    /// covers the other.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        view: &View,
        beside: Option<egui::Rect>,
    ) -> Option<Action> {
        if let Some(p) = view.plot {
            self.last = Some(p.clone());
        }
        let open = view.plot.is_some();
        let shown = ctx.animate_bool_with_time(Id::new("capitol_plot_fade"), open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            self.last = None;
            return None;
        }
        let plot = self.last.as_ref()?;
        let ease = 1.0 - (1.0 - shown).powi(3);
        let left = beside.map_or(16.0, |r| r.right() + 12.0);
        let area = egui::Area::new(Id::new("capitol_plot"))
            .anchor(Align2::LEFT_BOTTOM, [left, -20.0 + RISE * (1.0 - ease)])
            .interactable(open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                // With the hire form open it can outgrow the screen: then
                // it scrolls under the header strip's gap
                let hiring = (self.hiring && view.hire.is_some() || self.bill.open && view.upward.is_some())
                    && view.offer == Offer::Yours;
                let room = (ui.ctx().available_rect().height() - 20.0 - 12.0 - 30.0).max(160.0);
                theme::panel_edged(GOLD)
                    .show(ui, |ui| {
                        let mut inner = |ui: &mut egui::Ui| {
                            let mut action = body(ui, plot, view, &mut self.confirm, &mut self.bill);
                            if let (Some(h), Offer::Yours) = (&view.hire, &view.offer) {
                                if let Some(a) = hire_section(ui, plot, h, view.strands, &mut self.hiring, &mut self.hire) {
                                    action = Some(a);
                                }
                            }
                            action
                        };
                        if hiring {
                            // An area's room is last frame's size: without
                            // this the scroll would never grow past it
                            ui.set_max_height(room);
                            theme::scrolled(ui, "capitol_plot_scroll", room, |ui| {
                                ui.set_width(WIDTH);
                                inner(ui)
                            })
                        } else {
                            inner(ui)
                        }
                    })
                    .inner
            });
        self.rect = open.then_some(area.response.rect);
        area.inner
    }
}

/// The panel's insides.
fn body(
    ui: &mut egui::Ui,
    plot: &PlotInfo,
    view: &View,
    confirm: &mut Option<uuid::Uuid>,
    bill: &mut Bill,
) -> Option<Action> {
    ui.set_width(WIDTH);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(theme::heading("Plot Details").color(GOLD));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui.add(
                egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM)).frame(false),
            );
            if close
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                action = Some(Action::Close);
            }
        });
    });
    ui.add_space(2.0);
    theme::divider(ui);
    ui.add_space(6.0);

    let zone = Zone::from_str(&plot.zone_id);
    let zone_name = zone.map_or_else(|| words(&plot.zone_id), |z| z.display_name().to_owned());
    let zone_ink = zone.map_or(TEXT, theme::zone);
    row(ui, "Zone", RichText::new(zone_name.clone()).color(zone_ink));
    if zone.is_some_and(|z| matches!(z, Zone::Urban | Zone::Suburban | Zone::Rural)) {
        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
            // "About the Urban zone": the display name already ends "Zone"
            let about = format!("About the {} zone", zone_name.trim_end_matches(" Zone"));
            if theme::link(ui, &about, size::LABEL).clicked() {
                action = Some(Action::AboutZone(plot.zone_id.clone()));
            }
        });
    }
    row(ui, "Size", theme::text(words(&plot.size_category)));
    row(ui, "Type", theme::text(words(&plot.plot_type)));
    let full = plot.station_count >= plot.station_capacity;
    row(
        ui,
        "Stations",
        theme::text(format!(
            "{} / {}",
            plot.station_count, plot.station_capacity
        ))
        .color(if full { SHORT } else { TEXT }),
    );
    let owner = match &view.offer {
        Offer::Yours => RichText::new("You").color(ENOUGH),
        Offer::Taken { owner } => theme::text(owner.clone()),
        _ => theme::dim("Unclaimed"),
    };
    row(ui, "Owner", owner);
    if plot.owner_id.is_some() {
        row(
            ui,
            "Value",
            theme::text(format!("{} Strands", plot.assessed_value)),
        );
        // What it's taxed, as the road in's tax card says it (#449)
        if let Some(week) = weekly_tax(plot) {
            row(ui, "Tax", theme::text(format!("{week} a week")));
        }
    }

    match &view.offer {
        Offer::Yours if plot.tax_owed > 0 => {
            section(ui);
            if let Some(a) = debt(ui, plot, view) {
                action = Some(a);
            }
        }
        // With a building, its section says what the plot holds instead
        Offer::Yours if view.upward.is_some() => {}
        Offer::Yours => {
            section(ui);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("You own this plot. Place stations here!")
                        .color(ENOUGH)
                        .size(size::BODY),
                );
            });
        }
        Offer::Taken { .. } => {}
        offer => {
            section(ui);
            if let Some(a) = claim(ui, plot, offer, view) {
                action = Some(a);
            }
            if let Some(a) = deed(ui, plot, view, confirm) {
                action = Some(a);
            }
        }
    }
    if let (Offer::Yours, Some(up)) = (&view.offer, &view.upward) {
        if let Some(a) = building(ui, plot, up, view, bill) {
            action = Some(a);
        }
    }
    if let Some(why) = view.refused {
        ui.add_space(6.0);
        theme::error_box(ui, why);
    }
    action
}

/// The building on a plot of yours (V3.5): its storeys and room indoors,
/// and the floor going up, or *Build upward…* and the next floor's bill.
/// Kept short, as the panel is often on a 480-point screen.
fn building(ui: &mut egui::Ui, plot: &PlotInfo, up: &Upward, view: &View, bill: &mut Bill) -> Option<Action> {
    section(ui);
    let room = plot.indoor_room.unwrap_or(0);
    // The floor going up has no room yet
    let built = match up {
        Upward::GoingUp { .. } => plot.floors - 1,
        Upward::Next { .. } => plot.floors,
    }
    .max(0);
    let storeys = 1 + built;
    let what = format!(
        "{storeys} {} · {} indoors",
        if storeys == 1 { "storey" } else { "storeys" },
        room * storeys
    );
    row(ui, "Building", theme::text(what));
    let mut action = None;
    match up {
        Upward::GoingUp { floor, left, secs } => {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let title = capitol_core::protocol::rules::floors::storey_title(*floor);
                ui.label(RichText::new(format!("{title} is going up")).color(theme::GOLD_LIGHT).size(size::SMALL));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::dim(format!("{} left", plots::build_words(*left))).size(size::LABEL));
                });
            });
            ui.add_space(2.0);
            let done = 1.0 - (*left as f32 / (*secs).max(1) as f32).clamp(0.0, 1.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(WIDTH, 4.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 2.0, theme::WELL);
            let mut filled = rect;
            filled.set_width(rect.width() * done);
            painter.rect_filled(filled, 2.0, GOLD);
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        }
        Upward::Next { floor, permit, short, goods, build_secs, room } => {
            ui.horizontal(|ui| {
                let label = if bill.open { "Hide the bill" } else { "Build upward…" };
                if theme::link(ui, label, size::SMALL)
                    .on_hover_text("Another storey over this one, with the building's room again. Each costs about twice the last")
                    .clicked()
                {
                    if bill.open {
                        bill.open = false;
                    } else {
                        bill.open();
                    }
                }
                if bill.open {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(theme::dim(format!("+{room} indoors, {} to go up", plots::build_words(*build_secs))).size(size::LABEL));
                    });
                }
            });
            if !bill.open {
                return None;
            }
            ui.add_space(4.0);
            theme::well().show(ui, |ui| {
                ui.set_width(ui.available_width());
                let ink = |short: bool| if short { SHORT } else { TEXT };
                egui::Grid::new("capitol_floor_bill").num_columns(3).spacing([10.0, 2.0]).min_row_height(16.0).show(ui, |ui| {
                    let mut line = |what: &str, need: String, short: bool, held: Option<String>| {
                        ui.label(theme::dim(what).size(size::LABEL));
                        ui.label(RichText::new(need).color(ink(short)).size(size::LABEL));
                        ui.label(theme::dim(held.unwrap_or_default()).size(size::LABEL));
                        ui.end_row();
                    };
                    line("Permit", format!("{permit} Strands"), *short > 0, view.strands.map(|s| format!("you hold {s}")));
                    for g in goods {
                        line(&g.name, g.need.to_string(), g.short() > 0, Some(format!("you hold {}", g.held)));
                    }
                });
            });
            ui.add_space(6.0);
            let (text, enabled) = match () {
                _ if view.building => ("Building…".to_owned(), false),
                _ if !up.can_build() => ("Not enough to build it".to_owned(), false),
                _ => (plots::build_label(*floor), true),
            };
            let button = theme::accent_button(ui, &text, GOLD, enabled)
                .on_hover_text("The goods are used up, the lowest grade first")
                .on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::NotAllowed });
            // Just opened on a short screen: bring the button into view,
            // frame after frame as the panel grows, until it's all there
            if bill.scroll {
                if ui.clip_rect().contains_rect(button.rect) {
                    bill.scroll = false;
                } else {
                    button.scroll_to_me(Some(egui::Align::Max));
                }
            }
            if button.clicked() && enabled {
                action = Some(Action::BuildFloor(plot.id));
            }
        }
    }
    action
}

/// *Hire a gatherer*, and the form under it once opened: work gathering
/// what grows on this plot of yours.
fn hire_section(
    ui: &mut egui::Ui,
    plot: &PlotInfo,
    h: &Hire,
    strands: Option<i64>,
    open: &mut bool,
    draft: &mut Option<wg::Draft>,
) -> Option<Action> {
    let offers = h.wages.mine_at(h.me, plot.id).len();
    section(ui);
    ui.horizontal(|ui| {
        let label = match (*open, offers) {
            (true, _) => "Hide the hiring".to_owned(),
            (false, 0) => "Hire a gatherer…".to_owned(),
            (false, n) => format!("Hire a gatherer · {n} offered"),
        };
        if theme::link(ui, &label, size::SMALL)
            .on_hover_text("Pay someone by the unit to gather here. What they gather goes to your market box")
            .clicked()
        {
            *open = !*open;
        }
    });
    if !*open {
        return None;
    }
    ui.add_space(6.0);
    let fresh = || {
        let work = Work::Gather { plot_id: plot.id, resource: h.resources.first().map(|r| r.0.clone()).unwrap_or_default() };
        let rate = wg::rate_for(&work, h.catalog);
        wg::Draft::new(work, rate)
    };
    let d = match draft {
        Some(d) if wg::place_of(&d.work) == plot.id => d,
        slot => slot.insert(fresh()),
    };
    let view = hire::View {
        wages: h.wages,
        me: h.me,
        strands,
        catalog: h.catalog,
        now: h.now,
        online: h.online,
        accent: GOLD,
    };
    hire::show(ui, plot.id, d, &h.resources, &view).map(Action::Wages)
}

/// The deed cost, your balance under it, and the Claim button.
fn claim(ui: &mut egui::Ui, plot: &PlotInfo, offer: &Offer, view: &View) -> Option<Action> {
    let cost = match offer {
        Offer::Claimable { cost } | Offer::TooDear { cost, .. } => Some(*cost),
        _ => None,
    };
    ui.vertical_centered(|ui| {
        let price = match cost {
            Some(c) => format!("{c} Strands"),
            None => "N/A".into(),
        };
        let mut job = egui::text::LayoutJob::default();
        let format = |font, color| egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        };
        job.append("Deed Cost: ", 0.0, format(theme::body(size::LARGE), DIM));
        job.append(&price, 0.0, format(theme::strong(size::LARGE), GOLD));
        ui.label(job);
        if let Some(held) = view.strands {
            let ink = if matches!(offer, Offer::TooDear { .. }) {
                SHORT
            } else {
                DIM
            };
            ui.label(
                RichText::new(format!("You hold {held} Strands"))
                    .color(ink)
                    .size(size::LABEL),
            );
        }
    });
    ui.add_space(8.0);
    let (text, enabled) = match offer {
        _ if view.claiming => ("Claiming…".to_owned(), false),
        Offer::Claimable { .. } => ("Claim Plot".to_owned(), true),
        Offer::TooDear { short, .. } => (format!("{short} Strands short"), false),
        Offer::NotForSale => ("Not for sale".to_owned(), false),
        _ => ("Claim Plot".to_owned(), false),
    };
    theme::accent_button(ui, &text, ENOUGH, enabled)
        .on_hover_cursor(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::NotAllowed
        })
        .clicked()
        .then_some(Action::Claim(plot.id))
}

/// The road in's deed, under the price (#448): "Claim with your deed",
/// asked twice, or where it waits.
fn deed(ui: &mut egui::Ui, plot: &PlotInfo, view: &View, confirm: &mut Option<uuid::Uuid>) -> Option<Action> {
    if *confirm != Some(plot.id) {
        *confirm = None;
    }
    match view.deed {
        DeedOffer::None => None,
        DeedOffer::AtTheOffice => {
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new("Your deed waits at the Deeds Office, by the hall.")
                            .color(theme::GOLD_LIGHT)
                            .size(size::LABEL),
                    )
                    .wrap(),
                );
            });
            None
        }
        DeedOffer::Claim if confirm.is_some() => {
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Spend your deed on this lot?").color(theme::GOLD_LIGHT).size(size::BODY));
            });
            let mut action = None;
            ui.columns(2, |cols| {
                if theme::accent_button(&mut cols[0], "Not yet", DIM, true).clicked() {
                    *confirm = None;
                }
                if theme::accent_button(&mut cols[1], "Yes, claim it", GOLD, !view.claiming)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    *confirm = None;
                    action = Some(Action::ClaimWithDeed(plot.id));
                }
            });
            action
        }
        DeedOffer::Claim => {
            ui.add_space(6.0);
            let text = if view.claiming { "Claiming…" } else { "Claim with your deed" };
            if theme::accent_button(ui, text, GOLD, !view.claiming)
                .on_hover_text("The deed the road in earned: this lot for nothing, taxed as any deed is")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                *confirm = Some(plot.id);
            }
            None
        }
    }
}

/// The tax a plot of yours owes, how near that is to foreclosure, and the
/// button that pays it off as far as your strands reach.
fn debt(ui: &mut egui::Ui, plot: &PlotInfo, view: &View) -> Option<Action> {
    row(
        ui,
        "Tax owed",
        RichText::new(format!("{} Strands", plot.tax_owed)).color(SHORT),
    );
    if let Some(limit) = plot.forecloses_at.filter(|l| *l > 0) {
        let near = (plot.tax_owed as f32 / limit as f32).clamp(0.0, 1.0);
        ui.add_space(4.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(WIDTH, 4.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 2.0, theme::WELL);
        let mut filled = rect;
        filled.set_width(rect.width() * near);
        // Gold while there's time, red as the end nears
        painter.rect_filled(filled, 2.0, GOLD.lerp_to_gamma(SHORT, near));
        ui.add_space(3.0);
        ui.label(theme::dim(format!("Foreclosed at {limit} owed")).size(size::LABEL));
    }
    ui.add_space(8.0);
    let pay = view.strands.unwrap_or(0).clamp(0, plot.tax_owed);
    let (text, enabled) = match pay {
        _ if view.paying => ("Paying…".to_owned(), false),
        0 => ("No Strands to pay with".to_owned(), false),
        n if n == plot.tax_owed => (format!("Pay {n} Strands"), true),
        n => (format!("Pay {n} of it"), true),
    };
    theme::accent_button(ui, &text, GOLD, enabled)
        .on_hover_cursor(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::NotAllowed
        })
        .clicked()
        .then_some(Action::PayTax(plot.id, pay))
        .filter(|_| enabled)
}

/// A week's tax on an owned plot: a quarter of the debt that forecloses it
/// (`rules::FORECLOSE_AFTER_WEEKS`). `None` unowned or untaxed.
fn weekly_tax(plot: &PlotInfo) -> Option<i64> {
    let weeks = capitol_core::protocol::rules::FORECLOSE_AFTER_WEEKS;
    plot.forecloses_at.filter(|f| *f > 0 && plot.owner_id.is_some()).map(|f| f / weeks)
}

/// `--plot-panel-sample` (V3.5): an urban lot of yours with a building,
/// and what you hold.
pub struct Sample {
    pub plot: PlotInfo,
    pub pack: capitol_core::state::Container,
    pub strands: i64,
}

/// [`Sample`]: `next`, one floor up and the second's bill open with all it
/// takes in your pack; `short`, the same short of planks, ingots and
/// Strands; `going-up`, the second floor going up, 12 minutes to go.
pub fn sample(which: &str) -> Sample {
    use capitol_core::protocol::rest::{Bounds, FloorBill, SlotItem};
    use capitol_core::protocol::rules::floors;
    use capitol_core::protocol::ws::ItemCount;
    let me = uuid::Uuid::from_u128(0xca9);
    let now = capitol_core::chrono::Utc::now();
    let bill = |k: i32| {
        let cost = floors::cost(k);
        FloorBill {
            floor: k,
            permit: cost.permit,
            goods: cost.goods.iter().map(|(t, n)| ItemCount { item_type: (*t).into(), quantity: *n as i32 }).collect(),
            build_secs: floors::build_secs(k),
        }
    };
    let going_up = which == "going-up";
    let floors = if going_up { 2 } else { 1 };
    let plot = PlotInfo {
        guild: None,
        id: uuid::Uuid::from_u128(0x477),
        zone_id: "urban".into(),
        world_x: 0.0,
        world_y: 0.0,
        bounds: Bounds::new(0.0, 0.0, 90.0, 93.0),
        size_category: "small".into(),
        plot_type: "standard".into(),
        owner_id: Some(me),
        owner_name: Some("You".into()),
        claimed_at: None,
        assessed_value: if going_up { 2200 } else { 1400 },
        tax_owed: 0,
        forecloses_at: Some(80),
        deed_cost: Some(1000),
        station_count: 6,
        station_capacity: 4 * (1 + floors),
        terrain_id: Some("urb_sample".into()),
        corners: None,
        indoor_room: Some(4),
        yard_room: Some(0),
        owner_citizen: false,
        floor_ready_at: going_up.then(|| now + capitol_core::chrono::Duration::minutes(12)),
        floors,
        next_floor: Some(bill(floors + 1)),
    };
    let catalog = capitol_core::catalog::Catalog::fallback();
    let goods: &[(&str, i32)] = match which {
        "short" => &[("plank", 22), ("brick", 70), ("iron_nails", 15)],
        _ => &[("plank", 46), ("brick", 80), ("iron_nails", 20), ("iron_ingot", 9)],
    };
    let slots: Vec<SlotItem> = goods
        .iter()
        .enumerate()
        .map(|(i, (t, n))| SlotItem::new(uuid::Uuid::from_u128(0x4770 + i as u128), (*t).into(), catalog.item_name(t), 55, *n, i as i32))
        .collect();
    Sample {
        plot,
        pack: capitol_core::state::Container::new(uuid::Uuid::from_u128(0x2351), "player_inventory", 24, 6, &slots),
        strands: if which == "short" { 650 } else { 2400 },
    }
}

/// One label and value, spread across the panel (React's `.plot-info-row`).
fn row(ui: &mut egui::Ui, label: &str, value: RichText) {
    ui.horizontal(|ui| {
        ui.label(theme::dim(label).size(size::BODY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value.size(size::BODY));
        });
    });
}

/// The rule above the claim or ownership section.
fn section(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    theme::divider(ui);
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::protocol::rest::Bounds;
    use uuid::Uuid;

    fn plot(owner: Option<Uuid>) -> PlotInfo {
        PlotInfo {
            guild: None,
            id: Uuid::from_u128(9),
            zone_id: "suburban".into(),
            world_x: 0.0,
            world_y: 0.0,
            bounds: Bounds::new(0.0, 0.0, 10.0, 10.0),
            size_category: "large".into(),
            plot_type: "standard".into(),
            owner_id: owner,
            owner_name: owner.map(|_| "Ada".into()),
            claimed_at: None,
            assessed_value: 500,
            tax_owed: 0,
            forecloses_at: None,
            deed_cost: Some(500),
            station_count: 1,
            station_capacity: 9,
            terrain_id: None,
            corners: None,
            indoor_room: None,
            yard_room: None,
            owner_citizen: false,
            floor_ready_at: None,
            floors: 0,
            next_floor: None,
        }
    }

    /// Runs a few frames of the panel and returns the last frame's action
    /// and the panel's rect.
    fn run(view: &View, click_at: Option<egui::Pos2>) -> (Option<Action>, Option<egui::Rect>) {
        let ctx = theme::test_context();
        let mut panel = PlotPanel::default();
        let mut out = (None, None);
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
                raw.events.push(egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                });
                raw.events.push(egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                });
            }
            let _ = ctx.run(raw, |ctx| {
                theme::install(ctx);
                let a = panel.show(ctx, view, None);
                if a.is_some() {
                    out.0 = a;
                }
                out.1 = panel.rect;
            });
        }
        out
    }

    #[test]
    fn the_panel_sits_bottom_left_and_claims_with_the_button() {
        let p = plot(None);
        let view = View {
            plot: Some(&p),
            offer: Offer::Claimable { cost: 500 },
            strands: Some(800),
            claiming: false,
            paying: false,
            refused: None,
            deed: DeedOffer::None,
            hire: None,
            upward: None,
            building: false,
        };
        let (_, rect) = run(&view, None);
        let rect = rect.expect("drawn");
        assert!((rect.left() - 16.0).abs() < 1.0, "{rect:?}");
        assert!((720.0 - rect.bottom() - 20.0).abs() < 1.0, "{rect:?}");
        // The button spans the bottom of the panel, just above its margin.
        let button = egui::pos2(rect.center().x, rect.bottom() - 30.0);
        let (action, _) = run(&view, Some(button));
        assert_eq!(action, Some(Action::Claim(p.id)));
    }

    #[test]
    fn a_plot_you_cant_afford_doesnt_claim() {
        let p = plot(None);
        let view = View {
            plot: Some(&p),
            offer: Offer::TooDear {
                cost: 500,
                short: 100,
            },
            strands: Some(400),
            claiming: false,
            paying: false,
            refused: None,
            deed: DeedOffer::None,
            hire: None,
            upward: None,
            building: false,
        };
        let (_, rect) = run(&view, None);
        let rect = rect.unwrap();
        let (action, _) = run(
            &view,
            Some(egui::pos2(rect.center().x, rect.bottom() - 30.0)),
        );
        assert_eq!(action, None);
    }

    #[test]
    fn a_plot_that_owes_tax_pays_what_your_strands_cover() {
        let me = Uuid::from_u128(1);
        let mut p = plot(Some(me));
        p.tax_owed = 120;
        p.forecloses_at = Some(480);
        let view = View {
            plot: Some(&p),
            offer: Offer::Yours,
            strands: Some(90),
            claiming: false,
            paying: false,
            refused: None,
            deed: DeedOffer::None,
            hire: None,
            upward: None,
            building: false,
        };
        let (_, rect) = run(&view, None);
        let rect = rect.unwrap();
        let button = egui::pos2(rect.center().x, rect.bottom() - 30.0);
        assert_eq!(run(&view, Some(button)).0, Some(Action::PayTax(p.id, 90)));

        let broke = View {
            strands: Some(0),
            ..view
        };
        assert_eq!(run(&broke, Some(button)).0, None, "nothing to pay with");
    }

    /// The road in's deed (#448): offered under the price even when the
    /// Strands fall short, and asked twice before it's spent
    #[test]
    fn a_free_lot_claims_with_the_deed_on_the_second_ask() {
        let p = plot(None);
        let view = View {
            plot: Some(&p),
            offer: Offer::TooDear { cost: 500, short: 400 },
            strands: Some(100),
            claiming: false,
            paying: false,
            refused: None,
            deed: DeedOffer::Claim,
            hire: None,
            upward: None,
            building: false,
        };
        let ctx = theme::test_context();
        let mut panel = PlotPanel::default();
        let mut frame = |click: Option<egui::Pos2>| {
            let mut out = (None, None);
            for f in 0..6 {
                let mut raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                    ..Default::default()
                };
                if let (Some(p), 3) = (click, f) {
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
                    if let Some(a) = panel.show(ctx, &view, None) {
                        out.0 = Some(a);
                    }
                    out.1 = panel.rect;
                });
            }
            out
        };
        let rect = frame(None).1.expect("drawn");
        // The deed's button is the panel's last, at its foot
        let at = egui::pos2(rect.center().x, rect.bottom() - 30.0);
        assert_eq!(frame(Some(at)).0, None, "the first click only asks");
        let rect = frame(None).1.unwrap();
        // "Yes, claim it" is the right of the two at the foot
        let yes = egui::pos2(rect.right() - 60.0, rect.bottom() - 30.0);
        assert_eq!(frame(Some(yes)).0, Some(Action::ClaimWithDeed(p.id)));
    }

    /// Build upward (V3.5): the bill opened, the button at the panel's foot
    /// builds, and only while you hold it all
    #[test]
    fn a_plot_of_yours_builds_upward_when_you_hold_the_bill() {
        let sample = sample("next");
        let catalog = capitol_core::catalog::Catalog::fallback();
        let me = sample.plot.owner_id;
        let now = capitol_core::chrono::Utc::now();
        let upward = capitol_core::plot::upward(&sample.plot, me, now, Some(sample.strands), Some(&sample.pack), &catalog);
        assert!(upward.as_ref().is_some_and(Upward::can_build), "{upward:?}");
        let view = View {
            plot: Some(&sample.plot),
            offer: Offer::Yours,
            strands: Some(sample.strands),
            claiming: false,
            paying: false,
            refused: None,
            deed: DeedOffer::None,
            hire: None,
            upward,
            building: false,
        };
        let ctx = theme::test_context();
        let mut panel = PlotPanel::default();
        panel.bill.open();
        let mut frame = |click: Option<egui::Pos2>| {
            let mut out = (None, None);
            for f in 0..6 {
                let mut raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                    ..Default::default()
                };
                if let (Some(p), 3) = (click, f) {
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
                    if let Some(a) = panel.show(ctx, &view, None) {
                        out.0 = Some(a);
                    }
                    out.1 = panel.rect;
                });
            }
            out
        };
        let rect = frame(None).1.expect("drawn");
        assert!(rect.top() >= 0.0, "{rect:?}");
        let at = egui::pos2(rect.center().x, rect.bottom() - 30.0);
        assert_eq!(frame(Some(at)).0, Some(Action::BuildFloor(sample.plot.id)));
        // Short of it: the button does nothing
        let short = super::sample("short");
        let upward = capitol_core::plot::upward(&short.plot, me, now, Some(short.strands), Some(&short.pack), &catalog);
        assert!(upward.as_ref().is_some_and(|u| !u.can_build()));
    }

    /// A floor going up says so, with no button
    #[test]
    fn a_floor_going_up_has_no_button() {
        let sample = sample("going-up");
        let catalog = capitol_core::catalog::Catalog::fallback();
        let now = capitol_core::chrono::Utc::now();
        let upward = capitol_core::plot::upward(&sample.plot, sample.plot.owner_id, now, Some(sample.strands), Some(&sample.pack), &catalog);
        assert!(matches!(upward, Some(Upward::GoingUp { floor: 2, .. })), "{upward:?}");
    }

    #[test]
    fn nothing_picked_draws_nothing() {
        let view = View {
            plot: None,
            offer: Offer::NotForSale,
            strands: None,
            claiming: false,
            paying: false,
            refused: None,
            deed: DeedOffer::None,
            hire: None,
            upward: None,
            building: false,
        };
        assert_eq!(run(&view, None), (None, None));
    }

    /// A week's tax is a quarter of the debt that forecloses the plot (#449)
    #[test]
    fn an_owned_plot_says_its_weeks_tax() {
        let mut p = plot(Some(Uuid::from_u128(1)));
        assert_eq!(weekly_tax(&p), None, "untaxed");
        p.forecloses_at = Some(40);
        assert_eq!(weekly_tax(&p), Some(10));
        p.owner_id = None;
        assert_eq!(weekly_tax(&p), None, "nobody's to tax");
    }

    /// A plot of yours offers gathering work on what grows there (S2.7),
    /// and the form opened still fits a 720-high screen.
    #[test]
    fn a_plot_of_yours_hires_gatherers() {
        let me = Uuid::from_u128(1);
        let p = plot(Some(me));
        let catalog = capitol_core::catalog::Catalog::fallback();
        let wages = Wages::default();
        let view = View {
            plot: Some(&p),
            offer: Offer::Yours,
            strands: Some(900),
            claiming: false,
            paying: false,
            refused: None,
            deed: DeedOffer::None,
            hire: Some(Hire {
                wages: &wages,
                me: Some(me),
                resources: vec![("grass".into(), "Wild Grass".into())],
                catalog: &catalog,
                now: capitol_core::chrono::Utc::now(),
                online: true,
            }),
            upward: None,
            building: false,
        };
        let ctx = theme::test_context();
        let mut panel = PlotPanel { hiring: true, ..Default::default() };
        for _ in 0..8 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                theme::install(ctx);
                assert_eq!(panel.show(ctx, &view, None), None);
            });
        }
        let draft = panel.hire.as_ref().expect("the form");
        assert_eq!(draft.work, Work::Gather { plot_id: p.id, resource: "grass".into() });
        let rect = panel.rect.expect("drawn");
        assert!(rect.top() >= 0.0, "{rect:?}");
    }
}
