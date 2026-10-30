//! The world's books (C1.7): `GET /admin/ledger`'s answer, and what two of
//! them say about each other.
//!
//! A [`Ledger`] is one moment: every account's Strands and what its audit
//! trail (`currency_transactions`) says moved them, the Strands held for buy
//! orders, every unit in the world, the units made and destroyed since the
//! server started, and the server's load. [`reconcile`] holds two of them up
//! against each other. Strands are conserved when every account's change is
//! what its trail explains, and the world's change is the faucets less the
//! sinks. Units are conserved when each item's change is the units the
//! server counted in and out. Anything else is drift, and drift fails a soak.
//! [`LoadWindow`] is the load between two of them, and goes in each of the
//! Observatory's samples (`metrics::Sample`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a new player starts with: `players.strand_balance`'s default
/// (migration `20260208_001_m5_economy`). It's the one Strand balance with
/// no audit row.
pub const STARTING_STRANDS: i64 = 100;

/// The buckets [`TICK_BUCKETS_MS`] cuts each doubling into.
pub const TICK_BUCKETS_PER_OCTAVE: usize = 16;
/// The first bucket's bound, in ms: everything up to it is one bucket.
const TICK_BUCKETS_FLOOR_MS: f64 = 1.0 / 16.0;
/// Doublings from the floor to the last bound, 1024 ms.
const TICK_BUCKET_OCTAVES: usize = 14;

/// The upper bounds of [`Load::tick_buckets`], in milliseconds of work a
/// tick. A last bucket past them holds the slower ticks.
///
/// They're log-linear (#409), as an HDR histogram's are: up to 1/16 ms is
/// one bucket, then each doubling to 1024 ms is cut into 16 even ones, so
/// a bucket is never wider than a 16th of its value. 16 to 32 ms goes up a
/// millisecond at a time, 32 to 64 two. Every bound is a binary fraction,
/// so it reads back from JSON exactly. [`LoadWindow`]'s percentiles are
/// interpolated inside the bucket, so they move with a change smaller than
/// a bucket too.
pub const TICK_BUCKETS_MS: [f64; 1 + TICK_BUCKET_OCTAVES * TICK_BUCKETS_PER_OCTAVE] = tick_bounds();

const fn tick_bounds() -> [f64; 1 + TICK_BUCKET_OCTAVES * TICK_BUCKETS_PER_OCTAVE] {
    let mut bounds = [TICK_BUCKETS_FLOOR_MS; 1 + TICK_BUCKET_OCTAVES * TICK_BUCKETS_PER_OCTAVE];
    let mut base = TICK_BUCKETS_FLOOR_MS;
    let mut octave = 0;
    while octave < TICK_BUCKET_OCTAVES {
        let mut k = 1;
        while k <= TICK_BUCKETS_PER_OCTAVE {
            bounds[octave * TICK_BUCKETS_PER_OCTAVE + k] = base + base * k as f64 / TICK_BUCKETS_PER_OCTAVE as f64;
            k += 1;
        }
        base *= 2.0;
        octave += 1;
    }
    bounds
}

/// The bucket of [`TICK_BUCKETS_MS`] a run of `ms` counts in: the first
/// whose bound it's at or under, or `TICK_BUCKETS_MS.len()` past the last.
pub fn tick_bucket(ms: f64) -> usize {
    TICK_BUCKETS_MS.partition_point(|bound| *bound < ms)
}

/// The upper bounds of [`Load::command_buckets`]: commands waiting when a
/// tick took them up (D3.1). A last bucket past them holds the deeper queues.
pub const COMMAND_BUCKETS: [f64; 10] = [0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 250.0, 1000.0];

/// `GET /admin/ledger`: the world's books at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Ledger {
    /// When it was read, by the server's clock (Unix ms)
    pub at_ms: i64,
    /// When the server started. [`unit_flows`](Self::unit_flows) count from
    /// here, so two ledgers with different boots can't check units.
    pub booted_ms: i64,
    pub accounts: Vec<AccountBooks>,
    /// Strands held for someone, outside every balance: open buy orders'
    /// escrow, and since S0.2 the court's (commissions and wage offers) and
    /// the guilds' treasuries (S4.2). The backend's `ledger::held_strands`
    /// is the one sum.
    pub escrow_strands: i64,
    /// Strands that left a holding for no one, by transaction type, ever: a
    /// guild's dues and roster fees out of its treasury (S4.2). They're sinks
    /// no account's trail can carry, so the holding's own books write them
    /// (`guild_ledger`), negative as an account's sink is. A ledger from
    /// before reads with none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub held_flows: BTreeMap<String, i64>,
    /// Every unit in the world, by item type: inventories, boxes, stalls,
    /// escrow, impounds
    pub units: BTreeMap<String, i64>,
    /// Units made (positive) and destroyed (negative) since the server
    /// started, by item type, then by [`flow`](UnitFlow)
    pub unit_flows: BTreeMap<String, BTreeMap<String, i64>>,
    /// Whether the units and their flows were read with no unit write
    /// committing between. A busy server may never give a quiet moment;
    /// then this is false and the two may be a write apart.
    pub units_settled: bool,
    /// The crafts finished since the server started, by recipe id: by
    /// hand and by a station's automation (D2.4). A ledger from before
    /// reads with none.
    #[serde(default)]
    pub crafts: BTreeMap<String, Crafts>,
    /// The road in's deed grants (#440): land given, not bought. They're a
    /// faucet of land, outside the Strands' books, since no Strand moves
    /// when one is spent. A ledger from before reads with none.
    #[serde(default)]
    pub grants: DeedGrants,
    /// The land as it stands, by zone (#451): what the free choice of the
    /// road in's deed does to where people live. Every zone with lots. A
    /// ledger from before reads with none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub land: BTreeMap<String, ZoneLand>,
    /// Gathers the server refused since it started (#562), by resource type
    /// then why: `spent` (the node is regrowing), `taken` (someone else is
    /// gathering it), `lot` (it's on a lot the gatherer may not gather on),
    /// `busy`, `far` and `tool`. A ledger from before reads with none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refused: BTreeMap<String, BTreeMap<String, u64>>,
    pub load: Load,
}

/// The road in's deed grants as they stand (#440, plan D5): opened by
/// walking it, spent on a lot, and handed back when a granted lot is retired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DeedGrants {
    /// Grants earned, ever: one an account at most
    pub opened: u64,
    /// Those spent on a plot, now. A foreclosed plot's grant stays spent.
    pub used: u64,
    /// Those a retired lot has handed back, spent again since or not
    pub returned: u64,
    /// What the granted plots held now are assessed at: the zone's deed
    /// price when each was claimed (D4), and what their tax is on
    pub granted_value: i64,
}

/// One zone's lots as they stand, on a [`Ledger`] (#451).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ZoneLand {
    /// Its lots, claimed or free
    pub lots: u64,
    /// Those someone holds
    pub owned: u64,
    /// Of those, the ones claimed with the road in's deed
    pub granted: u64,
    /// The accounts that hold one there or more: its homes
    pub owners: u64,
    /// What the held lots are assessed at, all together
    pub assessed: i64,
}

/// Crafts of one recipe: since the server started on a [`Ledger`], over the
/// window on a sample (D2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Crafts {
    /// Crafts finished
    pub runs: u64,
    /// Units they made, every output
    pub units: i64,
    /// Their outputs' quality, summed over the runs: a craft's outputs all
    /// come out at one quality
    pub quality: i64,
}

impl Crafts {
    /// One craft, making `units` at `quality`
    pub fn add(&mut self, units: i64, quality: i32) {
        self.runs += 1;
        self.units += units;
        self.quality += i64::from(quality);
    }

    /// The mean quality out; `None` with no runs
    pub fn mean_quality(&self) -> Option<f64> {
        (self.runs > 0).then(|| self.quality as f64 / self.runs as f64)
    }

    /// What these crafts are past `before`'s: `self` from a later moment
    pub fn since(&self, before: &Crafts) -> Crafts {
        Crafts {
            runs: self.runs.saturating_sub(before.runs),
            units: self.units - before.units,
            quality: self.quality - before.quality,
        }
    }
}

/// One account's Strands, and everything its audit trail says moved them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct AccountBooks {
    pub player_id: Uuid,
    pub name: String,
    /// A citizen (C0.1), not a person
    pub citizen: bool,
    pub strands: i64,
    /// The sum of its `currency_transactions` amounts, by type, ever
    pub flows: BTreeMap<String, i64>,
    /// Its Strands in escrow, outside [`strands`](Self::strands) (#553): a
    /// part of [`Ledger::escrow_strands`]. A ledger from before reads with
    /// none.
    #[serde(default, skip_serializing_if = "AccountEscrow::is_empty")]
    pub escrow: AccountEscrow,
}

impl AccountBooks {
    /// Its Strands with those it has in escrow: what it holds, all told.
    pub fn holding(&self) -> i64 {
        self.strands + self.escrow.total()
    }
}

/// An account's Strands in escrow (#553), by what holds them. They're still
/// its own, put up for something not yet come: a bid's for its fill, a
/// commission's and a wage offer's for the work. The guilds' treasuries are
/// no one account's, so they aren't here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct AccountEscrow {
    /// Its open buy orders on the Exchange (E3.1)
    #[serde(default, skip_serializing_if = "crate::rules::is_zero")]
    pub bids: i64,
    /// What's unspent of its open commissions (S2.1)
    #[serde(default, skip_serializing_if = "crate::rules::is_zero")]
    pub commissions: i64,
    /// What's unspent of its wage offers (S2.6)
    #[serde(default, skip_serializing_if = "crate::rules::is_zero")]
    pub wages: i64,
}

impl AccountEscrow {
    pub fn total(&self) -> i64 {
        self.bids + self.commissions + self.wages
    }

    pub fn is_empty(&self) -> bool {
        *self == AccountEscrow::default()
    }

    /// `other`'s added to it, each by what holds it
    pub fn add(&mut self, other: &AccountEscrow) {
        self.bids += other.bids;
        self.commissions += other.commissions;
        self.wages += other.wages;
    }
}

/// How hard the server is working, as running totals since it started:
/// two of them make a [`LoadWindow`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Load {
    /// The tick's period: 100 ms over `SIM_TICK_SCALE`
    pub tick_period_ms: f64,
    pub ticks: u64,
    /// Seconds of work all those ticks took
    pub tick_secs: f64,
    /// The slowest tick yet, in ms
    pub tick_max_ms: f64,
    /// Ticks by how long they took, under [`TICK_BUCKETS_MS`], and one more
    /// for those slower than all of them. Empty buckets at the end may be
    /// left off.
    pub tick_buckets: Vec<u64>,
    /// Avatars in the world now
    pub players_in_world: u64,
    /// WebSockets open now
    pub sockets_open: u64,
    pub sockets_opened: u64,
    /// Sockets that ended, by how: `closed` (the client said goodbye),
    /// `vanished` (it went without a word), `send_failed` (the server
    /// couldn't write to it), `session_ended`, `key_revoked`, `auth_failed`
    pub sockets_ended: BTreeMap<String, u64>,
    /// Sockets that fell behind the event stream and were resynced (C0.3)
    pub socket_resyncs: u64,
    pub db_pool_size: u32,
    pub db_pool_idle: u32,
    /// Transactions committed and rolled back in the server's database
    /// (`pg_stat_database`), every client's. Each statement outside a
    /// transaction is one, so it's about the statements a second.
    pub db_transactions: i64,
    /// The tick's work by phase (D0.1): `lock`, `commands`, `movement`,
    /// `extraction`, `crafting`, `regeneration`, `trades`, `broadcast`,
    /// `gathers`, `crafts`, `positions`, `tax`, `subscriptions`. Every part of
    /// a tick is in one of them, so their times add up to
    /// [`tick_secs`](Self::tick_secs). Empty from a server before D0.1.
    #[serde(default)]
    pub phases: BTreeMap<String, PhaseLoad>,
    /// The loops that run beside the tick, each a cycle at a time:
    /// `automation`, `market`, `bazaar`, and since D0.2 `sampler`. Not in
    /// the tick's time.
    #[serde(default)]
    pub loops: BTreeMap<String, PhaseLoad>,
    /// Statements that took [`slow_statement_ms`](Self::slow_statement_ms)
    /// or longer outside the tick and the loops: the routes, the sockets,
    /// startup (D3.2). Each phase and loop counts its own.
    #[serde(default)]
    pub slow_statements_elsewhere: u64,
    /// What counts as a slow statement: `SLOW_STATEMENT_MS`. 0 from a server
    /// before D3.2.
    #[serde(default)]
    pub slow_statement_ms: f64,
    /// Events missed by receivers of the event bus that fell behind
    /// (`RecvError::Lagged`), by receiver: `sockets`, and the Observatory's
    /// `admin_tap` and `admin_stream` (D0.3). Empty from a server before it.
    #[serde(default)]
    pub events_lagged: BTreeMap<String, u64>,
    /// Commands the ticks took up from the queue (D3.1)
    #[serde(default)]
    pub commands: u64,
    /// Ticks by how many commands were waiting for them, under
    /// [`COMMAND_BUCKETS`], and one more for deeper queues. Empty from a
    /// server before D3.1.
    #[serde(default)]
    pub command_buckets: Vec<u64>,
    /// The deepest the queue has been when a tick took it up
    #[serde(default)]
    pub command_queue_max: u64,
    /// Gathers and crafts the ticks handed the writer (`loops.writer`) that
    /// it hasn't written yet, now (#407). A writer falling behind shows here
    /// first: the tick doesn't slow, its saves just wait longer.
    #[serde(default)]
    pub writer_queued: u64,
    /// The most that have waited at once
    #[serde(default)]
    pub writer_queue_max: u64,
    /// Ticks' saves the writer has written, each tick's as one batch
    #[serde(default)]
    pub writer_batches: u64,
    /// Seconds those batches waited, all told, from the tick handing them
    /// over to the last of them written: how late their gatherers and
    /// crafters heard
    #[serde(default)]
    pub writer_lag_secs: f64,
    /// The longest a batch has waited, in ms
    #[serde(default)]
    pub writer_lag_max_ms: f64,
}

/// One phase of the tick, or one loop beside it, as running totals.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PhaseLoad {
    /// Times it ran. A phase that runs only some ticks (positions, tax)
    /// counts only those.
    pub runs: u64,
    /// Seconds it took, all told
    pub secs: f64,
    /// Its slowest run yet, in ms
    pub max_ms: f64,
    /// Runs by how long they took, under [`TICK_BUCKETS_MS`], and one more
    /// for those slower than all of them. Empty buckets at the end may be
    /// left off.
    pub buckets: Vec<u64>,
    /// Its database statements that took [`Load::slow_statement_ms`] or
    /// longer (D3.2)
    #[serde(default)]
    pub slow_statements: u64,
}

/// How a unit came into the world or left it, as [`Ledger::unit_flows`]
/// files it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnitFlow {
    /// From a node, by hand or by a tool
    Gathered,
    /// A craft's output, by hand or by a station's automation
    Crafted,
    /// A craft's inputs
    Consumed,
    /// A tool's last use
    WornOut,
    /// Fiber into the Exchange for Strands
    Deposited,
    /// Fiber out of the Exchange for Strands
    Withdrawn,
    /// A kit that became a station
    KitPlaced,
    /// A station taken down, or emptied by a foreclosure, back to its kit
    KitReturned,
    /// `POST /player/grant` (`DEV_GRANTS`)
    Granted,
    /// What a foreclosed station held that found no room anywhere, gone with
    /// the station
    Lost,
    /// Goods a station's repair used up (V3.2)
    Repaired,
    /// Goods a floor's building used up (V3.3)
    Built,
}

impl UnitFlow {
    pub const ALL: [UnitFlow; 12] = [
        UnitFlow::Gathered,
        UnitFlow::Crafted,
        UnitFlow::Consumed,
        UnitFlow::WornOut,
        UnitFlow::Deposited,
        UnitFlow::Withdrawn,
        UnitFlow::KitPlaced,
        UnitFlow::KitReturned,
        UnitFlow::Granted,
        UnitFlow::Lost,
        UnitFlow::Repaired,
        UnitFlow::Built,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            UnitFlow::Gathered => "gathered",
            UnitFlow::Crafted => "crafted",
            UnitFlow::Consumed => "consumed",
            UnitFlow::WornOut => "worn_out",
            UnitFlow::Deposited => "deposited",
            UnitFlow::Withdrawn => "withdrawn",
            UnitFlow::KitPlaced => "kit_placed",
            UnitFlow::KitReturned => "kit_returned",
            UnitFlow::Granted => "granted",
            UnitFlow::Lost => "lost",
            UnitFlow::Repaired => "repaired",
            UnitFlow::Built => "built",
        }
    }

    /// Whether it brings units in. A sink's count is filed negative.
    pub fn is_faucet(&self) -> bool {
        matches!(
            self,
            UnitFlow::Gathered | UnitFlow::Crafted | UnitFlow::Withdrawn | UnitFlow::KitReturned | UnitFlow::Granted
        )
    }
}

/// Every `currency_transactions.transaction_type` this build knows, in the
/// order the console stacks them: faucets, sinks, transfers, escrow. One
/// missing here is [`StrandFlow::Other`].
pub const TRANSACTION_TYPES: [&str; 28] = [
    "exchange_deposit",
    "deed_refund",
    "foreclosure_refund",
    "bazaar_refund",
    "admin_adjustment",
    "exchange_withdraw",
    "deed_purchase",
    "tax_payment",
    "property_tax",
    "market_fee",
    "bazaar_rent",
    "court_fee",
    "postage",
    "guild_fee",
    "guild_dues",
    "floor_permit",
    "trade_sent",
    "trade_received",
    "bazaar_purchase",
    "bazaar_sale",
    "market_escrow",
    "market_sale",
    "market_refund",
    "court_escrow",
    "court_payout",
    "court_refund",
    "guild_deposit",
    "guild_withdrawal",
];

/// What a `currency_transactions` type does to the world's Strands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum StrandFlow {
    /// Strands from outside: labour's deposits, refunds, grants
    Faucet,
    /// Strands gone for good: deeds, fees, rent, tax, the Exchange's fiber
    Sink,
    /// Between players, in pairs that net to nothing
    Transfer,
    /// Into and out of Strands held for someone ([`Ledger::escrow_strands`]):
    /// a buy order's escrow, the court's (S0.2), a guild's treasury (S4.2).
    /// Money paid out of a holding to someone else is this too, not a
    /// [`Transfer`](Self::Transfer): only its leaving the holding explains it.
    Escrow,
    /// A type this build doesn't know, counted in the world's sum as it is
    Other,
}

impl StrandFlow {
    /// The flow of a `currency_transactions.transaction_type`.
    pub fn of(transaction_type: &str) -> StrandFlow {
        match transaction_type {
            "exchange_deposit" | "deed_refund" | "foreclosure_refund" | "bazaar_refund" | "admin_adjustment" => {
                StrandFlow::Faucet
            }
            "exchange_withdraw" | "deed_purchase" | "tax_payment" | "property_tax" | "market_fee" | "bazaar_rent"
            | "court_fee" | "postage" | "guild_fee" | "guild_dues" | "floor_permit" => StrandFlow::Sink,
            "trade_sent" | "trade_received" | "bazaar_purchase" | "bazaar_sale" => StrandFlow::Transfer,
            "market_escrow" | "market_sale" | "market_refund" | "court_escrow" | "court_payout" | "court_refund"
            | "guild_deposit" | "guild_withdrawal" => StrandFlow::Escrow,
            _ => StrandFlow::Other,
        }
    }
}

/// Two ledgers held up against each other.
#[derive(Debug, Clone, PartialEq)]
pub struct Reconciliation {
    pub strands: StrandBooks,
    pub units: UnitBooks,
}

impl Reconciliation {
    /// Whether nothing drifted: every account explained, the world's sum
    /// explained, escrow explained, and every item's units explained.
    pub fn holds(&self) -> bool {
        self.strands.holds() && self.units.holds()
    }
}

/// The Strands between two ledgers.
#[derive(Debug, Clone, PartialEq)]
pub struct StrandBooks {
    /// Every account's Strands plus escrow, at each end
    pub start_total: i64,
    pub end_total: i64,
    /// Accounts made between, each starting with [`STARTING_STRANDS`]
    pub new_accounts: usize,
    /// The audit trail's change, by type
    pub flows: BTreeMap<String, i64>,
    pub faucets: i64,
    /// Negative: what left
    pub sinks: i64,
    /// What the transfers net to: 0 when every pair has both halves
    pub transfers: i64,
    /// Types this build doesn't know
    pub other: i64,
    pub escrow_start: i64,
    pub escrow_end: i64,
    /// Escrow's change less what the escrow types say went in and out
    pub escrow_drift: i64,
    /// Accounts whose change their trail doesn't explain: name, and by how
    /// much the balance is over (positive) or under what it should be
    pub accounts: Vec<(String, i64)>,
    /// Accounts at the start that are gone at the end, with their Strands
    pub gone: Vec<(String, i64)>,
    /// The world's change less the new accounts', the faucets, the sinks,
    /// the transfers and the unknown types
    pub drift: i64,
}

impl StrandBooks {
    pub fn holds(&self) -> bool {
        self.drift == 0 && self.escrow_drift == 0 && self.transfers == 0 && self.accounts.is_empty() && self.gone.is_empty()
    }
}

/// The units between two ledgers.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitBooks {
    /// The server restarted between, so its flows started again and units
    /// can't be checked
    pub void: bool,
    /// Whether both ledgers read their units in a quiet moment
    pub settled: bool,
    /// Each item's change in units, and what the flows say it should be
    pub items: BTreeMap<String, (i64, i64)>,
    /// The flows' change, by flow, over every item
    pub flows: BTreeMap<String, i64>,
    /// Items whose change the flows don't explain: item, and by how much the
    /// world holds more (positive) or fewer than it should
    pub drift: Vec<(String, i64)>,
}

impl UnitBooks {
    pub fn holds(&self) -> bool {
        !self.void && self.drift.is_empty()
    }
}

/// What `end` says about `start`.
pub fn reconcile(start: &Ledger, end: &Ledger) -> Reconciliation {
    Reconciliation {
        strands: strand_books(start, end),
        units: unit_books(start, end),
    }
}

fn strand_books(start: &Ledger, end: &Ledger) -> StrandBooks {
    let before: BTreeMap<Uuid, &AccountBooks> = start.accounts.iter().map(|a| (a.player_id, a)).collect();
    let mut flows: BTreeMap<String, i64> = BTreeMap::new();
    let mut accounts = Vec::new();
    let mut new_accounts = 0;
    for account in &end.accounts {
        let was = before.get(&account.player_id);
        let opening = was.map_or(STARTING_STRANDS, |a| a.strands);
        new_accounts += usize::from(was.is_none());
        let mut explained = 0;
        for (kind, sum) in &account.flows {
            let moved = sum - was.and_then(|a| a.flows.get(kind)).copied().unwrap_or(0);
            explained += moved;
            if moved != 0 {
                *flows.entry(kind.clone()).or_insert(0) += moved;
            }
        }
        let over = account.strands - opening - explained;
        if over != 0 {
            accounts.push((account.name.clone(), over));
        }
    }
    // What left a holding for no one: in the world's flows, and out of escrow
    let mut held_out = 0;
    for (kind, sum) in &end.held_flows {
        let moved = sum - start.held_flows.get(kind).copied().unwrap_or(0);
        held_out += moved;
        if moved != 0 {
            *flows.entry(kind.clone()).or_insert(0) += moved;
        }
    }
    let now: std::collections::HashSet<Uuid> = end.accounts.iter().map(|a| a.player_id).collect();
    let gone: Vec<(String, i64)> = start
        .accounts
        .iter()
        .filter(|a| !now.contains(&a.player_id))
        .map(|a| (a.name.clone(), a.strands))
        .collect();
    let total = |l: &Ledger| l.accounts.iter().map(|a| a.strands).sum::<i64>() + l.escrow_strands;
    let sum_of = |flow: StrandFlow| flows.iter().filter(|(k, _)| StrandFlow::of(k) == flow).map(|(_, v)| v).sum::<i64>();
    let (faucets, sinks, transfers, other) =
        (sum_of(StrandFlow::Faucet), sum_of(StrandFlow::Sink), sum_of(StrandFlow::Transfer), sum_of(StrandFlow::Other));
    let (start_total, end_total) = (total(start), total(end));
    // Escrow fills from buyers and empties to sellers and back to buyers,
    // and a treasury's dues leave it for no one
    let escrow_drift = (end.escrow_strands - start.escrow_strands) + sum_of(StrandFlow::Escrow) - held_out;
    StrandBooks {
        start_total,
        end_total,
        new_accounts,
        drift: end_total - start_total - new_accounts as i64 * STARTING_STRANDS - faucets - sinks - transfers - other,
        flows,
        faucets,
        sinks,
        transfers,
        other,
        escrow_start: start.escrow_strands,
        escrow_end: end.escrow_strands,
        escrow_drift,
        accounts,
        gone,
    }
}

fn unit_books(start: &Ledger, end: &Ledger) -> UnitBooks {
    let void = start.booted_ms != end.booted_ms;
    let flowed = |l: &Ledger, item: &str| l.unit_flows.get(item).map_or(0, |f| f.values().sum::<i64>());
    let mut flows: BTreeMap<String, i64> = BTreeMap::new();
    for (item, by_flow) in &end.unit_flows {
        for (flow, n) in by_flow {
            let moved = n - start.unit_flows.get(item).and_then(|f| f.get(flow)).copied().unwrap_or(0);
            if moved != 0 {
                *flows.entry(flow.clone()).or_insert(0) += moved;
            }
        }
    }
    let names: std::collections::BTreeSet<&String> =
        start.units.keys().chain(end.units.keys()).chain(start.unit_flows.keys()).chain(end.unit_flows.keys()).collect();
    let mut items = BTreeMap::new();
    let mut drift = Vec::new();
    for item in names {
        let held = end.units.get(item).copied().unwrap_or(0) - start.units.get(item).copied().unwrap_or(0);
        let explained = flowed(end, item) - flowed(start, item);
        if held != 0 || explained != 0 {
            items.insert(item.clone(), (held, explained));
        }
        if !void && held != explained {
            drift.push((item.clone(), held - explained));
        }
    }
    UnitBooks {
        void,
        settled: start.units_settled && end.units_settled,
        items,
        flows,
        drift,
    }
}

/// The load between two ledgers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct LoadWindow {
    pub secs: f64,
    pub ticks: u64,
    /// The tick's budget: its period
    pub tick_period_ms: f64,
    pub tick_mean_ms: f64,
    /// The window's ticks' percentiles, interpolated inside the bucket of
    /// [`TICK_BUCKETS_MS`] each falls in (#409); `None` past the last
    pub tick_p50_ms: Option<f64>,
    pub tick_p95_ms: Option<f64>,
    pub tick_p99_ms: Option<f64>,
    /// Ticks that took longer than the period
    pub ticks_over: u64,
    pub sockets_open: u64,
    pub sockets_opened: u64,
    /// Sockets that vanished or couldn't be written to
    pub sockets_dropped: u64,
    pub socket_resyncs: u64,
    pub players_in_world: u64,
    pub db_pool_size: u32,
    pub db_pool_idle: u32,
    pub db_transactions_per_sec: f64,
    /// The tick's phases over the window, and the loops beside it
    pub phases: BTreeMap<String, PhaseWindow>,
    pub loops: BTreeMap<String, PhaseWindow>,
    /// Slow statements outside the tick and the loops (D3.2)
    pub slow_statements_elsewhere: u64,
    /// Events each receiver missed over the window (D0.3); a sample stored
    /// before it reads with none
    #[serde(default)]
    pub events_lagged: BTreeMap<String, u64>,
    /// Commands the ticks took up over the window (D3.1); a sample stored
    /// before it reads with none
    #[serde(default)]
    pub commands: u64,
    /// The bound of the [`COMMAND_BUCKETS`] the 99th percentile of the
    /// queue's depth falls under, a tick at a time; `None` past the last,
    /// or with no ticks or no buckets
    #[serde(default)]
    pub command_queue_p99: Option<f64>,
    /// Saves waiting for the writer at the window's end (#407)
    #[serde(default)]
    pub writer_queued: u64,
    /// The mean wait of the window's batches, from the tick to written, in
    /// ms; `None` with none written
    #[serde(default)]
    pub writer_lag_ms: Option<f64>,
}

/// One phase, or one loop, between two loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PhaseWindow {
    pub runs: u64,
    /// Its time over the window
    pub secs: f64,
    /// The mean of a run, in ms
    pub mean_ms: f64,
    /// Its runs' 95th percentile, interpolated inside its bucket; `None`
    /// past the last
    pub p95_ms: Option<f64>,
    /// Its slow statements over the window (D3.2)
    pub slow_statements: u64,
}

impl PhaseWindow {
    fn between(a: Option<&PhaseLoad>, b: &PhaseLoad) -> PhaseWindow {
        let runs = b.runs.saturating_sub(a.map_or(0, |a| a.runs));
        let secs = (b.secs - a.map_or(0.0, |a| a.secs)).max(0.0);
        let buckets = since(a.map_or(&[][..], |a| &a.buckets), &b.buckets);
        PhaseWindow {
            runs,
            secs,
            mean_ms: if runs > 0 { secs * 1000.0 / runs as f64 } else { 0.0 },
            p95_ms: percentile_ms(&buckets, runs, 0.95),
            slow_statements: b.slow_statements.saturating_sub(a.map_or(0, |a| a.slow_statements)),
        }
    }
}

/// Each bucket's count in `b` less its count in `a`.
fn since(a: &[u64], b: &[u64]) -> Vec<u64> {
    b.iter()
        .enumerate()
        .map(|(i, n)| n.saturating_sub(a.get(i).copied().unwrap_or(0)))
        .collect()
}

/// The `q`th of `runs` timed under [`TICK_BUCKETS_MS`], in ms: found in
/// its bucket as if the bucket's runs were spread evenly across it, to the
/// µs. `None` in the bucket past the last, or with no runs.
fn percentile_ms(buckets: &[u64], runs: u64, q: f64) -> Option<f64> {
    if runs == 0 {
        return None;
    }
    let rank = runs as f64 * q;
    let mut seen = 0;
    for (i, &n) in buckets.iter().enumerate() {
        if n == 0 {
            continue;
        }
        if (seen + n) as f64 >= rank {
            let upper = *TICK_BUCKETS_MS.get(i)?;
            let lower = if i == 0 { 0.0 } else { TICK_BUCKETS_MS[i - 1] };
            let into = ((rank - seen as f64) / n as f64).clamp(0.0, 1.0);
            return Some(((lower + (upper - lower) * into) * 1000.0).round() / 1000.0);
        }
        seen += n;
    }
    None
}

/// The bound in `bounds` of the bucket the `q`th of `runs` falls under;
/// `None` past the last.
fn percentile(bounds: &[f64], buckets: &[u64], runs: u64, q: f64) -> Option<f64> {
    let rank = ((runs as f64 * q).ceil() as u64).max(1);
    let mut seen = 0;
    for (i, n) in buckets.iter().enumerate() {
        seen += n;
        if seen >= rank {
            return bounds.get(i).copied();
        }
    }
    None
}

fn phase_windows(a: &BTreeMap<String, PhaseLoad>, b: &BTreeMap<String, PhaseLoad>) -> BTreeMap<String, PhaseWindow> {
    b.iter().map(|(name, load)| (name.clone(), PhaseWindow::between(a.get(name), load))).collect()
}

impl LoadWindow {
    pub fn between(a: &Load, b: &Load, secs: f64) -> LoadWindow {
        let ticks = b.ticks.saturating_sub(a.ticks);
        let buckets = since(&a.tick_buckets, &b.tick_buckets);
        let tick_percentile = |q: f64| percentile_ms(&buckets, ticks, q);
        // Every tick takes the queue up once, so the depths count ticks
        let depths = since(&a.command_buckets, &b.command_buckets);
        let depth_runs: u64 = depths.iter().sum();
        // The buckets wholly over the period: those whose lower bound is at
        // or past it. One the period falls inside isn't counted.
        let ticks_over = TICK_BUCKETS_MS
            .iter()
            .position(|bound| *bound >= b.tick_period_ms)
            .map_or(0, |i| buckets.iter().skip(i + 1).sum());
        let dropped = |l: &Load| ["vanished", "send_failed"].iter().filter_map(|k| l.sockets_ended.get(*k)).sum::<u64>();
        LoadWindow {
            secs,
            ticks,
            tick_period_ms: b.tick_period_ms,
            tick_mean_ms: if ticks > 0 { (b.tick_secs - a.tick_secs) * 1000.0 / ticks as f64 } else { 0.0 },
            tick_p50_ms: tick_percentile(0.5),
            tick_p95_ms: tick_percentile(0.95),
            tick_p99_ms: tick_percentile(0.99),
            ticks_over,
            sockets_open: b.sockets_open,
            sockets_opened: b.sockets_opened.saturating_sub(a.sockets_opened),
            sockets_dropped: dropped(b).saturating_sub(dropped(a)),
            socket_resyncs: b.socket_resyncs.saturating_sub(a.socket_resyncs),
            players_in_world: b.players_in_world,
            db_pool_size: b.db_pool_size,
            db_pool_idle: b.db_pool_idle,
            db_transactions_per_sec: if secs > 0.0 { (b.db_transactions - a.db_transactions) as f64 / secs } else { 0.0 },
            phases: phase_windows(&a.phases, &b.phases),
            loops: phase_windows(&a.loops, &b.loops),
            slow_statements_elsewhere: b.slow_statements_elsewhere.saturating_sub(a.slow_statements_elsewhere),
            events_lagged: b
                .events_lagged
                .iter()
                .map(|(receiver, n)| (receiver.clone(), n.saturating_sub(a.events_lagged.get(receiver).copied().unwrap_or(0))))
                .collect(),
            commands: b.commands.saturating_sub(a.commands),
            command_queue_p99: if depth_runs > 0 { percentile(&COMMAND_BUCKETS, &depths, depth_runs, 0.99) } else { None },
            writer_queued: b.writer_queued,
            writer_lag_ms: match b.writer_batches.saturating_sub(a.writer_batches) {
                0 => None,
                n => Some((b.writer_lag_secs - a.writer_lag_secs).max(0.0) * 1000.0 / n as f64),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(n: u128, name: &str, strands: i64, flows: &[(&str, i64)]) -> AccountBooks {
        AccountBooks {
            player_id: Uuid::from_u128(n),
            name: name.into(),
            citizen: true,
            strands,
            flows: flows.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            escrow: Default::default(),
        }
    }

    fn ledger(accounts: Vec<AccountBooks>, escrow: i64) -> Ledger {
        Ledger {
            at_ms: 0,
            booted_ms: 1,
            accounts,
            escrow_strands: escrow,
            units: BTreeMap::new(),
            unit_flows: BTreeMap::new(),
            units_settled: true,
            crafts: BTreeMap::new(),
            grants: Default::default(),
            land: Default::default(),
            refused: Default::default(),
            held_flows: BTreeMap::new(),
            load: Load::default(),
        }
    }

    /// A deposit, a deed, a market fill with a refund, a P2P trade, and a
    /// newcomer: all explained, so nothing drifts.
    #[test]
    fn every_known_type_has_a_flow_in_the_order_they_stack() {
        let flows: Vec<StrandFlow> = TRANSACTION_TYPES.iter().map(|t| StrandFlow::of(t)).collect();
        assert!(!flows.contains(&StrandFlow::Other));
        let order = |f: &StrandFlow| [StrandFlow::Faucet, StrandFlow::Sink, StrandFlow::Transfer, StrandFlow::Escrow].iter().position(|g| g == f);
        assert!(flows.windows(2).all(|w| order(&w[0]) <= order(&w[1])));
    }

    #[test]
    fn strands_explained_by_their_trails_hold() {
        let start = ledger(
            vec![
                account(1, "Ada Reeder", 400, &[("exchange_deposit", 300)]),
                account(2, "Percy Sawyer", 100, &[]),
            ],
            0,
        );
        // Ada: +200 deposit, −250 deed, bids 10 logs at 6 (60 in escrow), and
        // gets 5 back after a fill at 5 less fee 1; sends Percy 20.
        // Percy: sells her 10 logs for 50, gets 20.
        // Maud: new, untouched.
        let end = ledger(
            vec![
                account(
                    1,
                    "Ada Reeder",
                    400 + 200 - 250 - 60 - 1 + 10 - 20,
                    &[
                        ("exchange_deposit", 500),
                        ("deed_purchase", -250),
                        ("market_escrow", -60),
                        ("market_fee", -1),
                        ("market_refund", 10),
                        ("trade_sent", -20),
                    ],
                ),
                account(2, "Percy Sawyer", 100 + 50 + 20, &[("market_sale", 50), ("trade_received", 20)]),
                account(3, "Maud Mason", STARTING_STRANDS, &[]),
            ],
            0,
        );
        let books = reconcile(&start, &end).strands;
        assert!(books.holds(), "{books:?}");
        assert_eq!(books.new_accounts, 1);
        assert_eq!((books.faucets, books.sinks, books.transfers), (200, -251, 0));
        assert_eq!(books.end_total - books.start_total, 100 + 200 - 251);
    }

    /// The court's money (S0.2): a payout leaves the holding for someone who
    /// never put it in, and the books still hold.
    #[test]
    fn the_courts_escrow_and_payouts_hold() {
        let start = ledger(vec![account(1, "Ada Reeder", 500, &[]), account(2, "Percy Sawyer", 100, &[])], 40);
        // Ada posts a commission worth 200, at the court's fee of 5 + 1; Percy
        // delivers 3 of its 5 units and is paid 120; Ada cancels the rest and
        // gets 80 back. A guild of Percy's takes 30 into its treasury.
        let end = ledger(
            vec![
                account(
                    1,
                    "Ada Reeder",
                    500 - 200 - 6 + 80,
                    &[("court_escrow", -200), ("court_fee", -6), ("court_refund", 80)],
                ),
                account(2, "Percy Sawyer", 100 + 120 - 30, &[("court_payout", 120), ("guild_deposit", -30)]),
            ],
            40 + 30,
        );
        let books = reconcile(&start, &end).strands;
        assert!(books.holds(), "{books:?}");
        assert_eq!((books.sinks, books.transfers, books.escrow_drift), (-6, 0, 0));
    }

    /// A guild's treasury (S4.2): a member's deposit goes in, an officer's
    /// draw comes out, and the week's dues and a roster fee leave it for no
    /// one, written in the treasury's own books.
    #[test]
    fn a_treasurys_dues_are_sinks_from_the_holding() {
        let start = ledger(vec![account(1, "Ada Reeder", 500, &[])], 0);
        let mut end = ledger(
            vec![account(1, "Ada Reeder", 500 - 200 + 40, &[("guild_deposit", -200), ("guild_withdrawal", 40)])],
            200 - 40 - 55 - 1,
        );
        end.held_flows = [("guild_dues".to_string(), -55), ("guild_fee".to_string(), -1)].into();
        let books = reconcile(&start, &end).strands;
        assert!(books.holds(), "{books:?}");
        assert_eq!((books.sinks, books.escrow_drift), (-56, 0));
        assert_eq!(books.flows["guild_dues"], -55);
        // Dues taken with nothing written are drift, in the world and in escrow
        end.held_flows.clear();
        let books = reconcile(&start, &end).strands;
        assert_eq!((books.drift, books.escrow_drift), (-56, -56));
    }

    #[test]
    fn a_write_without_a_trail_is_drift() {
        let start = ledger(vec![account(1, "Ada Reeder", 400, &[])], 0);
        // The hourly tax took 5 and wrote nothing down
        let end = ledger(vec![account(1, "Ada Reeder", 395, &[])], 0);
        let books = reconcile(&start, &end).strands;
        assert!(!books.holds());
        assert_eq!(books.accounts, vec![("Ada Reeder".to_string(), -5)]);
        assert_eq!(books.drift, -5);
        // Escrow that changed with no escrow rows
        let books = reconcile(&ledger(vec![], 0), &ledger(vec![], 7)).strands;
        assert_eq!((books.escrow_drift, books.drift), (7, 7));
        // An account that vanished took its Strands with it
        let books = reconcile(&start, &ledger(vec![], 0)).strands;
        assert_eq!(books.gone, vec![("Ada Reeder".to_string(), 400)]);
        assert!(!books.holds());
    }

    #[test]
    fn a_trade_with_one_half_is_drift() {
        let start = ledger(vec![account(1, "Ada Reeder", 100, &[]), account(2, "Percy Sawyer", 100, &[])], 0);
        let end = ledger(
            vec![account(1, "Ada Reeder", 80, &[("trade_sent", -20)]), account(2, "Percy Sawyer", 100, &[])],
            0,
        );
        let books = reconcile(&start, &end).strands;
        assert!(books.accounts.is_empty(), "Ada's trail explains her");
        assert_eq!(books.transfers, -20);
        assert!(!books.holds());
    }

    fn units(l: &mut Ledger, held: &[(&str, i64)], flows: &[(&str, UnitFlow, i64)]) {
        l.units = held.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        for (item, flow, n) in flows {
            l.unit_flows.entry(item.to_string()).or_default().insert(flow.as_str().into(), *n);
        }
    }

    #[test]
    fn units_explained_by_their_flows_hold_and_others_drift() {
        let mut start = ledger(vec![], 0);
        units(&mut start, &[("log", 10)], &[("log", UnitFlow::Gathered, 40)]);
        let mut end = ledger(vec![], 0);
        // 12 logs gathered, 6 into planks; 3 planks made; a fiber gathered
        // and deposited; and 2 stone from nowhere
        units(
            &mut end,
            &[("log", 16), ("plank", 3), ("stone", 2)],
            &[
                ("log", UnitFlow::Gathered, 52),
                ("log", UnitFlow::Consumed, -6),
                ("plank", UnitFlow::Crafted, 3),
                ("fiber", UnitFlow::Gathered, 1),
                ("fiber", UnitFlow::Deposited, -1),
            ],
        );
        let books = reconcile(&start, &end).units;
        assert_eq!(books.drift, vec![("stone".to_string(), 2)]);
        assert_eq!(books.items["log"], (6, 6));
        assert_eq!(books.flows["gathered"], 13);
        assert!(!books.items.contains_key("fiber") || books.items["fiber"] == (0, 0));
        assert!(!books.holds());
        // A restart starts the flows again: no verdict on units
        end.booted_ms = 2;
        let books = reconcile(&start, &end).units;
        assert!(books.void && books.drift.is_empty() && !books.holds());
    }

    #[test]
    fn every_flow_names_itself_and_its_side() {
        let names: std::collections::HashSet<&str> = UnitFlow::ALL.iter().map(|f| f.as_str()).collect();
        assert_eq!(names.len(), UnitFlow::ALL.len());
        assert!(UnitFlow::Gathered.is_faucet() && !UnitFlow::Consumed.is_faucet());
        assert_eq!(StrandFlow::of("property_tax"), StrandFlow::Sink);
        assert_eq!(StrandFlow::of("bazaar_sale"), StrandFlow::Transfer);
        assert_eq!(StrandFlow::of("something_new"), StrandFlow::Other);
        // The commons' types (S0.2): fees are sinks, and everything into or
        // out of the court's escrow or a treasury is escrow, payouts too
        for sink in ["court_fee", "postage", "guild_fee", "guild_dues"] {
            assert_eq!(StrandFlow::of(sink), StrandFlow::Sink, "{sink}");
        }
        for held in ["court_escrow", "court_payout", "court_refund", "guild_deposit", "guild_withdrawal"] {
            assert_eq!(StrandFlow::of(held), StrandFlow::Escrow, "{held}");
        }
        let known: std::collections::HashSet<&str> = TRANSACTION_TYPES.iter().copied().collect();
        assert_eq!(known.len(), TRANSACTION_TYPES.len(), "a type listed twice");
    }

    #[test]
    fn a_load_window_reads_its_ticks_from_the_buckets() {
        let mut a = Load {
            tick_period_ms: 10.0,
            tick_buckets: vec![0; TICK_BUCKETS_MS.len() + 1],
            db_transactions: 1000,
            ..Load::default()
        };
        a.sockets_ended.insert("vanished".into(), 2);
        let mut b = a.clone();
        // 100 ticks: 90 just under 2 ms, 8 just under 10, 1 just under 20
        // and 1 past a second
        b.ticks = 100;
        b.tick_secs = 0.5;
        b.tick_buckets[tick_bucket(2.0)] = 90;
        b.tick_buckets[tick_bucket(10.0)] = 8;
        b.tick_buckets[tick_bucket(20.0)] = 1;
        b.tick_buckets[tick_bucket(5000.0)] = 1;
        b.db_transactions = 1600;
        b.sockets_ended.insert("vanished".into(), 3);
        b.sockets_ended.insert("closed".into(), 9);
        let w = LoadWindow::between(&a, &b, 60.0);
        assert_eq!(w.ticks, 100);
        assert!((w.tick_mean_ms - 5.0).abs() < 1e-9);
        // The 50th of 90 in 1.9375 to 2 ms, the 5th of 8 in 9.5 to 10, and the
        // 99th the top of its bucket
        assert_eq!((w.tick_p50_ms, w.tick_p95_ms, w.tick_p99_ms), (Some(1.972), Some(9.813), Some(20.0)));
        assert_eq!(w.ticks_over, 2, "the 20 ms tick and the slow one");
        assert_eq!(w.sockets_dropped, 1);
        assert!((w.db_transactions_per_sec - 10.0).abs() < 1e-9);
        assert_eq!((w.commands, w.command_queue_p99), (0, None), "a server before D3.1 has no queue to read");
    }

    /// One bucket to 1/16 ms, then each doubling cut in 16 to 1024 ms, so
    /// a bucket is at most a 16th of its value wide (#409)
    #[test]
    fn the_tick_buckets_are_log_linear() {
        assert_eq!(TICK_BUCKETS_MS[0], 0.0625);
        assert_eq!(TICK_BUCKETS_MS[TICK_BUCKETS_MS.len() - 1], 1024.0);
        for pair in TICK_BUCKETS_MS.windows(2) {
            assert!(pair[1] > pair[0]);
            assert!(pair[1] - pair[0] <= pair[1] / 16.0 + 1e-12, "{pair:?}");
        }
        let at = |ms: f64| TICK_BUCKETS_MS.iter().position(|b| *b == ms);
        assert_eq!(at(21.0).unwrap() - at(20.0).unwrap(), 1, "a millisecond apart from 16 to 32");
        assert_eq!(at(42.0).unwrap() - at(40.0).unwrap(), 1, "two from 32 to 64");
        assert!(at(100.0).is_some() && at(10.0).is_some(), "the periods at 1× and 10× are bounds");
        // A run on a bound counts under it; past the last, in the one after
        assert_eq!(tick_bucket(0.0), 0);
        assert_eq!(tick_bucket(0.0625), 0);
        assert_eq!(tick_bucket(0.065), 1, "1/16 to 17/256 ms");
        assert_eq!(tick_bucket(20.0), at(20.0).unwrap());
        assert_eq!(tick_bucket(20.001), at(21.0).unwrap());
        assert_eq!(tick_bucket(1024.0), TICK_BUCKETS_MS.len() - 1);
        assert_eq!(tick_bucket(1024.5), TICK_BUCKETS_MS.len());
    }

    /// The question #409 asks: a loaded town's ticks spread from 15 to
    /// 45 ms, then the same ticks half a millisecond slower. The p95 moves
    /// by the half millisecond, where the old buckets read 50 both times.
    #[test]
    fn a_half_millisecond_shows_at_p95_in_a_loaded_town() {
        let window = |shift: f64| {
            let mut b = Load { tick_period_ms: 100.0, tick_buckets: vec![0; TICK_BUCKETS_MS.len() + 1], ..Load::default() };
            for i in 0..10_000 {
                b.tick_buckets[tick_bucket(15.0 + 30.0 * i as f64 / 10_000.0 + shift)] += 1;
                b.ticks += 1;
            }
            LoadWindow::between(&Load::default(), &b, 60.0)
        };
        let (before, after) = (window(0.0).tick_p95_ms.unwrap(), window(0.5).tick_p95_ms.unwrap());
        assert!((before - 43.5).abs() < 0.05, "the true p95 is 43.5 ms, read {before}");
        assert!((after - before - 0.5).abs() < 0.05, "moved {}", after - before);
    }

    #[test]
    fn a_load_window_reads_the_command_queue_from_its_buckets() {
        let a = Load { command_buckets: vec![0; COMMAND_BUCKETS.len() + 1], ..Load::default() };
        let mut b = a.clone();
        // 100 ticks: 90 found nothing waiting, 9 up to 5 and 1 found 40
        b.command_buckets[0] = 90;
        b.command_buckets[3] = 9;
        b.command_buckets[6] = 1;
        b.commands = 9 * 4 + 40;
        let w = LoadWindow::between(&a, &b, 10.0);
        assert_eq!(w.commands, 76);
        assert_eq!(w.command_queue_p99, Some(5.0));
    }

    /// The writer's backlog is the window's end, and its lag the window's
    /// batches' mean (#407)
    #[test]
    fn a_load_window_reads_the_writers_backlog_and_lag() {
        let a = Load { writer_batches: 10, writer_lag_secs: 0.5, ..Load::default() };
        let quiet = LoadWindow::between(&a, &a, 10.0);
        assert_eq!((quiet.writer_queued, quiet.writer_lag_ms), (0, None), "nothing written, no lag to say");
        let b = Load { writer_queued: 7, writer_batches: 30, writer_lag_secs: 0.9, ..a.clone() };
        let w = LoadWindow::between(&a, &b, 10.0);
        assert_eq!(w.writer_queued, 7);
        assert!((w.writer_lag_ms.unwrap() - 20.0).abs() < 1e-9, "0.4 s over 20 batches");
    }
}
