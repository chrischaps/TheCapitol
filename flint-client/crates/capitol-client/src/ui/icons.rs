//! Item icons (P5.2). Each item type has an SVG in `assets/icons/` named for
//! its `item_type` (`fiber.svg`, `forge_kit.svg`), so a new item type only
//! needs a file. A type without one is drawn with its catalog category's
//! icon, `assets/icons/categories/<category>.svg` (E2.6): the category's
//! shape in slate with a gold question tag, so a tool the server added
//! since this client was built still reads as a tool. A type the catalog
//! doesn't know either is drawn with `fallback.svg`, a sack with a question
//! mark on it. That file is also compiled in, so a missing directory still
//! gives every slot an icon. The category comes from the catalog
//! `slots::know_catalog` leaves in the egui context each frame.
//!
//! The SVGs are parsed once and rasterized for each size they're drawn at,
//! in physical pixels, so they stay sharp at any UI scale. They share one
//! style (`assets/icons/README.md`): drawn on a 64-unit square, warm flat
//! fills with a dark ink outline, light from the top left, a soft shadow on
//! the ground, and one touch of the Capitol's gold.
//!
//! `--icon-sheet` shows every icon on a panel, in slots and large, for
//! review ([`sheet`]).

use super::slots::Slot;
use super::theme::{self, size, DIM, TEXT};
use capitol_core::protocol::rest::SlotItem;
use egui::{Color32, RichText};
use resvg::{tiny_skia, usvg};
use std::collections::HashMap;
use std::path::Path;

/// The type an icon is drawn with when its own and its category's are
/// missing.
const FALLBACK: &str = "fallback";

/// The directory, under the icons', of the icons by catalog category.
const CATEGORIES: &str = "categories";

/// Compiled in, in case `assets/icons/` can't be read.
const FALLBACK_SVG: &str = include_str!("../../../../assets/icons/fallback.svg");

pub struct ItemIcons {
    trees: HashMap<String, usvg::Tree>,
    /// The category icons, by category, from `categories/`.
    categories: HashMap<String, usvg::Tree>,
    /// Rasterized icons by item type, then by size in physical pixels.
    textures: HashMap<String, HashMap<u32, egui::TextureHandle>>,
}

impl ItemIcons {
    /// Relative to `flint-client/`, where the client runs.
    pub const DIR: &'static str = "assets/icons";

    /// Every `*.svg` in `dir`, and the category icons in
    /// `dir/categories`. One that won't parse is logged and skipped, and
    /// its type gets the next icon down.
    pub fn load(dir: &Path) -> Self {
        let mut trees = svgs(dir);
        if !trees.contains_key(FALLBACK) {
            let tree = parse(FALLBACK_SVG).expect("the compiled-in fallback icon parses");
            trees.insert(FALLBACK.to_owned(), tree);
        }
        Self {
            trees,
            categories: svgs(&dir.join(CATEGORIES)),
            textures: HashMap::new(),
        }
    }

    /// Whether `item_type` has an icon of its own.
    pub fn has(&self, item_type: &str) -> bool {
        item_type != FALLBACK && self.trees.contains_key(item_type)
    }

    /// The texture key `item_type` is drawn with: its own icon, else its
    /// category's (`categories/tool`), else the fallback's. `category` is
    /// the catalog's for it, if the catalog knows it.
    fn key(&self, item_type: &str, category: Option<&str>) -> String {
        if self.has(item_type) {
            return item_type.to_owned();
        }
        match category.filter(|c| self.categories.contains_key(*c)) {
            Some(c) => format!("{CATEGORIES}/{c}"),
            None => FALLBACK.to_owned(),
        }
    }

    fn tree(&self, key: &str) -> &usvg::Tree {
        match key
            .strip_prefix(CATEGORIES)
            .and_then(|k| k.strip_prefix('/'))
        {
            Some(category) => &self.categories[category],
            None => &self.trees[key],
        }
    }

    /// The icon for `item_type`, `size` points square, rasterized for the
    /// context's pixels per point the first time it's asked for.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        item_type: &str,
        size: f32,
    ) -> &egui::TextureHandle {
        let px = ((size * ctx.pixels_per_point()).round() as u32).max(1);
        let category = if self.has(item_type) {
            None
        } else {
            super::slots::catalog(ctx)
                .and_then(|c| c.item_type(item_type).map(|t| t.category.clone()))
        };
        let key = self.key(item_type, category.as_deref());
        self.keyed(ctx, key, px)
    }

    /// The texture for `key` (an item type with an icon, `categories/<c>`
    /// or the fallback) at `px` physical pixels square.
    fn keyed(&mut self, ctx: &egui::Context, key: String, px: u32) -> &egui::TextureHandle {
        if !self.textures.get(&key).is_some_and(|s| s.contains_key(&px)) {
            let image = rasterize(self.tree(&key), px);
            let texture = ctx.load_texture(
                format!("icon:{key}@{px}"),
                image,
                egui::TextureOptions::LINEAR,
            );
            self.textures
                .entry(key.clone())
                .or_default()
                .insert(px, texture);
        }
        &self.textures[&key][&px]
    }

    /// Draw `item_type`'s icon filling `rect`, which should be square.
    pub fn paint(&mut self, ui: &egui::Ui, rect: egui::Rect, item_type: &str) {
        self.paint_on(ui.painter(), rect, item_type, 1.0);
    }

    /// Draw `item_type`'s icon filling `rect` with `painter`, at `opacity`
    /// (0 to 1).
    pub fn paint_on(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        item_type: &str,
        opacity: f32,
    ) {
        let texture = self.texture(painter.ctx(), item_type, rect.width());
        painter.image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE.gamma_multiply(opacity),
        );
    }
}

/// Every `*.svg` in `dir`, by file stem. One that won't parse is logged
/// and left out.
fn svgs(dir: &Path) -> HashMap<String, usvg::Tree> {
    let mut trees = HashMap::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("icons: can't read {}: {e}", dir.display());
            return trees;
        }
    };
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
    trees
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

/// Qualities for the sheet's slots, one per grade A to F, so the icons are
/// seen next to every pip colour.
const QUALITIES: [i32; 5] = [94, 81, 63, 41, 20];

/// The catalog's categories, in the order the sheet shows their icons.
const CATEGORY_ORDER: [&str; 5] = [
    "raw_material",
    "processed_material",
    "component",
    "tool",
    "placeable",
];

/// A made-up item in slot `i`, for the sheet.
fn sheet_item(i: usize, item_type: &str, name: &str, quantity: i32) -> SlotItem {
    SlotItem::new(
        uuid::Uuid::nil(),
        item_type.to_owned(),
        name.to_owned(),
        QUALITIES[i % QUALITIES.len()],
        quantity,
        i as i32,
    )
}

/// One icon, `size` points square, with a name under it and a key under
/// that.
fn labelled(
    ui: &mut egui::Ui,
    size: f32,
    name: &str,
    key: &str,
    paint: impl FnOnce(&egui::Ui, egui::Rect),
) {
    ui.vertical(|ui| {
        ui.set_width(size + 18.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        paint(ui, rect);
        ui.label(RichText::new(name).color(TEXT).size(size::TINY));
        ui.label(RichText::new(key).color(DIM).size(size::TINY).monospace());
    });
}

/// Every icon on one panel, for review: each item the catalog knows,
/// large and named; the same again in inventory slots, as a bag holding
/// one of everything; and the category icons and the fallback that stand
/// in for a type without its own. Shown by `--icon-sheet`.
pub fn sheet(ctx: &egui::Context, icons: &mut ItemIcons) {
    let catalog = super::slots::catalog(ctx).unwrap_or_default();
    let items: Vec<(String, String)> = catalog
        .item_types()
        .iter()
        .map(|t| (t.id.clone(), t.name.clone()))
        .collect();
    const PER_ROW: usize = 11;
    egui::Window::new("item icons")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(theme::panel())
        .show(ctx, |ui| {
            ui.label(theme::title("Item icons", size::BODY));
            ui.add_space(8.0);
            for row in items.chunks(PER_ROW) {
                ui.horizontal(|ui| {
                    for (id, name) in row {
                        labelled(ui, 56.0, name, id, |ui, rect| icons.paint(ui, rect, id));
                    }
                });
                ui.add_space(4.0);
            }
            ui.add_space(8.0);
            let per_slot_row = 2 * PER_ROW;
            for (r, row) in items.chunks(per_slot_row).enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (k, (id, name)) in row.iter().enumerate() {
                        let i = r * per_slot_row + k;
                        let item = sheet_item(i, id, name, [1, 3, 12, 7, 24][i % 5]);
                        Slot {
                            container_id: uuid::Uuid::nil(),
                            index: i,
                            item: Some(&item),
                            enabled: true,
                        }
                        .show(ui, icons); // a drop here goes nowhere
                    }
                });
            }
            ui.add_space(12.0);
            ui.label(RichText::new("Stand-ins").color(DIM).size(size::SMALL));
            ui.horizontal(|ui| {
                for category in CATEGORY_ORDER {
                    labelled(ui, 56.0, category, "categories/", |ui, rect| {
                        let px =
                            ((rect.width() * ui.ctx().pixels_per_point()).round() as u32).max(1);
                        let key = icons.key("", Some(category));
                        let texture = icons.keyed(ui.ctx(), key, px);
                        ui.painter().image(
                            texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    });
                }
                labelled(ui, 56.0, "unknown", "fallback", |ui, rect| {
                    icons.paint(ui, rect, "lantern_oil")
                });
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
    fn every_item_in_the_catalog_has_its_own_icon() {
        let icons = ItemIcons::load(&dir());
        let catalog = capitol_core::catalog::Catalog::fallback();
        for t in catalog.item_types() {
            assert!(icons.has(&t.id), "{}.svg is missing or broken", t.id);
            assert!(
                icons.categories.contains_key(&t.category),
                "no categories/{}.svg for {}",
                t.category,
                t.id
            );
        }
        for c in CATEGORY_ORDER {
            assert!(icons.categories.contains_key(c), "categories/{c}.svg");
        }
    }

    #[test]
    fn a_type_without_an_icon_takes_its_categorys() {
        let icons = ItemIcons::load(&dir());
        assert_eq!(icons.key("rope", Some("processed_material")), "rope");
        assert_eq!(icons.key("bronze_axe", Some("tool")), "categories/tool");
        assert_eq!(icons.key("bronze_axe", Some("weapon")), FALLBACK);
        assert_eq!(icons.key("bronze_axe", None), FALLBACK);
        // Each key names a tree.
        icons.tree("categories/tool");
        icons.tree(FALLBACK);
    }

    #[test]
    fn every_icon_parses_and_draws_something() {
        let icons = ItemIcons::load(&dir());
        for (name, tree) in icons.trees.iter().chain(&icons.categories) {
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
