//! Trading as a player does it (P7.3), on top of the store's state machine
//! (`state::trade`, P2.6): asking someone, and building your side of the
//! window.
//!
//! - **[`Approach`]** is a shift-click on a player. Within
//!   [`rules::TRADE_RANGE`] it sends `trade_initiate` at once. Further away
//!   it walks you toward them, and asks as you come into range. React walks
//!   you onto them and you shift-click again. If they wander off while you
//!   walk, the walk is re-aimed at where they are now.
//! - **[`Draft`]** is your offer as you're building it: the stacks you've
//!   dragged in, how many of each, and the Strands. It sends itself
//!   ([`Draft::due`]) once you've stopped changing it for [`SETTLE`], so
//!   there's no Update button, and what your partner sees is what you see.
//!   Every offer the server takes resets both accepts, as it should: a
//!   changed offer is a new deal. Accept waits until the server has
//!   confirmed the draft ([`Draft::confirmed`]).

use std::time::{Duration, Instant};

use capitol_protocol::rest::SlotItem;
use capitol_protocol::rules;
use capitol_protocol::ws::{ClientMessage, TradeOfferItem};
use uuid::Uuid;

use crate::state::{Container, GameState, Offer, OpenTrade, StateEvent, TradeRefusal};

/// Where a walk toward a partner stops: this fraction of the trade range
/// from where they stood, on your side. Inside range with room for them to
/// drift.
pub const APPROACH: f64 = 0.5;

/// How far a partner may move from where the walk was aimed before it's
/// re-aimed, in server units.
pub const DRIFT: f64 = rules::TRADE_RANGE * 0.5;

/// How long a draft must sit unchanged before it's sent. Long enough that
/// typing "250" doesn't send 2, then 25.
pub const SETTLE: Duration = Duration::from_millis(450);

/// Where to walk to trade with someone at `them`, from `at`.
pub fn approach(at: [f64; 2], them: [f64; 2]) -> [f64; 2] {
    let (dx, dy) = (at[0] - them[0], at[1] - them[1]);
    let d = dx.hypot(dy);
    if d < 1e-6 {
        return them;
    }
    let r = rules::TRADE_RANGE * APPROACH;
    [them[0] + dx / d * r, them[1] + dy / d * r]
}

/// What the game should do for a click or an arrival.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Send this (`trade_initiate`).
    Send(ClientMessage),
    /// Walk to this server point. The player stays the target.
    WalkTo { x: f64, y: f64 },
    /// The click is taken and there's nothing to send.
    Hold,
}

/// Something worth telling the player.
#[derive(Debug, Clone, PartialEq)]
pub enum Note {
    /// The request wasn't sent, and why.
    Refused(TradeRefusal),
    /// The player you were walking to left.
    Lost { name: String },
}

/// A shift-click on a player: ask, or walk closer and ask.
#[derive(Debug, Default)]
pub struct Approach {
    /// Who you're walking to, their name, and where the walk was aimed at
    /// them.
    target: Option<(Uuid, String, [f64; 2])>,
    notes: Vec<Note>,
}

impl Approach {
    /// The player you're walking toward, to ask on arrival.
    pub fn target(&self) -> Option<Uuid> {
        self.target.as_ref().map(|(id, _, _)| *id)
    }

    /// You shift-clicked `player`. `None` if they're you or unknown, so the
    /// click falls through.
    pub fn click(&mut self, player: Uuid, state: &mut GameState, now: Instant) -> Option<Step> {
        let me = state.local_player()?;
        if me.id == player {
            return None;
        }
        let them = state.player(player)?.clone();
        self.target = None;
        match state.trade_initiate(now, player) {
            Ok(message) => Some(Step::Send(message)),
            Err(TradeRefusal::OutOfRange { .. }) => {
                let me = state.local_player()?;
                let [x, y] = approach([me.x, me.y], [them.x, them.y]);
                self.target = Some((player, them.name, [them.x, them.y]));
                Some(Step::WalkTo { x, y })
            }
            Err(why) => {
                self.notes.push(Note::Refused(why));
                Some(Step::Hold)
            }
        }
    }

    /// You walked somewhere else: the target is dropped.
    pub fn walked(&mut self) {
        self.target = None;
    }

    /// After every batch of state changes. Asks once you're in range, and
    /// re-aims the walk if they've moved.
    pub fn observe(
        &mut self,
        changes: &[StateEvent],
        state: &mut GameState,
        now: Instant,
    ) -> Option<Step> {
        if changes
            .iter()
            .any(|c| matches!(c, StateEvent::MovementBlocked { .. }))
        {
            self.target = None;
        }
        let (id, name, aimed) = self.target.clone()?;
        let Some(them) = state.player(id).cloned() else {
            self.target = None;
            self.notes.push(Note::Lost { name });
            return None;
        };
        let me = state.local_player()?.clone();
        let distance = (them.x - me.x).hypot(them.y - me.y);
        if distance <= rules::TRADE_RANGE {
            self.target = None;
            return match state.trade_initiate(now, id) {
                Ok(message) => Some(Step::Send(message)),
                Err(why) => {
                    self.notes.push(Note::Refused(why));
                    None
                }
            };
        }
        if (them.x - aimed[0]).hypot(them.y - aimed[1]) > DRIFT {
            let [x, y] = approach([me.x, me.y], [them.x, them.y]);
            self.target = Some((id, name, [them.x, them.y]));
            return Some(Step::WalkTo { x, y });
        }
        None
    }

    /// Everything worth telling the player since the last call.
    pub fn take_notes(&mut self) -> Vec<Note> {
        std::mem::take(&mut self.notes)
    }
}

/// One stack in a draft: which item, and how many of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub item_id: Uuid,
    pub quantity: i32,
}

/// Why a draft can't be sent as it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// More Strands than you hold.
    Short { strands: i64, short: i64 },
}

/// Your side of the trade window as you build it.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    /// The trade this draft is for. A new trade starts an empty draft.
    trade_id: Option<Uuid>,
    lines: Vec<Line>,
    strands: i64,
    /// What was last sent, so a refused offer isn't sent again until you
    /// change it. An open trade starts with an empty offer on the server.
    sent: (Vec<Line>, i64),
    /// When the draft last changed, while it differs from what was sent.
    changed: Option<Instant>,
}

impl Draft {
    /// The stacks offered, in the order they went in.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn strands(&self) -> i64 {
        self.strands
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.strands == 0
    }

    /// Keeps the draft true to the trade and the inventory: emptied when a
    /// trade starts or ends, each stack that left the inventory dropped,
    /// and each quantity held to what the stack has now.
    pub fn sync(&mut self, open: Option<&OpenTrade>, inventory: Option<&Container>, now: Instant) {
        let id = open.map(|o| o.trade_id);
        if id != self.trade_id {
            *self = Draft {
                trade_id: id,
                ..Draft::default()
            };
            return;
        }
        let Some(inventory) = inventory else {
            return;
        };
        let before = self.lines.clone();
        self.lines
            .retain_mut(|line| match inventory.find(line.item_id) {
                Some(item) => {
                    line.quantity = line.quantity.min(item.quantity);
                    line.quantity > 0
                }
                None => false,
            });
        if self.lines != before {
            self.touch(now);
        }
    }

    /// Offers `item`, the whole stack. A stack already in it goes up to the
    /// whole stack.
    pub fn add(&mut self, item: &SlotItem, now: Instant) {
        if self.trade_id.is_none() || item.quantity < 1 {
            return;
        }
        match self.lines.iter_mut().find(|l| l.item_id == item.item_id) {
            Some(line) if line.quantity == item.quantity => return,
            Some(line) => line.quantity = item.quantity,
            None => self.lines.push(Line {
                item_id: item.item_id,
                quantity: item.quantity,
            }),
        }
        self.touch(now);
    }

    /// Sets how many of `item_id` are offered, at least one and at most
    /// `stack`.
    pub fn set_quantity(&mut self, item_id: Uuid, quantity: i32, stack: i32, now: Instant) {
        let quantity = quantity.clamp(1, stack.max(1));
        if let Some(line) = self.lines.iter_mut().find(|l| l.item_id == item_id) {
            if line.quantity != quantity {
                line.quantity = quantity;
                self.touch(now);
            }
        }
    }

    /// Takes `item_id` out of the offer.
    pub fn remove(&mut self, item_id: Uuid, now: Instant) {
        let before = self.lines.len();
        self.lines.retain(|l| l.item_id != item_id);
        if self.lines.len() != before {
            self.touch(now);
        }
    }

    /// Sets the Strands offered, never below nothing.
    pub fn set_strands(&mut self, strands: i64, now: Instant) {
        let strands = strands.max(0);
        if self.trade_id.is_some() && strands != self.strands {
            self.strands = strands;
            self.touch(now);
        }
    }

    fn touch(&mut self, now: Instant) {
        self.changed = Some(now);
    }

    /// What's wrong with the draft as it stands, if anything. `balance` is
    /// yours, once known.
    pub fn problem(&self, balance: Option<i64>) -> Option<Problem> {
        let balance = balance?;
        (self.strands > balance).then_some(Problem::Short {
            strands: self.strands,
            short: self.strands - balance,
        })
    }

    /// The server's last word on your offer is this draft.
    pub fn confirmed(&self, mine: &Offer) -> bool {
        if mine.strands != self.strands || mine.items.len() != self.lines.len() {
            return false;
        }
        self.lines.iter().all(|l| {
            mine.items
                .iter()
                .any(|i| i.item_id == l.item_id && i.quantity == l.quantity)
        })
    }

    /// The draft still has to reach the server: it differs from what was
    /// last sent, or is waiting to settle.
    pub fn unsent(&self) -> bool {
        (self.lines.clone(), self.strands) != self.sent
    }

    /// The offer to send now, if the draft has changed since it was last
    /// sent, has sat still for [`SETTLE`], and can go (`balance`, see
    /// [`Draft::problem`]). Taking it counts it as sent.
    pub fn due(
        &mut self,
        now: Instant,
        balance: Option<i64>,
    ) -> Option<(Vec<TradeOfferItem>, i64)> {
        self.trade_id?;
        if !self.unsent() || self.problem(balance).is_some() {
            return None;
        }
        if self
            .changed
            .is_some_and(|at| now.duration_since(at) < SETTLE)
        {
            return None;
        }
        self.sent = (self.lines.clone(), self.strands);
        self.changed = None;
        let items = self
            .lines
            .iter()
            .map(|l| TradeOfferItem {
                item_id: l.item_id.to_string(),
                quantity: l.quantity,
                // The lowest go first; the server settles exactly which.
                // Choosing qualities here is #205
                qualities: None,
            })
            .collect();
        Some((items, self.strands))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::ConnectionState;
    use capitol_protocol::ws::{PlayerPosition, ServerMessage as M, TradeItemInfo};

    const ME: Uuid = Uuid::from_u128(1);
    const BRAM: Uuid = Uuid::from_u128(2);
    const TRADE: Uuid = Uuid::from_u128(50);
    const ROPE: Uuid = Uuid::from_u128(60);
    const FIBER: Uuid = Uuid::from_u128(61);

    fn at(state: &mut GameState, me: [f64; 2], bram: [f64; 2]) -> Vec<StateEvent> {
        state.apply_ws(&M::Positions {
            players: vec![
                PlayerPosition {
                    id: ME,
                    name: "Ada".into(),
                    x: me[0],
                    y: me[1],
                },
                PlayerPosition {
                    id: BRAM,
                    name: "Bram".into(),
                    x: bram[0],
                    y: bram[1],
                },
            ],
        })
    }

    fn world(bram_x: f64) -> GameState {
        let mut state = GameState::new();
        state.apply_connection(&ConnectionState::Connected { player_id: ME });
        at(&mut state, [4000.0, 4000.0], [bram_x, 4000.0]);
        state
    }

    fn initiate() -> Step {
        Step::Send(ClientMessage::TradeInitiate {
            target_player_id: BRAM.to_string(),
        })
    }

    #[test]
    fn in_range_asks_at_once() {
        let mut state = world(4060.0);
        let mut a = Approach::default();
        assert_eq!(a.click(BRAM, &mut state, Instant::now()), Some(initiate()));
        assert_eq!(a.target(), None);
    }

    #[test]
    fn out_of_range_walks_closer_then_asks_on_arrival() {
        let mut state = world(4300.0);
        let mut a = Approach::default();
        let now = Instant::now();
        let step = a.click(BRAM, &mut state, now);
        assert_eq!(
            step,
            Some(Step::WalkTo {
                x: 4300.0 - rules::TRADE_RANGE * APPROACH,
                y: 4000.0
            })
        );
        assert_eq!(a.target(), Some(BRAM));
        // Halfway there, nothing yet.
        let changes = at(&mut state, [4150.0, 4000.0], [4300.0, 4000.0]);
        assert_eq!(a.observe(&changes, &mut state, now), None);
        let changes = at(&mut state, [4250.0, 4000.0], [4300.0, 4000.0]);
        assert_eq!(a.observe(&changes, &mut state, now), Some(initiate()));
        assert_eq!(a.target(), None);
    }

    #[test]
    fn a_partner_who_wanders_off_is_followed() {
        let mut state = world(4300.0);
        let mut a = Approach::default();
        let now = Instant::now();
        a.click(BRAM, &mut state, now);
        let changes = at(&mut state, [4100.0, 4000.0], [4300.0, 4300.0]);
        let Some(Step::WalkTo { x, y }) = a.observe(&changes, &mut state, now) else {
            panic!("not re-aimed");
        };
        let d = (x - 4300.0).hypot(y - 4300.0);
        assert!((d - rules::TRADE_RANGE * APPROACH).abs() < 1e-6, "{d}");
        // A small drift isn't worth a new walk.
        let changes = at(&mut state, [4150.0, 4050.0], [4310.0, 4310.0]);
        assert_eq!(a.observe(&changes, &mut state, now), None);
    }

    #[test]
    fn a_partner_who_leaves_or_a_walk_elsewhere_drops_the_target() {
        let mut state = world(4300.0);
        let mut a = Approach::default();
        let now = Instant::now();
        a.click(BRAM, &mut state, now);
        let changes = state.apply_ws(&M::PlayerLeft { player_id: BRAM });
        assert_eq!(a.observe(&changes, &mut state, now), None);
        assert_eq!(
            a.take_notes(),
            vec![Note::Lost {
                name: "Bram".into()
            }]
        );

        let mut state = world(4300.0);
        a.click(BRAM, &mut state, now);
        a.walked();
        let changes = at(&mut state, [4250.0, 4000.0], [4300.0, 4000.0]);
        assert_eq!(a.observe(&changes, &mut state, now), None);
    }

    #[test]
    fn clicking_yourself_falls_through_and_asking_twice_is_refused() {
        let mut state = world(4060.0);
        let mut a = Approach::default();
        let now = Instant::now();
        assert_eq!(a.click(ME, &mut state, now), None);
        a.click(BRAM, &mut state, now);
        assert_eq!(a.click(BRAM, &mut state, now), Some(Step::Hold));
        assert_eq!(
            a.take_notes(),
            vec![Note::Refused(TradeRefusal::AlreadyAsked)]
        );
    }

    fn stack(id: Uuid, quantity: i32, slot: i32) -> SlotItem {
        SlotItem::new(id, "rope".into(), "Rope".into(), 70, quantity, slot)
    }

    fn inventory(items: &[SlotItem]) -> Container {
        Container::new(Uuid::from_u128(100), "player_inventory", 24, 6, items)
    }

    fn open() -> OpenTrade {
        OpenTrade {
            trade_id: TRADE,
            partner_id: BRAM,
            partner_name: "Bram".into(),
            mine: Offer::default(),
            theirs: Offer::default(),
            i_accepted: false,
            they_accepted: false,
        }
    }

    #[test]
    fn a_draft_waits_to_settle_then_sends_once() {
        let t0 = Instant::now();
        let inv = inventory(&[stack(ROPE, 3, 0)]);
        let trade = open();
        let mut d = Draft::default();
        d.sync(Some(&trade), Some(&inv), t0);
        // A fresh trade's empty offer is already on the server.
        assert_eq!(d.due(t0, Some(100)), None);
        d.add(inv.find(ROPE).unwrap(), t0);
        d.set_strands(25, t0 + Duration::from_millis(200));
        assert_eq!(d.due(t0 + Duration::from_millis(300), Some(100)), None);
        let sent = d.due(t0 + Duration::from_millis(700), Some(100));
        assert_eq!(
            sent,
            Some((
                vec![TradeOfferItem {
                    item_id: ROPE.to_string(),
                    quantity: 3,
                    qualities: None,
                }],
                25
            ))
        );
        assert_eq!(d.due(t0 + Duration::from_secs(5), Some(100)), None);
        assert!(!d.unsent());
    }

    #[test]
    fn a_draft_is_confirmed_by_the_matching_echo() {
        let t0 = Instant::now();
        let inv = inventory(&[stack(ROPE, 3, 0)]);
        let mut trade = open();
        let mut d = Draft::default();
        d.sync(Some(&trade), Some(&inv), t0);
        d.add(inv.find(ROPE).unwrap(), t0);
        d.set_quantity(ROPE, 2, 3, t0);
        assert!(!d.confirmed(&trade.mine));
        trade.mine = Offer {
            items: vec![TradeItemInfo {
                item_id: ROPE,
                item_type: "rope".into(),
                item_name: "Rope".into(),
                quantity: 2,
                quality: 70,
                units: capitol_protocol::rules::stacking::Units::uniform(2, 70),
            }],
            strands: 0,
        };
        assert!(d.confirmed(&trade.mine));
    }

    #[test]
    fn too_many_strands_is_held_back_until_fixed() {
        let t0 = Instant::now();
        let trade = open();
        let mut d = Draft::default();
        d.sync(Some(&trade), None, t0);
        d.set_strands(150, t0);
        let later = t0 + Duration::from_secs(1);
        assert_eq!(
            d.problem(Some(100)),
            Some(Problem::Short {
                strands: 150,
                short: 50
            })
        );
        assert_eq!(d.due(later, Some(100)), None);
        d.set_strands(100, later);
        assert_eq!(d.due(later + SETTLE, Some(100)), Some((vec![], 100)));
    }

    #[test]
    fn stacks_that_leave_the_inventory_leave_the_draft() {
        let t0 = Instant::now();
        let trade = open();
        let mut d = Draft::default();
        let inv = inventory(&[stack(ROPE, 3, 0), stack(FIBER, 10, 1)]);
        d.sync(Some(&trade), Some(&inv), t0);
        d.add(inv.find(ROPE).unwrap(), t0);
        d.add(inv.find(FIBER).unwrap(), t0);
        d.due(t0 + SETTLE, None);
        // The rope went to the crafting box, and the fiber's stack shrank.
        let inv = inventory(&[stack(FIBER, 4, 1)]);
        d.sync(Some(&trade), Some(&inv), t0 + SETTLE);
        assert_eq!(
            d.lines(),
            &[Line {
                item_id: FIBER,
                quantity: 4
            }]
        );
        assert!(d.unsent());
    }

    #[test]
    fn a_new_trade_starts_an_empty_draft() {
        let t0 = Instant::now();
        let mut trade = open();
        let mut d = Draft::default();
        d.sync(Some(&trade), None, t0);
        d.set_strands(5, t0);
        trade.trade_id = Uuid::from_u128(51);
        d.sync(Some(&trade), None, t0);
        assert!(d.is_empty());
        d.sync(None, None, t0);
        d.set_strands(5, t0);
        assert!(d.is_empty(), "no trade, no draft");
    }
}
