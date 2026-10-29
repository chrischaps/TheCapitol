//! A citizen among people (C1.5): answering trades, standing bids for what
//! it wants, and a stall for what it has spare that the hall doesn't take
//! (Callings K1.4: across its jobs, [`surplus`](super::super::surplus)).
//! Among other citizens it asks for trades too (#295). Everything
//! here follows the normal rules and pays the normal costs: the fee on a
//! bid, the rent on a stall (D2).

use std::time::{Duration, Instant};

use capitol_protocol::rest::{BazaarDirectory, PlaceOrder, SlotItem};
use capitol_protocol::rules::bazaar::{MAX_LISTINGS, MAX_SHOP_NAME, RENT_DAY_SECS};
use capitol_protocol::rules::lately::Lately;
use capitol_protocol::rules::market::OrderSide;
use uuid::Uuid;

use super::super::plan::Holdings;
use super::super::report::{AskFate, Entry};
use super::super::{bid_menu, grade, is_tool, stall_at, stall_menu, surplus, worth_unbooked, Trade, STALL_DAYS};
use super::{Action, SimBot};
use crate::bazaar;
use crate::bot::BotError;
use crate::citizen::decide::log::Fate;
use crate::citizen::decide::{Choice, Question, Who};
use crate::citizen::meet::{self, Answering, Asking, Met, Neighbour};
use crate::citizen::town::{Standing, Tether};
use crate::net::{RestError, RestRequest, RestResponse};

/// A standing bid is taken down and thought over again after this many
/// game seconds, at the citizen's next visit to the hall.
const BID_LIFE: f64 = 3600.0;
/// Game hours a citizen with no stall waits, once it found every stall at
/// the bazaar taken, before it reads the directory for a free one again
/// (#551).
const STALLS_AGAIN: f64 = 1.0;
/// Seconds of its hours a citizen signing off with an empty stall must be
/// on before the rent runs out to keep it (#523): less than an hour on,
/// and it couldn't stock it to any purpose.
const STALL_BACK_SECS: f64 = 3600.0;
/// How far from its spot a lingering citizen looks for a citizen to ask
/// (#295): about fifteen metres, across the hall's steps.
const ASK_REACH: f64 = 150.0;
/// Game seconds before it asks the same citizen again: half a game hour.
const ASK_AGAIN: f64 = 1800.0;
/// How close it stands to ask: about a metre and a quarter, as it stands
/// with company (C2.1).
const BESIDE: f64 = 12.0;

impl SimBot {
    /// Answers the trade someone asked for, where the citizen stands.
    pub(super) fn meet(&mut self, town: Option<&Tether>) -> Result<(), BotError> {
        let me = self.answering();
        let citizen = match self.bot.state().trade() {
            crate::state::Trade::Incoming { name, citizen, .. } => {
                if let Some(town) = town {
                    let doing = format!("trading with {name}");
                    town.health(|h| h.doing(Standing::Working, &doing));
                }
                *citizen
            }
            _ => false,
        };
        let mark = self.mind.mark();
        let met = meet::answer(&mut self.bot, &mut self.mind, &me);
        self.after_trade(mark, town, met, citizen, false)
    }

    /// While it lingers `about` a spot among others, whether to ask a
    /// citizen near it to trade (#295): it reads the cards of the nearest
    /// it hasn't asked lately (Callings K3.2, [`card_of`](Self::card_of)),
    /// then the mind's answer from [`meet::ask_menu`], and if it says ask,
    /// over beside them and the asking. Each is asked once in [`ASK_AGAIN`]
    /// at most. `true` if it asked someone.
    pub(super) fn ask_around(&mut self, about: (f64, f64), town: Option<&Tether>) -> Result<bool, BotError> {
        if !self.people {
            return Ok(false);
        }
        let now = Instant::now();
        let again = self.wall(ASK_AGAIN);
        self.asked.retain(|_, at| now.duration_since(*at) < again);
        let seen: Vec<Neighbour> = meet::near(self.bot.state(), about, ASK_REACH)
            .into_iter()
            .filter(|n| !self.asked.contains_key(&n.id))
            .collect();
        let mut neighbours = Vec::with_capacity(seen.len());
        for n in seen {
            let card = self.card_of(n.id)?;
            if let Some(n) = n.read_card(card.as_ref(), self.bot.state().catalog().recipes()) {
                neighbours.push(n);
            }
        }
        let holdings = self.holdings();
        let market = meet::quotes(self.bot.state());
        let spare = self.spare(&holdings, self.goal_now(&holdings).as_ref());
        let Some(menu) = meet::ask_menu(&self.wants(), &spare, &holdings, &market, &neighbours) else {
            return Ok(false);
        };
        let mark = self.mind.mark();
        let rest = menu.len() - 1;
        let (partner, asking) = match self.decide(Question::Ask, menu, rest, &holdings) {
            Some((d, Choice::BuyFrom { partner, item, .. })) => (
                partner,
                Asking::Buy {
                    item,
                    quantity: d.quantity.unwrap_or(1),
                    price: d.price.unwrap_or(1),
                },
            ),
            Some((d, Choice::SellTo { partner, item, .. })) => (
                partner,
                Asking::Sell {
                    item,
                    quantity: d.quantity.unwrap_or(1),
                },
            ),
            _ => {
                self.settle(mark, None, &Ok(()));
                self.entries.push(Entry::Asking { fate: AskFate::LetBe });
                return Ok(false);
            }
        };
        let Some(them) = neighbours.iter().find(|n| n.name == partner).map(|n| n.id) else {
            self.settle(mark, None, &Ok(()));
            return Ok(false);
        };
        self.asked.insert(them, now);
        if let Some(town) = town {
            let doing = asking.label(&partner);
            town.health(|h| h.doing(Standing::Working, &doing));
        }
        let me = self.answering();
        let met = self.step_up_to(them).and_then(|()| meet::ask(&mut self.bot, &mut self.mind, &me, them, &asking));
        let fate = match &met {
            Ok(Met::Traded { .. }) => AskFate::Traded,
            Ok(Met::Declined { .. }) => AskFate::Declined,
            Ok(Met::Ended { .. }) => AskFate::Ended,
            Err(_) => AskFate::Failed,
        };
        self.entries.push(Entry::Asking { fate });
        self.after_trade(mark, town, met, true, true)?;
        Ok(true)
    }

    /// What `them`'s card says they've lately been busy with (Callings
    /// K3.2), as anyone reading it would see: `GET /players/{id}/card`,
    /// read once a game day each, since a card looks three game days back
    /// and changes slowly. `None` is a card with nothing on it, or one the
    /// server wouldn't show (an HTTP refusal, kept for the day like a quiet
    /// card). A call that went unanswered isn't kept, so the next beat tries
    /// again; a key revoked or the server lost is the bot's to deal with.
    pub(super) fn card_of(&mut self, them: Uuid) -> Result<Option<Lately>, BotError> {
        let day = self.day();
        if let Some((read, lately)) = self.cards.get(&them) {
            if *read == day {
                return Ok(lately.clone());
            }
        }
        let lately = match self.bot.call("reading a card", RestRequest::PlayerCard { player_id: them }) {
            Ok(RestResponse::Card(card)) => card.lately,
            Ok(_)
            | Err(BotError::Rest {
                source: RestError::Http { .. },
                ..
            }) => None,
            Err(BotError::Rest { .. } | BotError::Timeout { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        self.cards.insert(them, (day, lately.clone()));
        Ok(lately)
    }

    /// Over to beside the player `them`, near enough to trade, turned
    /// their way.
    fn step_up_to(&mut self, them: Uuid) -> Result<(), BotError> {
        let Some((ox, oy)) = self.bot.state().player(them).map(|p| (p.x, p.y)) else {
            return Ok(());
        };
        let (x, y) = self.bot.position();
        let (dx, dy) = (x - ox, y - oy);
        let d = dx.hypot(dy);
        if d > BESIDE * 1.5 {
            // Blocked short of them: the ask finds them out of range
            match self.bot.walk_to(ox + dx / d * BESIDE, oy + dy / d * BESIDE) {
                Ok(()) | Err(BotError::Blocked { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        self.bot.face(ox, oy)
    }

    /// The citizen as a trade's cascade sees it, with what it has spare to
    /// lay out.
    fn answering(&self) -> Answering {
        let holdings = self.holdings();
        let wares = self
            .spare(&holdings, self.goal_now(&holdings).as_ref())
            .into_iter()
            .map(|(item, n)| (item.to_string(), n))
            .collect();
        Answering {
            who: Who {
                name: self.bot.name().to_string(),
                role: self.role.slug().to_string(),
                temperament: self.mind.temperament,
                calling: self.persona.as_ref().map(|p| p.calling.clone()),
            },
            wares,
            hours: self.now(),
        }
    }

    /// What a trade it was in came to, since `mark`: the trade's last
    /// decision logged with it (C3.1), a trade that went through written
    /// down for the report (with a `citizen` or a person, and whether it
    /// `asked`), and the health line.
    fn after_trade(
        &mut self,
        mark: usize,
        town: Option<&Tether>,
        met: Result<Met, BotError>,
        citizen: bool,
        asked: bool,
    ) -> Result<(), BotError> {
        self.journal();
        let fate = match &met {
            Ok(Met::Traded { .. }) => Fate::Traded,
            Ok(Met::Declined { .. }) => Fate::Declined,
            Ok(Met::Ended { reason, .. }) => Fate::Ended { reason: reason.clone() },
            Err(e) => Fate::Failed { why: e.to_string() },
        };
        let after = meet::held(self.bot.state());
        self.mind.settle(mark, &after, |_| fate.clone());
        let met = met?;
        if let Met::Traded { strands, .. } = &met {
            self.entries.push(Entry::Dealt {
                citizen,
                asked,
                strands: *strands,
                at: self.now(),
            });
        }
        if let Some(town) = town {
            town.health(|h| h.doing(Standing::Working, &met.label()));
        }
        Ok(())
    }

    /// What `item` is worth by the citizen's books, or before it has
    /// traded, its target (a tool's since K2.1, a good's since #410).
    pub(super) fn worth(&self, item: &str) -> Option<i64> {
        let state = self.bot.state();
        meet::worth(state, item).or_else(|| worth_unbooked(state.catalog(), item))
    }

    /// Whether it has a standing bid up for `item`.
    pub(super) fn bidding(&self, item: &str) -> bool {
        self.bot
            .state()
            .market()
            .open_orders()
            .iter()
            .any(|o| o.side == OrderSide::Buy && o.item_type == item)
    }

    /// Asks the mind whether to stock a stall with the `spare` it has of
    /// `item`: a citizen's question when nobody at the hall takes it.
    pub(super) fn choose_stall(&mut self, holdings: &Holdings, item: &str, spare: i32) -> Option<Trade> {
        if !self.people {
            return None;
        }
        // How full its shop is, if it keeps one: a full one takes no more (#544)
        let shelf = self.bot.state().bazaar().my_shop().map(|s| s.listings.len());
        let menu = stall_menu(holdings, item, spare, self.worth(item), shelf)?;
        // With none, only if there's one free to rent (#551)
        if shelf.is_none() && !self.stall_to_rent() {
            return None;
        }
        let question = Question::Sell { item: item.into() };
        match self.decide(question, menu, 1, holdings)? {
            (d, Choice::Stock { item, .. }) => Some(Trade::Stock {
                item,
                quantity: d.quantity?,
                price: d.price?,
            }),
            _ => None,
        }
    }

    /// Asks the mind whether to leave a standing bid for `item`, which the
    /// next step would gather and nobody is selling, or which it wants for
    /// a job it leans to: a citizen's question.
    pub(super) fn choose_bid(&mut self, holdings: &Holdings, item: &str) -> Option<Trade> {
        if !self.people {
            return None;
        }
        let menu = bid_menu(holdings, item, &self.wants(), self.worth(item), self.bidding(item))?;
        match self.decide(Question::Buy { item: item.into() }, menu, 1, holdings)? {
            (d, Choice::Bid { item, .. }) => Some(Trade::Bid {
                item,
                quantity: d.quantity?,
                price: d.price?,
            }),
            _ => None,
        }
    }

    /// At the hall: takes down standing bids, and asks that opened a book
    /// (#501), that have stood a game hour, and asks again about a bid for each thing it wants (the job at
    /// hand's inputs, then its nearest jobs', Callings K1.4), so whoever
    /// makes it has someone to sell to.
    pub(super) fn post_demand(&mut self) -> Result<(), BotError> {
        let life = chrono::Duration::from_std(self.wall(BID_LIFE)).unwrap_or(chrono::Duration::MAX);
        let now = self.bot.state().utc_now();
        let stale: Vec<_> = self
            .bot
            .state()
            .market()
            .open_orders()
            .iter()
            .filter(|o| now - o.created_at > life)
            .map(|o| o.id)
            .collect();
        // A merchant's quotes come down at its spell's end, so a sell an
        // hour old is an ask that opened a book. Its units go to the box
        let any = !stale.is_empty();
        for id in stale {
            match self.bot.cancel_order(id) {
                Ok(_) => {}
                // It filled meanwhile
                Err(BotError::Rest { source, .. }) if source.status().is_some_and(|s| s < 500) => {}
                Err(e) => return Err(e),
            }
        }
        if any {
            self.bot.collect_market_box()?;
        }
        for item in self.wants() {
            let holdings = self.holdings();
            let mark = self.mind.mark();
            let Some(trade @ Trade::Bid { .. }) = self.choose_bid(&holdings, item) else {
                self.settle(mark, None, &Ok(()));
                continue;
            };
            let Trade::Bid { item, quantity, price } = &trade else { unreachable!() };
            let result = self.bid(item, *quantity, *price);
            self.settle(mark, Some(&Action::Trade(trade)), &result);
            result?;
        }
        // What its bids haven't filled in a day, it posts for (S2.5)
        self.post_wants()?;
        // And with money to spare, for what its home lacks (K2.3)
        self.post_patronage()
    }

    /// Leaves a bid for `quantity` of `item` at `price` on the book, to
    /// stand until it fills or goes stale. The bot must be at the hall.
    pub(super) fn bid(&mut self, item: &str, quantity: i32, price: i64) -> Result<(), BotError> {
        // For the main ingredient of what it makes, a grade by its thrift, at
        // the grade's price, carried from the worth its price came from (#429)
        let jobs = surplus::near(&self.calling, self.job());
        let state = self.bot.state();
        let wanted = (!is_tool(item))
            .then(|| grade::bid_grade(state.catalog(), item, &jobs, self.mind.temperament.thrift))
            .flatten();
        let (price, quantity) = match wanted.and_then(grade::grade_mid) {
            Some(mid) => {
                let market = state.market();
                let at = grade::worth_quality(market.book(item), grade::typical_quality(market.trades(item)));
                let price = grade::carried(price, at, mid);
                (price, quantity.min(super::super::affordable(&self.holdings(), price)))
            }
            None => (price, quantity),
        };
        if quantity < 1 {
            return Ok(());
        }
        let order = self.bot.place_order(PlaceOrder {
            side: OrderSide::Buy,
            item_type: item.to_string(),
            price,
            quantity,
            item_id: None,
            qualities: None,
            min_grade: wanted.map(String::from),
            grade_rule: None,
            // A tool: at least half its uses (#547)
            min_durability: super::super::fewest_uses(self.bot.state().catalog(), item),
        })?;
        self.mind.placed(order.id);
        self.board.bid(item, price);
        Ok(())
    }

    /// Whether a citizen with no stall could rent one: the bazaar's
    /// directory shows one free. Once it finds every stall taken it doesn't
    /// read again for [`STALLS_AGAIN`], so it isn't asked to stock a stall
    /// it can't have, round after round (#551). If the directory can't be
    /// read, it tries, and [`keep_stall`](Self::keep_stall) says why not.
    pub(super) fn stall_to_rent(&mut self) -> bool {
        let now = self.now();
        if !looks_again(self.stalls_full, now) {
            return false;
        }
        let Ok(directory) = self.bot.bazaar() else {
            return true;
        };
        let free = free_stall(&directory).is_some();
        self.stalls_full = (!free).then_some(now);
        free
    }

    /// Gives its stall up when it has stopped using it (#523): with
    /// nothing on the shelf and nothing spare enough to stock it with
    /// ([`stall_at`]), once the rent has less than a day to run or it hasn't
    /// stocked it for a day ([`gives_up`]). The days paid aren't refunded,
    /// so a shelf with goods on it is kept to its lapse.
    pub(super) fn give_up_idle_stall(&mut self, holdings: &Holdings) -> Result<(), BotError> {
        if !self.people {
            return Ok(());
        }
        let Some(shop) = self.bot.state().bazaar().my_shop() else {
            self.stall_stocked = None;
            return Ok(());
        };
        let (empty, paid_until) = (shop.listings.is_empty(), shop.paid_until);
        let now = self.bot.state().utc_now();
        let stocked = *self.stall_stocked.get_or_insert(now);
        if !empty {
            return Ok(());
        }
        let stocking = self
            .spare(holdings, None)
            .iter()
            .any(|&(item, n)| n >= (stall_at(item) / 2).max(1));
        let left = (paid_until - now).num_seconds();
        let idle = (now - stocked).num_seconds();
        if gives_up(left, idle, stocking).is_none() {
            return Ok(());
        }
        self.give_stall_up()
    }

    /// As its session ends, gives up an empty stall it won't be on to stock
    /// for [`STALL_BACK_SECS`] before the rent runs out (#523): its habit's
    /// hours (#276) say when it's back, and a citizen on always keeps it.
    /// A citizen dropped from the roster never signs in to do this, so its
    /// stall lapses by the bazaar's sweep (#585).
    pub(super) fn close_stall_on_leaving(&mut self) -> Result<(), BotError> {
        if !self.people {
            return Ok(());
        }
        let Some(shop) = self.bot.state().bazaar().my_shop() else {
            return Ok(());
        };
        let (empty, paid_until) = (shop.listings.is_empty(), shop.paid_until);
        let now = self.bot.state().utc_now();
        if !leaves(empty, (paid_until - now).num_seconds(), self.on_between(now, paid_until)) {
            return Ok(());
        }
        self.give_stall_up()
    }

    fn give_stall_up(&mut self) -> Result<(), BotError> {
        self.bot.close_stall()?;
        self.stall_stocked = None;
        Ok(())
    }

    /// Stocks the citizen's stall with `quantity` of `item` at `price`,
    /// renting the first free stall for [`STALL_DAYS`] if it keeps none,
    /// and paying for more days when its rent has less than one to run.
    pub(super) fn keep_stall(&mut self, item: &str, quantity: i32, price: i64) -> Result<(), BotError> {
        let now = self.bot.state().utc_now();
        match self.bot.state().bazaar().my_shop().cloned() {
            None => {
                let directory = self.bot.bazaar()?;
                let Some(free) = free_stall(&directory).map(str::to_string) else {
                    // Taken since it looked: it waits before it asks again
                    self.stalls_full = Some(self.now());
                    return Err(BotError::Failed("every stall at the bazaar is taken".into()));
                };
                let name = shop_name(self.bot.name(), item);
                self.bot.rent_stall(&free, STALL_DAYS, Some(&name))?;
            }
            Some(shop) if (shop.paid_until - now).num_seconds() < RENT_DAY_SECS => {
                let days = bazaar::days_payable(Some(&shop), now).min(STALL_DAYS);
                if days > 0 {
                    self.bot.rent_stall(&shop.stall_id, days, None)?;
                }
            }
            Some(_) => {}
        }
        let mut stacks: Vec<_> = self
            .bot
            .state()
            .inventory()
            .into_iter()
            .flat_map(|inv| inv.items())
            .filter(|i| i.item_type == item)
            .cloned()
            .collect();
        // The biggest stacks first; of tools, the freshest (K2.1)
        stacks.sort_by_key(|s| std::cmp::Reverse((s.quantity, s.durability.unwrap_or(0))));
        // A good's price is for the quality its worth was (#578)
        let tool = is_tool(item);
        let keeps_best = self.keeps_best(item);
        let at = {
            let market = self.bot.state().market();
            grade::worth_quality(market.book(item), grade::typical_quality(market.trades(item)))
        };
        let mut left = quantity;
        for stack in stacks {
            if left <= 0 {
                break;
            }
            let n = left.min(stack.quantity);
            // A tool to a listing, priced by its wear; a good a grade to a
            // listing, its units named, each at the grade's price (#578)
            let lots = if tool {
                vec![(None, n, worn_price(price, &stack))]
            } else {
                let room = MAX_LISTINGS.saturating_sub(self.bot.state().bazaar().my_shop().map_or(0, |s| s.listings.len()));
                grade::plan_stall(&stack.units, n, price, at, keeps_best, room.max(1))
                    .into_iter()
                    .map(|sale| (Some(sale.units.clone()), sale.units.count() as i32, sale.price))
                    .collect()
            };
            for (units, n, price) in lots {
                let listings = self.bot.state().bazaar().my_shop().map_or(0, |s| s.listings.len());
                // Once the shelf fills, the rest stays in the pack, unless
                // nothing went up at all (#544)
                if listings >= MAX_LISTINGS && left < quantity {
                    break;
                }
                let listing = match &units {
                    Some(units) => bazaar::stock_units(&stack, units, price, listings),
                    None => bazaar::stock(&stack, n, price, listings),
                }
                .map_err(|e| BotError::Failed(e.message()))?;
                self.bot.stock(listing)?;
                left -= n;
            }
        }
        self.mind.stocked(item);
        self.stall_stocked = Some(now);
        // Give the stall a moment to show on the bazaar's signs
        self.bot.idle(Duration::from_millis(100))?;
        Ok(())
    }
}

/// What a stack is priced at, at `price` new (Callings K2.1): a tool by the
/// share of its uses it has left, which the stall and (since #547) the
/// hall's book show, at least a Strand.
/// Anything else, `price`.
pub(super) fn worn_price(price: i64, stack: &SlotItem) -> i64 {
    match (stack.durability, stack.max_durability) {
        (Some(left), Some(most)) if most > 0 && left < most => (price * i64::from(left) / i64::from(most)).max(1),
        _ => price,
    }
}

/// Why a citizen gives up a stall with an empty shelf, if it does (#523),
/// with `left` seconds of rent to run and `idle` since it last stocked it:
/// `short`, with less than a day to run, when it would have to pay more to
/// stock it again anyway, or `idle`, when it hasn't stocked it in a day.
/// Never while it has something spare to stock it with (`stocking`), and
/// not once the rent has run out: the bazaar's sweep has it then.
fn gives_up(left: i64, idle: i64, stocking: bool) -> Option<&'static str> {
    if stocking || left <= 0 {
        return None;
    }
    if left < RENT_DAY_SECS {
        Some("short")
    } else if idle >= RENT_DAY_SECS {
        Some("idle")
    } else {
        None
    }
}

/// Whether a citizen signing off gives its stall up (#523): its shelf is
/// `empty` and it's `on` less than [`STALL_BACK_SECS`] of the `left`
/// seconds before the rent runs out, so the days left would go unused.
fn leaves(empty: bool, left: i64, on: f64) -> bool {
    empty && left > 0 && on < STALL_BACK_SECS
}

/// The first stall the directory shows free to rent, if any.
fn free_stall(directory: &BazaarDirectory) -> Option<&str> {
    directory.stalls.iter().find(|s| s.shop.is_none()).map(|s| s.stall_id.as_str())
}

/// Whether a citizen that last found every stall taken at `full` (game
/// hours, or never) looks for a free one again at `now`: after
/// [`STALLS_AGAIN`] (#551).
fn looks_again(full: Option<f64>, now: f64) -> bool {
    full.is_none_or(|at| now - at >= STALLS_AGAIN)
}

/// A citizen's shop: "Percy Sawyer's logs", shortened to fit.
fn shop_name(name: &str, item: &str) -> String {
    let goods = match item {
        _ if is_tool(item) => "tools".to_string(),
        "iron_ore" => "ore".to_string(),
        "iron_ingot" => "iron".to_string(),
        "stone" | "charcoal" => item.to_string(),
        other => format!("{}s", other.replace('_', " ")),
    };
    let full = format!("{name}'s {goods}");
    if full.chars().count() <= MAX_SHOP_NAME {
        return full;
    }
    full.chars().take(MAX_SHOP_NAME).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shop_is_named_for_its_keeper_and_its_goods() {
        assert_eq!(shop_name("Percy Sawyer", "log"), "Percy Sawyer's logs");
        assert_eq!(shop_name("Ida Smith", "iron_ingot"), "Ida Smith's iron");
        assert_eq!(shop_name("Ida Smith", "iron_ore"), "Ida Smith's ore");
        assert_eq!(shop_name("Ida Smith", "stone_axe"), "Ida Smith's tools");
    }

    #[test]
    fn a_worn_tool_is_priced_by_the_uses_it_has_left() {
        let stack = |durability: Option<i32>, most: Option<i32>| SlotItem {
            durability,
            max_durability: most,
            ..SlotItem::new(Uuid::nil(), "stone_axe".into(), "Stone Axe".into(), 50, 1, 0)
        };
        assert_eq!(worn_price(50, &stack(Some(60), Some(60))), 50);
        assert_eq!(worn_price(50, &stack(Some(30), Some(60))), 25);
        assert_eq!(worn_price(50, &stack(Some(1), Some(60))), 1);
        assert_eq!(worn_price(8, &stack(None, None)), 8, "not a tool");
        let long = shop_name("Bartholomew Fitzwilliam-Cartwright", "plank");
        assert_eq!(long.chars().count(), MAX_SHOP_NAME);
    }

    /// #551: with every stall taken there's no stall to rent, and a citizen
    /// that found so waits a game hour before it reads the directory again.
    #[test]
    fn a_full_bazaar_has_no_stall_to_rent_until_it_looks_again() {
        let sign = || capitol_protocol::rest::ShopSign {
            name: "Ida Smith's ore".into(),
            owner_id: Uuid::nil(),
            owner_name: "Ida Smith".into(),
            owner_citizen: true,
            item_types: vec!["iron_ore".into()],
        };
        let stall = |id: &str, taken: bool| capitol_protocol::rest::BazaarStall {
            stall_id: id.into(),
            shop: taken.then(sign),
        };
        let full = BazaarDirectory {
            stalls: vec![stall("bazaar_stall_0", true), stall("bazaar_stall_1", true)],
        };
        assert_eq!(free_stall(&full), None);
        let one = BazaarDirectory {
            stalls: vec![stall("bazaar_stall_0", true), stall("bazaar_stall_1", false)],
        };
        assert_eq!(free_stall(&one), Some("bazaar_stall_1"));
        assert!(looks_again(None, 0.0), "it looks the first time");
        assert!(!looks_again(Some(2.0), 2.5), "not within the hour");
        assert!(looks_again(Some(2.0), 3.0), "and again after it");
    }

    /// #523: a citizen gives up an empty stall it has stopped using, once
    /// the rent is short or it hasn't stocked it in a day, and never one it
    /// has goods to stock it with.
    #[test]
    fn an_empty_stall_it_has_stopped_using_is_given_up() {
        let day = RENT_DAY_SECS;
        let hour = 3600;
        // Freshly rented and sold out: it may stock again soon
        assert_eq!(gives_up(2 * day - hour, hour, false), None);
        // Less than a day to run: stocking again would mean paying more
        assert_eq!(gives_up(day - hour, hour, false), Some("short"));
        // A day without stocking, on a stall paid ahead
        assert_eq!(gives_up(day + hour, day, false), Some("idle"));
        assert_eq!(gives_up(day + hour, day - 1, false), None);
        // Something spare to stock it with: it keeps it, however short
        assert_eq!(gives_up(hour, 2 * day, true), None);
        // Run out already: the sweep closes it
        assert_eq!(gives_up(0, 2 * day, false), None);
        assert_eq!(gives_up(-hour, 2 * day, false), None);
    }

    /// #523: signing off, a citizen gives up an empty stall it won't be on
    /// to stock before the rent runs out, and keeps one with goods on it.
    #[test]
    fn a_citizen_leaving_gives_up_an_empty_stall_it_wont_be_back_for() {
        let day = RENT_DAY_SECS;
        assert!(leaves(true, day, 0.0), "not on again before it lapses");
        assert!(leaves(true, day, STALL_BACK_SECS - 1.0), "on too little to stock it");
        assert!(!leaves(true, day, STALL_BACK_SECS), "back in time");
        assert!(!leaves(false, day, 0.0), "goods on the shelf sell while it's away");
        assert!(!leaves(true, 0, 0.0), "lapsed already");
        // A citizen on always is on the whole span
        assert!(!leaves(true, day, day as f64));
    }
}
