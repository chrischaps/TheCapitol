//! The Observatory's feed (D0.3, #356): what `GET /admin/stream` sends.
//!
//! One broadcast for the whole process, apart from the game's event bus. The
//! sampler sends each [`Sample`](capitol_protocol::metrics::Sample) into it,
//! [`annotations`](super::annotations) each annotation once it's written,
//! and [`run_tap`] the few game events an admin watches for: market fills,
//! deeds changing hands, the Gazette, and warnings. Every admin's stream
//! subscribes here ([`subscribe`]), so the game's bus has one more reader
//! however many consoles are open.
//!
//! With nobody watching, [`send`] drops what it's given and the tap skips
//! its events unread.

use std::sync::{Arc, LazyLock};

use capitol_protocol::metrics::{DeedChange, Feed, WarningKind};
use capitol_protocol::rest::PauseReason;
use tokio::sync::broadcast::{self, error::RecvError};

use super::load::Receiver;
use super::GameEvent;

/// What a stream may fall behind by before it misses some. A sample every
/// ten seconds and a busy market's fills fit many times over.
pub const CAPACITY: usize = 1024;

static FEED: LazyLock<broadcast::Sender<Arc<Feed>>> = LazyLock::new(|| broadcast::channel(CAPACITY).0);

/// To every admin's stream, if any is open.
pub fn send(feed: Feed) {
    if FEED.receiver_count() > 0 {
        let _ = FEED.send(Arc::new(feed));
    }
}

/// A new admin stream's receiver: what's sent from now on.
pub fn subscribe() -> broadcast::Receiver<Arc<Feed>> {
    FEED.subscribe()
}

/// Pass the game events an admin watches for on to the feed, until the
/// game's bus closes.
pub async fn run_tap(mut events: broadcast::Receiver<GameEvent>) {
    loop {
        match events.recv().await {
            Ok(event) => {
                if FEED.receiver_count() > 0 {
                    if let Some(feed) = tap(&event) {
                        send(feed);
                    }
                }
            }
            Err(RecvError::Lagged(missed)) => Receiver::AdminTap.lagged(missed),
            Err(RecvError::Closed) => return,
        }
    }
}

/// The feed's account of `event`, if it's one an admin watches for.
pub fn tap(event: &GameEvent) -> Option<Feed> {
    let warning = |player_id, kind, text: String| Some(Feed::Warning { player_id, kind, text });
    match event {
        // Each fill goes out three times: once to each party, and once to
        // the item's channel. That one is the fill.
        GameEvent::MarketTrade { trade } => Some(Feed::Fill { trade: trade.clone() }),
        GameEvent::PlotClaimed { terrain_id, owner_id, .. } => Some(Feed::Deed {
            terrain_id: terrain_id.clone(),
            change: DeedChange::Claimed,
            owner_id: Some(*owner_id),
        }),
        GameEvent::PlotReleased { terrain_id } => Some(Feed::Deed {
            terrain_id: terrain_id.clone(),
            change: DeedChange::Released,
            owner_id: None,
        }),
        GameEvent::PlotForeclosed { terrain_id, owner_id, .. } => Some(Feed::Deed {
            terrain_id: terrain_id.clone(),
            change: DeedChange::Foreclosed,
            owner_id: Some(*owner_id),
        }),
        GameEvent::GazettePublished { day, number, headline } => {
            Some(Feed::Gazette { day: *day, number: *number, headline: headline.clone() })
        }
        GameEvent::TaxDelinquent { owner_id, terrain_id, tax_owed, .. } => warning(
            *owner_id,
            WarningKind::TaxDelinquent,
            format!("{tax_owed} Strands of tax unpaid on {terrain_id}"),
        ),
        GameEvent::SubscriptionLapsed { player_id, until, .. } => warning(
            *player_id,
            WarningKind::SubscriptionLapsed,
            format!("A subscription ran out at {}", until.format("%Y-%m-%d %H:%M UTC")),
        ),
        GameEvent::StationDisplaced { owner_id, station_type, terrain_id, .. } => warning(
            *owner_id,
            WarningKind::StationDisplaced,
            format!("A {station_type} was taken down off {terrain_id}"),
        ),
        // A station pausing because its owner went home is the usual end of
        // a day, not a warning
        GameEvent::AutomationPaused { reason: PauseReason::OwnerOffline | PauseReason::Shift, .. } => None,
        GameEvent::AutomationPaused { owner_id, station_type, reason, .. } => {
            let why = match reason {
                PauseReason::MissingInput { item_type } => format!("its box has no {item_type}"),
                PauseReason::OutputFull => "its box is full".to_string(),
                PauseReason::ToolBroken => "its tool broke".to_string(),
                PauseReason::ToolMissing => "it has no tool".to_string(),
                PauseReason::OwnerOffline => "its owner left".to_string(),
                PauseReason::Shift => "someone is working it".to_string(),
            };
            warning(*owner_id, WarningKind::AutomationPaused, format!("A {station_type} paused: {why}"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn the_tap_takes_fills_deeds_the_gazette_and_warnings_only() {
        let ada = Uuid::from_u128(1);
        let feed = tap(&GameEvent::PlotClaimed { terrain_id: "farm_3".into(), owner_id: ada, claimed_at: chrono::Utc::now() });
        assert_eq!(feed, Some(Feed::Deed { terrain_id: "farm_3".into(), change: DeedChange::Claimed, owner_id: Some(ada) }));
        let gazette = tap(&GameEvent::GazettePublished { day: 4, number: 5, headline: "Rope at a record".into() });
        assert_eq!(gazette.map(|f| f.type_name()), Some("gazette"));
        let paused = |reason| GameEvent::AutomationPaused {
            owner_id: ada,
            station_id: Uuid::nil(),
            station_type: "loom".into(),
            reason,
            notice_id: None,
        };
        let Some(Feed::Warning { kind, text, .. }) = tap(&paused(PauseReason::OutputFull)) else {
            panic!("a full box is a warning");
        };
        assert_eq!((kind, text.as_str()), (WarningKind::AutomationPaused, "A loom paused: its box is full"));
        assert_eq!(tap(&paused(PauseReason::OwnerOffline)), None);
        assert_eq!(tap(&GameEvent::TerrainUpdated), None);
        assert_eq!(tap(&GameEvent::CurrencyChanged { player_id: ada, new_balance: 3 }), None);
    }

    #[tokio::test]
    async fn the_feed_reaches_every_stream_and_nothing_waits_for_none() {
        // Nobody watching: dropped, not queued for the next to subscribe
        send(Feed::Lagged { missed: 1 });
        let mut one = subscribe();
        let mut two = subscribe();
        send(Feed::Lagged { missed: 2 });
        // The feed is the process's, so another test may send between
        for rx in [&mut one, &mut two] {
            loop {
                match &*rx.recv().await.unwrap() {
                    Feed::Lagged { missed: 1 } => panic!("a stream heard what was sent before it opened"),
                    Feed::Lagged { missed: 2 } => break,
                    _ => {}
                }
            }
        }
    }
}
