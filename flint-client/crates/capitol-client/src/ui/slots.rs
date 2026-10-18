//! The slot grid (P5.3): any container drawn as React draws one
//! (`InventorySlot.tsx`, `InventoryGrid.tsx`), with drag and drop between
//! any two grids on screen.
//!
//! A slot shows its item's icon (36 in 48, P5.2), the quantity bottom right
//! and a quality pip top right in the grade's colour (plan §4.7). The pip
//! and the number are the mean of the stack's units, and every unit keeps
//! its own quality (plan D2, E1.8). A stack that spans more than one grade
//! rings its pip in its lowest unit's colour. Hovering one shows the name,
//! the mean and its grade, and the makeup: "6 × 92 A, 3 × 81 B, 1 × 44 D",
//! or a bar of grade colours when there are more qualities than that.
//!
//! Dragging uses egui's drag-and-drop payload, so a drag can start in one
//! panel and end in another without either knowing about the other. A drop
//! on another slot comes back from [`SlotGrid::show`] as a [`Move`]. The
//! caller sends it (`Session::move_item`), and the server decides whether
//! it moves, merges or swaps. A drop onto a stack of the same type joins
//! the two whatever their grades, and nothing is diluted (plan D2).
//! Nothing changes here until the refetch that follows (`poll.rs`), so the
//! grid never shows a state the server didn't.
//!
//! **Splitting (E1.2, E1.8).** A shift-drag onto an empty slot of the same
//! grid opens a count field under the slot, set to half the stack: the
//! lowest units go. A stack of more than one quality also gets a row per
//! quality, each with its own count, to take out exactly those
//! ([`Move::qualities`]). Enter or Split sends the move with
//! [`Move::split`]; Escape or ✕ lets it go.
//! Anywhere else, a shift-drag is a plain drag: the server splits only
//! within a container ([#190](https://github.com/chrischaps/TheCapitol/issues/190)).
//! A capture can't hold Shift for egui (#189), so the rig below checks it.
//!
//! **Tools (E1.7)** don't stack, so in place of the quantity they carry a
//! thin wear bar along the bottom: pale steel while sound, ember below 20%
//! of their uses, where their work starts to suffer
//! (`rules::tools::retention`), and breathing below 5%, on the cliff.
//! Their tooltip adds the type and tier, the power, the uses left, what the
//! wear leaves of their work, and what they do to each resource they work.
//! Those need the catalog, which the game hands over once a frame
//! ([`know_catalog`]); without it the tooltip leaves the power out. A tool
//! that breaks shatters where it was last drawn ([`shatter`]).
//!
//! A grid made [`SlotGrid::placing`] also reports a right click on a
//! `*_kit` ([`SlotGrid::show_clicks`]), and its tooltip says so. That's
//! how a station is placed (P6.2), as React's `InventorySlot.tsx` does it.

use super::icons::ItemIcons;
use super::theme::{self, size, CRAFT, SLOT, SLOT_EDGE, SLOT_EDGE_FULL, SLOT_GAP, SLOT_SIZE};
use capitol_core::catalog::Catalog;
use capitol_core::protocol::rest::SlotItem;
use capitol_core::protocol::rules::quality_grade;
use capitol_core::protocol::rules::stacking::Units;
use capitol_core::protocol::rules::tools;
use capitol_core::state::Container;
use egui::epaint::Shadow;
use egui::{Color32, CursorIcon, Id, Painter, Pos2, Rect, Sense, Stroke, Ui};
use uuid::Uuid;

/// A drop: put `item_id` in `target_slot` of `target_container_id`. The
/// body of `move_item`, REST or WebSocket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub item_id: Uuid,
    pub target_container_id: Uuid,
    pub target_slot: i32,
    /// Split this many off into the slot, an empty one of the same
    /// container, rather than moving the whole stack (`split_stack`, E1.2).
    pub split: Option<i32>,
    /// Which units the split takes, adding up to `split` (E1.8). `None`
    /// takes the lowest.
    pub qualities: Option<Units>,
}

impl Move {
    /// The whole stack `item_id` into `target_slot` of `target_container_id`.
    pub fn whole(item_id: Uuid, target_container_id: Uuid, target_slot: i32) -> Self {
        Self {
            item_id,
            target_container_id,
            target_slot,
            split: None,
            qualities: None,
        }
    }
}

/// What a drag carries: the item, where it came from, and whether shift
/// was down as it started (a split).
#[derive(Debug, Clone)]
struct Held {
    item: SlotItem,
    container_id: Uuid,
    slot: usize,
    split: bool,
}

/// A split waiting on its count: the stack, where it goes, and how many.
/// One at a time, in egui's memory, drawn by the grid it's for.
#[derive(Debug, Clone)]
struct PendingSplit {
    item: SlotItem,
    container_id: Uuid,
    slot: usize,
    quantity: i32,
    /// A row per quality in the stack, high first, when there's more than
    /// one: (quality, how many to take)
    picks: Vec<(i32, i32)>,
    /// The target slot, to stand the field under
    at: Rect,
}

impl PendingSplit {
    fn id() -> Id {
        Id::new("capitol_pending_split")
    }

    fn get(ctx: &egui::Context) -> Option<Self> {
        ctx.data(|d| d.get_temp(Self::id()))
    }

    fn set(ctx: &egui::Context, pending: Option<Self>) {
        ctx.data_mut(|d| match pending {
            Some(p) => d.insert_temp(Self::id(), p),
            None => d.remove::<Self>(Self::id()),
        });
    }
}

/// Hands the slots this frame's catalog, for what a tool's tooltip says.
/// Call it once a frame, before the panels; it's an `Arc`, so cheap.
pub fn know_catalog(ctx: &egui::Context, catalog: &Catalog) {
    ctx.data_mut(|d| d.insert_temp(Id::new("capitol_slot_catalog"), catalog.clone()));
}

fn catalog(ctx: &egui::Context) -> Option<Catalog> {
    ctx.data(|d| d.get_temp(Id::new("capitol_slot_catalog")))
}

/// What a split starts at: half, rounded down, and at least one.
pub fn half(quantity: i32) -> i32 {
    (quantity / 2).max(1)
}

/// A split field's quality rows for `units`, high first, all at nought; none
/// for a stack of one quality, which a count says all about.
fn pick_rows(units: &Units) -> Vec<(i32, i32)> {
    if units.pairs().nth(1).is_none() {
        return Vec::new();
    }
    units.pairs().rev().map(|(q, _)| (q, 0)).collect()
}

/// The units the quality rows pick, if any row picks one.
fn picked(picks: &[(i32, i32)]) -> Option<Units> {
    let units = Units::from_pairs(picks.iter().copied());
    (!units.is_empty()).then_some(units)
}

/// What a grid's slots were asked this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clicks {
    /// A drop onto one of them.
    pub moved: Option<Move>,
    /// A right click on a kit, with [`SlotGrid::placing`].
    pub place: Option<SlotItem>,
}

/// One container as a grid of slots, `layout_columns` wide.
pub struct SlotGrid<'a> {
    container: &'a Container,
    enabled: bool,
    /// Set by [`SlotGrid::placing`]: the catalog that says what's a kit.
    placing: Option<&'a Catalog>,
}

impl<'a> SlotGrid<'a> {
    pub fn new(container: &'a Container) -> Self {
        Self {
            container,
            enabled: true,
            placing: None,
        }
    }

    /// A right click on a kit asks to place its station (the inventory's
    /// grid). A kit is what `catalog` pairs with a station type (#181). See
    /// [`SlotGrid::show_clicks`].
    pub fn placing(mut self, catalog: &'a Catalog) -> Self {
        self.placing = Some(catalog);
        self
    }

    /// A disabled grid is dimmed, and nothing can be dragged out of it or
    /// dropped into it: crafting's inputs while a craft runs, say.
    #[allow(dead_code)] // P5.5 is the first panel to disable one
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Draws the grid in its tray. Returns the move a drop onto it asks for.
    pub fn show(self, ui: &mut Ui, icons: &mut ItemIcons) -> Option<Move> {
        self.show_clicks(ui, icons).moved
    }

    /// Draws the grid in its tray. Returns the move a drop onto it asks
    /// for, and the kit a right click asks to place.
    pub fn show_clicks(self, ui: &mut Ui, icons: &mut ItemIcons) -> Clicks {
        let mut clicks = Clicks::default();
        theme::tray().show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(SLOT_GAP, SLOT_GAP);
            let columns = self.container.layout_columns.max(1);
            for row in self.container.slots.chunks(columns).enumerate() {
                let (r, items) = row;
                ui.horizontal(|ui| {
                    for (c, item) in items.iter().enumerate() {
                        let index = r * columns + c;
                        let slot = Slot {
                            container_id: self.container.id,
                            index,
                            item: item.as_ref(),
                            enabled: self.enabled,
                        };
                        let (moved, right) = slot.show_inner(ui, icons, self.placing);
                        clicks.moved = clicks.moved.take().or(moved);
                        if let Some(catalog) = self.placing.filter(|_| right) {
                            clicks.place = item.clone().filter(|i| is_kit(catalog, i));
                        }
                    }
                });
            }
        });
        if self.enabled {
            clicks.moved = clicks.moved.take().or(split_field(ui.ctx(), self.container.id));
        }
        clicks
    }
}

/// The count field of a split waiting on this container, if there is one.
/// Returns the split once it's confirmed.
fn split_field(ctx: &egui::Context, container_id: Uuid) -> Option<Move> {
    let mut pending = PendingSplit::get(ctx).filter(|p| p.container_id == container_id)?;
    let most = pending.item.quantity - 1;
    let (mut confirmed, mut dropped) = (false, false);
    egui::Area::new(Id::new(("capitol_split_field", container_id)))
        .order(egui::Order::Foreground)
        .fixed_pos(pending.at.left_bottom() + egui::vec2(0.0, 4.0))
        .show(ctx, |ui| {
            theme::panel().inner_margin(6.0).show(ui, |ui| {
                let by_quality = picked(&pending.picks).is_some();
                ui.horizontal(|ui| {
                    ui.label(theme::dim("Split"));
                    // Picked by quality, the count is theirs to add up
                    let field = ui.add_enabled(
                        !by_quality,
                        egui::DragValue::new(&mut pending.quantity)
                            .range(1..=most)
                            .speed(0.1),
                    );
                    if !by_quality && !field.has_focus() && !field.lost_focus() && !field.dragged() {
                        field.request_focus();
                    }
                    ui.label(theme::dim(format!("of {}", pending.item.quantity)));
                    let fits = (1..=most).contains(&pending.quantity);
                    confirmed = theme::button(ui, "Split", true).clicked() && fits;
                    dropped = ui.small_button("✕").clicked();
                });
                if !pending.picks.is_empty() {
                    ui.label(theme::dim(if by_quality { "Taking these" } else { "The lowest go, or pick" }));
                    for (quality, take) in pending.picks.iter_mut() {
                        let have = pending.item.units.count_at(*quality);
                        ui.horizontal(|ui| {
                            theme::quality_badge(ui, *quality);
                            ui.add(egui::DragValue::new(take).range(0..=have).speed(0.1));
                            ui.label(theme::dim(format!("of {have}")));
                        });
                    }
                }
            });
        });
    if let Some(units) = picked(&pending.picks) {
        pending.quantity = units.count() as i32;
    }
    let (enter, escape) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    let qualities = picked(&pending.picks);
    if qualities.is_none() {
        pending.quantity = pending.quantity.clamp(1, most);
    }
    // Every unit picked would leave nothing behind: not a split
    let fits = (1..=most).contains(&pending.quantity);
    if (confirmed || enter) && fits {
        PendingSplit::set(ctx, None);
        return Some(Move {
            item_id: pending.item.item_id,
            target_container_id: container_id,
            target_slot: pending.slot as i32,
            split: Some(pending.quantity),
            qualities,
        });
    }
    PendingSplit::set(ctx, (!(dropped || escape)).then_some(pending));
    None
}

/// The item a slot's drag let go of over `response` this frame, if any:
/// how a panel that isn't a container takes a drop (the trade window, P7.3).
pub fn dropped_item(response: &egui::Response) -> Option<SlotItem> {
    response
        .dnd_release_payload::<Held>()
        .map(|held| held.item.clone())
}

/// A slot's item is being dragged over `response`.
pub fn item_over(response: &egui::Response) -> bool {
    response.dnd_hover_payload::<Held>().is_some()
}

/// A slot's item is being dragged anywhere.
pub fn dragging_item(ctx: &egui::Context) -> bool {
    egui::DragAndDrop::has_payload_of_type::<Held>(ctx)
}

/// A station kit: a right click on one places its station.
fn is_kit(catalog: &Catalog, item: &SlotItem) -> bool {
    catalog.station_for_kit(&item.item_type).is_some()
}

/// One slot of a container. Grids are made of these; a panel with a lone
/// slot (crafting's output) can use one directly.
pub struct Slot<'a> {
    pub container_id: Uuid,
    pub index: usize,
    pub item: Option<&'a SlotItem>,
    pub enabled: bool,
}

impl Slot<'_> {
    pub fn show(self, ui: &mut Ui, icons: &mut ItemIcons) -> Option<Move> {
        self.show_inner(ui, icons, None).0
    }

    /// The drop onto this slot, and whether it was right-clicked. `placing`
    /// adds the kit's hint to the tooltip.
    fn show_inner(
        self,
        ui: &mut Ui,
        icons: &mut ItemIcons,
        placing: Option<&Catalog>,
    ) -> (Option<Move>, bool) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(SLOT_SIZE, SLOT_SIZE), Sense::hover());
        let id = Id::new(("capitol_slot", self.container_id, self.index));
        let draggable = self.enabled && self.item.is_some();
        let sense = if draggable {
            Sense::click_and_drag()
        } else {
            Sense::click()
        };
        let response = ui.interact(rect, id, sense);

        if let (true, Some(item)) = (draggable, self.item) {
            // Read as the drag starts: egui keeps the payload it's given then.
            let shift = ui.input(|i| i.modifiers.shift);
            response.dnd_set_drag_payload(Held {
                item: item.clone(),
                container_id: self.container_id,
                slot: self.index,
                split: shift && item.quantity > 1,
            });
        }
        let held = egui::DragAndDrop::payload::<Held>(ui.ctx());
        let is_source = held
            .as_ref()
            .is_some_and(|h| h.container_id == self.container_id && h.slot == self.index);
        let accepts = self.enabled && !is_source;
        let over = accepts && response.dnd_hover_payload::<Held>().is_some();

        let look = if !self.enabled {
            Look::Disabled
        } else if is_source {
            Look::Source
        } else if over {
            Look::Target
        } else if response.hovered() && self.item.is_some() {
            Look::Hovered
        } else {
            Look::Plain
        };
        paint_slot(ui.painter(), icons, rect, self.item, look);
        if let Some(tool) = self.item.filter(|i| i.durability.is_some()) {
            remember_tool(ui.ctx(), tool.item_id, rect);
        }
        let waiting = PendingSplit::get(ui.ctx())
            .is_some_and(|p| p.container_id == self.container_id && p.slot == self.index);
        if waiting {
            dashed_edge(
                ui.painter(),
                rect.shrink(1.0),
                Stroke::new(2.0, CRAFT.gamma_multiply(0.8)),
            );
        }

        if let Some(item) = self.item {
            if held.is_none() {
                let response = if draggable {
                    response.clone().on_hover_cursor(CursorIcon::Grab)
                } else {
                    response.clone()
                };
                response.on_hover_ui(|ui| {
                    tooltip(ui, item);
                    if placing.is_some_and(|c| is_kit(c, item)) {
                        ui.label(theme::dim("Right-click to place"));
                    }
                    if draggable && item.quantity > 1 {
                        ui.label(theme::dim("Shift-drag to an empty slot to split"));
                    }
                });
            }
        }
        let right = self.item.is_some() && response.secondary_clicked();
        if is_source && response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            if let (Some(item), Some(at)) = (self.item, ui.ctx().pointer_interact_pos()) {
                let splitting = held.as_ref().is_some_and(|h| h.split);
                let shown = if splitting { half(item.quantity) } else { item.quantity };
                paint_held(ui.ctx(), icons, item, shown, at);
            }
        }

        if !accepts {
            return (None, right);
        }
        let moved = response.dnd_release_payload::<Held>().and_then(|held| {
            if held.split && held.container_id == self.container_id && self.item.is_none() {
                // Ask how many first: the grid draws the field (`split_field`)
                PendingSplit::set(
                    ui.ctx(),
                    Some(PendingSplit {
                        quantity: half(held.item.quantity),
                        picks: pick_rows(&held.item.units),
                        item: held.item.clone(),
                        container_id: self.container_id,
                        slot: self.index,
                        at: rect,
                    }),
                );
                return None;
            }
            Some(Move::whole(
                held.item.item_id,
                self.container_id,
                self.index as i32,
            ))
        });
        (moved, right)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Plain,
    Hovered,
    /// Being dragged from: faded, with a dashed edge.
    Source,
    /// A drag is over it: lit in purple.
    Target,
    Disabled,
}

fn paint_slot(
    painter: &Painter,
    icons: &mut ItemIcons,
    rect: Rect,
    item: Option<&SlotItem>,
    look: Look,
) {
    let fade = match look {
        Look::Source => 0.4,
        Look::Disabled => 0.5,
        _ => 1.0,
    };
    let inner = rect.shrink(1.0); // a 2 px edge, drawn centred on this
    let (fill, edge) = match (look, item.is_some()) {
        (Look::Target, _) => (CRAFT.gamma_multiply(0.15), CRAFT.gamma_multiply(0.8)),
        (Look::Hovered, _) => (theme::SLOT_HOVER, CRAFT.gamma_multiply(0.6)),
        (_, true) => (SLOT, SLOT_EDGE_FULL),
        (_, false) => (SLOT, SLOT_EDGE),
    };
    if look == Look::Target {
        // React's `box-shadow: 0 0 8px rgba(139,92,246,0.4)`.
        let glow = Shadow {
            offset: egui::Vec2::ZERO,
            blur: 8.0,
            spread: 0.0,
            color: CRAFT.gamma_multiply(0.4),
        };
        painter.add(glow.as_shape(inner, 6.0));
    }
    if look == Look::Source {
        painter.rect_filled(inner, 6.0, SLOT.gamma_multiply(fade));
        dashed_edge(painter, inner, Stroke::new(2.0, edge.gamma_multiply(fade)));
    } else {
        painter.rect(
            inner,
            6.0,
            fill.gamma_multiply(fade),
            Stroke::new(2.0, edge.gamma_multiply(fade)),
        );
    }
    let Some(item) = item else {
        return;
    };
    let icon = Rect::from_center_size(
        rect.center(),
        egui::vec2(theme::SLOT_ICON, theme::SLOT_ICON),
    );
    icons.paint_on(painter, icon, &item.item_type, fade);
    if quality_grade(item.quality) == "A" && look != Look::Source {
        let time = painter.ctx().input(|i| i.time);
        glint(painter, icon, item.item_id.as_u128(), time, fade);
    }
    // React's pip: 8 px, 3 px in from the inside of the 2 px edge.
    pip(painter, rect.right_top() + egui::vec2(-9.0, 9.0), item, fade);
    if let (Some(left), Some(max)) = (item.durability, item.max_durability) {
        wear_bar(painter, rect, left, max, fade);
    } else {
        quantity(
            painter,
            rect.right_bottom() + egui::vec2(-6.0, -3.0),
            item.quantity,
            fade,
        );
    }
}

/// How a tool's wear reads: sound, ember (its work is suffering), or on
/// the cliff (about to break).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wear {
    Sound,
    Ember,
    Cliff,
}

impl Wear {
    /// The rules' knees: below 20% left a tool's work suffers, below 5% it
    /// falls off the cliff (`rules::tools::retention`).
    pub fn of(left: i32, max: i32) -> Self {
        let share = left as f64 / max.max(1) as f64;
        if share < tools::CLIFF_EDGE {
            Wear::Cliff
        } else if share < tools::PLATEAU_FLOOR {
            Wear::Ember
        } else {
            Wear::Sound
        }
    }
}

/// How fast a tool on the cliff breathes, in breaths a second.
const CLIFF_PULSE_HZ: f64 = 1.6;

/// A tool's wear bar, 3 px along the slot's bottom, inside the edge: the
/// share of its uses left, in steel, then ember, then breathing to white
/// heat on the cliff, with a glow.
pub fn wear_bar(painter: &Painter, slot: Rect, left: i32, max: i32, fade: f32) {
    let track = Rect::from_min_max(
        egui::pos2(slot.left() + 6.0, slot.bottom() - 8.0),
        egui::pos2(slot.right() - 6.0, slot.bottom() - 5.0),
    );
    painter.rect_filled(track.expand(0.5), 2.0, theme::WEAR_TRACK.gamma_multiply(fade));
    let share = (left as f32 / max.max(1) as f32).clamp(0.0, 1.0);
    // Never quite empty while a use remains: a sliver still shows.
    let width = (track.width() * share).max(if left > 0 { 2.0 } else { 0.0 });
    let fill = Rect::from_min_size(track.min, egui::vec2(width, track.height()));
    let colour = match Wear::of(left, max) {
        Wear::Sound => theme::WEAR,
        Wear::Ember => theme::EMBER,
        Wear::Cliff => {
            let time = painter.ctx().input(|i| i.time);
            painter.ctx().request_repaint();
            let breath = (0.5 - 0.5 * (time * CLIFF_PULSE_HZ * std::f64::consts::TAU).cos()) as f32;
            let glow = Shadow {
                offset: egui::Vec2::ZERO,
                blur: 6.0,
                spread: 1.0,
                color: theme::EMBER.gamma_multiply(0.7 * breath * fade),
            };
            painter.add(glow.as_shape(fill, 2.0));
            lerp(theme::EMBER, theme::EMBER_HOT, breath)
        }
    };
    painter.rect_filled(fill, 1.5, colour.gamma_multiply(fade));
}

/// Where each tool was last drawn, so one that breaks can shatter there.
fn remember_tool(ctx: &egui::Context, item_id: Uuid, rect: Rect) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<ToolRects>(Id::new("capitol_tool_rects"))
            .0
            .insert(item_id, rect);
    });
}

#[derive(Debug, Clone, Default)]
struct ToolRects(std::collections::HashMap<Uuid, Rect>);

/// A broken tool's shards, flying from where it was.
#[derive(Debug, Clone)]
struct Shatter {
    rect: Rect,
    item_id: Uuid,
    since: f64,
}

#[derive(Debug, Clone, Default)]
struct Shatters(Vec<Shatter>);

/// How long a shatter lasts, in seconds.
const SHATTER_LIFE: f64 = 0.9;

/// A tool broke (`tool_broke`): its slot shatters where it was last drawn
/// (P8.2's juice). Nothing happens for one that was never on screen.
pub fn shatter(ctx: &egui::Context, item_id: Uuid) {
    let at = ctx.data(|d| {
        d.get_temp::<ToolRects>(Id::new("capitol_tool_rects"))
            .and_then(|r| r.0.get(&item_id).copied())
    });
    let Some(rect) = at else {
        return;
    };
    let since = ctx.input(|i| i.time);
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Shatters>(Id::new("capitol_shatters"))
            .0
            .push(Shatter { rect, item_id, since });
    });
}

/// Draws the shatters under way, over every panel. Call once a frame,
/// after the panels.
pub fn paint_shatters(ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let mut live = ctx.data_mut(|d| {
        std::mem::take(&mut d.get_temp_mut_or_default::<Shatters>(Id::new("capitol_shatters")).0)
    });
    live.retain(|s| now - s.since < SHATTER_LIFE);
    if live.is_empty() {
        return;
    }
    ctx.request_repaint();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        Id::new("capitol_shatters"),
    ));
    for s in &live {
        paint_shards(&painter, s, ((now - s.since) / SHATTER_LIFE) as f32);
    }
    ctx.data_mut(|d| d.insert_temp(Id::new("capitol_shatters"), Shatters(live)));
}

/// A shatter at `k` (0 to 1): a ring of ember heat thrown off the slot, then a dozen
/// shards of steel and ember thrown out from its middle, turning, falling a
/// little and fading. Each tool's shards are its own, seeded from its id.
fn paint_shards(painter: &Painter, s: &Shatter, k: f32) {
    let flash = (1.0 - k * 2.5).max(0.0);
    if flash > 0.0 {
        let glow = Shadow {
            offset: egui::Vec2::ZERO,
            blur: 14.0,
            spread: 0.0,
            color: theme::EMBER.gamma_multiply(0.5 * flash),
        };
        painter.add(glow.as_shape(s.rect.shrink(6.0), 6.0));
        let ring = s.rect.expand(10.0 * (1.0 - flash));
        painter.rect_stroke(ring, 8.0, Stroke::new(2.0_f32, theme::EMBER_HOT.gamma_multiply(flash)));
    }
    let centre = s.rect.center();
    let mut seed = s.item_id.as_u128() as u64 | 1;
    let mut next = || {
        // xorshift: enough to scatter a dozen shards
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 10_000) as f32 / 10_000.0
    };
    let ease = 1.0 - (1.0 - k).powi(3);
    let fade = (1.0 - k).powf(1.5);
    for i in 0..12 {
        let angle = (i as f32 + next() * 0.8) / 12.0 * std::f32::consts::TAU;
        let reach = 18.0 + 22.0 * next();
        let dir = egui::vec2(angle.cos(), angle.sin());
        let at = centre + dir * reach * ease + egui::vec2(0.0, 14.0 * k * k);
        let size = 2.0 + 3.0 * next();
        let turn = angle + k * (3.0 + 4.0 * next());
        let u = egui::vec2(turn.cos(), turn.sin()) * size;
        let n = egui::vec2(-u.y, u.x) * 0.45;
        let colour = if i % 3 == 0 { theme::EMBER } else { theme::WEAR };
        painter.add(egui::Shape::convex_polygon(
            vec![at + u, at + n, at - u * 0.6, at - n],
            colour.gamma_multiply(fade),
            Stroke::NONE,
        ));
    }
}

/// The pip's colour for a quality: its grade's (plan §4.7). Worked out from
/// the quality, not the server's `quality_grade`, so a merge that the
/// client hasn't refetched yet can't show a stale letter.
pub fn pip_colour(quality: i32) -> Color32 {
    theme::grade(quality_grade(quality))
}

/// A grade pip: a 4-point disc, its grade's colour lightening towards the
/// bottom right (React's 135° gradient), ringed in dark. An A glows. A
/// stack that spans grades is ringed again, in its lowest unit's grade's
/// colour (E1.2, E1.8).
fn pip(painter: &Painter, centre: Pos2, item: &SlotItem, fade: f32) {
    let quality = item.quality;
    let letter = quality_grade(quality);
    let (dark, light) = (pip_colour(quality), theme::grade_light(letter));
    const R: f32 = 4.0;
    if letter == "A" {
        painter.circle_filled(centre, R + 2.0, dark.gamma_multiply(0.3 * fade));
    }
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, lerp(dark, light, 0.5).gamma_multiply(fade));
    const N: u32 = 20;
    for i in 0..N {
        let a = i as f32 / N as f32 * std::f32::consts::TAU;
        let d = egui::vec2(a.cos(), a.sin());
        // Projected onto the gradient's axis (down and right), 0 to 1.
        let t = ((d.x + d.y) / std::f32::consts::SQRT_2 + 1.0) / 2.0;
        mesh.colored_vertex(centre + d * R, lerp(dark, light, t).gamma_multiply(fade));
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % N);
    }
    painter.add(mesh);
    painter.circle_stroke(
        centre,
        R,
        Stroke::new(1.0, theme::PIP_EDGE.gamma_multiply(fade)),
    );
    if item.units.spans_grades() {
        painter.circle_stroke(
            centre,
            R + 1.75,
            Stroke::new(1.5, pip_colour(item.min_quality).gamma_multiply(fade)),
        );
    }
}

/// How often an A's slot glints, and how long a glint lasts (s).
const GLINT_EVERY: f64 = 4.5;
const GLINT_LIFE: f64 = 0.55;

/// An A-grade item's glint (P8.2): every few seconds a four-pointed star
/// swells and turns on the icon's upper left and is gone. Each item keeps
/// its own time, from its id, so a row of A's doesn't flash at once.
fn glint(painter: &Painter, icon: Rect, id: u128, time: f64, fade: f32) {
    let offset = (id % 997) as f64 / 997.0 * GLINT_EVERY;
    let age = (time + offset) % GLINT_EVERY;
    if age > GLINT_LIFE {
        return;
    }
    let k = (age / GLINT_LIFE) as f32;
    let swell = 1.0 - (2.0 * k - 1.0).abs();
    let centre = icon.left_top() + icon.size() * egui::vec2(0.3, 0.28);
    let r = 7.0 * swell;
    let w = r * 0.22;
    let colour = theme::grade_light("A").gamma_multiply(0.95 * fade * swell);
    let turn = k * 0.6;
    for i in 0..2 {
        let a = turn + i as f32 * std::f32::consts::FRAC_PI_2;
        let u = egui::vec2(a.cos(), a.sin());
        let n = egui::vec2(-u.y, u.x);
        painter.add(egui::Shape::convex_polygon(
            vec![
                centre + u * r,
                centre + n * w,
                centre - u * r,
                centre - n * w,
            ],
            colour,
            Stroke::NONE,
        ));
    }
    painter.circle_filled(centre, w * 1.2, Color32::WHITE.gamma_multiply(fade * swell));
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}

/// The stack size, bottom right, in white over a dark shadow.
fn quantity(painter: &Painter, corner: Pos2, quantity: i32, fade: f32) {
    let font = theme::strong(size::LABEL);
    let text = quantity.to_string();
    painter.text(
        corner + egui::vec2(1.0, 1.0),
        egui::Align2::RIGHT_BOTTOM,
        &text,
        font.clone(),
        theme::INK_SHADOW.gamma_multiply(fade),
    );
    painter.text(
        corner,
        egui::Align2::RIGHT_BOTTOM,
        text,
        font,
        theme::QUANTITY.gamma_multiply(fade),
    );
}

/// A slot's edge as dashes, for the slot a drag came from.
fn dashed_edge(painter: &Painter, rect: Rect, stroke: Stroke) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(&corners, stroke, 5.0, 3.0));
}

/// The item under the pointer while it's dragged, over everything.
fn paint_held(ctx: &egui::Context, icons: &mut ItemIcons, item: &SlotItem, shown: i32, at: Pos2) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        Id::new("capitol_slot_held"),
    ));
    let rect = Rect::from_center_size(at, egui::vec2(SLOT_SIZE, SLOT_SIZE));
    let icon = Rect::from_center_size(at, egui::vec2(theme::SLOT_ICON, theme::SLOT_ICON));
    icons.paint_on(&painter, icon, &item.item_type, 0.9);
    pip(&painter, rect.right_top() + egui::vec2(-9.0, 9.0), item, 0.9);
    if let (Some(left), Some(max)) = (item.durability, item.max_durability) {
        wear_bar(&painter, rect, left, max, 0.9);
    } else {
        quantity(
            &painter,
            rect.right_bottom() + egui::vec2(-6.0, -3.0),
            shown,
            0.9,
        );
    }
}

/// The most qualities a tooltip lists by name; a stack of more shows a bar.
const MAKEUP_ROWS: usize = 4;

/// A stack's units by quality, high first: "6 × 92 A, 3 × 81 B, 1 × 44 D".
pub fn makeup(units: &Units) -> String {
    units
        .pairs()
        .rev()
        .map(|(q, n)| format!("{n} × {q} {}", quality_grade(q)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A stack's units by grade, high first: "6 A · 3 B · 1 D".
pub fn makeup_by_grade(units: &Units) -> String {
    let mut grades: Vec<(&str, i64)> = Vec::new();
    for (q, n) in units.pairs().rev() {
        let letter = quality_grade(q);
        match grades.last_mut() {
            Some((g, total)) if *g == letter => *total += n as i64,
            _ => grades.push((letter, n as i64)),
        }
    }
    grades
        .iter()
        .map(|(g, n)| format!("{n} {g}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A bar of the units, high first, each quality's share of the width in
/// its grade's colour.
fn makeup_bar(ui: &mut Ui, units: &Units) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(140.0, 6.0), Sense::hover());
    let painter = ui.painter();
    let total = units.count().max(1) as f32;
    let mut x = rect.left();
    for (q, n) in units.pairs().rev() {
        let w = rect.width() * n as f32 / total;
        let part = Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(w, rect.height()));
        painter.rect_filled(part, 0.0, pip_colour(q));
        x += w;
    }
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, theme::PIP_EDGE));
}

/// What hovering a full slot says: the name, the mean quality and its
/// grade, how many, and for a stack of more than one quality its makeup.
fn tooltip(ui: &mut Ui, item: &SlotItem) {
    ui.label(
        egui::RichText::new(&item.item_name)
            .font(theme::strong(size::BODY))
            .color(theme::TEXT),
    );
    ui.horizontal(|ui| {
        ui.label(theme::dim("Quality"));
        theme::quality_badge(ui, item.quality);
        if item.quantity > 1 {
            ui.label(theme::dim(format!("× {}", item.quantity)));
        }
    });
    let qualities = item.units.pairs().count();
    if qualities > MAKEUP_ROWS {
        makeup_bar(ui, &item.units);
        ui.label(theme::dim(makeup_by_grade(&item.units)));
    } else if qualities > 1 {
        ui.label(theme::dim(makeup(&item.units)));
    }
    if let (Some(left), Some(max)) = (item.durability, item.max_durability) {
        let catalog = catalog(ui.ctx());
        tool_lines(ui, catalog.as_ref(), &item.item_type, item.quality, left, max);
    }
}

/// A tool's numbers, every one the server works with (E1.7): its type and
/// tier, its power (its material's, scaled by quality), the uses left,
/// what its wear leaves of its work, and for each resource it works, the
/// yield, speed and the most quality it can knock off. Without the catalog
/// only the uses and the wear.
pub fn tool_lines(
    ui: &mut Ui,
    catalog: Option<&Catalog>,
    item_type: &str,
    quality: i32,
    left: i32,
    max: i32,
) {
    let spec = catalog.and_then(|c| c.item_type(item_type)?.tool.clone());
    let retention = tools::retention(left, max);
    let wear = Wear::of(left, max);
    let tone = match wear {
        Wear::Sound => theme::TEXT,
        _ => theme::EMBER,
    };
    egui::Grid::new(("capitol_tool_lines", item_type))
        .num_columns(2)
        .spacing([10.0, 1.0])
        .show(ui, |ui| {
            if let Some(spec) = &spec {
                ui.label(theme::dim("Tool"));
                ui.label(theme::text(format!(
                    "{} {}",
                    capitalised(&spec.tier),
                    spec.tool_type
                )));
                ui.end_row();
                ui.label(theme::dim("Power"));
                ui.label(theme::text(format!(
                    "{:.1}  ({:.0} × {:.2} for quality)",
                    tools::effective_power(spec.base_power, quality),
                    spec.base_power,
                    tools::quality_multiplier(quality),
                )));
                ui.end_row();
            }
            ui.label(theme::dim("Uses left"));
            ui.label(egui::RichText::new(format!("{left} of {max}")).color(tone).font(theme::strong(size::BODY)));
            ui.end_row();
            ui.label(theme::dim("Wear"));
            let state = match wear {
                Wear::Sound => "none yet: works at full quality".to_owned(),
                Wear::Ember => format!("worn: keeps {:.0}% of quality", retention * 100.0),
                Wear::Cliff => format!("about to break: keeps {:.0}%", retention * 100.0),
            };
            ui.label(egui::RichText::new(state).color(tone));
            ui.end_row();
        });
    let (Some(catalog), Some(spec)) = (catalog, spec) else {
        return;
    };
    let power = tools::effective_power(spec.base_power, quality);
    for resource in catalog
        .resource_types()
        .iter()
        .filter(|r| r.tool_type.as_deref() == Some(spec.tool_type.as_str()))
    {
        let difficulty = resource.difficulty.unwrap_or(0.0);
        let leverage = tools::leverage(power, difficulty);
        ui.label(
            theme::dim(format!(
                "{}: ×{:.2} yield, {:.0}% faster, up to −{} quality",
                resource.name,
                1.0 + tools::YIELD_PER_LEVERAGE * leverage,
                tools::SPEED_PER_LEVERAGE * leverage * 100.0,
                tools::max_damage(power, difficulty),
            ))
            .size(size::LABEL),
        );
    }
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pips_change_colour_at_the_grade_boundaries() {
        let (a, b, c, d, f) = (
            theme::grade("A"),
            theme::grade("B"),
            theme::grade("C"),
            theme::grade("D"),
            theme::grade("F"),
        );
        for (quality, colour) in [
            (100, a),
            (90, a),
            (89, b),
            (75, b),
            (74, c),
            (55, c),
            (54, d),
            (35, d),
            (34, f),
            (0, f),
        ] {
            assert_eq!(pip_colour(quality), colour, "quality {quality}");
        }
        // The plan's palette (§4.7).
        assert_eq!(a, Color32::from_rgb(0xff, 0xd7, 0x00));
        assert_eq!(b, Color32::from_rgb(0x22, 0xc5, 0x5e));
        assert_eq!(c, Color32::from_rgb(0x3b, 0x82, 0xf6));
        assert_eq!(d, Color32::from_rgb(0xa8, 0x55, 0xf7));
        assert_eq!(f, Color32::from_rgb(0x6b, 0x72, 0x80));
    }

    /// The wear bar turns at the rules' knees: ember below 20% of a tool's
    /// uses, the cliff below 5% (E1.7)
    #[test]
    fn wear_turns_at_the_rules_knees() {
        assert_eq!(Wear::of(60, 60), Wear::Sound);
        assert_eq!(Wear::of(12, 60), Wear::Sound, "20% is still the plateau");
        assert_eq!(Wear::of(11, 60), Wear::Ember);
        assert_eq!(Wear::of(3, 60), Wear::Ember, "5% is still the decline");
        assert_eq!(Wear::of(2, 60), Wear::Cliff);
        assert_eq!(Wear::of(0, 60), Wear::Cliff);
        // Where the work starts to suffer is where the bar turns.
        assert_eq!(tools::retention(12, 60), 1.0);
        assert!(tools::retention(11, 60) < 1.0);
    }

    /// A tool's tooltip and a shattered slot draw without panicking, with
    /// and without the catalog, and a shatter lasts its life
    #[test]
    fn a_tool_tooltip_and_a_shatter_draw() {
        let ctx = theme::test_context();
        let sickle = item(0, "stone_sickle", 1).with_durability(3, 81);
        for know in [false, true] {
            let _ = ctx.run(Default::default(), |ctx| {
                if know {
                    know_catalog(ctx, &Catalog::fallback());
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    tooltip(ui, &sickle);
                    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(48.0, 48.0));
                    remember_tool(ctx, sickle.item_id, rect);
                });
                shatter(ctx, sickle.item_id);
                paint_shatters(ctx);
            });
        }
        let live = |ctx: &egui::Context| {
            ctx.data(|d| d.get_temp::<Shatters>(Id::new("capitol_shatters")))
                .map_or(0, |s| s.0.len())
        };
        // egui may run a frame twice to settle a layout, shattering twice
        assert!(live(&ctx) >= 2);
        let later = egui::RawInput {
            time: Some(SHATTER_LIFE + 1.0),
            ..Default::default()
        };
        let _ = ctx.run(later, paint_shatters);
        assert_eq!(live(&ctx), 0, "gone once its life is over");
    }

    // ---- drag and drop, driven through a headless egui ----
    //
    // A workaround: a capture can't play the mouse (#76), so the drag is
    // tested here, not in a screenshot.

    /// A fresh ID (the client's uuid has no `v4` feature).
    fn id() -> Uuid {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Uuid::from_u128(NEXT.fetch_add(1, Ordering::Relaxed) as u128)
    }

    fn item(slot: i32, item_type: &str, quantity: i32) -> SlotItem {
        SlotItem::new(id(), item_type.into(), item_type.into(), 80, quantity, slot)
    }

    /// An inventory (6 × 2) holding fiber in slot 0 and rope in slot 3, and
    /// a crafting input (2 × 2) holding fiber in slot 1.
    fn containers() -> (Container, Container) {
        let inventory = Container::new(
            id(),
            "player_inventory",
            12,
            6,
            &[
                item(0, "fiber", 12),
                item(3, "rope", 1),
                item(10, "forge_kit", 1),
            ],
        );
        let input = Container::new(id(), "crafting_input", 4, 2, &[item(1, "fiber", 3)]);
        (inventory, input)
    }

    struct Rig {
        ctx: egui::Context,
        icons: ItemIcons,
        inventory: Container,
        input: Container,
        /// Every slot's rect, by (container, index), from the last frame.
        rects: std::collections::HashMap<(Uuid, usize), Rect>,
        moves: Vec<Move>,
        /// Kits a right click asked to place.
        places: Vec<SlotItem>,
        /// Inventory disabled: nothing goes in or out.
        inventory_enabled: bool,
        /// Shift held, for every frame until it's let go.
        shift: bool,
    }

    impl Rig {
        fn new() -> Self {
            let (inventory, input) = containers();
            let mut rig = Rig {
                ctx: theme::test_context(),
                icons: ItemIcons::load(std::path::Path::new("no/icons/needed")),
                inventory,
                input,
                rects: Default::default(),
                moves: Vec::new(),
                places: Vec::new(),
                inventory_enabled: true,
                shift: false,
            };
            rig.frame(vec![]);
            rig.frame(vec![]); // settle the layout before the first click
            rig
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0))),
                events,
                modifiers: egui::Modifiers {
                    shift: self.shift,
                    ..Default::default()
                },
                ..Default::default()
            };
            let Rig {
                ctx,
                icons,
                inventory,
                input: crafting,
                rects,
                moves,
                places,
                inventory_enabled,
                ..
            } = self;
            let _ = ctx.run(input, |ctx| {
                theme::install(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    // The inventory places kits, as its panel's grid does.
                    let catalog = Catalog::fallback();
                    let grids = [
                        (&*inventory, *inventory_enabled, true),
                        (&*crafting, true, false),
                    ];
                    for (c, enabled, placing) in grids {
                        let mut grid = SlotGrid::new(c).enabled(enabled);
                        if placing {
                            grid = grid.placing(&catalog);
                        }
                        let clicks = grid.show_clicks(ui, icons);
                        moves.extend(clicks.moved);
                        places.extend(clicks.place);
                    }
                });
                // Where each slot landed, read back from its interact rect.
                for c in [&*inventory, &*crafting] {
                    for i in 0..c.slot_count() {
                        let id = Id::new(("capitol_slot", c.id, i));
                        if let Some(r) = ctx.read_response(id) {
                            rects.insert((c.id, i), r.rect);
                        }
                    }
                }
            });
        }

        fn centre(&self, container: Uuid, slot: usize) -> Pos2 {
            self.rects[&(container, slot)].center()
        }

        /// Press on one slot, move across in steps, release on another.
        fn drag(&mut self, from: (Uuid, usize), to: (Uuid, usize)) {
            let (a, b) = (self.centre(from.0, from.1), self.centre(to.0, to.1));
            self.frame(vec![egui::Event::PointerMoved(a)]);
            self.frame(vec![button(a, true)]);
            for i in 1..=6 {
                let p = a + (b - a) * (i as f32 / 6.0);
                self.frame(vec![egui::Event::PointerMoved(p)]);
            }
            self.frame(vec![button(b, false)]);
            self.frame(vec![]);
        }

        /// A right click on a slot.
        fn right_click(&mut self, container: Uuid, slot: usize) {
            let at = self.centre(container, slot);
            self.frame(vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                self.frame(vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Default::default(),
                }]);
            }
            self.frame(vec![]);
        }
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn a_right_click_on_a_kit_asks_to_place_it() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        rig.right_click(inv, 3); // rope: not a kit
        rig.right_click(inv, 1); // empty
        assert!(rig.places.is_empty());
        rig.right_click(inv, 10);
        let kit = rig.inventory.slot(10).unwrap().clone();
        assert_eq!(rig.places, [kit]);
        assert!(rig.moves.is_empty(), "a right click moves nothing");
    }

    #[test]
    fn only_a_placing_grid_places() {
        let mut rig = Rig::new();
        let kit = item(2, "chest_kit", 1);
        rig.input = Container::new(rig.input.id, "crafting_input", 4, 2, &[kit]);
        rig.frame(vec![]);
        rig.right_click(rig.input.id, 2);
        assert!(rig.places.is_empty());
    }

    #[test]
    fn dragging_within_a_grid_asks_to_move_there() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        let fiber = rig.inventory.slot(0).unwrap().item_id;
        rig.drag((inv, 0), (inv, 7));
        assert_eq!(rig.moves, [Move::whole(fiber, inv, 7)]);
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    /// A shift-drag to an empty slot asks how many, starting at half, and
    /// Enter sends the split (E1.2)
    #[test]
    fn a_shift_drag_to_an_empty_slot_splits_half() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        let fiber = rig.inventory.slot(0).unwrap().item_id;
        rig.shift = true;
        rig.drag((inv, 0), (inv, 7));
        rig.shift = false;
        assert!(rig.moves.is_empty(), "it waits on the count");
        assert!(PendingSplit::get(&rig.ctx).is_some());

        rig.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(
            rig.moves,
            [Move {
                split: Some(6),
                ..Move::whole(fiber, inv, 7)
            }]
        );
        assert!(PendingSplit::get(&rig.ctx).is_none());
    }

    /// A stack of several qualities gets a row per quality, and a split of
    /// picked rows names those units (E1.8)
    #[test]
    fn a_split_can_take_chosen_qualities() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        let mixed = rig.inventory.slots[0]
            .take()
            .unwrap()
            .with_units(Units::from_pairs([(50, 6), (92, 6)]));
        let fiber = mixed.item_id;
        rig.inventory.slots[0] = Some(mixed);
        rig.shift = true;
        rig.drag((inv, 0), (inv, 7));
        rig.shift = false;
        let mut pending = PendingSplit::get(&rig.ctx).unwrap();
        assert_eq!(pending.picks, [(92, 0), (50, 0)], "high first, none picked");

        pending.picks[0].1 = 4;
        PendingSplit::set(&rig.ctx, Some(pending));
        rig.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(
            rig.moves,
            [Move {
                split: Some(4),
                qualities: Some(Units::uniform(4, 92)),
                ..Move::whole(fiber, inv, 7)
            }]
        );
    }

    /// Picking every unit leaves nothing to split from: Enter does nothing
    #[test]
    fn a_split_of_every_unit_waits() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        let mixed = rig.inventory.slots[0].take().unwrap().with_units(Units::from_pairs([(50, 1), (92, 1)]));
        rig.inventory.slots[0] = Some(mixed);
        rig.shift = true;
        rig.drag((inv, 0), (inv, 7));
        rig.shift = false;
        let mut pending = PendingSplit::get(&rig.ctx).unwrap();
        pending.picks = vec![(92, 1), (50, 1)];
        PendingSplit::set(&rig.ctx, Some(pending));
        rig.frame(vec![key(egui::Key::Enter)]);
        assert!(rig.moves.is_empty());
        assert!(PendingSplit::get(&rig.ctx).is_some());
    }

    #[test]
    fn the_makeup_reads_high_first() {
        let units = Units::from_pairs([(44, 1), (81, 3), (92, 6)]);
        assert_eq!(makeup(&units), "6 × 92 A, 3 × 81 B, 1 × 44 D");
        let many = Units::from_pairs([(44, 1), (81, 3), (85, 1), (92, 6), (95, 2)]);
        assert_eq!(makeup_by_grade(&many), "8 A · 4 B · 1 D");
        assert!(pick_rows(&Units::uniform(5, 60)).is_empty(), "one quality: a count is enough");
        assert_eq!(picked(&[(92, 0), (50, 0)]), None);
        assert_eq!(picked(&[(92, 2), (50, 0)]), Some(Units::uniform(2, 92)));
    }

    #[test]
    fn escape_lets_a_split_go() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        rig.shift = true;
        rig.drag((inv, 0), (inv, 7));
        rig.shift = false;
        rig.frame(vec![key(egui::Key::Escape)]);
        rig.frame(vec![key(egui::Key::Enter)]);
        assert!(rig.moves.is_empty());
    }

    /// Onto a stack, into another grid, or of a single item: a plain drag
    #[test]
    fn a_shift_drag_that_cant_split_moves() {
        let mut rig = Rig::new();
        let (inv, input) = (rig.inventory.id, rig.input.id);
        let fiber = rig.inventory.slot(0).unwrap().item_id;
        let rope = rig.inventory.slot(3).unwrap().item_id;
        rig.shift = true;
        rig.drag((inv, 0), (input, 0)); // another grid
        rig.drag((inv, 3), (inv, 8)); // one rope
        rig.shift = false;
        assert_eq!(
            rig.moves,
            [Move::whole(fiber, input, 0), Move::whole(rope, inv, 8)]
        );
        assert!(PendingSplit::get(&rig.ctx).is_none());
    }

    #[test]
    fn dragging_between_grids_names_the_other_container() {
        let mut rig = Rig::new();
        let (inv, input) = (rig.inventory.id, rig.input.id);
        let rope = rig.inventory.slot(3).unwrap().item_id;
        rig.drag((inv, 3), (input, 1)); // onto the fiber: the server swaps
        let fiber = rig.input.slot(1).unwrap().item_id;
        rig.drag((input, 1), (inv, 0)); // fiber onto fiber: the server merges
        assert_eq!(
            rig.moves,
            [Move::whole(rope, input, 1), Move::whole(fiber, inv, 0)]
        );
    }

    #[test]
    fn dropping_where_it_started_or_off_a_slot_does_nothing() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        rig.drag((inv, 0), (inv, 0));
        assert!(rig.moves.is_empty(), "back where it started");

        let a = rig.centre(inv, 0);
        rig.frame(vec![egui::Event::PointerMoved(a)]);
        rig.frame(vec![button(a, true)]);
        let away = Pos2::new(900.0, 650.0);
        for i in 1..=4 {
            rig.frame(vec![egui::Event::PointerMoved(
                a + (away - a) * (i as f32 / 4.0),
            )]);
        }
        rig.frame(vec![button(away, false)]);
        rig.frame(vec![]);
        assert!(rig.moves.is_empty(), "dropped on nothing");
        assert!(
            !egui::DragAndDrop::has_any_payload(&rig.ctx),
            "the drag is over"
        );
    }

    #[test]
    fn empty_slots_and_disabled_grids_give_nothing_to_drag() {
        let mut rig = Rig::new();
        let (inv, input) = (rig.inventory.id, rig.input.id);
        rig.drag((inv, 5), (input, 0)); // slot 5 is empty
        assert!(rig.moves.is_empty());

        rig.inventory_enabled = false;
        rig.frame(vec![]);
        rig.drag((inv, 0), (input, 0)); // out of a disabled grid
        rig.drag((input, 1), (inv, 4)); // into one
        assert!(rig.moves.is_empty());
    }
}
