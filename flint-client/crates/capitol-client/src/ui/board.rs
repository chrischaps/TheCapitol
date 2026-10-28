//! The notice board's Wanted page (Commons S2.3): the city's commissions,
//! each a paper card pinned to cork.
//!
//! The board in the hall's forecourt opens to two pages, Wanted and The
//! Gazette (`ui/gazette.rs`, which draws the tabs and holds the panel). A
//! card shows what's wanted (the item, the grade and over, how many still
//! out of how many), the price each and in all, how long until it's due,
//! who posted it, and the claims standing on it as a pencilled tally. Under
//! it is what you can do: claim some, deliver, give your claim up, or take
//! your own down.
//!
//! Over the cards: what to show (everything, what your pack could fill, or
//! yours), an item, and *Post a commission*. Posting and delivering each
//! open a larger card, a form with the court's numbers worked out before
//! you commit: the fee is `rules::court::court_fee`, which units go is
//! `rules::court::delivery`, both through `capitol_core::commissions`.
//! Reading, claiming and taking down work from anywhere; a post or a
//! delivery away from the hall walks you there and goes on arrival.
//!
//! The panel works the `Board` directly. What reaches past it comes back as
//! an [`Action`]: a call to make, and a name that opens a card.

use super::pad_nav::FollowFocus;
use capitol_core::catalog::Catalog;
use capitol_core::chrono::{DateTime, Utc};
use capitol_core::commissions::{self as cm, Ask, Board, Show, Showing, GRADES};
use capitol_core::protocol::rest::{Commission, SlotItem};
use capitol_core::protocol::rules::court;
use capitol_core::state::Container;
use egui::{Align, Color32, Layout, Margin, RichText, Sense, Stroke};
use uuid::Uuid;

use super::icons::ItemIcons;
use super::slots;
use super::theme::{self, size};

/// Two columns of cards from this wide.
pub(super) const TWO_COLUMNS: f32 = 560.0;
/// The gap between cards, and the room over each for its pin.
pub(super) const GAP: f32 = 12.0;
pub(super) const PIN: f32 = 5.0;
/// A card's icon.
pub(super) const ICON: f32 = 28.0;
/// Where a form's button was drawn this frame, kept in egui's memory for
/// the tests to click.
const GO: &str = "capitol_board_go";
/// The most tally marks a card pencils before it writes a number.
const TALLY_MOST: usize = 25;

/// What the page needs besides the board.
pub struct View<'a> {
    pub pack: Option<&'a Container>,
    pub me: Option<Uuid>,
    /// Your Strands, for posting. `None` until the server says.
    pub strands: Option<i64>,
    /// The server's clock, for what's due when.
    pub now: DateTime<Utc>,
    pub catalog: &'a Catalog,
    /// Standing at the Exchange hall, where posting and delivering happen
    pub at_hall: bool,
    /// Walking there to post or deliver
    pub walking: bool,
    /// Signed in. Offline it's a sample, and nothing is sent.
    pub online: bool,
}

/// What the page asks of the rest of the game.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Do this on the board: the session sends it, or walks you to the hall first
    Act(Ask),
    /// See who someone is: their card (S0.3)
    Card(Uuid),
}

/// The Wanted page, under the tabs: the filters and cards, or a form.
pub fn page(ui: &mut egui::Ui, board: &mut Board, view: &View, icons: &mut ItemIcons, room: f32) -> Option<Action> {
    let mut action = None;
    match board.showing() {
        Showing::List => {
            toolbar(ui, board, view, &mut action);
            ui.add_space(8.0);
            news(ui, board);
            egui::ScrollArea::vertical()
                .id_salt("capitol_board_cards")
                .max_height(room)
                .min_scrolled_height(room)
                .auto_shrink([false, true])
                .show_following(ui, |ui| cards(ui, board, view, icons, &mut action));
        }
        Showing::Post => {
            egui::ScrollArea::vertical()
                .id_salt("capitol_board_post")
                .max_height(room + 40.0)
                .auto_shrink([false, true])
                .show_following(ui, |ui| post_form(ui, board, view, icons, &mut action));
        }
        Showing::Deliver(id) => {
            egui::ScrollArea::vertical()
                .id_salt("capitol_board_deliver")
                .max_height(room + 40.0)
                .auto_shrink([false, true])
                .show_following(ui, |ui| deliver_form(ui, board, view, icons, id, &mut action));
        }
    }
    action
}

/// What to show, an item, and the way to post.
fn toolbar(ui: &mut egui::Ui, board: &mut Board, view: &View, action: &mut Option<Action>) {
    let fill = board.can_fill(view.pack, view.me).len();
    let mine = board.mine(view.me).len();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (show, label) in [
            (Show::All, format!("Everything · {}", board.open().len())),
            (Show::CanFill, format!("I can fill · {fill}")),
            (Show::Mine, format!("Mine · {mine}")),
        ] {
            if tag(ui, &label, board.show == show).clicked() {
                board.show = show;
            }
        }
        let items = board.items();
        if !items.is_empty() {
            let shown = board.item.as_ref().map_or_else(|| "Any item".to_owned(), |i| view.catalog.item_name(i));
            let mut picked = board.item.clone();
            ui.scope(|ui| {
                theme::paper_widgets(ui);
                egui::ComboBox::from_id_salt("capitol_board_item")
                    .selected_text(RichText::new(shown).size(size::SMALL))
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut picked, None, "Any item");
                        for item in items {
                            ui.selectable_value(&mut picked, Some(item.to_owned()), view.catalog.item_name(item));
                        }
                    });
            });
            board.item = picked;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if note_button(ui, "Post a commission", view.online).clicked() {
                let item = board.item.clone().unwrap_or_else(|| first_item(view.catalog));
                let price = view.catalog.target_price(&item).unwrap_or(10);
                board.write(&item, price);
            }
        });
    });
    let _ = action;
}

/// What the last thing you did did, or why it didn't.
fn news(ui: &mut egui::Ui, board: &Board) {
    let (text, colour) = match (board.refused(), board.done()) {
        (Some(why), _) => (why, theme::INK_DOWN),
        (None, Some(done)) => (done, theme::INK),
        _ => return,
    };
    slip(ui, |ui| {
        ui.label(RichText::new(text).size(size::SMALL).color(colour));
    });
    ui.add_space(6.0);
}

/// The cards, two to a row where there's room.
fn cards(ui: &mut egui::Ui, board: &mut Board, view: &View, icons: &mut ItemIcons, action: &mut Option<Action>) {
    if !board.loaded() && view.online {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(on_cork("Reading the board…")));
        return;
    }
    let shown: Vec<Commission> = board.listed_now(view.pack, view.me).into_iter().cloned().collect();
    if shown.is_empty() {
        ui.add_space(20.0);
        let line = match board.show {
            Show::All => "Nothing's wanted just now. Post a commission, and the city will see it here.",
            Show::CanFill => "Nothing on the board that your pack could fill.",
            Show::Mine => "You have nothing up. Post a commission to ask the city for goods.",
        };
        ui.vertical_centered(|ui| ui.label(on_cork(line).italics()));
        ui.add_space(20.0);
        return;
    }
    let two = ui.available_width() >= TWO_COLUMNS;
    let width = if two { (ui.available_width() - GAP) / 2.0 } else { ui.available_width() };
    for (row, pair) in shown.chunks(if two { 2 } else { 1 }).enumerate() {
        ui.add_space(PIN + 2.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for (i, c) in pair.iter().enumerate() {
                // A hand pinned them: every other card sits a little lower
                let drop = if (row + i) % 2 == 1 { 3.0 } else { 0.0 };
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(drop);
                    if let Some(a) = card(ui, board, view, icons, c) {
                        *action = Some(a);
                    }
                });
            }
        });
        ui.add_space(GAP - PIN);
    }
}

/// One commission's card.
fn card(ui: &mut egui::Ui, board: &mut Board, view: &View, icons: &mut ItemIcons, c: &Commission) -> Option<Action> {
    let mut action = None;
    let mine = view.me == Some(c.poster_id);
    let left = c.quantity - c.delivered;
    let response = paper_card(mine).show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::paper_widgets(ui);
        // The item, and the grade it asks for
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), Sense::hover());
            icons.paint(ui, rect, &c.item_type);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(view.catalog.item_name(&c.item_type)).font(theme::strong(size::LARGE)).color(theme::INK));
                ui.label(ink_dim(&cm::grade_words(c.min_quality), size::SMALL));
            });
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                        ui.label(RichText::new(format!("{} each", c.price)).font(theme::strong(size::BODY)).color(theme::INK));
                        ui.label(ink_dim(&format!("{} in all", court::commission_value(c.price, left)), size::SMALL));
                    });
                });
            });
        });
        ui.add_space(4.0);
        rule(ui);
        // How many, by when, from whom
        ui.horizontal(|ui| {
            ui.label(ink(&format!("{left} of {} wanted", c.quantity), size::BODY));
            ui.label(ink_dim("·", size::BODY));
            ui.label(ink_dim(&format!("due in {}", cm::time_left(c.due_at, view.now)), size::BODY));
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.label(ink_dim("Posted by", size::SMALL));
            if c.poster_citizen {
                theme::citizen_mark(ui, size::SMALL, Some(theme::INK));
            }
            if mine {
                ui.label(RichText::new("you").font(theme::strong(size::SMALL)).color(theme::INK));
            } else {
                if name_link(ui, &c.poster_name) {
                    action = Some(Action::Card(c.poster_id));
                }
                // What's said of them (S3.1)
                if !c.poster_standing.is_empty() {
                    ui.label(ink_dim(&format!("· {}", c.poster_standing), size::SMALL).italics());
                }
            }
        });
        claims(ui, board, c);
        ui.add_space(4.0);
        if let Some(a) = moves(ui, board, view, c) {
            action = Some(a);
        }
    });
    theme::pin(ui.painter(), egui::pos2(response.response.rect.center().x, response.response.rect.top() + 1.0), PIN);
    action
}

/// The claims standing on `c`, pencilled: a tally of the units held, and
/// your own claim in words.
fn claims(ui: &mut egui::Ui, board: &Board, c: &Commission) {
    if c.withdrawn {
        ui.label(ink_dim("Taken down: only its claims are delivered now.", size::SMALL).italics());
    }
    if c.claimed <= 0 {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Claimed").font(theme::body(size::SMALL)).color(theme::PENCIL).italics());
        let h = size::SMALL;
        let n = c.claimed as usize;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(tally_width(n.min(TALLY_MOST), h), h + 2.0), Sense::hover());
        theme::tally(ui.painter(), rect.min + egui::vec2(2.0, 1.0), h, n, TALLY_MOST);
        if n > TALLY_MOST {
            ui.label(RichText::new(format!("+{}", n - TALLY_MOST)).size(size::SMALL).color(theme::PENCIL));
        }
        ui.label(RichText::new(format!("{n}")).size(size::SMALL).color(theme::PENCIL));
    });
    let _ = board;
}

/// How wide `n` tally marks of height `h` run, as `theme::tally` draws them.
fn tally_width(n: usize, h: f32) -> f32 {
    let gap = h * 0.28;
    let fives = n / 5;
    let rest = n % 5;
    fives as f32 * (gap * 4.0 + gap * 1.6) + rest as f32 * gap + 4.0
}

/// What you can do with `c`: claim, deliver, give a claim up, or take
/// yours down.
fn moves(ui: &mut egui::Ui, board: &mut Board, view: &View, c: &Commission) -> Option<Action> {
    let mut action = None;
    if view.me == Some(c.poster_id) {
        ui.horizontal(|ui| {
            if c.withdrawn {
                ui.label(ink_dim("Waiting on its claims", size::SMALL));
            } else {
                ui.label(ink_dim(&format!("{} Strands held", c.escrow_strands), size::SMALL));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let hover = if c.claimed > 0 {
                        "The unclaimed part comes back now, and the claims may still be delivered"
                    } else {
                        "What's unspent comes back. The court keeps its fee."
                    };
                    if note_button(ui, "Take it down", view.online && !board.busy()).on_hover_text(hover).clicked() {
                        action = Some(Action::Act(Ask::Cancel(c.id)));
                    }
                });
            }
        });
        return action;
    }
    // Your claim on it, if you have one
    if let Some(claim) = board.my_claim(c.id).cloned() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "Yours: {} of {} in, {} left",
                    claim.delivered,
                    claim.quantity,
                    cm::time_left(claim.until, view.now)
                ))
                .font(theme::strong(size::SMALL))
                .color(theme::INK),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let late = court::release_status(view.now, claim.until) == court::ClaimStatus::Lapsed;
                let (label, hover) = if late {
                    ("Give up (a lapse)", "An hour or less is left, so giving it up counts on your record")
                } else {
                    ("Give it up", "With more than an hour left, it's no mark against you")
                };
                if note_button(ui, label, view.online && !board.busy()).on_hover_text(hover).clicked() {
                    action = Some(Action::Act(Ask::Release(c.id)));
                }
            });
        });
    }
    let open = Board::unclaimed(c);
    let could = board.open_to(c, view.me);
    let stacks = cm::stacks_for(c, view.pack);
    ui.horizontal(|ui| {
        if open > 0 && board.my_claim(c.id).is_none() {
            let id = egui::Id::new(("capitol_board_claim", c.id));
            let mut n: i32 = ui.data(|d| d.get_temp(id)).unwrap_or(open.min(5)).clamp(1, open);
            ui.add(egui::DragValue::new(&mut n).range(1..=open).speed(0.1));
            ui.data_mut(|d| d.insert_temp(id, n));
            if note_button(ui, "Claim", view.online && !board.busy())
                .on_hover_text("Hold that many for yourself for a day (never past the due date). Bring them in time, or it's a lapse.")
                .clicked()
            {
                action = Some(Action::Act(Ask::Claim { commission_id: c.id, quantity: n }));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let can = could > 0 && !stacks.is_empty();
            let hover = if could < 1 {
                "Everything still wanted is claimed"
            } else if stacks.is_empty() {
                "Nothing in your pack meets it"
            } else {
                "Choose the units, then hand them in at the hall"
            };
            if note_button(ui, "Deliver…", can).on_hover_text(hover).clicked() {
                board.start_delivery(c.id, view.pack, view.me);
            }
        });
    });
    action
}

/// The form for a new commission: what's wanted on the left, the court's
/// numbers and the button on the right (stacked where it's narrow).
fn post_form(ui: &mut egui::Ui, board: &mut Board, view: &View, icons: &mut ItemIcons, action: &mut Option<Action>) {
    let mine = board.mine(view.me).len();
    let busy = board.busy();
    let refused = board.refused().map(str::to_owned);
    let mut buttons = (false, false);
    let Some(draft) = board.draft.as_mut() else {
        board.back();
        return;
    };
    ui.add_space(PIN + 2.0);
    let response = paper_card(false).inner_margin(Margin::symmetric(18.0, 14.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::paper_widgets(ui);
        ui.label(kicker("A new commission"));
        rule(ui);
        ui.add_space(4.0);
        let sums = |ui: &mut egui::Ui, draft: &cm::Draft| post_sums(ui, draft, view, mine, refused.as_deref(), busy);
        if ui.available_width() >= TWO_COLUMNS {
            ui.columns(2, |c| {
                post_fields(&mut c[0], draft, view, icons);
                buttons = sums(&mut c[1], draft);
            });
        } else {
            post_fields(ui, draft, view, icons);
            ui.add_space(8.0);
            rule(ui);
            buttons = sums(ui, draft);
        }
    });
    theme::pin(ui.painter(), egui::pos2(response.response.rect.center().x, response.response.rect.top() + 1.0), PIN);
    let (back, go) = buttons;
    if back {
        board.back();
    }
    if go {
        if let Some(ask) = board.post(view.strands, view.me) {
            *action = Some(Action::Act(ask));
        }
    }
}

/// What's wanted: the item, the grade, how many, at what, by when.
fn post_fields(ui: &mut egui::Ui, draft: &mut cm::Draft, view: &View, icons: &mut ItemIcons) {
    egui::Grid::new("capitol_board_post_grid").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
        ui.label(ink_dim("Item", size::BODY));
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
            icons.paint(ui, rect, &draft.item_type);
            egui::ComboBox::from_id_salt("capitol_board_post_item")
                .selected_text(view.catalog.item_name(&draft.item_type))
                .width(160.0)
                .height(320.0)
                .show_ui(ui, |ui| {
                    for t in view.catalog.item_types() {
                        if ui.selectable_label(draft.item_type == t.id, &t.name).clicked() && draft.item_type != t.id {
                            draft.item_type = t.id.clone();
                            if let Some(p) = view.catalog.target_price(&t.id) {
                                draft.price = p.max(1);
                            }
                        }
                    }
                });
        });
        ui.end_row();
        ui.label(ink_dim("Grade", size::BODY));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for g in GRADES {
                if tag(ui, g, draft.grade == g).clicked() {
                    draft.grade = g.to_owned();
                }
            }
        });
        ui.end_row();
        ui.label("");
        let words = if draft.grade == "F" { "Any quality".to_owned() } else { format!("{} and over: {} or better", draft.grade, draft.min_quality()) };
        ui.label(ink_dim(&words, size::SMALL));
        ui.end_row();
        ui.label(ink_dim("How many", size::BODY));
        ui.add(egui::DragValue::new(&mut draft.quantity).range(1..=10_000).speed(0.2));
        ui.end_row();
        ui.label(ink_dim("Price each", size::BODY));
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut draft.price).range(1..=1_000_000).speed(0.2).suffix(" Strands"));
            if let Some(t) = view.catalog.target_price(&draft.item_type) {
                ui.label(ink_dim(&format!("about {t} at the Exchange"), size::SMALL));
            }
        });
        ui.end_row();
        ui.label(ink_dim("Due in", size::BODY));
        ui.add(
            egui::DragValue::new(&mut draft.due_days)
                .range(court::MIN_DUE_DAYS..=court::MAX_DUE_DAYS)
                .speed(0.05)
                .custom_formatter(|d, _| if d as i64 == 1 { "1 day".into() } else { format!("{} days", d as i64) }),
        );
        ui.end_row();
    });
}

/// The court's numbers, what stops it, and the buttons: (back, go).
fn post_sums(ui: &mut egui::Ui, draft: &cm::Draft, view: &View, mine: usize, refused: Option<&str>, busy: bool) -> (bool, bool) {
    let (mut back, mut go) = (false, false);
    egui::Grid::new("capitol_board_post_sums").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
        for (what, n) in [("Held in escrow", draft.value()), ("The court's fee", draft.fee())] {
            ui.label(ink_dim(what, size::SMALL));
            ui.label(ink(&format!("{n}"), size::SMALL));
            ui.end_row();
        }
        ui.label(RichText::new("It takes").font(theme::strong(size::BODY)).color(theme::INK));
        ui.label(RichText::new(format!("{} Strands", draft.total())).font(theme::strong(size::BODY)).color(theme::INK));
        ui.end_row();
        if let Some(s) = view.strands {
            ui.label(ink_dim("You'd have", size::SMALL));
            ui.label(ink_dim(&format!("{}", s - draft.total()), size::SMALL));
            ui.end_row();
        }
    });
    ui.add_space(2.0);
    ui.label(ink_dim("The fee is kept if it's taken down or runs out; what's unspent comes back.", size::TINY));
    ui.add_space(6.0);
    let stop = draft.stopped(view.strands, mine);
    if let Some(why) = refused.or(stop.as_deref()) {
        ui.label(RichText::new(why).size(size::SMALL).color(theme::INK_DOWN));
        ui.add_space(4.0);
    }
    let label = if busy {
        if view.walking { "Walking to the hall…" } else { "Pinning it up…" }
    } else if view.at_hall || !view.online {
        "Pin it up"
    } else {
        "Walk to the hall and pin it up"
    };
    let button = theme::accent_button(ui, label, theme::OAK, view.online && stop.is_none() && !busy);
    ui.data_mut(|d| d.insert_temp(egui::Id::new(GO), button.rect));
    go |= button.clicked();
    ui.add_space(4.0);
    back |= note_button(ui, "‹ Back to the board", true).clicked();
    (back, go)
}

/// The form for a delivery to `id`: which stack and which units on the
/// left, what goes and what it pays on the right (stacked where narrow).
fn deliver_form(ui: &mut egui::Ui, board: &mut Board, view: &View, icons: &mut ItemIcons, id: Uuid, action: &mut Option<Action>) {
    let Some(c) = board.commission(id).cloned() else {
        board.back();
        return;
    };
    let stacks: Vec<SlotItem> = cm::stacks_for(&c, view.pack).into_iter().cloned().collect();
    let could = board.open_to(&c, view.me);
    let Some(mut d) = board.delivery.clone() else {
        board.back();
        return;
    };
    let mut buttons = (false, false);
    ui.add_space(PIN + 2.0);
    let response = paper_card(false).inner_margin(Margin::symmetric(18.0, 14.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::paper_widgets(ui);
        ui.label(kicker("A delivery"));
        rule(ui);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), Sense::hover());
            icons.paint(ui, rect, &c.item_type);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(view.catalog.item_name(&c.item_type)).font(theme::strong(size::LARGE)).color(theme::INK));
                ui.label(ink_dim(
                    &format!("{}, {} each, for {}. Up to {could} from you.", cm::grade_words(c.min_quality), c.price, c.poster_name),
                    size::SMALL,
                ));
            });
        });
        ui.add_space(6.0);
        if ui.available_width() >= TWO_COLUMNS {
            ui.columns(2, |cols| {
                deliver_units(&mut cols[0], &c, &stacks, could, &mut d, icons);
                buttons = deliver_sums(&mut cols[1], board, &c, &stacks, &d, view);
            });
        } else {
            deliver_units(ui, &c, &stacks, could, &mut d, icons);
            ui.add_space(6.0);
            rule(ui);
            buttons = deliver_sums(ui, board, &c, &stacks, &d, view);
        }
    });
    theme::pin(ui.painter(), egui::pos2(response.response.rect.center().x, response.response.rect.top() + 1.0), PIN);
    board.delivery = Some(d);
    let (back, go) = buttons;
    if back {
        board.back();
    } else if go {
        if let Some(ask) = board.deliver(view.pack, view.me) {
            *action = Some(Action::Act(ask));
        }
    }
}

/// Which stack, how many, and which of its qualities.
fn deliver_units(ui: &mut egui::Ui, c: &Commission, stacks: &[SlotItem], could: i32, d: &mut cm::Delivery, icons: &mut ItemIcons) {
    if stacks.is_empty() {
        ui.label(ink("Nothing in your pack meets it now.", size::BODY).italics());
        return;
    }
    ui.label(ink_dim("From", size::SMALL));
    ui.horizontal_wrapped(|ui| {
        for s in stacks {
            let on = d.item_id == Some(s.item_id);
            if stack_chip(ui, icons, s, c.min_quality, on).clicked() && !on {
                d.item_id = Some(s.item_id);
                d.picked = None;
                d.quantity = could.min(cm::meeting(&s.units, c.min_quality) as i32).max(1);
            }
        }
    });
    let Some(stack) = d.item_id.and_then(|i| stacks.iter().find(|s| s.item_id == i)) else {
        return;
    };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(ink_dim("How many", size::SMALL));
        let most = could.min(cm::meeting(&stack.units, c.min_quality) as i32).max(1);
        ui.add_enabled(d.picked.is_none(), egui::DragValue::new(&mut d.quantity).range(1..=most).speed(0.1));
        ui.label(ink_dim(&format!("of {most}"), size::SMALL));
    });
    // Several qualities: pick which, as a trade's line does (#205)
    let mut rows = slots::pick_rows(&stack.units, d.picked.as_ref());
    if !rows.is_empty() {
        ui.add_space(4.0);
        ui.label(ink_dim(
            if d.picked.is_some() { "Handing in these" } else { "The lowest that meet it go, or pick" },
            size::TINY,
        ));
        let fields = slots::quality_rows(ui, &mut rows, &stack.units);
        if fields.iter().any(|f| f.changed()) {
            d.picked = slots::picked(&rows);
            if let Some(p) = &d.picked {
                d.quantity = p.count() as i32;
            }
        }
    }
}

/// What goes and what it pays, why not, and the buttons: (back, go).
fn deliver_sums(ui: &mut egui::Ui, board: &Board, c: &Commission, stacks: &[SlotItem], d: &cm::Delivery, view: &View) -> (bool, bool) {
    let (mut back, mut go) = (false, false);
    let stack = d.item_id.and_then(|i| stacks.iter().find(|s| s.item_id == i));
    let quote = stack.map(|s| board.quote(c, s, d, view.me));
    match &quote {
        Some(Ok(q)) => {
            ui.label(ink_dim("Going", size::SMALL));
            ui.label(ink(&slots::makeup(&q.units), size::BODY));
            ui.add_space(2.0);
            ui.label(RichText::new(format!("You're paid {} Strands", q.paid)).font(theme::strong(size::LARGE)).color(theme::INK));
        }
        Some(Err(why)) => {
            ui.label(RichText::new(why).size(size::SMALL).color(theme::INK_DOWN));
        }
        None => {}
    }
    ui.add_space(2.0);
    ui.label(ink_dim("Each unit keeps its quality and your mark, in their box at the hall.", size::TINY));
    if let Some(why) = board.refused() {
        ui.add_space(4.0);
        ui.label(RichText::new(why).size(size::SMALL).color(theme::INK_DOWN));
    }
    ui.add_space(6.0);
    let busy = board.busy();
    let label = if busy {
        if view.walking { "Walking to the hall…" } else { "Handing them in…" }
    } else if view.at_hall || !view.online {
        "Deliver"
    } else {
        "Walk to the hall and deliver"
    };
    let ok = matches!(quote, Some(Ok(_)));
    let button = theme::accent_button(ui, label, theme::OAK, view.online && ok && !busy);
    ui.data_mut(|d| d.insert_temp(egui::Id::new(GO), button.rect));
    go |= button.clicked();
    ui.add_space(4.0);
    back |= note_button(ui, "‹ Back to the board", true).clicked();
    (back, go)
}

/// A stack to deliver from: its icon, how many meet the grade, and its spread.
fn stack_chip(ui: &mut egui::Ui, icons: &mut ItemIcons, s: &SlotItem, min_quality: i32, on: bool) -> egui::Response {
    let fill = if on { theme::PAPER_EDGE } else { theme::PAPER_DEEP };
    let edge = if on { theme::INK } else { theme::INK_RULE };
    let inner = egui::Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, edge))
        .rounding(3.0)
        .inner_margin(Margin::symmetric(6.0, 3.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
                icons.paint(ui, rect, &s.item_type);
                ui.label(ink(&format!("{} meet it", cm::meeting(&s.units, min_quality)), size::SMALL));
                ui.label(ink_dim(&format!("of {}", s.quantity), size::SMALL));
            });
        });
    let r = inner.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    r.on_hover_ui(|ui| {
        slots::makeup_lines(ui, &s.units, size::SMALL);
    })
}

/// A card of paper on the cork; yours edged in gold.
pub(super) fn paper_card(mine: bool) -> egui::Frame {
    let edge = if mine { theme::GOLD } else { theme::PAPER_EDGE };
    egui::Frame::none()
        .fill(theme::PAPER)
        .stroke(Stroke::new(if mine { 1.5_f32 } else { 1.0 }, edge))
        .rounding(1.5)
        .inner_margin(Margin::symmetric(12.0, 9.0))
        .shadow(egui::epaint::Shadow { offset: [2.0, 4.0].into(), blur: 8.0, spread: 0.0, color: Color32::from_black_alpha(110) })
}

/// A narrow slip of paper for the page's news.
pub(super) fn slip(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(theme::PAPER)
        .stroke(Stroke::new(1.0_f32, theme::PAPER_EDGE))
        .rounding(1.0)
        .inner_margin(Margin::symmetric(10.0, 4.0))
        .show(ui, add);
}

/// A filter or a grade: a little paper tag, darker when it's chosen.
pub(super) fn tag(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let (fill, ink_colour) = if on { (theme::PAPER, theme::INK) } else { (theme::PAPER_DEEP.gamma_multiply(0.75), theme::INK_DIM) };
    let inner = egui::Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, if on { theme::INK } else { theme::PAPER_EDGE }))
        .rounding(2.0)
        .inner_margin(Margin::symmetric(7.0, 2.0))
        .show(ui, |ui| {
            let font = if on { theme::strong(size::SMALL) } else { theme::body(size::SMALL) };
            ui.label(RichText::new(text).font(font).color(ink_colour));
        });
    inner.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small button lettered on a slip of paper. One that can't be pressed
/// is lettered faintly but still says why on hover, which egui's disabled
/// widgets don't.
pub(super) fn note_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    ui.scope(|ui| {
        theme::paper_widgets(ui);
        let label = RichText::new(text).font(theme::strong(size::SMALL));
        if enabled {
            ui.add(egui::Button::new(label.color(theme::INK)))
        } else {
            let button = egui::Button::new(label.color(theme::INK_DIM.gamma_multiply(0.8)))
                .fill(theme::PAPER_DEEP.gamma_multiply(0.55))
                .stroke(Stroke::new(1.0_f32, theme::PAPER_EDGE))
                .sense(Sense::hover());
            ui.add(button)
        }
    })
    .inner
}

/// A person's name on a card: underlined under the pointer, and a click
/// opens their card (S0.3). Whether it was clicked.
pub(super) fn name_link(ui: &mut egui::Ui, name: &str) -> bool {
    let r = ui.add(egui::Label::new(RichText::new(name).font(theme::strong(size::SMALL)).color(theme::INK)).sense(Sense::click()));
    if r.hovered() {
        ui.painter().hline(r.rect.x_range(), r.rect.bottom() - 1.0, Stroke::new(1.0_f32, theme::INK_RULE));
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Who they are").clicked()
}

fn first_item(catalog: &Catalog) -> String {
    catalog.item_types().first().map_or_else(|| "plank".to_owned(), |t| t.id.clone())
}

pub(super) fn kicker(text: &str) -> RichText {
    RichText::new(text).font(theme::display(size::HEADING)).color(theme::INK).extra_letter_spacing(1.5)
}

pub(super) fn rule(ui: &mut egui::Ui) {
    theme::rule_in(ui, theme::INK_RULE.gamma_multiply(0.6));
}

pub(super) fn ink(text: &str, size: f32) -> RichText {
    RichText::new(text).font(theme::body(size)).color(theme::INK)
}

pub(super) fn ink_dim(text: &str, size: f32) -> RichText {
    RichText::new(text).font(theme::body(size)).color(theme::INK_DIM)
}

/// Words straight on the cork, in the paper's colour.
pub(super) fn on_cork(text: &str) -> RichText {
    RichText::new(text).font(theme::body(size::BODY)).color(theme::PAPER)
}

/// A board for `--board-sample`: a few commissions on it, one of them
/// yours, one you've claimed, and a pack that can fill two. `post` or
/// `deliver` opens the form.
pub fn sample(which: &str, catalog: &Catalog) -> (Board, Option<Container>, Uuid, DateTime<Utc>) {
    use capitol_core::protocol::rest::{ClaimsResponse, CommissionClaim, CommissionsResponse};
    use capitol_core::protocol::rules::court::{ClaimStatus, CommissionStatus};
    use capitol_core::protocol::rules::stacking::Units;
    use capitol_core::protocol::rules::standing;
    let posted: Commission = capitol_core::serde_json::from_str::<capitol_core::protocol::ws::ServerMessage>(include_str!(
        "../../../../../protocol/tests/fixtures/server/commission_posted.json"
    ))
    .ok()
    .and_then(|m| match m {
        capitol_core::protocol::ws::ServerMessage::CommissionPosted { commission } => Some(commission),
        _ => None,
    })
    .expect("the commission fixture parses");
    let now = posted.posted_at + capitol_core::chrono::Duration::hours(5);
    let me = Uuid::from_u128(0x389);
    let hours = capitol_core::chrono::Duration::hours;
    let make = |n: u128, item: &str, poster: &str, citizen: bool, q: i32, of: i32, done: i32, price: i64, due: i64| {
        let mut c = posted.clone();
        c.id = Uuid::from_u128(0x3890 + n);
        c.poster_id = Uuid::from_u128(0x4000 + n);
        c.poster_name = poster.into();
        c.poster_citizen = citizen;
        c.item_type = item.into();
        c.min_quality = q;
        c.quantity = of;
        c.delivered = done;
        c.price = price;
        c.escrow_strands = price * (of - done) as i64;
        c.fee = court::court_fee(price * of as i64);
        c.posted_at = now - hours(n as i64);
        c.due_at = now + hours(due);
        c.status = CommissionStatus::Open;
        c.poster_standing = match n {
            1 => standing::WELL_REGARDED.into(),
            2 => standing::DEPENDABLE.into(),
            3 => format!("{}, but {}", standing::FINDING_THEIR_FEET, standing::LET_DOWN_LATELY),
            5 => standing::PILLAR.into(),
            _ => standing::NEW_IN_TOWN.into(),
        };
        c
    };
    let iron = make(1, "iron_ore", "Citizen Cy", true, 55, 20, 6, 6, 77);
    let mut planks = make(2, "plank", "Odile Fenn", false, 75, 12, 0, 14, 30);
    planks.claimed = 7;
    let mut rope = make(3, "rope", "Brin Ashdown", false, 0, 8, 2, 9, 140);
    rope.claimed = 3;
    let mut mine = make(4, "stone", "You", false, 35, 40, 15, 3, 50);
    mine.poster_id = me;
    mine.claimed = 12;
    let bricks = make(5, "brick", "Citizen Wren", true, 90, 30, 0, 22, 160);
    let commissions = vec![iron.clone(), planks, rope.clone(), mine, bricks];
    let mut board = Board::default();
    board.connected();
    let ask = board.wants().expect("the board");
    board.sent(ask);
    board.listed(CommissionsResponse { commissions });
    let ask = board.wants().expect("your claims");
    board.sent(ask);
    board.claims_listed(ClaimsResponse {
        claims: vec![CommissionClaim {
            id: Uuid::from_u128(0x5389),
            commission_id: rope.id,
            claimant_id: me,
            claimant_name: "You".into(),
            claimant_citizen: false,
            quantity: 3,
            delivered: 1,
            claimed_at: now - hours(4),
            until: now + hours(20),
            status: ClaimStatus::Open,
            claimant_standing: String::new(),
        }],
    });
    // A pack with iron ore of three qualities, rope, and a few other things
    let stack = |n: u128, item: &str, units: Units| SlotItem {
        item_id: Uuid::from_u128(0x6000 + n),
        item_type: item.into(),
        item_name: catalog.item_name(item),
        quality: units.mean().unwrap_or(0),
        quality_grade: capitol_core::protocol::rules::quality_grade(units.mean().unwrap_or(0)).into(),
        quantity: units.count() as i32,
        min_quality: units.min().unwrap_or(0),
        units,
        slot_index: n as i32,
        durability: None,
        max_durability: None,
        maker: None,
        owner_id: None,
    };
    let mut slots: Vec<Option<SlotItem>> = vec![
        Some(stack(0, "iron_ore", Units::from_pairs([(48, 4), (61, 6), (78, 3)]))),
        Some(stack(1, "rope", Units::uniform(5, 66))),
        Some(stack(2, "plank", Units::uniform(9, 62))),
        Some(stack(3, "fiber", Units::uniform(20, 40))),
    ];
    slots.resize(20, None);
    let pack = Container { id: Uuid::from_u128(0x7000), container_type: "player_inventory".into(), layout_columns: 5, slots };
    match which {
        "post" => {
            board.write("iron_ore", catalog.target_price("iron_ore").unwrap_or(6));
            if let Some(d) = board.draft.as_mut() {
                d.quantity = 25;
                d.grade = "B".into();
            }
        }
        "deliver" => board.start_delivery(iron.id, Some(&pack), Some(me)),
        _ => {}
    }
    (board, Some(pack), me, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(board: &mut Board, pack: Option<&Container>, me: Uuid, now: DateTime<Utc>, width: f32) -> Option<Action> {
        let ctx = theme::test_context();
        let catalog = Catalog::fallback();
        let mut icons = ItemIcons::load(std::path::Path::new(ItemIcons::DIR));
        let view = View { pack, me: Some(me), strands: Some(500), now, catalog: &catalog, at_hall: false, walking: false, online: true };
        let mut action = None;
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 900.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = page(ui, board, &view, &mut icons, 600.0);
                });
            });
        }
        action
    }

    #[test]
    fn the_board_draws_wide_and_narrow_and_each_form() {
        let catalog = Catalog::fallback();
        for which in ["", "post", "deliver"] {
            let (mut board, pack, me, now) = sample(which, &catalog);
            for width in [900.0, 380.0] {
                assert_eq!(draw(&mut board, pack.as_ref(), me, now, width), None, "{which} at {width}");
            }
        }
    }

    #[test]
    fn the_sample_fills_the_filters() {
        let catalog = Catalog::fallback();
        let (board, pack, me, _) = sample("", &catalog);
        assert_eq!(board.open().len(), 5);
        assert_eq!(board.mine(Some(me)).len(), 1);
        let fill: Vec<String> = board.can_fill(pack.as_ref(), Some(me)).iter().map(|c| c.item_type.clone()).collect();
        assert_eq!(fill, ["iron_ore", "rope"], "your planks are under B, and nothing meets the A bricks");
    }

    #[test]
    fn the_delivery_form_previews_the_lowest_that_meet_it() {
        let catalog = Catalog::fallback();
        let (board, pack, me, _) = sample("deliver", &catalog);
        let d = board.delivery.clone().unwrap();
        let c = board.commission(d.commission_id).unwrap();
        let stack = pack.as_ref().unwrap().slots[0].as_ref().unwrap();
        let q = board.quote(c, stack, &d, Some(me)).unwrap();
        // 14 still wanted: the six 61s and three 78s that meet C, not the 48s
        assert_eq!(q.units.count(), 9);
        assert_eq!(q.units.min(), Some(61));
        assert_eq!(q.paid, 54);
    }

    /// Draws the page at `at_hall`, then clicks the form's button: what it asks
    fn click_go(board: &mut Board, pack: Option<&Container>, me: Uuid, now: DateTime<Utc>, at_hall: bool) -> Option<Action> {
        let ctx = theme::test_context();
        let catalog = Catalog::fallback();
        let mut icons = ItemIcons::load(std::path::Path::new(ItemIcons::DIR));
        let view = View { pack, me: Some(me), strands: Some(5000), now, catalog: &catalog, at_hall, walking: false, online: true };
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 1000.0));
        let mut action = None;
        let mut frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(a) = page(ui, board, &view, &mut icons, 600.0) {
                        action = Some(a);
                    }
                });
            });
        };
        frame(vec![]);
        frame(vec![]);
        let at = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(GO))).expect("the button was drawn").center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![button(true)]);
        frame(vec![button(false)]);
        action
    }

    #[test]
    fn the_post_form_pins_up_the_draft() {
        let catalog = Catalog::fallback();
        let (mut board, pack, me, now) = sample("post", &catalog);
        let draft = board.draft.clone().unwrap();
        let Some(Action::Act(Ask::Post(post))) = click_go(&mut board, pack.as_ref(), me, now, false) else {
            panic!("a post")
        };
        assert_eq!(post, draft.request());
        assert_eq!((post.quantity, post.min_quality), (25, 75), "B's floor");
    }

    #[test]
    fn the_delivery_form_hands_in_the_stack() {
        let catalog = Catalog::fallback();
        let (mut board, pack, me, now) = sample("deliver", &catalog);
        let Some(Action::Act(Ask::Deliver { commission_id, request })) = click_go(&mut board, pack.as_ref(), me, now, true)
        else {
            panic!("a delivery")
        };
        assert_eq!(commission_id, board.delivery.as_ref().unwrap().commission_id);
        assert_eq!(request.item_id, Uuid::from_u128(0x6000), "the iron ore");
        assert_eq!((request.quantity, request.qualities), (Some(9), None), "every unit that meets it, the lowest first");
    }

    #[test]
    fn tallies_run_as_wide_as_they_draw() {
        assert!(tally_width(5, 12.0) > tally_width(4, 12.0));
        assert!(tally_width(10, 12.0) >= 2.0 * tally_width(5, 12.0) - 4.0);
    }
}
