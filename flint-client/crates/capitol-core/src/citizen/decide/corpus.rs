//! A town's worth of situations, made up (C3.3): every role's work, sell,
//! buy, bid, stall and free-time questions over a spread of holdings, books
//! and temperaments. A fresh live town asks only work and bank questions
//! (#294), so this is what a new layer is measured on until the logs hold
//! the rest. `capitol-town corpus` writes it as a rules-only citizen's log.

use std::path::Path;
use std::sync::{Arc, Mutex};

use super::log::{self, Fate, Log};
use super::{Cascade, Choice, Held, Mind, Place, Question, Quote, Situation, Temperament, When, Who};
use crate::catalog::Catalog;
use crate::sim::plan::Holdings;
use crate::sim::{Role, MIX};

/// The name the corpus's citizen logs under.
pub const CITIZEN: &str = "Percy Sawyer";

/// Every role's work, sell, buy, bid and stall questions over a spread
/// of holdings, books and temperaments: what a town asks in a day.
pub fn situations() -> Vec<Situation> {
    round(0)
}

/// `rounds` of [`situations`], each with other temperaments, and its books
/// and stock at another scale: a bigger town, to measure a layer's
/// calibration on.
pub fn town(rounds: u32) -> Vec<Situation> {
    (0..rounds.max(1)).flat_map(round).collect()
}

/// Round 0 is [`situations`].
fn round(r: u32) -> Vec<Situation> {
    let scale = 1 + (r % 3) as i64;
    let catalog = Catalog::fallback();
    let mut out = Vec::new();
    let mut n = 0u32;
    for (role, _) in MIX {
        for (strands, fiber, product, last) in [(0, 0, 0, None), (40, 120, 12, Some(3)), (300, 260, 45, Some(9)), (900, 30, 80, Some(1))] {
            n += 1;
            let t = |k: u32| ((n * 37 + k * 11 + r * 53 + r * k * 7) % 10) as f32 / 9.0;
            let (product, last) = (product * scale as i32, last.map(|l: i64| l * scale));
            let temperament = Temperament {
                thrift: t(1),
                risk: t(2),
                sociability: t(3),
                patience: t(4),
            };
            let mut h = Holdings {
                strands,
                deed_cost: 250,
                ..Holdings::default()
            };
            h.items.insert("fiber".into(), fiber);
            if let Some(p) = role.product() {
                h.items.insert(p.into(), product);
            }
            let quote = |item: &str| Quote {
                item: item.into(),
                bid: last.map(|l: i64| l - 1),
                ask: last.map(|l: i64| l + 2),
                last,
            };
            let market: Vec<Quote> = ["log", "plank", "iron_ore", "iron_ingot", "brick", "stone"].iter().map(|i| quote(i)).collect();
            let s = |question, menu: Vec<Choice>, carry_on| Situation {
                who: Who {
                    name: format!("Citizen {n}"),
                    role: role.slug().into(),
                    temperament,
                },
                at: Place {
                    x: 6000.0 + n as f64 * 13.7,
                    y: 6000.0 - n as f64 * 3.1,
                    zone: "urban".into(),
                },
                when: When {
                    time_of_day: Some(0.3 + n as f64 * 0.01),
                    hours: n as f64 * 0.37,
                },
                holdings: (&h).into(),
                market: market.clone(),
                question,
                menu,
                carry_on: Some(carry_on),
            };
            let Ok(errands) = role.errands(&catalog, &h) else { continue };
            out.push(s(Question::Work, errands.iter().map(|e| e.choice(&catalog)).collect(), 0));
            out.push(s(Question::FreeTime, vec![Choice::Rest, Choice::Hall], 0));
            if let Some(p) = role.product() {
                let goal = errands.first().and_then(|e| e.goal.as_ref());
                if let Some(menu) = role.sell_menu(&h, goal, last.map(|l| l - 1), last.map(|l| l + 2)) {
                    out.push(s(Question::Sell { item: p.into() }, menu, 1));
                }
                if let Some(menu) = role.stall_menu(&h, last, false) {
                    out.push(s(Question::Sell { item: p.into() }, menu, 1));
                }
            }
            for e in &errands {
                if let Some(menu) = role.buy_menu(&h, &e.step, last.map(|l| l + 2)) {
                    let item = menu.iter().find_map(|c| match c {
                        Choice::Buy { item, .. } => Some(item.clone()),
                        _ => None,
                    });
                    out.push(s(Question::Buy { item: item.unwrap_or_default() }, menu, 1));
                }
            }
            for item in role.needs() {
                if let Some(menu) = role.bid_menu(&h, item, last, false) {
                    out.push(s(Question::Buy { item: item.to_string() }, menu, 1));
                }
            }
        }
    }
    out
}

/// Writes `situations` to `path` as a rules-only citizen's log, each
/// decision done.
pub fn record(path: &Path, situations: &[Situation]) -> std::io::Result<()> {
    let log = Log::open(path, CITIZEN, Role::Builder.slug(), &["rules".into()], log::LOG_LIMIT, log::LOG_KEEP)?;
    let mut mind = Mind::new(Cascade::rules(), Temperament::default()).with_log(Arc::new(Mutex::new(log)));
    for s in situations {
        let mark = mind.mark();
        mind.ask(s);
        mind.settle(mark, &Held::default(), |_| Fate::Done);
    }
    Ok(())
}
