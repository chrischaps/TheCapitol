//! The inventory panel (P5.4): the `player_inventory` container, bottom
//! right, as React's `InventoryPanel.tsx` has it. `I` (the `inventory`
//! action, `config/input.toml`) or the header's Inventory button opens and
//! closes it. It's open when you arrive, as React's always is.
//!
//! The grid is the slot grid (P5.3), so a drag to or from any other grid
//! on screen just works. It fills from the store, which the poller and the
//! WebSocket keep fresh: gathered fiber arrives with no refresh.
//!
//! A right click on a `*_kit` item starts placing its station (P6.2): the
//! panel hands the kit back, and `Session::begin_placing` takes it.

use super::icons::ItemIcons;
use super::slots::{Clicks, SlotGrid};
use super::theme::{self, size, CRAFT, DIM, SLOT_GAP};
use capitol_core::state::Container;
use egui::{Align2, Id, RichText};

/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;
/// How far it slides as it opens, in points.
const RISE: f32 = 12.0;

/// Whether the panel is open, between frames.
pub struct InventoryPanel {
    pub open: bool,
    /// Where it was drawn last frame, for the station panel to sit beside.
    /// `None` while it's closed.
    pub rect: Option<egui::Rect>,
}

impl Default for InventoryPanel {
    fn default() -> Self {
        Self {
            open: true,
            rect: None,
        }
    }
}

impl InventoryPanel {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Draws the panel, or its fade, over the world. `inventory` is `None`
    /// until the first fetch; `refused` is the server's reason for the last
    /// move, while it's fresh. Returns the move a drop asks for, and the kit
    /// a right click asks to place.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        icons: &mut ItemIcons,
        inventory: Option<&Container>,
        refused: Option<&str>,
        pulse: f32,
    ) -> Clicks {
        let shown = ctx.animate_bool_with_time(Id::new("capitol_inventory_fade"), self.open, FADE);
        if shown <= 0.0 {
            self.rect = None;
            return Clicks::default();
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        let area = egui::Area::new(Id::new("capitol_inventory"))
            .anchor(Align2::RIGHT_BOTTOM, [-20.0, -20.0 + RISE * (1.0 - ease)])
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(CRAFT)
                    .show(ui, |ui| body(ui, icons, inventory, refused))
                    .inner
            });
        self.rect = self.open.then_some(area.response.rect);
        // A pickup landing lights its edge (P8.2).
        theme::landed_glow(
            &ctx.layer_painter(area.response.layer_id),
            area.response.rect,
            12.0,
            pulse * ease,
        );
        area.inner
    }
}

/// The panel's insides: the header with its count, the grid, the total,
/// and a refused move's reason. `--ui-sample` draws its inventory with this
/// too, so the sample is the panel.
pub fn body(
    ui: &mut egui::Ui,
    icons: &mut ItemIcons,
    inventory: Option<&Container>,
    refused: Option<&str>,
) -> Clicks {
    // Six slots and their gaps, and the tray's margin (React's min-width).
    ui.set_width(6.0 * theme::SLOT_SIZE + 5.0 * SLOT_GAP + 16.0);
    let Some(c) = inventory else {
        theme::header(ui, "Inventory", None, Some(("Loading…", theme::PILL)));
        return Clicks::default();
    };
    let badge = format!("{}/{}", c.used_slots(), c.slot_count());
    theme::header(ui, "Inventory", None, Some((&badge, theme::PILL)));
    let clicks = SlotGrid::new(c).placing(true).show_clicks(ui, icons);
    let total: i32 = c.items().map(|i| i.quantity).sum();
    if total > 0 {
        ui.add_space(2.0);
        theme::divider(ui);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(if total == 1 {
                    "1 item".to_owned()
                } else {
                    format!("{total} items")
                })
                .color(DIM)
                .size(size::LABEL),
            );
        });
    }
    if let Some(why) = refused {
        ui.add_space(4.0);
        theme::error_box(ui, why);
    }
    clicks
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::protocol::rest::SlotItem;
    use uuid::Uuid;

    fn run(ctx: &egui::Context, panel: &mut InventoryPanel, c: Option<&Container>) -> egui::Rect {
        let mut icons = ItemIcons::load(std::path::Path::new("no/icons/needed"));
        let mut rect = egui::Rect::NOTHING;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        // The theme's fonts go in on the first frame, and an anchored area
        // takes a frame to learn its size before it's placed.
        for _ in 0..4 {
            let _ = ctx.run(raw.clone(), |ctx| {
                if !theme::install(ctx) {
                    return;
                }
                panel.show(ctx, &mut icons, c, None, 0.0);
            });
        }
        if let Some(area) = ctx.memory(|m| m.area_rect(Id::new("capitol_inventory"))) {
            rect = area;
        }
        rect
    }

    fn inventory() -> Container {
        let fiber = SlotItem::new(Uuid::from_u128(2), "fiber".into(), "Fiber".into(), 62, 7, 0);
        Container::new(Uuid::from_u128(1), "player_inventory", 24, 6, &[fiber])
    }

    #[test]
    fn the_panel_sits_bottom_right_six_slots_wide() {
        let ctx = egui::Context::default();
        let c = inventory();
        let rect = run(&ctx, &mut InventoryPanel::default(), Some(&c));
        assert!(rect.is_positive(), "the panel was drawn");
        assert!((1280.0 - rect.right() - 20.0).abs() < 1.0, "{rect:?}");
        // The grid, its tray and the panel's margins.
        let inner = 6.0 * theme::SLOT_SIZE + 5.0 * SLOT_GAP + 16.0;
        assert!(
            rect.width() >= inner && rect.width() < inner + 40.0,
            "{rect:?}"
        );
    }

    #[test]
    fn closed_it_draws_nothing() {
        let ctx = egui::Context::default();
        let c = inventory();
        let mut panel = InventoryPanel::default();
        panel.toggle();
        // Closed from the first frame, it starts at its target: no fade.
        run(&ctx, &mut panel, Some(&c));
        assert!(ctx
            .memory(|m| m.area_rect(Id::new("capitol_inventory")))
            .is_none());
    }
}
