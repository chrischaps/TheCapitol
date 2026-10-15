//! The trade state machine (plan §4.5). It lives in the store because deciding
//! which trade messages are ours is a filtering job, like everything else
//! there, and because a trade ends when its partner leaves.
//!
//! ```text
//! Idle ──initiate──▶ Requested(to) ──trade_started──▶ Open(trade_id) ──trade_executed──▶ Idle
//!   ▲ ╲                    │                            │  offers, accepts
//!   │  trade_requested     declined / they left         ╰─ cancelled / partner left ─▶ Idle
//!   │    ╲                 ▼
//!   │     Incoming(from) ──accept_request──▶ (trade_started) ──▶ Open
//!   ╰──── decline / they left
//! ```
//!
//! The actions (`trade_initiate` and so on) check what they can locally and
//! return the [`ClientMessage`] for the game to send. The server's replies come
//! back through [`GameState::apply`] as [`StateEvent::Trade`].
//!
//! **Failures.** Since B5 (#47) the server answers a trade command it refuses
//! with `error{message}`, sent only to us. The `error` carries no command, so
//! one that arrives while an action is waiting is taken as that action's
//! refusal: [`TradeEvent::Refused`], with the wait cleared. The store still
//! reports the `error` as [`StateEvent::ServerError`] too, so the UI should
//! toast that and use `Refused` only to update the trade window. Each action
//! still waits [`REPLY_TIMEOUT`] as a backstop, and [`GameState::tick`]
//! reports [`TradeEvent::NoReply`] when nothing comes. A request is a special
//! case: the server never confirms a delivered one, so `NoReply(Initiate)`
//! means it most likely arrived, and the request stays [`Trade::Requested`]
//! until it's answered.
//!
//! **Whose message is it.** `trade_requested`, `trade_request_declined` and
//! `trade_started` reach only the players involved. `trade_offer_updated` and
//! `trade_accepted` are filtered by the server too, but carry a `trade_id`
//! anyway. `trade_executed` and `trade_cancelled` go to *everyone*
//! (`ws/connection.rs`), so only a matching `trade_id` counts.
//!
//! **Disconnects.** When a player drops mid-trade, `remove_player`
//! (`engine/tick.rs`) sends `trade_cancelled` with a fresh dummy `trade_id`,
//! then that player's `player_left`. The dummy can't be told from a
//! stranger's cancel, so it's ignored, and the leave ends the trade instead.
//! A leave always means the server has cancelled: `remove_player` cancels
//! first. A partner missing from `positions` counts as a leave. B6 (#48) will
//! send the real `trade_id`; the cancel then ends the trade itself and the
//! leave finds it already closed.

use std::fmt;
use std::time::{Duration, Instant};

use capitol_protocol::rules;
use capitol_protocol::ws::{ClientMessage, ServerMessage, TradeItemInfo, TradeOfferItem};
use uuid::Uuid;

use super::{GameState, StateEvent};

/// How long a trade command waits for the server's answer.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// The `reason` of the cancel `remove_player` sends, reused for a partner who
/// left.
pub const PARTNER_LEFT: &str = "Player disconnected";
/// The `reason` when the client's own connection dropped.
pub const CONNECTION_LOST: &str = "Connection lost";
/// The `reason` when a cancel got no answer. The server only fails a cancel
/// when it has no trade for us, so the trade is over either way.
pub const CANCEL_UNCONFIRMED: &str = "The server has no open trade";

/// Where the local player's trading stands.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Trade {
    #[default]
    Idle,
    /// We asked `to`. The server keeps a request until it's answered or either
    /// player leaves; it has no expiry and no way to withdraw one.
    Requested {
        to: Uuid,
        name: String,
    },
    /// `from` asked us. A newer request replaces it, as on the server, which
    /// holds one request per target.
    Incoming {
        from: Uuid,
        name: String,
    },
    Open(OpenTrade),
}

/// A trade window's contents.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenTrade {
    pub trade_id: Uuid,
    pub partner_id: Uuid,
    pub partner_name: String,
    /// What the server last confirmed for each side.
    pub mine: Offer,
    pub theirs: Offer,
    pub i_accepted: bool,
    pub they_accepted: bool,
}

/// One side of a trade. Items carry type, name and quality from the server.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Offer {
    pub items: Vec<TradeItemInfo>,
    pub strands: i64,
}

/// A trade command the client sent and is waiting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeAction {
    Initiate,
    AcceptRequest,
    Offer,
    Accept,
    Cancel,
}

impl TradeAction {
    /// A default line for the UI when this action gets no reply.
    pub fn hint(self) -> &'static str {
        match self {
            TradeAction::Initiate => "No answer yet. The request stays open until they answer.",
            TradeAction::AcceptRequest => {
                "The trade didn't start. They may have left, or started another trade."
            }
            TradeAction::Offer => "The server didn't confirm your offer. Try again.",
            TradeAction::Accept => "The server didn't confirm your accept. Try again.",
            TradeAction::Cancel => CANCEL_UNCONFIRMED,
        }
    }
}

/// A trade change for the local player.
#[derive(Debug, Clone, PartialEq)]
pub enum TradeEvent {
    /// `from` asked to trade: show accept and decline.
    Requested {
        from: Uuid,
        name: String,
    },
    /// `by` declined our request.
    Declined {
        by: Uuid,
    },
    /// A request to or from `player_id` is gone, because one of us left.
    RequestGone {
        player_id: Uuid,
    },
    Started {
        trade_id: Uuid,
        partner_id: Uuid,
    },
    /// `player_id`'s offer changed. The server reset both accepts.
    OfferChanged {
        player_id: Uuid,
    },
    Accepted {
        player_id: Uuid,
    },
    /// Our trade went through. Containers and currency are refetched (P2.4),
    /// because no `inventory_updated` follows.
    Executed {
        trade_id: Uuid,
    },
    /// Our trade ended without a swap. Nothing is held in escrow, so nothing
    /// needs refetching. `reason` is the server's, or one of this module's
    /// constants.
    Cancelled {
        trade_id: Uuid,
        reason: String,
    },
    /// The server didn't answer in time. See [`TradeAction::hint`].
    NoReply(TradeAction),
    /// The server refused `action` with an `error` while it was waiting.
    /// `reason` is the server's message. The machine has already moved on: a
    /// refused request or accept-request is gone, a refused cancel has closed
    /// the trade, and a refused offer or accept leaves the trade as it was.
    Refused {
        action: TradeAction,
        reason: String,
    },
}

/// Why an action wasn't sent. Each is a failure the server would have
/// swallowed.
#[derive(Debug, Clone, PartialEq)]
pub enum TradeRefusal {
    NotConnected,
    Yourself,
    /// The target isn't in the last `positions`.
    Offline,
    /// Further than [`rules::TRADE_RANGE`] at the last `positions`. Walk closer.
    OutOfRange {
        distance: f64,
    },
    AlreadyTrading,
    /// Our request to them is still open; the server would ignore another.
    AlreadyAsked,
    NoRequest,
    NotTrading,
    /// Our last offer isn't confirmed. Its arrival would reset the accept.
    OfferUnconfirmed,
}

impl fmt::Display for TradeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TradeRefusal::NotConnected => f.write_str("Not connected"),
            TradeRefusal::Yourself => f.write_str("You can't trade with yourself"),
            TradeRefusal::Offline => f.write_str("That player isn't online"),
            TradeRefusal::OutOfRange { .. } => f.write_str("Too far away to trade"),
            TradeRefusal::AlreadyTrading => f.write_str("Already trading"),
            TradeRefusal::AlreadyAsked => f.write_str("Still waiting for their answer"),
            TradeRefusal::NoRequest => f.write_str("No trade request to answer"),
            TradeRefusal::NotTrading => f.write_str("No open trade"),
            TradeRefusal::OfferUnconfirmed => f.write_str("Wait for your offer to update"),
        }
    }
}

impl std::error::Error for TradeRefusal {}

/// The machine's bookkeeping, a field of [`GameState`].
#[derive(Debug, Clone, Default)]
pub(super) struct TradeBook {
    trade: Trade,
    /// The command we're waiting on, and when to give up.
    waiting: Option<(TradeAction, Instant)>,
    /// The last trade that ended here. A cancel that timed out may still
    /// execute, and its `trade_executed` must still refetch.
    ended: Option<Uuid>,
}

impl GameState {
    /// Where trading stands.
    pub fn trade(&self) -> &Trade {
        &self.trade.trade
    }

    /// The open trade, if any.
    pub fn open_trade(&self) -> Option<&OpenTrade> {
        match &self.trade.trade {
            Trade::Open(open) => Some(open),
            _ => None,
        }
    }

    /// The trade command still waiting on the server, for a spinner.
    pub fn trade_waiting(&self) -> Option<TradeAction> {
        self.trade.waiting.map(|(action, _)| action)
    }

    /// Runs the trade timeouts. Call it once per frame.
    pub fn tick(&mut self, now: Instant) -> Vec<StateEvent> {
        let mut out = Vec::new();
        let Some((action, due)) = self.trade.waiting else {
            return out;
        };
        if now < due {
            return out;
        }
        self.trade.waiting = None;
        match action {
            // Nothing changes: a request may still be answered, and an
            // unconfirmed offer or accept leaves the trade as the server last said.
            TradeAction::Initiate | TradeAction::Offer | TradeAction::Accept => {}
            TradeAction::AcceptRequest => {
                if let Trade::Incoming { from, .. } = self.trade.trade {
                    self.trade.trade = Trade::Idle;
                    out.push(StateEvent::Trade(TradeEvent::RequestGone {
                        player_id: from,
                    }));
                }
            }
            TradeAction::Cancel => {
                self.end_trade(CANCEL_UNCONFIRMED, &mut out);
                return out;
            }
        }
        out.push(StateEvent::Trade(TradeEvent::NoReply(action)));
        out
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    /// Asks `target` to trade. Asking someone who has asked us starts the trade
    /// at once, as the server does (`TradeManager::request_trade`).
    pub fn trade_initiate(
        &mut self,
        now: Instant,
        target: Uuid,
    ) -> Result<ClientMessage, TradeRefusal> {
        let me = self.local_player().ok_or(TradeRefusal::NotConnected)?;
        if !self.is_connected() {
            return Err(TradeRefusal::NotConnected);
        }
        if me.id == target {
            return Err(TradeRefusal::Yourself);
        }
        match &self.trade.trade {
            Trade::Open(_) => return Err(TradeRefusal::AlreadyTrading),
            Trade::Requested { to, .. } if *to == target => return Err(TradeRefusal::AlreadyAsked),
            _ => {}
        }
        let them = self.players.get(&target).ok_or(TradeRefusal::Offline)?;
        let distance = ((them.x - me.x).powi(2) + (them.y - me.y).powi(2)).sqrt();
        if distance > rules::TRADE_RANGE {
            return Err(TradeRefusal::OutOfRange { distance });
        }
        self.trade.trade = Trade::Requested {
            to: target,
            name: them.name.clone(),
        };
        self.trade.waiting = Some((TradeAction::Initiate, now + REPLY_TIMEOUT));
        Ok(ClientMessage::TradeInitiate {
            target_player_id: target.to_string(),
        })
    }

    /// Forgets our request locally. The server has no withdraw, so if they
    /// accept later the trade still opens; cancel it then.
    pub fn trade_withdraw(&mut self) -> Result<(), TradeRefusal> {
        match self.trade.trade {
            Trade::Requested { .. } => {
                self.trade.trade = Trade::Idle;
                self.trade.waiting = None;
                Ok(())
            }
            _ => Err(TradeRefusal::NoRequest),
        }
    }

    /// Accepts the incoming request. The trade opens on `trade_started`.
    pub fn trade_accept_request(&mut self, now: Instant) -> Result<ClientMessage, TradeRefusal> {
        self.connected_or_refuse()?;
        match self.trade.trade {
            Trade::Incoming { .. } => {
                self.trade.waiting = Some((TradeAction::AcceptRequest, now + REPLY_TIMEOUT));
                Ok(ClientMessage::TradeAcceptRequest)
            }
            _ => Err(TradeRefusal::NoRequest),
        }
    }

    /// Declines the incoming request. The server confirms nothing to us.
    pub fn trade_decline_request(&mut self) -> Result<ClientMessage, TradeRefusal> {
        self.connected_or_refuse()?;
        match self.trade.trade {
            Trade::Incoming { .. } => {
                self.trade.trade = Trade::Idle;
                self.trade.waiting = None;
                Ok(ClientMessage::TradeDeclineRequest)
            }
            _ => Err(TradeRefusal::NoRequest),
        }
    }

    /// Replaces our whole offer. The open trade changes when the server echoes
    /// it back, which also resets both accepts.
    pub fn trade_offer(
        &mut self,
        now: Instant,
        items: Vec<TradeOfferItem>,
        strands: i64,
    ) -> Result<ClientMessage, TradeRefusal> {
        self.open_or_refuse()?;
        self.trade.waiting = Some((TradeAction::Offer, now + REPLY_TIMEOUT));
        Ok(ClientMessage::TradeOffer { items, strands })
    }

    /// Accepts the trade as it stands. When both sides have, the server
    /// executes it.
    pub fn trade_accept(&mut self, now: Instant) -> Result<ClientMessage, TradeRefusal> {
        self.open_or_refuse()?;
        // The server queues an offer for the next tick but handles an accept at
        // once, so the offer would land after the accept and reset it.
        if matches!(self.trade.waiting, Some((TradeAction::Offer, _))) {
            return Err(TradeRefusal::OfferUnconfirmed);
        }
        self.trade.waiting = Some((TradeAction::Accept, now + REPLY_TIMEOUT));
        Ok(ClientMessage::TradeAccept)
    }

    /// Cancels the open trade. It closes on the server's `trade_cancelled`, or
    /// after [`REPLY_TIMEOUT`] without one.
    pub fn trade_cancel(&mut self, now: Instant) -> Result<ClientMessage, TradeRefusal> {
        self.open_or_refuse()?;
        self.trade.waiting = Some((TradeAction::Cancel, now + REPLY_TIMEOUT));
        Ok(ClientMessage::TradeCancel)
    }

    fn connected_or_refuse(&self) -> Result<(), TradeRefusal> {
        if self.is_connected() {
            Ok(())
        } else {
            Err(TradeRefusal::NotConnected)
        }
    }

    fn open_or_refuse(&self) -> Result<(), TradeRefusal> {
        self.connected_or_refuse()?;
        match self.trade.trade {
            Trade::Open(_) => Ok(()),
            _ => Err(TradeRefusal::NotTrading),
        }
    }

    // -----------------------------------------------------------------------
    // Applying
    // -----------------------------------------------------------------------

    pub(super) fn apply_trade(&mut self, message: &ServerMessage, out: &mut Vec<StateEvent>) {
        use ServerMessage as M;
        let book = &mut self.trade;
        match message {
            M::TradeRequested {
                from_player,
                from_player_name,
            } => match &book.trade {
                // We asked each other at once. Our request starts the trade on
                // the server, so `trade_started` is on its way: no prompt.
                Trade::Requested { to, .. } if to == from_player => {}
                // The server refuses requests to a player who's trading.
                Trade::Open(_) => {}
                _ => {
                    book.trade = Trade::Incoming {
                        from: *from_player,
                        name: from_player_name.clone(),
                    };
                    book.waiting = None;
                    out.push(StateEvent::Trade(TradeEvent::Requested {
                        from: *from_player,
                        name: from_player_name.clone(),
                    }));
                }
            },
            M::TradeRequestDeclined { by_player } => {
                if matches!(&book.trade, Trade::Requested { to, .. } if to == by_player) {
                    book.trade = Trade::Idle;
                    book.waiting = None;
                }
                // Sent only to the asker, so it's ours even after a withdraw.
                out.push(StateEvent::Trade(TradeEvent::Declined { by: *by_player }));
            }
            // Sent only to the two players, so it's ours whatever we thought
            // was happening: a request we'd withdrawn can still be accepted.
            M::TradeStarted {
                trade_id,
                partner_id,
                partner_name,
            } => {
                book.trade = Trade::Open(OpenTrade {
                    trade_id: *trade_id,
                    partner_id: *partner_id,
                    partner_name: partner_name.clone(),
                    mine: Offer::default(),
                    theirs: Offer::default(),
                    i_accepted: false,
                    they_accepted: false,
                });
                book.waiting = None;
                out.push(StateEvent::Trade(TradeEvent::Started {
                    trade_id: *trade_id,
                    partner_id: *partner_id,
                }));
            }
            M::TradeOfferUpdated {
                trade_id,
                player_id,
                items,
                strands,
            } => {
                let Trade::Open(open) = &mut book.trade else {
                    return;
                };
                if open.trade_id != *trade_id {
                    return;
                }
                let offer = Offer {
                    items: items.clone(),
                    strands: *strands,
                };
                let mine = self.me == Some(*player_id);
                if mine {
                    open.mine = offer;
                    if matches!(book.waiting, Some((TradeAction::Offer, _))) {
                        book.waiting = None;
                    }
                } else {
                    open.theirs = offer;
                }
                // `TradeManager::update_offer` resets both.
                open.i_accepted = false;
                open.they_accepted = false;
                out.push(StateEvent::Trade(TradeEvent::OfferChanged {
                    player_id: *player_id,
                }));
            }
            M::TradeAccepted {
                trade_id,
                player_id,
            } => {
                let Trade::Open(open) = &mut book.trade else {
                    return;
                };
                if open.trade_id != *trade_id {
                    return;
                }
                if self.me == Some(*player_id) {
                    open.i_accepted = true;
                    if matches!(book.waiting, Some((TradeAction::Accept, _))) {
                        book.waiting = None;
                    }
                } else {
                    open.they_accepted = true;
                }
                out.push(StateEvent::Trade(TradeEvent::Accepted {
                    player_id: *player_id,
                }));
            }
            // Broadcast to everyone: only our trade_id counts. The final
            // `trade_accepted` can be dropped: the trade may have left
            // `active_trades` by the time the server filters it, so this may
            // follow a single accept.
            M::TradeExecuted { trade_id } => {
                let ours = matches!(&book.trade, Trade::Open(open) if open.trade_id == *trade_id);
                if ours {
                    book.trade = Trade::Idle;
                    book.waiting = None;
                    book.ended = Some(*trade_id);
                }
                if ours || book.ended == Some(*trade_id) {
                    out.push(StateEvent::Trade(TradeEvent::Executed {
                        trade_id: *trade_id,
                    }));
                }
            }
            // Broadcast to everyone, and a disconnect's has a dummy id. Only a
            // match is ours; a partner's disconnect ends the trade on their leave.
            M::TradeCancelled { trade_id, reason } => {
                if matches!(&book.trade, Trade::Open(open) if open.trade_id == *trade_id) {
                    self.end_trade(reason, out);
                }
            }
            _ => {}
        }
    }

    /// The server sent `error`. If a trade action is waiting, it's that
    /// action's refusal (B5, #47); otherwise it isn't ours.
    pub(super) fn trade_error(&mut self, message: &str, out: &mut Vec<StateEvent>) {
        let Some((action, _)) = self.trade.waiting.take() else {
            return;
        };
        match action {
            // The server keeps no request it refused.
            TradeAction::Initiate => {
                if matches!(self.trade.trade, Trade::Requested { .. }) {
                    self.trade.trade = Trade::Idle;
                }
            }
            // Gone already, or the asker started another trade.
            TradeAction::AcceptRequest => {
                if matches!(self.trade.trade, Trade::Incoming { .. }) {
                    self.trade.trade = Trade::Idle;
                }
            }
            TradeAction::Offer | TradeAction::Accept => {}
            // The server only refuses a cancel when it has no trade for us.
            TradeAction::Cancel => self.end_trade(message, out),
        }
        out.push(StateEvent::Trade(TradeEvent::Refused {
            action,
            reason: message.to_string(),
        }));
    }

    /// A player left (`player_left`, or missing from `positions`).
    pub(super) fn trade_player_left(&mut self, player_id: Uuid, out: &mut Vec<StateEvent>) {
        match &self.trade.trade {
            Trade::Open(open) if open.partner_id == player_id || self.me == Some(player_id) => {
                self.end_trade(PARTNER_LEFT, out);
            }
            Trade::Requested { to: other, .. } | Trade::Incoming { from: other, .. }
                if *other == player_id =>
            {
                self.trade.trade = Trade::Idle;
                self.trade.waiting = None;
                out.push(StateEvent::Trade(TradeEvent::RequestGone { player_id }));
            }
            _ => {}
        }
    }

    /// The connection dropped. The server removed us, which cancelled any trade
    /// and dropped every request to or from us.
    pub(super) fn trade_connection_lost(&mut self, out: &mut Vec<StateEvent>) {
        match &self.trade.trade {
            Trade::Idle => {}
            Trade::Open(_) => self.end_trade(CONNECTION_LOST, out),
            Trade::Requested { to: other, .. } | Trade::Incoming { from: other, .. } => {
                let player_id = *other;
                self.trade.trade = Trade::Idle;
                self.trade.waiting = None;
                out.push(StateEvent::Trade(TradeEvent::RequestGone { player_id }));
            }
        }
    }

    /// Closes the open trade without a swap.
    fn end_trade(&mut self, reason: &str, out: &mut Vec<StateEvent>) {
        let open = match std::mem::take(&mut self.trade.trade) {
            Trade::Open(open) => open,
            other => {
                self.trade.trade = other;
                return;
            }
        };
        self.trade.waiting = None;
        self.trade.ended = Some(open.trade_id);
        out.push(StateEvent::Trade(TradeEvent::Cancelled {
            trade_id: open.trade_id,
            reason: reason.to_string(),
        }));
    }
}

#[cfg(test)]
pub(super) mod tests;
