//! A merchant (C1.6): it earns its float as a fiber gatherer does, banking
//! fiber at the exchange, then makes a market in its goods at the hall with
//! [`market_maker::quotes`], sized to what it holds. Its target grows with
//! its wealth ([`market_maker::share`]). Nothing is granted (D2): its stock is
//! what its bids bought, and its Strands what it banked and sold.
//!
//! Whether to make a market is a decision ([`Question::Market`]), asked at
//! each turn of its work: make one with its float, or go on with its errand.
//! A spell at the hall lasts a game hour, then it's asked again. It breaks
//! off for a trade request, which the loop answers, its town stopping, or
//! dusk (C1.2): a merchant keeps its persona's day like anyone else.
//! Its orders stay up between spells, as a market maker's do.

use std::time::{Duration, Instant};

use super::super::plan::Holdings;
use super::super::{merchant_items, Errand, Trade};
use super::SimBot;
use crate::bot::BotError;
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

/// A merchant's goods, and its market once it has made one.
pub(super) struct Merchant {
    items: Vec<String>,
    maker: Option<Maker>,
    /// Whether it was making a market at its last turn.
    making: bool,
}

impl Merchant {
    /// A merchant dealing in the goods `seed` picks ([`merchant_items`]).
    pub(super) fn new(seed: u64) -> Merchant {
        Merchant {
            items: merchant_items(seed).into_iter().map(String::from).collect(),
            maker: None,
            making: false,
        }
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
        make.then_some(Trade::MakeMarket)
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
