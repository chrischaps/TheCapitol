//! Tools bought, not made (Callings K2.1, plan D7). A tool crafter's spare
//! tools go to the hall and the bazaar as any citizen's spare goods do
//! ([`surplus::spare`](super::super::surplus::spare)). Here is the other
//! side: a citizen whose next step climbs toward a tool, its last one worn
//! out or never had, buys one instead when it has the Strands to spare, as
//! a person with a broken pickaxe would.
//!
//! Where it looks, in order:
//! - **the hall**, for the best buy anyone asks for: the cheapest a use at
//!   its full life ([`tool_offer`]), so an iron axe beats a stone one at
//!   their targets;
//! - **a stall** whose sign lists a tool of the type, at most once a game
//!   hour ([`LOOK_AGAIN`]). There it sees each tool's wear before it buys,
//!   and takes the one cheapest a use left, at no more a use than the
//!   price it decided on for a new one;
//! - **a standing bid** at the hall when nobody sells one, so whoever makes
//!   them has someone to sell to. It climbs on meanwhile.
//!
//! The hall's book doesn't show a tool's wear (#547), so what a citizen
//! sells there is its freshest spare and it keeps its most worn to use up.
//!
//! A tool crafter makes its own: it's its trade.

use capitol_protocol::rest::BuyListing;

use super::super::plan::{self, Holdings};
use super::super::report::Entry;
use super::super::{kept, per_use, tool_menu, tool_offer, Craft, Errand, Role, Trade, TOOLS};
use super::SimBot;
use crate::bot::BotError;
use crate::citizen::decide::{Choice, Question};

/// Game seconds between looks at the bazaar's signs for a tool: an hour.
const LOOK_AGAIN: f64 = 3600.0;

impl SimBot {
    /// Asks the mind whether to buy the tool `errand`'s next step climbs
    /// toward (Callings K2.1), at the hall, at a stall, or with a standing
    /// bid: a citizen's question. `None`: it climbs, as it would have.
    pub(super) fn choose_tool(&mut self, holdings: &Holdings, errand: &Errand) -> Option<Trade> {
        if !self.people || self.job() == Role::Crafter(Craft::Tools) {
            return None;
        }
        let thrift = self.mind.temperament.thrift;
        let (tool_type, at_the_hall) = {
            let catalog = self.bot.state().catalog();
            let tool_type = plan::tool_wanted(catalog, holdings, errand.goal.as_ref()?)?;
            let ask = tool_offer(catalog, &tool_type, |item| self.best(item).1);
            (tool_type, ask)
        };
        if let Some((item, ask)) = at_the_hall {
            let menu = tool_menu(holdings, item, ask, thrift, false)?;
            return match self.decide(Question::Buy { item: item.into() }, menu, 1, holdings)? {
                (d, Choice::Buy { item, .. }) => Some(Trade::Buy {
                    item,
                    quantity: 1,
                    price: d.price?,
                }),
                _ => None,
            };
        }
        if let Some((stall, item)) = self.tool_stall(&tool_type) {
            let menu = tool_menu(holdings, item, self.worth(item)?, thrift, false)?;
            return match self.decide(Question::Buy { item: item.into() }, menu, 1, holdings)? {
                (d, Choice::Buy { item, .. }) => Some(Trade::Shop {
                    stall,
                    item,
                    price: d.price?,
                }),
                _ => None,
            };
        }
        // Nobody sells one: a bid for the best buy at its worth, once
        let (item, worth) = {
            let catalog = self.bot.state().catalog();
            let of_type = |item: &str| catalog.item_type(item).and_then(|t| t.tool.as_ref()).is_some_and(|t| t.tool_type == tool_type);
            if TOOLS.iter().any(|t| of_type(t) && self.bidding(t)) {
                return None;
            }
            tool_offer(catalog, &tool_type, |item| self.worth(item))?
        };
        let menu = tool_menu(holdings, item, worth, thrift, true)?;
        match self.decide(Question::Buy { item: item.into() }, menu, 1, holdings)? {
            (d, Choice::Bid { item, .. }) => Some(Trade::Bid {
                item,
                quantity: 1,
                price: d.price?,
            }),
            _ => None,
        }
    }

    /// A stall someone else keeps whose sign lists a tool of `tool_type`,
    /// and that tool: the best buy at its worth among those listed. Read
    /// from the bazaar's directory, from anywhere, at most once a game
    /// hour.
    fn tool_stall(&mut self, tool_type: &str) -> Option<(String, &'static str)> {
        let now = self.now();
        if self.tools_looked.is_some_and(|at| now - at < LOOK_AGAIN / 3600.0) {
            return None;
        }
        self.tools_looked = Some(now);
        let directory = self.bot.bazaar().ok()?;
        let me = self.bot.state().me();
        let signs: Vec<_> = directory
            .stalls
            .into_iter()
            .filter_map(|s| Some((s.stall_id, s.shop?)))
            .filter(|(_, sign)| Some(sign.owner_id) != me)
            .collect();
        let listed = |item: &str| signs.iter().any(|(_, sign)| sign.item_types.iter().any(|t| t == item));
        let (item, _) = {
            let catalog = self.bot.state().catalog();
            tool_offer(catalog, tool_type, |item| if listed(item) { self.worth(item) } else { None })?
        };
        let (stall, _) = signs.into_iter().find(|(_, sign)| sign.item_types.iter().any(|t| t == item))?;
        Some((stall, item))
    }

    /// Walks to `stall`, looks over its `item`s and buys the one cheapest
    /// a use left, if that's no dearer a use than `price` for a new one and
    /// it can pay with what it keeps back. Nothing that suits: it looked,
    /// as a person would, and goes back to its work.
    pub(super) fn buy_at_a_stall(&mut self, stall: &str, item: &str, price: i64) -> Result<(), BotError> {
        let shop = self.bot.browse(stall)?;
        let holdings = self.holdings();
        let budget = holdings.strands - kept(&holdings);
        let life = self
            .bot
            .state()
            .catalog()
            .item_type(item)
            .and_then(|t| t.tool.as_ref())
            .map_or(1, |t| t.max_durability);
        let most = per_use(price, life);
        let pick = shop
            .listings
            .iter()
            .filter(|l| l.item_type == item && l.price <= budget)
            .map(|l| (l, per_use(l.price, l.durability.unwrap_or(life))))
            .filter(|&(_, cost)| cost <= most)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(l, _)| l.id);
        let Some(listing) = pick else {
            return Ok(());
        };
        let bought = self.bot.buy(
            listing,
            BuyListing {
                quantity: 1,
                qualities: None,
            },
        )?;
        self.entries.push(Entry::Traded {
            item: item.to_string(),
            sold: false,
            quantity: 1,
            price: bought.paid,
            at: self.now(),
        });
        Ok(())
    }
}
