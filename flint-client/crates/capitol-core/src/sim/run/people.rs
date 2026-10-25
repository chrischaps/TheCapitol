//! A citizen among people (C1.5): answering trades, standing bids for what
//! it wants, and a stall for what it has spare that the hall doesn't take
//! (Callings K1.4: across its jobs, [`surplus`](super::super::surplus)).
//! Among other citizens it asks for trades too (#295). Everything
//! here follows the normal rules and pays the normal costs: the fee on a
//! bid, the rent on a stall (D2).

use std::time::{Duration, Instant};

use capitol_protocol::rest::PlaceOrder;
use capitol_protocol::rules::bazaar::{RENT_DAY_SECS, MAX_SHOP_NAME};
use capitol_protocol::rules::lately::Lately;
use capitol_protocol::rules::market::OrderSide;
use uuid::Uuid;

use super::super::plan::Holdings;
use super::super::report::Entry;
use super::super::{bid_menu, stall_menu, Trade, STALL_DAYS};
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

    /// What `item` is worth by the citizen's books.
    pub(super) fn worth(&self, item: &str) -> Option<i64> {
        meet::worth(self.bot.state(), item)
    }

    /// Whether it has a standing bid up for `item`.
    fn bidding(&self, item: &str) -> bool {
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
        let keeps = self.bot.state().bazaar().my_shop().is_some();
        let menu = stall_menu(holdings, item, spare, self.worth(item), keeps)?;
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

    /// At the hall: takes down standing bids that have stood a game hour,
    /// and asks again about a bid for each thing it wants (the job at
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
            .filter(|o| o.side == OrderSide::Buy && now - o.created_at > life)
            .map(|o| o.id)
            .collect();
        for id in stale {
            match self.bot.cancel_order(id) {
                Ok(_) => {}
                // It filled meanwhile
                Err(BotError::Rest { source, .. }) if source.status().is_some_and(|s| s < 500) => {}
                Err(e) => return Err(e),
            }
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
        self.post_wants()
    }

    /// Leaves a bid for `quantity` of `item` at `price` on the book, to
    /// stand until it fills or goes stale. The bot must be at the hall.
    pub(super) fn bid(&mut self, item: &str, quantity: i32, price: i64) -> Result<(), BotError> {
        let order = self.bot.place_order(PlaceOrder {
            side: OrderSide::Buy,
            item_type: item.to_string(),
            price,
            quantity,
            item_id: None,
            qualities: None,
            min_grade: None,
            grade_rule: None,
        })?;
        self.mind.placed(order.id);
        self.board.bid(item, price);
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
                let free = directory
                    .stalls
                    .iter()
                    .find(|s| s.shop.is_none())
                    .map(|s| s.stall_id.clone())
                    .ok_or_else(|| BotError::Failed("every stall at the bazaar is taken".into()))?;
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
        stacks.sort_by_key(|s| std::cmp::Reverse(s.quantity));
        let mut left = quantity;
        for stack in stacks {
            if left <= 0 {
                break;
            }
            let listings = self.bot.state().bazaar().my_shop().map_or(0, |s| s.listings.len());
            let n = left.min(stack.quantity);
            let listing = bazaar::stock(&stack, n, price, listings).map_err(|e| BotError::Failed(e.message()))?;
            self.bot.stock(listing)?;
            left -= n;
        }
        self.mind.stocked(item);
        // Give the stall a moment to show on the bazaar's signs
        self.bot.idle(Duration::from_millis(100))?;
        Ok(())
    }
}

/// A citizen's shop: "Percy Sawyer's logs", shortened to fit.
fn shop_name(name: &str, item: &str) -> String {
    let goods = match item {
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
        let long = shop_name("Bartholomew Fitzwilliam-Cartwright", "plank");
        assert_eq!(long.chars().count(), MAX_SHOP_NAME);
    }
}
