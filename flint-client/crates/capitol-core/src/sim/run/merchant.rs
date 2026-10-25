//! Trading (C1.6, Callings K2.2): a citizen on a trading stint makes a
//! market in its goods at the hall with [`market_maker::quotes`], sized to
//! what it holds. Its target grows with its wealth ([`market_maker::share`]).
//! Nothing is granted (D2): its stock is what its bids bought, and its
//! Strands what it banked and sold.
//!
//! Since K2.2 trading is a leaning, not a role. Every citizen has its goods
//! ([`trading_goods`], from its calling's leanings), and trading is on its
//! stint menu once it has its float ([`SimBot::trades`]), or always when
//! it's its favourite (`mint --role merchant`). A favourite short of its
//! float earns it on its trading stint as a fiber gatherer does, banking
//! fiber at the exchange, as C1.6's merchants did; anyone else earns it at
//! its other jobs first.
//!
//! Whether to make a market is a decision ([`Question::Market`]), asked at
//! each turn of its work: make one with its float, or go on with its errand.
//! A spell at the hall lasts a game hour, then it's asked again. It breaks
//! off for a trade request, which the loop answers, its town stopping, or
//! dusk (C1.2): a trader keeps its persona's day like anyone else.
//! Its orders stay up between spells and between stints, as a market
//! maker's do. Between spells, what its bids bought and it holds in hand it
//! may put on its stall at the bazaar ([`SimBot::stall_its_goods`]).

use std::time::{Duration, Instant};

use super::super::plan::Holdings;
use super::super::stint::trade_line;
use super::super::{stall_menu, trading_goods, Errand, Role, Trade};
use super::SimBot;
use crate::bot::BotError;
use crate::citizen::calling::Calling;
use crate::citizen::decide::rules::enough_float;
use crate::citizen::decide::{Choice, Question};
use crate::citizen::meet;
use crate::citizen::schedule::Phase;
use crate::citizen::town::{Standing, Tether};
use crate::market_maker::{self, Maker, MakerConfig};

/// How long a spell of making a market lasts, in game seconds, before the
/// merchant is asked again.
const SPELL: f64 = 3600.0;
/// How often, in wall time, a merchant's limits follow its wealth.
const RESIZE: Duration = market_maker::REQUOTE_EVERY;
/// Strands a merchant keeps back from its quotes, for the fees.
const RESERVE: i64 = 20;

/// A citizen's goods to trade in, its market once it has made one, and its
/// line for trading.
pub(super) struct Merchant {
    items: Vec<String>,
    maker: Option<Maker>,
    /// Whether it was making a market at its last turn.
    making: bool,
    /// The town's `[trading] line` (K2.2), which its patience scales.
    line: i64,
}

impl Merchant {
    /// A trader dealing in the goods its `calling` and `seed` pick
    /// ([`trading_goods`]), with the town's trading `line`.
    pub(super) fn new(calling: &Calling, seed: u64, line: i64) -> Merchant {
        Merchant {
            items: trading_goods(calling, seed).into_iter().map(String::from).collect(),
            maker: None,
            making: false,
            line,
        }
    }

    /// Whether it makes a market in `item`: its stock, never spare.
    pub(super) fn deals(&self, item: &str) -> bool {
        self.items.iter().any(|i| i == item)
    }

    fn config(&self) -> MakerConfig {
        MakerConfig {
            items: self.items.clone(),
            reserve: RESERVE,
            ..MakerConfig::default()
        }
    }
}

impl SimBot {
    /// Whether trading is on its stint menu (Callings K2.2): always for its
    /// favourite, which earns its float on the stint if it must, and for
    /// anyone else leaning to it once it has its float, as the rules weigh
    /// one ([`enough_float`]: half of it while it's making a market).
    pub(super) fn trades(&self) -> bool {
        let Some(merchant) = &self.merchant else { return false };
        if self.calling.favourite == Some(Role::Merchant) {
            return true;
        }
        self.calling.share(Role::Merchant) > 0.0 && self.float() as f64 >= enough_float(&self.mind.temperament, merchant.making)
    }

    /// Its line for trading (K2.2): past it, wealth raises trading's share of
    /// its calling. 0 for a bot that doesn't trade.
    pub(super) fn trade_line(&self) -> i64 {
        self.merchant.as_ref().map_or(0, |m| trade_line(m.line, self.mind.temperament.patience))
    }

    /// Whether `item` is its market's stock, and so never spare: a trading
    /// favourite's always, anyone else's while it's on a trading stint.
    pub(super) fn deals(&self, item: &str) -> bool {
        let trading = self.calling.favourite == Some(Role::Merchant) || self.job() == Role::Merchant;
        trading && self.merchant.as_ref().is_some_and(|m| m.deals(item))
    }

    /// What the merchant has to make a market with, in Strands: its
    /// Strands, those on its bids included, and its goods, in hand, in the
    /// market box and on its asks, at its maker's references, or else its
    /// books, or else the catalog's target price.
    fn float(&self) -> i64 {
        let Some(merchant) = &self.merchant else { return 0 };
        let goods: i64 = merchant
            .items
            .iter()
            .map(|item| {
                let price = merchant
                    .maker
                    .as_ref()
                    .and_then(|m| m.reference(item))
                    .or_else(|| meet::worth(self.bot.state(), item))
                    .or_else(|| self.bot.state().catalog().target_price(item))
                    .unwrap_or(0);
                market_maker::position(&self.bot, item) as i64 * price
            })
            .sum();
        market_maker::all_strands(&self.bot) + goods
    }

    /// Asks the mind whether to make a market now, or go on with `errand`.
    /// `None`: the errand.
    pub(super) fn choose_market(&mut self, holdings: &Holdings, errand: &Errand) -> Option<Trade> {
        let catalog = self.bot.state().catalog().clone();
        let float = self.float();
        let merchant = self.merchant.as_ref()?;
        let making = merchant.making;
        let menu = vec![
            Choice::MakeMarket {
                items: merchant.items.clone(),
                float,
                making,
            },
            errand.choice(&catalog),
        ];
        let make = matches!(
            self.decide(Question::Market, menu, if making { 0 } else { 1 }, holdings),
            Some((_, Choice::MakeMarket { .. }))
        );
        if let Some(m) = self.merchant.as_mut() {
            m.making = make;
        }
        if make {
            return Some(Trade::MakeMarket);
        }
        self.stall_its_goods(holdings)
    }

    /// Between spells at the hall, a stall (K2.2): of its goods, what it
    /// holds most of in hand, bought by its bids, priced from its maker's
    /// reference, or else its books, or else the catalog's target. Asked as a
    /// stall question (C1.5), from half of [`STALL_AT`](super::super::STALL_AT).
    fn stall_its_goods(&mut self, holdings: &Holdings) -> Option<Trade> {
        if !self.people {
            return None;
        }
        let merchant = self.merchant.as_ref()?;
        let (item, held) = merchant
            .items
            .iter()
            .map(|i| (i.clone(), holdings.count(i)))
            .max_by_key(|&(_, n)| n)
            .filter(|&(_, n)| n > 0)?;
        let worth = merchant
            .maker
            .as_ref()
            .and_then(|m| m.reference(&item))
            .or_else(|| meet::worth(self.bot.state(), &item))
            .or_else(|| self.bot.state().catalog().target_price(&item));
        let keeps = self.bot.state().bazaar().my_shop().is_some();
        let menu = stall_menu(holdings, &item, held, worth, keeps)?;
        match self.decide(Question::Sell { item: item.clone() }, menu, 1, holdings)? {
            (d, Choice::Stock { item, .. }) => Some(Trade::Stock {
                item,
                quantity: d.quantity?,
                price: d.price?,
            }),
            _ => None,
        }
    }

    /// A spell of making a market at the hall: to the hall, its maker
    /// started the first time, then quoting until the spell is up, someone
    /// asks to trade, or the town stops. Each [`RESIZE`] its limits follow
    /// its wealth.
    pub(super) fn make_market(&mut self, town: Option<&Tether>) -> Result<(), BotError> {
        let Some(merchant) = self.merchant.as_mut() else {
            return Err(BotError::Failed("only a merchant makes a market".into()));
        };
        let first = merchant.maker.is_none();
        let mut maker = merchant.maker.take().unwrap_or_else(|| Maker::new(merchant.config()));
        let result = self.quote(&mut maker, town, first);
        if let Some(merchant) = self.merchant.as_mut() {
            merchant.maker = Some(maker);
        }
        result
    }

    fn quote(&mut self, maker: &mut Maker, town: Option<&Tether>, first: bool) -> Result<(), BotError> {
        self.bot.go_to_exchange()?;
        if first {
            maker.start(&mut self.bot)?;
            // The measure C1.6 asks for: how long a merchant that started
            // with nothing took to earn its float
            eprintln!(
                "{}: making a market in {} after {:.2} game hours, with a float of {} Strands",
                self.bot.name(),
                maker.config().items.join(", "),
                self.now(),
                self.float()
            );
        }
        let stopped = || town.is_some_and(Tether::stopped);
        let end = Instant::now() + self.wall(SPELL);
        let day_over = |me: &SimBot| matches!(me.phase(), Some(Phase::Home | Phase::Night));
        while Instant::now() < end && !stopped() && !self.bot.asked_to_trade() && !day_over(self) {
            let wealth = (self.float() - RESERVE).max(0);
            let items = maker.config().items.clone();
            for item in &items {
                if let Some(reference) = maker.reference(item) {
                    maker.limit(item, market_maker::share(wealth, reference, items.len()));
                }
            }
            if let Some(town) = town {
                let doing = format!("making a market in {} with {wealth} S", items.join(", "));
                town.health(|h| h.doing(Standing::Working, &doing));
            }
            let slice = end.min(Instant::now() + RESIZE);
            maker.run_while(&mut self.bot, Some(slice), |bot| !stopped() && !bot.asked_to_trade())?;
            self.journal();
        }
        Ok(())
    }
}
