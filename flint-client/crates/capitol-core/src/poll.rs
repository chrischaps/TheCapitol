//! The polling scheduler. The server never sends `currency_changed`,
//! `nearby_nodes`, `nearby_stations`, `item_moved` or `items_merged`, so the
//! client refetches over REST instead, the way React does. [`Poller`] decides
//! *what* to fetch and *when*; it never touches the network itself.
//!
//! ```no_run
//! # use std::time::Instant;
//! # use capitol_core::{net::*, poll::*, state::*};
//! # let net = Net::spawn(NetConfig::default()).unwrap();
//! let mut state = GameState::new();
//! let mut poller = Poller::new();
//! // once per frame:
//! for event in net.drain() {
//!     let changes = state.apply(&event);
//!     if let Some(Fetch::Terrain) = poller.observe(&event, &changes, &state) {
//!         // rebuild the world from the `/terrain` answer in `event`
//!     }
//! }
//! for (request, tag) in poller.tick(Instant::now()) {
//!     net.rest(request, tag);
//! }
//! ```
//!
//! **The rules** (plan §4.5):
//!
//! | Trigger | Fetch |
//! |---|---|
//! | first `Connected`, every `Resync` | me (name, position, currency), containers, nearby nodes, nearby stations, plots, the catalog (recipes among it), everyone's claimed lots |
//! | every 5 s while connected | nearby nodes, nearby stations, plots, currency |
//! | `inventory_updated` (mine, or a container I hold) | containers, or the open station's container |
//! | `station_opened` (mine) | that station's container |
//! | `station_placed` (mine: a kit was used) | containers |
//! | extraction or crafting completed (mine) | containers |
//! | `trade_executed` (our trade) | containers, currency |
//! | a deposit or withdrawal answered | containers, currency |
//! | a REST `move_item` answered | containers, and the open station's container |
//! | `zone_changed` (mine, a real crossing) | plots |
//! | `plot_claimed` (mine, not yet in my plots) | plots |
//! | `plot_claimed` or `plot_released` while the claimed lots are in flight | claimed lots, again |
//! | `terrain_updated` | terrain |
//! | `/health` reports a catalog version the store doesn't have | the catalog |
//!
//! **Plots are your own** (`/plots?owner=<you>`, P6.4), as React lists
//! them since B7: placement tests against them and the map edges them in
//! gold. Any other plot is one `/plots/at`, which the plot panel asks for.
//! Everyone's claimed lots (`/plots/claimed`, B13) are fetched with the
//! snapshot only, never polled: WS `plot_claimed` and `plot_released` keep
//! the store's set current (P8.6b), and a resync fetches it whole again.
//! An event that lands while the list is in flight may predate the answer,
//! so it's sent once more.
//!
//! **One in flight per fetch.** Asking for something already queued does
//! nothing. Asking for something already *in flight* marks it to be sent once
//! more when the answer arrives, because that answer may predate the change.
//! It often does: the backend sends `extraction_completed` from the tick, then
//! writes the item and only then sends `inventory_updated`
//! (`backend/src/engine/tick.rs`, the loop after `game.tick()`). So however
//! many triggers land, a fetch has at most one request out and one waiting.
//!
//! **Failures aren't retried here.** The 5 s poll covers nodes, stations and
//! plots, and a dropped connection ends in a resync that refetches the rest.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use capitol_protocol::rest::{ListPlotsQuery, NearbyQuery};
use capitol_protocol::rules::NEARBY_NODE_RANGE_MAX;
use uuid::Uuid;

use crate::net::{ConnectionState, NetEvent, RestRequest, RestResponse, Tag};
use crate::state::{GameState, StateEvent, TradeEvent};

/// The first tag the poller uses. Tags count up from here, so the game's own
/// tags should stay below it.
pub const FIRST_TAG: Tag = 1 << 63;

/// How often nodes, stations and plots are polled. React uses 5 s.
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// How many of your plots to list, as React does: `/plots` pages at 100.
const PLOT_LIMIT: i32 = 100;

/// Something the client refetches because the server doesn't push it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fetch {
    /// `GET /player/me`
    Me,
    /// `GET /inventory/containers`
    Containers,
    /// `GET /stations/{id}/container`
    StationContainer { station_id: Uuid },
    /// `GET /world/nearby`
    NearbyNodes,
    /// `GET /stations/nearby`
    NearbyStations,
    /// `GET /player/currency`
    Currency,
    /// `GET /plots?owner=<you>&limit=100`
    Plots,
    /// `GET /catalog`: the static game data, recipes among it (E0.2)
    Catalog,
    /// `GET /plots/claimed`
    ClaimedPlots,
    /// `GET /terrain`
    Terrain,
}

impl Fetch {
    /// Everything the server doesn't push, refetched on connect and resync.
    /// `Me` comes first and carries the balance, so `Currency` isn't needed here.
    pub const SNAPSHOT: [Fetch; 7] = [
        Fetch::Me,
        Fetch::Containers,
        Fetch::NearbyNodes,
        Fetch::NearbyStations,
        Fetch::Plots,
        Fetch::Catalog,
        Fetch::ClaimedPlots,
    ];

    /// What the timer refetches. Currency is here for what the server
    /// changes without saying so, such as the hourly property tax (P4.5's
    /// header shows the balance).
    pub const POLLED: [Fetch; 4] = [
        Fetch::NearbyNodes,
        Fetch::NearbyStations,
        Fetch::Plots,
        Fetch::Currency,
    ];

    /// The call, for the player `me` (whose plots are listed).
    pub fn request(&self, me: Option<Uuid>) -> RestRequest {
        match *self {
            Fetch::Me => RestRequest::PlayerMe,
            Fetch::Containers => RestRequest::Containers,
            Fetch::StationContainer { station_id } => RestRequest::StationContainer { station_id },
            // The widest the server gives, so a meadow across the road is
            // already there at the district camera (#79).
            Fetch::NearbyNodes => RestRequest::WorldNearby(NearbyQuery {
                radius: Some(NEARBY_NODE_RANGE_MAX),
            }),
            Fetch::NearbyStations => RestRequest::NearbyStations,
            Fetch::Currency => RestRequest::PlayerCurrency,
            Fetch::Plots => RestRequest::ListPlots(ListPlotsQuery {
                owner: me,
                limit: Some(PLOT_LIMIT),
                ..Default::default()
            }),
            Fetch::Catalog => RestRequest::Catalog,
            Fetch::ClaimedPlots => RestRequest::ClaimedPlots,
            // `terrain_updated` means it changed, so there's nothing to revalidate.
            Fetch::Terrain => RestRequest::Terrain {
                cached_version: None,
            },
        }
    }
}

/// Turns what the store reports into REST calls. It keeps no clock of its
/// own: [`Poller::tick`] takes the time, so tests can drive it.
#[derive(Debug, Clone)]
pub struct Poller {
    interval: Duration,
    next_tag: Tag,
    /// Asked for and not yet sent, in the order asked.
    queued: Vec<Fetch>,
    in_flight: HashMap<Tag, Fetch>,
    /// Asked for again while in flight; queued when the answer arrives.
    again: Vec<Fetch>,
    /// The timer runs only while connected.
    connected: bool,
    /// `None` restarts the timer at the next tick.
    next_poll: Option<Instant>,
    /// The local player, from the store, whose plots are listed.
    me: Option<Uuid>,
}

impl Default for Poller {
    fn default() -> Self {
        Self::new()
    }
}

impl Poller {
    pub fn new() -> Self {
        Self::with_interval(POLL_INTERVAL)
    }

    pub fn with_interval(interval: Duration) -> Self {
        Self {
            interval,
            next_tag: FIRST_TAG,
            queued: Vec::new(),
            in_flight: HashMap::new(),
            again: Vec::new(),
            connected: false,
            next_poll: None,
            me: None,
        }
    }

    /// Asks for `fetch`, for anything the rules don't cover (a panel opening,
    /// a manual refresh). It goes out on the next [`tick`](Self::tick) unless
    /// one is already queued or in flight (see the module docs).
    pub fn request(&mut self, fetch: Fetch) {
        if self.in_flight.values().any(|f| *f == fetch) {
            if !self.again.contains(&fetch) {
                self.again.push(fetch);
            }
        } else if !self.queued.contains(&fetch) {
            self.queued.push(fetch);
        }
    }

    /// Reads one net event after the store has applied it: `changes` is what
    /// [`GameState::apply`] returned, and `state` is the state it left.
    /// Returns the fetch the event answered, if it was the reply to one.
    pub fn observe(
        &mut self,
        event: &NetEvent,
        changes: &[StateEvent],
        state: &GameState,
    ) -> Option<Fetch> {
        // Known from `Connected`, before the snapshot is asked for.
        self.me = state.me();
        for change in changes {
            self.observe_change(change, state);
        }
        let NetEvent::Rest(tag, result) = event else {
            return None;
        };
        if let Some(fetch) = self.in_flight.remove(tag) {
            if let Some(i) = self.again.iter().position(|f| *f == fetch) {
                self.again.remove(i);
                self.request(fetch);
            }
            return Some(fetch);
        }
        // The game's own calls. These change things the server never reports.
        match result {
            Ok(RestResponse::Deposit(_) | RestResponse::Withdraw(_)) => {
                self.request(Fetch::Containers);
                self.request(Fetch::Currency);
            }
            Ok(RestResponse::MoveItem(_)) => {
                self.request(Fetch::Containers);
                if let Some(open) = state.open_station() {
                    self.request(Fetch::StationContainer {
                        station_id: open.station_id,
                    });
                }
            }
            _ => {}
        }
        None
    }

    fn observe_change(&mut self, change: &StateEvent, state: &GameState) {
        match change {
            StateEvent::Connection(ConnectionState::Connected { .. }) => self.connected = true,
            StateEvent::Connection(connection) => {
                self.connected = false;
                self.next_poll = None;
                // Leaving a session drops whatever was waiting: the resync after
                // a reconnect refetches it. A fetch asked for before the first
                // connect (terrain on the login screen) survives `Connecting`.
                if matches!(
                    connection,
                    ConnectionState::Reconnecting { .. }
                        | ConnectionState::Disconnected { .. }
                        | ConnectionState::AuthFailed { .. }
                ) {
                    self.queued.clear();
                    self.again.clear();
                }
            }
            StateEvent::SnapshotStale => {
                for fetch in Fetch::SNAPSHOT {
                    self.request(fetch);
                }
                // The snapshot covers the polled fetches; the next poll is a full interval away.
                self.next_poll = None;
            }
            StateEvent::InventoryChanged { container_id } => {
                match state
                    .open_station()
                    .filter(|s| s.container_id == *container_id)
                {
                    Some(open) => self.request(Fetch::StationContainer {
                        station_id: open.station_id,
                    }),
                    None => self.request(Fetch::Containers),
                }
            }
            StateEvent::StationOpened { station_id, .. } => self.request(Fetch::StationContainer {
                station_id: *station_id,
            }),
            StateEvent::StationClosed => {
                // An answer already in flight is dropped by the store.
                let station = |f: &Fetch| matches!(f, Fetch::StationContainer { .. });
                self.queued.retain(|f| !station(f));
                self.again.retain(|f| !station(f));
            }
            StateEvent::StationPlaced { owner_id, .. } if state.me() == Some(*owner_id) => {
                self.request(Fetch::Containers)
            }
            StateEvent::ExtractionCompleted { .. } | StateEvent::CraftingCompleted { .. } => {
                self.request(Fetch::Containers)
            }
            // Only our own trade: the store drops other players' `trade_executed`.
            // Nothing is held in escrow, so a cancel changes nothing.
            StateEvent::Trade(TradeEvent::Executed { .. }) => {
                self.request(Fetch::Containers);
                self.request(Fetch::Currency);
            }
            // The first zone (`from: None`) arrives with the snapshot, which fetches plots.
            StateEvent::ZoneChanged { from: Some(_), .. } => self.request(Fetch::Plots),
            StateEvent::TerrainUpdated => self.request(Fetch::Terrain),
            StateEvent::CatalogStale => self.request(Fetch::Catalog),
            // A claim of yours we didn't make through this client's REST
            // (another client, say). A release of yours the store applies itself.
            StateEvent::LotClaimed(lot)
                if state.me() == Some(lot.owner_id)
                    && !state
                        .plots()
                        .iter()
                        .any(|p| p.terrain_id.as_deref() == Some(lot.terrain_id.as_str())) =>
            {
                self.request(Fetch::Plots)
            }
            _ => {}
        }
        // Not `request`: with nothing in flight, the store already has it.
        if matches!(
            change,
            StateEvent::LotClaimed(_) | StateEvent::LotReleased { .. }
        ) && self.in_flight.values().any(|f| *f == Fetch::ClaimedPlots)
            && !self.again.contains(&Fetch::ClaimedPlots)
        {
            self.again.push(Fetch::ClaimedPlots);
        }
    }

    /// Runs the timer and returns everything due, each with the tag its
    /// answer will carry. Send them with [`Net::rest`](crate::net::Net::rest).
    pub fn tick(&mut self, now: Instant) -> Vec<(RestRequest, Tag)> {
        if self.connected {
            match self.next_poll {
                None => self.next_poll = Some(now + self.interval),
                Some(due) if now >= due => {
                    for fetch in Fetch::POLLED {
                        self.request(fetch);
                    }
                    // Keep the cadence, but after a stall start over rather than catch up.
                    let next = due + self.interval;
                    self.next_poll = Some(if next > now {
                        next
                    } else {
                        now + self.interval
                    });
                }
                Some(_) => {}
            }
        }
        self.queued
            .drain(..)
            .map(|fetch| {
                let tag = self.next_tag;
                self.next_tag += 1;
                self.in_flight.insert(tag, fetch);
                (fetch.request(self.me), tag)
            })
            .collect()
    }

    /// Whether `fetch` is queued, in flight or waiting to go again.
    pub fn is_pending(&self, fetch: Fetch) -> bool {
        self.queued.contains(&fetch)
            || self.again.contains(&fetch)
            || self.in_flight.values().any(|f| *f == fetch)
    }

    /// Nothing queued or in flight.
    pub fn is_idle(&self) -> bool {
        self.queued.is_empty() && self.in_flight.is_empty()
    }

    /// The fetch a tag belongs to, while its answer is outstanding.
    pub fn in_flight(&self, tag: Tag) -> Option<Fetch> {
        self.in_flight.get(&tag).copied()
    }
}

#[cfg(test)]
mod tests;
