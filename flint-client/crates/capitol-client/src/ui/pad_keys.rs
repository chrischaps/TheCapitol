//! The pad's keys (#517): with the ring on a field, A brings up a keyboard
//! beside it, and the d-pad walks its keys. A types the one in the ring, X
//! deletes, the shoulders move the caret, and B (or Done) puts it away.
//!
//! A number (a `DragValue`, or a field holding only digits) gets a number
//! pad instead, with a step down and up on either side of the 0. Its typing
//! is kept here and written over the field whole each time, so a field
//! that shows "3 days" or rounds a step of 0.1 away still takes it.
//!
//! Nothing here owns the field's text: what a key does is egui input
//! ([`Typed::Events`]) that [`super::pad_nav::PadNav`] pushes into the
//! pass it lends the field the focus for, as a key typed into it would be.

use super::pad_nav::Dir;
use super::theme::{self, size, GOLD, GOLD_LIGHT, KEYCAP, KEYCAP_EDGE, KEYCAP_SIDE, TEXT};
use egui::{Context, Event, Id, Modifiers, Order, Rect, Stroke};

/// Which keys are up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Letters,
    /// Digits and the marks, typed where the caret is.
    Symbols,
    /// A number pad: what's typed replaces the field.
    Numbers,
}

/// One key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cap {
    Char(char),
    Shift,
    Space,
    Delete,
    Return,
    Done,
    Swap(Layout),
    /// A number's step down or up.
    Step(i32),
}

impl Cap {
    fn label(self, shift: bool) -> String {
        match self {
            Cap::Char(c) if shift => c.to_uppercase().collect(),
            Cap::Char(c) => c.to_string(),
            Cap::Shift => "Shift".into(),
            Cap::Space => "space".into(),
            Cap::Delete => "Delete".into(),
            Cap::Return => "Return".into(),
            Cap::Done => "Done".into(),
            Cap::Swap(Layout::Letters) => "ABC".into(),
            Cap::Swap(_) => "123".into(),
            Cap::Step(n) if n < 0 => format!("−{}", -n),
            Cap::Step(n) => format!("+{n}"),
        }
    }
}

/// A row of keys, each with its width in keys.
type Row = Vec<(Cap, f32)>;

fn chars(s: &str) -> Row {
    s.chars().map(|c| (Cap::Char(c), 1.0)).collect()
}

/// The keys of `layout`. A number pad that may also take letters (a field
/// of digits, not a `DragValue`) has a key back to them.
pub fn rows(layout: Layout, letters_too: bool) -> Vec<Row> {
    let foot = |swap: Layout| vec![(Cap::Swap(swap), 1.5), (Cap::Space, 4.0), (Cap::Delete, 1.5), (Cap::Return, 1.5), (Cap::Done, 1.5)];
    match layout {
        Layout::Letters => {
            let mut third = vec![(Cap::Shift, 1.0)];
            third.extend(chars("zxcvbnm,."));
            vec![chars("qwertyuiop"), chars("asdfghjkl'"), third, foot(Layout::Symbols)]
        }
        Layout::Symbols => vec![chars("1234567890"), chars("-/:;()&@\"?"), chars("!#%+=_*$,."), foot(Layout::Letters)],
        Layout::Numbers => {
            let mut foot = vec![(Cap::Delete, 1.0), (Cap::Done, 1.0)];
            if letters_too {
                foot.insert(0, (Cap::Swap(Layout::Letters), 1.0));
            } else {
                foot[1].1 = 2.0;
            }
            vec![chars("789"), chars("456"), chars("123"), vec![(Cap::Step(-1), 1.0), (Cap::Char('0'), 1.0), (Cap::Step(1), 1.0)], foot]
        }
    }
}

/// What the field the keys are typing into holds, as the ring saw it last.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Text(String),
    /// A `DragValue`, and its value.
    Number(Option<f64>),
}

/// Whether a field's text is a number: only digits, and the commas and
/// spaces that group them.
pub fn is_number(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit()) && text.chars().all(|c| c.is_ascii_digit() || c == ',' || c == ' ')
}

/// A number being typed on the pad.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    typed: String,
    /// Nothing typed yet: the first digit replaces what the field held.
    fresh: bool,
    /// The field's own number, which a step goes from.
    value: Option<f64>,
}

impl Entry {
    fn of(value: Option<f64>) -> Self {
        Entry { typed: value.map(whole).unwrap_or_default(), fresh: true, value }
    }
}

fn whole(n: f64) -> String {
    format!("{}", n.round() as i64)
}

/// What a key did.
#[derive(Debug, Clone, PartialEq)]
pub enum Typed {
    /// Input for the field, this pass.
    Events(Vec<Event>),
    /// Done: the keys go.
    Done,
}

/// The keys, up over a field.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyboard {
    pub field: Id,
    layout: Layout,
    /// The ring's key: its row and place along it.
    at: (usize, usize),
    shift: bool,
    /// A number's typing, while the number pad is up.
    entry: Option<Entry>,
    /// A `DragValue` takes nothing but numbers.
    numbers_only: bool,
}

impl Keyboard {
    /// The keys for `field`: a number pad on a number, the letters
    /// otherwise, shifted for the first letter of an empty field.
    pub fn open(field: Id, holds: &Field) -> Self {
        let (layout, entry, numbers_only) = match holds {
            Field::Number(value) => (Layout::Numbers, Some(Entry::of(*value)), true),
            Field::Text(text) if is_number(text) => (Layout::Numbers, Some(Entry::of(digits(text))), false),
            Field::Text(_) => (Layout::Letters, None, false),
        };
        let shift = matches!(holds, Field::Text(t) if t.is_empty());
        Keyboard { field, layout, at: (0, 0), shift, entry, numbers_only }
    }

    #[cfg(test)]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    fn rows(&self) -> Vec<Row> {
        rows(self.layout, !self.numbers_only)
    }

    /// The key in the ring.
    pub fn cap(&self) -> Cap {
        let rows = self.rows();
        rows[self.at.0][self.at.1].0
    }

    /// The d-pad: along a row and round, or up and down to the key nearest
    /// above or below, round from the top to the foot.
    pub fn go(&mut self, dir: Dir) {
        let rows = self.rows();
        let (r, k) = self.at;
        let n = rows.len();
        match dir {
            Dir::Left => self.at.1 = (k + rows[r].len() - 1) % rows[r].len(),
            Dir::Right => self.at.1 = (k + 1) % rows[r].len(),
            Dir::Up | Dir::Down => {
                let centre = |row: &Row, i: usize| row[..i].iter().map(|c| c.1).sum::<f32>() + row[i].1 / 2.0;
                let x = centre(&rows[r], k);
                let to = if dir == Dir::Up { (r + n - 1) % n } else { (r + 1) % n };
                let nearest = (0..rows[to].len())
                    .min_by(|a, b| (centre(&rows[to], *a) - x).abs().total_cmp(&(centre(&rows[to], *b) - x).abs()))
                    .unwrap_or(0);
                self.at = (to, nearest);
            }
        }
    }

    /// A: the key in the ring.
    pub fn press(&mut self) -> Typed {
        let cap = self.cap();
        match cap {
            Cap::Done => return Typed::Done,
            Cap::Shift => self.shift = !self.shift,
            Cap::Swap(layout) => {
                self.layout = layout;
                if layout != Layout::Numbers {
                    self.entry = None;
                }
                let rows = self.rows();
                self.at.0 = self.at.0.min(rows.len() - 1);
                self.at.1 = self.at.1.min(rows[self.at.0].len() - 1);
            }
            Cap::Return => return Typed::Events(vec![key(egui::Key::Enter, Modifiers::NONE)]),
            Cap::Delete => return self.delete(),
            Cap::Space => return Typed::Events(vec![Event::Text(" ".into())]),
            Cap::Step(by) => {
                if let Some(entry) = &mut self.entry {
                    let from = entry.value.or_else(|| entry.typed.parse().ok()).unwrap_or(0.0);
                    let to = (from + by as f64).max(0.0);
                    entry.typed = whole(to);
                    entry.value = Some(to);
                    return Typed::Events(write_over(&entry.typed));
                }
            }
            Cap::Char(c) => {
                if let Some(entry) = &mut self.entry {
                    if entry.fresh {
                        entry.typed.clear();
                        entry.fresh = false;
                    }
                    entry.typed.push(c);
                    entry.value = entry.typed.parse().ok();
                    return Typed::Events(write_over(&entry.typed));
                }
                let text = if self.shift { c.to_uppercase().collect() } else { c.to_string() };
                self.shift = false;
                return Typed::Events(vec![Event::Text(text)]);
            }
        }
        Typed::Events(Vec::new())
    }

    /// X, or Delete: the character before the caret, or a number's last
    /// digit.
    pub fn delete(&mut self) -> Typed {
        if let Some(entry) = &mut self.entry {
            entry.fresh = false;
            entry.typed.pop();
            entry.value = entry.typed.parse().ok();
            return Typed::Events(write_over(&entry.typed));
        }
        Typed::Events(vec![key(egui::Key::Backspace, Modifiers::NONE)])
    }

    /// The shoulders: the caret along the text. A number has none.
    pub fn caret(&self, by: i32) -> Vec<Event> {
        if self.entry.is_some() || by == 0 {
            return Vec::new();
        }
        let way = if by < 0 { egui::Key::ArrowLeft } else { egui::Key::ArrowRight };
        (0..by.unsigned_abs()).map(|_| key(way, Modifiers::NONE)).collect()
    }

    /// What the field holds now, from a pass that typed nothing into it: a
    /// number the field held under what was typed (its most) is taken up.
    pub fn saw(&mut self, holds: &Field) {
        let Some(entry) = &mut self.entry else { return };
        let value = match holds {
            Field::Number(value) => *value,
            Field::Text(text) => digits(text),
        };
        let Some(value) = value else { return };
        if entry.typed.parse::<f64>().is_ok_and(|typed| typed > value) {
            entry.typed = whole(value);
        }
        entry.value = Some(value);
    }

    /// Draws the keys by the field at `field`, the ring on the key and the
    /// pad's words under them: under the field, or over it, or beside it
    /// where neither has the room, never over it.
    pub fn show(&self, ctx: &Context, field: Rect, time: f64) {
        let id = Id::new("capitol_pad_keys");
        let unit = if self.layout == Layout::Numbers { 40.0 } else { 30.0 };
        const GAP: f32 = 4.0;
        let size = ctx.memory(|m| m.area_rect(id)).map_or(egui::vec2(460.0, 230.0), |r| r.size());
        let (at, pivot) = place(ctx.screen_rect(), field, size);
        let mut ringed = None;
        egui::Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(at)
            .pivot(pivot)
            .constrain(true)
            .interactable(false)
            .show(ctx, |ui| {
                theme::panel_edged(GOLD).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
                    for (r, row) in self.rows().iter().enumerate() {
                        ui.horizontal(|ui| {
                            for (k, (cap, width)) in row.iter().enumerate() {
                                let size = egui::vec2(unit * width + GAP * (width - 1.0), unit);
                                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                                let lit = (*cap == Cap::Shift && self.shift) || (r, k) == self.at;
                                key_cap(ui, rect, &cap.label(self.shift), lit);
                                if (r, k) == self.at {
                                    ringed = Some(rect);
                                }
                            }
                        });
                    }
                    ui.add_space(2.0);
                    if self.layout == Layout::Numbers {
                        ui.horizontal(|ui| super::shortcuts::keys_row(ui, "{A} type  {X} delete"));
                        ui.horizontal(|ui| super::shortcuts::keys_row(ui, "{B} done"));
                    } else {
                        ui.horizontal(|ui| super::shortcuts::keys_row(ui, "{A} type  {X} delete  {LB}{RB} caret  {B} done"));
                    }
                });
            });
        if let Some(rect) = ringed {
            super::pad_nav::ring(ctx, rect, time);
        }
    }
}

/// Where keys of `size` go by `field` on `screen`, and which of their
/// corners that is: under the field, over it, then to its right or left,
/// 8 points off.
pub fn place(screen: Rect, field: Rect, size: egui::Vec2) -> (egui::Pos2, egui::Align2) {
    const OFF: f32 = 8.0;
    let room = |span: f32| span >= size.y + OFF;
    if room(screen.bottom() - field.bottom()) {
        (egui::pos2(field.left(), field.bottom() + OFF), egui::Align2::LEFT_TOP)
    } else if room(field.top() - screen.top()) {
        (egui::pos2(field.left(), field.top() - OFF), egui::Align2::LEFT_BOTTOM)
    } else if screen.right() - field.right() >= size.x + OFF {
        (egui::pos2(field.right() + OFF, field.center().y - size.y / 2.0), egui::Align2::LEFT_TOP)
    } else {
        (egui::pos2(field.left() - OFF, field.center().y - size.y / 2.0), egui::Align2::RIGHT_TOP)
    }
}

/// One key, a cap like the shortcut card's, gold while lit.
fn key_cap(ui: &egui::Ui, rect: Rect, label: &str, lit: bool) {
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, KEYCAP_SIDE);
    let face = Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - 2.0));
    painter.rect(face, 5.0, if lit { GOLD.gamma_multiply(0.22) } else { KEYCAP }, Stroke::new(1.0_f32, if lit { GOLD } else { KEYCAP_EDGE }));
    let font = if label.chars().count() > 1 { theme::strong(size::SMALL) } else { theme::strong(size::LARGE) };
    painter.text(face.center(), egui::Align2::CENTER_CENTER, label, font, if lit { GOLD_LIGHT } else { TEXT });
}

/// The digits in `text`, as a number.
fn digits(text: &str) -> Option<f64> {
    text.chars().filter(char::is_ascii_digit).collect::<String>().parse().ok()
}

fn key(key: egui::Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// Selects the field's text whole and types `text` over it (or deletes it).
fn write_over(text: &str) -> Vec<Event> {
    let all = key(egui::Key::A, Modifiers::COMMAND);
    if text.is_empty() {
        vec![all, key(egui::Key::Backspace, Modifiers::NONE)]
    } else {
        vec![all, Event::Text(text.to_owned())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(t: Typed) -> Vec<Event> {
        match t {
            Typed::Events(e) => e,
            Typed::Done => panic!("done"),
        }
    }

    /// Walks the ring to `cap` by the d-pad, row by row.
    fn walk_to(k: &mut Keyboard, cap: Cap) {
        for _ in 0..12 {
            for _ in 0..12 {
                if k.cap() == cap {
                    return;
                }
                k.go(Dir::Right);
            }
            k.go(Dir::Down);
        }
        panic!("no {cap:?} on {:?}", k.layout);
    }

    #[test]
    fn every_row_of_a_board_is_as_wide_as_the_rest() {
        for (layout, letters) in [(Layout::Letters, true), (Layout::Symbols, true), (Layout::Numbers, true), (Layout::Numbers, false)] {
            let widths: Vec<f32> = rows(layout, letters).iter().map(|r| r.iter().map(|c| c.1).sum()).collect();
            assert!(widths.windows(2).all(|w| w[0] == w[1]), "{layout:?}: {widths:?}");
        }
    }

    #[test]
    fn a_name_starts_shifted_and_types_where_the_caret_is() {
        let mut k = Keyboard::open(Id::new("f"), &Field::Text(String::new()));
        assert_eq!(k.layout(), Layout::Letters);
        walk_to(&mut k, Cap::Char('h'));
        assert_eq!(typed(k.press()), vec![Event::Text("H".into())], "the first letter is a capital");
        walk_to(&mut k, Cap::Char('i'));
        assert_eq!(typed(k.press()), vec![Event::Text("i".into())], "and the shift lets go");
        assert_eq!(typed(k.delete()), vec![key(egui::Key::Backspace, Modifiers::NONE)]);
        assert_eq!(k.caret(-2).len(), 2);
        walk_to(&mut k, Cap::Done);
        assert_eq!(k.press(), Typed::Done);
    }

    #[test]
    fn the_d_pad_goes_round_and_down_to_the_nearest_key() {
        let mut k = Keyboard::open(Id::new("f"), &Field::Text("x".into()));
        k.go(Dir::Left);
        assert_eq!(k.cap(), Cap::Char('p'), "round from the first to the last");
        k.go(Dir::Up);
        assert_eq!(k.cap(), Cap::Done, "up from the top is the foot");
        k.go(Dir::Up);
        assert_eq!(k.cap(), Cap::Char('.'));
        k.go(Dir::Left);
        k.go(Dir::Left);
        k.go(Dir::Down);
        assert_eq!(k.cap(), Cap::Return, "under m is Return");
    }

    #[test]
    fn a_number_is_typed_over_whole_and_steps() {
        let mut k = Keyboard::open(Id::new("n"), &Field::Number(Some(12.0)));
        assert_eq!(k.layout(), Layout::Numbers);
        walk_to(&mut k, Cap::Char('4'));
        assert_eq!(typed(k.press())[1], Event::Text("4".into()), "the first digit replaces the 12");
        walk_to(&mut k, Cap::Char('5'));
        assert_eq!(typed(k.press())[1], Event::Text("45".into()));
        walk_to(&mut k, Cap::Step(1));
        assert_eq!(typed(k.press())[1], Event::Text("46".into()));
        // The field's most is 40: it shows 40, and the next step goes from it
        k.saw(&Field::Number(Some(40.0)));
        walk_to(&mut k, Cap::Step(-1));
        assert_eq!(typed(k.press())[1], Event::Text("39".into()));
        let rubbed = typed(k.delete());
        assert_eq!(rubbed[1], Event::Text("3".into()));
        typed(k.delete());
        assert_eq!(typed(k.delete())[1], key(egui::Key::Backspace, Modifiers::NONE), "nothing left: the field's emptied");
        assert!(k.caret(1).is_empty(), "a number has no caret to move");
    }

    #[test]
    fn the_keys_go_under_a_field_then_over_then_beside_never_on_it() {
        let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(853.0, 480.0));
        let field = |y: f32| Rect::from_min_size(egui::pos2(200.0, y), egui::vec2(80.0, 24.0));
        let pad = egui::vec2(150.0, 300.0);
        assert_eq!(place(screen, field(20.0), pad).1, egui::Align2::LEFT_TOP, "room under");
        assert_eq!(place(screen, field(420.0), pad).1, egui::Align2::LEFT_BOTTOM, "room over");
        let (at, corner) = place(screen, field(240.0), pad);
        assert_eq!(corner, egui::Align2::LEFT_TOP);
        assert!(at.x >= 280.0, "beside, clear of the field: {at:?}");
        let wide = Rect::from_min_size(egui::pos2(600.0, 240.0), egui::vec2(200.0, 24.0));
        assert_eq!(place(screen, wide, pad).1, egui::Align2::RIGHT_TOP, "and on its left where the right has no room");
    }

    #[test]
    fn a_step_never_goes_under_nothing() {
        let mut k = Keyboard::open(Id::new("n"), &Field::Number(Some(0.0)));
        walk_to(&mut k, Cap::Step(-1));
        assert_eq!(typed(k.press())[1], Event::Text("0".into()));
    }

    #[test]
    fn a_field_of_digits_gets_the_number_pad_and_a_way_to_letters() {
        assert!(is_number("1,200") && !is_number("") && !is_number("Plot 5"));
        let mut k = Keyboard::open(Id::new("f"), &Field::Text("1,200".into()));
        assert_eq!(k.layout(), Layout::Numbers);
        walk_to(&mut k, Cap::Step(1));
        assert_eq!(typed(k.press())[1], Event::Text("1201".into()));
        walk_to(&mut k, Cap::Swap(Layout::Letters));
        typed(k.press());
        assert_eq!(k.layout(), Layout::Letters);
        walk_to(&mut k, Cap::Char('a'));
        assert_eq!(typed(k.press()), vec![Event::Text("a".into())], "letters type at the caret again");
        let mut drag = Keyboard::open(Id::new("d"), &Field::Number(None));
        for _ in 0..20 {
            assert!(!matches!(drag.cap(), Cap::Swap(_)), "a DragValue takes numbers only");
            drag.go(Dir::Right);
            drag.go(Dir::Down);
        }
    }
}
