//! Citizens meet people (C1.5): a citizen answers the trades it's asked to,
//! and asks a citizen nearby for one (#295).
//!
//! A citizen deals through its side of the trade window, and says a line
//! from its phrasebook as it acts (Commons S1.6, [`phrases`](super::phrases)):
//!
//! 1. **It takes the request** and stands where it is: a citizen's walks stop
//!    for a request ([`Bot::answer_trades`]), because the server drops one
//!    after 30 s and executes a trade only within range. Once the window is
//!    open it steps over to within earshot ([`HEARD`](rules::talk::HEARD)),
//!    since a trade is asked from further off than a word carries.
//! 2. **It lays out its wares**: everything it has spare, whatever job it
//!    came of (Callings K1.4, [`surplus`](crate::sim::surplus)), that it can
//!    put a worth to. That's the shop window.
//! 3. **It weighs what they put up**, once they've stopped changing it for
//!    [`TRADE_THINK`], through its cascade ([`Question::Trade`]):
//!    - Strands for its wares: **accept**; **counter**, once, by taking goods
//!      off its side until what they offer pays for what's left ("for that,
//!      you get this"); or **decline**, and close the window.
//!    - Goods: **pay** for them at its books, Strands in place of its wares,
//!      or decline. Barter isn't on the menu.
//! 4. **It accepts** what it settled on once the server has its side, and
//!    the trade executes when they accept too. A change on their side is
//!    weighed afresh; a spent counter stays spent.
//!
//! It closes a window nobody puts anything in for [`PATIENCE`], and gives a
//! trade [`LONGEST`] in all. Everything is valued at its books (the last
//! trade, else the middle of the book), what it can't put a worth to it
//! won't sell, and goods it can't value it won't pay for. All of it is the
//! trade any player makes: nothing is granted (D2).
//!
//! **Asking** (#295, [`ask`]). A citizen lingering among others knows the
//! citizens near it by their cards (Callings K3.2, [`Neighbour::read_card`]),
//! as anyone could: what the card says they've lately been busy with
//! (`PlayerCard.lately`, K3.1) is the goods they make, and the recipes
//! anyone can read say what those take. A card that says nothing (nothing
//! made lately, or someone who keeps out of the papers) says nothing to
//! ask them about, so they aren't asked. A surname is only a name.
//! When one makes what it wants and nobody is selling it at the hall, or
//! needs what it has spare and nobody is bidding, it may ask them
//! ([`ask_menu`], [`Question::Ask`]). It walks over and asks, and the window
//! goes as any other: buying, it waits for their wares and puts up Strands
//! for them, no more than it decided to spend; selling, it lays out what it
//! offered and waits for their Strands. Both sides weigh it through the same
//! [`Question::Trade`], each at its own books, and each turns to the other.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use capitol_protocol::rest::RecipeInfo;
use capitol_protocol::rules::lately::{self, Lately};
use capitol_protocol::rules::{self, Zone};
use capitol_protocol::ws::TradeOfferItem;
use uuid::Uuid;

use super::decide::rules::worth_in;
use super::decide::log::Fate;
use super::decide::{Choice, Held, Lot, Mind, Place, Question, Quote, Situation, Span, When, Who};
use super::phrases::Occasion;
use crate::bot::{Bot, BotError, TRADE_THINK};
use crate::sim::plan::Holdings;
use crate::sim::{grade, is_tool, watched, LOT, SELL_AT};
use crate::state::{Container, GameState, Offer, OpenTrade, StateEvent, Trade, TradeEvent};

/// How long a window may stand with nothing offered before it's closed.
pub const PATIENCE: Duration = Duration::from_secs(60);
/// The longest a citizen gives one trade.
pub const LONGEST: Duration = Duration::from_secs(300);
/// How long it waits for the trade to open once it has said yes.
const OPENING: Duration = Duration::from_secs(5);
/// How long an asker waits for the other side to put anything up: they lay
/// out their wares as the window opens, so not long (#295).
const ASKED_PATIENCE: Duration = Duration::from_secs(15);
/// How many of the citizens nearby an asker thinks of asking: the nearest.
const ASK_AMONG: usize = 3;
/// How close an answering citizen steps to whoever asked it, so its words
/// carry: well within [`HEARD`](rules::talk::HEARD).
const EARSHOT: f64 = 15.0;

/// The books a citizen watches, as it sees them: best bid and ask, and the
/// last trade, of each of the hall's goods and the tools ([`watched`],
/// Callings K2.1). A good's last trade is the median of its latest few
/// ([`grade::typical_last`], #429), so a lot sold at a grade's premium
/// doesn't raise its worth for any grade; a tool's is as it sold.
pub fn quotes(state: &GameState) -> Vec<Quote> {
    watched()
        .map(|item| {
            let book = state.market().book(item);
            let trades: Vec<_> = state.market().trades(item).collect();
            Quote {
                item: item.to_string(),
                bid: book.and_then(|b| b.best_bid()),
                ask: book.and_then(|b| b.best_ask()),
                last: if is_tool(item) { trades.first().map(|t| t.price) } else { grade::typical_last(&trades) },
            }
        })
        .collect()
}

/// What `item` is worth by the books in `state`: the last trade, else the
/// middle of the book, else either side.
pub fn worth(state: &GameState, item: &str) -> Option<i64> {
    worth_in(&quotes(state), item)
}

/// Units of each item in `offer`, by item.
pub fn lots(offer: &Offer) -> Vec<Lot> {
    let mut by: BTreeMap<&str, i32> = BTreeMap::new();
    for item in &offer.items {
        *by.entry(&item.item_type).or_insert(0) += item.quantity;
    }
    by.into_iter()
        .map(|(item, quantity)| Lot {
            item: item.to_string(),
            quantity,
        })
        .collect()
}

/// The trade question for the window as it stands.
pub fn question(open: &OpenTrade) -> Question {
    Question::Trade {
        partner: open.partner_name.clone(),
        partner_citizen: open.partner_citizen,
        theirs: lots(&open.theirs),
        their_strands: open.theirs.strands,
        ours: lots(&open.mine),
        our_strands: open.mine.strands,
    }
}

/// One side of the window as the citizen sends it: stacks, how many of
/// each, and Strands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Side {
    /// (stack, its item, how many of it)
    pub lines: Vec<(Uuid, String, i32)>,
    pub strands: i64,
}

impl Side {
    /// The side the server holds for us now.
    pub fn of(offer: &Offer) -> Side {
        Side {
            lines: offer.items.iter().map(|i| (i.item_id, i.item_type.clone(), i.quantity)).collect(),
            strands: offer.strands,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.strands == 0
    }

    /// Whether the server's side for us is this one.
    pub fn is(&self, offer: &Offer) -> bool {
        let mut mine: Vec<(Uuid, i32)> = offer.items.iter().map(|i| (i.item_id, i.quantity)).collect();
        let mut ours: Vec<(Uuid, i32)> = self.lines.iter().map(|(id, _, n)| (*id, *n)).collect();
        mine.sort();
        ours.sort();
        offer.strands == self.strands && mine == ours
    }

    /// As a `trade_offer`. Unpicked, the lowest units go.
    pub fn offer(&self) -> (Vec<TradeOfferItem>, i64) {
        let items = self
            .lines
            .iter()
            .map(|(id, _, n)| TradeOfferItem {
                item_id: id.to_string(),
                quantity: *n,
                qualities: None,
            })
            .collect();
        (items, self.strands)
    }
}

/// The shop window: what the citizen has spare (`wares`, by item), of what
/// it can put a worth to, the biggest stacks first.
pub fn window(inventory: Option<&Container>, wares: &[(String, i32)], market: &[Quote]) -> Side {
    let mut side = Side::default();
    for (item, n) in wares {
        if worth_in(market, item).is_some() {
            side.lines.extend(some_of(inventory, item, *n).lines);
        }
    }
    side
}

/// Where one trade stands, for the citizen in it.
#[derive(Debug, Clone, Default)]
pub struct Deal {
    /// Its counter is spent.
    pub countered: bool,
    /// Their side as it was when it was last weighed.
    pub weighed: Option<Offer>,
    /// The goods it last put Strands up for.
    pub paid_for: Option<Vec<Lot>>,
    /// The side it settled on, to accept once the server holds it.
    pub settled: Option<Side>,
}

/// What a citizen may answer, given their side, ours, and the Strands it
/// holds. Nothing while their side is empty: it waits.
///
/// - Goods offered, not yet paid for: pay (if it has a Strand), or decline.
///   Once paid for: accept, or decline.
/// - Strands for its wares: accept, counter (while it hasn't, and its side
///   has more than one unit's worth to take off), or decline. With no
///   wares out, only decline: it has nothing to sell them.
pub fn menu(deal: &Deal, open: &OpenTrade, strands: i64, market: &[Quote]) -> Vec<Choice> {
    let theirs = &open.theirs;
    if !theirs.items.is_empty() {
        if deal.paid_for.as_ref() == Some(&lots(theirs)) {
            return vec![Choice::Accept, Choice::Decline];
        }
        let mut menu = Vec::new();
        if strands >= 1 {
            menu.push(Choice::Pay {
                strands: Span::new(1, strands),
            });
        }
        menu.push(Choice::Decline);
        return menu;
    }
    if theirs.strands == 0 {
        return Vec::new();
    }
    if open.mine.items.is_empty() && open.mine.strands == 0 {
        return vec![Choice::Decline];
    }
    let mut menu = vec![Choice::Accept];
    let off = most_off(&open.mine, market);
    if !deal.countered && off >= 1 {
        menu.push(Choice::Counter {
            strands: Span::new(1, off),
        });
    }
    menu.push(Choice::Decline);
    menu
}

/// The worth of a unit of `item` to a counter: what it's valued at, and
/// never nothing.
fn unit(market: &[Quote], item: &str) -> i64 {
    worth_in(market, item).unwrap_or(0).max(1)
}

/// The most a counter can take off `ours` and still leave something to
/// give: its Strands, and all its goods but one unit of the cheapest.
fn most_off(ours: &Offer, market: &[Quote]) -> i64 {
    let goods: i64 = ours.items.iter().map(|i| i.quantity as i64 * unit(market, &i.item_type)).sum();
    match ours.items.iter().filter(|i| i.quantity > 0).map(|i| unit(market, &i.item_type)).min() {
        Some(least) => ours.strands + goods - least,
        None => (ours.strands - 1).max(0),
    }
}

/// A counter of `strands`, said with its own side: that many Strands off
/// what it gives, then goods at their worth, rounded its way, from the last
/// stack back. At least one unit stays.
pub fn trim(ours: &Offer, strands: i64, market: &[Quote]) -> Side {
    let mut side = Side::of(ours);
    let mut off = strands.max(0);
    let take = off.min(side.strands);
    side.strands -= take;
    off -= take;
    let mut units: i32 = side.lines.iter().map(|(_, _, n)| *n).sum();
    for (_, item, n) in side.lines.iter_mut().rev() {
        if off <= 0 {
            break;
        }
        let w = unit(market, item);
        let drop = ((off + w - 1) / w).min(*n as i64).min(units as i64 - 1).max(0) as i32;
        *n -= drop;
        units -= drop;
        off -= drop as i64 * w;
    }
    side.lines.retain(|(_, _, n)| *n > 0);
    side
}

/// How a trade a citizen answered came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Met {
    /// It went through: `strands` came to the citizen in it (less than
    /// nothing when it paid).
    Traded { partner: String, strands: i64 },
    /// The citizen declined it.
    Declined { partner: String },
    /// It ended otherwise: they cancelled or walked off, nobody offered
    /// anything, or it never opened.
    Ended { partner: String, reason: String },
}

impl Met {
    /// For a health line: "traded with Ada".
    pub fn label(&self) -> String {
        match self {
            Met::Traded { partner, .. } => format!("traded with {partner}"),
            Met::Declined { partner } => format!("declined {partner}'s trade"),
            Met::Ended { partner, reason } => format!("a trade with {partner} ended: {reason}"),
        }
    }
}

/// The citizen answering: who it is, what it has spare to lay out, by item
/// (Callings K1.4), and game hours since its life began.
#[derive(Debug, Clone)]
pub struct Answering {
    pub who: Who,
    pub wares: Vec<(String, i32)>,
    pub hours: f64,
}

/// What a layer knows when it weighs a trade: what it holds is its
/// inventory and Strands.
pub fn situation(bot: &Bot, me: &Answering, question: Question, menu: Vec<Choice>) -> Situation {
    let state = bot.state();
    let (x, y) = bot.position();
    let holdings = held(state);
    Situation {
        who: me.who.clone(),
        at: Place {
            x,
            y,
            zone: Zone::from_position(x, y).as_str().to_string(),
        },
        when: When {
            time_of_day: state.time_of_day(),
            hours: me.hours,
        },
        holdings,
        market: quotes(state),
        question,
        menu,
        // While a layer thinks, the fallback answers: there's no carrying on
        // with a trade someone is waiting on
        carry_on: None,
    }
}

/// What a citizen holds when it weighs a trade: its inventory and Strands.
pub fn held(state: &GameState) -> Held {
    let mut holdings = Held {
        strands: state.strands().unwrap_or(0),
        ..Held::default()
    };
    for item in state.inventory().into_iter().flat_map(|c| c.items()) {
        *holdings.items.entry(item.item_type.clone()).or_insert(0) += item.quantity;
    }
    holdings
}

/// Answers the trade request the bot holds (see the module's docs), and
/// returns how it came out once the trade is over. `Err` only for what any
/// call fails on: a drop, or the server ending the session.
pub fn answer(bot: &mut Bot, mind: &mut Mind, me: &Answering) -> Result<Met, BotError> {
    let partner = match bot.state().trade() {
        Trade::Incoming { name, .. } => name.clone(),
        _ => return Ok(ended("someone", "no request to answer")),
    };
    if bot.trade_act(|s, now| s.trade_accept_request(now)).is_err() {
        return Ok(ended(&partner, "the request was gone"));
    }
    match bot.until("the trade to open", OPENING, |s, _| !matches!(s.trade(), Trade::Incoming { .. })) {
        Ok(_) | Err(BotError::Timeout { .. }) => {}
        Err(e) => return Err(e),
    }
    let Some(trade_id) = bot.state().open_trade().map(|o| o.trade_id) else {
        return Ok(ended(&partner, "it never opened"));
    };
    bot.log(format!("trading with {partner}"));
    come_over(bot)?;
    face_partner(bot)?;
    speak_to_partner(bot, Occasion::Opening);
    let wares = window(bot.state().inventory(), &me.wares, &quotes(bot.state()));
    if !wares.is_empty() {
        let (items, strands) = wares.offer();
        let _ = bot.trade_act(|s, now| s.trade_offer(now, items, strands));
    }
    haggle(bot, mind, me, partner, trade_id, PATIENCE, menu)
}

/// What a citizen asks a neighbour for (#295): what a [`Choice::BuyFrom`]
/// or [`Choice::SellTo`] came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asking {
    /// Their `item`, `quantity` of it at about `price` each: it puts up no
    /// more Strands than that comes to.
    Buy { item: String, quantity: i32, price: i64 },
    /// `quantity` of its `item`, laid out for what they'll pay.
    Sell { item: String, quantity: i32 },
}

impl Asking {
    /// For a health line: "asking Ada Woodward for log".
    pub fn label(&self, partner: &str) -> String {
        match self {
            Asking::Buy { item, .. } => format!("asking {partner} for {item}"),
            Asking::Sell { item, .. } => format!("offering {partner} its {item}"),
        }
    }
}

/// Asks the citizen `them`, who must be within trading range, to trade
/// (see the module's docs), and returns how it came out. `Err` only for
/// what any call fails on.
pub fn ask(bot: &mut Bot, mind: &mut Mind, me: &Answering, them: Uuid, asking: &Asking) -> Result<Met, BotError> {
    let Some(partner) = bot.state().player(them).map(|p| p.name.clone()) else {
        return Ok(ended("someone", "they'd gone"));
    };
    if let Err(refused) = bot.trade_act(|s, now| s.trade_initiate(now, them)) {
        return Ok(ended(&partner, &refused.to_string()));
    }
    bot.log(format!("asking {partner} to trade"));
    bot.speak(Occasion::Opening, them);
    // They answer at their next step, within the request's life
    let wait = Duration::from_secs(rules::TRADE_REQUEST_TTL_SECS + 5);
    let seen = match bot.until("an answer", wait, |s, _| !matches!(s.trade(), Trade::Requested { .. })) {
        Ok(seen) => seen,
        Err(BotError::Timeout { .. }) => return Ok(ended(&partner, "no answer")),
        Err(e) => return Err(e),
    };
    let trade_id = match bot.state().trade() {
        Trade::Open(open) if open.partner_id == them => open.trade_id,
        Trade::Open(_) => {
            let _ = bot.trade_act(|s, now| s.trade_cancel(now));
            return Ok(ended(&partner, "another trade opened"));
        }
        // Someone else asked meanwhile: the loop answers them
        Trade::Incoming { .. } => return Ok(ended(&partner, "someone else asked")),
        _ if seen.iter().any(|e| matches!(e, StateEvent::Trade(TradeEvent::Declined { .. }))) => {
            return Ok(ended(&partner, "they said no"))
        }
        _ => return Ok(ended(&partner, "no answer")),
    };
    face_partner(bot)?;
    match asking {
        Asking::Buy { quantity, price, .. } => {
            // Its Strands go up for their wares, once it sees them
            let budget = price * *quantity as i64;
            haggle(bot, mind, me, partner, trade_id, ASKED_PATIENCE, |deal, open, strands, market| {
                menu(deal, open, strands.min(budget), market)
            })
        }
        Asking::Sell { item, quantity } => {
            let side = some_of(bot.state().inventory(), item, *quantity);
            if side.is_empty() {
                let _ = bot.trade_act(|s, now| s.trade_cancel(now));
                return Ok(ended(&partner, "it had none to sell"));
            }
            let (items, strands) = side.offer();
            let _ = bot.trade_act(|s, now| s.trade_offer(now, items, strands));
            // Their wares are no answer: it waits for their Strands
            haggle(bot, mind, me, partner, trade_id, PATIENCE, |deal, open, strands, market| {
                if open.theirs.strands == 0 {
                    return Vec::new();
                }
                menu(deal, open, strands, market)
            })
        }
    }
}

/// Up to `quantity` of `item` from the inventory, the biggest stacks first.
fn some_of(inventory: Option<&Container>, item: &str, quantity: i32) -> Side {
    let mut stacks: Vec<_> = inventory
        .into_iter()
        .flat_map(|inv| inv.items())
        .filter(|i| i.item_type == item && i.quantity > 0)
        .collect();
    stacks.sort_by_key(|i| std::cmp::Reverse(i.quantity));
    let mut left = quantity;
    let mut side = Side::default();
    for stack in stacks {
        if left <= 0 {
            break;
        }
        let n = left.min(stack.quantity);
        side.lines.push((stack.item_id, stack.item_type.clone(), n));
        left -= n;
    }
    side
}

/// Steps over to within [`EARSHOT`] of whoever the open trade is with, if
/// they stand further off, so what it says reaches them. A walk it can't
/// make is no matter: the trade goes on from where it stands.
fn come_over(bot: &mut Bot) -> Result<(), BotError> {
    let Some((x, y)) = bot.state().open_trade().and_then(|o| bot.state().player(o.partner_id)).map(|p| (p.x, p.y))
    else {
        return Ok(());
    };
    let (mx, my) = bot.position();
    let (dx, dy) = (mx - x, my - y);
    let d = dx.hypot(dy);
    if d <= rules::talk::HEARD * 0.8 {
        return Ok(());
    }
    match bot.walk_to(x + dx / d * EARSHOT, y + dy / d * EARSHOT) {
        Ok(()) | Err(BotError::Blocked { .. } | BotError::Timeout { .. }) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Says the line for `occasion` to whoever the open trade is with.
fn speak_to_partner(bot: &mut Bot, occasion: Occasion) {
    if let Some(partner) = bot.state().open_trade().map(|o| o.partner_id) {
        bot.speak(occasion, partner);
    }
}

/// Turns to whoever the open trade is with, as anyone at a trade stands
/// (C2.1). Before anything is offered, so the turn's frames carry no news
/// of the trade. Two citizens in a trade turn to each other.
fn face_partner(bot: &mut Bot) -> Result<(), BotError> {
    let facing = bot.state().open_trade().and_then(|o| bot.state().player(o.partner_id)).map(|p| (p.x, p.y));
    match facing {
        Some((x, y)) => bot.face(x, y),
        None => Ok(()),
    }
}

/// The open trade `trade_id` with `partner`, until it's over: each time
/// their side settles, it asks the cascade about it from what `menu_for`
/// offers (an empty menu: wait for more), and says what it decided with
/// its own side. It gives up on a window they put nothing in for
/// `patience`.
fn haggle(
    bot: &mut Bot,
    mind: &mut Mind,
    me: &Answering,
    partner: String,
    trade_id: Uuid,
    patience: Duration,
    menu_for: impl Fn(&Deal, &OpenTrade, i64, &[Quote]) -> Vec<Choice>,
) -> Result<Met, BotError> {
    let me_id = bot.state().me();
    // Each fresh look at their offer supersedes the last (C3.1); the
    // caller logs what the last came to
    let mark = mind.mark();
    let began = Instant::now();
    let mut changed = Instant::now();
    let mut deal = Deal::default();
    // The window as last seen: what went through, if it does
    let mut last: Option<OpenTrade> = None;
    loop {
        let changes = bot.until("a trade", Duration::ZERO, |_, _| true)?;
        for change in &changes {
            match change {
                StateEvent::Trade(TradeEvent::OfferChanged { player_id }) if Some(*player_id) != me_id => {
                    changed = Instant::now()
                }
                StateEvent::Trade(TradeEvent::Executed { trade_id: id }) if *id == trade_id => {
                    bot.log(format!("traded with {partner}"));
                    let strands = last.as_ref().map_or(0, |o| o.theirs.strands - o.mine.strands);
                    return Ok(Met::Traded { partner, strands });
                }
                StateEvent::Trade(TradeEvent::Cancelled { trade_id: id, reason }) if *id == trade_id => {
                    return Ok(ended(&partner, reason));
                }
                _ => {}
            }
        }
        let Some(open) = bot.state().open_trade().filter(|o| o.trade_id == trade_id).cloned() else {
            return Ok(ended(&partner, "the window closed"));
        };
        last = Some(open.clone());
        let nothing = open.theirs.items.is_empty() && open.theirs.strands == 0;
        if began.elapsed() > LONGEST || (nothing && changed.elapsed() > patience) {
            let _ = bot.trade_act(|s, now| s.trade_cancel(now));
            let why = if nothing { "nothing was offered" } else { "it took too long" };
            return Ok(ended(&partner, why));
        }
        if bot.state().trade_waiting().is_some() || changed.elapsed() < TRADE_THINK {
            continue;
        }
        if !nothing && deal.weighed.as_ref() != Some(&open.theirs) {
            let market = quotes(bot.state());
            let strands = bot.state().strands().unwrap_or(0);
            let menu = menu_for(&deal, &open, strands, &market);
            if menu.is_empty() {
                continue;
            }
            let s = situation(bot, me, question(&open), menu);
            deal.weighed = Some(open.theirs.clone());
            mind.settle(mark, &s.holdings, |_| Fate::Superseded);
            let ruling = mind.ask(&s);
            let side = match ruling.decision.map(|d| (d, s.choice(&d).clone())) {
                Some((_, Choice::Accept)) => {
                    bot.speak(Occasion::Accepting, open.partner_id);
                    Side::of(&open.mine)
                }
                Some((d, Choice::Counter { .. })) => {
                    deal.countered = true;
                    bot.speak(Occasion::Countering, open.partner_id);
                    trim(&open.mine, d.price.unwrap_or(0), &market)
                }
                Some((d, Choice::Pay { .. })) => {
                    deal.paid_for = Some(lots(&open.theirs));
                    bot.speak(Occasion::Accepting, open.partner_id);
                    Side {
                        lines: Vec::new(),
                        strands: d.price.unwrap_or(0),
                    }
                }
                _ => {
                    bot.log(format!("declining {partner}'s offer"));
                    bot.speak(Occasion::Declining, open.partner_id);
                    let _ = bot.trade_act(|s, now| s.trade_cancel(now));
                    return Ok(Met::Declined { partner });
                }
            };
            if !side.is(&open.mine) {
                let (items, strands) = side.offer();
                let _ = bot.trade_act(|s, now| s.trade_offer(now, items, strands));
            }
            deal.settled = Some(side);
            continue;
        }
        if deal.settled.as_ref().is_some_and(|side| side.is(&open.mine)) && !open.i_accepted {
            let _ = bot.trade_act(|s, now| s.trade_accept(now));
        }
    }
}

/// A citizen near an asker, as it sees them (#295): who and where, and
/// what their card says they deal in (Callings K3.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Neighbour {
    pub id: Uuid,
    pub name: String,
    pub at: (f64, f64),
    /// The goods their card says they've lately made: every good of each
    /// activity on it ([`lately::items_of`]), most first. A grouped
    /// activity names them all ("mining ore" is iron, copper and tin), so
    /// an asker knows the trade, not the exact good.
    pub makes: Vec<&'static str>,
    /// What those goods take, by the recipes anyone can read, less what they
    /// make themselves: what they'd buy.
    pub needs: Vec<String>,
}

impl Neighbour {
    /// Who's near, as an asker sees them before it has read their card:
    /// nothing yet known of what they make.
    pub fn seen(id: Uuid, name: &str, at: (f64, f64)) -> Self {
        Neighbour {
            id,
            name: name.to_string(),
            at,
            makes: Vec::new(),
            needs: Vec::new(),
        }
    }

    /// What their card's `lately` says of them (K3.2), the goods its
    /// activities make and what `recipes` say those take. `None` when it
    /// says nothing made: a quiet card (nothing made these three game days,
    /// or someone who keeps out of the papers, which reads the same), or
    /// only activities the table doesn't know. They aren't asked then.
    pub fn read_card(mut self, card: Option<&Lately>, recipes: &[RecipeInfo]) -> Option<Self> {
        let mut makes: Vec<&'static str> = Vec::new();
        for activity in &card?.doing {
            for &item in lately::items_of(activity) {
                if !makes.contains(&item) {
                    makes.push(item);
                }
            }
        }
        if makes.is_empty() {
            return None;
        }
        let mut needs: Vec<String> = Vec::new();
        for recipe in recipes.iter().filter(|r| r.outputs.iter().any(|o| makes.contains(&o.item_type.as_str()))) {
            for input in &recipe.inputs {
                let item = input.item_type.as_str();
                if !makes.contains(&item) && !needs.iter().any(|n| n == item) {
                    needs.push(item.to_string());
                }
            }
        }
        self.makes = makes;
        self.needs = needs;
        Some(self)
    }
}

/// The citizens within `reach` of `about`, the nearest to the bot first,
/// [`ASK_AMONG`] at most, as [seen](Neighbour::seen): whose cards to read.
/// People aren't asked: K3.2 kept #295's rule, though a person's card now says
/// what they do too ([#546](https://github.com/chrischaps/TheCapitol/issues/546)).
pub fn near(state: &GameState, about: (f64, f64), reach: f64) -> Vec<Neighbour> {
    let Some(me) = state.local_player() else {
        return Vec::new();
    };
    let (mx, my) = (me.x, me.y);
    let mut near: Vec<Neighbour> = state
        .players()
        .values()
        .filter(|p| p.citizen && p.id != me.id && (p.x - about.0).hypot(p.y - about.1) <= reach)
        .map(|p| Neighbour::seen(p.id, &p.name, (p.x, p.y)))
        .collect();
    near.sort_by(|a, b| (a.at.0 - mx).hypot(a.at.1 - my).total_cmp(&(b.at.0 - mx).hypot(b.at.1 - my)));
    near.truncate(ASK_AMONG);
    near
}

/// The menu for asking one of `neighbours` to trade (#295), if there's a
/// question to ask, for a citizen holding `holdings` that `wants` these
/// goods and has `spare` of those (Callings K1.4,
/// [`surplus`](crate::sim::surplus)):
///
/// - **buy from** one whose card says they make what it wants (the first
///   of its wants they make), when nobody is selling it at the hall, it can put a worth to it, and it can afford some (up to
///   half again the worth, up to two lots), keeping a deed's price back
///   until it has a plot, as a bid does;
/// - **sell to** one whose goods take what it has spare, when nobody is
///   bidding for it at the hall, it can put a worth to it, and it has half
///   of [`SELL_AT`] or more spare: the most it has spare of what they
///   need;
///
/// and **rest**, last: leave them be. What a neighbour makes and needs is
/// what their card says ([`Neighbour::read_card`], K3.2).
pub fn ask_menu(
    wants: &[&str],
    spare: &[(&str, i32)],
    holdings: &Holdings,
    market: &[Quote],
    neighbours: &[Neighbour],
) -> Option<Vec<Choice>> {
    let book = |item: &str| market.iter().find(|q| q.item == item);
    let selling = |item: &&str| book(item).is_some_and(|q| q.ask.is_some());
    let bidding = |item: &&str| book(item).is_some_and(|q| q.bid.is_some());
    let reserve = if holdings.plot_room.is_none() { holdings.deed_cost } else { 0 };
    let budget = holdings.strands - reserve;
    let mut menu = Vec::new();
    for n in neighbours {
        let wanted = wants
            .iter()
            .filter(|item| n.makes.contains(item) && !selling(item))
            .find_map(|&item| Some((item, worth_in(market, item)?.max(1))));
        if let Some((item, worth)) = wanted {
            let top = worth + (worth / 2).max(1);
            let afford = (budget / top).min(2 * LOT as i64) as i32;
            if afford > 0 {
                menu.push(Choice::BuyFrom {
                    partner: n.name.clone(),
                    item: item.to_string(),
                    price: Span::new((worth / 2).max(1), top),
                    quantity: Span::new(1, afford),
                });
            }
        }
        let needed = spare.iter().find(|&&(item, n_spare)| {
            n_spare >= SELL_AT / 2 && n.needs.iter().any(|n| n == item) && !bidding(&item) && worth_in(market, item).is_some()
        });
        if let Some(&(item, n_spare)) = needed {
            menu.push(Choice::SellTo {
                partner: n.name.clone(),
                item: item.to_string(),
                quantity: Span::new(1, n_spare),
            });
        }
    }
    if menu.is_empty() {
        return None;
    }
    menu.push(Choice::Rest);
    Some(menu)
}

fn ended(partner: &str, reason: &str) -> Met {
    Met::Ended {
        partner: partner.to_string(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citizen::decide::{Cascade, Temperament};
    use capitol_protocol::rules::stacking::Units;
    use capitol_protocol::ws::TradeItemInfo;

    const LOGS: Uuid = Uuid::from_u128(10);
    const MORE_LOGS: Uuid = Uuid::from_u128(11);
    const ORE: Uuid = Uuid::from_u128(12);

    fn logs_at(price: i64) -> Vec<Quote> {
        vec![Quote {
            item: "log".into(),
            bid: None,
            ask: None,
            last: Some(price),
        }]
    }

    fn item(id: Uuid, kind: &str, quantity: i32) -> TradeItemInfo {
        TradeItemInfo {
            item_id: id,
            item_type: kind.into(),
            item_name: kind.into(),
            quantity,
            quality: 50,
            units: Units::uniform(quantity, 50),
            maker: None,
        }
    }

    fn open(mine: Offer, theirs: Offer) -> OpenTrade {
        OpenTrade {
            trade_id: Uuid::from_u128(1),
            partner_id: Uuid::from_u128(2),
            partner_name: "Ada".into(),
            partner_citizen: false,
            mine,
            theirs,
            i_accepted: false,
            they_accepted: false,
        }
    }

    fn strands(n: i64) -> Offer {
        Offer {
            items: Vec::new(),
            strands: n,
        }
    }

    fn logs(stacks: &[(Uuid, i32)]) -> Offer {
        Offer {
            items: stacks.iter().map(|&(id, n)| item(id, "log", n)).collect(),
            strands: 0,
        }
    }

    #[test]
    fn the_window_is_its_wares_if_it_can_price_them() {
        let inv = Container::new(
            Uuid::from_u128(100),
            "player_inventory",
            24,
            6,
            &[
                capitol_protocol::rest::SlotItem::new(LOGS, "log".into(), "Log".into(), 50, 7, 0),
                capitol_protocol::rest::SlotItem::new(ORE, "iron_ore".into(), "Ore".into(), 50, 3, 1),
            ],
        );
        let wares = |w: &[(&str, i32)]| -> Vec<(String, i32)> { w.iter().map(|&(i, n)| (i.to_string(), n)).collect() };
        let side = window(Some(&inv), &wares(&[("log", 7)]), &logs_at(10));
        assert_eq!(side.lines, vec![(LOGS, "log".to_string(), 7)]);
        // What it has spare and no more, of what it can put a worth to
        let side = window(Some(&inv), &wares(&[("log", 4), ("iron_ore", 3)]), &logs_at(10));
        assert_eq!(side.lines, vec![(LOGS, "log".to_string(), 4)]);
        let mut market = logs_at(10);
        market.push(Quote {
            item: "iron_ore".into(),
            bid: None,
            ask: None,
            last: Some(6),
        });
        let side = window(Some(&inv), &wares(&[("log", 4), ("iron_ore", 3)]), &market);
        assert_eq!(side.lines, vec![(LOGS, "log".to_string(), 4), (ORE, "iron_ore".to_string(), 3)]);
        assert!(window(Some(&inv), &wares(&[("log", 7)]), &[]).is_empty(), "no worth, no sale");
        assert!(window(Some(&inv), &[], &logs_at(10)).is_empty());
    }

    #[test]
    fn the_menu_follows_what_they_put_up() {
        let market = logs_at(10);
        let deal = Deal::default();
        // Nothing yet: wait
        assert!(menu(&deal, &open(logs(&[(LOGS, 10)]), Offer::default()), 500, &market).is_empty());
        // Strands for ten logs: all three, the counter taking up to nine logs off
        let m = menu(&deal, &open(logs(&[(LOGS, 10)]), strands(80)), 500, &market);
        assert_eq!(
            m,
            vec![
                Choice::Accept,
                Choice::Counter {
                    strands: Span::new(1, 90)
                },
                Choice::Decline
            ]
        );
        // Its counter spent
        let spent = Deal {
            countered: true,
            ..Deal::default()
        };
        assert_eq!(menu(&spent, &open(logs(&[(LOGS, 10)]), strands(80)), 500, &market).len(), 2);
        // Strands, and nothing of its own out: nothing to sell
        assert_eq!(menu(&deal, &open(Offer::default(), strands(80)), 500, &market), vec![Choice::Decline]);
        // Goods: pay, then once paid, accept
        let theirs = logs(&[(MORE_LOGS, 4)]);
        let m = menu(&deal, &open(Offer::default(), theirs.clone()), 300, &market);
        assert_eq!(m[0], Choice::Pay { strands: Span::new(1, 300) });
        let paid = Deal {
            paid_for: Some(lots(&theirs)),
            ..Deal::default()
        };
        assert_eq!(menu(&paid, &open(strands(40), theirs.clone()), 300, &market), vec![Choice::Accept, Choice::Decline]);
        // Broke: only decline
        assert_eq!(menu(&deal, &open(Offer::default(), theirs), 0, &market), vec![Choice::Decline]);
    }

    #[test]
    fn a_counter_takes_goods_off_its_side_its_way_and_leaves_one() {
        let market = logs_at(10);
        let side = trim(&logs(&[(LOGS, 10)]), 70, &market);
        assert_eq!(side.lines, vec![(LOGS, "log".to_string(), 3)]);
        // Rounded its way: 25 off is three logs
        let side = trim(&logs(&[(LOGS, 10)]), 25, &market);
        assert_eq!(side.lines, vec![(LOGS, "log".to_string(), 7)]);
        // From the last stack back, and never the last log
        let side = trim(&logs(&[(LOGS, 3), (MORE_LOGS, 2)]), 1000, &market);
        assert_eq!(side.lines, vec![(LOGS, "log".to_string(), 1)]);
        // Strands go first
        let mut ours = logs(&[(LOGS, 2)]);
        ours.strands = 15;
        let side = trim(&ours, 20, &market);
        assert_eq!((side.strands, side.lines[0].2), (0, 1));
    }

    /// The rules' answer to the window, and the side it settles on.
    fn weigh(deal: &Deal, open: &OpenTrade, held: i64) -> (Choice, Side) {
        let market = logs_at(10);
        let menu = menu(deal, open, held, &market);
        let s = Situation {
            who: Who {
                name: "Percy Sawyer".into(),
                role: "gatherer-wood".into(),
                temperament: Temperament::default(),
                calling: None,
            },
            at: Place {
                x: 0.0,
                y: 0.0,
                zone: "rural".into(),
            },
            when: When {
                time_of_day: None,
                hours: 0.0,
            },
            holdings: Held::default(),
            market: market.clone(),
            question: question(open),
            menu,
            carry_on: None,
        };
        let d = Cascade::rules().ask(&s, Instant::now()).decision.unwrap();
        let choice = s.choice(&d).clone();
        let side = match &choice {
            Choice::Counter { .. } => trim(&open.mine, d.price.unwrap(), &market),
            Choice::Pay { .. } => Side {
                lines: Vec::new(),
                strands: d.price.unwrap(),
            },
            _ => Side::of(&open.mine),
        };
        (choice, side)
    }

    #[test]
    fn a_human_buying_logs_is_sold_them_at_the_books() {
        // A fair price for the window: taken as it stands
        let (choice, _) = weigh(&Deal::default(), &open(logs(&[(LOGS, 10)]), strands(100)), 0);
        assert_eq!(choice, Choice::Accept);
        // Thirty for ten: three, for thirty
        let (choice, side) = weigh(&Deal::default(), &open(logs(&[(LOGS, 10)]), strands(30)), 0);
        assert!(matches!(choice, Choice::Counter { .. }));
        assert_eq!(side.lines, vec![(LOGS, "log".to_string(), 3)]);
        // Then they come down to ten for the three: declined
        let spent = Deal {
            countered: true,
            ..Deal::default()
        };
        let (choice, _) = weigh(&spent, &open(logs(&[(LOGS, 3)]), strands(10)), 0);
        assert_eq!(choice, Choice::Decline);
        // A Strand for ten logs can't be countered into anything
        let (choice, _) = weigh(&Deal::default(), &open(logs(&[(LOGS, 10)]), strands(5)), 0);
        assert_eq!(choice, Choice::Decline);
    }

    #[test]
    fn a_human_selling_logs_is_paid_for_them_then_accepted() {
        let theirs = logs(&[(MORE_LOGS, 6)]);
        let (choice, side) = weigh(&Deal::default(), &open(logs(&[(LOGS, 10)]), theirs.clone()), 500);
        assert!(matches!(choice, Choice::Pay { .. }));
        assert_eq!(side, Side { lines: Vec::new(), strands: 60 });
        let paid = Deal {
            paid_for: Some(lots(&theirs)),
            ..Deal::default()
        };
        let (choice, _) = weigh(&paid, &open(strands(60), theirs), 500);
        assert_eq!(choice, Choice::Accept);
        // Goods it can't value: declined
        let ore = Offer {
            items: vec![item(ORE, "iron_ore", 5)],
            strands: 0,
        };
        let (choice, _) = weigh(&Deal::default(), &open(Offer::default(), ore), 500);
        assert_eq!(choice, Choice::Decline);
    }

    /// A card's `lately` busy with `doing`.
    fn busy(doing: &[&str]) -> Lately {
        Lately {
            doing: doing.iter().map(|d| d.to_string()).collect(),
            sells: Vec::new(),
        }
    }

    /// `name`, whose card says they've lately been busy with `doing`.
    fn neighbour(n: u128, name: &str, doing: &[&str]) -> Neighbour {
        let recipes = crate::catalog::Catalog::fallback();
        Neighbour::seen(Uuid::from_u128(n), name, (0.0, 0.0))
            .read_card(Some(&busy(doing)), recipes.recipes())
            .unwrap()
    }

    #[test]
    fn a_card_says_what_someone_makes_and_what_that_takes() {
        let catalog = crate::catalog::Catalog::fallback();
        let read = |doing: &[&str]| {
            Neighbour::seen(Uuid::from_u128(1), "Ada Woodward", (0.0, 0.0)).read_card(Some(&busy(doing)), catalog.recipes())
        };
        // Felling trees: logs, and nothing anyone makes logs from
        let ada = read(&["felling trees"]).unwrap();
        assert_eq!(ada.makes, vec!["log"]);
        assert!(ada.needs.is_empty());
        // A grouped activity names every good of it: the trade, not the item
        let ore = read(&["mining ore"]).unwrap();
        assert_eq!(ore.makes, vec!["iron_ore", "copper_ore", "tin_ore"]);
        // Smelting takes ore and charcoal, iron and bronze alike
        let smith = read(&["smelting metal"]).unwrap();
        for item in ["iron_ore", "charcoal", "copper_ore", "tin_ore"] {
            assert!(smith.needs.iter().any(|n| n == item), "{item}: {:?}", smith.needs);
        }
        // What they make themselves isn't what they'd buy: a sawyer making
        // handles from its own planks wants logs, sticks and rope
        let joiner = read(&["sawing planks", "making handles"]).unwrap();
        assert_eq!(joiner.makes, vec!["plank", "tool_handle"]);
        assert!(!joiner.needs.iter().any(|n| n == "plank"), "{:?}", joiner.needs);
        for item in ["log", "sticks", "rope"] {
            assert!(joiner.needs.iter().any(|n| n == item), "{item}: {:?}", joiner.needs);
        }
        // A quiet card (nothing lately, or kept out of the papers) and one
        // the table doesn't know say nothing to ask about
        assert_eq!(read(&[]), None);
        assert_eq!(
            Neighbour::seen(Uuid::from_u128(1), "Ada", (0.0, 0.0)).read_card(None, catalog.recipes()),
            None
        );
        assert_eq!(read(&["making lace"]), None);
        // Selling and nothing made: nothing to ask about either
        let selling = Lately {
            doing: Vec::new(),
            sells: vec![lately::Venue::Bazaar],
        };
        assert_eq!(
            Neighbour::seen(Uuid::from_u128(1), "Ada", (0.0, 0.0)).read_card(Some(&selling), catalog.recipes()),
            None
        );
    }

    #[test]
    fn a_citizen_asks_whoever_makes_what_it_wants_or_needs_what_it_has_spare() {
        // A sawyer at its sawing: it wants logs, and has its planks spare
        let wants = ["log"];
        let spare = [("plank", 14)];
        let ada = neighbour(1, "Ada Woodward", &["felling trees"]);
        let ivor = neighbour(2, "Ivor Mason", &["building workshops"]);
        let hazel = neighbour(3, "Hazel Reeder", &["cutting grass"]);
        let mut holdings = Holdings {
            strands: 500,
            plot_room: Some(2),
            deed_cost: 250,
            ..Holdings::default()
        };
        holdings.items.insert("plank".into(), 14);
        let mut market = logs_at(10);
        market.push(Quote {
            item: "plank".into(),
            bid: None,
            ask: None,
            last: Some(20),
        });
        let menu = ask_menu(&wants, &spare, &holdings, &market, &[ada.clone(), ivor.clone(), hazel.clone()]).unwrap();
        assert_eq!(
            menu,
            vec![
                Choice::BuyFrom {
                    partner: "Ada Woodward".into(),
                    item: "log".into(),
                    price: Span::new(5, 15),
                    quantity: Span::new(1, 2 * LOT),
                },
                Choice::SellTo {
                    partner: "Ivor Mason".into(),
                    item: "plank".into(),
                    quantity: Span::new(1, 14),
                },
                Choice::Rest,
            ]
        );
        // A fiber gatherer makes nothing it needs, nor needs what it makes
        assert_eq!(ask_menu(&wants, &spare, &holdings, &market, std::slice::from_ref(&hazel)), None);
        // A surname is only a name (K3.2): a Woodward whose card says she's
        // been sawing planks has no logs to sell, and takes the planks' logs
        let sawing = neighbour(1, "Ada Woodward", &["sawing planks"]);
        assert_eq!(ask_menu(&wants, &spare, &holdings, &market, std::slice::from_ref(&sawing)), None);
        // and a Reeder felling trees is asked for logs
        let felling = neighbour(3, "Hazel Reeder", &["cutting grass", "felling trees"]);
        let menu = ask_menu(&wants, &spare, &holdings, &market, std::slice::from_ref(&felling)).unwrap();
        assert!(matches!(&menu[0], Choice::BuyFrom { partner, item, .. } if partner == "Hazel Reeder" && item == "log"), "{menu:?}");
        // Someone selling logs at the hall, someone bidding for planks: the
        // book first
        let mut booked = market.clone();
        booked[0].ask = Some(11);
        booked[1].bid = Some(19);
        assert_eq!(ask_menu(&wants, &spare, &holdings, &booked, &[ada.clone(), ivor.clone()]), None);
        // No plot yet: a deed's price kept back, so three logs at most
        let homeless = Holdings {
            strands: 300,
            plot_room: None,
            ..holdings.clone()
        };
        let menu = ask_menu(&wants, &spare, &homeless, &market, std::slice::from_ref(&ada)).unwrap();
        assert!(matches!(&menu[0], Choice::BuyFrom { quantity, .. } if quantity.max == 3), "{menu:?}");
        // Logs it can't put a worth to: nothing to ask for
        assert_eq!(ask_menu(&wants, &spare, &holdings, &[], std::slice::from_ref(&ada)), None);
        // Too few planks spare to be worth the asking
        assert_eq!(ask_menu(&[], &[("plank", 9)], &holdings, &market, std::slice::from_ref(&ivor)), None);
        // On a wood stint after an ore one (Callings K1.4): it wants nothing
        // of Ada, and offers a smith its spare ore, the most it has spare
        // of what a smith needs
        let ida = neighbour(4, "Ida Smith", &["smelting metal"]);
        market.push(Quote {
            item: "iron_ore".into(),
            bid: None,
            ask: None,
            last: Some(6),
        });
        let spare = [("iron_ore", 30), ("plank", 14)];
        let menu = ask_menu(&[], &spare, &holdings, &market, &[ada, ida, hazel]).unwrap();
        assert_eq!(
            menu,
            vec![
                Choice::SellTo {
                    partner: "Ida Smith".into(),
                    item: "iron_ore".into(),
                    quantity: Span::new(1, 30),
                },
                Choice::Rest,
            ]
        );
    }

    #[test]
    fn some_of_takes_the_biggest_stacks_first() {
        let inv = Container::new(
            Uuid::from_u128(100),
            "player_inventory",
            24,
            6,
            &[
                capitol_protocol::rest::SlotItem::new(LOGS, "log".into(), "Log".into(), 50, 4, 0),
                capitol_protocol::rest::SlotItem::new(MORE_LOGS, "log".into(), "Log".into(), 50, 9, 1),
                capitol_protocol::rest::SlotItem::new(ORE, "iron_ore".into(), "Ore".into(), 50, 3, 2),
            ],
        );
        let side = some_of(Some(&inv), "log", 11);
        assert_eq!(side.lines, vec![(MORE_LOGS, "log".to_string(), 9), (LOGS, "log".to_string(), 2)]);
        assert!(some_of(Some(&inv), "plank", 5).is_empty());
    }

    #[test]
    fn a_citizen_buying_from_another_gets_what_it_set_out_to_spend_for() {
        // The asker sets out to spend 100 (ten logs at 10); the other lays
        // out all forty. It puts up all it set out to spend
        let budget = 100;
        let asker = open(Offer::default(), logs(&[(LOGS, 40)]));
        let (choice, ours) = weigh(&Deal::default(), &asker, budget);
        assert!(matches!(choice, Choice::Pay { .. }));
        assert_eq!(ours, Side { lines: Vec::new(), strands: 100 });
        // The other counters: for that, ten logs
        let seller = open(logs(&[(LOGS, 40)]), strands(100));
        let (choice, theirs) = weigh(&Deal::default(), &seller, 0);
        assert!(matches!(choice, Choice::Counter { .. }));
        assert_eq!(theirs.lines, vec![(LOGS, "log".to_string(), 10)]);
        // The asker pays the same for the ten, and the other takes it
        let paid = Deal {
            paid_for: Some(lots(&logs(&[(LOGS, 40)]))),
            ..Deal::default()
        };
        let (_, ours) = weigh(&paid, &open(strands(100), logs(&[(LOGS, 10)])), budget);
        assert_eq!(ours.strands, 100);
        let spent = Deal {
            countered: true,
            ..Deal::default()
        };
        let (choice, _) = weigh(&spent, &open(logs(&[(LOGS, 10)]), strands(100)), 0);
        assert_eq!(choice, Choice::Accept);
    }

    #[test]
    fn a_side_knows_the_servers_echo_of_it() {
        let side = Side {
            lines: vec![(LOGS, "log".into(), 3), (MORE_LOGS, "log".into(), 2)],
            strands: 0,
        };
        assert!(side.is(&logs(&[(MORE_LOGS, 2), (LOGS, 3)])));
        assert!(!side.is(&logs(&[(LOGS, 3)])));
        assert!(Side::default().is(&Offer::default()));
    }
}
