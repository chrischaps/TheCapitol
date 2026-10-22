//! The Observatory's samples (D0.2): the world every ten seconds, for the
//! console's charts.
//!
//! A [`Sample`] is what changed between two [`Ledger`]s a sample apart, plus
//! what the ledger doesn't hold: the market's and the bazaar's trades, who is
//! in the world, and how far the event bus has backed up. The backend's
//! sampler takes one every [`SAMPLE_SECS`], keeps a day of them in memory and
//! writes one a minute to `metrics_samples`.
//!
//! Every flow in a sample is since the sample before, so a run of samples
//! adds up to what [`reconcile`](crate::ledger::reconcile) says about the
//! ledgers at its ends ([`tally`]).
//!
//! Since D0.3, `GET /admin/stream` sends each sample live as a [`Feed`], with
//! the annotations and the game's fills, deeds, Gazettes and warnings, and
//! `GET /admin/series` serves them back as a [`Series`].
//!
//! Since D2.5 each sample says which watches were firing as it was taken
//! (`crate::watch`).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::annotation::Annotation;
use crate::ledger::{Ledger, LoadWindow};
use crate::rest::MarketTrade;
use crate::watch::Watch;

/// Seconds between samples
pub const SAMPLE_SECS: u64 = 10;
/// The samples kept in memory: a day of them
pub const RING_SAMPLES: usize = (24 * 60 * 60 / SAMPLE_SECS) as usize;
/// Every how many samples one is written to `metrics_samples`: one a minute
pub const STORE_EVERY: u64 = 60 / SAMPLE_SECS;
/// Days a stored sample is kept
pub const KEEP_DAYS: i64 = 30;

/// The world over one sample's window: from the sample before, at
/// `at_ms - since_ms`, to `at_ms`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Sample {
    /// When it was taken, by the server's clock (Unix ms)
    pub at_ms: i64,
    /// When the server started: flows count again from a new boot
    pub booted_ms: i64,
    /// The window's length, in ms: usually [`SAMPLE_SECS`], longer when a
    /// sample was missed
    pub since_ms: i64,
    /// The tick, its phases and the loops beside it, over the window
    pub load: LoadWindow,
    /// Strands held now
    pub strands: Strands,
    /// What the audit trail says moved Strands over the window, by
    /// transaction type
    pub strand_flows: BTreeMap<String, i64>,
    /// Every unit in the world now, by item type
    pub units: BTreeMap<String, i64>,
    /// Units made (positive) and destroyed (negative) over the window, by
    /// item type, then by flow
    pub unit_flows: BTreeMap<String, BTreeMap<String, i64>>,
    /// Whether both ends' units and flows were read in a quiet moment
    pub units_settled: bool,
    /// The market's fills over the window, by item type
    pub market: BTreeMap<String, Trades>,
    /// The bazaar's sales over the window, by item type
    pub bazaar: BTreeMap<String, Trades>,
    pub players: Players,
    pub events: EventBus,
    /// The watches firing as it was taken (D2.5); a sample stored before
    /// them reads with none
    #[serde(default)]
    pub watches: Vec<Watch>,
}

/// Strands by who holds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Strands {
    /// Citizens' (C0.1)
    pub citizens: i64,
    /// People's, not counting admins
    pub people: i64,
    /// Admins': a market maker's float, say, apart from the people's
    pub admins: i64,
    /// Held for open buy orders
    pub escrow: i64,
}

impl Strands {
    /// Every Strand in the world
    pub fn total(&self) -> i64 {
        self.citizens + self.people + self.admins + self.escrow
    }
}

/// Trades in one item over a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Trades {
    /// Fills or sales
    pub trades: u64,
    /// Units that changed hands
    pub units: i64,
    /// Strands paid for them
    pub strands: i64,
}

impl Trades {
    /// The volume-weighted mean price, Strands a unit; `None` with no trades
    pub fn vwap(&self) -> Option<f64> {
        (self.units > 0).then(|| self.strands as f64 / self.units as f64)
    }
}

/// Who is in the world now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Players {
    pub people: u64,
    pub citizens: u64,
}

/// The game's event bus now (`GameState::event_sender`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct EventBus {
    /// Events the slowest receiver hasn't read yet. At
    /// [`capacity`](Self::capacity) it starts to miss them, and its socket
    /// resyncs.
    pub queued: u64,
    pub capacity: u64,
}

impl Sample {
    /// The part of a sample two ledgers make: the load, the Strands and the
    /// units, and their flows between. `admins` are the players whose
    /// accounts are admins'. The trades, the players and the bus are left
    /// empty for the caller, which has them.
    pub fn between(start: &Ledger, end: &Ledger, admins: &HashSet<Uuid>) -> Sample {
        let mut strands = Strands { escrow: end.escrow_strands, ..Strands::default() };
        let before: BTreeMap<Uuid, &BTreeMap<String, i64>> =
            start.accounts.iter().map(|a| (a.player_id, &a.flows)).collect();
        let mut strand_flows = BTreeMap::new();
        for account in &end.accounts {
            let class = if account.citizen {
                &mut strands.citizens
            } else if admins.contains(&account.player_id) {
                &mut strands.admins
            } else {
                &mut strands.people
            };
            *class += account.strands;
            let was = before.get(&account.player_id);
            for (kind, sum) in &account.flows {
                let moved = sum - was.and_then(|f| f.get(kind)).copied().unwrap_or(0);
                if moved != 0 {
                    *strand_flows.entry(kind.clone()).or_insert(0) += moved;
                }
            }
        }
        // A new boot counts the flows from nothing again
        let same_boot = start.booted_ms == end.booted_ms;
        let mut unit_flows: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
        for (item, by_flow) in &end.unit_flows {
            for (flow, n) in by_flow {
                let was = if same_boot {
                    start.unit_flows.get(item).and_then(|f| f.get(flow)).copied().unwrap_or(0)
                } else {
                    0
                };
                if n - was != 0 {
                    unit_flows.entry(item.clone()).or_default().insert(flow.clone(), n - was);
                }
            }
        }
        let since_ms = end.at_ms - start.at_ms;
        Sample {
            at_ms: end.at_ms,
            booted_ms: end.booted_ms,
            since_ms,
            load: LoadWindow::between(&start.load, &end.load, since_ms as f64 / 1000.0),
            strands,
            strand_flows,
            units: end.units.clone(),
            unit_flows,
            units_settled: start.units_settled && end.units_settled,
            market: BTreeMap::new(),
            bazaar: BTreeMap::new(),
            players: Players::default(),
            events: EventBus::default(),
            watches: Vec::new(),
        }
    }
}

/// The Gini of `balances`: 0 when everyone holds the same, towards 1 as one
/// holds it all. A debt counts as nothing. `None` for fewer than two, or
/// when nobody holds anything.
pub fn gini(balances: &[i64]) -> Option<f64> {
    let mut held: Vec<i64> = balances.iter().map(|b| (*b).max(0)).collect();
    let total: i64 = held.iter().sum();
    if held.len() < 2 || total == 0 {
        return None;
    }
    held.sort_unstable();
    let n = held.len() as f64;
    // Σ (2i - n - 1) xᵢ over n Σx, with i from 1 in rising order
    let weighted: f64 = held.iter().enumerate().map(|(i, x)| (2.0 * (i as f64 + 1.0) - n - 1.0) * *x as f64).sum();
    Some(weighted / (n * total as f64))
}

/// A run of samples' flows added up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tally {
    /// Strands by transaction type, as `reconcile`'s `strands.flows`
    pub strand_flows: BTreeMap<String, i64>,
    /// Units by flow over every item, as `reconcile`'s `units.flows`
    pub unit_flows: BTreeMap<String, i64>,
}

/// What `samples` add up to. Taken one after another from the same
/// accounts, that's what `reconcile` says about the ledgers at the ends.
pub fn tally<'a>(samples: impl IntoIterator<Item = &'a Sample>) -> Tally {
    let mut tally = Tally::default();
    for sample in samples {
        for (kind, n) in &sample.strand_flows {
            *tally.strand_flows.entry(kind.clone()).or_insert(0) += n;
        }
        for (flow, n) in sample.unit_flows.values().flatten() {
            *tally.unit_flows.entry(flow.clone()).or_insert(0) += n;
        }
    }
    tally.strand_flows.retain(|_, n| *n != 0);
    tally.unit_flows.retain(|_, n| *n != 0);
    tally
}

/// One thing on `GET /admin/stream` (D0.3), sent as a server-sent event
/// named for its `type`, with the whole of it, tag and all, as its data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Feed {
    /// A new sample, as it went into the ring
    Sample(Box<Sample>),
    /// A new annotation, as it went into `admin_events`
    Annotation(Annotation),
    /// A fill on the market
    Fill { trade: MarketTrade },
    /// A deed changed hands: claimed, released or foreclosed
    Deed {
        terrain_id: String,
        change: DeedChange,
        /// Who claimed it, or whose it was; `None` for a release, which
        /// doesn't say
        owner_id: Option<Uuid>,
    },
    /// A day's Gazette came out
    Gazette { day: i64, number: i64, headline: String },
    /// Something went wrong for a player: tax unpaid, a subscription run
    /// out, a station taken down or paused
    Warning {
        player_id: Uuid,
        kind: WarningKind,
        /// A plain sentence for the console
        text: String,
    },
    /// This stream fell behind and `missed` things went past it. The
    /// samples are in `/admin/series`; the rest are gone.
    Lagged { missed: u64 },
}

impl Feed {
    /// Its `type`, which is also its event's name
    pub fn type_name(&self) -> &'static str {
        match self {
            Feed::Sample(_) => "sample",
            Feed::Annotation(_) => "annotation",
            Feed::Fill { .. } => "fill",
            Feed::Deed { .. } => "deed",
            Feed::Gazette { .. } => "gazette",
            Feed::Warning { .. } => "warning",
            Feed::Lagged { .. } => "lagged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DeedChange {
    Claimed,
    Released,
    Foreclosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    TaxDelinquent,
    SubscriptionLapsed,
    StationDisplaced,
    AutomationPaused,
}

/// `GET /admin/series` (D0.3): the samples and annotations between two
/// times, oldest first. The newest day's samples come from the ring, ten
/// seconds apart; before that, and after a restart, from `metrics_samples`,
/// a minute apart. Each sample's `since_ms` says how long it covers. Asked
/// for `fields`, each sample holds only those, with its times.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Series {
    pub samples: Vec<Sample>,
    pub annotations: Vec<Annotation>,
}

/// The fields every sample keeps in a `/admin/series` asked for some:
/// when it was taken, and the window it covers
pub const SERIES_TIMES: [&str; 3] = ["at_ms", "booted_ms", "since_ms"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{reconcile, AccountBooks, Load};

    fn account(n: u128, citizen: bool, strands: i64, flows: &[(&str, i64)]) -> AccountBooks {
        AccountBooks {
            player_id: Uuid::from_u128(n),
            name: format!("player {n}"),
            citizen,
            strands,
            flows: flows.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    fn ledger(at_ms: i64, accounts: Vec<AccountBooks>, escrow: i64, units: &[(&str, i64)], flows: &[(&str, &str, i64)]) -> Ledger {
        let mut unit_flows: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
        for (item, flow, n) in flows {
            unit_flows.entry(item.to_string()).or_default().insert(flow.to_string(), *n);
        }
        Ledger {
            at_ms,
            booted_ms: 1,
            accounts,
            escrow_strands: escrow,
            units: units.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            unit_flows,
            units_settled: true,
            load: Load::default(),
        }
    }

    /// Four ledgers ten seconds apart: a deposit, a newcomer, a bid into
    /// escrow, a fill and a gather or two. Each sample's flows are its own
    /// window's, and the three add up to what the ends reconcile to.
    #[test]
    fn a_run_of_samples_adds_up_to_its_ends() {
        let admins = HashSet::from([Uuid::from_u128(9)]);
        let l0 = ledger(
            0,
            vec![account(1, true, 100, &[]), account(9, false, 5000, &[("admin_adjustment", 4900)])],
            0,
            &[("log", 10)],
            &[("log", "gathered", 10)],
        );
        let l1 = ledger(
            10_000,
            vec![
                account(1, true, 130, &[("exchange_deposit", 30)]),
                account(2, false, 100, &[]),
                account(9, false, 5000, &[("admin_adjustment", 4900)]),
            ],
            0,
            &[("log", 14)],
            &[("log", "gathered", 14)],
        );
        // The admin bids 4 logs at 5; citizen 1 sells them
        let l2 = ledger(
            20_000,
            vec![
                account(1, true, 150, &[("exchange_deposit", 30), ("market_sale", 20)]),
                account(2, false, 100, &[]),
                account(9, false, 4970, &[("admin_adjustment", 4900), ("market_escrow", -30)]),
            ],
            10,
            &[("log", 14)],
            &[("log", "gathered", 14)],
        );
        let l3 = ledger(
            30_000,
            vec![
                account(1, true, 150, &[("exchange_deposit", 30), ("market_sale", 20)]),
                account(2, false, 100, &[]),
                account(9, false, 4970, &[("admin_adjustment", 4900), ("market_escrow", -30)]),
            ],
            10,
            &[("log", 13), ("plank", 1)],
            &[("log", "gathered", 15), ("log", "consumed", -2), ("plank", "crafted", 1)],
        );
        let samples: Vec<Sample> =
            [(&l0, &l1), (&l1, &l2), (&l2, &l3)].iter().map(|(a, b)| Sample::between(a, b, &admins)).collect();

        assert_eq!(samples[0].since_ms, 10_000);
        assert_eq!(samples[0].strand_flows, BTreeMap::from([("exchange_deposit".to_string(), 30)]));
        assert_eq!(samples[0].unit_flows["log"], BTreeMap::from([("gathered".to_string(), 4)]));
        assert_eq!(samples[1].strands, Strands { citizens: 150, people: 100, admins: 4970, escrow: 10 });
        assert_eq!(samples[1].strands.total(), 5230);
        assert!(samples[1].unit_flows.is_empty(), "nothing made or destroyed");
        assert_eq!(samples[2].units["plank"], 1);

        let books = reconcile(&l0, &l3);
        assert!(books.holds(), "{books:?}");
        let tally = tally(&samples);
        assert_eq!(tally.strand_flows, books.strands.flows);
        assert_eq!(tally.unit_flows, books.units.flows);
    }

    /// Across a restart the unit flows start again from nothing
    #[test]
    fn a_new_boot_counts_its_flows_from_nothing() {
        let a = ledger(0, vec![], 0, &[("log", 40)], &[("log", "gathered", 40)]);
        let mut b = ledger(10_000, vec![], 0, &[("log", 43)], &[("log", "gathered", 3)]);
        b.booted_ms = 2;
        let sample = Sample::between(&a, &b, &HashSet::new());
        assert_eq!(sample.unit_flows["log"]["gathered"], 3);
        assert_eq!(sample.booted_ms, 2);
    }

    #[test]
    fn vwap_is_strands_over_units() {
        assert_eq!(Trades { trades: 2, units: 4, strands: 22 }.vwap(), Some(5.5));
        assert_eq!(Trades::default().vwap(), None);
    }

    #[test]
    fn the_gini_runs_from_even_to_one_holding_all() {
        assert_eq!(gini(&[50, 50, 50, 50]), Some(0.0));
        assert_eq!(gini(&[0, 0, 0, 100]), Some(0.75));
        // The order doesn't matter, and a debt is nothing
        assert_eq!(gini(&[100, -40, 0, 0]), Some(0.75));
        let g = gini(&[1, 2, 3, 4]).unwrap();
        assert!((g - 0.25).abs() < 1e-12, "{g}");
        assert_eq!(gini(&[100]), None);
        assert_eq!(gini(&[0, 0]), None);
    }

    #[test]
    fn a_day_of_samples_is_the_ring() {
        assert_eq!(RING_SAMPLES, 8640);
        assert_eq!(STORE_EVERY, 6);
    }
}
