//! A citizen among people (C1.5): answering trades, standing bids for what
//! its trade needs, and a stall for what it makes more of than the hall
//! takes. Everything here follows the normal rules and pays the normal
//! costs: the fee on a bid, the rent on a stall (D2).

use std::time::Duration;

use capitol_protocol::rest::PlaceOrder;
use capitol_protocol::rules::bazaar::{RENT_DAY_SECS, MAX_SHOP_NAME};
use capitol_protocol::rules::market::OrderSide;

use super::super::plan::Holdings;
use super::super::{Trade, STALL_DAYS};
use super::{Action, SimBot};
use crate::bazaar;
use crate::bot::BotError;
use crate::citizen::decide::log::Fate;
use crate::citizen::decide::{Choice, Question, Who};
use crate::citizen::meet::{self, Answering, Met};
use crate::citizen::town::{Standing, Tether};

/// A standing bid is taken down and thought over again after this many
/// game seconds, at the citizen's next visit to the hall.
const BID_LIFE: f64 = 3600.0;

impl SimBot {
    /// Answers the trade someone asked for, where the citizen stands.
    pub(super) fn meet(&mut self, town: Option<&Tether>) -> Result<(), BotError> {
        let me = Answering {
            who: Who {
                name: self.bot.name().to_string(),
                role: self.role.slug().to_string(),
                temperament: self.mind.temperament,
            },
            product: self.role.product(),
            hours: self.now(),
        };
        if let (Some(town), crate::state::Trade::Incoming { name, .. }) = (town, self.bot.state().trade()) {
            let doing = format!("trading with {name}");
            town.health(|h| h.doing(Standing::Working, &doing));
        }
        let mark = self.mind.mark();
        let met = meet::answer(&mut self.bot, &mut self.mind, &me);
        self.journal();
        // The trade's last decision, which it came to (C3.1)
        let fate = match &met {
            Ok(Met::Traded { .. }) => Fate::Traded,
            Ok(Met::Declined { .. }) => Fate::Declined,
            Ok(Met::Ended { reason, .. }) => Fate::Ended { reason: reason.clone() },
            Err(e) => Fate::Failed { why: e.to_string() },
        };
        let after = meet::held(self.bot.state());
        self.mind.settle(mark, &after, |_| fate.clone());
        let met = met?;
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

    /// Asks the mind whether to stock a stall with `product`: a citizen's
    /// question when nobody bids for what it makes at the hall.
    pub(super) fn choose_stall(&mut self, holdings: &Holdings, product: &str) -> Option<Trade> {
        if !self.people {
            return None;
        }
        let keeps = self.bot.state().bazaar().my_shop().is_some();
        let menu = self.role.stall_menu(holdings, self.worth(product), keeps)?;
        let question = Question::Sell { item: product.into() };
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
    /// next step would gather and nobody is selling: a citizen's question.
    pub(super) fn choose_bid(&mut self, holdings: &Holdings, item: &str) -> Option<Trade> {
        if !self.people {
            return None;
        }
        let menu = self.role.bid_menu(holdings, item, self.worth(item), self.bidding(item))?;
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
    /// and asks again about a bid for each thing its trade needs, so
    /// whoever makes it has someone to sell to.
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
        for item in self.role.needs() {
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
        Ok(())
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
