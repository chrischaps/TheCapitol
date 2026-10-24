//! The notice board's Standings page (Commons S3.2, plan D9): the week's
//! leaderboards, as an honours board hung on the cork.
//!
//! Halls keep their names on oak in gold leaf, and so does this one: a dark
//! panel, the board's title in the inscriptional capitals, then the names
//! from the top, the first three larger, each with a dotted leader out to
//! its value. Your own line is washed in gold, and if you're past the top
//! 20 it's set under the rest after a break, so you always know where you
//! stand.
//!
//! Over it, paper tags turn between the four boards (Wealth, Makers,
//! Traders, Kept their word), the whole city and each home zone, and
//! *People only*, which leaves citizens out and counts the ranks again.
//! The arrows turn the weeks. A citizen carries their mark, here in gold;
//! someone who keeps out of the papers is "a resident", with no card to
//! open.
//!
//! The page reads and steers the session's
//! `capitol_core::standings::Standings`; only a click on a name reaches past
//! it, as an [`Action`].

use capitol_core::protocol::rest::{LeaderboardEntry, LeaderboardKind};
use capitol_core::standings::{self as st, Standings};
use egui::{Align, Color32, Layout, Margin, RichText, Sense, Stroke};
use uuid::Uuid;

use super::board::{on_cork, tag};
use super::hud::grouped;
use super::theme::{self, size};

/// How many of the top are set larger.
const PODIUM: i64 = 3;
/// The honours board's name sizes: the podium, and everyone else.
const PODIUM_TEXT: f32 = 15.5;
const LINE_TEXT: f32 = 13.5;
/// The rank's column, wide enough for "20."
const RANK_WIDTH: f32 = 34.0;
/// The board's title, in the inscriptional capitals.
const TITLE_TEXT: f32 = 20.0;
/// Room the tags over the board take, and the board's title, foot and
/// margins round its names.
const TAGS: f32 = 84.0;
const TITLE_AND_FOOT: f32 = 112.0;
/// Less room for the names than this, and the board drops the line under
/// its title.
const SHORT_OF_ROOM: f32 = 160.0;

/// What the page asks of the rest of the game.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// See who someone is: their card (S0.3)
    Card(Uuid),
}

/// The Standings page, under the board's tabs.
pub fn page(ui: &mut egui::Ui, standings: &mut Standings, room: f32) -> Option<Action> {
    let mut action = None;
    boards(ui, standings);
    ui.add_space(6.0);
    scope(ui, standings);
    ui.add_space(10.0);
    // The two rows of tags over the board, and its title and foot round the names
    honours(ui, standings, (room - TAGS - TITLE_AND_FOOT).max(LINE_TEXT * 4.0), &mut action);
    action
}

/// The four boards, as paper tags.
fn boards(ui: &mut egui::Ui, standings: &mut Standings) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for board in LeaderboardKind::ALL {
            if tag(ui, board.title(), standings.board == board).clicked() {
                standings.board = board;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            weeks(ui, standings);
        });
    });
}

/// The whole city or a home zone, and people only.
fn scope(ui: &mut egui::Ui, standings: &mut Standings) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        if tag(ui, "Whole city", standings.zone().is_none()).clicked() {
            standings.set_zone(None);
        }
        let mut zones: Vec<String> = standings.zones().to_vec();
        // The zone asked for stays, even in a week nobody lived there
        if let Some(z) = standings.zone().filter(|z| !zones.iter().any(|x| x == z)) {
            zones.push(z.to_owned());
        }
        for zone in zones {
            let on = standings.zone() == Some(zone.as_str());
            if tag(ui, &st::zone_name(&zone), on).on_hover_text("Those whose oldest plot is here").clicked() {
                standings.set_zone(Some(zone));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let people = standings.people();
            let label = if people { "☑ People only" } else { "☐ People only" };
            if tag(ui, label, people).on_hover_text("Leave citizens out, and count the ranks again").clicked() {
                standings.set_people(!people);
            }
        });
    });
}

/// "‹ Week 3 ›": turn back and on, never past the latest.
fn weeks(ui: &mut egui::Ui, standings: &mut Standings) {
    let (Some(week), Some(latest)) = (standings.week(), standings.latest()) else {
        return;
    };
    let arrow = |ui: &mut egui::Ui, text: &str, enabled: bool, hint: &str| {
        let colour = if enabled { theme::PAPER } else { theme::PAPER.gamma_multiply(0.35) };
        let label = egui::Label::new(RichText::new(text).font(theme::strong(size::LARGE)).color(colour));
        ui.add_enabled(enabled, label.sense(Sense::click())).on_hover_text(hint).clicked()
    };
    // Right to left: the later arrow first
    if arrow(ui, "›", week < latest, "The week after") {
        standings.turn(1);
    }
    let title = if week == latest { format!("Week {} · the latest", week + 1) } else { format!("Week {}", week + 1) };
    ui.label(on_cork(&title).size(size::SMALL)).on_hover_text(st::week_words(week));
    if arrow(ui, "‹", week > 0, "The week before") {
        standings.turn(-1);
    }
}

/// The honours board: oak, a brass edge, gold leaf.
fn panel() -> egui::Frame {
    egui::Frame::none()
        .fill(HONOURS_OAK)
        .stroke(Stroke::new(2.0_f32, theme::BRASS))
        .rounding(2.0)
        .inner_margin(Margin::symmetric(22.0, 12.0))
        .shadow(egui::epaint::Shadow { offset: [2.0, 5.0].into(), blur: 12.0, spread: 0.0, color: Color32::from_black_alpha(130) })
}

/// The board's dark oak, a shade deeper than the frame's so the gold reads.
const HONOURS_OAK: Color32 = Color32::from_rgb(0x3a, 0x26, 0x15);
/// Your own line's wash.
const YOURS: Color32 = Color32::from_rgba_premultiplied(60, 48, 12, 60);

/// The board: its title, the names (scrolling in `names` of height), and
/// your own line under them when you're past the top.
fn honours(ui: &mut egui::Ui, standings: &Standings, names: f32, action: &mut Option<Action>) {
    let board = standings.board;
    // Short of room, the line under the title goes into its hover, for a name more
    let compact = names < SHORT_OF_ROOM;
    let names = if compact { names + size::SMALL + 4.0 } else { names };
    panel().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            let mut under = st::board_blurb(board).to_owned();
            if let Some(zone) = standings.zone() {
                under = format!("{} · {}", st::zone_name(zone), under);
            }
            let title = ui.label(
                RichText::new(board.title().to_uppercase())
                    .font(theme::display(TITLE_TEXT))
                    .color(theme::GOLD_LIGHT)
                    .extra_letter_spacing(3.0),
            );
            if compact {
                title.on_hover_text(under);
            } else {
                ui.label(RichText::new(under).font(theme::body(size::SMALL)).color(theme::GOLD_DIM).italics());
            }
        });
        ui.add_space(2.0);
        gilt_rule(ui);
        ui.add_space(6.0);

        let shown = standings.shown().filter(|_| standings.any());
        let Some(shown) = shown else {
            let line = match standings.answer() {
                None => "The names are being lettered…",
                Some(_) => "The first week's names go up when it ends, at the midnight before Towerday.",
            };
            empty(ui, line);
            return;
        };
        if shown.top.is_empty() {
            empty(ui, match board {
                LeaderboardKind::KeptWord => "Nobody kept a claim that week.",
                _ => "Nobody on this board that week.",
            });
        }
        egui::ScrollArea::vertical()
            .id_salt("capitol_standings")
            .max_height(names)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for entry in &shown.top {
                    line(ui, entry, action);
                }
            });
        // Past the top: a break, then you, always in sight
        if let Some(mine) = shown.mine.as_ref().filter(|m| !shown.top.iter().any(|e| e.me && e.rank == m.rank)) {
            ui.vertical_centered(|ui| ui.label(RichText::new("· · ·").font(theme::body(size::SMALL)).color(theme::GOLD_DIM)));
            line(ui, mine, action);
        }
        ui.add_space(4.0);
        gilt_rule(ui);
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            let mut notes = vec![format!("{} on this board", grouped(shown.entries))];
            if shown.mine.is_none() {
                notes.push("you're not on it this week".to_owned());
            }
            ui.label(RichText::new(notes.join(" · ")).font(theme::body(size::LABEL)).color(theme::GOLD_DIM));
            if shown.top.iter().any(|e| e.citizen) {
                ui.add_space(8.0);
                theme::citizen_mark(ui, size::LABEL, Some(theme::GOLD));
                ui.label(RichText::new("marks a citizen").font(theme::body(size::LABEL)).color(theme::GOLD_DIM).italics());
            }
        });
    });
}

/// A line where the names would be.
fn empty(ui: &mut egui::Ui, line: &str) {
    ui.vertical_centered(|ui| ui.label(RichText::new(line).font(theme::body(size::LARGE)).color(theme::GOLD_DIM).italics()));
}

/// One name on the board: the rank, the mark, the name, a dotted leader and
/// the value in Strands.
fn line(ui: &mut egui::Ui, e: &LeaderboardEntry, action: &mut Option<Action>) {
    let podium = e.rank <= PODIUM;
    let text = if podium { PODIUM_TEXT } else { LINE_TEXT };
    let gold = if podium { theme::GOLD_LIGHT } else { theme::GOLD };
    // Laid out by hand: a row exactly as tall as its line, so twenty fit
    let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), text + if podium { 9.0 } else { 6.0 }), Sense::hover());
    let painter = ui.painter_at(row);
    if e.me {
        painter.rect(row, 2.0, YOURS, Stroke::new(1.0_f32, theme::GOLD_DIM));
    }
    let y = row.center().y;
    painter.text(
        egui::pos2(row.left() + RANK_WIDTH, y),
        egui::Align2::RIGHT_CENTER,
        format!("{}.", e.rank),
        theme::display(text),
        gold,
    );
    let value = painter.text(
        egui::pos2(row.right() - 6.0, y),
        egui::Align2::RIGHT_CENTER,
        format!("{} Strands", grouped(e.value)),
        theme::body(text),
        gold,
    );
    // The mark and the name, in a cell of their own, for the click
    let cell = egui::Rect::from_min_max(egui::pos2(row.left() + RANK_WIDTH + 8.0, row.top()), egui::pos2(value.left() - 12.0, row.bottom()));
    let mut name_ui = ui.new_child(egui::UiBuilder::new().max_rect(cell).layout(Layout::left_to_right(Align::Center)));
    name_ui.spacing_mut().item_spacing.x = 5.0;
    if e.citizen {
        theme::citizen_mark(&mut name_ui, text, Some(gold));
    }
    let name = match (&e.name, e.player_id) {
        (Some(name), id) => {
            let shown = if e.me { format!("{name} (you)") } else { name.clone() };
            let font = if podium { theme::strong(text) } else { theme::body(text) };
            let r = name_ui.add(egui::Label::new(RichText::new(shown).font(font).color(gold)).sense(Sense::click()));
            if let Some(id) = id.filter(|_| r.hovered()) {
                painter.hline(r.rect.x_range(), r.rect.bottom() - 1.0, Stroke::new(1.0_f32, theme::GOLD_DIM));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if r.clicked() {
                    *action = Some(Action::Card(id));
                }
            }
            r.rect
        }
        (None, _) => {
            let shown = if e.me { "You, as a resident" } else { "a resident" };
            name_ui
                .label(RichText::new(shown).font(theme::body(text)).color(gold).italics())
                .on_hover_text("Keeps their name out of the papers")
                .rect
        }
    };
    // The leader: dots from the name out to the value
    let dot_y = y + text * 0.25;
    let mut x = name.right() + 8.0;
    while x < value.left() - 8.0 {
        painter.circle_filled(egui::pos2(x, dot_y), 0.9, theme::GOLD_DIM.gamma_multiply(0.7));
        x += 6.0;
    }
}

/// A thin gold line across the board.
fn gilt_rule(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0_f32, theme::GOLD_DIM));
}

/// Standings for `--board-sample standings`: a week's four boards with
/// citizens among the names, a resident, and you past the top 20 on
/// wealth. `people` leaves citizens out, as the server would.
pub fn sample(people: bool) -> Standings {
    use capitol_core::protocol::rest::{Leaderboard, LeaderboardQuery, LeaderboardsResponse};
    const NAMES: [(&str, bool); 26] = [
        ("Ada", false), ("Cog", true), ("Bo", false), ("Wren", true), ("", false), ("Hal", false), ("Pip", true),
        ("Mara", false), ("Tam", false), ("Quill", true), ("Ivo", false), ("Sel", false), ("Juno", true),
        ("Odo", false), ("Rue", false), ("Fen", true), ("Lark", false), ("Nim", false), ("Bram", true),
        ("Cass", false), ("Edda", false), ("Tib", true), ("Vale", false), ("Oren", false), ("Dee", false),
        ("Moss", true),
    ];
    let me = 24;
    let board = |kind: LeaderboardKind, top_value: i64, step: i64, count: usize| -> Leaderboard {
        let mut lines = Vec::new();
        let mut rank = 0;
        for (k, (name, citizen)) in NAMES.iter().enumerate().take(count) {
            if people && *citizen {
                continue;
            }
            rank += 1;
            // Two tie for fifth
            let value = (top_value - step * k as i64).max(1);
            let named = !name.is_empty();
            lines.push(LeaderboardEntry {
                rank: if k == 6 && !people { rank - 1 } else { rank },
                name: named.then(|| name.to_string()),
                player_id: named.then(|| Uuid::from_u128(k as u128 + 1)),
                citizen: *citizen,
                value: if k == 6 && !people { (top_value - step * 5).max(1) } else { value },
                me: k == me,
            });
        }
        let mine = lines.iter().find(|e| e.me).cloned();
        let entries = lines.len() as i64;
        lines.truncate(20);
        Leaderboard { board: kind, top: lines, mine, entries }
    };
    let response = LeaderboardsResponse {
        week: Some(2),
        latest: Some(2),
        zone: None,
        people,
        zones: vec!["urban".into(), "suburban".into(), "rural".into()],
        boards: vec![
            board(LeaderboardKind::Wealth, 14_820, 430, 26),
            board(LeaderboardKind::Maker, 3_960, 260, 12),
            board(LeaderboardKind::Trader, 9_400, 610, 9),
            board(LeaderboardKind::KeptWord, 0, 0, 0),
        ],
    };
    let mut standings = Standings::new();
    standings.set_people(people);
    let query = standings.wants().unwrap_or_else(|| LeaderboardQuery { people, ..Default::default() });
    standings.sent(query.clone());
    standings.answered(query, response);
    standings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(standings: &mut Standings, width: f32) -> Option<Action> {
        let ctx = theme::test_context();
        let mut action = None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width + 40.0, 800.0))),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(width);
                action = page(ui, standings, 500.0);
            });
        });
        action
    }

    #[test]
    fn every_board_draws_wide_and_narrow_with_and_without_citizens() {
        for people in [false, true] {
            let mut s = sample(people);
            for board in LeaderboardKind::ALL {
                s.board = board;
                for width in [720.0, 320.0] {
                    assert_eq!(draw(&mut s, width), None);
                }
            }
        }
        // Before the first week ends, the board says so
        assert_eq!(draw(&mut Standings::new(), 600.0), None);
    }

    #[test]
    fn the_sample_puts_you_past_the_top_and_people_only_drops_citizens() {
        let s = sample(false);
        let wealth = s.shown().unwrap();
        assert_eq!(wealth.top.len(), 20);
        assert!(wealth.mine.as_ref().is_some_and(|m| m.rank > 20));
        assert!(wealth.top.iter().any(|e| e.citizen) && wealth.top.iter().any(|e| e.name.is_none() && e.player_id.is_none()));
        let p = sample(true);
        let wealth = p.shown().unwrap();
        assert!(wealth.top.iter().all(|e| !e.citizen));
        assert_eq!(wealth.top.iter().map(|e| e.rank).take(3).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(p.people());
    }
}
