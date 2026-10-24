//! The `cap_*` functions scripts call into Rust (plan §4.2). Registered by
//! `CapitolExtension::register_script_api`.
//!
//! Rhai runs with its `sync` feature, so every closure here and the state
//! it captures must be `Send + Sync`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::labels;

/// State the Rust side writes and scripts read.
#[derive(Clone, Default)]
pub struct Shared {
    /// The inscription over the Capitol (`--note`).
    pub note: Arc<Mutex<String>>,
    /// The camera view the script frames (`--view`).
    pub view: Arc<Mutex<String>>,
    /// A camera move to film (`--camera-path`, issue #92), played by the
    /// rig in place of the view.
    pub camera_path: Arc<Mutex<Option<crate::camera_path::CameraPath>>>,
    /// A clean capture (`--clean`, P8.7): offline, no inscription panel
    /// and no free-point mark, just the city.
    pub clean: Arc<AtomicBool>,
    /// The login form is up (P4.1): the camera circles the tower behind it.
    pub at_login: Arc<AtomicBool>,
    /// The camera rig's yaw as it means it to be (`cap_view_yaw`, controls
    /// T1.3), radians round the point it looks at, 0 standing it south:
    /// what Direct mode's keys walk against. `None` until the rig says.
    pub view_yaw: Arc<Mutex<Option<f64>>>,
    /// Something wants the cursor free (`cap_wants_cursor`, T1.4): a panel,
    /// the login, Alt held, a station being placed, or Escape having let it
    /// go. The extension sets it at the end of `ui`; in Direct the camera
    /// rig captures the cursor whenever it's false.
    pub wants_cursor: Arc<AtomicBool>,
    /// Escape let the cursor go with no panel open (T1.4). A click on the
    /// world takes it back (`cap_take_cursor`), and so does a panel closing.
    pub cursor_let_go: Arc<AtomicBool>,
    /// Where you stand in world metres (P4.2), for the camera to follow.
    /// `None` offline, and until the server has said where you are.
    pub follow: Arc<Mutex<Option<[f32; 3]>>>,
    /// The name over each avatar this frame (P4.2), for `hud.rhai`.
    pub players: Arc<Mutex<Vec<PlayerLabel>>>,
    /// Where a script last asked to walk (`cap_move`, P4.3), in server
    /// units, clamped to the world. The extension takes it next frame.
    pub move_to: Arc<Mutex<Option<[f64; 2]>>>,
    /// Where you're walking to (P4.3), in world metres, for the marker.
    /// `None` when standing, and offline.
    pub destination: Arc<Mutex<Option<[f32; 2]>>>,
    /// The corners your walk has still to pass round the water (#267), in
    /// world metres, ending at the destination. Empty when standing.
    pub route: Arc<Mutex<Vec<[f32; 2]>>>,
    /// What the admin's world layers draw this frame (D5.3), for
    /// `debug.rhai`. `None` while they're off.
    pub debug: Arc<Mutex<Option<crate::debug_layers::Mark>>>,
    /// The zone you last entered (P4.5), for the banner in `hud.rhai`.
    pub banner: Arc<Mutex<Option<ZoneBanner>>>,
    /// Every resource node shown this frame (P4.4), as `nodes::Nodes` left
    /// them, for picking and the HUD.
    pub nodes: Arc<Mutex<Vec<NodeMark>>>,
    /// The node a script last asked to extract from (`cap_extract`). The
    /// extension takes it next frame.
    pub extract: Arc<Mutex<Option<String>>>,
    /// Every station shown this frame (P6.1), as `stations::Stations` left
    /// them, for the in-range ring and picking.
    pub stations: Arc<Mutex<Vec<StationMark>>>,
    /// Where a script last asked to pick a plot (`cap_pick_plot`, P6.4),
    /// in server units. The extension takes it next frame.
    pub pick_plot: Arc<Mutex<Option<[f64; 2]>>>,
    /// The plot the plot panel shows, for its outline on the ground.
    pub picked_plot: Arc<Mutex<Option<PlotMark>>>,
    /// The station being placed (P6.2), as the extension left it this
    /// frame. `None` when nothing is.
    pub placement: Arc<Mutex<Option<PlacementMark>>>,
    /// The ground under the cursor, world metres, as `input.rhai` last saw
    /// it (`cap_place_hover`). The ghost follows it.
    pub place_hover: Arc<Mutex<Option<[f32; 2]>>>,
    /// Where a script last asked to put the station down (`cap_place`),
    /// world metres. The extension takes it next frame.
    pub place_at: Arc<Mutex<Option<[f32; 2]>>>,
    /// The station a script last asked to open (`cap_open_station`, P6.3).
    /// The extension takes it next frame.
    pub open_station: Arc<Mutex<Option<String>>>,
    /// The Exchange hall (E3.7) as the extension left it this frame. `None`
    /// when the terrain has none.
    pub hall: Arc<Mutex<Option<HallMark>>>,
    /// A script asked to go to the hall (`cap_visit_hall`). The extension
    /// takes it next frame.
    pub visit_hall: Arc<AtomicBool>,
    /// A script asked to open the Gazette (`cap_read_gazette`, E3.9). The
    /// extension takes it next frame.
    pub read_gazette: Arc<AtomicBool>,
    /// A script asked to look in at the Deeds Office (`cap_open_office`,
    /// #448). The extension takes it next frame.
    pub open_office: Arc<AtomicBool>,
    /// The bazaar's stalls (E3.11) as the extension left them this frame.
    /// `None` when the terrain has no bazaar.
    pub bazaar: Arc<Mutex<Option<BazaarMark>>>,
    /// Where the road in's card leads this frame (#442), for its ring.
    /// `None` while the card leads nowhere, or is hidden.
    pub lead: Arc<Mutex<Option<LeadMark>>>,
    /// The stall a script last asked to go to (`cap_visit_stall`). The
    /// extension takes it next frame.
    pub visit_stall: Arc<Mutex<Option<String>>>,
    /// The bazaar's stalls' footprints (E3.13), for `cap_in_bazaar`.
    pub stall_feet: Arc<Mutex<Vec<[[f32; 2]; 4]>>>,
    /// Where the egui UI's landmarks were drawn last frame (P8.2), by name,
    /// for scripts to fly things to. Only "inventory" so far.
    pub anchors: Arc<Mutex<HashMap<&'static str, UiAnchor>>>,
    /// When a script last lit each landmark (`cap_ui_pulse`), by name.
    pub pulses: Arc<Mutex<HashMap<String, Instant>>>,
    /// The player a script last asked to trade with (`cap_trade_with`,
    /// P7.3). The extension takes it next frame.
    pub trade_with: Arc<Mutex<Option<String>>>,
    /// The player a script last asked to see the card of (`cap_open_card`,
    /// S0.3). The extension takes it next frame.
    pub open_card: Arc<Mutex<Option<String>>>,
    /// Where trading stands, for the thread between you (`cap_trade`).
    pub trade: Arc<Mutex<Option<TradeMark>>>,
    /// How the camera rig answers you (P8.4), from the settings, for
    /// `cap_setting`.
    pub camera_feel: Arc<Mutex<crate::settings::CameraFeel>>,
    /// How you walk and look (T1.1), for `cap_setting`. Its mode is the one
    /// in play, which a capture or `--controls` may pin apart from the file.
    pub controls: Arc<Mutex<crate::settings::Controls>>,
    /// egui, once its fonts are in, so `cap_name_tags` measures names with
    /// the fonts the HUD draws them in. `None` until the first `ui`.
    pub egui: Arc<Mutex<Option<egui::Context>>>,
    /// The camera scripts' `world_to_screen` uses (Flint ADR 0087), so
    /// `cap_name_tags` projects with the frame's camera. `None` in tests.
    pub projector: Option<flint_player::Projector>,
}

/// Trading as scripts see it.
#[derive(Debug, Clone, PartialEq)]
pub struct TradeMark {
    /// "asking", "incoming", "walking" or "open".
    pub state: &'static str,
    /// Where the other player stands, world metres (X, feet Y, Z), if they're
    /// drawn.
    pub them: Option<[f32; 3]>,
    /// In an open trade, who has accepted.
    pub mine: bool,
    pub theirs: bool,
}

/// The Exchange hall as scripts see it, in world metres.
#[derive(Debug, Clone, PartialEq)]
pub struct HallMark {
    /// Its footprint, in order round it.
    pub corners: [[f32; 2]; 4],
    /// The middle of its entrance side.
    pub door: [f32; 2],
    /// The middle of its roof (X, Y, Z), where the mote rises from.
    pub roof: [f32; 3],
    /// You're in range to trade there.
    pub near: bool,
    /// Goods wait in your market box.
    pub goods: bool,
    /// The market panel is open.
    pub open: bool,
    /// The Gazette's notice board in its forecourt (E3.9): round its
    /// slot, and the middle of its roof (X, Y, Z), where its prompt hangs.
    pub board: [[f32; 2]; 4],
    pub board_top: [f32; 3],
    /// An edition is out that you haven't read.
    pub news: bool,
    /// How many commissions on its Wanted page your pack could fill (S2.3)
    pub fill: usize,
    /// The Gazette is open.
    pub reading: bool,
    /// The Deeds Office's desk beside it (#448): round its slot, and the
    /// middle of its awning (X, Y, Z), where its prompt hangs.
    pub office: [[f32; 2]; 4],
    pub office_top: [f32; 3],
    /// Your deed waits there, earned and not yet spent.
    pub deed_waits: bool,
    /// The Deeds Office's panel is open.
    pub office_open: bool,
}

/// The bazaar (E3.11) as scripts see it: its stalls, in world metres.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BazaarMark {
    pub stalls: Vec<StallMark>,
    /// The stall you're walking to, if any.
    pub walking: Option<String>,
}

/// Where the road in's card leads (#442), in world metres, for its gold
/// ring (`input.rhai`'s `draw_lead`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LeadMark {
    /// The ring's middle, on the ground.
    pub at: [f32; 2],
    /// Its radius.
    pub radius: f32,
    /// A lot's edge, traced with it; empty for anything else.
    pub outline: Vec<[f32; 2]>,
}

/// One stall in the bazaar.
#[derive(Debug, Clone, PartialEq)]
pub struct StallMark {
    pub id: String,
    /// Its footprint, in order round it.
    pub corners: [[f32; 2]; 4],
    /// Where its name hangs: the middle of its sign, or over an empty
    /// counter (X, Y, Z).
    pub sign: [f32; 3],
    /// The way its sign's front faces, how wide a name may run on it, and
    /// how far each face stands from `sign` (`world::bazaar::Sign`).
    pub sign_facing: [f32; 3],
    pub sign_width: f32,
    pub sign_depth: f32,
    /// Its shop's colour, linear RGB.
    pub colour: [f32; 3],
    /// Its shop's name and keeper, if it's kept.
    pub name: Option<String>,
    pub keeper: Option<String>,
    pub citizen: bool,
    /// It's yours.
    pub mine: bool,
    /// You're in range to buy or stock there.
    pub near: bool,
    /// Its shop panel is open.
    pub open: bool,
}

impl BazaarMark {
    /// The stall whose footprint holds the ground point (x, z), if any.
    pub fn stall_at(&self, x: f32, z: f32) -> Option<&StallMark> {
        self.stalls.iter().find(|s| quad_covers(&s.corners, x, z, 0.0))
    }
}

impl HallMark {
    /// Whether the ground point (x, z) is on the footprint, or within `pad`
    /// metres of it.
    pub fn covers(&self, x: f32, z: f32, pad: f32) -> bool {
        quad_covers(&self.corners, x, z, pad)
    }

    /// Whether the ground point (x, z) is on the notice board's slot.
    pub fn board_covers(&self, x: f32, z: f32) -> bool {
        quad_covers(&self.board, x, z, 0.1)
    }

    /// Whether the ground point (x, z) is on the Deeds Office's slot.
    pub fn office_covers(&self, x: f32, z: f32) -> bool {
        quad_covers(&self.office, x, z, 0.1)
    }
}

/// Whether the ground point (x, z) is in the convex quad `c`, or within
/// `pad` metres of it.
fn quad_covers(c: &[[f32; 2]; 4], x: f32, z: f32, pad: f32) -> bool {
    // Inside a convex quad: the same side of every edge.
    let side = |i: usize| {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        (b[0] - a[0]) * (z - a[1]) - (b[1] - a[1]) * (x - a[0])
    };
    let sides = [side(0), side(1), side(2), side(3)];
    if sides.iter().all(|&s| s >= 0.0) || sides.iter().all(|&s| s <= 0.0) {
        return true;
    }
    (0..4).any(|i| {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        let ab = [b[0] - a[0], b[1] - a[1]];
        let t = (((x - a[0]) * ab[0] + (z - a[1]) * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-9))
            .clamp(0.0, 1.0);
        let p = [a[0] + ab[0] * t, a[1] + ab[1] * t];
        (x - p[0]).hypot(z - p[1]) <= pad
    })
}

/// One of the UI's landmarks as scripts see it: a rect in logical points,
/// as the script HUD draws in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiAnchor {
    /// Left, top, width, height.
    pub rect: [f32; 4],
    /// It's an open panel, not the button that opens it.
    pub panel: bool,
}

/// How long a landmark glows after `cap_ui_pulse`, in seconds.
pub const PULSE: f32 = 0.75;

impl Shared {
    /// How strongly the landmark `name` glows now: 1 as it's lit, easing
    /// to 0 over [`PULSE`].
    pub fn pulse(&self, name: &str, now: Instant) -> f32 {
        let Some(at) = self.pulses.lock().ok().and_then(|p| p.get(name).copied()) else {
            return 0.0;
        };
        let k = 1.0 - now.saturating_duration_since(at).as_secs_f32() / PULSE;
        k.clamp(0.0, 1.0).powf(1.5)
    }
}

/// The picked plot as scripts see it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlotMark {
    /// Its edge as drawn, world metres (X, Z), in order round it.
    pub outline: Vec<[f32; 2]>,
    /// The height of its top, metres.
    pub y: f32,
    /// It's yours.
    pub mine: bool,
    /// Someone owns it.
    pub owned: bool,
}

/// The station being placed, as scripts see it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementMark {
    /// As in [`StationMark::kind`].
    pub kind: &'static str,
    /// The kit's name, "Workbench Kit".
    pub kit: String,
    /// Where the ghost is, world metres (X, Z). `None` while the cursor is
    /// off the ground.
    pub at: Option<[f32; 2]>,
    /// Its ring's radius, metres, as the in-range ring's.
    pub radius: f32,
    /// It can go down where the ghost is.
    pub ok: bool,
    /// Why not, in words, when it can't.
    pub why: Option<String>,
    /// The click went out, and the server hasn't answered.
    pub asking: bool,
    /// Where you stand (world metres) and how far you can place (metres).
    pub you: Option<[f32; 2]>,
    pub reach: f32,
    /// How far apart stations must stand (metres), and the stations within
    /// a few of that of the ghost, world metres.
    pub spacing: f32,
    pub crowding: Vec<[f32; 2]>,
    /// Your plots' corners, world metres, for their outlines.
    pub plots: Vec<[[f32; 2]; 4]>,
}

/// The zone banner's words and colour, and when it started.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneBanner {
    pub name: String,
    /// sRGB, 0 to 1.
    pub colour: [f32; 3],
    /// When the banner starts; it may be a moment ahead.
    pub at: Instant,
    /// The zone you came into the world in, not one you walked into.
    pub first: bool,
}

/// One resource node as scripts see it.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeMark {
    pub id: String,
    /// Where it stands, world metres (X, Z).
    pub at: [f32; 2],
    /// Its model (`nodes::models::Kind::name`): "grass", "tree", ...
    pub kind: &'static str,
    /// The radius of the ring round it, metres: clear of a trunk or a rock.
    pub r: f32,
    /// "available", "harvesting" or "depleted".
    pub state: &'static str,
    /// You're walking to it to extract, or have asked to.
    pub targeted: bool,
    /// You're extracting it.
    pub mine: bool,
    /// How far the extraction on it has got, 0 to 1, if one is under way.
    pub progress: Option<f32>,
}

/// One station as scripts see it.
#[derive(Debug, Clone, PartialEq)]
pub struct StationMark {
    pub id: String,
    /// "workbench", "forge", "storage_chest", "sawbench", "kiln", "anvil",
    /// or "generic" (a crate) for a type the client has no model for.
    pub kind: &'static str,
    /// Where it stands, world metres (X, Z).
    pub at: [f32; 2],
    /// The in-range ring's radius, metres: just round the model.
    pub radius: f32,
    /// You're within its interaction range (`capitol_core::station`).
    pub in_reach: bool,
    /// 0 to 1: the ring, eased toward `in_reach`.
    pub reach: f32,
    /// How far over the ground its top is, m: where its ember floats.
    pub top: f32,
    /// Yours, and waiting (E4.4): why, in a few words ("Out of iron ore").
    pub paused: Option<String>,
    /// Yours (#487): a gold mark floats over it, so yours stands out in a
    /// crowd of the same.
    pub mine: bool,
    /// Out in the weather and below `rules::weather::NEEDS_REPAIR` (V3.4):
    /// its condition, for the mark that says it needs repair.
    pub worn: Option<i32>,
}

/// How near a stall you stand to be in the bazaar, in metres: its streets'
/// crowns are all within it (E3.13).
pub const IN_BAZAAR: f32 = 2.0;

/// How near a click must land to a player's feet to pick them, in metres.
/// An avatar is about 0.7 m across.
pub const PLAYER_PICK: f32 = 1.2;

/// How near a click must land to a node to pick it, in metres. A node's
/// tuft is about 1.2 m across; React picks within 16 server units.
pub const NODE_PICK: f32 = 1.8;

/// How far off the Exchange hall's footprint a click still picks it, in
/// metres: the eaves and the stalls' awnings reach about this far.
pub const HALL_PICK: f32 = 0.3;

/// One avatar's name tag, as `entities::Players` placed it this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerLabel {
    pub id: uuid::Uuid,
    pub name: String,
    /// Where the name hangs, over the head, in world metres.
    pub at: [f32; 3],
    /// Where they stand: the height of their feet, in world metres.
    pub feet_y: f32,
    /// 0 to 1 as the player fades in or out.
    pub alpha: f32,
    /// You.
    pub me: bool,
    /// A citizen (C0.1): their name carries the glyph (C0.2).
    pub citizen: bool,
    /// Their guild's tag (S4.4), after the name.
    pub tag: Option<String>,
    /// How far their craft has got, 0 to 1, while they craft (P5.5).
    pub craft: Option<f32>,
    /// What they're saying (Commons S1.2).
    pub line: Option<Speech>,
}

/// A line over a player's head (Commons S1.2), as the talk log has it this
/// frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    pub text: String,
    /// The bubble's alpha as it opens and fades.
    pub alpha: f32,
    /// Seconds since it was said: the newest bubble hangs lowest.
    pub age: f32,
}

impl Speech {
    /// From what `talk::Log::bubble` gives.
    pub fn of((text, alpha, age): (&str, f32, f64)) -> Self {
        Self {
            text: text.to_owned(),
            alpha,
            age: age as f32,
        }
    }
}

/// A tag's `bubble` for `hud.rhai`: `#{ x, y, w, h, size, a, tip_x, tip_y,
/// rows: [#{ text, x, y }], caption, paper, edge, ink, dim }`. The box's
/// top left and size, where its tail points, each row's centre and top, the
/// speaker's name when it had no room over their head (`#{ text, x, y,
/// size, glyph }`, the name's top left, or `()`), and the theme's paper and
/// inks as `[r, g, b]`, so the HUD keeps the panels' look.
fn bubble_map(b: &crate::labels::Bubble) -> rhai::Map {
    use crate::ui::theme;
    let f = |v: f32| rhai::Dynamic::from(v as f64);
    let rgb = |c: egui::Color32| {
        rhai::Dynamic::from_array(
            [c.r(), c.g(), c.b()]
                .iter()
                .map(|v| f(*v as f32 / 255.0))
                .collect(),
        )
    };
    let mut m = rhai::Map::new();
    for (k, v) in [
        ("x", b.x),
        ("y", b.y),
        ("w", b.w),
        ("h", b.h),
        ("size", b.size),
        ("a", b.alpha),
        ("tip_x", b.tip[0]),
        ("tip_y", b.tip[1]),
    ] {
        m.insert(k.into(), f(v));
    }
    let rows: rhai::Array = b
        .rows
        .iter()
        .map(|r| {
            let mut row = rhai::Map::new();
            row.insert("text".into(), r.text.clone().into());
            row.insert("x".into(), f(r.x));
            row.insert("y".into(), f(r.y));
            row.into()
        })
        .collect();
    m.insert("rows".into(), rows.into());
    let caption = b.caption.as_ref().map_or(rhai::Dynamic::UNIT, |c| {
        let mut m = rhai::Map::new();
        m.insert("text".into(), c.text.clone().into());
        m.insert("x".into(), f(c.x));
        m.insert("y".into(), f(c.y));
        m.insert("size".into(), f(c.size));
        let glyph = c.glyph.map_or(rhai::Dynamic::UNIT, |[x, y, r]| {
            let mut g = rhai::Map::new();
            g.insert("x".into(), f(x));
            g.insert("y".into(), f(y));
            g.insert("r".into(), f(r));
            g.into()
        });
        m.insert("glyph".into(), glyph);
        m.into()
    });
    m.insert("caption".into(), caption);
    m.insert("paper".into(), rgb(theme::PAPER));
    m.insert("edge".into(), rgb(theme::PAPER_EDGE));
    m.insert("ink".into(), rgb(theme::INK));
    m.insert("dim".into(), rgb(theme::INK_DIM));
    m
}

/// `cap_debug()`'s map.
fn debug_map(mark: &crate::debug_layers::Mark) -> rhai::Map {
    let f = |v: f32| rhai::Dynamic::from(v as f64);
    let pair = |p: [f32; 2]| rhai::Dynamic::from_array(vec![f(p[0]), f(p[1])]);
    let mut m = rhai::Map::new();
    m.insert("layer".into(), mark.layer.into());
    let cells: rhai::Array = mark
        .cells
        .iter()
        .map(|c| {
            let mut cell = rhai::Map::new();
            cell.insert("x0".into(), f(c.min[0]));
            cell.insert("z0".into(), f(c.min[1]));
            cell.insert("x1".into(), f(c.max[0]));
            cell.insert("z1".into(), f(c.max[1]));
            cell.insert("label".into(), c.label.clone().into());
            cell.insert("mine".into(), c.mine.into());
            cell.into()
        })
        .collect();
    m.insert("cells".into(), cells.into());
    let nodes: rhai::Array = mark
        .nodes
        .iter()
        .map(|n| {
            let mut node = rhai::Map::new();
            node.insert("x".into(), f(n.at[0]));
            node.insert("z".into(), f(n.at[1]));
            node.insert("kind".into(), n.kind.clone().into());
            node.insert("ground".into(), n.ground.clone().into());
            node.insert("available".into(), n.available.into());
            node.into()
        })
        .collect();
    m.insert("nodes".into(), nodes.into());
    let edges: rhai::Array = mark
        .edges
        .iter()
        .map(|e| rhai::Dynamic::from_array(e.iter().map(|&v| f(v)).collect()))
        .collect();
    m.insert("edges".into(), edges.into());
    let walkers: rhai::Array = mark
        .walkers
        .iter()
        .map(|w| {
            let mut walker = rhai::Map::new();
            walker.insert("x".into(), f(w.at[0]));
            walker.insert("z".into(), f(w.at[1]));
            walker.insert("dest".into(), w.dest.map_or(rhai::Dynamic::UNIT, pair));
            walker.insert("name".into(), w.name.clone().into());
            walker.insert("citizen".into(), w.citizen.into());
            walker.insert("doing".into(), w.doing.clone().into());
            walker.into()
        })
        .collect();
    m.insert("walkers".into(), walkers.into());
    m
}

pub fn register(engine: &mut rhai::Engine, shared: &Shared) {
    // `cap_spike_note() -> String`: the text written over the Capitol.
    let note = shared.note.clone();
    engine.register_fn("cap_spike_note", move || -> String {
        note.lock().map(|s| s.clone()).unwrap_or_default()
    });
    // `cap_view() -> String`: which view the camera rig starts on.
    let view = shared.view.clone();
    engine.register_fn("cap_view", move || -> String {
        view.lock().map(|s| s.clone()).unwrap_or_default()
    });
    // `cap_world_radius() -> f64`: the world disc's radius in metres
    // (`rules::WILDERNESS_OUTER_RADIUS`), where the free point stops.
    engine.register_fn("cap_world_radius", || -> f64 {
        capitol_protocol::rules::WILDERNESS_OUTER_RADIUS * f64::from(crate::coords::S)
    });
    // `cap_camera_path(t) -> #{ pos, at, done }`: the camera move from
    // --camera-path at game time t, `done` once it has reached its last key.
    // () with no path.
    let path = shared.camera_path.clone();
    engine.register_fn("cap_camera_path", move |t: f64| -> rhai::Dynamic {
        let path = path.lock().unwrap();
        let Some(path) = path.as_ref() else {
            return rhai::Dynamic::UNIT;
        };
        let p = path.at(t);
        let mut m = rhai::Map::new();
        let v = |a: &[f64]| a.iter().map(|&x| x.into()).collect::<rhai::Array>();
        m.insert("pos".into(), v(&p[..3]).into());
        m.insert("at".into(), v(&p[3..]).into());
        m.insert("done".into(), (t >= path.end()).into());
        m.into()
    });
    // `cap_clean() -> bool`: a clean capture (--clean), so draw no marks
    // that are only there to steer by.
    let clean = shared.clean.clone();
    engine.register_fn("cap_clean", move || clean.load(Ordering::Relaxed));
    // `cap_setting(name) -> value`: one of the player's settings (P8.4),
    // for the scripts that need it: "camera_sensitivity" (a multiple of the
    // rig's turn and zoom rates, 1.0 as made), "invert_orbit" (bool),
    // "control_mode" ("click" or "direct", the one in play, T1.1),
    // "look_sensitivity" (a multiple of the action camera's mouse-look) and
    // "invert_look_y" (bool). () for any other name. Settings are read here
    // rather than from the engine's store; see settings.rs for why.
    let feel = shared.camera_feel.clone();
    let controls = shared.controls.clone();
    engine.register_fn("cap_setting", move |name: &str| -> rhai::Dynamic {
        let feel = feel.lock().map(|f| *f).unwrap_or_default();
        let controls = controls.lock().map(|c| *c).unwrap_or_default();
        match name {
            "camera_sensitivity" => (feel.sensitivity as f64).into(),
            "invert_orbit" => feel.invert_orbit.into(),
            "control_mode" => controls.mode.name().into(),
            "look_sensitivity" => (controls.look_sensitivity as f64).into(),
            "invert_look_y" => controls.invert_look_y.into(),
            _ => rhai::Dynamic::UNIT,
        }
    });
    // `cap_view_yaw(yaw)`: the camera rig's yaw as it means it to be, in
    // radians round the point it looks at, 0 standing it south. Direct
    // mode's keys walk against it (T1.3), not against the camera, which
    // drifts round the keep and eases out of a shot.
    let view_yaw = shared.view_yaw.clone();
    engine.register_fn("cap_view_yaw", move |yaw: f64| {
        if let Ok(mut v) = view_yaw.lock() {
            *v = yaw.is_finite().then_some(yaw);
        }
    });
    // `cap_wants_cursor() -> bool`: something wants the cursor free (T1.4):
    // a panel, the login, Alt, a station being placed, or Escape. In Direct
    // the camera rig captures it whenever nothing does.
    let wants = shared.wants_cursor.clone();
    engine.register_fn("cap_wants_cursor", move || wants.load(Ordering::Relaxed));
    // `cap_cursor_let_go() -> bool`: Escape let the cursor go with no panel
    // open, so a click on the world takes it back rather than acting, with
    // `cap_take_cursor()` (T1.4).
    let let_go = shared.cursor_let_go.clone();
    engine.register_fn("cap_cursor_let_go", move || let_go.load(Ordering::Relaxed));
    let let_go = shared.cursor_let_go.clone();
    engine.register_fn("cap_take_cursor", move || let_go.store(false, Ordering::Relaxed));
    // `cap_at_login() -> bool`: the login form is up, so the camera rig
    // circles the tower instead of taking input.
    let at_login = shared.at_login.clone();
    engine.register_fn("cap_at_login", move || at_login.load(Ordering::Relaxed));
    // `cap_follow() -> [x, y, z] or ()`: where you stand, in world metres,
    // for the camera rig. () offline, and until the server has placed you.
    let follow = shared.follow.clone();
    engine.register_fn("cap_follow", move || -> rhai::Dynamic {
        match follow.lock().ok().and_then(|f| *f) {
            Some(p) => rhai::Dynamic::from_array(p.iter().map(|v| (*v as f64).into()).collect()),
            None => rhai::Dynamic::UNIT,
        }
    });
    // `cap_players() -> [#{ name, x, y, z, feet_y, alpha, me, craft }]`:
    // every avatar's name and where it hangs (world metres), the height of
    // its feet, how far it has faded in, and how far its craft has got
    // (0 to 1, or () when it isn't crafting).
    let players = shared.players.clone();
    engine.register_fn("cap_players", move || -> rhai::Array {
        let Ok(players) = players.lock() else {
            return rhai::Array::new();
        };
        players
            .iter()
            .map(|p| {
                let mut m = rhai::Map::new();
                m.insert("name".into(), p.name.clone().into());
                m.insert("x".into(), (p.at[0] as f64).into());
                m.insert("y".into(), (p.at[1] as f64).into());
                m.insert("z".into(), (p.at[2] as f64).into());
                m.insert("alpha".into(), (p.alpha as f64).into());
                m.insert("me".into(), p.me.into());
                m.insert("feet_y".into(), (p.feet_y as f64).into());
                m.insert(
                    "craft".into(),
                    p.craft.map_or(rhai::Dynamic::UNIT, |c| (c as f64).into()),
                );
                m.into()
            })
            .collect()
    });
    // `cap_name_tags() -> [#{ name, x, y, size, a, me, glyph, named,
    // bubble }]`: the names to draw, in order, projected with the camera
    // this frame renders with. Each tag's x, y is its name's top left. A
    // citizen's `glyph` is `#{ x, y, r }`, the lozenge's centre and
    // half-height; anyone else's is `()`. Someone talking has a `bubble`
    // (S1.2, see `bubble_map`); anyone else's is `()`. A speaker whose name
    // had no room isn't `named`: their name is the bubble's caption. See
    // `labels::place`.
    let players = shared.players.clone();
    let egui = shared.egui.clone();
    let projector = shared.projector.clone();
    engine.register_fn(
        "cap_name_tags",
        move || -> rhai::Array {
            let Some(view) = projector.as_ref().map(|p| p.view()) else {
                return rhai::Array::new();
            };
            let Ok(players) = players.lock() else {
                return rhai::Array::new();
            };
            let screens: Vec<labels::Screen> = players
                .iter()
                .map(|p| view.world_to_screen(p.at).map(|s| [s.x, s.y, s.depth]))
                .collect();
            let egui = egui.lock().ok().and_then(|e| e.clone());
            let measure = |text: &str, size: f32| match &egui {
                Some(ctx) => ctx.fonts(|f| {
                    let size = f
                        .layout_no_wrap(
                            text.to_owned(),
                            egui::FontId::proportional(size),
                            egui::Color32::WHITE,
                        )
                        .size();
                    (size.x, size.y)
                }),
                // Flint's own fallback, before the fonts are in.
                None => (text.chars().count() as f32 * size * 0.6, size),
            };
            labels::place(&players, &screens, view.screen_size(), measure)
                .into_iter()
                .map(|t| {
                    let mut m = rhai::Map::new();
                    m.insert("name".into(), t.name.into());
                    m.insert("x".into(), (t.x as f64).into());
                    m.insert("y".into(), (t.y as f64).into());
                    m.insert("size".into(), (t.size as f64).into());
                    m.insert("a".into(), (t.alpha as f64).into());
                    m.insert("me".into(), t.me.into());
                    let glyph = t.glyph.map_or(rhai::Dynamic::UNIT, |[x, y, r]| {
                        let mut g = rhai::Map::new();
                        g.insert("x".into(), (x as f64).into());
                        g.insert("y".into(), (y as f64).into());
                        g.insert("r".into(), (r as f64).into());
                        g.into()
                    });
                    m.insert("glyph".into(), glyph);
                    let tag = t.tag.map_or(rhai::Dynamic::UNIT, |g| {
                        let mut m = rhai::Map::new();
                        m.insert("text".into(), g.text.into());
                        m.insert("x".into(), (g.x as f64).into());
                        m.insert("y".into(), (g.y as f64).into());
                        m.insert("size".into(), (g.size as f64).into());
                        m.into()
                    });
                    m.insert("tag".into(), tag);
                    m.insert("named".into(), t.named.into());
                    let bubble = t.bubble.map_or(rhai::Dynamic::UNIT, |b| bubble_map(&b).into());
                    m.insert("bubble".into(), bubble);
                    m.into()
                })
                .collect()
        },
    );
    // `cap_player_at(x, z) -> #{ id, name, x, z, feet_y } or ()`: the other
    // player whose feet are nearest the ground point (x, z), within
    // PLAYER_PICK metres, and where they stand. Never you.
    let players = shared.players.clone();
    engine.register_fn("cap_player_at", move |x: f64, z: f64| -> rhai::Dynamic {
        let Ok(players) = players.lock() else {
            return rhai::Dynamic::UNIT;
        };
        match player_at(&players, x as f32, z as f32) {
            Some(p) => {
                let mut m = rhai::Map::new();
                m.insert("id".into(), p.id.to_string().into());
                m.insert("name".into(), p.name.clone().into());
                m.insert("x".into(), (p.at[0] as f64).into());
                m.insert("z".into(), (p.at[2] as f64).into());
                m.insert("feet_y".into(), (p.feet_y as f64).into());
                m.into()
            }
            None => rhai::Dynamic::UNIT,
        }
    });
    // `cap_trade_with(id)`: ask the player `id` (from `cap_player_at`) to
    // trade: at once within range, otherwise after walking closer. Sent
    // next frame.
    let trade_with = shared.trade_with.clone();
    engine.register_fn("cap_trade_with", move |id: &str| {
        if let Ok(mut t) = trade_with.lock() {
            *t = Some(id.to_owned());
        }
    });
    // `cap_open_card(id)`: open the card of the player `id` (from
    // `cap_player_at`): who they are (S0.3). Opened next frame.
    let open_card = shared.open_card.clone();
    engine.register_fn("cap_open_card", move |id: &str| {
        if let Ok(mut c) = open_card.lock() {
            *c = Some(id.to_owned());
        }
    });
    // `cap_trade() -> #{ state, x, y, z, mine, theirs } or ()`: where trading
    // stands. `state` is "asking", "incoming", "walking" or "open"; x, y, z
    // are the other player's feet in world metres, or () if they aren't
    // drawn; `mine` and `theirs` say who has accepted an open trade.
    let trade = shared.trade.clone();
    engine.register_fn("cap_trade", move || -> rhai::Dynamic {
        let Some(t) = trade.lock().ok().and_then(|t| t.clone()) else {
            return rhai::Dynamic::UNIT;
        };
        let mut m = rhai::Map::new();
        m.insert("state".into(), t.state.into());
        let at = |i: usize| t.them.map_or(rhai::Dynamic::UNIT, |p| (p[i] as f64).into());
        m.insert("x".into(), at(0));
        m.insert("y".into(), at(1));
        m.insert("z".into(), at(2));
        m.insert("mine".into(), t.mine.into());
        m.insert("theirs".into(), t.theirs.into());
        m.into()
    });
    // `cap_move(x, z)`: walk to the ground point (x, z), in world metres.
    // It's clamped to the world's edge and sent next frame. Offline (but
    // for --offline-avatar), and before the server has placed you, it does
    // nothing.
    let move_to = shared.move_to.clone();
    engine.register_fn("cap_move", move |x: f64, z: f64| {
        if let Ok(mut m) = move_to.lock() {
            *m = Some(clamp_to_world(x as f32, z as f32));
        }
    });
    // `cap_destination() -> [x, z] or ()`: where you're walking to, in
    // world metres. () when you're standing, or following a walk the
    // server resumed after a reconnect.
    let destination = shared.destination.clone();
    engine.register_fn("cap_destination", move || -> rhai::Dynamic {
        match destination.lock().ok().and_then(|d| *d) {
            Some(p) => rhai::Dynamic::from_array(p.iter().map(|v| (*v as f64).into()).collect()),
            None => rhai::Dynamic::UNIT,
        }
    });
    // `cap_route() -> [[x, z], ...]`: the corners of your walk still to
    // pass, round the water by the bridges and fords, in world metres: the
    // one you're walking to first, the destination last. [] when standing.
    let route = shared.route.clone();
    engine.register_fn("cap_route", move || -> rhai::Array {
        route
            .lock()
            .map(|r| {
                r.iter()
                    .map(|p| rhai::Dynamic::from_array(p.iter().map(|v| (*v as f64).into()).collect()))
                    .collect()
            })
            .unwrap_or_default()
    });
    // `cap_debug() -> #{ layer, cells, nodes, edges, walkers } or ()`: what
    // the admin's world layers draw (D5.3), in world metres; () while
    // they're off. `cells` are `#{ x0, z0, x1, z1, label, mine }`, `nodes`
    // `#{ x, z, kind, ground, available }`, `edges` `[x0, z0, x1, z1]`, and
    // `walkers` `#{ x, z, dest, name, citizen, doing }` with `dest` `[x, z]`
    // or ().
    let debug = shared.debug.clone();
    engine.register_fn("cap_debug", move || -> rhai::Dynamic {
        let Some(mark) = debug.lock().ok().and_then(|d| d.clone()) else {
            return rhai::Dynamic::UNIT;
        };
        debug_map(&mark).into()
    });
    // `cap_zone_banner() -> #{ name, r, g, b, age, first } or ()`: the zone
    // you last entered, its colour (sRGB) and how many seconds ago its
    // banner started. `age` is negative while it waits to start. `first`
    // when it's the zone you came into the world in. () offline, and
    // signed out.
    let banner = shared.banner.clone();
    engine.register_fn("cap_zone_banner", move || -> rhai::Dynamic {
        let Some(b) = banner.lock().ok().and_then(|b| b.clone()) else {
            return rhai::Dynamic::UNIT;
        };
        let now = Instant::now();
        let age = if now >= b.at {
            (now - b.at).as_secs_f64()
        } else {
            -(b.at - now).as_secs_f64()
        };
        let mut m = rhai::Map::new();
        m.insert("name".into(), b.name.into());
        m.insert("r".into(), (b.colour[0] as f64).into());
        m.insert("g".into(), (b.colour[1] as f64).into());
        m.insert("b".into(), (b.colour[2] as f64).into());
        m.insert("age".into(), age.into());
        m.insert("first".into(), b.first.into());
        m.into()
    });
    // `cap_nodes() -> [#{ id, kind, x, z, r, state, targeted, mine, progress }]`:
    // the resource nodes someone is working or you're aiming at, at ground
    // level in world metres. Only those: the store holds every node in the
    // world (E2.5), and a script loop over a thousand would cost the frame.
    // `kind` is the model ("grass", "tree", "branches", "outcrop", "vein",
    // "clay", "other"), `r` the radius of the ring round it. `state` is
    // "available", "harvesting" or "depleted"; `progress` is 0 to 1 while
    // someone extracts, otherwise ().
    let nodes = shared.nodes.clone();
    engine.register_fn("cap_nodes", move || -> rhai::Array {
        let Ok(nodes) = nodes.lock() else {
            return rhai::Array::new();
        };
        nodes
            .iter()
            .filter(|n| n.targeted || n.progress.is_some())
            .map(|n| node_map(n).into())
            .collect()
    });
    // `cap_stations() -> [#{ id, kind, x, z, radius, in_reach, reach, top,
    // mine, paused, worn }]`: the stations shown, in world metres. `in_reach`
    // is whether you're within its interaction range; `reach` eases from 0
    // to 1 with it. `mine` is whether it's yours (#487). `worn` is its
    // condition when the weather has worn it to needing repair, else ().
    // (V3.4)
    let stations = shared.stations.clone();
    engine.register_fn("cap_stations", move || -> rhai::Array {
        let Ok(stations) = stations.lock() else {
            return rhai::Array::new();
        };
        stations.iter().map(|s| station_map(s).into()).collect()
    });
    // `cap_station_at(x, z) -> #{ ... } or ()`: the station whose ring the
    // ground point (x, z) falls inside, the nearest if several, as in
    // `cap_stations`.
    let stations = shared.stations.clone();
    engine.register_fn("cap_station_at", move |x: f64, z: f64| -> rhai::Dynamic {
        let Ok(stations) = stations.lock() else {
            return rhai::Dynamic::UNIT;
        };
        match station_at(&stations, x as f32, z as f32) {
            Some(s) => station_map(s).into(),
            None => rhai::Dynamic::UNIT,
        }
    });
    // `cap_open_station(id)`: open the station `id` (from `cap_station_at`):
    // at once within reach, otherwise after walking to it. Sent next frame.
    let open_station = shared.open_station.clone();
    engine.register_fn("cap_open_station", move |id: &str| {
        if let Ok(mut o) = open_station.lock() {
            *o = Some(id.to_owned());
        }
    });
    // `cap_hall() -> #{ corners, door_x, door_z, x, y, z, near, goods, open,
    // board_x, board_y, board_z, news, fill, reading, office_x, office_y, office_z,
    // deed_waits, office_open } or ()`: the Exchange hall
    // (E3.7), world metres. `board_*` is the top of the Gazette's notice
    // board in its forecourt (E3.9), `news` an edition you haven't read,
    // `fill` how many commissions on its Wanted page your pack could fill
    // (S2.3), and `reading` the board open. `office_*` is the top of the Deeds
    // Office's awning beside it (#448), `deed_waits` your deed earned and
    // unspent, and `office_open` its panel open. `corners` is
    // [[x, z], ...] round its footprint, `door_*` the middle of its entrance
    // side, and `x, y, z` the middle of its roof. `near` when you're in range
    // to trade there, `goods` when your market box holds anything, `open`
    // when the market panel is.
    let hall = shared.hall.clone();
    engine.register_fn("cap_hall", move || -> rhai::Dynamic {
        let Some(h) = hall.lock().ok().and_then(|h| h.clone()) else {
            return rhai::Dynamic::UNIT;
        };
        let mut m = rhai::Map::new();
        let corners: rhai::Array = h
            .corners
            .iter()
            .map(|p| rhai::Dynamic::from_array(vec![(p[0] as f64).into(), (p[1] as f64).into()]))
            .collect();
        m.insert("corners".into(), corners.into());
        m.insert("door_x".into(), (h.door[0] as f64).into());
        m.insert("door_z".into(), (h.door[1] as f64).into());
        m.insert("x".into(), (h.roof[0] as f64).into());
        m.insert("y".into(), (h.roof[1] as f64).into());
        m.insert("z".into(), (h.roof[2] as f64).into());
        m.insert("near".into(), h.near.into());
        m.insert("goods".into(), h.goods.into());
        m.insert("open".into(), h.open.into());
        m.insert("board_x".into(), (h.board_top[0] as f64).into());
        m.insert("board_y".into(), (h.board_top[1] as f64).into());
        m.insert("board_z".into(), (h.board_top[2] as f64).into());
        m.insert("news".into(), h.news.into());
        m.insert("fill".into(), (h.fill as i64).into());
        m.insert("reading".into(), h.reading.into());
        m.insert("office_x".into(), (h.office_top[0] as f64).into());
        m.insert("office_y".into(), (h.office_top[1] as f64).into());
        m.insert("office_z".into(), (h.office_top[2] as f64).into());
        m.insert("deed_waits".into(), h.deed_waits.into());
        m.insert("office_open".into(), h.office_open.into());
        m.into()
    });
    // `cap_board_at(x, z) -> bool`: whether the ground point (x, z), world
    // metres, is on the Gazette's notice board (E3.9).
    let hall = shared.hall.clone();
    engine.register_fn("cap_board_at", move |x: f64, z: f64| -> bool {
        hall.lock()
            .ok()
            .and_then(|h| h.as_ref().map(|h| h.board_covers(x as f32, z as f32)))
            .unwrap_or(false)
    });
    // `cap_read_gazette()`: open the Gazette. Sent next frame.
    let read = shared.read_gazette.clone();
    engine.register_fn("cap_read_gazette", move || read.store(true, Ordering::Relaxed));
    // `cap_office_at(x, z) -> bool`: whether the ground point (x, z), world
    // metres, is on the Deeds Office's desk (#448).
    let hall = shared.hall.clone();
    engine.register_fn("cap_office_at", move |x: f64, z: f64| -> bool {
        hall.lock()
            .ok()
            .and_then(|h| h.as_ref().map(|h| h.office_covers(x as f32, z as f32)))
            .unwrap_or(false)
    });
    // `cap_open_office()`: look in at the Deeds Office. Sent next frame.
    let office = shared.open_office.clone();
    engine.register_fn("cap_open_office", move || office.store(true, Ordering::Relaxed));
    // `cap_hall_at(x, z) -> bool`: whether the ground point (x, z), world
    // metres, is on the hall's footprint (or a hand's width off it).
    let hall = shared.hall.clone();
    engine.register_fn("cap_hall_at", move |x: f64, z: f64| -> bool {
        hall.lock()
            .ok()
            .and_then(|h| h.as_ref().map(|h| h.covers(x as f32, z as f32, HALL_PICK)))
            .unwrap_or(false)
    });
    // `cap_visit_hall()`: open the market at the hall, or walk to its door
    // and open it there. Sent next frame.
    let visit = shared.visit_hall.clone();
    engine.register_fn("cap_visit_hall", move || visit.store(true, Ordering::Relaxed));
    // `cap_bazaar() -> #{ stalls, walking } or ()`: the bazaar (E3.11), world
    // metres. Each of `stalls` is #{ id, number, corners, x, y, z, r, g, b, name,
    // keeper, citizen, mine, near, open, seen }: `corners` round its footprint,
    // `x, y, z` where its name hangs, `r, g, b` its shop's colour, `name`
    // and `keeper` () while it's to let, `near` when you're in its range,
    // `open` when its shop panel is, `seen` when no roof hides its sign from
    // the camera (#274). `walking` is the id of the stall
    // you're walking to, or ().
    let bazaar = shared.bazaar.clone();
    engine.register_fn("cap_bazaar", move || -> rhai::Dynamic {
        let Some(b) = bazaar.lock().ok().and_then(|b| b.clone()) else {
            return rhai::Dynamic::UNIT;
        };
        let opt = |s: &Option<String>| s.clone().map_or(rhai::Dynamic::UNIT, rhai::Dynamic::from);
        let stalls: rhai::Array = b
            .stalls
            .iter()
            .map(|s| {
                let mut m = rhai::Map::new();
                let corners: rhai::Array = s
                    .corners
                    .iter()
                    .map(|p| rhai::Dynamic::from_array(vec![(p[0] as f64).into(), (p[1] as f64).into()]))
                    .collect();
                m.insert("id".into(), s.id.clone().into());
                m.insert("number".into(), crate::ui::shop::stall_number(&s.id).to_owned().into());
                m.insert("corners".into(), corners.into());
                m.insert("x".into(), (s.sign[0] as f64).into());
                m.insert("y".into(), (s.sign[1] as f64).into());
                m.insert("z".into(), (s.sign[2] as f64).into());
                m.insert("fx".into(), (s.sign_facing[0] as f64).into());
                m.insert("fz".into(), (s.sign_facing[2] as f64).into());
                m.insert("sign_w".into(), (s.sign_width as f64).into());
                m.insert("sign_d".into(), (s.sign_depth as f64).into());
                m.insert("r".into(), (s.colour[0] as f64).into());
                m.insert("g".into(), (s.colour[1] as f64).into());
                m.insert("b".into(), (s.colour[2] as f64).into());
                m.insert("name".into(), opt(&s.name));
                m.insert("keeper".into(), opt(&s.keeper));
                m.insert("citizen".into(), s.citizen.into());
                m.insert("mine".into(), s.mine.into());
                m.insert("near".into(), s.near.into());
                m.insert("open".into(), s.open.into());
                rhai::Dynamic::from_map(m)
            })
            .collect();
        let mut m = rhai::Map::new();
        m.insert("stalls".into(), stalls.into());
        m.insert("walking".into(), opt(&b.walking));
        m.into()
    });
    // `cap_stall_at(x, z) -> id or ()`: the stall whose footprint holds the
    // ground point (x, z), world metres.
    let bazaar = shared.bazaar.clone();
    engine.register_fn("cap_stall_at", move |x: f64, z: f64| -> rhai::Dynamic {
        bazaar
            .lock()
            .ok()
            .and_then(|b| b.as_ref().and_then(|b| b.stall_at(x as f32, z as f32)).map(|s| s.id.clone()))
            .map_or(rhai::Dynamic::UNIT, rhai::Dynamic::from)
    });
    // `cap_visit_stall(id)`: open the stall's shop there, or walk to it
    // (in at the gate, up the lane) and open it on arrival. Sent next frame.
    let visit = shared.visit_stall.clone();
    engine.register_fn("cap_visit_stall", move |id: rhai::ImmutableString| {
        if let Ok(mut v) = visit.lock() {
            *v = Some(id.to_string());
        }
    });
    // `cap_lead() -> #{ x, z, r, outline } or ()`: where the road in's card
    // leads (#442), world metres: the ring's middle and radius, and a lot's
    // edge as [[x, z], ...] (empty for anything else). `()` while the card
    // leads nowhere or is hidden.
    let lead = shared.lead.clone();
    engine.register_fn("cap_lead", move || -> rhai::Dynamic {
        let Some(l) = lead.lock().ok().and_then(|l| l.clone()) else {
            return rhai::Dynamic::UNIT;
        };
        let mut m = rhai::Map::new();
        m.insert("x".into(), (l.at[0] as f64).into());
        m.insert("z".into(), (l.at[1] as f64).into());
        m.insert("r".into(), (l.radius as f64).into());
        let outline: rhai::Array = l
            .outline
            .iter()
            .map(|p| rhai::Dynamic::from_array(vec![(p[0] as f64).into(), (p[1] as f64).into()]))
            .collect();
        m.insert("outline".into(), outline.into());
        m.into()
    });
    // `cap_in_bazaar(x, z) -> bool`: whether the ground point (x, z), world
    // metres, is in the bazaar: on a stall or within 2 m of one, which takes
    // in its streets (E3.13).
    let feet = shared.stall_feet.clone();
    engine.register_fn("cap_in_bazaar", move |x: f64, z: f64| -> bool {
        feet.lock()
            .map(|f| f.iter().any(|q| quad_covers(q, x as f32, z as f32, IN_BAZAAR)))
            .unwrap_or(false)
    });
    // `cap_node_at(x, z) -> #{ ... } or ()`: the node nearest the ground
    // point (x, z), within NODE_PICK metres, as in `cap_nodes`.
    let nodes = shared.nodes.clone();
    engine.register_fn("cap_node_at", move |x: f64, z: f64| -> rhai::Dynamic {
        let Ok(nodes) = nodes.lock() else {
            return rhai::Dynamic::UNIT;
        };
        match node_at(&nodes, x as f32, z as f32) {
            Some(n) => node_map(n).into(),
            None => rhai::Dynamic::UNIT,
        }
    });
    // `cap_extract(id)`: extract from the node `id` (from `cap_node_at`):
    // at once within range, otherwise after walking there. Sent next frame.
    let extract = shared.extract.clone();
    engine.register_fn("cap_extract", move |id: &str| {
        if let Ok(mut e) = extract.lock() {
            *e = Some(id.to_owned());
        }
    });
    // `cap_pick_plot(x, z)`: show the plot under the ground point (x, z),
    // world metres, in the plot panel. Asked next frame; online only.
    let pick = shared.pick_plot.clone();
    engine.register_fn("cap_pick_plot", move |x: f64, z: f64| {
        if let Ok(mut p) = pick.lock() {
            *p = Some(clamp_to_world(x as f32, z as f32));
        }
    });
    // `cap_picked_plot() -> #{ outline, y, mine, owned } or ()`: the plot
    // the panel shows. `outline` is [[x, z], ...] round its edge in world
    // metres, `y` its top's height; `mine` and `owned` say whose it is.
    let picked = shared.picked_plot.clone();
    engine.register_fn("cap_picked_plot", move || -> rhai::Dynamic {
        let Some(p) = picked.lock().ok().and_then(|p| p.clone()) else {
            return rhai::Dynamic::UNIT;
        };
        let mut m = rhai::Map::new();
        let outline: rhai::Array = p
            .outline
            .iter()
            .map(|[x, z]| rhai::Dynamic::from_array(vec![(*x as f64).into(), (*z as f64).into()]))
            .collect();
        m.insert("outline".into(), outline.into());
        m.insert("y".into(), (p.y as f64).into());
        m.insert("mine".into(), p.mine.into());
        m.insert("owned".into(), p.owned.into());
        m.into()
    });
    // `cap_placing() -> #{ kind, kit, x, z, radius, ok, why, asking, you_x,
    // you_z, reach, spacing, crowding, plots } or ()`: the station being
    // placed. `x`/`z` are the ghost's (or () off the ground), `why` the
    // reason it can't go there (or ()), `crowding` the [x, z] of stations
    // near it, `plots` your plots as four [x, z] corners each. World
    // metres throughout. () when nothing is being placed.
    let placement = shared.placement.clone();
    engine.register_fn("cap_placing", move || -> rhai::Dynamic {
        match placement.lock().ok().and_then(|p| p.clone()) {
            Some(p) => placement_map(&p).into(),
            None => rhai::Dynamic::UNIT,
        }
    });
    // `cap_place_hover(x, z)`: the ground under the cursor this frame, for
    // the ghost to follow. `cap_place_off()` when it's over the sky.
    let hover = shared.place_hover.clone();
    engine.register_fn("cap_place_hover", move |x: f64, z: f64| {
        if let Ok(mut h) = hover.lock() {
            *h = Some([x as f32, z as f32]);
        }
    });
    let hover = shared.place_hover.clone();
    engine.register_fn("cap_place_off", move || {
        if let Ok(mut h) = hover.lock() {
            *h = None;
        }
    });
    // `cap_place(x, z)`: put the station down at the ground point (x, z),
    // world metres. Sent next frame if it can go there; refused otherwise.
    let place_at = shared.place_at.clone();
    engine.register_fn("cap_place", move |x: f64, z: f64| {
        if let Ok(mut p) = place_at.lock() {
            *p = Some([x as f32, z as f32]);
        }
    });
    // `cap_ui_anchor(name) -> #{ x, y, w, h, panel } or ()`: where the
    // UI's landmark `name` was drawn last frame, in the HUD's points.
    // "inventory" is the inventory panel while it's open (`panel` true),
    // else the header's Inventory button. () while neither is drawn.
    let anchors = shared.anchors.clone();
    engine.register_fn("cap_ui_anchor", move |name: &str| -> rhai::Dynamic {
        let Some(a) = anchors.lock().ok().and_then(|a| a.get(name).copied()) else {
            return rhai::Dynamic::UNIT;
        };
        let [x, y, w, h] = a.rect.map(|v| v as f64);
        let mut m = rhai::Map::new();
        m.insert("x".into(), x.into());
        m.insert("y".into(), y.into());
        m.insert("w".into(), w.into());
        m.insert("h".into(), h.into());
        m.insert("panel".into(), a.panel.into());
        m.into()
    });
    // `cap_ui_pulse(name)`: light the landmark `name` for a moment, as
    // something lands on it.
    let pulses = shared.pulses.clone();
    engine.register_fn("cap_ui_pulse", move |name: &str| {
        if let Ok(mut p) = pulses.lock() {
            p.insert(name.to_owned(), Instant::now());
        }
    });
    // `cap_ground_y() -> float`: the land's height in metres, for picking.
    // The sea is at 0.
    engine.register_fn("cap_ground_y", || crate::coords::layer::GROUND as f64);
    // `cap_ground_at(x, z) -> float`: the ground's height under world
    // (x, z), metres: `cap_ground_y()` in the city, the land in the wilds
    // once it's in (W9).
    engine.register_fn("cap_ground_at", |x: f64, z: f64| {
        crate::world::land::ground_y([x as f32, z as f32]) as f64
    });
    // `cap_rise(x, z) -> float`: how far the land under (x, z) stands over
    // the flat ground, metres; 0 in the city. Marks laid at `cap_mark_y()`
    // add it to lie on the land.
    engine.register_fn("cap_rise", |x: f64, z: f64| {
        crate::world::land::rise([x as f32, z as f32]) as f64
    });
    // `cap_mark_y() -> float`: where marks on the ground are drawn with
    // `draw_world_line`, above every road and lot and under the curbs (#86).
    engine.register_fn("cap_mark_y", || crate::coords::layer::MARK as f64);
    // `cap_crown_y() -> float`: just above the tip of the Capitol's needle,
    // in metres, where the inscription hangs.
    engine.register_fn("cap_crown_y", || crate::world::tower::CROWN_Y as f64);
}

fn node_map(n: &NodeMark) -> rhai::Map {
    let mut m = rhai::Map::new();
    m.insert("id".into(), n.id.clone().into());
    m.insert("kind".into(), n.kind.into());
    m.insert("x".into(), (n.at[0] as f64).into());
    m.insert("z".into(), (n.at[1] as f64).into());
    m.insert("r".into(), (n.r as f64).into());
    m.insert("state".into(), n.state.into());
    m.insert("targeted".into(), n.targeted.into());
    m.insert("mine".into(), n.mine.into());
    m.insert(
        "progress".into(),
        n.progress
            .map_or(rhai::Dynamic::UNIT, |p| (p as f64).into()),
    );
    m
}

fn point(p: [f32; 2]) -> rhai::Dynamic {
    rhai::Dynamic::from_array(vec![(p[0] as f64).into(), (p[1] as f64).into()])
}

fn placement_map(p: &PlacementMark) -> rhai::Map {
    let unit = || rhai::Dynamic::UNIT;
    let mut m = rhai::Map::new();
    m.insert("kind".into(), p.kind.into());
    m.insert("kit".into(), p.kit.clone().into());
    m.insert("x".into(), p.at.map_or_else(unit, |a| (a[0] as f64).into()));
    m.insert("z".into(), p.at.map_or_else(unit, |a| (a[1] as f64).into()));
    m.insert("radius".into(), (p.radius as f64).into());
    m.insert("ok".into(), p.ok.into());
    m.insert(
        "why".into(),
        p.why.clone().map_or_else(unit, rhai::Dynamic::from),
    );
    m.insert("asking".into(), p.asking.into());
    m.insert(
        "you_x".into(),
        p.you.map_or_else(unit, |a| (a[0] as f64).into()),
    );
    m.insert(
        "you_z".into(),
        p.you.map_or_else(unit, |a| (a[1] as f64).into()),
    );
    m.insert("reach".into(), (p.reach as f64).into());
    m.insert("spacing".into(), (p.spacing as f64).into());
    m.insert(
        "crowding".into(),
        rhai::Dynamic::from_array(p.crowding.iter().map(|c| point(*c)).collect()),
    );
    m.insert(
        "plots".into(),
        rhai::Dynamic::from_array(
            p.plots
                .iter()
                .map(|q| rhai::Dynamic::from_array(q.iter().map(|c| point(*c)).collect()))
                .collect(),
        ),
    );
    m
}

fn station_map(s: &StationMark) -> rhai::Map {
    let mut m = rhai::Map::new();
    m.insert("id".into(), s.id.clone().into());
    m.insert("kind".into(), s.kind.into());
    m.insert("x".into(), (s.at[0] as f64).into());
    m.insert("z".into(), (s.at[1] as f64).into());
    m.insert("radius".into(), (s.radius as f64).into());
    m.insert("in_reach".into(), s.in_reach.into());
    m.insert("reach".into(), (s.reach as f64).into());
    m.insert("top".into(), (s.top as f64).into());
    m.insert("mine".into(), s.mine.into());
    match &s.paused {
        Some(why) => m.insert("paused".into(), why.clone().into()),
        None => m.insert("paused".into(), rhai::Dynamic::UNIT),
    };
    match s.worn {
        Some(c) => m.insert("worn".into(), (c as i64).into()),
        None => m.insert("worn".into(), rhai::Dynamic::UNIT),
    };
    m
}

/// The node nearest (x, z), within [`NODE_PICK`] or just inside its ring,
/// whichever is wider: a click on a boulder's flank takes the boulder.
pub fn node_at(nodes: &[NodeMark], x: f32, z: f32) -> Option<&NodeMark> {
    nodes
        .iter()
        .map(|n| (n, (n.at[0] - x).hypot(n.at[1] - z)))
        .filter(|(n, d)| *d <= NODE_PICK.max(n.r + 0.3))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(n, _)| n)
}

/// The station nearest (x, z) whose ring it falls inside.
/// The other player whose feet are nearest (x, z), within [`PLAYER_PICK`].
pub fn player_at(players: &[PlayerLabel], x: f32, z: f32) -> Option<&PlayerLabel> {
    players
        .iter()
        .filter(|p| !p.me && p.alpha > 0.5)
        .map(|p| (p, (p.at[0] - x).hypot(p.at[2] - z)))
        .filter(|(_, d)| *d <= PLAYER_PICK)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p)
}

pub fn station_at(stations: &[StationMark], x: f32, z: f32) -> Option<&StationMark> {
    stations
        .iter()
        .map(|s| (s, (s.at[0] - x).hypot(s.at[1] - z)))
        .filter(|(s, d)| *d <= s.radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(s, _)| s)
}

/// World (X, Z) in metres to the server point a move goes to: on the world
/// disc, since the server refuses a destination off it (W1, #194). A point
/// past the edge is pulled in along its line from the centre.
pub fn clamp_to_world(wx: f32, wz: f32) -> [f64; 2] {
    use capitol_protocol::rules::{CENTER_X, CENTER_Y, WILDERNESS_OUTER_RADIUS};
    let [x, y] = crate::coords::to_server(wx, wz);
    let (dx, dy) = (x - CENTER_X, y - CENTER_Y);
    let r = dx.hypot(dy);
    // A hair inside, so rounding never lands it just off the disc.
    let edge = WILDERNESS_OUTER_RADIUS - 0.01;
    if r <= edge {
        return [x, y];
    }
    [CENTER_X + dx * edge / r, CENTER_Y + dy * edge / r]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_read_the_note() {
        let shared = Shared::default();
        *shared.note.lock().unwrap() = "Ad astra".into();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        let note: String = engine.eval("cap_spike_note()").unwrap();
        assert_eq!(note, "Ad astra");
    }

    #[test]
    fn the_rig_says_its_yaw() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        engine.eval::<()>("cap_view_yaw(1.25)").unwrap();
        assert_eq!(*shared.view_yaw.lock().unwrap(), Some(1.25));
        engine.eval::<()>("cap_view_yaw(0.0 / 0.0)").unwrap();
        assert_eq!(*shared.view_yaw.lock().unwrap(), None, "a NaN walks nowhere");
    }

    #[test]
    fn scripts_read_where_you_stand_and_who_is_about() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>("cap_follow()").is_ok());
        *shared.follow.lock().unwrap() = Some([1.0, 0.7, -2.0]);
        shared.players.lock().unwrap().push(PlayerLabel {
            id: uuid::Uuid::nil(),
            name: "Ada".into(),
            at: [1.0, 2.9, -2.0],
            feet_y: 0.65,
            alpha: 0.5,
            me: true,
            citizen: false,
            tag: None,
            craft: Some(0.25),
            line: None,
        });
        let z: f64 = engine.eval("cap_follow()[2]").unwrap();
        assert_eq!(z, -2.0);
        let who: String = engine
            .eval(r#"let p = cap_players()[0]; `${p.name} ${p.alpha} ${p.me}`"#)
            .unwrap();
        assert_eq!(who, "Ada 0.5 true");
        let craft: f64 = engine.eval("cap_players()[0].craft").unwrap();
        assert_eq!(craft, 0.25);
    }

    /// Every script parses, so a typo fails here rather than in a capture.
    /// (Parsing only: the engine's own functions aren't registered.)
    #[test]
    fn the_scripts_parse() {
        let mut engine = rhai::Engine::new();
        engine.set_max_expr_depths(128, 128); // as flint-script sets it
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
        let mut parsed = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rhai") {
                if let Err(e) = engine.compile_file(path.clone()) {
                    panic!("{}: {e}", path.display());
                }
                parsed += 1;
            }
        }
        assert!(parsed >= 4, "only {parsed} scripts in {}", dir.display());
    }

    #[test]
    fn scripts_read_the_zone_banner() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>("cap_zone_banner()").is_ok());
        *shared.banner.lock().unwrap() = Some(ZoneBanner {
            name: "Trade District".into(),
            colour: [0.25, 0.5, 1.0],
            at: Instant::now() + std::time::Duration::from_secs(60),
            first: true,
        });
        let words: String = engine
            .eval(r#"let b = cap_zone_banner(); `${b.name} ${b.b} ${b.first} ${b.age < -59.0}`"#)
            .unwrap();
        assert_eq!(words, "Trade District 1.0 true true");
    }

    #[test]
    fn a_move_is_clamped_to_the_world_and_the_last_one_wins() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        engine
            .run("cap_move(5.0, -2.5); cap_move(1000.0, -1000.0);")
            .unwrap();
        let [x, y] = shared.move_to.lock().unwrap().unwrap();
        assert!(capitol_protocol::rules::in_world(x, y), "({x}, {y}) is off the disc");
        // Pulled in toward the centre: north-east, on the edge.
        let (dx, dy) = (x - 8300.0, y - 8300.0);
        assert!((dx.hypot(dy) - 8300.0).abs() < 0.1 && (dx + dy).abs() < 1e-6);
        let [x, y] = clamp_to_world(5.0, -2.5);
        assert!((x - 8350.0).abs() < 1e-3 && (y - 8275.0).abs() < 1e-3);
    }

    #[test]
    fn a_click_picks_the_nearest_node_and_asks_to_extract_it() {
        let shared = Shared::default();
        let mark = |id: &str, x: f32| NodeMark {
            id: id.into(),
            at: [x, 0.0],
            kind: "grass",
            r: 1.15,
            state: "available",
            targeted: false,
            mine: false,
            progress: None,
        };
        *shared.nodes.lock().unwrap() = vec![mark("a", 0.0), mark("b", 2.0)];
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        let picked: String = engine
            .eval("let n = cap_node_at(1.2, 0.3); cap_extract(n.id); n.id")
            .unwrap();
        assert_eq!(picked, "b");
        assert_eq!(shared.extract.lock().unwrap().as_deref(), Some("b"));
        assert!(engine.eval::<()>("cap_node_at(9.0, 9.0)").is_ok());
        // Nobody's working or aiming at either, so the scripts aren't given them.
        assert_eq!(engine.eval::<i64>("cap_nodes().len()").unwrap(), 0);
        shared.nodes.lock().unwrap()[1].targeted = true;
        let p: bool = engine
            .eval(r#"let n = cap_nodes()[0]; n.progress == () && n.kind == "grass" && n.r > 1.0"#)
            .unwrap();
        assert!(p);
    }

    #[test]
    fn scripts_pick_a_plot_and_read_the_picked_one() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>("cap_picked_plot()").is_ok());
        engine.run("cap_pick_plot(5.0, -2.5)").unwrap();
        let [x, y] = shared.pick_plot.lock().unwrap().unwrap();
        assert!((x - 8350.0).abs() < 1e-3 && (y - 8275.0).abs() < 1e-3);
        *shared.picked_plot.lock().unwrap() = Some(PlotMark {
            outline: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 3.0]],
            y: 0.7,
            mine: true,
            owned: true,
        });
        let words: String = engine
            .eval(r#"let p = cap_picked_plot(); `${p.outline.len()} ${p.outline[2][1]} ${p.mine}`"#)
            .unwrap();
        assert_eq!(words, "3 3.0 true");
    }

    #[test]
    fn scripts_read_the_placement_and_ask_to_place() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>("cap_placing()").is_ok());
        *shared.placement.lock().unwrap() = Some(PlacementMark {
            kind: "forge",
            kit: "Forge Kit".into(),
            at: Some([1.0, 2.0]),
            radius: 1.95,
            ok: false,
            why: Some("Too far from you".into()),
            asking: false,
            you: None,
            reach: 7.5,
            spacing: 5.0,
            crowding: vec![[3.0, 4.0]],
            plots: vec![[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]],
        });
        let words: String = engine
            .eval(
                r#"let p = cap_placing();
                   `${p.kind} ${p.z} ${p.why} ${p.you_x == ()} ${p.crowding[0][1]} ${p.plots[0][2][0]}`"#,
            )
            .unwrap();
        assert_eq!(words, "forge 2.0 Too far from you true 4.0 1.0");
        engine
            .run("cap_place_hover(5.0, 6.0); cap_place(7.0, 8.0);")
            .unwrap();
        assert_eq!(*shared.place_hover.lock().unwrap(), Some([5.0, 6.0]));
        assert_eq!(*shared.place_at.lock().unwrap(), Some([7.0, 8.0]));
        engine.run("cap_place_off()").unwrap();
        assert_eq!(*shared.place_hover.lock().unwrap(), None);
    }

    #[test]
    fn the_debug_layers_reach_the_scripts_only_while_on() {
        use crate::debug_layers::{Cell, Mark, NodeMark, WalkerMark};
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>("cap_debug()").is_ok(), "() while off");
        *shared.debug.lock().unwrap() = Some(Mark {
            layer: "every layer",
            cells: vec![Cell { min: [0.0, 1.0], max: [30.0, 31.0], label: "(2, 3)".into(), mine: true }],
            nodes: vec![NodeMark { at: [4.0, 5.0], kind: "iron_vein".into(), ground: "h 412".into(), available: true }],
            edges: vec![[1.0, 2.0, 3.0, 4.0]],
            walkers: vec![
                WalkerMark { at: [6.0, 7.0], dest: Some([8.0, 9.0]), name: "Ada".into(), citizen: true, doing: String::new() },
                WalkerMark { at: [1.0, 1.0], dest: None, name: "Bram".into(), citizen: false, doing: "crafting rope".into() },
            ],
        });
        let words: String = engine
            .eval(
                r#"let d = cap_debug();
                   let c = d.cells[0];
                   let w = d.walkers;
                   `${d.layer} ${c.label} ${c.mine} ${c.x1} ${d.nodes[0].kind} ${d.nodes[0].ground} ${d.edges[0][3]} ${w[0].dest[1]} ${w[1].dest == ()} ${w[1].doing}`"#,
            )
            .unwrap();
        assert_eq!(words, "every layer (2, 3) true 30.0 iron_vein h 412 4.0 9.0 true crafting rope");
    }

    #[test]
    fn a_click_inside_a_ring_picks_the_station_and_asks_to_open_it() {
        let shared = Shared::default();
        let mark = |id: &str, x: f32, mine: bool| StationMark {
            id: id.into(),
            kind: "workbench",
            at: [x, 0.0],
            radius: 1.5,
            in_reach: false,
            reach: 0.0,
            top: 1.0,
            paused: None,
            mine,
            worn: None,
        };
        *shared.stations.lock().unwrap() = vec![mark("bench", 0.0, false), mark("chest", 2.5, true)];
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        let picked: String = engine
            .eval("let s = cap_station_at(1.4, 0.2); cap_open_station(s.id); s.id")
            .unwrap();
        assert_eq!(picked, "chest", "both rings hold it; the chest is nearer");
        assert_eq!(
            shared.open_station.lock().unwrap().as_deref(),
            Some("chest")
        );
        assert!(engine.eval::<()>("cap_station_at(0.0, 1.6)").is_ok());
        let mine: String = engine
            .eval(r#"let s = cap_stations(); `${s[0].mine} ${s[1].mine}`"#)
            .unwrap();
        assert_eq!(mine, "false true", "the chest is yours, the bench isn't");
    }

    #[test]
    fn a_click_picks_another_player_for_a_trade_or_their_card_never_you() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        let label = |n: u128, x: f32, me: bool| PlayerLabel {
            id: uuid::Uuid::from_u128(n),
            name: format!("P{n}"),
            at: [x, 2.9, 0.0],
            feet_y: 0.65,
            alpha: 1.0,
            me,
            citizen: false,
            tag: None,
            craft: None,
            line: None,
        };
        shared
            .players
            .lock()
            .unwrap()
            .extend([label(1, 0.0, true), label(2, 0.8, false)]);
        let name: String = engine
            .eval("let p = cap_player_at(0.1, 0.0); cap_trade_with(p.id); p.name")
            .unwrap();
        assert_eq!(name, "P2");
        assert_eq!(
            shared.trade_with.lock().unwrap().as_deref(),
            Some(uuid::Uuid::from_u128(2).to_string().as_str())
        );
        assert!(engine.eval::<()>("cap_player_at(5.0, 5.0)").is_ok());
        engine.eval::<()>("cap_open_card(cap_player_at(0.9, 0.0).id)").unwrap();
        assert_eq!(
            shared.open_card.lock().unwrap().as_deref(),
            Some(uuid::Uuid::from_u128(2).to_string().as_str())
        );
        assert!(engine.eval::<()>("cap_trade()").is_ok());
        *shared.trade.lock().unwrap() = Some(TradeMark {
            state: "open",
            them: Some([0.8, 0.65, 0.0]),
            mine: true,
            theirs: false,
        });
        let t: String = engine
            .eval(r#"let t = cap_trade(); `${t.state} ${t.x} ${t.mine} ${t.theirs}`"#)
            .unwrap();
        assert!(t.starts_with("open 0.8"), "{t}");
        assert!(t.ends_with("true false"), "{t}");
    }

    #[test]
    fn scripts_read_where_you_are_walking_to() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>("cap_destination()").is_ok());
        *shared.destination.lock().unwrap() = Some([3.0, -4.0]);
        let z: f64 = engine.eval("cap_destination()[1]").unwrap();
        assert_eq!(z, -4.0);
        assert_eq!(engine.eval::<i64>("cap_route().len()").unwrap(), 0);
        *shared.route.lock().unwrap() = vec![[1.0, 2.0], [3.0, -4.0]];
        let x: f64 = engine.eval("cap_route()[0][0]").unwrap();
        assert_eq!(x, 1.0);
        assert_eq!(engine.eval::<i64>("cap_route().len()").unwrap(), 2);
    }

    #[test]
    fn scripts_find_and_light_the_inventory() {
        let shared = Shared::default();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        assert!(engine.eval::<()>(r#"cap_ui_anchor("inventory")"#).is_ok());
        shared.anchors.lock().unwrap().insert(
            "inventory",
            UiAnchor {
                rect: [10.0, 20.0, 80.0, 26.0],
                panel: false,
            },
        );
        let mid: f64 = engine
            .eval(r#"let r = cap_ui_anchor("inventory"); r.x + r.w / 2.0"#)
            .unwrap();
        assert_eq!(mid, 50.0);
        let now = Instant::now();
        assert_eq!(shared.pulse("inventory", now), 0.0);
        engine.run(r#"cap_ui_pulse("inventory")"#).unwrap();
        assert!(shared.pulse("inventory", Instant::now()) > 0.9);
        let later = Instant::now() + std::time::Duration::from_secs_f32(PULSE);
        assert_eq!(shared.pulse("inventory", later), 0.0);
    }

    /// The toast stack (notices.rhai): the same message again takes a
    /// count, and past four the oldest is sent out.
    #[test]
    fn toasts_stack_count_and_make_room() {
        use rhai::{CallFnOptions, Scope};
        let mut engine = rhai::Engine::new();
        engine.set_max_expr_depths(128, 128);
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/notices.rhai");
        let ast = engine.compile_file(path).unwrap();
        let add = |list: rhai::Array, title: &str, t: f64| -> rhai::Array {
            let options = CallFnOptions::new().eval_ast(false);
            engine
                .call_fn_with_options(
                    options,
                    &mut Scope::new(),
                    &ast,
                    "add_toast",
                    (
                        list,
                        title.to_owned(),
                        "body".to_owned(),
                        "note".to_owned(),
                        t,
                    ),
                )
                .unwrap()
        };
        let field = |q: &rhai::Dynamic, k: &str| q.clone().cast::<rhai::Map>()[k].clone();
        let mut list = add(rhai::Array::new(), "Taken", 0.0);
        list = add(list, "Find a bridge", 0.5);
        list = add(list, "Taken", 1.0);
        assert_eq!(list.len(), 2, "the second Taken counts on the first");
        let first = &list[0];
        assert_eq!(
            field(first, "title").into_string().unwrap(),
            "Taken",
            "where it stood"
        );
        assert_eq!(field(first, "count").as_int().unwrap(), 2);
        for (i, title) in ["a", "b", "c"].iter().enumerate() {
            list = add(list, title, 2.0 + i as f64);
        }
        assert_eq!(list.len(), 5);
        let leaving: Vec<_> = list
            .iter()
            .filter(|q| !field(q, "leave").is_unit())
            .collect();
        assert_eq!(leaving.len(), 1, "a fifth sends one out");
        assert_eq!(
            field(leaving[0], "title").into_string().unwrap(),
            "Taken",
            "the oldest"
        );
    }
}
