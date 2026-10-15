//! Item icons (P5.2). Each item type has an SVG in `assets/icons/` named for
//! its `item_type` (`fiber.svg`, `forge_kit.svg`), so a new item type only
//! needs a file. A type without one is drawn with `fallback.svg`, a sack
//! with a question mark on it. That file is also compiled in, so a missing
//! directory still gives every slot an icon.
//!
//! The SVGs are parsed once and rasterized for each size they're drawn at,
//! in physical pixels, so they stay sharp at any UI scale. They share one
//! style (`assets/icons/README.md`): drawn on a 64-unit square, warm flat
//! fills with a dark ink outline, light from the top left, a soft shadow on
//! the ground, and one touch of the Capitol's gold.
//!
//! `--icon-sheet` shows every icon on a panel, in slots and large, for
//! review ([`sheet`]).

use super::{panel_frame, DIM, GOLD, TEXT};
use egui::{Color32, RichText};
use resvg::{tiny_skia, usvg};
use std::collections::HashMap;
use std::path::Path;

/// The type an icon is drawn with when its own is missing.
const FALLBACK: &str = "fallback";

/// Compiled in, in case `assets/icons/` can't be read.
const FALLBACK_SVG: &str = include_str!("../../../../assets/icons/fallback.svg");

pub struct ItemIcons {
    trees: HashMap<String, usvg::Tree>,
    /// Rasterized icons by item type, then by size in physical pixels.
    textures: HashMap<String, HashMap<u32, egui::TextureHandle>>,
}

impl ItemIcons {
    /// Relative to `flint-client/`, where the client runs.
    pub const DIR: &'static str = "assets/icons";

    /// Every `*.svg` in `dir`. One that won't parse is logged and skipped,
    /// and its type gets the fallback.
    pub fn load(dir: &Path) -> Self {
        let mut trees = HashMap::new();
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                for path in entries.flatten().map(|e| e.path()) {
                    if path.extension().and_then(|e| e.to_str()) != Some("svg") {
                        continue;
                    }
                    let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    match std::fs::read_to_string(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|text| parse(&text))
                    {
                        Ok(tree) => {
                            trees.insert(name.to_owned(), tree);
                        }
                        Err(e) => tracing::warn!("icons: skipping {}: {e}", path.display()),
                    }
                }
            }
            Err(e) => tracing::warn!("icons: can't read {}: {e}", dir.display()),
        }
        if !trees.contains_key(FALLBACK) {
            let tree = parse(FALLBACK_SVG).expect("the compiled-in fallback icon parses");
            trees.insert(FALLBACK.to_owned(), tree);
        }
        Self {
            trees,
            textures: HashMap::new(),
        }
    }

    /// Whether `item_type` has an icon of its own.
    pub fn has(&self, item_type: &str) -> bool {
        item_type != FALLBACK && self.trees.contains_key(item_type)
    }

    /// The icon for `item_type`, `size` points square, rasterized for the
    /// context's pixels per point the first time it's asked for.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        item_type: &str,
        size: f32,
    ) -> &egui::TextureHandle {
        let key = if self.has(item_type) {
            item_type
        } else {
            FALLBACK
        };
        let px = ((size * ctx.pixels_per_point()).round() as u32).max(1);
        let tree = &self.trees[key];
        self.textures
            .entry(key.to_owned())
            .or_default()
            .entry(px)
            .or_insert_with(|| {
                ctx.load_texture(
                    format!("icon:{key}@{px}"),
                    rasterize(tree, px),
                    egui::TextureOptions::LINEAR,
                )
            })
    }

    /// Draw `item_type`'s icon filling `rect`, which should be square.
    pub fn paint(&mut self, ui: &egui::Ui, rect: egui::Rect, item_type: &str) {
        let texture = self.texture(ui.ctx(), item_type, rect.width());
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

fn parse(text: &str) -> Result<usvg::Tree, String> {
    usvg::Tree::from_str(text, &usvg::Options::default()).map_err(|e| e.to_string())
}

/// `tree` scaled to fit `px` square, centred.
fn rasterize(tree: &usvg::Tree, px: u32) -> egui::ColorImage {
    let mut pixmap = tiny_skia::Pixmap::new(px, px).expect("icon size is at least 1 px");
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    let dx = (px as f32 - size.width() * scale) / 2.0;
    let dy = (px as f32 - size.height() * scale) / 2.0;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    // tiny-skia's pixels are premultiplied RGBA, as egui wants them.
    egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data())
}

/// The item types the server knows today, with their names, for the sheet.
/// The last one has no icon, to show the fallback.
const SHEET: [(&str, &str); 6] = [
    ("fiber", "Plant Fiber"),
    ("rope", "Rope"),
    ("workbench_kit", "Workbench Kit"),
    ("forge_kit", "Forge Kit"),
    ("chest_kit", "Storage Chest Kit"),
    ("lantern_oil", "(no icon yet)"),
];

/// Grade pips for the sheet's slots (plan §4.7), A to F, so the icons are
/// seen next to what P5.3 will put in the slot's corner.
const PIPS: [Color32; 5] = [
    Color32::from_rgb(0xff, 0xd7, 0x00),
    Color32::from_rgb(0x22, 0xc5, 0x5e),
    Color32::from_rgb(0x3b, 0x82, 0xf6),
    Color32::from_rgb(0xa8, 0x55, 0xf7),
    Color32::from_rgb(0x6b, 0x72, 0x80),
];

/// A slot as the React client draws one (`InventorySlot.css`): 48 points,
/// with the icon at 36 in the middle, a grade pip top right and the
/// quantity bottom right. P5.3's slot widget replaces this.
fn sheet_slot(ui: &mut egui::Ui, icons: &mut ItemIcons, item_type: &str, pip: Color32, qty: u32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect.shrink(1.0),
        6.0,
        Color32::from_rgba_unmultiplied(30, 30, 40, 204),
        egui::Stroke::new(2.0, Color32::from_rgba_unmultiplied(100, 100, 120, 179)),
    );
    icons.paint(
        ui,
        egui::Rect::from_center_size(rect.center(), egui::vec2(36.0, 36.0)),
        item_type,
    );
    let painter = ui.painter();
    painter.circle(
        rect.right_top() + egui::vec2(-7.0, 7.0),
        4.0,
        pip,
        egui::Stroke::new(1.0, Color32::from_black_alpha(77)),
    );
    painter.text(
        rect.right_bottom() + egui::vec2(-5.0, -3.0),
        egui::Align2::RIGHT_BOTTOM,
        qty.to_string(),
        egui::FontId::proportional(11.0),
        TEXT,
    );
}

/// Every icon on one panel: large, then in slots, then at the React
/// client's emoji size (20 points). Shown by `--icon-sheet`.
pub fn sheet(ctx: &egui::Context, icons: &mut ItemIcons) {
    egui::Window::new("item icons")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(panel_frame())
        .show(ctx, |ui| {
            ui.label(
                RichText::new("ITEM ICONS")
                    .color(GOLD)
                    .size(12.0)
                    .extra_letter_spacing(1.5)
                    .strong(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                for (item_type, name) in SHEET {
                    ui.vertical(|ui| {
                        ui.set_width(104.0);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(96.0, 96.0), egui::Sense::hover());
                        icons.paint(ui, rect, item_type);
                        ui.label(RichText::new(name).color(TEXT).size(12.0));
                        ui.label(RichText::new(item_type).color(DIM).size(10.0).monospace());
                    });
                }
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, (item_type, _)) in SHEET.iter().enumerate() {
                    sheet_slot(
                        ui,
                        icons,
                        item_type,
                        PIPS[i % PIPS.len()],
                        [12, 3, 1, 1, 1, 7][i],
                    );
                }
                ui.add_space(20.0);
                for (item_type, _) in SHEET {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                    icons.paint(ui, rect, item_type);
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons")
    }

    #[test]
    fn every_icon_parses_and_draws_something() {
        let icons = ItemIcons::load(&dir());
        for (item_type, _) in &SHEET[..5] {
            assert!(icons.has(item_type), "{item_type}.svg is missing or broken");
        }
        for (name, tree) in &icons.trees {
            let image = rasterize(tree, 64);
            let covered = image.pixels.iter().filter(|p| p.a() > 0).count();
            assert!(covered > 64 * 64 / 8, "{name} draws almost nothing");
        }
    }

    #[test]
    fn unknown_types_and_a_missing_directory_fall_back() {
        let icons = ItemIcons::load(&dir());
        assert!(!icons.has("lantern_oil"));
        assert!(!icons.has(FALLBACK));
        let none = ItemIcons::load(Path::new("no/such/dir"));
        assert!(!none.has("fiber"));
        assert!(none.trees.contains_key(FALLBACK));
    }

    #[test]
    fn a_texture_is_made_once_per_size() {
        let ctx = egui::Context::default();
        let mut icons = ItemIcons::load(&dir());
        let a = icons.texture(&ctx, "rope", 36.0).id();
        let b = icons.texture(&ctx, "rope", 36.0).id();
        let c = icons.texture(&ctx, "rope", 20.0).id();
        let d = icons.texture(&ctx, "nothing_like_it", 36.0).id();
        let e = icons.texture(&ctx, "something_else", 36.0).id();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(d, e, "unknown types share the fallback's texture");
    }
}
