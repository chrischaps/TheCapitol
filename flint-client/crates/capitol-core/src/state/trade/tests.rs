//! Trade machine tests: one per transition, through the store's real entry
//! points. The `pub(in crate::state)` ones are also the per-variant tests in
//! `state/tests.rs`.

use std::time::{Duration, Instant};

use capitol_protocol::ws::{PlayerPosition, ServerMessage as M, TradeItemInfo, TradeOfferItem};

use super::*;
use crate::net::{ConnectionState, NetEvent};

const ME: Uuid = Uuid::from_u128(1);
/// Next to me, in range.
const BRIN: Uuid = Uuid::from_u128(2);
/// Next to me too.
const CATO: Uuid = Uuid::from_u128(3);
/// Across the map.
const FAR: Uuid = Uuid::from_u128(4);
const TRADE: Uuid = Uuid::from_u128(60);
const OTHER_TRADE: Uuid = Uuid::from_u128(61);
const ITEM: Uuid = Uuid::from_u128(70);

fn pos(id: Uuid, name: &str, x: f64, y: f64) -> PlayerPosition {
    PlayerPosition {
        id,
        name: name.into(),
        x,
        y,
    }
}

fn everyone() -> Vec<PlayerPosition> {
    vec![
        pos(ME, "Ada", 4000.0, 4000.0),
        pos(BRIN, "Brin", 4040.0, 4000.0),
        pos(CATO, "Cato", 4000.0, 4050.0),
        pos(FAR, "Fen", 6000.0, 4000.0),
    ]
}

/// Connected as `ME` with three other players about.
fn world() -> GameState {
    let mut state = GameState::new();
    state.apply_connection(&ConnectionState::Connected { player_id: ME });
    state.apply_ws(&M::Positions {
        players: everyone(),
    });
    state
}

fn started(trade_id: Uuid, partner_id: Uuid, partner_name: &str) -> M {
    M::TradeStarted {
        trade_id,
        partner_id,
        partner_name: partner_name.into(),
    }
}

/// Trading with Brin.
fn trading() -> GameState {
    let mut state = world();
    state.apply_ws(&started(TRADE, BRIN, "Brin"));
    state
}

fn requested_by(from: Uuid, name: &str) -> M {
    M::TradeRequested {
        from_player: from,
        from_player_name: name.into(),
    }
}

fn offer_updated(trade_id: Uuid, player_id: Uuid, strands: i64) -> M {
    M::TradeOfferUpdated {
        trade_id,
        player_id,
        items: vec![TradeItemInfo {
            item_id: ITEM,
            item_type: "rope".into(),
            item_name: "Rope".into(),
            quantity: 1,
            quality: 80,
        }],
        strands,
    }
}

fn accepted(trade_id: Uuid, player_id: Uuid) -> M {
    M::TradeAccepted {
        trade_id,
        player_id,
    }
}

fn cancelled(trade_id: Uuid, reason: &str) -> M {
    M::TradeCancelled {
        trade_id,
        reason: reason.into(),
    }
}

/// Only the trade events, in order.
fn trades(events: Vec<StateEvent>) -> Vec<TradeEvent> {
    events
        .into_iter()
        .filter_map(|e| match e {
            StateEvent::Trade(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn ws(state: &mut GameState, message: M) -> Vec<TradeEvent> {
    trades(state.apply_ws(&message))
}

fn open(state: &GameState) -> &OpenTrade {
    state.open_trade().expect("an open trade")
}

fn cancelled_event(trade_id: Uuid, reason: &str) -> TradeEvent {
    TradeEvent::Cancelled {
        trade_id,
        reason: reason.into(),
    }
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[test]
fn initiate_asks_and_waits() {
    let mut state = world();
    let now = Instant::now();
    assert_eq!(
        state.trade_initiate(now, BRIN),
        Ok(ClientMessage::TradeInitiate {
            target_player_id: BRIN.to_string()
        })
    );
    assert_eq!(
        state.trade(),
        &Trade::Requested {
            to: BRIN,
            name: "Brin".into()
        }
    );
    assert_eq!(state.trade_waiting(), Some(TradeAction::Initiate));
}

#[test]
fn initiate_refuses_what_the_server_would_swallow() {
    let now = Instant::now();
    let mut state = world();
    assert_eq!(state.trade_initiate(now, ME), Err(TradeRefusal::Yourself));
    assert_eq!(
        state.trade_initiate(now, Uuid::from_u128(99)),
        Err(TradeRefusal::Offline)
    );
    assert_eq!(
        state.trade_initiate(now, FAR),
        Err(TradeRefusal::OutOfRange { distance: 2000.0 })
    );
    state.trade_initiate(now, BRIN).unwrap();
    assert_eq!(
        state.trade_initiate(now, BRIN),
        Err(TradeRefusal::AlreadyAsked)
    );
    assert_eq!(
        state.trade(),
        &Trade::Requested {
            to: BRIN,
            name: "Brin".into()
        }
    );

    let mut state = trading();
    assert_eq!(
        state.trade_initiate(now, CATO),
        Err(TradeRefusal::AlreadyTrading)
    );

    let mut state = GameState::new();
    assert_eq!(
        state.trade_initiate(now, BRIN),
        Err(TradeRefusal::NotConnected)
    );
    let mut state = world();
    state.apply_connection(&ConnectionState::Reconnecting {
        attempt: 1,
        delay: Duration::from_millis(500),
        reason: "reset".into(),
    });
    assert_eq!(
        state.trade_initiate(now, BRIN),
        Err(TradeRefusal::NotConnected)
    );
}

#[test]
fn asking_someone_else_replaces_our_request() {
    let now = Instant::now();
    let mut state = world();
    state.trade_initiate(now, BRIN).unwrap();
    state.trade_initiate(now, CATO).unwrap();
    assert_eq!(
        state.trade(),
        &Trade::Requested {
            to: CATO,
            name: "Cato".into()
        }
    );
}

/// The server never tells the asker a request arrived, so silence is normal.
/// The hint comes once and the request stays open.
#[test]
fn an_unanswered_request_hints_but_stays_open() {
    let now = Instant::now();
    let mut state = world();
    state.trade_initiate(now, BRIN).unwrap();
    assert!(state
        .tick(now + REPLY_TIMEOUT - Duration::from_millis(1))
        .is_empty());
    assert_eq!(
        trades(state.tick(now + REPLY_TIMEOUT)),
        vec![TradeEvent::NoReply(TradeAction::Initiate)]
    );
    assert!(state.tick(now + REPLY_TIMEOUT * 3).is_empty());
    assert!(matches!(state.trade(), Trade::Requested { to: BRIN, .. }));
    assert_eq!(state.trade_waiting(), None);

    // Brin answers late; it still opens.
    assert_eq!(
        ws(&mut state, started(TRADE, BRIN, "Brin")),
        vec![TradeEvent::Started {
            trade_id: TRADE,
            partner_id: BRIN
        }]
    );
}

#[test]
fn withdraw_forgets_our_request() {
    let now = Instant::now();
    let mut state = world();
    assert_eq!(state.trade_withdraw(), Err(TradeRefusal::NoRequest));
    state.trade_initiate(now, BRIN).unwrap();
    assert_eq!(state.trade_withdraw(), Ok(()));
    assert_eq!(state.trade(), &Trade::Idle);
    assert!(state.tick(now + REPLY_TIMEOUT).is_empty());
}

#[test]
pub(in crate::state) fn trade_requested_prompts() {
    let mut state = world();
    assert_eq!(
        ws(&mut state, requested_by(BRIN, "Brin")),
        vec![TradeEvent::Requested {
            from: BRIN,
            name: "Brin".into()
        }]
    );
    assert_eq!(
        state.trade(),
        &Trade::Incoming {
            from: BRIN,
            name: "Brin".into()
        }
    );

    // The server keeps one request per target, the newest.
    ws(&mut state, requested_by(CATO, "Cato"));
    assert_eq!(
        state.trade(),
        &Trade::Incoming {
            from: CATO,
            name: "Cato".into()
        }
    );

    // It also replaces a request of ours to someone else (a later
    // `trade_started` for that one would still open).
    let mut state = world();
    state.trade_initiate(Instant::now(), CATO).unwrap();
    ws(&mut state, requested_by(BRIN, "Brin"));
    assert!(matches!(state.trade(), Trade::Incoming { from: BRIN, .. }));
    assert_eq!(state.trade_waiting(), None);

    // A player who's trading can't be asked.
    let mut state = trading();
    assert!(ws(&mut state, requested_by(CATO, "Cato")).is_empty());
    assert!(state.open_trade().is_some());
}

/// We asked Brin while Brin asked us. The server starts the trade the moment
/// the second request lands (`TradeManager::request_trade`), so the client
/// shows no prompt and waits for `trade_started`.
#[test]
fn simultaneous_requests_start_without_a_prompt() {
    let now = Instant::now();
    let mut state = world();
    state.trade_initiate(now, BRIN).unwrap();
    assert!(ws(&mut state, requested_by(BRIN, "Brin")).is_empty());
    assert!(matches!(state.trade(), Trade::Requested { to: BRIN, .. }));
    ws(&mut state, started(TRADE, BRIN, "Brin"));
    assert_eq!(open(&state).partner_id, BRIN);
    assert_eq!(state.trade_waiting(), None);
}

/// Brin asked first; we ask Brin back instead of accepting. The server treats
/// that as an accept, and so does the client: it sends the request, which the
/// server turns into a trade.
#[test]
fn asking_someone_who_asked_us_starts_the_trade() {
    let now = Instant::now();
    let mut state = world();
    ws(&mut state, requested_by(BRIN, "Brin"));
    assert_eq!(
        state.trade_initiate(now, BRIN),
        Ok(ClientMessage::TradeInitiate {
            target_player_id: BRIN.to_string()
        })
    );
    assert!(matches!(state.trade(), Trade::Requested { to: BRIN, .. }));
    ws(&mut state, started(TRADE, BRIN, "Brin"));
    assert_eq!(open(&state).trade_id, TRADE);
}

#[test]
fn accept_request_opens_on_trade_started() {
    let now = Instant::now();
    let mut state = world();
    assert_eq!(
        state.trade_accept_request(now),
        Err(TradeRefusal::NoRequest)
    );
    ws(&mut state, requested_by(BRIN, "Brin"));
    assert_eq!(
        state.trade_accept_request(now),
        Ok(ClientMessage::TradeAcceptRequest)
    );
    assert_eq!(state.trade_waiting(), Some(TradeAction::AcceptRequest));
    ws(&mut state, started(TRADE, BRIN, "Brin"));
    assert_eq!(
        open(&state),
        &OpenTrade {
            trade_id: TRADE,
            partner_id: BRIN,
            partner_name: "Brin".into(),
            mine: Offer::default(),
            theirs: Offer::default(),
            i_accepted: false,
            they_accepted: false,
        }
    );
    assert_eq!(state.trade_waiting(), None);
    assert!(state.tick(now + REPLY_TIMEOUT).is_empty());
}

/// The request was gone (Brin left and we missed it) or Brin is now trading.
#[test]
fn accept_request_without_an_answer_drops_the_request() {
    let now = Instant::now();
    let mut state = world();
    ws(&mut state, requested_by(BRIN, "Brin"));
    state.trade_accept_request(now).unwrap();
    assert_eq!(
        trades(state.tick(now + REPLY_TIMEOUT)),
        vec![
            TradeEvent::RequestGone { player_id: BRIN },
            TradeEvent::NoReply(TradeAction::AcceptRequest)
        ]
    );
    assert_eq!(state.trade(), &Trade::Idle);
}

#[test]
fn decline_request_goes_idle_at_once() {
    let mut state = world();
    assert_eq!(state.trade_decline_request(), Err(TradeRefusal::NoRequest));
    ws(&mut state, requested_by(BRIN, "Brin"));
    assert_eq!(
        state.trade_decline_request(),
        Ok(ClientMessage::TradeDeclineRequest)
    );
    assert_eq!(state.trade(), &Trade::Idle);
}

#[test]
pub(in crate::state) fn trade_request_declined_ends_our_request() {
    let now = Instant::now();
    let mut state = world();
    state.trade_initiate(now, BRIN).unwrap();
    assert_eq!(
        ws(&mut state, M::TradeRequestDeclined { by_player: BRIN }),
        vec![TradeEvent::Declined { by: BRIN }]
    );
    assert_eq!(state.trade(), &Trade::Idle);
    assert!(state.tick(now + REPLY_TIMEOUT).is_empty());

    // It's only ever sent to the asker, so it's reported even after a
    // withdraw, but it doesn't touch a different request.
    state.trade_initiate(now, CATO).unwrap();
    assert_eq!(
        ws(&mut state, M::TradeRequestDeclined { by_player: BRIN }),
        vec![TradeEvent::Declined { by: BRIN }]
    );
    assert!(matches!(state.trade(), Trade::Requested { to: CATO, .. }));
}

#[test]
fn requests_end_when_either_side_leaves() {
    let now = Instant::now();
    let mut state = world();
    state.trade_initiate(now, BRIN).unwrap();
    assert_eq!(
        ws(&mut state, M::PlayerLeft { player_id: BRIN }),
        vec![TradeEvent::RequestGone { player_id: BRIN }]
    );
    assert_eq!(state.trade(), &Trade::Idle);
    assert!(state.tick(now + REPLY_TIMEOUT).is_empty());

    let mut state = world();
    ws(&mut state, requested_by(CATO, "Cato"));
    // Someone else leaving changes nothing.
    assert!(ws(&mut state, M::PlayerLeft { player_id: FAR }).is_empty());
    // Missing from `positions` is a leave.
    let without_cato: Vec<_> = everyone().into_iter().filter(|p| p.id != CATO).collect();
    assert_eq!(
        ws(
            &mut state,
            M::Positions {
                players: without_cato
            }
        ),
        vec![TradeEvent::RequestGone { player_id: CATO }]
    );
    assert_eq!(state.trade(), &Trade::Idle);
}

// ---------------------------------------------------------------------------
// The open trade
// ---------------------------------------------------------------------------

#[test]
pub(in crate::state) fn trade_started_opens_from_any_state() {
    let starts = |mut state: GameState| {
        assert_eq!(
            ws(&mut state, started(TRADE, BRIN, "Brin")),
            vec![TradeEvent::Started {
                trade_id: TRADE,
                partner_id: BRIN
            }]
        );
        assert_eq!(open(&state).partner_name, "Brin");
    };
    // Idle: a request we'd withdrawn, or one that timed out, was accepted.
    starts(world());
    let mut state = world();
    state.trade_initiate(Instant::now(), BRIN).unwrap();
    starts(state);
    let mut state = world();
    ws(&mut state, requested_by(BRIN, "Brin"));
    starts(state);
}

#[test]
fn offer_waits_for_its_echo() {
    let now = Instant::now();
    let mut state = trading();
    let items = vec![TradeOfferItem {
        item_id: ITEM.to_string(),
        quantity: 1,
    }];
    assert_eq!(
        state.trade_offer(now, items.clone(), 10),
        Ok(ClientMessage::TradeOffer { items, strands: 10 })
    );
    // Not applied until the server says so.
    assert_eq!(open(&state).mine, Offer::default());
    assert_eq!(state.trade_waiting(), Some(TradeAction::Offer));

    assert_eq!(
        ws(&mut state, offer_updated(TRADE, ME, 10)),
        vec![TradeEvent::OfferChanged { player_id: ME }]
    );
    assert_eq!(open(&state).mine.strands, 10);
    assert_eq!(open(&state).mine.items[0].item_name, "Rope");
    assert_eq!(open(&state).theirs, Offer::default());
    assert_eq!(state.trade_waiting(), None);
}

#[test]
fn an_unconfirmed_offer_hints_and_leaves_the_trade_as_it_was() {
    let now = Instant::now();
    let mut state = trading();
    state.trade_offer(now, vec![], 10).unwrap();
    assert_eq!(
        trades(state.tick(now + REPLY_TIMEOUT)),
        vec![TradeEvent::NoReply(TradeAction::Offer)]
    );
    assert_eq!(open(&state).mine, Offer::default());
}

/// A new offer from either side resets both accepts, as `update_offer` does.
#[test]
pub(in crate::state) fn offers_update_their_side_and_reset_accepts() {
    let mut state = trading();
    ws(&mut state, accepted(TRADE, ME));
    ws(&mut state, accepted(TRADE, BRIN));
    assert!(open(&state).i_accepted && open(&state).they_accepted);

    assert_eq!(
        ws(&mut state, offer_updated(TRADE, BRIN, 3)),
        vec![TradeEvent::OfferChanged { player_id: BRIN }]
    );
    assert_eq!(open(&state).theirs.strands, 3);
    assert!(!open(&state).i_accepted && !open(&state).they_accepted);

    ws(&mut state, accepted(TRADE, BRIN));
    ws(&mut state, offer_updated(TRADE, ME, 4));
    assert!(!open(&state).they_accepted);

    // Another trade's offer is someone else's business.
    assert!(ws(&mut state, offer_updated(OTHER_TRADE, BRIN, 99)).is_empty());
    assert_eq!(open(&state).theirs.strands, 3);
    let mut idle = world();
    assert!(ws(&mut idle, offer_updated(TRADE, BRIN, 99)).is_empty());
}

#[test]
pub(in crate::state) fn trade_accepted_marks_the_right_side() {
    let now = Instant::now();
    let mut state = trading();
    assert_eq!(state.trade_accept(now), Ok(ClientMessage::TradeAccept));
    assert_eq!(state.trade_waiting(), Some(TradeAction::Accept));
    assert_eq!(
        ws(&mut state, accepted(TRADE, ME)),
        vec![TradeEvent::Accepted { player_id: ME }]
    );
    assert!(open(&state).i_accepted && !open(&state).they_accepted);
    assert_eq!(state.trade_waiting(), None);

    ws(&mut state, accepted(TRADE, BRIN));
    assert!(open(&state).they_accepted);

    let mut state = trading();
    assert!(ws(&mut state, accepted(OTHER_TRADE, BRIN)).is_empty());
    assert!(!open(&state).they_accepted);
}

/// The server queues an offer for the next tick but handles an accept at once,
/// so accepting right after offering would be reset by our own offer.
#[test]
fn accept_waits_for_an_unconfirmed_offer() {
    let now = Instant::now();
    let mut state = trading();
    state.trade_offer(now, vec![], 5).unwrap();
    assert_eq!(state.trade_accept(now), Err(TradeRefusal::OfferUnconfirmed));
    ws(&mut state, offer_updated(TRADE, ME, 5));
    assert!(state.trade_accept(now).is_ok());
}

#[test]
fn an_unconfirmed_accept_hints() {
    let now = Instant::now();
    let mut state = trading();
    state.trade_accept(now).unwrap();
    assert_eq!(
        trades(state.tick(now + REPLY_TIMEOUT)),
        vec![TradeEvent::NoReply(TradeAction::Accept)]
    );
    assert!(!open(&state).i_accepted);
}

#[test]
fn trade_actions_need_an_open_trade() {
    let now = Instant::now();
    let mut state = world();
    assert_eq!(
        state.trade_offer(now, vec![], 1),
        Err(TradeRefusal::NotTrading)
    );
    assert_eq!(state.trade_accept(now), Err(TradeRefusal::NotTrading));
    assert_eq!(state.trade_cancel(now), Err(TradeRefusal::NotTrading));
    assert_eq!(state.trade_waiting(), None);
}

// ---------------------------------------------------------------------------
// Endings
// ---------------------------------------------------------------------------

#[test]
pub(in crate::state) fn trade_executed_counts_only_ours() {
    let now = Instant::now();
    let mut state = trading();
    // `trade_executed` goes to everyone.
    assert!(ws(
        &mut state,
        M::TradeExecuted {
            trade_id: OTHER_TRADE
        }
    )
    .is_empty());
    assert!(state.open_trade().is_some());

    // The final `trade_accepted` can be dropped (the trade may have left
    // `active_trades` when the server filters it), so one accept then the
    // execute is a normal ending.
    ws(&mut state, accepted(TRADE, BRIN));
    state.trade_accept(now).unwrap();
    assert_eq!(
        ws(&mut state, M::TradeExecuted { trade_id: TRADE }),
        vec![TradeEvent::Executed { trade_id: TRADE }]
    );
    assert_eq!(state.trade(), &Trade::Idle);
    assert_eq!(state.trade_waiting(), None);

    let mut idle = world();
    assert!(ws(&mut idle, M::TradeExecuted { trade_id: TRADE }).is_empty());
}

#[test]
pub(in crate::state) fn trade_cancelled_counts_only_ours() {
    let mut state = trading();
    // Someone else's cancel, with a real id.
    assert!(ws(&mut state, cancelled(OTHER_TRADE, "Cancelled by player")).is_empty());
    assert!(state.open_trade().is_some());

    assert_eq!(
        ws(&mut state, cancelled(TRADE, "Cancelled by player")),
        vec![cancelled_event(TRADE, "Cancelled by player")]
    );
    assert_eq!(state.trade(), &Trade::Idle);
    // Repeats and strangers' cancels after the end change nothing.
    assert!(ws(&mut state, cancelled(TRADE, "Cancelled by player")).is_empty());
}

/// A failed execute cancels with the real id and the server's reason.
#[test]
fn a_failed_execute_cancels_with_the_servers_reason() {
    let mut state = trading();
    ws(&mut state, accepted(TRADE, BRIN));
    ws(&mut state, accepted(TRADE, ME));
    assert_eq!(
        ws(
            &mut state,
            cancelled(TRADE, "Players moved out of trade range")
        ),
        vec![cancelled_event(TRADE, "Players moved out of trade range")]
    );
}

#[test]
fn cancel_closes_on_the_servers_confirmation() {
    let now = Instant::now();
    let mut state = trading();
    assert_eq!(state.trade_cancel(now), Ok(ClientMessage::TradeCancel));
    assert_eq!(state.trade_waiting(), Some(TradeAction::Cancel));
    assert!(state.open_trade().is_some(), "open until confirmed");
    assert_eq!(
        ws(&mut state, cancelled(TRADE, "Cancelled by player")),
        vec![cancelled_event(TRADE, "Cancelled by player")]
    );
    assert!(state.tick(now + REPLY_TIMEOUT).is_empty());
}

/// The server only fails a cancel when it has no trade for us, so an
/// unanswered cancel closes the trade anyway. If the trade had in fact just
/// executed, its late `trade_executed` must still trigger the refetch.
#[test]
fn an_unanswered_cancel_closes_and_a_late_execute_still_counts() {
    let now = Instant::now();
    let mut state = trading();
    state.trade_cancel(now).unwrap();
    assert_eq!(
        trades(state.tick(now + REPLY_TIMEOUT)),
        vec![cancelled_event(TRADE, CANCEL_UNCONFIRMED)]
    );
    assert_eq!(state.trade(), &Trade::Idle);
    assert_eq!(
        ws(&mut state, M::TradeExecuted { trade_id: TRADE }),
        vec![TradeEvent::Executed { trade_id: TRADE }]
    );
}

// ---------------------------------------------------------------------------
// Disconnects: the cancel with the real `trade_id` (B6), then the leave
// ---------------------------------------------------------------------------

/// `remove_player` cancels with the trade's real id, then sends the leave.
/// The cancel ends the trade, and the leave finds nothing open.
#[test]
fn the_partners_disconnect_cancel_ends_the_trade_once() {
    let mut state = trading();
    assert_eq!(
        ws(&mut state, cancelled(TRADE, PARTNER_LEFT)),
        vec![cancelled_event(TRADE, PARTNER_LEFT)]
    );
    assert_eq!(state.trade(), &Trade::Idle);
    assert!(ws(&mut state, M::PlayerLeft { player_id: BRIN }).is_empty());
}

/// Every client gets every disconnect cancel. A stranger's, from another
/// trade, isn't ours.
#[test]
fn a_strangers_disconnect_cancel_is_ignored() {
    let mut state = trading();
    let theirs = Uuid::from_u128(0xdead);
    assert!(ws(&mut state, cancelled(theirs, PARTNER_LEFT)).is_empty());
    // Fen, not Brin, was the one who left.
    assert!(ws(&mut state, M::PlayerLeft { player_id: FAR }).is_empty());
    // The next snapshot still has Brin.
    let still_here: Vec<_> = everyone().into_iter().filter(|p| p.id != FAR).collect();
    assert!(ws(
        &mut state,
        M::Positions {
            players: still_here
        }
    )
    .is_empty());
    assert_eq!(open(&state).trade_id, TRADE);
}

/// The cancel can be lost (a lagging client), or come from a server before
/// B6 with a dummy id. The leave still ends the trade.
#[test]
fn the_partners_leave_ends_the_trade() {
    let mut state = trading();
    ws(&mut state, cancelled(Uuid::from_u128(0xdead), PARTNER_LEFT));
    assert_eq!(
        ws(&mut state, M::PlayerLeft { player_id: BRIN }),
        vec![cancelled_event(TRADE, PARTNER_LEFT)]
    );
    assert_eq!(state.trade(), &Trade::Idle);
}

/// The leave can be lost too (a resync, a lagging client). Missing from
/// `positions` is the same signal.
#[test]
fn a_partner_missing_from_positions_ends_the_trade() {
    let mut state = trading();
    let without_brin: Vec<_> = everyone().into_iter().filter(|p| p.id != BRIN).collect();
    assert_eq!(
        ws(
            &mut state,
            M::Positions {
                players: without_brin
            }
        ),
        vec![cancelled_event(TRADE, PARTNER_LEFT)]
    );
}

/// B8 (#59): a stale handler can `remove_player` us while we're connected.
/// The server cancelled our trade then too.
#[test]
fn our_own_leave_ends_the_trade() {
    let mut state = trading();
    assert_eq!(
        ws(&mut state, M::PlayerLeft { player_id: ME }),
        vec![cancelled_event(TRADE, PARTNER_LEFT)]
    );
}

#[test]
fn losing_the_connection_ends_everything() {
    let reconnecting = NetEvent::Connection(ConnectionState::Reconnecting {
        attempt: 1,
        delay: Duration::from_millis(500),
        reason: "reset".into(),
    });

    let mut state = trading();
    state.trade_cancel(Instant::now()).unwrap();
    assert_eq!(
        trades(state.apply(&reconnecting)),
        vec![cancelled_event(TRADE, CONNECTION_LOST)]
    );
    assert_eq!(state.trade_waiting(), None);
    // The resync that follows has nothing left to end.
    assert!(trades(state.apply(&NetEvent::Resync)).is_empty());

    let mut state = world();
    state.trade_initiate(Instant::now(), BRIN).unwrap();
    assert_eq!(
        trades(state.apply(&reconnecting)),
        vec![TradeEvent::RequestGone { player_id: BRIN }]
    );

    // A resync on its own (no state change seen) ends it too.
    let mut state = world();
    ws(&mut state, requested_by(CATO, "Cato"));
    assert_eq!(
        trades(state.apply(&NetEvent::Resync)),
        vec![TradeEvent::RequestGone { player_id: CATO }]
    );

    // Connecting from idle reports nothing.
    let mut state = GameState::new();
    assert!(trades(state.apply(&NetEvent::Connection(ConnectionState::Connecting))).is_empty());
}

// ---------------------------------------------------------------------------
// Refusals (B5, #47): the server answers a refused command with `error`
// ---------------------------------------------------------------------------

fn error(message: &str) -> M {
    M::Error {
        message: message.into(),
    }
}

fn refused(action: TradeAction, reason: &str) -> TradeEvent {
    TradeEvent::Refused {
        action,
        reason: reason.into(),
    }
}

#[test]
fn a_refused_request_goes_idle_with_the_servers_reason() {
    let mut state = world();
    state.trade_initiate(Instant::now(), BRIN).unwrap();
    let events = state.apply_ws(&error("Target player is already in a trade"));
    assert!(events.contains(&StateEvent::ServerError {
        message: "Target player is already in a trade".into()
    }));
    assert_eq!(
        trades(events),
        vec![refused(TradeAction::Initiate, "Target player is already in a trade")]
    );
    assert_eq!(state.trade(), &Trade::Idle);
    assert_eq!(state.trade_waiting(), None);
}

#[test]
fn a_refused_accept_request_drops_the_request() {
    let mut state = world();
    state.apply_ws(&requested_by(BRIN, "Brin"));
    state.trade_accept_request(Instant::now()).unwrap();
    assert_eq!(
        ws(&mut state, error("Other player is now in a trade")),
        vec![refused(TradeAction::AcceptRequest, "Other player is now in a trade")]
    );
    assert_eq!(state.trade(), &Trade::Idle);
}

#[test]
fn a_refused_offer_leaves_the_trade_as_it_was() {
    let mut state = trading();
    state.apply_ws(&offer_updated(TRADE, ME, 5));
    state
        .trade_offer(
            Instant::now(),
            vec![TradeOfferItem {
                item_id: ITEM.to_string(),
                quantity: 9,
            }],
            0,
        )
        .unwrap();
    assert_eq!(
        ws(&mut state, error("You only have 1 of an item you offered 9 of")),
        vec![refused(TradeAction::Offer, "You only have 1 of an item you offered 9 of")]
    );
    assert_eq!(open(&state).mine.strands, 5);
    assert_eq!(state.trade_waiting(), None);
    // No longer waiting on the offer, so accepting is allowed again.
    assert!(state.trade_accept(Instant::now()).is_ok());
}

#[test]
fn a_refused_accept_leaves_the_trade_open() {
    let mut state = trading();
    state.trade_accept(Instant::now()).unwrap();
    assert_eq!(
        ws(&mut state, error("Not in a trade")),
        vec![refused(TradeAction::Accept, "Not in a trade")]
    );
    assert!(!open(&state).i_accepted);
}

#[test]
fn a_refused_cancel_closes_the_trade() {
    let mut state = trading();
    state.trade_cancel(Instant::now()).unwrap();
    assert_eq!(
        ws(&mut state, error("Not in a trade")),
        vec![
            cancelled_event(TRADE, "Not in a trade"),
            refused(TradeAction::Cancel, "Not in a trade"),
        ]
    );
    assert_eq!(state.trade(), &Trade::Idle);
}

#[test]
fn an_error_with_nothing_waiting_is_not_a_trade_event() {
    let mut state = trading();
    let events = state.apply_ws(&error("Slot is occupied"));
    assert_eq!(
        events,
        vec![StateEvent::ServerError {
            message: "Slot is occupied".into()
        }]
    );
    assert!(state.open_trade().is_some());
}

#[test]
fn a_refusal_stops_the_timeout() {
    let mut state = world();
    let now = Instant::now();
    state.trade_initiate(now, BRIN).unwrap();
    state.apply_ws(&error("That player is offline"));
    assert!(trades(state.tick(now + REPLY_TIMEOUT + Duration::from_secs(1))).is_empty());
}
