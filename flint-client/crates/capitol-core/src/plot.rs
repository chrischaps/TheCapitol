//! Plots (P6.4): what the plot panel offers for a plot. That depends on who
//! owns it, what its zone's deed costs and what you hold. Pure, so it's
//! tested without a server.
//!
//! **The deed cost is the zone's.** The server charges `zones.deed_cost`
//! (`POST /plots/{id}/claim`), which `/bureaucracy/deed-prices` lists. An
//! unclaimed plot's `assessed_value` is 0 until it's bought. React shows
//! that value as the deed cost, so it reads "N/A" and never says you're
//! short. `PlotInfo` has no deed cost of its own: issue #87.

use capitol_protocol::rest::{DeedPrice, PlotInfo};
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
    /// Unclaimed, and the deed prices haven't arrived yet.
    Unpriced,
}

impl Offer {
    /// For `plot`, seen by the player `me` holding `balance` Strands.
    /// `prices` is `None` until `/bureaucracy/deed-prices` answers.
    pub fn new(
        plot: &PlotInfo,
        me: Option<Uuid>,
        prices: Option<&[DeedPrice]>,
        balance: Option<i64>,
    ) -> Offer {
        if let Some(owner) = plot.owner_id {
            return if Some(owner) == me {
                Offer::Yours
            } else {
                Offer::Taken {
                    owner: plot.owner_name.clone().unwrap_or_else(|| "Unknown".into()),
                }
            };
        }
        let Some(prices) = prices else {
            return Offer::Unpriced;
        };
        let Some(price) = prices.iter().find(|p| p.zone_id == plot.zone_id) else {
            return Offer::NotForSale;
        };
        let cost = price.deed_cost as i64;
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

    fn plot(zone: &str, owner: Option<Uuid>) -> PlotInfo {
        PlotInfo {
            id: Uuid::from_u128(9),
            zone_id: zone.into(),
            world_x: 0.0,
            world_y: 0.0,
            bounds: Bounds::new(0.0, 0.0, 10.0, 10.0),
            size_category: "medium".into(),
            plot_type: "standard".into(),
            owner_id: owner,
            owner_name: owner.map(|_| "Brin".into()),
            claimed_at: None,
            assessed_value: 0,
            station_count: 0,
            station_capacity: 9,
            terrain_id: None,
            corners: None,
        }
    }

    fn prices() -> Vec<DeedPrice> {
        vec![DeedPrice {
            zone_id: "urban".into(),
            zone_name: "Urban Zone".into(),
            deed_cost: 1000,
            plot_size: "Medium Plot".into(),
            station_capacity: 9,
        }]
    }

    #[test]
    fn an_owned_plot_is_yours_or_theirs() {
        let p = prices();
        assert_eq!(
            Offer::new(&plot("urban", Some(ME)), Some(ME), Some(&p), Some(0)),
            Offer::Yours
        );
        assert_eq!(
            Offer::new(&plot("urban", Some(OTHER)), Some(ME), Some(&p), Some(5000)),
            Offer::Taken {
                owner: "Brin".into()
            }
        );
    }

    #[test]
    fn an_unclaimed_plot_costs_its_zones_deed() {
        let p = prices();
        let open = plot("urban", None);
        let offer = Offer::new(&open, Some(ME), Some(&p), Some(1000));
        assert_eq!(offer, Offer::Claimable { cost: 1000 });
        assert!(offer.can_claim());
        assert_eq!(
            Offer::new(&open, Some(ME), Some(&p), Some(400)),
            Offer::TooDear {
                cost: 1000,
                short: 600
            }
        );
        // The server decides while the balance is unknown.
        assert!(Offer::new(&open, Some(ME), Some(&p), None).can_claim());
        assert_eq!(
            Offer::new(&open, Some(ME), None, Some(9999)),
            Offer::Unpriced
        );
        assert_eq!(
            Offer::new(&plot("wilderness", None), Some(ME), Some(&p), Some(9999)),
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
