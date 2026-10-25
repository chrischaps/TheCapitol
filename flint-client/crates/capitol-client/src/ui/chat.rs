//! The chat panel (Commons S1.3): somewhere to type, and somewhere to read
//! back what was said while you weren't looking.
//!
//! Bottom left and narrow. Two tabs, Local (within earshot, 3 m) and
//! Capitol (everyone), and a third, Guild, for a guild's members (S4.4),
//! each with what you haven't read. The log under them
//! keeps its foot in view as lines come; hover a line for when it was said.
//! A citizen's name carries their mark (C0.2), the lantern-pale lozenge the
//! HUD draws by their name in the world. The input line is at the foot:
//! Enter sends and keeps it open for the next, an empty Enter or Esc closes
//! it. A refused line (too fast, a watcher speaking locally) comes back to
//! the input with the reason under it ([`capitol_core::chat`]). The lines
//! are speech's log (S1.2, [`capitol_core::talk::Log`]), which the bubbles
//! over heads read too.
//!
//! Closed, the last three lines show where it stands on a faint backing,
//! fading as they age, so talk nearby is seen without opening anything.
//!
//! Enter opens it with the input focused ([`enter_means`]), unless the
//! crafting panel or an open station is set to a recipe, when Enter still
//! crafts. While the input has the keys, Flint gives them to egui alone, so
//! nothing typed walks, turns or acts (`flint-player`'s `egui_input`).

use super::pad_nav::FollowFocus;
use std::time::{Duration, Instant};

use capitol_core::chat::Chat;
use capitol_core::talk::{Line, Log};
use capitol_core::protocol::rules;
use capitol_core::protocol::ws::Channel;
use egui::{Align2, Color32, Id, Margin, RichText, Sense, Stroke};
use uuid::Uuid;

use super::theme::{self, size};

/// The panel's width, and the narrowest it goes to fit between panels.
const WIDTH: f32 = 340.0;
const NARROWEST: f32 = 250.0;
/// The log's height while open: a third of the window, within these.
const LOG_MOST: f32 = 190.0;
const LOG_LEAST: f32 = 80.0;
/// What the panel holds besides the log (tabs, input, a refusal, the
/// keys), before it has been drawn once.
const AROUND_LOG: f32 = 170.0;
/// Its distance from the window's left and bottom edges: above the
/// controls hint, which steps aside while it's open.
const EDGE: f32 = 16.0;
const FLOOR: f32 = 14.0;
/// Between it and a panel beside it.
const GAP: f32 = 12.0;
/// What `theme::panel_edged` adds to the content's width.
const FRAME: f32 = 30.0;
/// The open and close fade, in seconds, and how far it rises as it comes.
const FADE: f32 = 0.18;
const RISE: f32 = 10.0;
/// Closed: how many lines show, how long each stays bright, and how long
/// it then takes to fade away.
const RECENT: usize = 3;
/// About how tall those lines stand, wrapped.
const COLLAPSED: f32 = 110.0;
const BRIGHT: f64 = 12.0;
const FADING: f64 = 8.0;
/// A citizen's mark (C0.2), in their lantern's pale (`hud.rhai`,
/// `entities.rs`' CITIZEN_LANTERN).
const CITIZEN: Color32 = Color32::from_rgb(0xd6, 0xe4, 0xea);
/// A person's name, warm against the citizens' cool.
const PERSON: Color32 = theme::GOLD_LIGHT;

/// What Enter means this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enter {
    /// Open chat, with the input focused
    Chat,
    /// Craft what the crafting or station panel is set to
    Craft,
}

/// Enter crafts while the crafting panel (P5.5) or an open station's
/// (P6.3) is set to a recipe, and opens chat otherwise. The crafting panel
/// is open from the start (as React's always was), so "while it's open"
/// would leave Enter nothing to open chat with. A key typed into the chat's
/// input never comes here: egui has it.
pub fn enter_means(crafting_ready: bool, at_station: bool) -> Enter {
    if crafting_ready || at_station {
        Enter::Craft
    } else {
        Enter::Chat
    }
}

/// What the panel needs besides the talk itself.
pub struct View {
    pub now: Instant,
    /// The talk log's clock now, in seconds (`Session::talk_clock`)
    pub clock: f64,
    /// Who you are, for your own lines
    pub me: Option<Uuid>,
    /// A panel at the left it keeps clear of (the crafting panel)
    pub beside: Option<egui::Rect>,
    /// One at the right it narrows for (the inventory panel)
    pub before: Option<egui::Rect>,
    /// Names open a person's card (S0.3). Offline they're plain.
    pub cards: bool,
    /// Your guild's name, when you're in one: the Guild tab (S4.4)
    pub guild: Option<String>,
}

/// What the reader did.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Say the input line on this channel
    Say(Channel),
    /// Open this person's card
    Card { id: Uuid, name: String, citizen: bool },
}

/// The panel.
pub struct ChatPanel {
    pub open: bool,
    tab: Channel,
    /// Opened for typing: focus the input once it's drawn
    focus: bool,
    was_open: bool,
    /// Where it was drawn, open or closed, for panels that keep clear
    pub rect: Option<egui::Rect>,
    /// How tall it stood open, last time
    height: Option<f32>,
}

impl Default for ChatPanel {
    fn default() -> Self {
        Self { open: false, tab: Channel::Local, focus: false, was_open: false, rect: None, height: None }
    }
}

/// Where the panel stands this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Place {
    left: f32,
    width: f32,
    log: f32,
}

impl ChatPanel {
    /// Open it with the input focused (Enter, or the header's chip).
    pub fn open_typing(&mut self) {
        self.open = true;
        self.focus = true;
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open_typing();
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.focus = false;
    }

    pub fn show(&mut self, ctx: &egui::Context, log: &mut Log, chat: &mut Chat, view: &View) -> Option<Action> {
        // Out of your guild, its tab goes
        if self.tab == Channel::Guild && view.guild.is_none() {
            self.tab = Channel::Local;
        }
        // What's on the open tab counts as read
        if self.open {
            log.read(self.tab);
        }
        let shown = ctx.animate_bool_with_time(Id::new("capitol_chat_fade"), self.open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            self.was_open = false;
            // Three lines over the controls hint: a corner a panel seldom reaches
            let low = place(ctx.screen_rect(), Some(super::CONTROLS_HEIGHT + COLLAPSED), view.beside, view.before);
            self.collapsed(ctx, log, view, low);
            return None;
        }
        let place = place(ctx.screen_rect(), self.height, view.beside, view.before);
        let ease = 1.0 - (1.0 - shown).powi(3);
        if self.open && !self.was_open {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, Id::new("capitol_chat")));
        }
        self.was_open = self.open;
        let mut action = None;
        let area = egui::Area::new(Id::new("capitol_chat"))
            .anchor(Align2::LEFT_BOTTOM, [place.left, -FLOOR + RISE * (1.0 - ease)])
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(theme::GOLD_DIM).show(ui, |ui| {
                    ui.set_width(place.width);
                    action = self.body(ui, log, chat, view, place);
                });
            });
        self.rect = self.open.then_some(area.response.rect);
        if self.open {
            self.height = Some(area.response.rect.height());
        }
        action
    }

    fn body(&mut self, ui: &mut egui::Ui, log: &Log, chat: &mut Chat, view: &View, place: Place) -> Option<Action> {
        let mut action = None;
        self.tabs(ui, log, view.guild.as_deref());
        theme::divider(ui);
        ui.add_space(4.0);
        let tab = self.tab;
        egui::ScrollArea::vertical()
            .id_salt(("capitol_chat_log", self.tab))
            .max_height(place.log)
            .min_scrolled_height(place.log)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_following(ui, |ui| {
                ui.set_max_width(place.width - 12.0);
                ui.spacing_mut().item_spacing.y = 3.0;
                if log.lines(tab).next().is_none() {
                    ui.add_space(place.log * 0.4);
                    ui.vertical_centered(|ui| ui.label(theme::dim(quiet(tab)).italics()));
                }
                for line in log.lines(tab) {
                    if said(ui, line, view, view.cards, 1.0, true) {
                        action = Some(Action::Card { id: line.from, name: line.name.clone(), citizen: line.citizen });
                    }
                }
            });
        ui.add_space(6.0);
        if let Some(a) = self.input(ui, chat, place.width) {
            action = Some(a);
        }
        if let Some(r) = chat.refusal(view.now) {
            ui.add_space(4.0);
            theme::error_box(ui, &r.message);
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            theme::keycap(ui, "Enter");
            ui.label(theme::dim("to send").size(size::LABEL));
            ui.add_space(6.0);
            theme::keycap(ui, "Esc");
            ui.label(theme::dim("to close").size(size::LABEL));
        });
        action
    }

    /// Local, Capitol and, in a guild, Guild, each with its unread count,
    /// and the close cross.
    fn tabs(&mut self, ui: &mut egui::Ui, log: &Log, guild: Option<&str>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let channels = [Channel::Local, Channel::Capitol].into_iter().chain(guild.map(|_| Channel::Guild));
            for channel in channels {
                let unread = log.unread(channel);
                let word = match channel {
                    Channel::Local => "Local",
                    Channel::Capitol => "Capitol",
                    Channel::Guild => "Guild",
                };
                let name = match unread {
                    0 => word.to_owned(),
                    n => format!("{word} · {n}"),
                };
                let hover = match channel {
                    Channel::Local => "Heard by those within 3 m of you".to_owned(),
                    Channel::Capitol => "Heard by everyone in the city".to_owned(),
                    Channel::Guild => format!("Heard by {}, wherever they are", guild.unwrap_or("your guild")),
                };
                if theme::toggle_chip(ui, &name, self.tab == channel).on_hover_text(hover).clicked() {
                    self.tab = channel;
                    self.focus = true;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::link(ui, "×", size::LARGE).on_hover_text("Close (Esc)").clicked() {
                    self.close();
                }
            });
        });
        ui.add_space(4.0);
    }

    /// The input line. Enter sends and keeps it open; an empty Enter or
    /// Esc closes the panel.
    fn input(&mut self, ui: &mut egui::Ui, chat: &mut Chat, width: f32) -> Option<Action> {
        let hint = match self.tab {
            Channel::Local => "Say something nearby…",
            Channel::Capitol => "Say something to the city…",
            Channel::Guild => "Say something to your guild…",
        };
        let field = egui::TextEdit::singleline(&mut chat.draft)
            .id(Id::new("capitol_chat_input"))
            .hint_text(hint)
            .char_limit(rules::talk::MAX_LINE)
            .desired_width(width)
            .margin(Margin::symmetric(8.0, 6.0));
        let response = ui.add(field);
        // After it's drawn, so the Enter that opened it isn't read as a send
        if std::mem::take(&mut self.focus) && self.open {
            response.request_focus();
        }
        if !response.lost_focus() {
            return None;
        }
        let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        if escape || (enter && chat.draft.trim().is_empty()) {
            self.close();
            None
        } else if enter {
            // Ready for the next line
            response.request_focus();
            Some(Action::Say(self.tab))
        } else {
            None
        }
    }

    /// Closed: the last lines on a faint backing, fading as they age. Not
    /// interactable, so clicks reach the world.
    fn collapsed(&mut self, ctx: &egui::Context, log: &Log, view: &View, place: Place) {
        let lines: Vec<(&Line, f32)> = recent(log, RECENT)
            .into_iter()
            .map(|l| (l, fade(view.clock - l.at)))
            .filter(|(_, a)| *a > 0.0)
            .collect();
        if lines.is_empty() {
            return;
        }
        // Lift clear of the controls hint
        let area = egui::Area::new(Id::new("capitol_chat_recent"))
            .anchor(Align2::LEFT_BOTTOM, [place.left, -(FLOOR + super::CONTROLS_HEIGHT)])
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width(place.width);
                theme::hint().show(ui, |ui| {
                    ui.set_max_width(place.width - 20.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (line, alpha) in &lines {
                        said(ui, line, view, false, *alpha, false);
                    }
                });
            });
        self.rect = Some(area.response.rect);
        // Keep drawing while a line fades
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

/// How bright a closed line is, `age` seconds after it was heard.
fn fade(age: f64) -> f32 {
    (1.0 - (age - BRIGHT) / FADING).clamp(0.0, 1.0) as f32
}

/// The last `n` lines heard on any channel, oldest first.
fn recent(log: &Log, n: usize) -> Vec<&Line> {
    let mut last: Vec<&Line> =
        [Channel::Local, Channel::Capitol, Channel::Guild].into_iter().flat_map(|c| log.lines(c).rev().take(n)).collect();
    // Stable, so lines heard in one frame keep the order they came in
    last.sort_by(|a, b| a.at.total_cmp(&b.at));
    last.split_off(last.len().saturating_sub(n))
}

/// Where it stands in `screen`, `height` tall when last open: in the
/// corner, or beside a panel at the left (`beside`) that reaches down into
/// it. Narrowed to stop short of one at the right (`before`), but never
/// below [`NARROWEST`]: past that it goes over it. The log takes a third
/// of the window's height.
fn place(screen: egui::Rect, height: Option<f32>, beside: Option<egui::Rect>, before: Option<egui::Rect>) -> Place {
    let log = (screen.height() / 3.0).clamp(LOG_LEAST, LOG_MOST);
    let top = screen.bottom() - FLOOR - height.unwrap_or(log + AROUND_LOG);
    let left = match beside {
        Some(r) if r.bottom() + GAP > top => r.right() + GAP,
        _ => EDGE,
    };
    let room = before
        .filter(|r| r.bottom() + GAP > top && r.right() > left)
        .map_or(WIDTH + FRAME, |r| r.left() - GAP - left);
    // The frame's margins and edge come on top of the content's width
    let width = (room - FRAME).clamp(NARROWEST, WIDTH);
    Place { left, width, log }
}

/// What an empty tab says.
fn quiet(channel: Channel) -> &'static str {
    match channel {
        Channel::Local => "Nobody nearby has said anything yet.",
        Channel::Capitol => "The city is quiet.",
        Channel::Guild => "Nobody in the guild has said anything yet.",
    }
}

/// One line: the name (a link to their card when `link`), a citizen's
/// mark, then what they said. Closed (`!open`), a Capitol or a guild line
/// says so.
/// Hover it for when it was said. True if the name was clicked.
fn said(ui: &mut egui::Ui, line: &Line, view: &View, link: bool, alpha: f32, open: bool) -> bool {
    let mut clicked = false;
    let mine = view.me == Some(line.from);
    let row = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.multiply_opacity(alpha);
        match line.channel {
            Channel::Capitol if !open => {
                ui.label(RichText::new("Capitol").size(size::LABEL).color(theme::GOLD_DIM));
            }
            Channel::Guild if !open => {
                ui.label(RichText::new("Guild").size(size::LABEL).color(theme::GUILD));
            }
            _ => {}
        }
        if line.citizen {
            citizen_mark(ui, size::BODY);
        }
        let colour = match (mine, line.citizen) {
            (true, _) => theme::GOLD,
            (false, true) => CITIZEN,
            (false, false) => PERSON,
        };
        let name = RichText::new(&line.name).font(theme::strong(size::BODY)).color(colour);
        if link && !mine {
            let r = ui
                .add(egui::Label::new(name).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("Who is {}?", line.name));
            if r.hovered() {
                ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0_f32, colour.gamma_multiply(0.6)));
            }
            clicked = r.clicked();
        } else {
            ui.label(name);
        }
        ui.label(RichText::new(&line.text).size(size::BODY).color(theme::TEXT));
    });
    if open {
        row.response.interact(Sense::hover()).on_hover_text(when(view.clock - line.at));
    }
    clicked
}

/// When a line said `ago` seconds back was said, in this machine's time:
/// "14:05".
fn when(ago: f64) -> String {
    let ago = chrono::Duration::milliseconds((ago.max(0.0) * 1000.0) as i64);
    (chrono::Local::now() - ago).format("%H:%M").to_string()
}

/// A citizen's mark (C0.2) before their name: the slender lozenge, a lamp's
/// pane, in their lantern's pale, as the HUD draws it in the world. `text`
/// is the name's size.
fn citizen_mark(ui: &mut egui::Ui, text: f32) {
    let r = crate::labels::GLYPH_R * 1.2 * text;
    let w = r * 0.72;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(w * 2.0 + 2.0, text), Sense::hover());
    let c = rect.center() + egui::vec2(0.0, text * 0.06);
    let points = |c: egui::Pos2| {
        vec![c + egui::vec2(0.0, -r), c + egui::vec2(w, 0.0), c + egui::vec2(0.0, r), c + egui::vec2(-w, 0.0)]
    };
    let painter = ui.painter();
    painter.add(egui::Shape::convex_polygon(points(c + egui::vec2(1.0, 1.0)), Color32::from_black_alpha(160), Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(points(c), CITIZEN.gamma_multiply(0.9), Stroke::NONE));
    response.on_hover_text("A citizen: an AI who lives, works and trades in the city, by the same rules as anyone");
}

/// `--chat-sample`: all three tabs, a few people and citizens talking, a line
/// of yours, and one refused, back in the input with the reason under it.
/// The log's clock is `clock` now; you are the id returned.
pub fn sample(now: Instant, clock: f64) -> (Log, Chat, Uuid) {
    let me = Uuid::from_u128(1);
    let (ada, cy, wen, rook) = (Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4), Uuid::from_u128(5));
    let mut log = Log::new();
    let mut chat = Chat::new();
    let lines: &[(Uuid, &str, bool, Channel, &str)] = &[
        (cy, "Cy Marlowe", true, Channel::Capitol, "Oak planks at the bazaar, stall 4. Good grain, fair price."),
        (rook, "Rook Ashdown", false, Channel::Capitol, "Anyone selling iron ore above B? Paying well."),
        (ada, "Ada Reeder", false, Channel::Local, "Morning. Is the forge on this lot yours?"),
        (me, "Wren Hollis", false, Channel::Local, "It is. Use it if you like, the fire's still warm."),
        (wen, "Wen Calder", true, Channel::Local, "Good morning to you both."),
        (cy, "Cy Marlowe", true, Channel::Capitol, "And clay, while the river's low."),
        (rook, "Rook Ashdown", false, Channel::Guild, "Fiber's in the long shed. Take what you need for the order."),
        (ada, "Ada Reeder", false, Channel::Local, "Thanks! I'll bring the copper back as ingots."),
    ];
    for (i, (from, name, citizen, channel, text)) in lines.iter().enumerate() {
        log.heard(Line {
            from: *from,
            name: (*name).into(),
            citizen: *citizen,
            channel: *channel,
            text: (*text).into(),
            x: 0.0,
            y: 0.0,
            at: clock - 20.0 + 2.0 * i as f64,
        });
    }
    // One said too soon after the last, back in the input with the reason
    chat.draft = "Twelve oak planks going, anyone?".into();
    chat.send(Channel::Capitol);
    chat.refused("You're talking too fast. Try again in 4 s", now);
    (log, chat, me)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_opens_chat_unless_theres_a_recipe_to_craft() {
        assert_eq!(enter_means(false, false), Enter::Chat);
        assert_eq!(enter_means(true, false), Enter::Craft);
        assert_eq!(enter_means(false, true), Enter::Craft);
    }

    /// The panel over a few frames at 1280×720, each frame's input events
    /// given. What it asked for, and whether egui had the keyboard at the
    /// end.
    fn run(ctx: &egui::Context, panel: &mut ChatPanel, chat: &mut Chat, frames: &[Vec<egui::Event>]) -> (Vec<Action>, bool) {
        run_with(ctx, panel, &mut Log::new(), chat, frames)
    }

    fn run_with(
        ctx: &egui::Context,
        panel: &mut ChatPanel,
        log: &mut Log,
        chat: &mut Chat,
        frames: &[Vec<egui::Event>],
    ) -> (Vec<Action>, bool) {
        let mut asked = Vec::new();
        let mut keys = false;
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                events: events.clone(),
                ..Default::default()
            };
            let view = View { now: Instant::now(), clock: 30.0, me: None, beside: None, before: None, cards: true, guild: None };
            let _ = ctx.run(raw, |ctx| {
                theme::install(ctx);
                asked.extend(panel.show(ctx, log, chat, &view));
            });
            keys = ctx.wants_keyboard_input();
        }
        (asked, keys)
    }

    fn key(key: egui::Key) -> Vec<egui::Event> {
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            })
            .collect()
    }

    fn typed(text: &str) -> Vec<egui::Event> {
        vec![egui::Event::Text(text.into())]
    }

    #[test]
    fn opened_by_enter_it_takes_the_keys_and_the_enter_isnt_a_send() {
        let ctx = theme::test_context();
        let mut panel = ChatPanel::default();
        let mut chat = Chat::new();
        // The Enter that opened it is in the same frame's input
        panel.open_typing();
        let (asked, keys) = run(&ctx, &mut panel, &mut chat, &[key(egui::Key::Enter), vec![], vec![]]);
        assert!(asked.is_empty(), "{asked:?}");
        assert!(panel.open, "the opening Enter didn't close it");
        assert!(keys, "the input has the keyboard, so Flint keeps keys from the game");
    }

    #[test]
    fn typed_keys_go_to_the_line_not_the_world() {
        let ctx = theme::test_context();
        let mut panel = ChatPanel::default();
        let mut chat = Chat::new();
        panel.open_typing();
        let mut frames = vec![vec![], vec![]];
        // WASD and E, typed: the line has them, and egui held the keys
        let mut wasd = typed("wasd e");
        for k in [egui::Key::W, egui::Key::A, egui::Key::S, egui::Key::D, egui::Key::E] {
            wasd.extend(key(k));
        }
        frames.push(wasd);
        let (asked, keys) = run(&ctx, &mut panel, &mut chat, &frames);
        assert!(asked.is_empty());
        assert_eq!(chat.draft, "wasd e");
        assert!(keys);
    }

    #[test]
    fn enter_sends_and_stays_open_and_an_empty_enter_closes() {
        let ctx = theme::test_context();
        let mut panel = ChatPanel::default();
        let mut chat = Chat::new();
        panel.open_typing();
        let frames = [vec![], vec![], typed("hello there"), key(egui::Key::Enter), vec![], vec![]];
        let (asked, keys) = run(&ctx, &mut panel, &mut chat, &frames);
        assert_eq!(asked, [Action::Say(Channel::Local)]);
        assert!(panel.open);
        assert!(keys, "still focused for the next line");
        // The session sends it, which clears the line
        assert_eq!(chat.send(Channel::Local).as_deref(), Some("hello there"));
        assert!(chat.draft.is_empty());
        let (asked, _) = run(&ctx, &mut panel, &mut chat, &[vec![], key(egui::Key::Enter), vec![]]);
        assert!(asked.is_empty());
        assert!(!panel.open, "an empty Enter closes it");
    }

    #[test]
    fn escape_closes_it() {
        let ctx = theme::test_context();
        let mut panel = ChatPanel::default();
        let mut chat = Chat::new();
        panel.open_typing();
        chat.draft = "never mind".into();
        let (asked, keys) = run(&ctx, &mut panel, &mut chat, &[vec![], vec![], key(egui::Key::Escape), vec![]]);
        assert!(asked.is_empty());
        assert!(!panel.open);
        assert!(!keys, "the keys go back to the game");
        assert_eq!(chat.draft, "never mind", "what was typed waits for next time");
    }

    #[test]
    fn a_refusal_keeps_the_line_and_shows_why() {
        let ctx = theme::test_context();
        let mut panel = ChatPanel::default();
        let mut chat = Chat::new();
        panel.open_typing();
        run(&ctx, &mut panel, &mut chat, &[vec![], vec![], typed("too fast")]);
        chat.send(Channel::Capitol);
        chat.refused("You're talking too fast. Try again in 2 s", Instant::now());
        assert_eq!(chat.draft, "too fast");
        let (_, keys) = run(&ctx, &mut panel, &mut chat, &[vec![], vec![]]);
        assert!(panel.open && keys);
    }

    #[test]
    fn the_open_tab_counts_as_read() {
        let ctx = theme::test_context();
        let mut panel = ChatPanel::default();
        let (mut log, mut chat, _) = sample(Instant::now(), 30.0);
        assert!(log.unread(Channel::Local) > 0);
        run_with(&ctx, &mut panel, &mut log, &mut chat, &[vec![]]);
        assert!(log.unread(Channel::Local) > 0, "closed, nothing is read");
        panel.open_typing();
        run_with(&ctx, &mut panel, &mut log, &mut chat, &[vec![]]);
        assert_eq!(log.unread(Channel::Local), 0);
        assert!(log.unread(Channel::Capitol) > 0);
    }

    #[test]
    fn closed_it_shows_the_last_lines_across_every_channel() {
        let (log, _, _) = sample(Instant::now(), 30.0);
        let texts: Vec<_> = recent(&log, 3).iter().map(|l| l.text.as_str()).collect();
        // Capitol, Guild, then Local: one from each
        assert_eq!(
            texts,
            [
                "And clay, while the river's low.",
                "Fiber's in the long shed. Take what you need for the order.",
                "Thanks! I'll bring the copper back as ingots."
            ]
        );
    }

    #[test]
    fn it_takes_the_corner_unless_a_panel_reaches_down_into_it() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 576.0));
        let p = place(screen, None, None, None);
        assert_eq!((p.left, p.width), (EDGE, WIDTH));
        assert_eq!(p.log, LOG_MOST);
        // The crafting panel, short: the chat stays in the corner under it
        let short = egui::Rect::from_min_max(egui::pos2(16.0, 40.0), egui::pos2(316.0, 200.0));
        assert_eq!(place(screen, Some(300.0), Some(short), None).left, EDGE);
        // Tall: it moves beside it
        let tall = egui::Rect::from_min_max(egui::pos2(16.0, 40.0), egui::pos2(316.0, 400.0));
        assert_eq!(place(screen, Some(300.0), Some(tall), None).left, 316.0 + GAP);
    }

    #[test]
    fn it_narrows_between_panels_but_no_further_than_it_reads() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 576.0));
        let crafting = egui::Rect::from_min_max(egui::pos2(16.0, 40.0), egui::pos2(316.0, 400.0));
        let inventory = egui::Rect::from_min_max(egui::pos2(660.0, 260.0), egui::pos2(1004.0, 556.0));
        let p = place(screen, Some(300.0), Some(crafting), Some(inventory));
        assert!(p.left + p.width + FRAME <= inventory.left() - GAP + 0.01, "{p:?}");
        assert!(p.width < WIDTH);
        // 853 points across (1280 at 150%): too narrow to fit, so it keeps NARROWEST
        let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(853.0, 480.0));
        let inventory = egui::Rect::from_min_max(egui::pos2(500.0, 180.0), egui::pos2(833.0, 460.0));
        assert_eq!(place(small, Some(300.0), Some(crafting), Some(inventory)).width, NARROWEST);
        assert_eq!(place(small, None, None, None).log, 160.0);
    }

    #[test]
    fn closed_lines_fade_with_age() {
        assert_eq!(fade(0.0), 1.0);
        assert_eq!(fade(BRIGHT), 1.0);
        assert!((fade(BRIGHT + FADING / 2.0) - 0.5).abs() < 1e-3);
        assert_eq!(fade(BRIGHT + FADING), 0.0);
    }
}
