use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

use capitol_protocol::rules::TRADE_REQUEST_TTL_SECS;

/// How long a trade request waits for an answer before the tick drops it
pub const TRADE_REQUEST_TTL: Duration = Duration::from_secs(TRADE_REQUEST_TTL_SECS);

/// Trade session between two players
#[derive(Debug, Clone)]
pub struct TradeSession {
    pub id: Uuid,
    pub player_a: Uuid,
    pub player_b: Uuid,
    pub offer_a: TradeOffer,
    pub offer_b: TradeOffer,
    pub accepted_a: bool,
    pub accepted_b: bool,
}

/// A player's offer in a trade
#[derive(Debug, Clone, Default)]
pub struct TradeOffer {
    /// Items being offered: (item_id, quantity)
    pub items: Vec<(Uuid, i32)>,
    /// Strands being offered
    pub strands: i64,
}

impl TradeOffer {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            strands: 0,
        }
    }
}

impl TradeSession {
    pub fn new(player_a: Uuid, player_b: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            player_a,
            player_b,
            offer_a: TradeOffer::new(),
            offer_b: TradeOffer::new(),
            accepted_a: false,
            accepted_b: false,
        }
    }

    /// Get mutable reference to a player's offer
    pub fn get_offer_mut(&mut self, player_id: Uuid) -> Option<&mut TradeOffer> {
        if player_id == self.player_a {
            Some(&mut self.offer_a)
        } else if player_id == self.player_b {
            Some(&mut self.offer_b)
        } else {
            None
        }
    }

    /// Set a player's accepted status
    pub fn set_accepted(&mut self, player_id: Uuid, accepted: bool) {
        if player_id == self.player_a {
            self.accepted_a = accepted;
        } else if player_id == self.player_b {
            self.accepted_b = accepted;
        }
    }

    /// Check if both players have accepted
    pub fn both_accepted(&self) -> bool {
        self.accepted_a && self.accepted_b
    }

    /// Reset accepted status for both players (called when offer changes)
    pub fn reset_accepted(&mut self) {
        self.accepted_a = false;
        self.accepted_b = false;
    }

    /// Get the other player in the trade
    pub fn other_player(&self, player_id: Uuid) -> Option<Uuid> {
        if player_id == self.player_a {
            Some(self.player_b)
        } else if player_id == self.player_b {
            Some(self.player_a)
        } else {
            None
        }
    }
}

/// Manage active trade sessions
pub struct TradeManager {
    /// Active trade sessions by trade ID
    pub active_trades: HashMap<Uuid, TradeSession>,
    /// Map player ID to their active trade ID
    pub player_trades: HashMap<Uuid, Uuid>,
    /// Pending trade requests: target_player -> (initiator, request_time)
    pub pending_requests: HashMap<Uuid, (Uuid, Instant)>,
}

impl TradeManager {
    pub fn new() -> Self {
        Self {
            active_trades: HashMap::new(),
            player_trades: HashMap::new(),
            pending_requests: HashMap::new(),
        }
    }

    /// Request to start a trade with another player. A request replaces any other
    /// pending request to `target`; the replaced request's asker is returned so they
    /// can be told (#182)
    pub fn request_trade(&mut self, initiator: Uuid, target: Uuid) -> Result<Option<Uuid>, String> {
        // Check neither player is already in a trade
        if self.player_trades.contains_key(&initiator) {
            return Err("You are already in a trade".to_string());
        }
        if self.player_trades.contains_key(&target) {
            return Err("Target player is already in a trade".to_string());
        }

        // Check for existing pending request
        if let Some((existing_initiator, _)) = self.pending_requests.get(&target) {
            if *existing_initiator == initiator {
                return Err("Trade request already pending".to_string());
            }
        }

        // Check if target has already requested to trade with us (auto-accept)
        if let Some((pending_initiator, asked_at)) = self.pending_requests.remove(&initiator) {
            if pending_initiator == target {
                // Both players want to trade - start immediately
                let session = TradeSession::new(target, initiator);
                let trade_id = session.id;
                self.active_trades.insert(trade_id, session);
                self.player_trades.insert(initiator, trade_id);
                self.player_trades.insert(target, trade_id);
                return Ok(None);
            }
            // Put it back, keeping its age so it still expires on time
            self.pending_requests.insert(initiator, (pending_initiator, asked_at));
        }

        // Add pending request, replacing any other request to `target`
        let replaced = self.pending_requests.insert(target, (initiator, Instant::now()));
        Ok(replaced.map(|(asker, _)| asker))
    }

    /// Accept a pending trade request
    pub fn accept_request(&mut self, player_id: Uuid) -> Result<TradeSession, String> {
        let (initiator, _) = self.pending_requests.remove(&player_id)
            .ok_or("No pending trade request")?;

        // Double-check neither player is in a trade
        if self.player_trades.contains_key(&player_id) {
            return Err("You are already in a trade".to_string());
        }
        if self.player_trades.contains_key(&initiator) {
            return Err("Other player is now in a trade".to_string());
        }

        // Create trade session
        let session = TradeSession::new(initiator, player_id);
        let trade_id = session.id;
        self.active_trades.insert(trade_id, session.clone());
        self.player_trades.insert(player_id, trade_id);
        self.player_trades.insert(initiator, trade_id);

        Ok(session)
    }

    /// Decline a pending trade request
    pub fn decline_request(&mut self, player_id: Uuid) -> Result<Uuid, String> {
        let (initiator, _) = self.pending_requests.remove(&player_id)
            .ok_or("No pending trade request")?;
        Ok(initiator)
    }

    /// Get a player's active trade
    pub fn get_player_trade(&self, player_id: Uuid) -> Option<&TradeSession> {
        self.player_trades.get(&player_id)
            .and_then(|trade_id| self.active_trades.get(trade_id))
    }

    /// Get a player's active trade mutably
    pub fn get_player_trade_mut(&mut self, player_id: Uuid) -> Option<&mut TradeSession> {
        self.player_trades.get(&player_id).copied()
            .and_then(move |trade_id| self.active_trades.get_mut(&trade_id))
    }

    /// Update a player's offer
    pub fn update_offer(&mut self, player_id: Uuid, items: Vec<(Uuid, i32)>, strands: i64) -> Result<(), String> {
        let trade = self.get_player_trade_mut(player_id)
            .ok_or("Not in a trade")?;

        // Update offer
        if let Some(offer) = trade.get_offer_mut(player_id) {
            offer.items = items;
            offer.strands = strands;
        }

        // Reset accepted status since offer changed
        trade.reset_accepted();

        Ok(())
    }

    /// Set a player's accepted status
    pub fn set_accepted(&mut self, player_id: Uuid, accepted: bool) -> Result<bool, String> {
        let trade = self.get_player_trade_mut(player_id)
            .ok_or("Not in a trade")?;

        trade.set_accepted(player_id, accepted);

        Ok(trade.both_accepted())
    }

    /// Cancel a trade
    pub fn cancel_trade(&mut self, player_id: Uuid) -> Result<(Uuid, Uuid), String> {
        let trade_id = self.player_trades.remove(&player_id)
            .ok_or("Not in a trade")?;

        let session = self.active_trades.remove(&trade_id)
            .ok_or("Trade session not found")?;

        // Remove other player from trade map
        self.player_trades.remove(&session.player_a);
        self.player_trades.remove(&session.player_b);

        Ok((session.player_a, session.player_b))
    }

    /// Complete a trade (both players accepted)
    /// Returns the session for execution
    pub fn complete_trade(&mut self, player_id: Uuid) -> Result<TradeSession, String> {
        let trade_id = *self.player_trades.get(&player_id)
            .ok_or("Not in a trade")?;

        let session = self.active_trades.get(&trade_id)
            .ok_or("Trade session not found")?;

        if !session.both_accepted() {
            return Err("Both players must accept".to_string());
        }

        // Remove from maps
        self.player_trades.remove(&session.player_a);
        self.player_trades.remove(&session.player_b);

        let session = self.active_trades.remove(&trade_id)
            .ok_or("Trade session not found")?;

        Ok(session)
    }

    /// Check if a player is in an active trade
    #[cfg(test)]
    pub fn is_in_trade(&self, player_id: Uuid) -> bool {
        self.player_trades.contains_key(&player_id)
    }

    /// Drop the requests nobody answered within [`TRADE_REQUEST_TTL`] of `now`, and return
    /// each as `(initiator, target)` so both can be told
    pub fn expire_requests(&mut self, now: Instant) -> Vec<(Uuid, Uuid)> {
        let mut expired = Vec::new();
        self.pending_requests.retain(|target, (initiator, asked_at)| {
            if now.saturating_duration_since(*asked_at) >= TRADE_REQUEST_TTL {
                expired.push((*initiator, *target));
                false
            } else {
                true
            }
        });
        expired
    }
}

impl Default for TradeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Trade proximity range (same as station interaction range)
pub use capitol_protocol::rules::TRADE_RANGE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_expires_after_its_ttl_and_not_before() {
        let mut trades = TradeManager::new();
        let (ada, bram) = (Uuid::new_v4(), Uuid::new_v4());
        trades.request_trade(ada, bram).unwrap();
        let asked_at = trades.pending_requests[&bram].1;

        let just_before = asked_at + TRADE_REQUEST_TTL - Duration::from_millis(1);
        assert!(trades.expire_requests(just_before).is_empty());
        assert!(trades.pending_requests.contains_key(&bram));

        assert_eq!(trades.expire_requests(asked_at + TRADE_REQUEST_TTL), vec![(ada, bram)]);
        assert!(trades.pending_requests.is_empty());
        assert!(trades.accept_request(bram).is_err(), "an expired request can't be accepted");
    }

    #[test]
    fn expiring_leaves_trades_and_fresh_requests_alone() {
        let mut trades = TradeManager::new();
        let (ada, bram, cato, dara) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        trades.request_trade(ada, bram).unwrap();
        trades.accept_request(bram).unwrap();
        trades.request_trade(cato, dara).unwrap();
        let fresh = trades.pending_requests[&dara].1;
        trades.pending_requests.get_mut(&dara).unwrap().1 = fresh - TRADE_REQUEST_TTL;
        let (eli, fay) = (Uuid::new_v4(), Uuid::new_v4());
        trades.request_trade(eli, fay).unwrap();

        assert_eq!(trades.expire_requests(fresh), vec![(cato, dara)]);
        assert!(trades.is_in_trade(ada) && trades.is_in_trade(bram));
        assert_eq!(trades.pending_requests[&fay].0, eli);
    }

    #[test]
    fn asking_a_third_player_keeps_the_age_of_the_request_you_hold() {
        let mut trades = TradeManager::new();
        let (ada, bram, cato) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        trades.request_trade(bram, ada).unwrap();
        let asked_at = trades.pending_requests[&ada].1;

        // Ada, asked by Bram, asks Cato instead: Bram's request stays, as old as it was
        trades.request_trade(ada, cato).unwrap();
        assert_eq!(trades.pending_requests[&ada], (bram, asked_at));
    }

    #[test]
    fn a_newer_request_names_the_asker_it_replaced() {
        let mut trades = TradeManager::new();
        let (ada, brin, cato) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(trades.request_trade(brin, ada), Ok(None));
        assert_eq!(trades.request_trade(cato, ada), Ok(Some(brin)));
        assert_eq!(trades.pending_requests[&ada].0, cato);

        // Brin can ask again, and that replaces Cato's
        assert_eq!(trades.request_trade(brin, ada), Ok(Some(cato)));
        // Asking back starts the trade and replaces nobody
        assert_eq!(trades.request_trade(ada, brin), Ok(None));
        assert!(trades.is_in_trade(ada) && trades.is_in_trade(brin));
    }
}
