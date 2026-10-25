//! Letters (S1.5): the mailbox, top centre under the header.
//!
//! L, or the header's Letters chip, opens it; *Write* on a person's card
//! opens it on a letter to them. Two boxes, Received and Sent, each a list
//! of envelopes: a sealed one, paper with a wax seal, for a letter you
//! haven't read, an open one for the rest, the unread first. A letter opens
//! on a sheet of the Gazette's paper in its ink (`theme::PAPER`,
//! `theme::INK`), sealed at its foot with its writer's initial, with
//! *Reply* and *Drop* under it. A letter from one of the city's offices
//! (the Registrar's, #447) is sealed with the Capitol's tower instead
//! (`theme::capitol_seal`), its name opens no card, and it takes no reply. Writing is the same sheet, with a pen:
//! what's left of the 1000 characters, and the Strand of postage.
//!
//! The rules are `capitol_core::mail`'s, and the panel works them directly.
//! Only what reaches past the mailbox comes back as an [`Action`]: a name
//! that opens a card, and the ×.

use super::pad_nav::FollowFocus;
use capitol_core::chrono::{DateTime, Utc};
use capitol_core::mail::{self, Mail, Showing};
use capitol_core::protocol::rest::{Correspondent, Letter, LetterBox};
use capitol_core::protocol::rules::talk;
use egui::{Align, Align2, Color32, Id, Layout, Margin, RichText, Sense, Stroke};

use super::theme::{self, size, DIM, GOLD, TEXT};

/// The open and close fade, in seconds, and how far it rises as it comes.
const FADE: f32 = 0.2;
const RISE: f32 = 12.0;
/// The panel's widest.
const WIDTH: f32 = 500.0;
/// A row of the list: its height, and the envelope's size in it.
const ROW: f32 = 44.0;
const ENVELOPE: egui::Vec2 = egui::vec2(20.0, 14.0);
/// A letter's writing, and the heading on its sheet.
const HAND: f32 = 15.0;
const SALUTATION: f32 = 17.0;

/// What the mailbox asks of the rest of the game.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// See who someone is: their card (S0.3)
    Card(Correspondent),
    Close,
}

/// What the mailbox draws from, beside the letters themselves.
pub struct View {
    /// Your Strands, for the postage. `None` until the server says.
    pub strands: Option<i64>,
    /// The server's clock: when a letter is read, and how long ago one was written.
    pub now: DateTime<Utc>,
    /// Signed in. Offline it's a sample, and nothing is sent.
    pub online: bool,
}

/// The panel.
#[derive(Default)]
pub struct MailPanel {
    pub open: bool,
    was_open: bool,
    /// Where it was drawn this frame, for the hints to keep clear of.
    pub rect: Option<egui::Rect>,
}

impl MailPanel {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn show(&mut self, ctx: &egui::Context, mail: &mut Mail, view: &View) -> Option<Action> {
        let shown = ctx.animate_bool_with_time(Id::new("capitol_mail_fade"), self.open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            self.was_open = false;
            return None;
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        let screen = ctx.available_rect();
        if self.open && !self.was_open {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, Id::new("capitol_mail")));
        }
        self.was_open = self.open;
        let width = WIDTH.min(screen.width() - 32.0).max(260.0);
        // Under the header, and clear of the bottom edge
        let room = (screen.height() - 24.0 - 24.0).max(180.0);
        let mut action = None;
        let area = egui::Area::new(Id::new("capitol_mail"))
            .anchor(Align2::CENTER_TOP, [0.0, screen.top() + 12.0 - RISE * (1.0 - ease)])
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(theme::PAPER_EDGE).show(ui, |ui| {
                    ui.set_width(width - 24.0);
                    ui.set_max_height(room);
                    action = body(ui, mail, view);
                });
            });
        self.rect = self.open.then_some(area.response.rect);
        if action == Some(Action::Close) {
            self.open = false;
        }
        action
    }
}

fn body(ui: &mut egui::Ui, mail: &mut Mail, view: &View) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(theme::title("Letters", size::TITLE * 0.7));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let close = ui.add(
                egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM))
                    .frame(false)
                    .min_size(egui::vec2(22.0, 22.0)),
            );
            if close.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Close (L)").clicked() {
                action = Some(Action::Close);
            }
        });
    });
    ui.add_space(2.0);
    theme::rule_in(ui, theme::PAPER_EDGE.gamma_multiply(0.45));
    ui.add_space(8.0);
    let shown = match mail.showing() {
        Showing::List => list(ui, mail, view),
        Showing::Letter(id) => match mail.letter(id).cloned() {
            Some(letter) => read(ui, mail, &letter, view),
            None => {
                mail.back();
                None
            }
        },
        Showing::Writing => write(ui, mail, view),
    };
    action.or(shown)
}

/// The boxes: the tabs, the letter in your hand, and the envelopes.
fn list(ui: &mut egui::Ui, mail: &mut Mail, view: &View) -> Option<Action> {
    ui.horizontal(|ui| {
        let received = match mail.unread() {
            0 => "Received".to_string(),
            n => format!("Received · {n}"),
        };
        let mut tab = mail.tab;
        theme::segmented(ui, &mut tab, &[(LetterBox::In, received.as_str()), (LetterBox::Out, "Sent")]);
        mail.tab = tab;
        if let Some(draft) = mail.draft() {
            let to = draft.to.clone();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::link(ui, &format!("Your letter to {}", to.name), size::SMALL)
                    .on_hover_text("Go back to the letter you were writing")
                    .clicked()
                {
                    mail.write_to(to);
                }
            });
        }
    });
    ui.add_space(8.0);
    let which = mail.tab;
    let letters: Vec<Letter> = mail.letters(which).into_iter().cloned().collect();
    if letters.is_empty() {
        let words = if !mail.loaded(which) && view.online {
            "Fetching your letters…"
        } else if which == LetterBox::In {
            "No letters have come for you yet."
        } else {
            "You haven't written to anyone yet. Write from a person's card: click them, or their name."
        };
        ui.add_space(10.0);
        ui.label(theme::dim(words).italics());
        ui.add_space(10.0);
        return None;
    }
    let room = (ui.available_height() - 4.0).max(ROW * 2.0);
    theme::scrolled(ui, "capitol_mail_list", room, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for letter in &letters {
            row(ui, mail, letter, which, view);
        }
        if mail.more(which) {
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                if theme::link(ui, "Older letters", size::SMALL).clicked() {
                    mail.older(which);
                }
            });
        }
    });
    None
}

/// One envelope in the list: who, when, and how it begins. A click opens it.
fn row(ui: &mut egui::Ui, mail: &mut Mail, letter: &Letter, which: LetterBox, view: &View) {
    let unread = which == LetterBox::In && letter.read_at.is_none();
    let other = if which == LetterBox::In { &letter.from } else { &letter.to };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter_at(rect);
    if response.hovered() {
        painter.rect_filled(rect, 6.0, theme::WELL);
    }
    let left = rect.left() + 10.0;
    let envelope = egui::Rect::from_center_size(egui::pos2(left + ENVELOPE.x / 2.0, rect.center().y + 2.0), ENVELOPE);
    theme::envelope(&painter, envelope, unread, DIM);

    let text_left = envelope.right() + 14.0;
    let top = rect.top() + 7.0;
    let name_font = if unread { theme::strong(size::BODY) } else { theme::body(size::BODY) };
    let name_ink = if unread { theme::GOLD_LIGHT } else { TEXT };
    let prefix = if which == LetterBox::Out { "To " } else { "" };
    let mut x = text_left;
    if !prefix.is_empty() {
        let g = painter.text(egui::pos2(x, top), Align2::LEFT_TOP, prefix, theme::body(size::BODY), DIM);
        x = g.right();
    }
    if other.office {
        // The Capitol's seal, small, where a citizen's lozenge would be
        let r = size::BODY * 0.46;
        theme::capitol_seal(&painter, egui::pos2(x + r, top + size::BODY * 0.62), r);
        x += r * 2.0 + 5.0;
    } else if other.citizen {
        // The lozenge, as `theme::citizen_mark` draws it, without taking room
        let r = crate::labels::GLYPH_R * size::BODY;
        let c = egui::pos2(x + r * 0.8, top + size::BODY * 0.62);
        let w = r * 0.72;
        painter.add(egui::Shape::convex_polygon(
            vec![c + egui::vec2(0.0, -r), c + egui::vec2(w, 0.0), c + egui::vec2(0.0, r), c + egui::vec2(-w, 0.0)],
            theme::CITIZEN_LANTERN,
            Stroke::NONE,
        ));
        x += r * 1.6 + 4.0;
    }
    painter.text(egui::pos2(x, top), Align2::LEFT_TOP, &other.name, name_font, name_ink);
    painter.text(
        egui::pos2(rect.right() - 10.0, top + 1.0),
        Align2::RIGHT_TOP,
        mail::when_written(letter.sent_at, view.now),
        theme::body(size::SMALL),
        DIM,
    );
    let gist = talk::first_line(&letter.body);
    let gist = painter.layout_no_wrap(gist, theme::body(size::SMALL), if unread { TEXT.gamma_multiply(0.8) } else { DIM });
    let gist_room = rect.right() - 10.0 - text_left;
    let at = egui::pos2(text_left, top + size::BODY + 6.0);
    if gist.size().x <= gist_room {
        painter.galley(at, gist, DIM);
    } else {
        // Clipped and faded at the right, rather than run under the edge
        let clip = egui::Rect::from_min_size(at, egui::vec2(gist_room, gist.size().y));
        ui.painter_at(clip).galley(at, gist, DIM);
    }
    if unread {
        painter.circle_filled(egui::pos2(rect.left() + 3.0, rect.center().y), 2.5, GOLD);
    }
    if response.clicked() {
        mail.open_letter(letter.id, view.now);
    }
}

/// One letter, on paper, with Reply and Drop under it.
fn read(ui: &mut egui::Ui, mail: &mut Mail, letter: &Letter, view: &View) -> Option<Action> {
    let mut action = None;
    let mine = mail.letters(LetterBox::Out).iter().any(|l| l.id == letter.id);
    let other = if mine { &letter.to } else { &letter.from };
    if theme::link(ui, "← Letters", size::SMALL).clicked() {
        mail.back();
    }
    ui.add_space(6.0);
    let room = (ui.available_height() - 60.0).max(120.0);
    paper().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(if mine { "To" } else { "From" }).font(theme::body(size::SMALL)).color(theme::INK_DIM).italics());
            if other.office {
                // An office has no card: its name is only its name
                ui.label(RichText::new(&other.name).font(theme::display(SALUTATION)).color(theme::INK))
                    .on_hover_text("One of the city's offices, not a person");
            } else {
                if other.citizen {
                    theme::citizen_mark(ui, SALUTATION, Some(theme::INK));
                }
                let name = ui
                    .add(
                        egui::Label::new(RichText::new(&other.name).font(theme::display(SALUTATION)).color(theme::INK))
                            .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Who they are");
                if name.clicked() {
                    action = Some(Action::Card(other.clone()));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(mail::when_written(letter.sent_at, view.now))
                        .font(theme::body(size::SMALL))
                        .color(theme::INK_DIM)
                        .italics(),
                );
            });
        });
        ui.add_space(4.0);
        ink_rule(ui);
        ui.add_space(8.0);
        egui::ScrollArea::vertical().id_salt("capitol_mail_letter").max_height(room - 96.0).show_following(ui, |ui| {
            ui.label(RichText::new(&letter.body).font(theme::body(HAND)).color(theme::INK));
        });
        ui.add_space(6.0);
        // Sealed at its foot with the writer's initial, or an office's
        // with the Capitol's seal
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), Sense::hover());
            if letter.from.office {
                theme::capitol_seal(ui.painter(), rect.center(), 13.0);
            } else {
                theme::wax_seal(ui.painter(), rect.center(), 13.0);
                let initial: String = letter.from.name.chars().take(1).collect::<String>().to_uppercase();
                ui.painter().text(rect.center() + egui::vec2(0.0, 0.5), Align2::CENTER_CENTER, initial, theme::display(12.0), theme::PAPER);
            }
        });
    });
    ui.add_space(10.0);
    ui.columns(2, |c| {
        if other.office {
            // Nobody to answer: the office's seal is its last word
            c[0].vertical_centered(|ui| {
                ui.add_space(10.0);
                ui.label(theme::dim("The office takes no replies").italics());
            });
        } else {
            let again = if mine { "Write again" } else { "Reply" };
            if theme::accent_button(&mut c[0], again, GOLD, view.online).clicked() {
                mail.reply(letter.id);
            }
        }
        let drop = c[1]
            .vertical_centered_justified(|ui| {
                ui.add_sized([ui.available_width(), 38.0], egui::Button::new(RichText::new("Drop").color(DIM)))
            })
            .inner
            .on_hover_text("Throw your copy away. Theirs is theirs to keep.");
        if drop.clicked() && view.online {
            mail.drop_letter(letter.id);
        }
    });
    action
}

/// The letter you're writing: the same paper, and a pen.
fn write(ui: &mut egui::Ui, mail: &mut Mail, view: &View) -> Option<Action> {
    let mut action = None;
    let Some(draft) = mail.draft().cloned() else {
        mail.back();
        return None;
    };
    if theme::link(ui, "← Letters", size::SMALL).on_hover_text("Your letter is kept").clicked() {
        mail.back();
    }
    ui.add_space(6.0);
    let room = (ui.available_height() - 90.0).max(110.0);
    let mut body = draft.body.clone();
    paper().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("To").font(theme::body(size::SMALL)).color(theme::INK_DIM).italics());
            if draft.to.citizen {
                theme::citizen_mark(ui, SALUTATION, Some(theme::INK));
            }
            let name = ui
                .add(
                    egui::Label::new(RichText::new(&draft.to.name).font(theme::display(SALUTATION)).color(theme::INK))
                        .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if name.clicked() {
                action = Some(Action::Card(draft.to.clone()));
            }
        });
        ui.add_space(4.0);
        ink_rule(ui);
        ui.add_space(6.0);
        ui.scope(|ui| {
            let v = ui.visuals_mut();
            v.extreme_bg_color = Color32::TRANSPARENT;
            v.text_cursor.stroke = Stroke::new(1.5_f32, theme::INK);
            v.selection.bg_fill = theme::PAPER_EDGE;
            v.selection.stroke = Stroke::new(1.0_f32, theme::INK);
            egui::ScrollArea::vertical().id_salt("capitol_mail_pen").max_height(room).show_following(ui, |ui| {
                let hint = format!("Dear {},", draft.to.name);
                let pen = egui::TextEdit::multiline(&mut body)
                    .id(Id::new("capitol_mail_pen_field"))
                    .frame(false)
                    .font(theme::body(HAND))
                    .text_color(theme::INK)
                    .hint_text(RichText::new(hint).color(theme::INK_DIM.gamma_multiply(0.7)).italics())
                    .desired_rows(7)
                    .desired_width(f32::INFINITY)
                    .char_limit(talk::MAX_LETTER)
                    .show(ui);
                // The pen is in hand as a fresh sheet comes up
                if body.is_empty() && ui.memory(|m| m.focused().is_none()) {
                    pen.response.request_focus();
                }
            });
        });
    });
    if body != draft.body {
        if let Some(d) = mail.draft_mut() {
            d.body = body.clone();
        }
    }
    ui.add_space(6.0);
    let used = mail::written(&body);
    let stop = mail::stopped(&capitol_core::mail::Draft { to: draft.to.clone(), body: body.clone() }, view.strands);
    let sending = mail.sending();
    ui.horizontal(|ui| {
        let left = talk::MAX_LETTER - used.min(talk::MAX_LETTER);
        let ink = if left < 50 { theme::EMBER } else { DIM };
        ui.label(RichText::new(format!("{left} characters left")).size(size::SMALL).color(ink));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(format!("Postage {} Strand", talk::POSTAGE)).size(size::SMALL).color(GOLD));
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
            theme::wax_seal(ui.painter(), rect.center(), 5.5);
        });
    });
    // What stops it, once there's something written; or the server's no
    let why = mail.refused().map(str::to_owned).or_else(|| stop.clone().filter(|_| !body.trim().is_empty()));
    if let Some(why) = why {
        ui.add_space(4.0);
        ui.label(RichText::new(why).size(size::SMALL).color(theme::EMBER));
    }
    ui.add_space(8.0);
    ui.columns(2, |c| {
        let label = if sending { "Sending…" } else { "Send" };
        let can = view.online && stop.is_none() && !sending;
        if theme::accent_button(&mut c[0], label, GOLD, can).clicked() && can {
            mail.send(view.strands);
        }
        let discard = c[1]
            .vertical_centered_justified(|ui| {
                ui.add_sized([ui.available_width(), 38.0], egui::Button::new(RichText::new("Discard").color(DIM)))
            })
            .inner;
        if discard.clicked() {
            mail.discard();
        }
    });
    action
}

/// The sheet a letter is written on: the Gazette's paper, smaller.
fn paper() -> egui::Frame {
    egui::Frame::none()
        .fill(theme::PAPER)
        .stroke(Stroke::new(1.0_f32, theme::PAPER_EDGE))
        .rounding(2.0)
        .inner_margin(Margin::symmetric(16.0, 12.0))
        .shadow(egui::epaint::Shadow { offset: [0.0, 3.0].into(), blur: 12.0, spread: 0.0, color: Color32::from_black_alpha(110) })
}

/// A hairline of ink across the sheet.
fn ink_rule(ui: &mut egui::Ui) {
    theme::rule_in(ui, theme::INK_RULE.gamma_multiply(0.6));
}

/// The Registrar's letter, as the server writes it, for the sample.
const REGISTRAR_SAMPLE: &str = "Welcome to the Capitol.\n\n\
    The Registry has noted your road in: things made with your own hands, the deeds of a farm, a house \
    and a yard, and trade at the hall.\n\n\
    Your deed is drawn up. Say a word to whoever is near and read a neighbour's card (a pale lozenge marks \
    a citizen, who lives and works here as you do), then come to the office by the hall and choose where \
    you'll live. The land is granted; its tax is yours, as anyone's is.\n\n\
    The Registrar";

/// `--mail-sample [in|letter|write|sent|registrar|registrar-in]`: the mailbox offline, from the
/// protocol's fixture box (`mailbox.json`), on that view.
pub fn sample(which: &str) -> (Mail, View) {
    use capitol_core::mail::Ask;
    use capitol_core::protocol::rest::{LettersQuery, Mailbox};
    let mut inbox: Mailbox = capitol_core::serde_json::from_str(include_str!(
        "../../../../../protocol/tests/fixtures/rest/mailbox.json"
    ))
    .expect("the mailbox fixture parses");
    let now = inbox.letters[0].sent_at + capitol_core::chrono::Duration::minutes(40);
    // A few more, so the list has a read one or two below
    let more = [
        ("Odile Fenn", false, "The kiln on Sorrel Lane takes clay at a fair rate if you ask.", 26),
        ("Citizen Wren", true, "Thank you for the stone. It went into the east wall.", 31),
    ];
    for (n, (name, citizen, body, hours)) in more.into_iter().enumerate() {
        let mut letter = inbox.letters[1].clone();
        letter.id = uuid::Uuid::from_u128(0x5000 + n as u128);
        letter.from = Correspondent { id: uuid::Uuid::from_u128(0x6000 + n as u128), name: name.into(), citizen, office: false };
        letter.body = body.into();
        letter.sent_at = now - capitol_core::chrono::Duration::hours(hours);
        letter.read_at = Some(letter.sent_at + capitol_core::chrono::Duration::hours(1));
        inbox.letters.push(letter);
    }
    inbox.more = false;
    let me = inbox.letters[0].to.clone();
    if which.starts_with("registrar") {
        // As `engine::letters::OfficeLetter::RegistrarWelcome` writes it
        let mut letter = inbox.letters[0].clone();
        letter.id = uuid::Uuid::from_u128(0x447);
        letter.from = Correspondent { id: uuid::Uuid::nil(), name: "The Registrar".into(), citizen: false, office: true };
        letter.body = REGISTRAR_SAMPLE.into();
        letter.sent_at = now - capitol_core::chrono::Duration::minutes(3);
        letter.read_at = None;
        inbox.letters.insert(0, letter);
    }
    let brin = inbox.letters[0].from.clone();
    let mut mail = Mail::default();
    mail.connected();
    let ask = mail.wants(false).expect("the in box");
    mail.sent(ask);
    mail.paged(inbox.clone());
    // The out box: one letter to Brin
    mail.tab = LetterBox::Out;
    let ask = Ask::Page(LettersQuery { which: LetterBox::Out, before: None });
    mail.sent(ask);
    let mut sent = inbox.letters[1].clone();
    sent.id = uuid::Uuid::from_u128(0x7000);
    sent.from = me;
    sent.to = brin.clone();
    sent.body = "Dear Brin,\n\nTwenty at grade B, at the hall tomorrow morning. Bring the cart.".into();
    sent.sent_at = now - capitol_core::chrono::Duration::hours(3);
    sent.read_at = None;
    mail.paged(Mailbox { letters: vec![sent], unread: 1, more: false });
    mail.tab = LetterBox::In;
    match which {
        "letter" | "registrar" => mail.open_letter(inbox.letters[0].id, now),
        "write" => {
            mail.write_to(brin);
            if let Some(d) = mail.draft_mut() {
                d.body = "Dear Brin,\n\nTwenty at grade B it is. I'll be at the hall when the bell goes.".into();
            }
        }
        "sent" => mail.tab = LetterBox::Out,
        _ => {}
    }
    // Drawn as it would be signed in; offline nothing is sent
    (mail, View { strands: Some(42), now, online: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a few frames of the mailbox at `width`, clicking at `click_at`
    /// on the fifth, and returns any action and where it was drawn.
    fn run(mail: &mut Mail, view: &View, width: f32, click_at: Option<egui::Pos2>) -> (Option<Action>, Option<egui::Rect>) {
        let ctx = theme::test_context();
        let mut panel = MailPanel { open: true, ..Default::default() };
        let mut out = (None, None);
        for frame in 0..8 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 480.0))),
                ..Default::default()
            };
            if let (Some(p), 5) = (click_at, frame) {
                raw.events.push(egui::Event::PointerMoved(p));
                for pressed in [true, false] {
                    raw.events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    });
                }
            }
            let _ = ctx.run(raw, |ctx| {
                if let Some(a) = panel.show(ctx, mail, view) {
                    out.0 = Some(a);
                }
                out.1 = panel.rect;
            });
        }
        out
    }

    #[test]
    fn every_view_draws_inside_a_small_screen() {
        for which in ["in", "letter", "write", "sent", "registrar"] {
            let (mut mail, view) = sample(which);
            let (_, rect) = run(&mut mail, &view, 853.0, None);
            let rect = rect.expect("drawn");
            assert!(rect.left() >= 16.0 && rect.right() <= 853.0 - 16.0, "{which}: {rect:?}");
            assert!(rect.bottom() <= 480.0, "{which}: {rect:?} runs off the bottom");
        }
    }

    #[test]
    fn a_click_on_an_envelope_opens_the_letter_and_reads_it() {
        let (mut mail, view) = sample("in");
        assert_eq!(mail.unread(), 1);
        let (_, rect) = run(&mut mail, &view, 1280.0, None);
        let rect = rect.unwrap();
        // The first row: under the title, the rule and the tabs
        let at = egui::pos2(rect.center().x, rect.top() + 12.0 + 30.0 + 8.0 + 30.0 + 8.0 + ROW / 2.0);
        run(&mut mail, &view, 1280.0, Some(at));
        assert!(matches!(mail.showing(), Showing::Letter(_)), "{:?}", mail.showing());
        assert_eq!(mail.unread(), 0);
    }

    #[test]
    fn the_name_on_a_letter_opens_their_card() {
        let (mut mail, view) = sample("letter");
        let (_, rect) = run(&mut mail, &view, 1280.0, None);
        let rect = rect.unwrap();
        // "From", the lozenge, and then the name, on the sheet's first line
        let at = egui::pos2(rect.left() + 99.0, rect.top() + 114.0);
        let (action, _) = run(&mut mail, &view, 1280.0, Some(at));
        assert!(matches!(&action, Some(Action::Card(c)) if c.name == "Brin"), "{action:?}");
    }

    /// The Registrar's letter (#447): its name is only a name, and there's
    /// no Reply to press
    #[test]
    fn an_offices_letter_opens_no_card_and_takes_no_reply() {
        let (mut mail, view) = sample("registrar");
        assert!(matches!(mail.showing(), Showing::Letter(id) if mail.letter(id).is_some_and(|l| l.from.office)));
        let (_, rect) = run(&mut mail, &view, 1280.0, None);
        let rect = rect.unwrap();
        let (action, _) = run(&mut mail, &view, 1280.0, Some(egui::pos2(rect.left() + 99.0, rect.top() + 114.0)));
        assert_eq!(action, None);
        // Where Reply would be
        let at = egui::pos2(rect.left() + rect.width() * 0.25, rect.bottom() - 30.0);
        let (action, _) = run(&mut mail, &view, 1280.0, Some(at));
        assert_eq!(action, None);
        assert!(mail.draft().is_none(), "nothing to write");
    }

    #[test]
    fn the_close_button_closes_it() {
        let (mut mail, view) = sample("in");
        let (_, rect) = run(&mut mail, &view, 1280.0, None);
        let rect = rect.unwrap();
        let (action, _) = run(&mut mail, &view, 1280.0, Some(egui::pos2(rect.right() - 20.0, rect.top() + 30.0)));
        assert_eq!(action, Some(Action::Close));
    }
}
