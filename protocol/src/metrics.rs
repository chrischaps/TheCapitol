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
//! Since D4.1 the town reports in: `capitol-town` posts a [`TownReport`]
//! each game minute, and each sample carries a [`TownSummary`] of the
//! reports in its window.
//! Since D2.2 each sample says how people's and citizens' Strands are spread
//! ([`Distribution`]): the Gini, the richest tenth's share, and the Strands
//! each trade holds.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::annotation::Annotation;
use crate::ledger::{Crafts, DeedGrants, Ledger, LoadWindow};
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
    /// Accounts made over the window, each with
    /// [`STARTING_STRANDS`](crate::ledger::STARTING_STRANDS) and no audit
    /// row: the one faucet `strand_flows` can't show (D2.1). A sample stored
    /// before it reads as none.
    #[serde(default)]
    pub new_accounts: u64,
    /// The road in's deed grants now (#440): land given, outside the
    /// Strands' books. A sample stored before them reads as none.
    #[serde(default)]
    pub grants: DeedGrants,
    /// Every unit in the world now, by item type
    pub units: BTreeMap<String, i64>,
    /// Units made (positive) and destroyed (negative) over the window, by
    /// item type, then by flow
    pub unit_flows: BTreeMap<String, BTreeMap<String, i64>>,
    /// Whether both ends' units and flows were read in a quiet moment
    pub units_settled: bool,
    /// The crafts finished over the window, by recipe id (D2.4); a sample
    /// stored before them reads with none
    #[serde(default)]
    pub crafts: BTreeMap<String, Crafts>,
    /// The market's fills over the window, by item type
    pub market: BTreeMap<String, Trades>,
    /// The bazaar's sales over the window, by item type
    pub bazaar: BTreeMap<String, Trades>,
    /// The market's open orders as it was taken, by item type (D2.3); only
    /// items with some, and a sample stored before it reads with none
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub books: BTreeMap<String, Book>,
    pub players: Players,
    pub events: EventBus,
    /// The watches firing as it was taken (D2.5); a sample stored before
    /// them reads with none
    #[serde(default)]
    pub watches: Vec<Watch>,
    /// What the books haven't explained since the server started, as the
    /// drift watch counts it (D1.2's drift tile); a sample stored before it
    /// reads as none
    #[serde(default)]
    pub drift: Drift,
    /// The town's reports over the window (D4.1); `None` while no town
    /// reports in
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub town: Option<TownSummary>,
    /// How people's and citizens' Strands are spread now (D2.2); `None` in
    /// a sample stored before it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distribution: Option<Distribution>,
}

/// What the books haven't explained since the server started: the drift
/// watch's running count (`watch::Watches::drift`). It should be zero.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Drift {
    /// Strands the audit trail doesn't explain
    pub strands: i64,
    /// Units each item's flows don't explain, by item type; only items with
    /// some
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub units: BTreeMap<String, i64>,
}

impl Drift {
    pub fn is_zero(&self) -> bool {
        self.strands == 0 && self.units.is_empty()
    }
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

/// One item's book at a moment: what rests on each side (D2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Book {
    /// Units wanted on open buy orders
    pub bids: i64,
    /// Units offered on open sell orders
    pub asks: i64,
    /// The highest bid, Strands a unit
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_bid: Option<i64>,
    /// The lowest ask, Strands a unit
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_ask: Option<i64>,
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
        let mut new_accounts = 0;
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
            new_accounts += u64::from(was.is_none());
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
        let mut crafts = BTreeMap::new();
        for (recipe, now) in &end.crafts {
            let was = if same_boot { start.crafts.get(recipe).copied().unwrap_or_default() } else { Crafts::default() };
            let done = now.since(&was);
            if done.runs > 0 {
                crafts.insert(recipe.clone(), done);
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
            new_accounts,
            grants: end.grants,
            units: end.units.clone(),
            unit_flows,
            units_settled: start.units_settled && end.units_settled,
            crafts,
            market: BTreeMap::new(),
            bazaar: BTreeMap::new(),
            books: BTreeMap::new(),
            players: Players::default(),
            events: EventBus::default(),
            watches: Vec::new(),
            drift: Drift::default(),
            town: None,
            distribution: None,
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

/// The share of what's held that the richest tenth hold: 0.1 when everyone
/// holds the same, 1 when they hold it all. The tenth is rounded up, so
/// it's at least one account. A debt counts as nothing. `None` for fewer
/// than two, or when nobody holds anything.
pub fn top_tenth_share(balances: &[i64]) -> Option<f64> {
    let mut held: Vec<i64> = balances.iter().map(|b| (*b).max(0)).collect();
    let total: i64 = held.iter().sum();
    if held.len() < 2 || total == 0 {
        return None;
    }
    held.sort_unstable_by(|a, b| b.cmp(a));
    let tenth = held.len().div_ceil(10);
    Some(held[..tenth].iter().sum::<i64>() as f64 / total as f64)
}

/// The role of a person's account in [`Distribution::roles`]
pub const ROLE_PERSON: &str = "person";
/// The role of a citizen the town hasn't named a trade for: no town has
/// reported, or it isn't on the roster
pub const ROLE_CITIZEN: &str = "citizen";
/// The role of a citizen with no favourite job (Callings K3.3): an
/// all-rounder. Every other citizen's is its favourite's slug.
pub const ROLE_ALL_ROUNDER: &str = "all-rounder";

/// How the Strands people and citizens hold are spread (D2.2). Admins'
/// aren't counted, as the Gini watch doesn't count them, nor is escrow.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Distribution {
    /// People's and citizens' accounts
    pub accounts: u64,
    /// The [`gini`] of their Strands
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gini: Option<f64>,
    /// The share the richest tenth of them hold ([`top_tenth_share`])
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_tenth: Option<f64>,
    /// Their Strands by role: each citizen's from the town's report, its
    /// favourite job's slug (`gatherer-wood`, `merchant`) or
    /// [`ROLE_ALL_ROUNDER`] (Callings K3.3; before callings, its trade's
    /// slug, which reads the same), [`ROLE_CITIZEN`] for one it doesn't
    /// name, and [`ROLE_PERSON`] for people
    pub roles: BTreeMap<String, Holding>,
}

/// What the accounts in one role hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Holding {
    pub accounts: u64,
    pub strands: i64,
}

impl Distribution {
    /// The spread of `ledger`'s Strands over its accounts but `admins`'.
    /// `role` names a citizen's trade from its name, when the town says.
    pub fn of<'a>(ledger: &Ledger, admins: &HashSet<Uuid>, role: impl Fn(&str) -> Option<&'a str>) -> Distribution {
        let mut balances = Vec::with_capacity(ledger.accounts.len());
        let mut roles: BTreeMap<String, Holding> = BTreeMap::new();
        for account in ledger.accounts.iter().filter(|a| !admins.contains(&a.player_id)) {
            balances.push(account.strands);
            let name = if account.citizen { role(&account.name).unwrap_or(ROLE_CITIZEN) } else { ROLE_PERSON };
            let holding = roles.entry(name.to_string()).or_default();
            holding.accounts += 1;
            holding.strands += account.strands;
        }
        Distribution {
            accounts: balances.len() as u64,
            gini: gini(&balances),
            top_tenth: top_tenth_share(&balances),
            roles,
        }
    }
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

/// `GET /admin/series`'s query (D0.3). The client's admin overlay (D5.3)
/// asks with it too.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesQuery {
    /// Unix ms, exclusive; the default is an hour before `to_ms`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ms: Option<i64>,
    /// Unix ms, inclusive; the default is now
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ms: Option<i64>,
    /// The sample's fields to send, comma-separated (`load,strands`); each
    /// sample keeps its times too. The default is all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<String>,
}

/// The fields every sample keeps in a `/admin/series` asked for some:
/// when it was taken, and the window it covers
pub const SERIES_TIMES: [&str; 3] = ["at_ms", "booted_ms", "since_ms"];

// ---------------------------------------------------------------------------
// The town's report (D4.1)
// ---------------------------------------------------------------------------

/// How often the town reports, in ms: each game minute, five real seconds
pub const TOWN_REPORT_MS: u64 = (crate::rules::DAY_LENGTH_SECS * 1000 / (24 * 60)) as u64;
/// A town that hasn't reported in this long, in ms, has gone quiet: it's
/// marked down
pub const TOWN_QUIET_MS: i64 = 60_000;
/// The failures a report carries, the newest
pub const TOWN_FAILURES: usize = 20;
/// The decisions a citizen's row carries, the newest
pub const CITIZEN_DECISIONS: usize = 5;

/// `POST /admin/town/report` (D4.1): the town, as `capitol-town` sees it,
/// each game minute ([`TOWN_REPORT_MS`]). Its counts are over the window
/// since the report before. The town's files (`town.status`, the presence
/// log, the decision logs) are written as before; this is what reaches the
/// server.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TownReport {
    /// When the town wrote it, by the town's clock (Unix ms)
    pub at_ms: i64,
    /// When this run of the town started: a new one is a new run
    pub started_ms: i64,
    /// The window's length, in ms: since the report before, or since the
    /// town started
    pub since_ms: i64,
    /// The town is stopping, and this is its last report
    #[serde(default, skip_serializing_if = "is_false")]
    pub stopping: bool,
    /// Every citizen on the roster, in its order
    pub citizens: Vec<CitizenReport>,
    /// Each decision layer's turns over the window, by name
    pub layers: BTreeMap<String, LayerReport>,
    /// How the questions were settled over the window: by a layer's name,
    /// `rules, unsure`, `the rules' own pick`, `carry on`
    pub rulings: BTreeMap<String, u64>,
    /// The newest failures, oldest first, at most [`TOWN_FAILURES`]
    pub failures: Vec<TownFailure>,
    /// Failures over the window. A citizen that fails twice between two
    /// reports shows only its last in `failures`, but both count here.
    pub failed: u64,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl TownReport {
    /// This report and the `later` one after it, as one: for a report the
    /// server didn't take, sent again with the next. The citizens are the
    /// later report's, each with its own decisions if it has new ones, else
    /// this one's; the counts add up.
    pub fn then(mut self, later: TownReport) -> TownReport {
        let mut decisions: BTreeMap<String, Vec<DecisionBrief>> =
            self.citizens.into_iter().map(|c| (c.name, c.decisions)).collect();
        let mut citizens = later.citizens;
        for c in citizens.iter_mut().filter(|c| c.decisions.is_empty()) {
            c.decisions = decisions.remove(&c.name).unwrap_or_default();
        }
        for (name, layer) in &later.layers {
            self.layers.entry(name.clone()).or_default().add(layer);
        }
        for (by, n) in later.rulings {
            *self.rulings.entry(by).or_insert(0) += n;
        }
        TownReport {
            at_ms: later.at_ms,
            started_ms: later.started_ms,
            since_ms: self.since_ms + later.since_ms,
            stopping: later.stopping,
            citizens,
            layers: self.layers,
            rulings: self.rulings,
            failures: later.failures,
            failed: self.failed + later.failed,
        }
    }

    /// Its citizens, by standing: `{"working": 18, "away": 600}`
    pub fn standing(&self) -> BTreeMap<String, u64> {
        let mut by = BTreeMap::new();
        for c in &self.citizens {
            *by.entry(c.standing.clone()).or_insert(0) += 1;
        }
        by
    }
}

/// One citizen in a [`TownReport`]: its health line, and its newest
/// decisions.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CitizenReport {
    pub name: String,
    /// The bucket its wealth and time are counted in: its favourite job's
    /// slug (`gatherer-wood`, `merchant`), or [`ROLE_ALL_ROUNDER`]
    /// (Callings K3.3). Before callings, its trade's slug.
    pub role: String,
    /// Where its life stands: `starting`, `working`, `resting`, `offline`,
    /// `waiting`, `away`, `retired` or `stopped`
    pub standing: String,
    /// Where in its day it is: `work`, `market`, `home`, `night`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strands: Option<i64>,
    /// What it's doing, as its health line says
    pub activity: String,
    /// Lives started after the first
    pub restarts: u32,
    /// The last thing that went wrong
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<TownFailure>,
    /// Its newest decisions, oldest first, at most [`CITIZEN_DECISIONS`].
    /// Empty when it has made none since the report before: the server
    /// keeps the last it was sent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<DecisionBrief>,
    /// Where it is on the road in (#450), once it has signed in and heard
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub road_in: Option<RoadInReport>,
    /// Who it is, as work goes (Callings K3.3): its style, favourite and
    /// top leanings. `None` from a town before callings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calling: Option<CallingReport>,
    /// The job of the moment, its stint (Callings K1.2), and how long it's
    /// run. Kept while it's away: the job it was last at. `None` before its
    /// first stint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<StintReport>,
    /// Game hours on each activity since the town started (Callings K0.1),
    /// by the activity's slug: `fiber`, `wood`, …, `trading`, `commons`,
    /// `detour`, `road_in`, and `free` for time with nothing to do
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub time: BTreeMap<String, f64>,
}

/// A citizen's calling, briefly (Callings K3.3): what the console's roster
/// says of it ("Wood, also ore and planks", "All-rounder").
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CallingReport {
    /// `specialist`, `leaner` or `all-rounder`
    pub style: String,
    /// Its favourite job's slug (`gatherer-wood`); none for an all-rounder
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favourite: Option<String>,
    /// The favourite's share of its work time, 0 to 1; 0 without one
    pub focus: f64,
    /// Its other jobs it leans to most, the most first: at most three, each
    /// a share it's willing to work at (5% and over)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leanings: Vec<Leaning>,
}

/// One of the jobs a calling leans to.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Leaning {
    /// The job's slug: `crafter-planks`
    pub job: String,
    /// Its share of the work time, 0 to 1
    pub share: f64,
}

/// A citizen's stint (Callings K1.2): the job it's at, and for how long.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StintReport {
    /// The job's slug: `crafter-planks`
    pub job: String,
    /// Game hours it has run, over its sessions
    pub hours: f64,
}

/// Where a citizen is on the road in (#450), briefly: the console's
/// roster shows it, and `capitol-town road-in` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RoadInReport {
    /// Steps done
    pub done: u32,
    /// Steps on the road, all told (`tutorial::Step::ALL`)
    pub of: u32,
    /// The first step not done, by its name on the wire, and its chapter's;
    /// none once the road is walked to its end
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter: Option<String>,
    /// Its deed: `unearned`, `open` or `spent`
    pub grant: crate::tutorial::Grant,
}

impl RoadInReport {
    /// From a player's progress on the road in.
    pub fn of(progress: &crate::tutorial::TutorialProgress) -> RoadInReport {
        let next = progress.done.current();
        RoadInReport {
            done: progress.done.iter().count() as u32,
            of: crate::tutorial::Step::ALL.len() as u32,
            next: next.map(|s| s.name().to_string()),
            chapter: next.map(|s| s.chapter().name().to_string()),
            grant: progress.grant,
        }
    }

    pub fn finished(&self) -> bool {
        self.done >= self.of
    }
}

/// One decision, as the console's drill-down shows it: the question, the
/// menu, what stood and how each layer answered.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DecisionBrief {
    /// By the town's clock (Unix ms)
    pub at_ms: i64,
    /// `work`, `sell plank`, `free time`
    pub question: String,
    /// The menu's entries: `work: plank`, `sell plank`, `keep`
    pub menu: Vec<String>,
    /// The entry that stood; `None` for an empty menu
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picked: Option<String>,
    /// Its price, if it took one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<i64>,
    /// Its quantity, if it took one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i32>,
    /// How it was settled, as [`TownReport::rulings`] counts it
    pub by: String,
    /// Each layer asked, in order
    pub trail: Vec<LayerTurn>,
}

/// One layer's turn at a decision.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct LayerTurn {
    pub layer: String,
    /// `settled`, `unsure`, `escalated`, `off menu`, `slow`, `broke`,
    /// `pending` or `late`
    pub outcome: String,
    pub micros: u64,
}

/// A decision layer's turns over a window, every citizen's together.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct LayerReport {
    /// Times it was asked
    pub asked: u64,
    /// What came of them, by [`LayerTurn::outcome`]
    pub outcomes: BTreeMap<String, u64>,
    /// Time it took over them all, in microseconds
    pub micros: u64,
    /// The longest one took
    pub max_micros: u64,
    /// US cents it cost over the window
    pub cents: f64,
    /// US cents it has cost since the town started
    pub cents_total: f64,
}

impl LayerReport {
    /// The times it settled a question
    pub fn settled(&self) -> u64 {
        self.outcomes.get("settled").copied().unwrap_or(0)
    }

    /// Its mean time to answer, in ms; `None` if it wasn't asked
    pub fn mean_ms(&self) -> Option<f64> {
        (self.asked > 0).then(|| self.micros as f64 / self.asked as f64 / 1000.0)
    }

    /// One turn more
    pub fn turn(&mut self, outcome: &str, micros: u64) {
        self.asked += 1;
        *self.outcomes.entry(outcome.to_string()).or_insert(0) += 1;
        self.micros += micros;
        self.max_micros = self.max_micros.max(micros);
    }

    /// `later`'s window after this one's: the counts add up, and the total
    /// cost is the later one's
    pub fn add(&mut self, later: &LayerReport) {
        self.asked += later.asked;
        for (outcome, n) in &later.outcomes {
            *self.outcomes.entry(outcome.clone()).or_insert(0) += n;
        }
        self.micros += later.micros;
        self.max_micros = self.max_micros.max(later.max_micros);
        self.cents += later.cents;
        self.cents_total = later.cents_total;
    }
}

/// Something that went wrong for a citizen.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TownFailure {
    /// By the town's clock (Unix ms)
    pub at_ms: i64,
    pub citizen: String,
    pub why: String,
}

/// `GET /admin/town` (D4.1): the newest report, with each citizen's last
/// decisions, however many reports ago they came.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TownLatest {
    /// When it came in, by the server's clock (Unix ms)
    pub received_ms: i64,
    /// Whether the town is on: it hasn't said it's stopping, nor gone quiet
    /// for [`TOWN_QUIET_MS`]
    pub up: bool,
    pub report: TownReport,
}

/// What a [`Sample`] keeps of the town's reports over its window.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TownSummary {
    /// When the newest report came in, by the server's clock (Unix ms)
    pub reported_ms: i64,
    /// When the town's run started, by the town's clock
    pub started_ms: i64,
    /// Reports over the window
    pub reports: u64,
    /// The roster now, by standing, as the newest report has it
    pub standing: BTreeMap<String, u64>,
    /// How the questions were settled over the window
    pub rulings: BTreeMap<String, u64>,
    /// Each layer's turns over the window
    pub layers: BTreeMap<String, LayerReport>,
    /// Failures over the window
    pub failed: u64,
}

impl TownSummary {
    /// Adds `report`, which came in at `received_ms`, to the window
    pub fn add(&mut self, report: &TownReport, received_ms: i64) {
        self.reported_ms = received_ms;
        self.started_ms = report.started_ms;
        self.reports += 1;
        self.standing = report.standing();
        for (by, n) in &report.rulings {
            *self.rulings.entry(by.clone()).or_insert(0) += n;
        }
        for (name, layer) in &report.layers {
            self.layers.entry(name.clone()).or_default().add(layer);
        }
        self.failed += report.failed;
    }

    /// This window and the `later` one after it, as one
    pub fn then(mut self, later: &TownSummary) -> TownSummary {
        self.reported_ms = later.reported_ms;
        self.started_ms = later.started_ms;
        self.reports += later.reports;
        self.standing = later.standing.clone();
        for (by, n) in &later.rulings {
            *self.rulings.entry(by.clone()).or_insert(0) += n;
        }
        for (name, layer) in &later.layers {
            self.layers.entry(name.clone()).or_default().add(layer);
        }
        self.failed += later.failed;
        self
    }

    /// Questions settled over the window, however they were
    pub fn decisions(&self) -> u64 {
        self.rulings.values().sum()
    }

    /// US cents the layers cost over the window
    pub fn cents(&self) -> f64 {
        self.layers.values().map(|l| l.cents).sum()
    }
}

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
            crafts: BTreeMap::new(),
            grants: Default::default(),
            held_flows: Default::default(),
            load: Load::default(),
        }
    }

    /// A window's crafts are the ends' difference, by recipe; a recipe
    /// that made nothing in it is left out, and a new boot counts from
    /// nothing again.
    #[test]
    fn a_samples_crafts_are_its_windows() {
        let crafted = |at_ms, booted_ms, crafts: &[(&str, Crafts)]| Ledger {
            booted_ms,
            crafts: crafts.iter().map(|(r, c)| (r.to_string(), *c)).collect(),
            ..ledger(at_ms, vec![], 0, &[], &[])
        };
        let rope = |runs, quality| Crafts { runs, units: runs as i64, quality };
        let l0 = crafted(0, 1, &[("rope_basic", rope(2, 120)), ("tool_handle", rope(1, 40))]);
        let l1 = crafted(10_000, 1, &[("rope_basic", rope(5, 300)), ("tool_handle", rope(1, 40))]);
        let sample = Sample::between(&l0, &l1, &HashSet::new());
        assert_eq!(sample.crafts.len(), 1, "the handle made nothing");
        assert_eq!(sample.crafts["rope_basic"], rope(3, 180));
        assert_eq!(sample.crafts["rope_basic"].mean_quality(), Some(60.0));
        let rebooted = crafted(20_000, 2, &[("rope_basic", rope(1, 70))]);
        assert_eq!(Sample::between(&l1, &rebooted, &HashSet::new()).crafts["rope_basic"], rope(1, 70));
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
        assert_eq!(samples.iter().map(|s| s.new_accounts).collect::<Vec<_>>(), [1, 0, 0], "the newcomer, once");
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
    fn the_richest_tenth_is_at_least_one_account() {
        assert_eq!(top_tenth_share(&[10; 10]), Some(0.1));
        // Eleven accounts make a tenth of two
        let mut eleven = vec![0; 9];
        eleven.extend([60, 40]);
        assert_eq!(top_tenth_share(&eleven), Some(1.0));
        // A debt is nothing
        assert_eq!(top_tenth_share(&[30, 10, -50]), Some(0.75));
        assert_eq!(top_tenth_share(&[100]), None);
        assert_eq!(top_tenth_share(&[0, -5]), None);
    }

    #[test]
    fn wealth_is_split_by_trade_without_the_admins() {
        let mut accounts = vec![
            account(1, true, 300, &[]),
            account(2, true, 100, &[]),
            account(3, true, 50, &[]),
            account(4, false, 20, &[]),
            account(5, false, 9000, &[]),
        ];
        accounts[0].name = "Ada".into();
        accounts[1].name = "Bram".into();
        let books = ledger(10_000, accounts, 500, &[], &[]);
        let admins = HashSet::from([Uuid::from_u128(5)]);
        let trades = BTreeMap::from([("Ada", "merchant"), ("Bram", "gatherer-wood")]);
        let d = Distribution::of(&books, &admins, |name| trades.get(name).copied());
        assert_eq!(d.accounts, 4);
        assert_eq!(d.gini, gini(&[300, 100, 50, 20]));
        assert_eq!(d.top_tenth, Some(300.0 / 470.0));
        let held = |role: &str| d.roles[role];
        assert_eq!(held("merchant"), Holding { accounts: 1, strands: 300 });
        assert_eq!(held("gatherer-wood"), Holding { accounts: 1, strands: 100 });
        // A citizen the town doesn't name, and a person
        assert_eq!(held(ROLE_CITIZEN), Holding { accounts: 1, strands: 50 });
        assert_eq!(held(ROLE_PERSON), Holding { accounts: 1, strands: 20 });
        assert_eq!(d.roles.len(), 4, "admins have no role");
    }

    #[test]
    fn a_town_reports_each_game_minute() {
        assert_eq!(TOWN_REPORT_MS, 5_000);
    }

    /// Two reports in a sample's window: the counts add up, the standing and
    /// the total cost are the newer one's
    #[test]
    fn a_summary_adds_up_its_reports() {
        let citizen = |name: &str, standing: &str| CitizenReport {
            name: name.into(),
            role: "gatherer-wood".into(),
            standing: standing.into(),
            ..CitizenReport::default()
        };
        let mut jev = LayerReport::default();
        jev.turn("settled", 300_000);
        jev.turn("unsure", 100_000);
        jev.cents = 0.02;
        jev.cents_total = 1.5;
        let first = TownReport {
            started_ms: 7,
            citizens: vec![citizen("Ada", "working"), citizen("Bo", "starting")],
            layers: BTreeMap::from([("jev".to_string(), jev.clone())]),
            rulings: BTreeMap::from([("jev".to_string(), 1), ("jev, unsure".to_string(), 1)]),
            failed: 1,
            ..TownReport::default()
        };
        jev.cents_total = 1.52;
        let second = TownReport {
            citizens: vec![citizen("Ada", "working"), citizen("Bo", "working")],
            layers: BTreeMap::from([("jev".to_string(), jev)]),
            rulings: BTreeMap::from([("jev".to_string(), 1)]),
            ..first.clone()
        };
        let mut summary = TownSummary::default();
        summary.add(&first, 100);
        summary.add(&second, 105);
        assert_eq!(summary.reported_ms, 105);
        assert_eq!(summary.reports, 2);
        assert_eq!(summary.standing, BTreeMap::from([("working".to_string(), 2)]));
        assert_eq!(summary.decisions(), 3);
        assert_eq!(summary.failed, 2);
        let jev = &summary.layers["jev"];
        assert_eq!((jev.asked, jev.settled(), jev.max_micros), (4, 2, 300_000));
        assert_eq!(jev.mean_ms(), Some(200.0));
        assert!((summary.cents() - 0.04).abs() < 1e-9);
        assert_eq!(jev.cents_total, 1.52);

        // A report sent again with the next one counts as both
        let both = first.clone().then(second.clone());
        assert_eq!(both.citizens, second.citizens);
        assert_eq!((both.failed, both.rulings.values().sum::<u64>()), (2, 3));
        assert_eq!((both.layers["jev"].asked, both.layers["jev"].cents_total), (4, 1.52));

        // Two windows run together are the one over both reports
        let (mut a, mut b) = (TownSummary::default(), TownSummary::default());
        a.add(&first, 100);
        b.add(&second, 105);
        assert_eq!(a.then(&b), summary);
    }

    #[test]
    fn a_day_of_samples_is_the_ring() {
        assert_eq!(RING_SAMPLES, 8640);
        assert_eq!(STORE_EVERY, 6);
    }
}
