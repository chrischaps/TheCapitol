//! `--juice-sample` (P8.2): the juice played out offline, on a timetable,
//! for captures. The local world has no A-grade grass (its nodes top out
//! at 70) and captures can't click (issue #76), so this stands in for the
//! server: it fires the events a session would at the scripts, and shows
//! an inventory panel holding an A-grade stack for the pickups to land on.
//!
//! In order: a B-grade gather and an A-grade one with their pickups,
//! the same refusal twice (one toast, counted), a canal in the way, and a
//! walk into the Trade District (the banner and the camera's ease).
//! `--offline fixtures/terrain.json --juice-sample` frames them all at the
//! street view's start.

use std::time::Instant;

use capitol_core::protocol::rest::SlotItem;
use capitol_core::state::{Container, Yield};
use capitol_protocol::rules::Zone;
use flint_player::ExtensionCtx;
use uuid::Uuid;

use crate::script_api::{Shared, UiAnchor, ZoneBanner};
use crate::ui::icons::ItemIcons;
use crate::ui::inventory::InventoryPanel;
use crate::world::palette;

/// What happens, and when (game seconds).
const TIMETABLE: &[(f64, Beat)] = &[
    (0.5, Beat::Arrive(Zone::Urban, true)),
    (
        1.5,
        Beat::Gather {
            x: 3.0,
            z: 42.0,
            quality: 78,
            quantity: 3,
        },
    ),
    (2.4, Beat::Note),
    (
        3.2,
        Beat::Gather {
            x: -3.5,
            z: 43.5,
            quality: 93,
            quantity: 2,
        },
    ),
    (3.6, Beat::Note),
    (4.6, Beat::Blocked),
    (6.0, Beat::Arrive(Zone::TradeDistrict, false)),
];

#[derive(Debug, Clone, Copy)]
enum Beat {
    Arrive(Zone, bool),
    Gather {
        x: f64,
        z: f64,
        quality: i32,
        quantity: i32,
    },
    Note,
    Blocked,
}

pub struct JuiceSample {
    next: usize,
    inventory: Container,
    panel: InventoryPanel,
}

impl Default for JuiceSample {
    fn default() -> Self {
        let item = |n: u128, slot: i32, quality: i32, quantity: i32| {
            SlotItem::new(
                Uuid::from_u128(n),
                "fiber".into(),
                "Plant Fiber".into(),
                quality,
                quantity,
                slot,
            )
        };
        Self {
            next: 0,
            inventory: Container::new(
                Uuid::from_u128(0x7a1c_e000),
                "player_inventory",
                24,
                6,
                &[item(1, 0, 93, 2), item(2, 1, 78, 3), item(3, 2, 64, 9)],
            ),
            panel: InventoryPanel::default(),
        }
    }
}

impl JuiceSample {
    /// Fires whatever is due.
    pub fn update(&mut self, ctx: &mut ExtensionCtx, shared: &Shared) {
        let t = ctx.clock.total_time;
        while let Some(&(at, beat)) = TIMETABLE.get(self.next) {
            if t < at {
                break;
            }
            self.next += 1;
            fire(ctx, shared, beat);
        }
    }

    /// The inventory panel, and where the pickups land.
    pub fn ui(&mut self, ctx: &egui::Context, icons: &mut ItemIcons, shared: &Shared) {
        let pulse = shared.pulse("inventory", Instant::now());
        let catalog = capitol_core::catalog::Catalog::fallback();
        self.panel
            .show(ctx, icons, &catalog, Some(&self.inventory), None, pulse);
        if let (Some(r), Ok(mut anchors)) = (self.panel.rect, shared.anchors.lock()) {
            let rect = [r.left(), r.top(), r.width(), r.height()];
            anchors.insert("inventory", UiAnchor { rect, panel: true });
        }
    }
}

fn fire(ctx: &mut ExtensionCtx, shared: &Shared, beat: Beat) {
    let mut data = toml::Table::new();
    match beat {
        Beat::Arrive(zone, first) => {
            if let Ok(mut b) = shared.banner.lock() {
                *b = Some(ZoneBanner {
                    name: zone.display_name().to_owned(),
                    colour: palette::srgb(palette::zone_mark(zone)),
                    at: Instant::now(),
                    first,
                });
            }
        }
        Beat::Gather {
            x,
            z,
            quality,
            quantity,
        } => {
            let mut data = crate::extension::yield_data(&Yield {
                item_type: "fiber".into(),
                item_name: "Plant Fiber".into(),
                quantity,
                quality,
            });
            data.insert("x".into(), x.into());
            data.insert("z".into(), z.into());
            ctx.fire_event("extraction_completed", data);
        }
        Beat::Note => {
            data.insert("title".into(), "Taken".into());
            data.insert("body".into(), "Someone else got to that grass first".into());
            ctx.fire_event("extraction_note", data);
        }
        Beat::Blocked => {
            data.insert("x".into(), 6.0.into());
            data.insert("z".into(), 60.0.into());
            data.insert("dir_x".into(), 0.0.into());
            data.insert("dir_z".into(), 1.0.into());
            data.insert("water".into(), "the inner canal".into());
            data.insert("reason".into(), "sample".into());
            ctx.fire_event("movement_blocked", data);
        }
    }
}
