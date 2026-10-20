//! Plots (P6.4): what the plot panel offers for a plot. That depends on who
//! owns it, what its zone's deed costs and what you hold. Pure, so it's
//! tested without a server.
//!
//! **The deed cost is the plot's `deed_cost`.** The server fills it from the
//! zone's `zones.deed_cost`, which `POST /plots/{id}/claim` charges (#87).
//! Not `assessed_value`: that's 0 until the plot is first bought.

use capitol_protocol::rest::PlotInfo;
use uuid::Uuid;

/// What the plot panel can offer for a plot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// It's yours.
    Yours,
    /// Someone else holds it.
    Taken { owner: String },
    /// Unclaimed, and you can pay for it (or your balance isn't known yet,
    /// in which case the server decides).
    Claimable { cost: i64 },
    /// Unclaimed, but you're `short` of its `cost`.
    TooDear { cost: i64, short: i64 },
    /// Unclaimed, in a zone whose plots can't be bought.
    NotForSale,
}

impl Offer {
    /// For `plot`, seen by the player `me` holding `balance` Strands.
    pub fn new(plot: &PlotInfo, me: Option<Uuid>, balance: Option<i64>) -> Offer {
        if let Some(owner) = plot.owner_id {
            return if Some(owner) == me {
                Offer::Yours
            } else {
                Offer::Taken {
                    owner: plot.owner_name.clone().unwrap_or_else(|| "Unknown".into()),
                }
            };
        }
        let Some(cost) = plot.deed_cost else {
            return Offer::NotForSale;
        };
        let cost = cost as i64;
        match balance {
            Some(held) if held < cost => Offer::TooDear {
                cost,
                short: cost - held,
            },
            _ => Offer::Claimable { cost },
        }
    }

    /// Whether the Claim button does anything.
    pub fn can_claim(&self) -> bool {
        matches!(self, Offer::Claimable { .. })
    }
}

/// A plot's size or type as people say it: "medium" → "Medium",
/// "guild_hall" → "Guild hall".
pub fn words(raw: &str) -> String {
    let spaced = raw.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::Bounds;

    const ME: Uuid = Uuid::from_u128(1);
    const OTHER: Uuid = Uuid::from_u128(2);

    fn plot(deed_cost: Option<i32>, owner: Option<Uuid>) -> PlotInfo {
        PlotInfo {
            id: Uuid::from_u128(9),
            zone_id: "urban".into(),
            world_x: 0.0,
            world_y: 0.0,
            bounds: Bounds::new(0.0, 0.0, 10.0, 10.0),
            size_category: "medium".into(),
            plot_type: "standard".into(),
            owner_id: owner,
            owner_name: owner.map(|_| "Brin".into()),
            claimed_at: None,
            assessed_value: 0,
            tax_owed: 0,
            forecloses_at: None,
            deed_cost,
            station_count: 0,
            station_capacity: 9,
            terrain_id: None,
            corners: None,
            owner_citizen: false,
        }
    }

    #[test]
    fn an_owned_plot_is_yours_or_theirs() {
        assert_eq!(
            Offer::new(&plot(Some(1000), Some(ME)), Some(ME), Some(0)),
            Offer::Yours
        );
        assert_eq!(
            Offer::new(&plot(Some(1000), Some(OTHER)), Some(ME), Some(5000)),
            Offer::Taken {
                owner: "Brin".into()
            }
        );
    }

    #[test]
    fn an_unclaimed_plot_costs_its_deed() {
        let open = plot(Some(1000), None);
        let offer = Offer::new(&open, Some(ME), Some(1000));
        assert_eq!(offer, Offer::Claimable { cost: 1000 });
        assert!(offer.can_claim());
        assert_eq!(
            Offer::new(&open, Some(ME), Some(400)),
            Offer::TooDear {
                cost: 1000,
                short: 600
            }
        );
        // The server decides while the balance is unknown.
        assert!(Offer::new(&open, Some(ME), None).can_claim());
        // Its assessed value plays no part: that's 0 until it's bought.
        assert_eq!(open.assessed_value, 0);
        assert_eq!(
            Offer::new(&plot(None, None), Some(ME), Some(9999)),
            Offer::NotForSale
        );
    }

    #[test]
    fn words_are_for_people() {
        assert_eq!(words("medium"), "Medium");
        assert_eq!(words("guild_hall"), "Guild hall");
        assert_eq!(words(""), "");
    }
}
