//! Citizens meet people (C1.5): a citizen answers the trades it's asked to.
//!
//! There's no chat, so a citizen speaks through its side of the trade
//! window:
//!
//! 1. **It takes the request** and stands where it is: a citizen's walks stop
//!    for a request ([`Bot::answer_trades`]), because the server drops one
//!    after 30 s and executes a trade only within range.
//! 2. **It lays out its wares**: everything it holds of what its role makes,
//!    if it can put a worth to it. That's the shop window.
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

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use capitol_protocol::rules::Zone;
use capitol_protocol::ws::TradeOfferItem;
use uuid::Uuid;

use super::decide::rules::worth_in;
use super::decide::log::Fate;
use super::decide::{Choice, Held, Lot, Mind, Place, Question, Quote, Situation, Span, When, Who};
use crate::bot::{Bot, BotError, TRADE_THINK};
use crate::sim::MARKET_ITEMS;
use crate::state::{Container, GameState, Offer, OpenTrade, StateEvent, Trade, TradeEvent};

/// How long a window may stand with nothing offered before it's closed.
pub const PATIENCE: Duration = Duration::from_secs(60);
/// The longest a citizen gives one trade.
pub const LONGEST: Duration = Duration::from_secs(300);
/// How long it waits for the trade to open once it has said yes.
const OPENING: Duration = Duration::from_secs(5);

/// The books a citizen watches, as it sees them: best bid and ask, and the
/// last trade, of each of [`MARKET_ITEMS`].
pub fn quotes(state: &GameState) -> Vec<Quote> {
    MARKET_ITEMS
        .iter()
        .map(|item| {
            let book = state.market().book(item);
            Quote {
                item: item.to_string(),
                bid: book.and_then(|b| b.best_bid()),
                ask: book.and_then(|b| b.best_ask()),
                last: state.market().trades(item).next().map(|t| t.price),
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

/// The shop window: every stack of `product` in the inventory, if the
/// citizen can put a worth to it.
pub fn window(inventory: Option<&Container>, product: Option<&str>, market: &[Quote]) -> Side {
    let (Some(inventory), Some(product)) = (inventory, product) else {
        return Side::default();
    };
    if worth_in(market, product).is_none() {
        return Side::default();
    }
    Side {
        lines: inventory
            .items()
            .filter(|i| i.item_type == product && i.quantity > 0)
            .map(|i| (i.item_id, i.item_type.clone(), i.quantity))
            .collect(),
        strands: 0,
    }
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
    /// It went through.
    Traded { partner: String },
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
            Met::Traded { partner } => format!("traded with {partner}"),
            Met::Declined { partner } => format!("declined {partner}'s trade"),
            Met::Ended { partner, reason } => format!("a trade with {partner} ended: {reason}"),
        }
    }
}

/// The citizen answering: who it is, what it makes, and game hours since
/// its life began.
#[derive(Debug, Clone)]
pub struct Answering<'a> {
    pub who: Who,
    pub product: Option<&'a str>,
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
    bot.say(format!("trading with {partner}"));
    // Turned to them, as anyone at a trade stands (C2.1). Before anything
    // is offered, so the turn's frames carry no news of the trade. Two
    // citizens would face each other, but no citizen asks another yet (#295)
    let facing = bot.state().open_trade().and_then(|o| bot.state().player(o.partner_id)).map(|p| (p.x, p.y));
    if let Some((x, y)) = facing {
        bot.face(x, y)?;
    }
    let me_id = bot.state().me();
    // Each fresh look at their offer supersedes the last (C3.1); the
    // caller logs what the last came to
    let mark = mind.mark();
    let began = Instant::now();
    let mut changed = Instant::now();
    let mut deal = Deal::default();
    let wares = window(bot.state().inventory(), me.product, &quotes(bot.state()));
    if !wares.is_empty() {
        let (items, strands) = wares.offer();
        let _ = bot.trade_act(|s, now| s.trade_offer(now, items, strands));
    }
    loop {
        let changes = bot.until("a trade", Duration::ZERO, |_, _| true)?;
        for change in &changes {
            match change {
                StateEvent::Trade(TradeEvent::OfferChanged { player_id }) if Some(*player_id) != me_id => {
                    changed = Instant::now()
                }
                StateEvent::Trade(TradeEvent::Executed { trade_id: id }) if *id == trade_id => {
                    bot.say(format!("traded with {partner}"));
                    return Ok(Met::Traded { partner });
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
        let nothing = open.theirs.items.is_empty() && open.theirs.strands == 0;
        if began.elapsed() > LONGEST || (nothing && changed.elapsed() > PATIENCE) {
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
            let menu = menu(&deal, &open, strands, &market);
            let s = situation(bot, me, question(&open), menu);
            deal.weighed = Some(open.theirs.clone());
            mind.settle(mark, &s.holdings, |_| Fate::Superseded);
            let ruling = mind.ask(&s);
            let side = match ruling.decision.map(|d| (d, s.choice(&d).clone())) {
                Some((_, Choice::Accept)) => Side::of(&open.mine),
                Some((d, Choice::Counter { .. })) => {
                    deal.countered = true;
                    trim(&open.mine, d.price.unwrap_or(0), &market)
                }
                Some((d, Choice::Pay { .. })) => {
                    deal.paid_for = Some(lots(&open.theirs));
                    Side {
                        lines: Vec::new(),
                        strands: d.price.unwrap_or(0),
                    }
                }
                _ => {
                    bot.say(format!("declining {partner}'s offer"));
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
        let side = window(Some(&inv), Some("log"), &logs_at(10));
        assert_eq!(side.lines, vec![(LOGS, "log".to_string(), 7)]);
        assert!(window(Some(&inv), Some("log"), &[]).is_empty(), "no worth, no sale");
        assert!(window(Some(&inv), None, &logs_at(10)).is_empty());
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
