//! The hall's board of commissions (Commons S2.3), as the client keeps it:
//! every open commission, your standing claims, the commission you're
//! posting, the delivery you're making, and the calls for them.
//!
//! [`Board`] is pure. The session makes the call [`Board::wants`] names, one
//! at a time, and hands back the answer. The board is fetched when you sign
//! in, then kept live from `commission_posted`, `commission_updated` and
//! `commission_closed`, which go to everyone; your claims are fetched with
//! it and again after a claim's notice. What you do waits in a queue, oldest
//! first.
//!
//! Posting and delivering happen at the Exchange hall (D5,
//! `rules::market::at_the_hall`). Away from it, the session walks you there
//! and the ask is [`Board::hold`]ed until you arrive ([`Board::release`]),
//! or dropped if you walk off ([`Board::drop_held`]). Reading, claiming and
//! cancelling work from anywhere.
//!
//! The numbers are the court's own: the fee is `rules::court::court_fee`,
//! what's open to you `unclaimed` and `deliverable`, and which units go
//! `delivery`. The words are here too, so they're tested without egui.

use std::collections::VecDeque;

use capitol_protocol::rest::{
    ClaimResponse, ClaimsResponse, Commission, CommissionClaim, CommissionDelivery, CommissionsResponse,
    DeliverCommission, PostCommission, SlotItem,
};
use capitol_protocol::rules::court::{self, ClaimStatus, CommissionStatus};
use capitol_protocol::rules::market::grade_floor;
use capitol_protocol::rules::stacking::Units;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::state::Container;

/// The grades a commission can ask for, best first. F takes anything.
pub const GRADES: [&str; 5] = ["A", "B", "C", "D", "F"];

/// A call [`Board`] wants made.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// `GET /commissions`: every open one
    Board,
    /// `GET /commissions/claims`: yours, standing
    Claims,
    /// `POST /commissions`, at the hall
    Post(PostCommission),
    /// `POST /commissions/{id}/deliver`, at the hall
    Deliver { commission_id: Uuid, request: DeliverCommission },
    /// `DELETE /commissions/{id}`
    Cancel(Uuid),
    /// `POST /commissions/{id}/claim`
    Claim { commission_id: Uuid, quantity: i32 },
    /// `DELETE /commissions/{id}/claim`
    Release(Uuid),
}

impl Ask {
    /// Whether the server takes it only at the Exchange hall.
    pub fn at_the_hall(&self) -> bool {
        matches!(self, Ask::Post(_) | Ask::Deliver { .. })
    }
}

/// Which commissions the page lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Show {
    /// Everyone's
    #[default]
    All,
    /// Those your pack could deliver to now
    CanFill,
    /// Your own
    Mine,
}

/// What the page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Showing {
    /// The cards
    #[default]
    List,
    /// The form for a new commission
    Post,
    /// The delivery to one
    Deliver(Uuid),
}

/// A commission being written, before it's posted.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub item_type: String,
    /// The lowest grade taken, one of [`GRADES`]
    pub grade: String,
    pub quantity: i32,
    /// Strands each
    pub price: i64,
    /// Due in this many days, [`court::MIN_DUE_DAYS`] to [`court::MAX_DUE_DAYS`]
    pub due_days: i64,
}

impl Draft {
    /// A fresh one for `item_type`, at `price` each (its target, say).
    pub fn new(item_type: &str, price: i64) -> Self {
        Draft { item_type: item_type.into(), grade: "C".into(), quantity: 10, price: price.max(1), due_days: 3 }
    }

    /// The lowest quality taken: the grade's floor.
    pub fn min_quality(&self) -> i32 {
        grade_floor(&self.grade).unwrap_or(0)
    }

    /// What it escrows: `price × quantity`.
    pub fn value(&self) -> i64 {
        court::commission_value(self.price, self.quantity)
    }

    /// The court's fee on top, the server's own formula.
    pub fn fee(&self) -> i64 {
        court::court_fee(self.value())
    }

    /// What posting takes from your balance: the escrow and the fee.
    pub fn total(&self) -> i64 {
        self.value().saturating_add(self.fee())
    }

    pub fn request(&self) -> PostCommission {
        PostCommission {
            item_type: self.item_type.clone(),
            min_quality: self.min_quality(),
            quantity: self.quantity,
            price: self.price,
            due_days: self.due_days,
        }
    }

    /// What stops it being posted, with `strands` in hand (`None`: not known
    /// yet) and `open` of yours already up, in words; `None` if it can go.
    pub fn stopped(&self, strands: Option<i64>, open: usize) -> Option<String> {
        if let Err(e) = court::validate_commission(self.price, self.quantity, self.min_quality(), self.due_days) {
            return Some(e.message().into());
        }
        if open as i64 >= court::MAX_OPEN_COMMISSIONS {
            return Some(format!("You have {} commissions up, the most anyone may", court::MAX_OPEN_COMMISSIONS));
        }
        match strands {
            Some(s) if s < self.total() => Some(format!("It takes {} Strands, and you have {s}", self.total())),
            _ => None,
        }
    }
}

/// A delivery being made: which stack from your pack, and which of its units.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub commission_id: Uuid,
    /// The stack the units come from
    pub item_id: Option<Uuid>,
    /// Which units, picked by quality; `None` takes the lowest that meet it
    pub picked: Option<Units>,
    /// The most to deliver
    pub quantity: i32,
}

/// What a delivery would do, previewed.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryQuote {
    /// The units that would go
    pub units: Units,
    /// What you'd be paid for them
    pub paid: i64,
}

/// The board as the client has it, and the calls for it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Board {
    /// Every open commission, newest first
    open: Vec<Commission>,
    /// Your standing claims
    claims: Vec<CommissionClaim>,
    loaded: bool,
    stale: bool,
    claims_stale: bool,
    /// Posts, deliveries, claims and the rest not yet asked, oldest first
    pending: VecDeque<Ask>,
    /// One held until you reach the hall
    held: Option<Ask>,
    /// The call that's out
    asked: Option<Ask>,
    /// Why the last thing you did was refused
    refused: Option<String>,
    /// What the last thing you did did
    done: Option<String>,
    online: bool,
    pub show: Show,
    /// Only this item; every item without
    pub item: Option<String>,
    showing: Showing,
    pub draft: Option<Draft>,
    pub delivery: Option<Delivery>,
}

impl Board {
    /// Signed in, or back after a drop: whatever was out won't be answered,
    /// so the board and your claims are asked again.
    pub fn connected(&mut self) {
        self.online = true;
        self.asked = None;
        self.stale = true;
        self.claims_stale = true;
    }

    /// Signed out: nothing is kept.
    pub fn signed_out(&mut self) {
        *self = Board::default();
    }

    /// The call to make next, if any. Mark it sent with [`Board::sent`].
    pub fn wants(&self) -> Option<Ask> {
        if !self.online || self.asked.is_some() {
            return None;
        }
        if let Some(ask) = self.pending.front() {
            return Some(ask.clone());
        }
        if self.stale || !self.loaded {
            return Some(Ask::Board);
        }
        self.claims_stale.then_some(Ask::Claims)
    }

    pub fn sent(&mut self, ask: Ask) {
        if self.pending.front() == Some(&ask) {
            self.pending.pop_front();
        }
        match ask {
            Ask::Board => self.stale = false,
            Ask::Claims => self.claims_stale = false,
            _ => {}
        }
        self.asked = Some(ask);
    }

    /// `GET /commissions` came back: the board as it stands.
    pub fn listed(&mut self, response: CommissionsResponse) {
        if self.asked == Some(Ask::Board) {
            self.asked = None;
        }
        self.open = response.commissions.into_iter().filter(|c| c.status == CommissionStatus::Open).collect();
        self.open.sort_by_key(|c| std::cmp::Reverse((c.posted_at, c.id)));
        self.loaded = true;
        self.tidy();
    }

    /// `GET /commissions/claims` came back: your standing claims.
    pub fn claims_listed(&mut self, response: ClaimsResponse) {
        if self.asked == Some(Ask::Claims) {
            self.asked = None;
        }
        self.claims = response.claims.into_iter().filter(|c| c.status == ClaimStatus::Open).collect();
    }

    /// A commission as the server last said it: from the board's events, or
    /// an answer. An open one goes on the board, or replaces itself there;
    /// a closed one comes off, with your claims on it.
    pub fn apply(&mut self, commission: Commission) {
        let at = self.open.iter().position(|c| c.id == commission.id);
        if commission.status != CommissionStatus::Open {
            if let Some(i) = at {
                self.open.remove(i);
            }
            self.claims.retain(|c| c.commission_id != commission.id);
            self.tidy();
            return;
        }
        match at {
            Some(i) => self.open[i] = commission,
            None => {
                let i = self.open.partition_point(|c| (c.posted_at, c.id) > (commission.posted_at, commission.id));
                self.open.insert(i, commission);
            }
        }
    }

    /// A claim's notice came (`commission_claimed`, `claim_due_soon`,
    /// `claim_lapsed`): your claims are asked again.
    pub fn claims_changed(&mut self) {
        self.claims_stale = true;
    }

    /// Your commission went up, or came down at your word.
    pub fn posted(&mut self, commission: Commission) {
        let posting = matches!(self.asked, Some(Ask::Post(_)));
        if matches!(self.asked, Some(Ask::Post(_) | Ask::Cancel(_))) {
            self.asked = None;
        }
        self.refused = None;
        self.done = Some(if posting {
            "Pinned to the board. The money is down.".into()
        } else if commission.withdrawn {
            "Taken down. What's unclaimed came back; the claims still stand.".into()
        } else {
            "Taken down. What was unspent came back.".into()
        });
        if posting {
            self.draft = None;
            if self.showing == Showing::Post {
                self.showing = Showing::List;
            }
        }
        self.apply(commission);
    }

    /// Your delivery went in, and was paid.
    pub fn delivered(&mut self, delivery: CommissionDelivery, item_name: &str) {
        if matches!(self.asked, Some(Ask::Deliver { .. })) {
            self.asked = None;
        }
        self.refused = None;
        self.done = Some(format!(
            "Delivered {} {item_name} and was paid {} Strands.",
            delivery.units.count(),
            delivery.paid
        ));
        self.delivery = None;
        if matches!(self.showing, Showing::Deliver(_)) {
            self.showing = Showing::List;
        }
        // A delivery counts against your claim first
        self.claims_stale = true;
        self.apply(delivery.commission);
    }

    /// Your claim was made, or ended.
    pub fn claimed(&mut self, response: ClaimResponse) {
        if matches!(self.asked, Some(Ask::Claim { .. } | Ask::Release(_))) {
            self.asked = None;
        }
        self.refused = None;
        let claim = response.claim;
        self.done = Some(match claim.status {
            ClaimStatus::Open => format!("Claimed {}. It's held for you until it runs out.", claim.quantity),
            ClaimStatus::Lapsed => "Given up late: it counts as a lapse.".into(),
            _ => "Given up in good time.".into(),
        });
        self.claims.retain(|c| c.id != claim.id);
        if claim.status == ClaimStatus::Open {
            self.claims.push(claim);
        }
        self.apply(response.commission);
    }

    /// The call that was out failed, saying `why`. What you did says so;
    /// a read is asked again.
    pub fn failed(&mut self, why: String) {
        match self.asked.take() {
            Some(Ask::Board) => self.stale = true,
            Some(Ask::Claims) => self.claims_stale = true,
            Some(_) => {
                self.done = None;
                self.refused = Some(why);
            }
            None => {}
        }
    }

    /// Do `ask` when it can be: now, or at the hall. See [`Board::hold`].
    pub fn queue(&mut self, ask: Ask) {
        self.refused = None;
        self.done = None;
        if !self.pending.contains(&ask) && self.asked.as_ref() != Some(&ask) {
            self.pending.push_back(ask);
        }
    }

    /// Keep `ask` until you reach the hall: the session is walking you there.
    pub fn hold(&mut self, ask: Ask) {
        self.refused = None;
        self.done = None;
        self.held = Some(ask);
    }

    /// What's held for the hall, if anything.
    pub fn held(&self) -> Option<&Ask> {
        self.held.as_ref()
    }

    /// You reached the hall: what was held goes.
    pub fn release(&mut self) {
        if let Some(ask) = self.held.take() {
            self.queue(ask);
        }
    }

    /// You walked off before reaching the hall: what was held is dropped.
    pub fn drop_held(&mut self) {
        if let Some(ask) = self.held.take() {
            self.refused = Some(
                match ask {
                    Ask::Post(_) => "You walked off, so nothing was posted",
                    _ => "You walked off, so nothing was delivered",
                }
                .into(),
            );
        }
    }

    /// Something you did is on its way, here or to the hall.
    pub fn busy(&self) -> bool {
        self.held.is_some()
            || self.pending.iter().any(|a| !matches!(a, Ask::Board | Ask::Claims))
            || self.asked.as_ref().is_some_and(|a| !matches!(a, Ask::Board | Ask::Claims))
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn done(&self) -> Option<&str> {
        self.done.as_deref()
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn showing(&self) -> Showing {
        self.showing
    }

    /// Back to the cards. A draft half written is kept.
    pub fn back(&mut self) {
        self.showing = Showing::List;
        self.refused = None;
    }

    /// Open the form for a new commission: the one in hand, or a fresh one
    /// for `item_type` at `price`.
    pub fn write(&mut self, item_type: &str, price: i64) {
        if self.draft.is_none() {
            self.draft = Some(Draft::new(item_type, price));
        }
        self.refused = None;
        self.done = None;
        self.showing = Showing::Post;
    }

    /// Post the draft, if it can go.
    pub fn post(&mut self, strands: Option<i64>, me: Option<Uuid>) -> Option<Ask> {
        let draft = self.draft.as_ref()?;
        if self.busy() || draft.stopped(strands, self.mine(me).len()).is_some() {
            return None;
        }
        Some(Ask::Post(draft.request()))
    }

    /// Open the delivery to `id`, from the first stack in `pack` that has
    /// units meeting it.
    pub fn start_delivery(&mut self, id: Uuid, pack: Option<&Container>, me: Option<Uuid>) {
        let Some(c) = self.commission(id) else { return };
        let stack = stacks_for(c, pack).first().copied();
        let item_id = stack.map(|s| s.item_id);
        // As many as are open to you, of those in the stack that meet it
        let quantity = self.open_to(c, me).min(stack.map_or(0, |s| meeting(&s.units, c.min_quality) as i32)).max(1);
        self.delivery = Some(Delivery { commission_id: id, item_id, picked: None, quantity });
        self.refused = None;
        self.done = None;
        self.showing = Showing::Deliver(id);
    }

    /// The delivery as asked, if it previews.
    pub fn deliver(&self, pack: Option<&Container>, me: Option<Uuid>) -> Option<Ask> {
        let d = self.delivery.as_ref()?;
        let c = self.commission(d.commission_id)?;
        let stack = stack_in(pack, d.item_id?)?;
        if self.busy() || self.quote(c, stack, d, me).is_err() {
            return None;
        }
        Some(Ask::Deliver {
            commission_id: c.id,
            request: DeliverCommission { item_id: stack.item_id, qualities: d.picked.clone(), quantity: Some(d.quantity) },
        })
    }

    /// What delivering `d` from `stack` to `c` would do, or why it can't.
    pub fn quote(&self, c: &Commission, stack: &SlotItem, d: &Delivery, me: Option<Uuid>) -> Result<DeliveryQuote, String> {
        if me == Some(c.poster_id) {
            return Err("You can't deliver to your own commission".into());
        }
        if stack.item_type != c.item_type {
            return Err("That isn't what's wanted".into());
        }
        let wanted = self.open_to(c, me);
        if wanted < 1 {
            return Err(if c.withdrawn {
                "It's been taken down; only its claims are delivered".into()
            } else {
                "Everything still wanted is claimed".into()
            });
        }
        let units = court::delivery(&stack.units, c.min_quality, wanted, d.picked.as_ref(), Some(d.quantity))
            .map_err(|e| e.message().to_owned())?;
        let paid = c.price.saturating_mul(units.count());
        Ok(DeliveryQuote { units, paid })
    }

    /// Every open commission, newest first.
    pub fn open(&self) -> &[Commission] {
        &self.open
    }

    pub fn commission(&self, id: Uuid) -> Option<&Commission> {
        self.open.iter().find(|c| c.id == id)
    }

    /// Your standing claims.
    pub fn claims(&self) -> &[CommissionClaim] {
        &self.claims
    }

    /// Your standing claim on `id`, if you have one.
    pub fn my_claim(&self, id: Uuid) -> Option<&CommissionClaim> {
        self.claims.iter().find(|c| c.commission_id == id)
    }

    /// Your commissions on the board.
    pub fn mine(&self, me: Option<Uuid>) -> Vec<&Commission> {
        self.open.iter().filter(|c| Some(c.poster_id) == me).collect()
    }

    /// What anyone may still claim or deliver of `c`.
    pub fn unclaimed(c: &Commission) -> i32 {
        court::unclaimed(c.quantity - c.delivered, c.claimed, c.withdrawn)
    }

    /// The most you may bring to `c`: the unclaimed part and what your claim
    /// still owes. Nothing to your own.
    pub fn open_to(&self, c: &Commission, me: Option<Uuid>) -> i32 {
        if me == Some(c.poster_id) {
            return 0;
        }
        let own = self.my_claim(c.id).map_or(0, |cl| cl.quantity - cl.delivered);
        court::deliverable(Board::unclaimed(c), own)
    }

    /// The commissions `pack` could deliver to now: someone else's, with
    /// some open to you, and a stack holding a unit at its quality.
    pub fn can_fill(&self, pack: Option<&Container>, me: Option<Uuid>) -> Vec<&Commission> {
        self.open
            .iter()
            .filter(|c| self.open_to(c, me) > 0 && !stacks_for(c, pack).is_empty())
            .collect()
    }

    /// The cards the page lists, by [`Board::show`] and [`Board::item`].
    pub fn listed_now(&self, pack: Option<&Container>, me: Option<Uuid>) -> Vec<&Commission> {
        let shown: Vec<&Commission> = match self.show {
            Show::All => self.open.iter().collect(),
            Show::CanFill => self.can_fill(pack, me),
            Show::Mine => self.mine(me),
        };
        shown.into_iter().filter(|c| self.item.as_ref().is_none_or(|i| *i == c.item_type)).collect()
    }

    /// The item types on the board, for the filter, in the order first seen.
    pub fn items(&self) -> Vec<&str> {
        let mut items: Vec<&str> = Vec::new();
        for c in &self.open {
            if !items.contains(&c.item_type.as_str()) {
                items.push(&c.item_type);
            }
        }
        items
    }

    /// Drops a delivery or a filter whose commission came off.
    fn tidy(&mut self) {
        if let Showing::Deliver(id) = self.showing {
            if self.commission(id).is_none() {
                self.showing = Showing::List;
                self.delivery = None;
            }
        }
    }
}

/// The stacks in `pack` of what `c` wants that hold a unit at its quality,
/// best-stocked first.
pub fn stacks_for<'a>(c: &Commission, pack: Option<&'a Container>) -> Vec<&'a SlotItem> {
    let mut stacks: Vec<&SlotItem> = pack
        .into_iter()
        .flat_map(|p| p.slots.iter().flatten())
        .filter(|s| s.item_type == c.item_type && s.units.max().is_some_and(|q| q >= c.min_quality))
        .collect();
    stacks.sort_by_key(|s| std::cmp::Reverse(meeting(&s.units, c.min_quality)));
    stacks
}

/// How many of `units` are at `min_quality` or over.
pub fn meeting(units: &Units, min_quality: i32) -> i64 {
    units.pairs().filter(|(q, _)| *q >= min_quality).map(|(_, n)| n as i64).sum()
}

fn stack_in(pack: Option<&Container>, item_id: Uuid) -> Option<&SlotItem> {
    pack?.slots.iter().flatten().find(|s| s.item_id == item_id)
}

/// The grade a commission asks for, in words: "C and over", or "any".
pub fn grade_words(min_quality: i32) -> String {
    if min_quality <= 0 {
        return "any quality".into();
    }
    let grade = capitol_protocol::rules::quality_grade(min_quality);
    if grade_floor(grade) == Some(min_quality) {
        format!("{grade} and over")
    } else {
        format!("{min_quality} and over")
    }
}

/// How long until `at`, in the court's days and hours: "2 d 4 h", "5 h",
/// "40 min", or "now" once it's past.
pub fn time_left(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (at - now).num_seconds();
    if secs <= 0 {
        return "now".into();
    }
    let (days, hours, mins) = (secs / court::DAY_SECS, secs % court::DAY_SECS / 3600, secs % 3600 / 60);
    match (days, hours) {
        (0, 0) => format!("{} min", mins.max(1)),
        (0, h) => format!("{h} h"),
        (d, 0) => format!("{d} d"),
        (d, h) => format!("{d} d {h} h"),
    }
}

/// The glint's hover line: how many commissions on the board your pack
/// could fill.
pub fn fill_words(n: usize) -> Option<String> {
    (n > 0).then(|| format!("{n} wanted that you could fill"))
}

/// The toast for a claim on your commission.
pub fn claimed_words(name: &str, citizen: bool, quantity: i32, item_name: &str, until: DateTime<Utc>, now: DateTime<Utc>) -> (String, String) {
    let who = if citizen { format!("{name}, a citizen,") } else { name.to_owned() };
    (
        "A claim on your commission".into(),
        format!("{who} promised {quantity} {item_name}, held for them for {}.", time_left(until, now)),
    )
}

/// The toast for your claim's last hour.
pub fn due_soon_words(quantity: i32, delivered: i32, item_name: &str, until: DateTime<Utc>, now: DateTime<Utc>) -> (String, String) {
    (
        "Your claim runs out soon".into(),
        format!(
            "{delivered} of the {quantity} {item_name} you promised are in, and {} is left. Bring the rest to the hall.",
            time_left(until, now)
        ),
    )
}

/// The toast for a lapse: yours, when `mine`, or one on your commission.
pub fn lapsed_words(mine: bool, name: &str, citizen: bool, quantity: i32, delivered: i32, item_name: &str) -> (String, String) {
    if mine {
        (
            "Your claim lapsed".into(),
            format!("Only {delivered} of the {quantity} {item_name} you promised came in. The rest is open to others again, and it's on your record."),
        )
    } else {
        let who = if citizen { format!("{name}, a citizen,") } else { name.to_owned() };
        (
            "A claim lapsed".into(),
            format!("{who} brought {delivered} of the {quantity} {item_name} they promised. The rest is open again."),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::ClaimsResponse;

    const ME: u128 = 1;
    const POSTER: u128 = 2;

    fn me() -> Option<Uuid> {
        Some(Uuid::from_u128(ME))
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap() + chrono::Duration::minutes(minutes)
    }

    fn commission(n: u128, item: &str, min_quality: i32, quantity: i32, minutes: i64) -> Commission {
        Commission {
            id: Uuid::from_u128(n),
            poster_id: Uuid::from_u128(POSTER),
            poster_name: "Brin".into(),
            poster_citizen: false,
            item_type: item.into(),
            min_quality,
            quantity,
            delivered: 0,
            price: 6,
            escrow_strands: 6 * quantity as i64,
            fee: court::court_fee(6 * quantity as i64),
            posted_at: at(minutes),
            due_at: at(minutes + 3 * 24 * 60),
            status: CommissionStatus::Open,
            claimed: 0,
            withdrawn: false,
        }
    }

    fn claim(n: u128, on: u128, quantity: i32, delivered: i32) -> CommissionClaim {
        CommissionClaim {
            id: Uuid::from_u128(n),
            commission_id: Uuid::from_u128(on),
            claimant_id: Uuid::from_u128(ME),
            claimant_name: "Ada".into(),
            claimant_citizen: false,
            quantity,
            delivered,
            claimed_at: at(0),
            until: at(24 * 60),
            status: ClaimStatus::Open,
        }
    }

    fn stack(n: u128, item: &str, units: Units) -> SlotItem {
        SlotItem {
            item_id: Uuid::from_u128(n),
            item_type: item.into(),
            item_name: item.into(),
            quality: units.mean().unwrap_or(0),
            quality_grade: "C".into(),
            quantity: units.count() as i32,
            min_quality: units.min().unwrap_or(0),
            units,
            slot_index: n as i32,
            durability: None,
            max_durability: None,
            maker: None,
        }
    }

    fn pack(stacks: Vec<SlotItem>) -> Container {
        let mut slots: Vec<Option<SlotItem>> = stacks.into_iter().map(Some).collect();
        slots.resize(12, None);
        Container { id: Uuid::from_u128(99), container_type: "player_inventory".into(), layout_columns: 4, slots }
    }

    /// Signed in, with `open` on the board and `claims` yours
    fn board(open: Vec<Commission>, claims: Vec<CommissionClaim>) -> Board {
        let mut b = Board::default();
        assert_eq!(b.wants(), None, "not before signing in");
        b.connected();
        let ask = b.wants().unwrap();
        assert_eq!(ask, Ask::Board);
        b.sent(ask);
        assert_eq!(b.wants(), None, "one call at a time");
        b.listed(CommissionsResponse { commissions: open });
        let ask = b.wants().unwrap();
        assert_eq!(ask, Ask::Claims);
        b.sent(ask);
        b.claims_listed(ClaimsResponse { claims });
        assert_eq!(b.wants(), None);
        b
    }

    #[test]
    fn the_store_follows_the_boards_events() {
        let mut b = board(vec![commission(1, "plank", 55, 10, 0)], vec![claim(7, 1, 2, 0)]);
        // Posted: newest first
        b.apply(commission(2, "iron_ore", 0, 5, 10));
        let ids: Vec<u128> = b.open().iter().map(|c| c.id.as_u128()).collect();
        assert_eq!(ids, [2, 1]);
        // Updated: in place
        let mut one = commission(1, "plank", 55, 10, 0);
        one.delivered = 4;
        b.apply(one);
        assert_eq!(b.commission(Uuid::from_u128(1)).unwrap().delivered, 4);
        assert_eq!(b.open().len(), 2);
        // Closed: off, with your claim on it
        let mut one = commission(1, "plank", 55, 10, 0);
        one.status = CommissionStatus::Filled;
        b.apply(one);
        assert_eq!(b.open().len(), 1);
        assert!(b.claims().is_empty(), "the claim ended with it");
        // A claim's notice asks for your claims again
        b.claims_changed();
        assert_eq!(b.wants(), Some(Ask::Claims));
        // A reconnect asks for the lot
        b.connected();
        assert_eq!(b.wants(), Some(Ask::Board));
    }

    #[test]
    fn can_fill_honours_the_grade() {
        let b = board(vec![commission(1, "plank", 55, 10, 0), commission(2, "plank", 90, 10, 1)], vec![]);
        let p = pack(vec![stack(10, "plank", Units::from_pairs([(40, 3), (60, 2)])), stack(11, "rope", Units::uniform(4, 95))]);
        let ids: Vec<u128> = b.can_fill(Some(&p), me()).iter().map(|c| c.id.as_u128()).collect();
        assert_eq!(ids, [1], "the 60s meet C; nothing meets A");
        assert!(b.can_fill(None, me()).is_empty(), "no pack, nothing");
        assert_eq!(fill_words(ids.len()).as_deref(), Some("1 wanted that you could fill"));
        assert_eq!(fill_words(0), None);
    }

    #[test]
    fn can_fill_honours_claims() {
        // All ten claimed by someone else: nothing open to you
        let mut taken = commission(1, "plank", 0, 10, 0);
        taken.claimed = 10;
        // Withdrawn: only its claims are delivered
        let mut withdrawn = commission(2, "plank", 0, 10, 1);
        withdrawn.withdrawn = true;
        // Yours: never
        let mut own = commission(3, "plank", 0, 10, 2);
        own.poster_id = Uuid::from_u128(ME);
        let p = pack(vec![stack(10, "plank", Units::uniform(5, 70))]);
        let b = board(vec![taken.clone(), withdrawn.clone(), own], vec![]);
        assert!(b.can_fill(Some(&p), me()).is_empty());
        // Your own claim on the taken one opens it to you, and the withdrawn one too
        let b = board(vec![taken, withdrawn], vec![claim(7, 1, 3, 1), claim(8, 2, 2, 0)]);
        let c = b.commission(Uuid::from_u128(1)).unwrap();
        assert_eq!(b.open_to(c, me()), 2, "what your claim still owes");
        assert_eq!(b.can_fill(Some(&p), me()).len(), 2);
    }

    #[test]
    fn the_fee_preview_is_the_courts() {
        let mut d = Draft::new("plank", 6);
        d.quantity = 100;
        assert_eq!(d.value(), 600);
        assert_eq!(d.fee(), court::court_fee(600));
        assert_eq!(d.fee(), 6 + 3, "1% and 0.5%, rounded up");
        assert_eq!(d.total(), 609);
        d.quantity = 1;
        assert_eq!(d.fee(), court::court_fee(6), "the filing's least");
        assert_eq!(d.request().min_quality, 55, "C's floor");
        d.grade = "F".into();
        assert_eq!(d.request().min_quality, 0);
    }

    #[test]
    fn what_stops_a_post_is_said() {
        let d = Draft::new("plank", 6);
        assert_eq!(d.stopped(Some(1000), 0), None);
        assert_eq!(d.stopped(Some(10), 0).as_deref(), Some("It takes 66 Strands, and you have 10"));
        assert!(d.stopped(Some(1000), 5).unwrap().contains("the most anyone may"));
        let mut bad = d.clone();
        bad.due_days = 9;
        assert_eq!(bad.stopped(Some(1000), 0).as_deref(), Some("A commission is due in 1 to 7 days"));
        bad.due_days = 3;
        bad.quantity = 0;
        assert!(bad.stopped(None, 0).is_some());
    }

    #[test]
    fn posting_from_afar_holds_it_for_the_hall() {
        let mut b = board(vec![], vec![]);
        b.write("plank", 6);
        assert_eq!(b.showing(), Showing::Post);
        let ask = b.post(Some(1000), me()).unwrap();
        assert!(ask.at_the_hall());
        b.hold(ask.clone());
        assert!(b.busy());
        assert_eq!(b.post(Some(1000), me()), None, "once");
        assert_eq!(b.wants(), None, "not until the hall");
        b.release();
        assert_eq!(b.wants(), Some(ask.clone()));
        b.sent(ask);
        let mut posted = commission(5, "plank", 55, 10, 0);
        posted.poster_id = Uuid::from_u128(ME);
        b.posted(posted);
        assert_eq!((b.showing(), b.draft.is_none(), b.busy()), (Showing::List, true, false));
        assert_eq!(b.mine(me()).len(), 1);
        assert_eq!(b.done(), Some("Pinned to the board. The money is down."));
        // Walking off drops one held
        b.write("plank", 6);
        let ask = b.post(Some(1000), me()).unwrap();
        b.hold(ask);
        b.drop_held();
        assert_eq!(b.refused(), Some("You walked off, so nothing was posted"));
        assert_eq!(b.wants(), None);
        assert!(b.draft.is_some(), "the draft stays in hand");
    }

    #[test]
    fn a_delivery_takes_the_lowest_that_meet_it_or_the_ones_picked() {
        let mut b = board(vec![commission(1, "plank", 55, 4, 0)], vec![]);
        let p = pack(vec![stack(10, "plank", Units::from_pairs([(40, 3), (60, 2), (80, 3)]))]);
        let id = Uuid::from_u128(1);
        b.start_delivery(id, Some(&p), me());
        assert_eq!(b.showing(), Showing::Deliver(id));
        let d = b.delivery.clone().unwrap();
        assert_eq!((d.item_id, d.quantity), (Some(Uuid::from_u128(10)), 4), "4 wanted; 5 meet it");
        let c = b.commission(id).unwrap();
        let s = p.slots[0].as_ref().unwrap();
        let q = b.quote(c, s, &d, me()).unwrap();
        assert_eq!(q.units, Units::from_pairs([(60, 2), (80, 2)]), "the 40s don't meet C, and the best 80 stays");
        assert_eq!(q.paid, 24);
        // Picked by quality
        let mut picked = d.clone();
        picked.picked = Some(Units::uniform(3, 80));
        assert_eq!(b.quote(c, s, &picked, me()).unwrap().units, Units::uniform(3, 80));
        picked.picked = Some(Units::uniform(1, 40));
        assert!(b.quote(c, s, &picked, me()).unwrap_err().contains("under the quality"));
        // It goes, and is paid
        let ask = b.deliver(Some(&p), me()).unwrap();
        let Ask::Deliver { request, .. } = &ask else { panic!() };
        assert_eq!((request.item_id, request.quantity, request.qualities.clone()), (Uuid::from_u128(10), Some(4), None));
        b.queue(ask.clone());
        b.sent(ask);
        let mut after = commission(1, "plank", 55, 4, 0);
        after.delivered = 4;
        after.status = CommissionStatus::Filled;
        b.delivered(CommissionDelivery { commission: after, units: q.units, paid: 24 }, "Planks");
        assert_eq!(b.done(), Some("Delivered 4 Planks and was paid 24 Strands."));
        assert_eq!((b.showing(), b.open().len()), (Showing::List, 0));
        assert_eq!(b.wants(), Some(Ask::Claims), "a delivery counts against a claim");
    }

    #[test]
    fn a_refused_delivery_says_why() {
        let mut b = board(vec![commission(1, "plank", 0, 4, 0)], vec![]);
        let p = pack(vec![stack(10, "plank", Units::uniform(4, 50))]);
        b.start_delivery(Uuid::from_u128(1), Some(&p), me());
        let ask = b.deliver(Some(&p), me()).unwrap();
        b.queue(ask.clone());
        b.sent(ask);
        b.failed("Only at the Exchange hall".into());
        assert_eq!(b.refused(), Some("Only at the Exchange hall"));
        assert_eq!(b.showing(), Showing::Deliver(Uuid::from_u128(1)), "the form stays open");
        // A read that fails is asked again
        b.connected();
        let ask = b.wants().unwrap();
        b.sent(ask);
        b.failed("network".into());
        assert_eq!(b.wants(), Some(Ask::Board));
    }

    #[test]
    fn a_claim_is_held_and_a_release_lets_it_go() {
        let mut b = board(vec![commission(1, "plank", 0, 10, 0)], vec![]);
        b.queue(Ask::Claim { commission_id: Uuid::from_u128(1), quantity: 3 });
        let ask = b.wants().unwrap();
        assert!(!ask.at_the_hall(), "from anywhere");
        b.sent(ask);
        let mut after = commission(1, "plank", 0, 10, 0);
        after.claimed = 3;
        b.claimed(ClaimResponse { claim: claim(7, 1, 3, 0), commission: after.clone() });
        assert_eq!(b.my_claim(Uuid::from_u128(1)).unwrap().quantity, 3);
        assert_eq!(Board::unclaimed(b.commission(Uuid::from_u128(1)).unwrap()), 7);
        assert_eq!(b.open_to(&after, me()), 10, "the 7 and your 3");
        b.queue(Ask::Release(Uuid::from_u128(1)));
        let ask = b.wants().unwrap();
        b.sent(ask);
        let mut released = claim(7, 1, 3, 0);
        released.status = ClaimStatus::Released;
        b.claimed(ClaimResponse { claim: released, commission: commission(1, "plank", 0, 10, 0) });
        assert!(b.claims().is_empty());
        assert_eq!(b.done(), Some("Given up in good time."));
    }

    #[test]
    fn the_filters_list_what_they_say() {
        let mut own = commission(3, "rope", 0, 2, 2);
        own.poster_id = Uuid::from_u128(ME);
        let mut b = board(vec![commission(1, "plank", 0, 10, 0), commission(2, "iron_ore", 0, 5, 1), own], vec![]);
        let p = pack(vec![stack(10, "plank", Units::uniform(4, 50))]);
        let ids = |b: &Board| -> Vec<u128> { b.listed_now(Some(&p), me()).iter().map(|c| c.id.as_u128()).collect() };
        assert_eq!(ids(&b), [3, 2, 1]);
        b.show = Show::CanFill;
        assert_eq!(ids(&b), [1]);
        b.show = Show::Mine;
        assert_eq!(ids(&b), [3]);
        b.show = Show::All;
        b.item = Some("iron_ore".into());
        assert_eq!(ids(&b), [2]);
        assert_eq!(b.items(), ["rope", "iron_ore", "plank"]);
    }

    #[test]
    fn the_words() {
        assert_eq!(grade_words(55), "C and over");
        assert_eq!(grade_words(0), "any quality");
        assert_eq!(grade_words(60), "60 and over");
        assert_eq!(time_left(at(3 * 24 * 60 + 4 * 60 + 10), at(0)), "3 d 4 h");
        assert_eq!(time_left(at(2 * 24 * 60), at(0)), "2 d");
        assert_eq!(time_left(at(300), at(0)), "5 h");
        assert_eq!(time_left(at(40), at(0)), "40 min");
        assert_eq!(time_left(at(0), at(5)), "now");
        let (title, body) = lapsed_words(true, "Ada", false, 4, 1, "Planks");
        assert_eq!(title, "Your claim lapsed");
        assert!(body.starts_with("Only 1 of the 4 Planks"));
        let (_, body) = claimed_words("Cy", true, 4, "Iron Ore", at(24 * 60), at(0));
        assert_eq!(body, "Cy, a citizen, promised 4 Iron Ore, held for them for 1 d.");
    }
}
