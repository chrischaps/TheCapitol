//! The fiber exchange (P7.2): what a deposit gives and what a withdrawal
//! costs, before you confirm. Pure, so it's tested without a server.
//!
//! The numbers are `protocol::rules::deposit_quote` and `withdraw_quote`,
//! the functions the server charges with, so the preview is its answer.
//!
//! - **Deposit** takes fiber stacks from the inventory, as React's panel
//!   does. The server would take fiber from any container you own, but a
//!   stack in the crafting input or a station's box is spoken for.
//! - **Withdraw** gives quality-50 fiber. The server merges it into a fiber
//!   stack in your inventory, else puts it in the first empty slot. It takes
//!   the Strands *before* it looks for that slot, so with neither it keeps
//!   them and answers "Inventory full" (issue #90). [`withdraw`] refuses
//!   that case here instead.

use std::collections::HashSet;

use capitol_protocol::rest::{DepositResponse, SlotItem, WithdrawResponse};
use capitol_protocol::rules::{deposit_quote, withdraw_quote};
use uuid::Uuid;

use crate::state::Container;

/// The item type the exchange takes and gives.
pub const FIBER: &str = "fiber";
/// The quality of withdrawn fiber.
pub const WITHDRAWN_QUALITY: i32 = 50;

/// The fiber stacks in `inventory` that a deposit can take, in slot order.
pub fn fiber_stacks(inventory: &Container) -> Vec<&SlotItem> {
    let mut stacks: Vec<&SlotItem> = inventory.items().filter(|i| i.item_type == FIBER).collect();
    stacks.sort_by_key(|i| i.slot_index);
    stacks
}

/// A deposit, previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositQuote {
    /// Fiber units going in.
    pub fiber: i64,
    pub fee: i64,
    /// Strands coming out, after the fee.
    pub strands: i64,
    /// Your balance afterwards, once your balance is known.
    pub balance_after: Option<i64>,
}

/// Why a deposit can't go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepositRefusal {
    /// No stack is selected.
    NothingSelected,
}

/// The deposit of the `selected` stacks in `inventory`. Ids that aren't
/// fiber stacks there any more (spent, moved) are left out.
pub fn deposit(
    inventory: &Container,
    selected: &HashSet<Uuid>,
    balance: Option<i64>,
) -> Result<DepositQuote, DepositRefusal> {
    let fiber: i64 = fiber_stacks(inventory)
        .into_iter()
        .filter(|i| selected.contains(&i.item_id))
        .map(|i| i.quantity as i64)
        .sum();
    if fiber <= 0 {
        return Err(DepositRefusal::NothingSelected);
    }
    let (fee, strands) = deposit_quote(fiber);
    Ok(DepositQuote {
        fiber,
        fee,
        strands,
        balance_after: balance.map(|b| b + strands),
    })
}

/// The ids in `selected` that are still fiber stacks in `inventory`, for
/// the request (the server refuses the lot if any id is gone).
pub fn deposit_ids(inventory: &Container, selected: &HashSet<Uuid>) -> Vec<Uuid> {
    fiber_stacks(inventory)
        .into_iter()
        .filter(|i| selected.contains(&i.item_id))
        .map(|i| i.item_id)
        .collect()
}

/// Where withdrawn fiber lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lands {
    /// On a fiber stack you hold. With one stack, which and its quality
    /// afterwards; with several, the server picks one.
    OnStack {
        slot: Option<i32>,
        quality_after: Option<i32>,
    },
    /// In this empty slot, as a stack of its own.
    NewSlot(usize),
}

/// A withdrawal, previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WithdrawQuote {
    /// Fiber units coming out.
    pub fiber: i64,
    pub fee: i64,
    /// Strands going in, the fee included.
    pub strands: i64,
    pub balance_after: Option<i64>,
    pub lands: Lands,
}

/// Why a withdrawal can't go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawRefusal {
    /// No quantity, or not a number.
    NoQuantity,
    /// More than a stack can hold.
    TooMany,
    /// It costs `strands` and you're `short` of them.
    Short { strands: i64, short: i64 },
    /// No fiber stack to join and no empty slot.
    NoRoom,
}

/// The most a stack can hold: the server's `items.quantity` is an `int4`.
const MAX_STACK: i64 = i32::MAX as i64;

/// The withdrawal of `quantity` fiber into `inventory`, holding `balance`
/// Strands (`None` until known; the server decides then).
pub fn withdraw(
    inventory: Option<&Container>,
    quantity: i64,
    balance: Option<i64>,
) -> Result<WithdrawQuote, WithdrawRefusal> {
    if quantity <= 0 {
        return Err(WithdrawRefusal::NoQuantity);
    }
    let stacks = inventory.map(fiber_stacks).unwrap_or_default();
    let held = stacks.iter().map(|s| s.quantity as i64).max().unwrap_or(0);
    if quantity > MAX_STACK - held {
        return Err(WithdrawRefusal::TooMany);
    }
    let (strands, fee) = withdraw_quote(quantity);
    if let Some(b) = balance {
        if b < strands {
            return Err(WithdrawRefusal::Short {
                strands,
                short: strands - b,
            });
        }
    }
    let lands = match stacks.as_slice() {
        [one] => Lands::OnStack {
            slot: Some(one.slot_index),
            quality_after: Some(merged_quality(one.quality, one.quantity, quantity)),
        },
        [_, ..] => Lands::OnStack {
            slot: None,
            quality_after: None,
        },
        [] => match inventory.and_then(Container::first_empty) {
            Some(slot) => Lands::NewSlot(slot),
            None if inventory.is_none() => Lands::OnStack {
                slot: None,
                quality_after: None,
            },
            None => return Err(WithdrawRefusal::NoRoom),
        },
    };
    Ok(WithdrawQuote {
        fiber: quantity,
        fee,
        strands,
        balance_after: balance.map(|b| b - strands),
        lands,
    })
}

/// A stack's quality after `added` quality-50 units join it: the server's
/// weighted mean, rounded down (`routes/exchange.rs`).
fn merged_quality(quality: i32, quantity: i32, added: i64) -> i32 {
    let total = quantity as i64 + added;
    ((quality as i64 * quantity as i64 + WITHDRAWN_QUALITY as i64 * added) / total) as i32
}

/// The most fiber `balance` Strands can withdraw.
pub fn max_withdraw(balance: i64) -> i64 {
    if balance <= 0 {
        return 0;
    }
    // ceil(200q / 199) <= b  <=>  q <= 199b / 200.
    let q = balance.saturating_mul(199) / 200;
    debug_assert!(withdraw_quote(q).0 <= balance);
    q.min(MAX_STACK)
}

/// A quote that was shown and confirmed, while the server answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    Deposit(DepositQuote),
    Withdraw(WithdrawQuote),
}

/// What the server did: the units moved, the fee, the Strands, and whether
/// that's what the panel showed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receipt {
    /// `fiber` in, `strands` out.
    Deposited {
        fiber: i64,
        fee: i64,
        strands: i64,
        as_quoted: bool,
    },
    /// `strands` in, `fiber` out.
    Withdrew {
        fiber: i64,
        fee: i64,
        strands: i64,
        as_quoted: bool,
    },
}

impl Receipt {
    /// The server's answer to a deposit quoted as `quote`.
    pub fn deposited(r: &DepositResponse, quote: Option<&DepositQuote>) -> Receipt {
        let (fiber, fee, strands) = (r.deposited_quantity as i64, r.fee, r.strands_received);
        Receipt::Deposited {
            fiber,
            fee,
            strands,
            as_quoted: quote.is_some_and(|q| (q.fiber, q.fee, q.strands) == (fiber, fee, strands)),
        }
    }

    /// The server's answer to a withdrawal quoted as `quote`.
    pub fn withdrew(r: &WithdrawResponse, quote: Option<&WithdrawQuote>) -> Receipt {
        let (fiber, fee, strands) = (r.fiber_received as i64, r.fee, r.strands_spent);
        Receipt::Withdrew {
            fiber,
            fee,
            strands,
            as_quoted: quote.is_some_and(|q| (q.fiber, q.fee, q.strands) == (fiber, fee, strands)),
        }
    }

    /// The server charged what the panel showed.
    pub fn as_quoted(&self) -> bool {
        match self {
            Receipt::Deposited { as_quoted, .. } | Receipt::Withdrew { as_quoted, .. } => {
                *as_quoted
            }
        }
    }
}

/// A quantity as typed: digits only, so "1,200" or "12 " read as numbers
/// and anything else as nothing.
pub fn parse_quantity(text: &str) -> Option<i64> {
    let digits: String = text
        .chars()
        .filter(|c| !matches!(c, ',' | ' ' | '_'))
        .collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(n: u128, item_type: &str, quality: i32, quantity: i32, slot: i32) -> SlotItem {
        SlotItem::new(
            Uuid::from_u128(n),
            item_type.into(),
            item_type.into(),
            quality,
            quantity,
            slot,
        )
    }

    fn inventory(slots: i32, items: Vec<SlotItem>) -> Container {
        Container::new(Uuid::from_u128(100), "player_inventory", slots, 6, &items)
    }

    #[test]
    fn deposit_sums_the_selected_fiber_and_takes_the_fee() {
        let inv = inventory(
            24,
            vec![
                item(1, "fiber", 60, 150, 3),
                item(2, "fiber", 40, 300, 0),
                item(3, "rope", 50, 9, 1),
            ],
        );
        let ids: Vec<_> = fiber_stacks(&inv).iter().map(|i| i.item_id).collect();
        assert_eq!(ids, [Uuid::from_u128(2), Uuid::from_u128(1)], "slot order");
        let all: HashSet<_> = [1, 2, 3].map(Uuid::from_u128).into();
        assert_eq!(
            deposit(&inv, &all, Some(10)),
            Ok(DepositQuote {
                fiber: 450,
                fee: 2,
                strands: 448,
                balance_after: Some(458),
            })
        );
        // Rope isn't fiber, and a gone id is left out of the request.
        let some: HashSet<_> = [1, 3, 99].map(Uuid::from_u128).into();
        assert_eq!(deposit(&inv, &some, None).unwrap().strands, 150);
        assert_eq!(deposit_ids(&inv, &some), [Uuid::from_u128(1)]);
        assert_eq!(
            deposit(&inv, &HashSet::new(), None),
            Err(DepositRefusal::NothingSelected)
        );
    }

    #[test]
    fn withdraw_costs_the_server_rule_and_says_where_it_lands() {
        let one = inventory(24, vec![item(1, "fiber", 70, 10, 4)]);
        assert_eq!(
            withdraw(Some(&one), 10, Some(100)),
            Ok(WithdrawQuote {
                fiber: 10,
                fee: 1,
                strands: 11,
                balance_after: Some(89),
                // (70 × 10 + 50 × 10) / 20
                lands: Lands::OnStack {
                    slot: Some(4),
                    quality_after: Some(60)
                },
            })
        );
        let none = inventory(24, vec![item(1, "rope", 70, 1, 0)]);
        assert_eq!(
            withdraw(Some(&none), 5, Some(100)).unwrap().lands,
            Lands::NewSlot(1)
        );
        let two = inventory(
            24,
            vec![item(1, "fiber", 70, 1, 0), item(2, "fiber", 30, 1, 1)],
        );
        assert_eq!(
            withdraw(Some(&two), 5, None).unwrap().lands,
            Lands::OnStack {
                slot: None,
                quality_after: None
            }
        );
    }

    #[test]
    fn withdraw_refuses_what_the_server_would() {
        let inv = inventory(24, vec![]);
        assert_eq!(
            withdraw(Some(&inv), 0, Some(5)),
            Err(WithdrawRefusal::NoQuantity)
        );
        assert_eq!(
            withdraw(Some(&inv), 100, Some(100)),
            Err(WithdrawRefusal::Short {
                strands: 101,
                short: 1
            })
        );
        // Unknown balance: the server decides.
        assert!(withdraw(Some(&inv), 100, None).is_ok());
        // Full, with no fiber to join: the server would keep the Strands.
        let full = inventory(2, vec![item(1, "rope", 1, 1, 0), item(2, "rope", 1, 1, 1)]);
        assert_eq!(
            withdraw(Some(&full), 1, Some(100)),
            Err(WithdrawRefusal::NoRoom)
        );
        let near = inventory(24, vec![item(1, "fiber", 50, i32::MAX - 1, 0)]);
        assert_eq!(
            withdraw(Some(&near), 2, None),
            Err(WithdrawRefusal::TooMany)
        );
    }

    #[test]
    fn max_withdraw_is_the_most_you_can_afford() {
        for b in 0..5000 {
            let q = max_withdraw(b);
            assert!(q == 0 || withdraw_quote(q).0 <= b, "{b}: {q} too dear");
            assert!(withdraw_quote(q + 1).0 > b, "{b}: {} affordable", q + 1);
        }
    }

    #[test]
    fn a_receipt_says_whether_the_server_charged_the_quote() {
        let quote = DepositQuote {
            fiber: 450,
            fee: 2,
            strands: 448,
            balance_after: None,
        };
        let answer = DepositResponse {
            deposited_quantity: 450,
            fee: 2,
            strands_received: 448,
            new_balance: 448,
        };
        assert!(Receipt::deposited(&answer, Some(&quote)).as_quoted());
        let other = DepositResponse { fee: 3, ..answer };
        assert!(!Receipt::deposited(&other, Some(&quote)).as_quoted());
        assert!(!Receipt::deposited(&answer, None).as_quoted());
        let inv = inventory(24, vec![]);
        let quote = withdraw(Some(&inv), 200, Some(500)).unwrap();
        let answer = WithdrawResponse {
            strands_spent: 202,
            fee: 2,
            fiber_received: 200,
            new_balance: 298,
        };
        assert!(Receipt::withdrew(&answer, Some(&quote)).as_quoted());
    }

    #[test]
    fn quantities_read_as_typed() {
        assert_eq!(parse_quantity("12"), Some(12));
        assert_eq!(parse_quantity(" 1,200 "), Some(1200));
        assert_eq!(parse_quantity(""), None);
        assert_eq!(parse_quantity("-3"), None);
        assert_eq!(parse_quantity("1e3"), None);
    }
}
