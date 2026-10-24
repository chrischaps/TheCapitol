//! The guild panel (Commons S4.4): somewhere to find a guild, found one,
//! and run one. U opens it, top centre under the header, edged in the
//! guilds' verdigris (`theme::GUILD`), the green of an old hall's copper
//! sign.
//!
//! Three pages:
//! - **Your guild**, once you're in one: its banner (the name, the tag as it
//!   hangs after a member's name, who founded it and who leads it), the
//!   charter on a sheet of the Gazette's paper, and the roster by role. An
//!   officer or the leader invites by name and answers who's waiting; the
//!   leader sets roles and hands on the lead; anyone may leave.
//! - **Browse**: invitations to you, then every guild, the largest first. A
//!   guild opens on its charter and roster, with *Apply*.
//! - **Found**, outside a guild: the form, the tag previewed after your
//!   name, and the 500. Founding is at the Exchange hall and needs a
//!   subscription (GDD §5); away from it, a button walks you there.
//!
//! The rules are `capitol_core::guilds`', and the panel works them
//! directly: what's offered to whom is `roster_acts` and `waiting_acts`,
//! the server's own rights. Only what reaches past the panel comes back as
//! an [`Action`]: a name that opens a card, the walk to the hall, the ×.

use capitol_core::chrono::{DateTime, Utc};
use capitol_core::guilds::{self, Ask, Draft, Guilds, RosterAct, WaitAct};
use capitol_core::protocol::rest::{GuildDetail, GuildInvite, GuildInviteKind, GuildMember, GuildSummary};
use capitol_core::protocol::rules::guild::{self as rules, GuildRole};
use egui::{Align, Align2, Color32, Id, Layout, Margin, RichText, Sense, Stroke};
use uuid::Uuid;

use super::theme::{self, size, DIM, GOLD, GUILD, TEXT};

/// The open and close fade, in seconds, and how far it rises as it comes.
const FADE: f32 = 0.2;
const RISE: f32 = 12.0;
/// The panel's widest.
const WIDTH: f32 = 520.0;
/// The banner's name.
const BANNER: f32 = 22.0;

/// What the panel asks of the rest of the game.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// See who someone is: their card (S0.3)
    Card { id: Uuid, name: String, citizen: bool },
    /// Walk to the Exchange hall, to found a guild there
    ToHall,
    Close,
}

/// What the panel draws from, beside the guilds themselves.
pub struct View {
    pub me: Option<Uuid>,
    /// Your name, for the tag's preview
    pub my_name: String,
    /// Your Strands, for the founding fee. `None` until the server says.
    pub strands: Option<i64>,
    /// Your subscription is running: founding needs one
    pub subscribed: bool,
    /// You stand at the Exchange hall, where guilds are founded
    pub at_hall: bool,
    /// Signed in. Offline it's a sample, and nothing is sent.
    pub online: bool,
    /// The server's clock, for how long someone's been in
    pub now: DateTime<Utc>,
}

/// Which page is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Yours,
    Browse,
    Found,
}

/// The panel.
#[derive(Default)]
pub struct GuildPanel {
    pub open: bool,
    was_open: bool,
    /// Where it was drawn this frame, for the hints to keep clear of.
    pub rect: Option<egui::Rect>,
    page: Page,
    /// The founding form, as typed
    pub draft: Draft,
    /// The name to invite, as typed
    pub invite: String,
    /// *Leave* was pressed once: it asks again
    leaving: bool,
}

impl GuildPanel {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.leaving = false;
    }

    /// Open it on `page`.
    pub fn open_on(&mut self, page: Page) {
        self.open = true;
        self.page = page;
    }

    pub fn show(&mut self, ctx: &egui::Context, guilds: &mut Guilds, view: &View) -> Option<Action> {
        let shown = ctx.animate_bool_with_time(Id::new("capitol_guild_fade"), self.open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            self.was_open = false;
            return None;
        }
        // In a guild there's no founding one, and out of one, no yours
        let in_guild = guilds.guild().is_some();
        self.page = match (self.page, in_guild) {
            (Page::Found, true) => Page::Yours,
            (Page::Yours, false) => Page::Browse,
            (page, _) => page,
        };
        if self.open && !self.was_open {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, Id::new("capitol_guild")));
            if self.page == Page::Browse {
                guilds.browse();
            }
        }
        self.was_open = self.open;
        let ease = 1.0 - (1.0 - shown).powi(3);
        let screen = ctx.available_rect();
        let width = WIDTH.min(screen.width() - 32.0).max(280.0);
        let room = (screen.height() - 24.0 - 24.0).max(200.0);
        let mut action = None;
        let area = egui::Area::new(Id::new("capitol_guild"))
            .anchor(Align2::CENTER_TOP, [0.0, screen.top() + 12.0 - RISE * (1.0 - ease)])
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(GUILD).show(ui, |ui| {
                    ui.set_width(width - 24.0);
                    ui.set_max_height(room);
                    action = self.body(ui, guilds, view);
                });
            });
        self.rect = self.open.then_some(area.response.rect);
        if action == Some(Action::Close) {
            self.open = false;
            self.leaving = false;
        }
        action
    }

    fn body(&mut self, ui: &mut egui::Ui, guilds: &mut Guilds, view: &View) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(theme::title("Guilds", size::TITLE * 0.7).color(GUILD));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let close = ui.add(
                    egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM))
                        .frame(false)
                        .min_size(egui::vec2(22.0, 22.0)),
                );
                if close.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Close (U)").clicked() {
                    action = Some(Action::Close);
                }
            });
        });
        ui.add_space(2.0);
        theme::rule_in(ui, GUILD.gamma_multiply(0.45));
        ui.add_space(8.0);
        self.tabs(ui, guilds);
        ui.add_space(8.0);
        if let Some(words) = guilds.refused() {
            theme::error_box(ui, words);
            ui.add_space(6.0);
        } else if let Some(words) = guilds.done() {
            ui.label(RichText::new(words).size(size::BODY).color(GUILD));
            ui.add_space(6.0);
        }
        let shown = match self.page {
            Page::Yours => self.yours(ui, guilds, view),
            Page::Browse => browse(ui, guilds, view),
            Page::Found => self.found(ui, guilds, view),
        };
        action.or(shown)
    }

    /// Your guild, Browse and, outside one, Found.
    fn tabs(&mut self, ui: &mut egui::Ui, guilds: &mut Guilds) {
        let invited = match guilds.invitations() {
            0 => "Browse".to_string(),
            n => format!("Browse · {n}"),
        };
        let mut page = self.page;
        ui.horizontal(|ui| match guilds.guild() {
            Some(_) => {
                theme::segmented(ui, &mut page, &[(Page::Yours, "Your guild"), (Page::Browse, invited.as_str())]);
            }
            None => {
                theme::segmented(ui, &mut page, &[(Page::Browse, invited.as_str()), (Page::Found, "Found one")]);
            }
        });
        if page != self.page {
            if page == Page::Browse {
                guilds.browse();
            }
            self.page = page;
            self.leaving = false;
        }
    }

    /// Your guild: its banner, its charter, its roster, who's waiting, and
    /// the way out.
    fn yours(&mut self, ui: &mut egui::Ui, guilds: &mut Guilds, view: &View) -> Option<Action> {
        let Some(guild) = guilds.guild().cloned() else {
            ui.label(theme::dim(if view.online { "Reading your guild…" } else { "In no guild." }).italics());
            return None;
        };
        let role = guilds.role();
        let me = view.me.unwrap_or_default();
        let busy = guilds.busy();
        let mut action = None;
        let room = (ui.available_height() - 4.0).max(160.0);
        theme::scrolled(ui, "capitol_guild_yours", room, |ui| {
            banner(ui, &guild.name, &guild.tag, &guild.founder.name, &guild.leader.name, guild.founded_at, guild.members.len());
            charter(ui, &guild.charter);
            ui.add_space(10.0);
            ui.label(theme::label("Roster"));
            ui.add_space(4.0);
            for member in &guild.members {
                if let Some(a) = roster_row(ui, guilds, &guild, member, role, me, view, busy) {
                    action = Some(a);
                }
            }
            if role.is_some_and(GuildRole::may_invite) {
                ui.add_space(10.0);
                ui.label(theme::label("Invite"));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let field = egui::TextEdit::singleline(&mut self.invite)
                        .id(Id::new("capitol_guild_invite"))
                        .hint_text("Their name")
                        .desired_width(ui.available_width() - 90.0)
                        .margin(Margin::symmetric(8.0, 5.0));
                    let typed = ui.add(field);
                    let name = self.invite.trim().to_string();
                    let ready = !name.is_empty() && !busy && view.online;
                    let pressed = ui.add_enabled(ready, egui::Button::new(RichText::new("Invite").color(GUILD))).clicked();
                    let entered = typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ready && (pressed || entered) {
                        guilds.queue(Ask::Find { guild_id: guild.id, name });
                        self.invite.clear();
                    }
                });
                ui.label(theme::dim(format!(
                    "Whoever completes a join pays its {} Strand: you, if they asked first.",
                    rules::ROSTER_FEE
                )));
                if !guild.waiting.is_empty() {
                    ui.add_space(10.0);
                    ui.label(theme::label("Waiting"));
                    ui.add_space(4.0);
                    for invite in &guild.waiting {
                        if let Some(a) = waiting_row(ui, guilds, invite, role, me, busy, false) {
                            action = Some(a);
                        }
                    }
                }
            }
            ui.add_space(14.0);
            theme::rule_in(ui, GUILD.gamma_multiply(0.25));
            ui.add_space(8.0);
            self.leave(ui, guilds, &guild, role, busy);
        });
        action
    }

    /// The way out: asked twice. A leader with others leaves last.
    fn leave(&mut self, ui: &mut egui::Ui, guilds: &mut Guilds, guild: &GuildDetail, role: Option<GuildRole>, busy: bool) {
        let alone = guild.members.len() == 1;
        if role == Some(GuildRole::Leader) && !alone {
            ui.label(theme::dim("A leader leaves last. Hand on the lead first, and you can go as an officer."));
            return;
        }
        if !self.leaving {
            let words = if alone { "Leave, and disband the guild" } else { "Leave the guild" };
            if theme::link(ui, words, size::SMALL).clicked() {
                self.leaving = true;
            }
            return;
        }
        let question = if alone {
            format!("You're the last of {}. Leaving disbands it, and frees its name and tag.", guild.name)
        } else {
            format!("Leave {}? Leaving is free.", guild.name)
        };
        ui.label(RichText::new(question).size(size::BODY).color(TEXT));
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new(RichText::new("Leave").color(theme::ERROR))).clicked() {
                guilds.queue(Ask::Leave(guild.id));
                self.leaving = false;
            }
            if theme::button(ui, "Stay", false).clicked() {
                self.leaving = false;
            }
        });
    }

    /// The founding form: the name, the tag after your name, the charter,
    /// and the fee.
    fn found(&mut self, ui: &mut egui::Ui, guilds: &mut Guilds, view: &View) -> Option<Action> {
        let mut action = None;
        ui.label(theme::dim(
            "A guild is a name over a hall and a shared purse. Its tag hangs after every member's name.",
        ));
        ui.add_space(8.0);
        ui.label(theme::label("Name"));
        ui.add(
            egui::TextEdit::singleline(&mut self.draft.name)
                .id(Id::new("capitol_guild_name"))
                .hint_text("The Rope Walkers")
                .char_limit(rules::NAME_MAX)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(8.0, 5.0)),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(theme::label("Tag"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.tag)
                        .id(Id::new("capitol_guild_tag"))
                        .hint_text("ROPE")
                        .char_limit(rules::TAG_MAX)
                        .desired_width(80.0)
                        .margin(Margin::symmetric(8.0, 5.0)),
                );
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(theme::label("Over your head"));
                let tag = self.draft.tag.trim().to_uppercase();
                preview(ui, &view.my_name, (!tag.is_empty()).then_some(tag.as_str()));
            });
        });
        ui.add_space(6.0);
        ui.label(theme::label("Charter"));
        ui.add(
            egui::TextEdit::multiline(&mut self.draft.charter)
                .id(Id::new("capitol_guild_charter"))
                .hint_text("What the guild is for, and what it asks of its members")
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(8.0, 5.0)),
        );
        ui.add_space(8.0);
        let checked = self.draft.check();
        let affordable = view.strands.is_some_and(Draft::affordable);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} Strands", rules::FOUNDING_FEE)).font(theme::strong(size::BODY)).color(GOLD));
            ui.label(theme::dim("to the Guild Registry, and a Strand for each change to the roster after"));
        });
        ui.add_space(8.0);
        let why_not = if !view.subscribed {
            Some("Founding a guild needs a subscription. Anyone may join one.".to_string())
        } else if let Err(why) = &checked {
            (!self.draft.name.trim().is_empty() || !self.draft.tag.trim().is_empty()).then(|| why.clone())
        } else if !affordable {
            Some(format!("You need {} Strands to found a guild.", rules::FOUNDING_FEE))
        } else {
            None
        };
        if let Some(why) = &why_not {
            ui.label(RichText::new(why).size(size::SMALL).color(theme::SHORT));
            ui.add_space(6.0);
        }
        let ready = why_not.is_none() && checked.is_ok() && !guilds.busy() && view.online;
        if view.at_hall || !view.subscribed {
            let words = match &checked {
                Ok(f) => format!("Found {}", f.name),
                Err(_) => "Found the guild".into(),
            };
            if theme::accent_button(ui, &words, GUILD, ready).clicked() {
                if let Ok(found) = checked {
                    guilds.queue(Ask::Found(found));
                }
            }
        } else {
            ui.label(theme::dim("Guilds are founded at the Exchange hall, in the Capitol."));
            ui.add_space(4.0);
            if theme::accent_button(ui, "Walk to the Exchange hall", GUILD, view.online).clicked() {
                action = Some(Action::ToHall);
            }
        }
        action
    }
}

/// The registry, and the invitations to you over it; or one guild, opened.
fn browse(ui: &mut egui::Ui, guilds: &mut Guilds, view: &View) -> Option<Action> {
    let me = view.me.unwrap_or_default();
    let busy = guilds.busy();
    if let Some(guild) = guilds.viewing().cloned() {
        return opened(ui, guilds, &guild, view);
    }
    let mut action = None;
    let invites: Vec<GuildInvite> = guilds.mine().map(|m| m.invites.clone()).unwrap_or_default();
    let room = (ui.available_height() - 4.0).max(160.0);
    theme::scrolled(ui, "capitol_guild_browse", room, |ui| {
        if !invites.is_empty() {
            ui.label(theme::label("Waiting on you"));
            ui.add_space(4.0);
            for invite in &invites {
                if let Some(a) = waiting_row(ui, guilds, invite, None, me, busy, true) {
                    action = Some(a);
                }
            }
            ui.add_space(10.0);
        }
        ui.label(theme::label("The registry"));
        ui.add_space(4.0);
        let list: Vec<GuildSummary> = guilds.registry().map(<[_]>::to_vec).unwrap_or_default();
        if guilds.registry().is_none() {
            ui.label(theme::dim(if view.online { "Reading the registry…" } else { "The registry is closed offline." }).italics());
        } else if list.is_empty() {
            ui.label(theme::dim("No guild stands yet. The first is founded at the Exchange hall.").italics());
        }
        let mine = guilds.guild_id();
        for g in &list {
            if registry_row(ui, g, mine == Some(g.id)) {
                guilds.open(g.id);
            }
        }
    });
    action
}

/// A guild from the registry: its banner, charter and roster, and *Apply*.
fn opened(ui: &mut egui::Ui, guilds: &mut Guilds, guild: &GuildDetail, view: &View) -> Option<Action> {
    let mut action = None;
    if theme::link(ui, "‹ The registry", size::SMALL).clicked() {
        guilds.close_viewing();
        return None;
    }
    ui.add_space(6.0);
    let room = (ui.available_height() - 4.0).max(160.0);
    theme::scrolled(ui, "capitol_guild_opened", room, |ui| {
        banner(ui, &guild.name, &guild.tag, &guild.founder.name, &guild.leader.name, guild.founded_at, guild.members.len());
        charter(ui, &guild.charter);
        ui.add_space(10.0);
        let busy = guilds.busy();
        let me = view.me.unwrap_or_default();
        match (guilds.guild_id(), guilds.waiting_with(guild.id).cloned()) {
            (Some(id), _) if id == guild.id => {
                ui.label(theme::dim("Your guild."));
            }
            (Some(_), _) => {
                ui.label(theme::dim("You're in a guild already. Leave it first to join another."));
            }
            (None, Some(waiting)) => {
                let words = match waiting.kind {
                    GuildInviteKind::Apply => "You've asked to join. An officer or the leader will answer.",
                    GuildInviteKind::Invite => "They've invited you.",
                };
                ui.label(RichText::new(words).size(size::BODY).color(GUILD));
                ui.horizontal(|ui| {
                    for act in guilds::waiting_acts(None, me, &waiting) {
                        if ui.add_enabled(!busy && view.online, egui::Button::new(act.words(me, &waiting))).clicked() {
                            guilds.queue(act.ask(me, &waiting));
                        }
                    }
                });
            }
            (None, None) => {
                if theme::accent_button(ui, "Ask to join", GUILD, !busy && view.online)
                    .on_hover_text("An officer or the leader answers. Joining costs whoever completes it a Strand.")
                    .clicked()
                {
                    guilds.queue(Ask::Apply(guild.id));
                }
            }
        }
        ui.add_space(10.0);
        ui.label(theme::label("Roster"));
        ui.add_space(4.0);
        for member in &guild.members {
            if let Some(a) = roster_row(ui, guilds, guild, member, None, me, view, true) {
                action = Some(a);
            }
        }
    });
    action
}

/// The guild's name large in its verdigris, its tag beside it, and who
/// founded and leads it, on a faint wash of its colour.
fn banner(ui: &mut egui::Ui, name: &str, tag: &str, founder: &str, leader: &str, founded: DateTime<Utc>, members: usize) {
    egui::Frame::none()
        .fill(GUILD.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0_f32, GUILD.gamma_multiply(0.35)))
        .rounding(6.0)
        .inner_margin(Margin::symmetric(14.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(name).font(theme::display(BANNER)).color(GUILD).extra_letter_spacing(BANNER * 0.08));
                ui.add_space(4.0);
                tag_pill(ui, tag);
            });
            ui.add_space(2.0);
            let founded = founded.format("%e %B %Y").to_string();
            let mut line = format!("Founded {} by {founder}", founded.trim());
            if leader != founder {
                line.push_str(&format!(" · led by {leader}"));
            }
            line.push_str(&format!(" · {}", guilds::members_words(members)));
            ui.label(theme::dim(line));
        });
}

/// A guild's tag as a small verdigris plate.
fn tag_pill(ui: &mut egui::Ui, tag: &str) {
    egui::Frame::none()
        .fill(GUILD.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0_f32, GUILD.gamma_multiply(0.6)))
        .rounding(3.0)
        .inner_margin(Margin::symmetric(6.0, 1.0))
        .show(ui, |ui| {
            ui.label(RichText::new(tag).font(theme::strong(size::SMALL)).color(GUILD).extra_letter_spacing(1.5));
        });
}

/// The charter on a sheet of the Gazette's paper, in its ink.
fn charter(ui: &mut egui::Ui, charter: &str) {
    if charter.trim().is_empty() {
        return;
    }
    ui.add_space(8.0);
    egui::Frame::none()
        .fill(theme::PAPER)
        .stroke(Stroke::new(1.0_f32, theme::PAPER_EDGE))
        .rounding(3.0)
        .inner_margin(Margin::symmetric(14.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("CHARTER").font(theme::strong(size::LABEL)).color(theme::INK_DIM).extra_letter_spacing(1.6));
            ui.add_space(2.0);
            ui.label(RichText::new(charter).size(size::LARGE).color(theme::INK).italics());
        });
}

/// Your name as it would hang over your head, with the tag after it in the
/// dimmer ink, as `hud.rhai` draws it.
fn preview(ui: &mut egui::Ui, name: &str, tag: Option<&str>) {
    egui::Frame::none()
        .fill(Color32::from_rgb(0x2a, 0x33, 0x40))
        .rounding(4.0)
        .inner_margin(Margin::symmetric(10.0, 5.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(RichText::new(name).font(theme::strong(size::BODY)).color(GOLD));
                match tag {
                    Some(tag) => ui.label(RichText::new(guilds::bracketed(tag)).size(size::BODY).color(DIM)),
                    None => ui.label(RichText::new("[TAG]").size(size::BODY).color(DIM.gamma_multiply(0.4))),
                };
            });
        });
}

/// A line of the registry: name, tag, members and leader. True if clicked.
fn registry_row(ui: &mut egui::Ui, g: &GuildSummary, yours: bool) -> bool {
    let response = egui::Frame::none()
        .fill(theme::WELL)
        .stroke(Stroke::new(1.0_f32, if yours { GUILD.gamma_multiply(0.5) } else { theme::BORDER }))
        .rounding(5.0)
        .inner_margin(Margin::symmetric(10.0, 6.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&g.name).font(theme::strong(size::BODY)).color(TEXT));
                tag_pill(ui, &g.tag);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(theme::dim(guilds::members_words(g.members as usize)));
                });
            });
            ui.label(theme::dim(format!("Led by {}", g.leader.name)));
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Its charter and roster");
    ui.add_space(3.0);
    response.clicked()
}

/// A member: their role, their name (a link to their card), what's said of
/// them, and what you may do to them.
#[allow(clippy::too_many_arguments)]
fn roster_row(
    ui: &mut egui::Ui,
    guilds: &mut Guilds,
    guild: &GuildDetail,
    member: &GuildMember,
    role: Option<GuildRole>,
    me: Uuid,
    view: &View,
    busy: bool,
) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (word, ink) = match member.role {
            GuildRole::Leader => ("Leader", GOLD),
            GuildRole::Officer => ("Officer", GUILD),
            GuildRole::Member => ("Member", DIM),
        };
        ui.add_sized([58.0, 18.0], egui::Label::new(RichText::new(word).font(theme::strong(size::LABEL)).color(ink)));
        if let Some(a) = name_link(ui, member.id, &member.name, member.citizen, member.id == me, view.online) {
            action = Some(a);
        }
        let days = (view.now - member.joined_at).num_days();
        let since = match days {
            0 => "joined today".to_string(),
            1 => "a day in".to_string(),
            n => format!("{n} days in"),
        };
        ui.label(theme::dim(format!("{} · {since}", member.standing.to_lowercase())));
        let acts = guilds::roster_acts(role, me, member);
        if !acts.is_empty() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Expel last, so it stands apart at the right
                for act in acts.iter().rev() {
                    let ink = if *act == RosterAct::Expel { theme::ERROR } else { GUILD };
                    let link = ui.add_enabled(
                        !busy && view.online,
                        egui::Label::new(RichText::new(act.words()).size(size::SMALL).color(ink).underline()).sense(Sense::click()),
                    );
                    if link.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(act.hover()).clicked() {
                        guilds.queue(act.ask(guild.id, member.id));
                    }
                }
            });
        }
    });
    action
}

/// An invitation or an application, with what you may do about it.
/// `guild` names which guild it is, for the list of those waiting on you.
fn waiting_row(
    ui: &mut egui::Ui,
    guilds: &mut Guilds,
    invite: &GuildInvite,
    role: Option<GuildRole>,
    me: Uuid,
    busy: bool,
    guild: bool,
) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let words = match (invite.kind, invite.player.id == me) {
            (GuildInviteKind::Invite, true) => format!("{} invited you", invite.by.name),
            (GuildInviteKind::Apply, true) => "You asked to join".to_string(),
            (GuildInviteKind::Apply, false) => format!("{} asked to join", invite.player.name),
            (GuildInviteKind::Invite, false) => format!("{} invited {}", invite.by.name, invite.player.name),
        };
        if guild {
            ui.label(RichText::new(&invite.guild).font(theme::strong(size::BODY)).color(TEXT));
            tag_pill(ui, &invite.tag);
        }
        ui.label(RichText::new(words).size(size::SMALL).color(DIM));
        if invite.player.id != me && !guild {
            if let Some(a) = name_link(ui, invite.player.id, "card", invite.player.citizen, false, true) {
                action = Some(a);
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            for act in guilds::waiting_acts(role, me, invite).into_iter().rev() {
                let ink = if act == WaitAct::Accept { GUILD } else { DIM };
                if ui.add_enabled(!busy, egui::Button::new(RichText::new(act.words(me, invite)).color(ink))).clicked() {
                    guilds.queue(act.ask(me, invite));
                }
            }
        });
    });
    action
}

/// A name that opens its person's card, with a citizen's mark. Yours, or
/// offline, it's plain.
fn name_link(ui: &mut egui::Ui, id: Uuid, name: &str, citizen: bool, yours: bool, online: bool) -> Option<Action> {
    if citizen {
        theme::citizen_mark(ui, size::BODY, None);
    }
    let ink = if yours { GOLD } else { TEXT };
    let text = RichText::new(name).font(theme::strong(size::BODY)).color(ink);
    if yours || !online {
        ui.label(text);
        return None;
    }
    let r = ui
        .add(egui::Label::new(text).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Who is {name}?"));
    if r.hovered() {
        ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0_f32, ink.gamma_multiply(0.6)));
    }
    r.clicked().then(|| Action::Card { id, name: name.to_owned(), citizen })
}

/// `--guild-sample [browse|yours|found|officer]`: the panel on a sample.
/// `yours` is the leader's view of a guild of six with two waiting,
/// `officer` the same as an officer, `browse` the registry with an
/// invitation, `found` the form half filled at the hall.
pub fn sample(which: &str) -> (Guilds, View, Page, Draft) {
    use capitol_core::protocol::rest::{Correspondent, MyGuild};
    let at = |days: i64| DateTime::from_timestamp(1_790_000_000, 0).unwrap() + capitol_core::chrono::Duration::days(days);
    let now = at(40);
    let me = Uuid::from_u128(1);
    let who = |n: u128, name: &str, citizen: bool| Correspondent { id: Uuid::from_u128(n), name: name.into(), citizen, office: false };
    let member = |n: u128, name: &str, citizen: bool, role: GuildRole, joined: i64, standing: &str| GuildMember {
        id: Uuid::from_u128(n),
        name: name.into(),
        citizen,
        role,
        joined_at: at(joined),
        standing: standing.into(),
    };
    let rope = Uuid::from_u128(100);
    let my_role = if which == "officer" { GuildRole::Officer } else { GuildRole::Leader };
    let (lead_id, lead_name) = if which == "officer" { (2, "Ada Reeder") } else { (1, "Wren Hollis") };
    let mut members = vec![
        member(1, "Wren Hollis", false, my_role, 0, "Known for good work"),
        member(2, "Ada Reeder", false, if which == "officer" { GuildRole::Leader } else { GuildRole::Officer }, 2, "Steady"),
        member(3, "Cy Marlowe", true, GuildRole::Member, 9, "Pays on time"),
        member(4, "Rook Ashdown", false, GuildRole::Member, 15, "New in town"),
        member(5, "Wen Calder", true, GuildRole::Member, 31, "Keeps their word"),
        member(6, "Bram Tolliver", false, GuildRole::Member, 39, "New in town"),
    ];
    members.sort_by_key(|m| match m.role {
        GuildRole::Leader => 0,
        GuildRole::Officer => 1,
        GuildRole::Member => 2,
    });
    let invite = |kind, player: Correspondent, by: Correspondent, guild: &str, tag: &str, id: Uuid| GuildInvite {
        guild_id: id,
        guild: guild.into(),
        tag: tag.into(),
        kind,
        player,
        by,
        created_at: at(39),
    };
    let rope_detail = GuildDetail {
        id: rope,
        name: "The Rope Walkers".into(),
        tag: "ROPE".into(),
        charter: "We walk the long sheds by the canal and twist the city's rope. Bring fiber, take your share, and never sell a cord you wouldn't hang from.".into(),
        founded_at: at(0),
        founder: who(1, "Wren Hollis", false),
        leader: who(lead_id, lead_name, false),
        members,
        waiting: vec![
            invite(GuildInviteKind::Apply, who(7, "Dot Fenwick", false), who(7, "Dot Fenwick", false), "The Rope Walkers", "ROPE", rope),
            invite(GuildInviteKind::Invite, who(8, "Tam Ivers", true), who(1, "Wren Hollis", false), "The Rope Walkers", "ROPE", rope),
        ],
    };
    let mut guilds = Guilds::default();
    let summary = |n: u128, name: &str, tag: &str, members: u32, leader: Correspondent| GuildSummary {
        id: Uuid::from_u128(n),
        name: name.into(),
        tag: tag.into(),
        members,
        leader,
        founded_at: at(n as i64 % 30),
    };
    let registry = vec![
        summary(101, "Oak & Iron", "OAK", 11, who(20, "Hester Vane", false)),
        summary(100, "The Rope Walkers", "ROPE", 6, who(1, "Wren Hollis", false)),
        summary(102, "The Kiln Yard", "KILN", 4, who(21, "Mabry Stone", true)),
        summary(103, "Riverside Carters", "CART", 2, who(22, "Olwen Pike", false)),
    ];
    let mine = match which {
        "browse" | "found" => MyGuild {
            guild: None,
            role: None,
            invites: vec![invite(
                GuildInviteKind::Invite,
                who(1, "Wren Hollis", false),
                who(20, "Hester Vane", false),
                "Oak & Iron",
                "OAK",
                Uuid::from_u128(101),
            )],
        },
        _ => MyGuild { guild: Some(rope_detail), role: Some(my_role), invites: Vec::new() },
    };
    guilds.answered(mine);
    guilds.listed(registry);
    let page = match which {
        "browse" => Page::Browse,
        "found" => Page::Found,
        _ => Page::Yours,
    };
    let draft = if which == "found" {
        Draft { name: "The Lamplighters".into(), tag: "lamp".into(), charter: "We keep the city's lanterns lit.".into() }
    } else {
        Draft::default()
    };
    let view = View {
        me: Some(me),
        my_name: "Wren Hollis".into(),
        strands: Some(1240),
        subscribed: true,
        at_hall: true,
        online: false,
        now,
    };
    (guilds, view, page, draft)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: egui::Vec2 = egui::vec2(1280.0, 720.0);

    /// Runs a few frames of the panel on `page` with `draft`, clicking at
    /// `click_at` on the sixth, and returns any action, where it was drawn,
    /// and the call the click queued.
    fn run(guilds: &Guilds, view: &View, page: Page, draft: &Draft, click_at: Option<egui::Pos2>) -> (Option<Action>, Option<egui::Rect>, Option<Ask>) {
        let ctx = theme::test_context();
        let mut guilds = guilds.clone();
        let mut panel = GuildPanel { draft: draft.clone(), ..Default::default() };
        panel.open_on(page);
        let (mut action, mut rect) = (None, None);
        for frame in 0..8 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
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
                if let Some(a) = panel.show(ctx, &mut guilds, view) {
                    action = Some(a);
                }
                rect = panel.rect;
            });
        }
        // What the click queued comes ahead of any read
        guilds.connected();
        let asked = guilds.wants().filter(|a| *a != Ask::Mine && *a != Ask::Registry);
        (action, rect, asked)
    }

    /// Every action and call a click somewhere on the panel makes, scanning
    /// it on a grid, so the test holds whatever the spacing.
    fn clicks_on(guilds: &Guilds, view: &View, page: Page, draft: &Draft) -> (Vec<Action>, Vec<Ask>) {
        let (_, rect, _) = run(guilds, view, page, draft, None);
        let rect = rect.expect("drawn");
        let (mut actions, mut asks) = (Vec::new(), Vec::new());
        let mut y = rect.top() + 4.0;
        while y < rect.bottom() {
            let mut x = rect.left() + 20.0;
            while x < rect.right() {
                let (action, _, ask) = run(guilds, view, page, draft, Some(egui::pos2(x, y)));
                if let Some(a) = action.filter(|a| !actions.contains(a)) {
                    actions.push(a);
                }
                if let Some(a) = ask.filter(|a| !asks.contains(a)) {
                    asks.push(a);
                }
                x += 24.0;
            }
            y += 6.0;
        }
        (actions, asks)
    }

    fn online(view: View) -> View {
        View { online: true, ..view }
    }

    const ROPE: Uuid = Uuid::from_u128(100);

    #[test]
    fn the_leader_sets_roles_hands_on_the_lead_and_expels() {
        let (guilds, view, page, draft) = sample("yours");
        let (_, found) = clicks_on(&guilds, &online(view), page, &draft);
        let (ada, cy) = (Uuid::from_u128(2), Uuid::from_u128(3));
        assert!(found.contains(&Ask::SetRole { guild_id: ROPE, player_id: ada, role: GuildRole::Member }), "{found:?}");
        assert!(found.contains(&Ask::SetRole { guild_id: ROPE, player_id: cy, role: GuildRole::Officer }), "{found:?}");
        assert!(found.contains(&Ask::SetRole { guild_id: ROPE, player_id: cy, role: GuildRole::Leader }), "{found:?}");
        assert!(found.contains(&Ask::Expel { guild_id: ROPE, player_id: ada }), "an officer too: {found:?}");
        // Dot applied: take her in, or turn her down. Tam's invitation is withdrawn.
        let dot = Uuid::from_u128(7);
        assert!(found.contains(&Ask::Invite { guild_id: ROPE, player_id: dot }), "{found:?}");
        assert!(found.contains(&Ask::Decline { guild_id: ROPE, player_id: Uuid::from_u128(8) }), "{found:?}");
        assert!(!found.iter().any(|a| matches!(a, Ask::Leave(_))), "a leader with others can't leave: {found:?}");
    }

    #[test]
    fn an_officer_expels_members_and_answers_applications_but_sets_no_roles() {
        let (guilds, view, page, draft) = sample("officer");
        let (_, found) = clicks_on(&guilds, &online(view), page, &draft);
        assert!(found.iter().all(|a| !matches!(a, Ask::SetRole { .. })), "{found:?}");
        assert!(found.contains(&Ask::Expel { guild_id: ROPE, player_id: Uuid::from_u128(3) }), "{found:?}");
        assert!(!found.contains(&Ask::Expel { guild_id: ROPE, player_id: Uuid::from_u128(2) }), "not the leader: {found:?}");
        assert!(found.contains(&Ask::Invite { guild_id: ROPE, player_id: Uuid::from_u128(7) }), "{found:?}");
    }

    #[test]
    fn a_member_only_reads_the_roster_and_names_open_cards() {
        let (mut guilds, view, page, draft) = sample("officer");
        let mut mine = guilds.mine().unwrap().clone();
        mine.role = Some(GuildRole::Member);
        if let Some(g) = mine.guild.as_mut() {
            for m in g.members.iter_mut().filter(|m| m.role == GuildRole::Officer) {
                m.role = GuildRole::Member;
            }
            g.waiting.clear();
        }
        guilds.answered(mine);
        let (actions, found) = clicks_on(&guilds, &online(view), page, &draft);
        // The leave link asks twice, so even it isn't a call
        assert!(found.is_empty(), "{found:?}");
        let cy = Action::Card { id: Uuid::from_u128(3), name: "Cy Marlowe".into(), citizen: true };
        assert!(actions.contains(&cy), "{actions:?}");
    }

    #[test]
    fn browse_answers_an_invitation_and_opens_a_guild() {
        let (guilds, view, page, draft) = sample("browse");
        let (_, found) = clicks_on(&guilds, &online(view), page, &draft);
        let oak = Uuid::from_u128(101);
        assert!(found.contains(&Ask::Apply(oak)), "join the guild that asked: {found:?}");
        assert!(found.contains(&Ask::Decline { guild_id: oak, player_id: Uuid::from_u128(1) }), "{found:?}");
        assert!(found.contains(&Ask::Guild(Uuid::from_u128(102))), "a row opens its guild: {found:?}");
    }

    #[test]
    fn the_founding_form_founds_at_the_hall_and_walks_there_from_away() {
        let (guilds, view, page, draft) = sample("found");
        let view = online(view);
        let (_, found) = clicks_on(&guilds, &view, page, &draft);
        let founded: Vec<_> = found.iter().filter_map(|a| match a { Ask::Found(f) => Some(f), _ => None }).collect();
        assert_eq!(founded.len(), 1, "{found:?}");
        assert_eq!((founded[0].name.as_str(), founded[0].tag.as_str()), ("The Lamplighters", "LAMP"));
        let (actions, found) = clicks_on(&guilds, &View { at_hall: false, ..view }, page, &draft);
        assert!(actions.contains(&Action::ToHall), "{actions:?}");
        assert!(!found.iter().any(|a| matches!(a, Ask::Found(_))), "not away from the hall: {found:?}");
    }

    #[test]
    fn every_sample_draws_inside_the_screen() {
        for which in ["browse", "yours", "found", "officer"] {
            let (guilds, view, page, draft) = sample(which);
            let (_, rect, _) = run(&guilds, &view, page, &draft, None);
            let rect = rect.expect("drawn");
            assert!(rect.bottom() <= SCREEN.y && rect.width() <= WIDTH + 1.0, "{which}: {rect:?}");
        }
    }
}
