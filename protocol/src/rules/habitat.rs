//! Habitats: the ground a wild resource grows on (W6, #199), as one rule the
//! server's node planner and the clients both read (#413).
//!
//! The bounds come from `backend/content/wilds.toml` and reach a client on
//! `/catalog` (`CatalogResponse::habitats`). The planner places a node only
//! where its habitat [`suits`](Habitat::suits) the ground and keeps to its
//! [massifs](Habitat::on_massif); the Flint client's F6 nodes layer asks the
//! same of the ground each node stands on, so a vein on the wrong ground
//! (the #347 class) shows at a glance.

use serde::{Deserialize, Serialize};

use crate::fmath::StableMath;
use crate::land::{self, LandParams};

/// Where in the wilds one resource grows. Every bound is optional, and each
/// is `[lo, hi]`, inclusive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Habitat {
    /// The land's height (`land::height_at`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<[f64; 2]>,
    /// Rise over run (`land::slope_at`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slope: Option<[f64; 2]>,
    /// How far past the rural edge
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<[f64; 2]>,
    /// Only on the even or the odd massifs (`land::massif_at`), so copper
    /// and tin keep to different ranges (#225).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub massifs: Option<Massifs>,
}

/// Which of the ranges' massifs a habitat keeps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Massifs {
    Even,
    Odd,
}

/// What a habitat makes of one spot's ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The ground suits it, massif and all.
    Suits,
    /// The ground's height, slope or depth is out of its bounds.
    WrongGround,
    /// The ground would do, but on a massif it doesn't keep to.
    WrongMassif,
}

impl Habitat {
    /// Whether the land `r` past the rural edge, at height `h` and slope
    /// `s`, suits it.
    pub fn suits(&self, r: f64, h: f64, s: f64) -> bool {
        let within = |b: Option<[f64; 2]>, v: f64| b.is_none_or(|[lo, hi]| (lo..=hi).contains(&v));
        within(self.depth, r) && within(self.height, h) && within(self.slope, s)
    }

    /// Whether massif `massif` (`land::massif_at`) is one it keeps to. With
    /// no massifs on the land, any is.
    pub fn on_massif(&self, massif: Option<u32>) -> bool {
        match (self.massifs, massif) {
            (Some(Massifs::Even), Some(m)) => m % 2 == 0,
            (Some(Massifs::Odd), Some(m)) => m % 2 == 1,
            _ => true,
        }
    }

    /// What it makes of the ground at (x, y) on `land`, reading the height,
    /// slope and massif there and the depth past `land`'s rural edge.
    pub fn verdict_at(&self, x: f64, y: f64, land: &LandParams) -> Verdict {
        let frame = &land.frame;
        let r = (x - frame.center.0).stable_hypot(y - frame.center.1) - frame.rural_edge();
        if !self.suits(r, land::height_at(x, y, land), land::slope_at(x, y, land)) {
            Verdict::WrongGround
        } else if self.massifs.is_some() && !self.on_massif(land::massif_at(x, y, land)) {
            Verdict::WrongMassif
        } else {
            Verdict::Suits
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vein() -> Habitat {
        Habitat { height: Some([200.0, 450.0]), slope: Some([0.45, 1.1]), massifs: Some(Massifs::Even), ..Default::default() }
    }

    #[test]
    fn each_bound_is_inclusive_and_a_missing_one_takes_anything() {
        let v = vein();
        assert!(v.suits(0.0, 200.0, 0.45) && v.suits(9999.0, 450.0, 1.1));
        assert!(!v.suits(0.0, 199.9, 0.8) && !v.suits(0.0, 300.0, 1.2));
        assert!(Habitat::default().suits(-5.0, 0.0, 3.0), "no bounds, any ground");
    }

    #[test]
    fn massifs_keep_to_their_parity_and_bare_land_takes_either() {
        let v = vein();
        assert!(v.on_massif(Some(4)) && !v.on_massif(Some(3)) && v.on_massif(None));
        let odd = Habitat { massifs: Some(Massifs::Odd), ..v };
        assert!(odd.on_massif(Some(3)) && !odd.on_massif(Some(4)));
    }

    #[test]
    fn the_plain_is_wrong_ground_for_a_vein() {
        let land = LandParams::default();
        let (cx, cy) = land.frame.center;
        assert_eq!(vein().verdict_at(cx, cy + 100.0, &land), Verdict::WrongGround);
        let anywhere = Habitat::default();
        assert_eq!(anywhere.verdict_at(cx, cy + 100.0, &land), Verdict::Suits);
    }

    #[test]
    fn it_reads_from_the_toml_shape() {
        let h: Habitat = serde_json::from_str(r#"{"height":[380,900],"slope":[0.55,1.15],"massifs":"odd"}"#).unwrap();
        assert_eq!(h.massifs, Some(Massifs::Odd));
        assert_eq!(serde_json::to_string(&h).unwrap(), r#"{"height":[380.0,900.0],"slope":[0.55,1.15],"massifs":"odd"}"#);
    }
}
