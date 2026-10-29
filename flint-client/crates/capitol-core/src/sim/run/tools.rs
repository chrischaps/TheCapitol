//! Tools bought, not made (Callings K2.1, plan D7). A tool crafter's spare
//! tools go to the hall and the bazaar as any citizen's spare goods do
//! ([`surplus::spare`](super::super::surplus::spare)). Here is the other
//! side: a citizen whose next step climbs toward a tool, its last one worn
//! out or never had, buys one instead when it has the Strands to spare, as
//! a person with a broken pickaxe would.
//!
//! Where it looks, in order:
//! - **the hall**, for the best buy anyone asks for: the cheapest a use
//!   left ([`tool_offer`]), since the book shows each tool's wear (#547),
//!   so an iron axe beats a stone one at their targets and a new stone one
//!   beats a worn iron one. Its buy asks for that tool's uses left, so it
//!   gets that one or a fresher one at the price (`SimBot::uses_wanted`);
//! - **a stall** whose sign lists a tool of the type, at most once a game
//!   hour ([`LOOK_AGAIN`]). There it sees each tool's wear before it buys,
//!   and takes the one cheapest a use left, at no more a use than the
//!   price it decided on for a new one;
//! - **a standing bid** at the hall when nobody sells one, so whoever makes
//!   them has someone to sell to: for the cheapest of the type at its
//!   worth, the one it would otherwise make, and what a tool crafter makes
//!   first for sale, taking one with at least half its uses
//!   ([`fewest_uses`](super::super::fewest_uses)). It climbs on meanwhile.
//!
//! A tool a standing bid bought waits in the market box. With one there of
//! the type, it goes and collects it rather than buy another: in the soak,
//! citizens bought two or three axes that way and sold the rest on.
//!
//! A citizen sells its freshest spare, at the hall or a stall, and keeps
//! its most worn to use up; either way it's priced by the share of its uses
//! left (`people::worn_price`), which both show (#547).
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
        let (tool_type, at_the_hall, boxed) = {
            let catalog = self.bot.state().catalog();
            let tool_type = plan::tool_wanted(catalog, holdings, errand.goal.as_ref()?)?;
            let market = self.bot.state().market();
            let ask = tool_offer(catalog, &tool_type, |item, life| market.book(item)?.cheapest_use(life, None));
            let boxed = self
                .boxed_tools
                .iter()
                .any(|t| catalog.item_type(t).and_then(|t| t.tool.as_ref()).is_some_and(|t| t.tool_type == tool_type));
            (tool_type, ask, boxed)
        };
        if boxed {
            return Some(Trade::Collect);
        }
        if let Some((item, ask, _)) = at_the_hall {
            let menu = tool_menu(holdings, item, ask, thrift, false)?;
            return match self.decide(Question::Buy { item: item.into() }, menu, 1, holdings)? {
                (d, Choice::Buy { item, grade, .. }) => Some(Trade::Buy {
                    item,
                    grade,
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
        // Nobody sells one: a bid for the cheapest at its worth, once. At
        // their targets iron is the better buy a use, but nobody sells iron
        // before the stone tier, and a bid for it would stand unfilled
        let (item, worth) = {
            let catalog = self.bot.state().catalog();
            let of_type = |item: &str| catalog.item_type(item).and_then(|t| t.tool.as_ref()).is_some_and(|t| t.tool_type == tool_type);
            if TOOLS.iter().any(|t| of_type(t) && self.bidding(t)) {
                return None;
            }
            TOOLS
                .into_iter()
                .filter(|t| of_type(t))
                .filter_map(|t| Some((t, self.worth(t)?)))
                .min_by_key(|&(_, worth)| worth)?
        };
        let menu = tool_menu(holdings, item, worth, thrift, true)?;
        match self.decide(Question::Buy { item: item.into() }, menu, 1, holdings)? {
            (d, Choice::Bid { item, grade, .. }) => Some(Trade::Bid {
                item,
                grade,
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
        let (item, ..) = {
            let catalog = self.bot.state().catalog();
            tool_offer(catalog, tool_type, |item, life| if listed(item) { Some((self.worth(item)?, life)) } else { None })?
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
