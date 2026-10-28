//! The pad inside a panel (#331, the controls plan's F2): with a panel open
//! and the pad the last thing used, the d-pad moves a gold ring between the
//! panels' widgets, A presses the one in it, X takes half a stack, B backs
//! out (the extension's `back`), and the shoulders hop from panel to panel.
//!
//! It's built on egui's own keyboard focus, so nothing here knows a panel.
//! Any widget that takes focus (a button, a tab, a check, a slot, a field)
//! is somewhere the ring can go; a press is the Enter a focused widget
//! already answers as a click (`Response::fake_primary_click`). The moves
//! are ours, not egui's arrow keys: egui reads those off the raw input
//! before the frame begins, and a game's extension only sees the frame from
//! inside it. So [`PadNav::begin`] picks the next widget from the last
//! frame's rects ([`next_in`], egui's cone search, with clipped widgets
//! kept so a list scrolls on) and asks for its focus.
//!
//! A list follows the ring: every scroll area in the panels is shown with
//! [`FollowFocus::show_following`], which scrolls it to the widget the ring
//! just moved to, if that widget is in it.
//!
//! Slots take the d-pad as a grid without anything of their own, since the
//! cone finds the next cell. A press on one picks its stack up, and a press
//! on another puts it there (`slots`, its `Carry`); X picks up half, as a
//! shift-drag does. Fields still want the keys: the pad has no way to type
//! (#517).

use super::theme::{GOLD, GOLD_LIGHT};
use egui::{Context, Id, LayerId, Order, Rect, Stroke, Ui, Vec2};
use std::collections::HashMap;

/// A d-pad direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    fn vec(self) -> Vec2 {
        match self {
            Dir::Up => Vec2::UP,
            Dir::Down => Vec2::DOWN,
            Dir::Left => Vec2::LEFT,
            Dir::Right => Vec2::RIGHT,
        }
    }
}

/// What the pad asked of the panels since the last frame's `ui`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Asked {
    pub dir: Option<Dir>,
    /// A: press the widget in the ring.
    pub press: bool,
    /// X: as A, but a slot gives up half its stack.
    pub half: bool,
    /// The shoulders: +1 to the next panel, -1 to the one before.
    pub hop: i32,
}

/// A widget the ring can go to: its id, its layer and where it was laid
/// out last frame, clipped or not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub id: Id,
    pub layer: LayerId,
    pub rect: Rect,
}

/// The ring, between frames.
#[derive(Debug, Default)]
pub struct PadNav {
    active: bool,
    asked: Asked,
    /// Where the ring was, to go on from when egui let the focus go (a
    /// field gives it up on Enter).
    last: Option<Id>,
    /// Each layer with somewhere to go, and the frame it came up: the
    /// newest is the panel the ring goes into.
    seen: HashMap<LayerId, u64>,
    frame: u64,
    /// The widget the ring lent egui's focus to this pass.
    lent: Option<Id>,
}

/// The scroll bars drawn last frame, which take clicks and drags but are no
/// place for the ring: [`note_scroll_bars`] keeps them, a frame at a time.
#[derive(Debug, Clone, Default)]
struct Bars {
    last: std::collections::HashSet<Id>,
    this: std::collections::HashSet<Id>,
}

impl Bars {
    fn id() -> Id {
        Id::new("capitol_pad_nav_bars")
    }
}

/// Notes the bars of the scroll area `area` (its `ScrollAreaOutput::id`),
/// so the ring passes them by. [`FollowFocus`] does it for its areas.
pub fn note_scroll_bars(ctx: &Context, area: Id) {
    ctx.data_mut(|d| {
        let bars = d.get_temp_mut_or_default::<Bars>(Bars::id());
        bars.this.extend([area.with(0usize), area.with(1usize)]);
    });
}

impl PadNav {
    /// Adds what the pad asked this frame (`pre_update`), for `begin`.
    pub fn ask(&mut self, asked: Asked) {
        self.asked.dir = asked.dir.or(self.asked.dir);
        self.asked.press |= asked.press;
        self.asked.half |= asked.half;
        self.asked.hop += asked.hop;
    }

    /// At the top of `ui`, before any panel: moves the ring and presses.
    /// `active` is a panel open with the pad the last thing used.
    pub fn begin(&mut self, ctx: &Context, active: bool) {
        self.frame += 1;
        ctx.data_mut(|d| {
            let bars = d.get_temp_mut_or_default::<Bars>(Bars::id());
            bars.last = std::mem::take(&mut bars.this);
        });
        let spots = spots(ctx);
        let newest = self.note_layers(&spots);
        let asked = std::mem::take(&mut self.asked);
        self.lent = None;
        let was = std::mem::replace(&mut self.active, active);
        ctx.data_mut(|d| {
            d.insert_temp(Self::active_id(), active);
            d.insert_temp(Self::half_id(), active && asked.half);
        });
        if !active {
            if was {
                super::slots::drop_carry(ctx);
                self.last = None;
            }
            return;
        }
        let find = |id: Option<Id>| id.and_then(|id| spots.iter().find(|s| s.id == id).copied());
        let focused = ctx.memory(|m| m.focused());
        let mut at = find(focused).or_else(|| find(self.last));
        // A panel that's just come up takes the ring, as does the newest
        // one when the ring is nowhere yet
        let into = match (newest, at) {
            (Some(layer), _) if was => Some(layer),
            (_, None) => self.newest_layer(ctx, &spots),
            _ => None,
        };
        if let Some(first) = into.and_then(|layer| first_of(&spots, layer)) {
            at = Some(first);
        }
        if let (Some(dir), Some(from)) = (asked.dir, at) {
            if let Some(to) = next_in(dir, from.rect, &spots, from.id) {
                at = Some(to);
            }
        }
        if asked.hop != 0 {
            if let Some(to) = hop(&spots, at, asked.hop) {
                at = Some(to);
            }
        }
        if let Some(spot) = at {
            // Kept between frames, but for a field (`end`)
            ctx.memory_mut(|m| m.request_focus(spot.id));
            self.lent = Some(spot.id);
            if self.last != Some(spot.id) {
                ctx.data_mut(|d| d.insert_temp(Self::scroll_id(), spot.id));
            }
        }
        self.last = at.map(|s| s.id);
        if (asked.press || asked.half) && at.is_some() {
            // What a focused widget answers as a click
            ctx.input_mut(|i| {
                i.events.push(egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
            });
        }
    }

    /// At the end of `ui`: draws the ring round the widget in focus. Flint
    /// takes only a focused text field for typing (#516), so a button, tab
    /// or slot keeps egui's focus between frames. A field the ring sits on
    /// gives it back: as typing, it would hold the pad that moves the ring.
    /// A text field that took the focus itself (the chat's, opened) keeps
    /// it: that is typing.
    pub fn end(&mut self, ctx: &Context) {
        ctx.data_mut(|d| d.remove::<Id>(Self::scroll_id()));
        if !self.active {
            return;
        }
        let Some(id) = ctx.memory(|m| m.focused()) else { return };
        let typing = egui::TextEdit::load_state(ctx, id).is_some();
        if self.lent == Some(id) || !typing {
            // A press can move the focus on: the ring follows it
            self.last = Some(id);
        }
        if self.lent == Some(id) && typing {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        let Some(response) = ctx.read_response(id) else { return };
        if response.interact_rect.is_positive() {
            ring(ctx, response.interact_rect, ctx.input(|i| i.time));
        }
    }

    /// Remembers which layers have somewhere to go, and returns the one
    /// that came up this frame, if any did.
    fn note_layers(&mut self, spots: &[Spot]) -> Option<LayerId> {
        let frame = self.frame;
        let mut now = HashMap::new();
        let mut newest = None;
        for s in spots {
            let since = *self.seen.get(&s.layer).unwrap_or(&frame);
            if since == frame && s.layer.order != Order::Background {
                newest = Some(s.layer);
            }
            now.insert(s.layer, since);
        }
        self.seen = now;
        // The first frame sees every layer come up at once: none is new
        newest.filter(|_| frame > 1)
    }

    /// Where the ring starts: a panel on top of the others (not one half
    /// under another, as the crafting panel is under a station's), then the
    /// one that came up last, and the header strip only for want of any.
    fn newest_layer(&self, ctx: &Context, spots: &[Spot]) -> Option<LayerId> {
        let on_top = |layer: LayerId| {
            let panel = ctx.memory(|m| m.area_rect(layer.id))?;
            Some(ctx.layer_id_at(panel.center()).is_none_or(|top| top == layer))
        };
        let spots: std::collections::HashSet<LayerId> = spots.iter().map(|s| s.layer).collect();
        self.seen
            .iter()
            .filter(|(layer, _)| spots.contains(layer))
            .max_by_key(|(layer, since)| {
                (layer.order != Order::Background, on_top(**layer).unwrap_or(false), **since, layer.id.value())
            })
            .map(|(layer, _)| *layer)
    }

    fn active_id() -> Id {
        Id::new("capitol_pad_nav_active")
    }

    pub(super) fn half_id() -> Id {
        Id::new("capitol_pad_nav_half")
    }

    fn scroll_id() -> Id {
        Id::new("capitol_pad_nav_scroll")
    }
}

/// Whether the ring is up this frame, for a widget that draws differently
/// for the pad (a slot's tooltip).
pub fn active(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp(PadNav::active_id())).unwrap_or(false)
}

/// Whether this frame's press was X's: a slot gives up half its stack.
pub fn taking_half(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp(PadNav::half_id())).unwrap_or(false)
}

/// Every widget last frame that can take the focus and is drawn somewhere,
/// on a layer that's showing, and not under another panel. Not the panel
/// itself (an area takes clicks as a whole, to come to the front), and not
/// what only drags: a list's surface for a drag to scroll, or a slider,
/// which wants keys the pad hasn't (#517), nor a scroll bar.
pub fn spots(ctx: &Context) -> Vec<Spot> {
    let showing = ctx.memory(|m| m.areas().visible_layer_ids());
    let bars = ctx.data_mut(|d| d.get_temp_mut_or_default::<Bars>(Bars::id()).last.clone());
    let mut spots: Vec<Spot> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .filter(|(layer, _)| layer.order == Order::Background || showing.contains(layer))
            .flat_map(|(_, widgets)| widgets.iter())
            .filter(|w| w.sense.focusable && w.enabled && w.rect.is_positive())
            .filter(|w| w.id != w.layer_id.id.with("move") && w.sense.click && !bars.contains(&w.id))
            .map(|w| Spot {
                id: w.id,
                layer: w.layer_id,
                rect: w.rect,
            })
            .collect()
    });
    spots.retain(|s| ctx.layer_id_at(s.rect.center()).is_none_or(|top| top == s.layer));
    spots
}

/// The widget `dir` of `from`: egui's cone search (90° about the way, the
/// nearest by distance over the cosine squared), where ranges that overlap
/// by half count as level. `skip` is the widget the ring is on.
pub fn next_in(dir: Dir, from: Rect, spots: &[Spot], skip: Id) -> Option<Spot> {
    let way = dir.vec();
    let diff = |a: egui::Rangef, b: egui::Rangef| {
        if a.intersection(b).span() >= 0.5 * b.span().min(a.span()) {
            0.0
        } else {
            a.center() - b.center()
        }
    };
    spots
        .iter()
        .filter(|s| s.id != skip)
        .filter_map(|s| {
            let to = egui::vec2(diff(s.rect.x_range(), from.x_range()), diff(s.rect.y_range(), from.y_range()));
            let cos = to.normalized().dot(way);
            (cos >= 0.5_f32.sqrt()).then(|| (to.length() / (cos * cos), s))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| *s)
}

/// Where the ring starts in `layer`: its first widget, reading across from
/// the top.
pub fn first_of(spots: &[Spot], layer: LayerId) -> Option<Spot> {
    spots
        .iter()
        .filter(|s| s.layer == layer)
        .min_by(|a, b| {
            let row = |s: &Spot| (s.rect.top() / 8.0).floor();
            row(a).total_cmp(&row(b)).then(a.rect.left().total_cmp(&b.rect.left()))
        })
        .copied()
}

/// The first widget of the panel `by` along from the ring's, the panels
/// taken left to right; round from the last to the first. The header strip
/// isn't a panel.
pub fn hop(spots: &[Spot], at: Option<Spot>, by: i32) -> Option<Spot> {
    let mut panels: Vec<(LayerId, f32)> = Vec::new();
    for s in spots.iter().filter(|s| s.layer.order != Order::Background) {
        match panels.iter_mut().find(|(l, _)| *l == s.layer) {
            Some((_, left)) => *left = left.min(s.rect.left()),
            None => panels.push((s.layer, s.rect.left())),
        }
    }
    if panels.is_empty() {
        return None;
    }
    panels.sort_by(|a, b| a.1.total_cmp(&b.1));
    let n = panels.len() as i32;
    let here = at.and_then(|a| panels.iter().position(|(l, _)| *l == a.layer));
    let to = match here {
        Some(i) => (i as i32 + by).rem_euclid(n),
        None if by > 0 => 0,
        None => n - 1,
    };
    first_of(spots, panels[to as usize].0)
}

/// The ring: gold, a hair outside the widget, breathing a little so the eye
/// finds it, with a glow under it.
fn ring(ctx: &Context, rect: Rect, time: f64) {
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("capitol_pad_ring")));
    let breath = 0.5 + 0.5 * (time * std::f64::consts::TAU / 1.6).sin() as f32;
    let outer = rect.expand(3.0);
    painter.rect_stroke(outer.expand(2.0), 7.0, Stroke::new(4.0_f32, GOLD.gamma_multiply(0.18 + 0.12 * breath)));
    painter.rect_stroke(outer, 6.0, Stroke::new(2.0_f32, GOLD.lerp_to_gamma(GOLD_LIGHT, breath * 0.6)));
}

/// A scroll area that follows the ring (#331).
pub trait FollowFocus {
    /// `ScrollArea::show`, then scrolled to the widget the ring just moved
    /// to, if it's in this area: in view by a margin, from the next frame.
    fn show_following<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R>;
}

impl FollowFocus for egui::ScrollArea {
    fn show_following<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
        let out = self.show(ui, add);
        note_scroll_bars(ui.ctx(), out.id);
        keep_in_view(ui, &out);
        out
    }
}

fn keep_in_view<R>(ui: &Ui, out: &egui::scroll_area::ScrollAreaOutput<R>) {
    const MARGIN: f32 = 8.0;
    let ctx = ui.ctx();
    let Some(id) = ctx.data(|d| d.get_temp::<Id>(PadNav::scroll_id())) else { return };
    let Some(target) = ctx.read_response(id) else { return };
    let view = out.inner_rect;
    let top = view.top() - out.state.offset.y;
    let content = Rect::from_min_size(egui::pos2(view.left(), top), egui::vec2(view.width(), out.content_size.y));
    if target.layer_id != ui.layer_id() || !content.expand(1.0).contains(target.rect.center()) {
        return;
    }
    ctx.data_mut(|d| d.remove::<Id>(PadNav::scroll_id()));
    let offset = scrolled_to(out.state.offset.y, view.y_range(), target.rect.y_range(), out.content_size.y, MARGIN);
    if offset != out.state.offset.y {
        let mut state = out.state;
        state.offset.y = offset;
        state.store(ctx, out.id);
        ctx.request_repaint();
    }
}

/// The offset that brings `target` into `view` with `margin` to spare,
/// moving as little as it can, within the content's `height`.
pub fn scrolled_to(offset: f32, view: egui::Rangef, target: egui::Rangef, height: f32, margin: f32) -> f32 {
    let most = (height - view.span()).max(0.0);
    let to = if target.min < view.min + margin {
        offset - (view.min + margin - target.min)
    } else if target.max > view.max - margin {
        offset + (target.max - (view.max - margin))
    } else {
        offset
    };
    to.clamp(0.0, most)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(n: u64, layer: &str, x: f32, y: f32) -> Spot {
        Spot {
            id: Id::new(n),
            layer: LayerId::new(Order::Middle, Id::new(layer)),
            rect: Rect::from_min_size(egui::pos2(x, y), egui::vec2(48.0, 48.0)),
        }
    }

    /// A 3 by 2 grid of slots 52 apart, ids 0 to 5 reading across.
    fn grid() -> Vec<Spot> {
        (0..6).map(|i| spot(i, "inv", (i % 3) as f32 * 52.0, (i / 3) as f32 * 52.0)).collect()
    }

    #[test]
    fn the_d_pad_walks_a_slot_grid_as_a_grid() {
        let g = grid();
        let go = |from: usize, dir| next_in(dir, g[from].rect, &g, g[from].id).map(|s| s.id);
        assert_eq!(go(0, Dir::Right), Some(Id::new(1u64)));
        assert_eq!(go(1, Dir::Down), Some(Id::new(4u64)));
        assert_eq!(go(4, Dir::Left), Some(Id::new(3u64)));
        assert_eq!(go(3, Dir::Up), Some(Id::new(0u64)));
        assert_eq!(go(2, Dir::Right), None, "the edge of the grid is the edge");
        assert_eq!(go(0, Dir::Up), None);
    }

    #[test]
    fn the_d_pad_crosses_from_panel_to_panel() {
        let mut spots = grid();
        spots.push(spot(9, "craft", 260.0, 60.0));
        let to = next_in(Dir::Right, spots[5].rect, &spots, spots[5].id);
        assert_eq!(to.map(|s| s.id), Some(Id::new(9u64)));
    }

    #[test]
    fn a_wide_button_under_a_row_is_down_from_any_of_it() {
        let mut spots = grid();
        let wide = Spot {
            id: Id::new("buy"),
            layer: spots[0].layer,
            rect: Rect::from_min_size(egui::pos2(0.0, 120.0), egui::vec2(152.0, 28.0)),
        };
        spots.push(wide);
        for from in 3..6 {
            let to = next_in(Dir::Down, spots[from].rect, &spots, spots[from].id);
            assert_eq!(to.map(|s| s.id), Some(Id::new("buy")), "from slot {from}");
        }
    }

    #[test]
    fn the_ring_starts_top_left() {
        let mut spots = grid();
        spots.reverse();
        let layer = spots[0].layer;
        assert_eq!(first_of(&spots, layer).map(|s| s.id), Some(Id::new(0u64)));
    }

    #[test]
    fn the_shoulders_hop_panels_left_to_right_and_round() {
        let mut spots = grid();
        spots.push(spot(9, "craft", 400.0, 0.0));
        spots.push(spot(10, "craft", 400.0, 60.0));
        let header = Spot {
            id: Id::new("chip"),
            layer: LayerId::background(),
            rect: Rect::from_min_size(egui::pos2(-50.0, -50.0), egui::vec2(40.0, 20.0)),
        };
        spots.push(header);
        let at = Some(spots[4]);
        assert_eq!(hop(&spots, at, 1).map(|s| s.id), Some(Id::new(9u64)));
        assert_eq!(hop(&spots, Some(spots[7]), 1).map(|s| s.id), Some(Id::new(0u64)), "round to the first");
        assert_eq!(hop(&spots, at, -1).map(|s| s.id), Some(Id::new(9u64)), "and back round");
        assert_eq!(hop(&spots, None, 1).map(|s| s.id), Some(Id::new(0u64)));
    }

    #[test]
    fn a_list_scrolls_as_little_as_it_can() {
        let view = egui::Rangef::new(100.0, 300.0);
        let at = |a: f32, b: f32| egui::Rangef::new(a, b);
        assert_eq!(scrolled_to(0.0, view, at(150.0, 180.0), 1000.0, 8.0), 0.0, "in view: stays");
        assert_eq!(scrolled_to(0.0, view, at(300.0, 330.0), 1000.0, 8.0), 38.0, "below: its foot in view");
        assert_eq!(scrolled_to(50.0, view, at(80.0, 110.0), 1000.0, 8.0), 22.0, "above: its head in view");
        assert_eq!(scrolled_to(0.0, view, at(1180.0, 1200.0), 1000.0, 8.0), 800.0, "never past the end");
    }

    /// Frames through egui with a column of buttons in an area, the ring
    /// moved down and a press, as the extension drives it.
    #[test]
    fn the_ring_moves_down_a_panel_and_a_press_clicks() {
        let ctx = egui::Context::default();
        let mut nav = PadNav::default();
        let mut clicked = Vec::new();
        let frame = |nav: &mut PadNav, asked: Asked, clicked: &mut Vec<usize>| {
            nav.ask(asked);
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                nav.begin(ctx, true);
                egui::Area::new(Id::new("panel")).fixed_pos(egui::pos2(100.0, 100.0)).show(ctx, |ui| {
                    for i in 0..3 {
                        if ui.button(format!("Button {i}")).clicked() {
                            clicked.push(i);
                        }
                    }
                });
                nav.end(ctx);
            });
        };
        // The panel comes up, and on the next frame the ring goes into it
        frame(&mut nav, Asked::default(), &mut clicked);
        frame(&mut nav, Asked::default(), &mut clicked);
        frame(&mut nav, Asked::default(), &mut clicked);
        assert!(nav.last.is_some(), "the ring is on the first button");
        assert_eq!(ctx.memory(|m| m.focused()), nav.last, "and egui keeps its focus between frames (#516)");
        frame(&mut nav, Asked { dir: Some(Dir::Down), ..Default::default() }, &mut clicked);
        frame(&mut nav, Asked { press: true, ..Default::default() }, &mut clicked);
        assert_eq!(clicked, vec![1], "A pressed the second button");
        frame(&mut nav, Asked { dir: Some(Dir::Down), ..Default::default() }, &mut clicked);
        frame(&mut nav, Asked { dir: Some(Dir::Down), ..Default::default() }, &mut clicked);
        frame(&mut nav, Asked { press: true, ..Default::default() }, &mut clicked);
        assert_eq!(clicked, vec![1, 2], "the foot of the column holds");
        // Put down, the ring goes and the focus stays for the mouse
        frame(&mut nav, Asked::default(), &mut clicked);
        nav.begin(&ctx, false);
        assert!(!active(&ctx));
    }
}
