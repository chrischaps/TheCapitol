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
//! shift-drag does.
//!
//! Some parts want more than a press (#517), and A takes hold of them
//! instead ([`Hold`]). What a part is comes from what egui tells a screen
//! reader as it takes the focus ([`kind_of`]):
//! - **A level** (`theme::level`, Settings' volumes and sensitivities): left
//!   and right step it while it's held, as the arrow keys do; A, B, up or
//!   down let go.
//! - **A field**, text or a number (a `DragValue` is a field while it has
//!   the focus): the pad's keys come up beside it ([`super::pad_keys`]),
//!   and the d-pad walks them. A types, X deletes, the shoulders move the
//!   caret, and B or Done puts them away.
//!
//! A long list drawn a row at a time (the Deeds Office's free lots) is shown
//! with [`FollowFocus::show_rows_following`], which lays a row out past each
//! edge of the view, so the ring has somewhere to go and the list scrolls
//! on to it.

use super::pad_keys::{Field, Keyboard, Typed};
use super::theme::{GOLD, GOLD_LIGHT};
use egui::output::OutputEvent;
use egui::{Context, Id, LayerId, Order, Rect, Stroke, Ui, Vec2, WidgetType};
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
    /// What the ring is on, as egui told it when the ring got there.
    ringed: Option<(Id, Kind)>,
    /// A level or a field the ring has hold of (#517).
    hold: Option<Hold>,
    /// The keys typed into the held field this pass.
    typed: bool,
}

/// What a widget the ring is on wants from A.
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// A press: a button, a tab, a slot.
    Press,
    /// Held, to step with left and right.
    Level,
    /// Held, to type into.
    Field(Field),
}

/// What the ring has hold of.
#[derive(Debug, Clone, PartialEq)]
enum Hold {
    Level(Id),
    Keys(Keyboard),
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
        let mut asked = std::mem::take(&mut self.asked);
        self.lent = None;
        self.typed = false;
        let was = std::mem::replace(&mut self.active, active);
        let typing = matches!(self.hold, Some(Hold::Keys(_)));
        ctx.data_mut(|d| {
            d.insert_temp(Self::active_id(), active);
            d.insert_temp(Self::half_id(), active && asked.half && !typing);
        });
        if !active {
            self.hold = None;
            if was {
                super::slots::drop_carry(ctx);
                self.last = None;
            }
            return;
        }
        if self.type_keys(ctx, &asked) {
            return;
        }
        if let Some(Hold::Level(_)) = self.hold {
            asked = self.step_level(ctx, asked);
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
        if let (true, Some(spot)) = (asked.press || asked.half, at) {
            self.take(ctx, spot.id);
        }
    }

    /// A or X on `id`: a field brings up the keys, a level is held, and
    /// anything else is pressed, with the Enter a focused widget answers as
    /// a click.
    fn take(&mut self, ctx: &Context, id: Id) {
        let kind = self.ringed.as_ref().filter(|(on, _)| *on == id).map(|(_, kind)| kind.clone());
        match kind {
            Some(Kind::Field(holds)) => self.hold = Some(Hold::Keys(Keyboard::open(id, &holds))),
            Some(Kind::Level) => self.hold = Some(Hold::Level(id)),
            _ => push_key(ctx, egui::Key::Enter),
        }
    }

    /// A held level: left and right step it, as the arrow keys do. Up or
    /// down let go and go on; A, X or a shoulder let go.
    fn step_level(&mut self, ctx: &Context, mut asked: Asked) -> Asked {
        match asked.dir {
            Some(Dir::Left) => push_key(ctx, egui::Key::ArrowLeft),
            Some(Dir::Right) => push_key(ctx, egui::Key::ArrowRight),
            Some(_) => self.hold = None,
            None => {}
        }
        if matches!(asked.dir, Some(Dir::Left | Dir::Right)) {
            asked.dir = None;
        }
        if asked.press || asked.half || asked.hop != 0 {
            self.hold = None;
            asked.press = false;
            asked.half = false;
        }
        asked
    }

    /// The keys up over a field: the d-pad walks them, A types, X deletes
    /// and the shoulders move the caret, into the field, which has the
    /// focus for the pass to take it. False with no keys up, or when their
    /// field's gone.
    fn type_keys(&mut self, ctx: &Context, asked: &Asked) -> bool {
        let Some(Hold::Keys(keys)) = &mut self.hold else { return false };
        let field = keys.field;
        if ctx.read_response(field).is_none() {
            self.hold = None;
            return false;
        }
        if let Some(dir) = asked.dir {
            keys.go(dir);
        }
        let mut events = keys.caret(asked.hop);
        let mut done = false;
        for typed in [asked.half.then(|| keys.delete()), asked.press.then(|| keys.press())].into_iter().flatten() {
            match typed {
                Typed::Events(e) => events.extend(e),
                Typed::Done => done = true,
            }
        }
        if done {
            self.hold = None;
        }
        ctx.memory_mut(|m| m.request_focus(field));
        self.lent = Some(field);
        self.last = Some(field);
        self.typed = !events.is_empty();
        ctx.input_mut(|i| i.events.extend(events));
        true
    }

    /// B: lets go of what the ring holds, a level or a field's keys. True
    /// if it held one, and B goes no further.
    pub fn let_go(&mut self) -> bool {
        self.hold.take().is_some()
    }

    /// What the widget in focus is, from what egui told a screen reader as
    /// it took the focus or changed. A widget that keeps the focus says
    /// nothing more, so what it said is kept.
    fn note_kind(&mut self, ctx: &Context, id: Id) {
        match ctx.output(|o| kind_of(&o.events)) {
            Some(kind) => self.ringed = Some((id, kind)),
            None if self.ringed.as_ref().is_none_or(|(on, _)| *on != id) => self.ringed = Some((id, Kind::Press)),
            None => {}
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
        let focused = ctx.memory(|m| m.focused());
        if let Some(id) = focused {
            self.note_kind(ctx, id);
        }
        if let Some(Hold::Keys(keys)) = &mut self.hold {
            if focused != Some(keys.field) {
                // A line that took Return lets the focus go, and the keys go
                // with it
                self.hold = None;
            } else if let (false, Some((_, Kind::Field(holds)))) = (self.typed, &self.ringed) {
                keys.saw(holds);
            }
        }
        let Some(id) = focused else { return };
        let typing = egui::TextEdit::load_state(ctx, id).is_some();
        if self.lent == Some(id) || !typing {
            // A press can move the focus on: the ring follows it
            self.last = Some(id);
        }
        if self.lent == Some(id) && typing {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        let Some(response) = ctx.read_response(id) else { return };
        let (rect, time) = (response.interact_rect, ctx.input(|i| i.time));
        if !rect.is_positive() {
            return;
        }
        match &self.hold {
            Some(Hold::Keys(keys)) => {
                // The field holds still, and the ring goes round the keys
                let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("capitol_pad_ring")));
                painter.rect_stroke(rect.expand(3.0), 6.0, Stroke::new(2.0_f32, GOLD));
                keys.show(ctx, rect, time);
            }
            Some(Hold::Level(_)) => {
                ring(ctx, rect, time);
                held(ctx, rect);
            }
            None => ring(ctx, rect, time),
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
/// what only drags: a list's surface for a drag to scroll, or egui's own
/// slider (the panels draw theirs with `theme::level`, which takes a
/// click, so the ring goes to it), nor a scroll bar.
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
    spots.retain(|s| {
        // A widget scrolled out past its panel's edge (a list's next row) is
        // over whatever lies beyond, the world as like as not, so it's
        // judged at the edge nearest it (#517)
        let panel = ctx.memory(|m| m.area_rect(s.layer.id)).map(|p| p.shrink(1.0));
        let at = panel.filter(|p| p.is_positive()).map_or(s.rect.center(), |p| s.rect.center().clamp(p.min, p.max));
        ctx.layer_id_at(at).is_none_or(|top| top == s.layer)
    });
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

/// What the widget that took the focus or changed is, from this pass's
/// events: a `DragValue`'s number, a level, a field's text, or a press.
fn kind_of(events: &[OutputEvent]) -> Option<Kind> {
    let mut kind = None;
    for event in events {
        let info = event.widget_info();
        match info.typ {
            // A DragValue with the focus is a field inside: what it says of
            // itself wins over what its field says
            WidgetType::DragValue => return Some(Kind::Field(Field::Number(info.value))),
            WidgetType::Slider => return Some(Kind::Level),
            WidgetType::TextEdit => kind = Some(Kind::Field(Field::Text(info.current_text_value.clone().unwrap_or_default()))),
            _ if matches!(event, OutputEvent::FocusGained(_)) => kind = kind.or(Some(Kind::Press)),
            _ => {}
        }
    }
    kind
}

fn push_key(ctx: &Context, key: egui::Key) {
    ctx.input_mut(|i| {
        i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE })
    });
}

/// A held level's marks: a gold chevron each side, for the way the d-pad
/// steps it.
fn held(ctx: &Context, rect: Rect) {
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("capitol_pad_ring")));
    let y = rect.center().y;
    for (x, way) in [(rect.left() - 10.0, -1.0), (rect.right() + 10.0, 1.0)] {
        let tip = egui::pos2(x + 4.0 * way, y);
        let points = vec![tip, egui::pos2(x - 3.0 * way, y - 6.0), egui::pos2(x - 3.0 * way, y + 6.0)];
        painter.add(egui::Shape::convex_polygon(points, GOLD_LIGHT, Stroke::NONE));
    }
}

/// The ring: gold, a hair outside the widget, breathing a little so the eye
/// finds it, with a glow under it.
pub(super) fn ring(ctx: &Context, rect: Rect, time: f64) {
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

    /// `ScrollArea::show_rows` for the ring (#517): `total` rows of
    /// `row_height`, each drawn by `add_row` with its index, only those in
    /// view and one more past each edge, so the ring can step on to a row
    /// out of view and the list scrolls it in. Each row is a scope of its
    /// own, so its widgets keep their ids as the list scrolls and the ring
    /// stays on the one it was on.
    fn show_rows_following(
        self,
        ui: &mut Ui,
        row_height: f32,
        total: usize,
        add_row: impl FnMut(&mut Ui, usize),
    ) -> egui::scroll_area::ScrollAreaOutput<()>;
}

impl FollowFocus for egui::ScrollArea {
    fn show_following<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
        let out = self.show(ui, add);
        note_scroll_bars(ui.ctx(), out.id);
        keep_in_view(ui, &out);
        out
    }

    fn show_rows_following(
        self,
        ui: &mut Ui,
        row_height: f32,
        total: usize,
        mut add_row: impl FnMut(&mut Ui, usize),
    ) -> egui::scroll_area::ScrollAreaOutput<()> {
        let gap = ui.spacing().item_spacing.y;
        let step = row_height + gap;
        let out = self.show_viewport(ui, |ui, viewport| {
            ui.set_height((step * total as f32 - gap).max(0.0));
            let rows = rows_around(viewport.y_range(), step, total);
            let top = ui.max_rect().top();
            let span = egui::Rangef::new(top + rows.start as f32 * step, top + rows.end as f32 * step);
            let rect = Rect::from_x_y_ranges(ui.max_rect().x_range(), span);
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                // One id a row, as egui's own `show_rows` skips
                ui.skip_ahead_auto_ids(rows.start);
                for i in rows {
                    ui.scope(|ui| add_row(ui, i));
                }
            });
        });
        note_scroll_bars(ui.ctx(), out.id);
        keep_in_view(ui, &out);
        out
    }
}

/// The rows of a list of `total` rows `step` apart to lay out for `view`
/// (the list's own y, from its top): those in view, and one more past each
/// edge.
pub fn rows_around(view: egui::Rangef, step: f32, total: usize) -> std::ops::Range<usize> {
    if step <= 0.0 || total == 0 {
        return 0..0;
    }
    let first = ((view.min / step).floor().max(0.0) as usize).saturating_sub(1);
    let last = ((view.max / step).ceil().max(0.0) as usize + 1).min(total);
    first.min(last)..last
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
    use std::cell::{Cell, RefCell};

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

    /// One pass through egui with `panel` in an area, as the extension
    /// drives the ring.
    fn pass(ctx: &Context, nav: &mut PadNav, asked: Asked, mut panel: impl FnMut(&mut Ui)) {
        nav.ask(asked);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            nav.begin(ctx, true);
            egui::Area::new(Id::new("panel")).fixed_pos(egui::pos2(100.0, 100.0)).show(ctx, |ui| panel(ui));
            nav.end(ctx);
        });
    }

    fn go(dir: Dir) -> Asked {
        Asked { dir: Some(dir), ..Default::default() }
    }

    const A: Asked = Asked { dir: None, press: true, half: false, hop: 0 };
    const X: Asked = Asked { dir: None, press: false, half: true, hop: 0 };

    #[test]
    fn a_has_hold_of_a_level_and_left_and_right_step_it() {
        let ctx = super::super::theme::test_context();
        let mut nav = PadNav::default();
        let (volume, pressed) = (Cell::new(0.5_f32), Cell::new(0));
        let mut panel = |ui: &mut Ui| {
            ui.set_width(300.0);
            let mut v = volume.get();
            super::super::theme::level(ui, &mut v, 0.0..=1.0, |v| format!("{v:.2}"));
            volume.set(v);
            if ui.button("Done").clicked() {
                pressed.set(pressed.get() + 1);
            }
        };
        for _ in 0..3 {
            pass(&ctx, &mut nav, Asked::default(), &mut panel);
        }
        pass(&ctx, &mut nav, A, &mut panel);
        assert!(matches!(nav.hold, Some(Hold::Level(_))), "A takes hold of the level");
        pass(&ctx, &mut nav, go(Dir::Right), &mut panel);
        pass(&ctx, &mut nav, go(Dir::Right), &mut panel);
        pass(&ctx, &mut nav, go(Dir::Left), &mut panel);
        assert!((volume.get() - 0.55).abs() < 1e-5, "two steps up and one down: {}", volume.get());
        assert!(nav.let_go(), "B lets go");
        pass(&ctx, &mut nav, go(Dir::Right), &mut panel);
        assert!((volume.get() - 0.55).abs() < 1e-5, "let go, right steps it no more");
        pass(&ctx, &mut nav, go(Dir::Down), &mut panel);
        pass(&ctx, &mut nav, A, &mut panel);
        assert_eq!(pressed.get(), 1, "and the ring goes on to press what's under it");
    }

    #[test]
    fn a_field_takes_the_pads_keys_and_b_puts_them_away() {
        let ctx = super::super::theme::test_context();
        let mut nav = PadNav::default();
        let name = RefCell::new(String::new());
        let mut panel = |ui: &mut Ui| {
            ui.text_edit_singleline(&mut *name.borrow_mut());
        };
        for _ in 0..3 {
            pass(&ctx, &mut nav, Asked::default(), &mut panel);
        }
        pass(&ctx, &mut nav, A, &mut panel);
        assert!(matches!(nav.hold, Some(Hold::Keys(_))), "A brings the keys up");
        // The ring starts on q: the first letter of an empty field is a capital
        pass(&ctx, &mut nav, A, &mut panel);
        pass(&ctx, &mut nav, go(Dir::Right), &mut panel);
        pass(&ctx, &mut nav, A, &mut panel);
        pass(&ctx, &mut nav, A, &mut panel);
        assert_eq!(*name.borrow(), "Qww");
        pass(&ctx, &mut nav, X, &mut panel);
        assert_eq!(*name.borrow(), "Qw", "X deletes");
        assert_eq!(ctx.memory(|m| m.focused()), None, "the field gives the focus back each pass, so the pad stays the game's (#516)");
        assert!(nav.let_go());
        pass(&ctx, &mut nav, A, &mut panel);
        assert!(matches!(nav.hold, Some(Hold::Keys(_))), "the ring stayed on the field, and A brings them back");
        assert_eq!(*name.borrow(), "Qw");
    }

    #[test]
    fn a_whole_number_dragged_by_tenths_is_typed_over_and_steps_by_one() {
        let ctx = super::super::theme::test_context();
        let mut nav = PadNav::default();
        let count = Cell::new(12_u32);
        let mut panel = |ui: &mut Ui| {
            let mut n = count.get();
            ui.add(egui::DragValue::new(&mut n).range(1..=50).speed(0.1));
            count.set(n);
        };
        for _ in 0..3 {
            pass(&ctx, &mut nav, Asked::default(), &mut panel);
        }
        pass(&ctx, &mut nav, A, &mut panel);
        assert!(matches!(&nav.hold, Some(Hold::Keys(k)) if k.layout() == super::super::pad_keys::Layout::Numbers));
        // From 7 down to the steps' row, and right to +1
        for dir in [Dir::Down, Dir::Down, Dir::Down, Dir::Right, Dir::Right] {
            pass(&ctx, &mut nav, go(dir), &mut panel);
        }
        pass(&ctx, &mut nav, A, &mut panel);
        pass(&ctx, &mut nav, A, &mut panel);
        assert_eq!(count.get(), 14, "+1 twice, where egui's own arrow adds a tenth and rounds it away");
        // Up to the 3: the first digit typed replaces the number
        pass(&ctx, &mut nav, go(Dir::Up), &mut panel);
        pass(&ctx, &mut nav, A, &mut panel);
        assert_eq!(count.get(), 3);
        pass(&ctx, &mut nav, A, &mut panel);
        pass(&ctx, &mut nav, A, &mut panel);
        assert_eq!(count.get(), 50, "333 is past its most");
        pass(&ctx, &mut nav, Asked::default(), &mut panel);
        pass(&ctx, &mut nav, X, &mut panel);
        assert_eq!(count.get(), 5, "the pad took up the 50 it held, and X took its 0");
    }

    #[test]
    fn a_list_lays_out_a_row_past_each_edge() {
        assert_eq!(rows_around(egui::Rangef::new(0.0, 100.0), 33.0, 50), 0..5);
        assert_eq!(rows_around(egui::Rangef::new(330.0, 430.0), 33.0, 50), 9..15);
        assert_eq!(rows_around(egui::Rangef::new(1500.0, 1650.0), 33.0, 50), 44..50);
        assert_eq!(rows_around(egui::Rangef::new(0.0, 100.0), 33.0, 0), 0..0);
    }

    #[test]
    fn the_ring_walks_down_a_list_drawn_by_rows_past_its_foot() {
        let ctx = egui::Context::default();
        let mut nav = PadNav::default();
        let clicked = RefCell::new(Vec::new());
        let mut panel = |ui: &mut Ui| {
            egui::ScrollArea::vertical().max_height(200.0).show_rows_following(ui, 30.0, 100, |ui, i| {
                if ui.add_sized([200.0, 30.0], egui::Button::new(format!("Lot {i}"))).clicked() {
                    clicked.borrow_mut().push(i);
                }
            });
        };
        for _ in 0..3 {
            pass(&ctx, &mut nav, Asked::default(), &mut panel);
        }
        for _ in 0..12 {
            pass(&ctx, &mut nav, go(Dir::Down), &mut panel);
        }
        pass(&ctx, &mut nav, A, &mut panel);
        assert_eq!(*clicked.borrow(), vec![12], "twelve rows down, well past the six in view");
        for _ in 0..12 {
            pass(&ctx, &mut nav, go(Dir::Up), &mut panel);
        }
        pass(&ctx, &mut nav, A, &mut panel);
        assert_eq!(*clicked.borrow(), vec![12, 0], "and back up to the head");
    }
}
