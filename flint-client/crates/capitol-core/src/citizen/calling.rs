//! A citizen's calling (Callings K0.2, `the-capitol-callings.md` D1, D3):
//! who it is, apart from what it's doing.
//!
//! A [`Calling`] is a favourite job (or none), how strongly it keeps to it
//! (`focus`), how it leans over the other jobs (`leanings`), and its taste
//! for detours (`wander`). The jobs are the eight [`ROLES`], one for each of
//! the plan's activities (D2): fiber, wood, ore, planks, ingots, tools,
//! building and trading (the merchant's). The calling is part of the
//! [`Persona`](super::persona::Persona), drawn **last** from the citizen's
//! stream, after its habit, so the names, temperaments, homes and habits
//! drawn before it don't change.
//!
//! Each citizen is dealt a [`Style`] by the town's [`Mix`] (`[callings] mix`
//! in `town.toml`, 40 / 40 / 20 unless it says):
//! - **specialist**: about 70% of its work time on its favourite, most of
//!   the rest on two or three jobs kin to it;
//! - **leaner**: about 45% on its favourite, the rest over all the others;
//! - **all-rounder**: no favourite, even over all eight, none above 25%.
//!
//! Everyone leans at least a little toward every job, and everyone has a
//! `wander`. The favourite is the citizen's birth trade, the roster's
//! `role`, unless it's an all-rounder.
//!
//! The stint question (K1.1, [`decide`](super::decide)) chases its
//! [`share`](Calling::share)s, and a model reads it in words. It travels on
//! a situation's `Who` and a decision log's `start` in the roster's form
//! ([`Wire`]). The `capitol-bot --sim` swarm has no persona, so its calling
//! is [`Calling::single`]: focus 1.0 on its `MIX` role, exactly today's one
//! job.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::persona::Rng;
use crate::sim::{Craft, Gather, Role, ROLES};

/// An all-rounder's favourite, as the roster and the log write it.
pub const NO_FAVOURITE: &str = "none";

/// How a citizen keeps to its favourite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Style {
    Specialist,
    Leaner,
    AllRounder,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::Specialist, Style::Leaner, Style::AllRounder];

    pub fn as_str(&self) -> &'static str {
        match self {
            Style::Specialist => "specialist",
            Style::Leaner => "leaner",
            Style::AllRounder => "all-rounder",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(name: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|s| s.as_str() == name)
    }
}

impl fmt::Display for Style {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The town's styles, as weights: what share of the citizens are dealt
/// each. They needn't sum to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mix {
    pub specialist: f64,
    pub leaner: f64,
    pub all_rounder: f64,
}

impl Default for Mix {
    /// Chris's mix (#526, D3): 40 / 40 / 20.
    fn default() -> Mix {
        Mix {
            specialist: 0.4,
            leaner: 0.4,
            all_rounder: 0.2,
        }
    }
}

impl Mix {
    /// A mix that deals `style` alone: `mint --calling`.
    pub fn only(style: Style) -> Mix {
        let w = |s: Style| if s == style { 1.0 } else { 0.0 };
        Mix {
            specialist: w(Style::Specialist),
            leaner: w(Style::Leaner),
            all_rounder: w(Style::AllRounder),
        }
    }

    pub fn weight(&self, style: Style) -> f64 {
        match style {
            Style::Specialist => self.specialist,
            Style::Leaner => self.leaner,
            Style::AllRounder => self.all_rounder,
        }
    }

    /// Checks no weight is negative and some are more than 0.
    pub fn check(&self) -> Result<(), String> {
        if Style::ALL.iter().any(|&s| !(self.weight(s) >= 0.0 && self.weight(s).is_finite())) {
            return Err("a style's weight in the mix is 0 or more".into());
        }
        if self.total() <= 0.0 {
            return Err("the mix needs a style with a weight more than 0".into());
        }
        Ok(())
    }

    fn total(&self) -> f64 {
        Style::ALL.iter().map(|&s| self.weight(s)).sum()
    }

    /// The style a draw `u` in [0, 1) falls on. A small change to the mix
    /// moves only the citizens whose draw is near a boundary.
    fn pick(&self, u: f64) -> Style {
        let mut at = u * self.total();
        for style in Style::ALL {
            at -= self.weight(style);
            if at < 0.0 {
                return style;
            }
        }
        Style::ALL.into_iter().rev().find(|&s| self.weight(s) > 0.0).unwrap_or(Style::Leaner)
    }
}

/// A specialist's focus at the least: a calling whose favourite has this
/// much of its time or more reads as a specialist's, less as a leaner's.
/// Between the two styles' draws ([`SPECIALIST_FOCUS`], [`LEANER_FOCUS`]).
pub const SPECIALIST_AT: f32 = 0.575;
/// The range a specialist's focus is drawn from: about 70%.
const SPECIALIST_FOCUS: (f64, f64) = (0.62, 0.78);
/// A leaner's: about 45%.
const LEANER_FOCUS: (f64, f64) = (0.38, 0.52);
/// The least a citizen leans toward any job, as the roster writes it.
const LEAST: f64 = 0.01;

/// Who a citizen is, as work goes: what it would rather be doing, and how
/// much.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "Wire", try_from = "Wire")]
pub struct Calling {
    /// Its favourite job: its birth trade, or none for an all-rounder.
    pub favourite: Option<Role>,
    /// The favourite's share of its work time, 0 to 1: 0 without one.
    pub focus: f32,
    /// Its lean toward each other job, as shares of its work time, in
    /// [`ROLES`] order. Never the favourite. A job left out has no lean.
    pub leanings: Vec<(Role, f32)>,
    /// Its taste for detours, 0 to 1.
    pub wander: f32,
}

impl Calling {
    /// One job and nothing else: the swarm's, focus 1.0 on its `MIX` role,
    /// with no detours. It's what every bot was before callings.
    pub fn single(role: Role) -> Calling {
        Calling {
            favourite: Some(role),
            focus: 1.0,
            leanings: Vec::new(),
            wander: 0.0,
        }
    }

    /// A calling drawn from the citizen's stream, its style by `mix`, its
    /// favourite its birth trade `role` unless it's an all-rounder.
    pub(crate) fn seeded(rng: &mut Rng, mix: &Mix, role: Role) -> Calling {
        let style = mix.pick(rng.unit());
        // As a temperament's traits: most middling, a few not
        let wander = hundredth((rng.unit() + rng.unit()) / 2.0);
        let others: Vec<Role> = ROLES.into_iter().filter(|&r| r != role).collect();
        let (favourite, focus, weights) = match style {
            Style::AllRounder => {
                // Even over all eight: weights of 0.7 to 1 keep any one
                // under 1 / 5.9, 17%
                let weights: Vec<(Role, f64)> = ROLES.into_iter().map(|r| (r, rng.between(0.7, 1.0))).collect();
                (None, 0.0, spread(&weights, 1.0))
            }
            Style::Specialist => {
                let focus = hundredth(rng.between(SPECIALIST_FOCUS.0, SPECIALIST_FOCUS.1));
                // Two or three of the jobs kin to its favourite take most
                // of the rest; the others a little each
                let mut kin = kin(role).to_vec();
                if rng.unit() < 0.5 {
                    kin.remove((rng.unit() * kin.len() as f64) as usize % kin.len());
                }
                let rest = 1.0 - f64::from(focus);
                let to_kin = rest * rng.between(0.75, 0.85);
                let (mut near, mut far) = (Vec::new(), Vec::new());
                for &r in &others {
                    if kin.contains(&r) {
                        near.push((r, rng.between(0.75, 1.0)));
                    } else {
                        far.push((r, rng.between(0.6, 1.0)));
                    }
                }
                let mut leanings = spread(&near, to_kin);
                leanings.extend(spread(&far, rest - to_kin));
                (Some(role), focus, leanings)
            }
            Style::Leaner => {
                let focus = hundredth(rng.between(LEANER_FOCUS.0, LEANER_FOCUS.1));
                // Over all the rest, its kin a little more
                let weights: Vec<(Role, f64)> = others
                    .iter()
                    .map(|&r| (r, rng.between(0.4, 1.0) * if kin(role).contains(&r) { 1.6 } else { 1.0 }))
                    .collect();
                (Some(role), focus, spread(&weights, 1.0 - f64::from(focus)))
            }
        };
        let mut leanings = weights;
        leanings.sort_by_key(|(r, _)| ROLES.iter().position(|o| o == r));
        Calling {
            favourite,
            focus,
            leanings,
            wander,
        }
    }

    /// The style it reads as: an all-rounder without a favourite, a
    /// specialist from a focus of [`SPECIALIST_AT`], a leaner below it.
    pub fn style(&self) -> Style {
        match self.favourite {
            None => Style::AllRounder,
            Some(_) if self.focus >= SPECIALIST_AT => Style::Specialist,
            Some(_) => Style::Leaner,
        }
    }

    /// The share of its work time it would give `job`, of the whole: its
    /// focus for its favourite, its lean for the rest, scaled so they sum
    /// to 1 (a roster's are written to two places, and may be changed by
    /// hand).
    pub fn share(&self, job: Role) -> f32 {
        let total = self.total();
        if total <= 0.0 {
            return 0.0;
        }
        let own = if self.favourite == Some(job) {
            self.focus
        } else {
            self.leanings.iter().find(|(r, _)| *r == job).map_or(0.0, |(_, w)| *w)
        };
        own / total
    }

    fn total(&self) -> f32 {
        self.focus * f32::from(u8::from(self.favourite.is_some())) + self.leanings.iter().map(|(_, w)| w).sum::<f32>()
    }

    /// Checks it's a calling `role`'s citizen can have: its favourite its
    /// role, or none; each share 0 to 1; no job twice; some time for
    /// something.
    pub fn check(&self, role: Role) -> Result<(), String> {
        let unit = |v: f32| (0.0..=1.0).contains(&v);
        match self.favourite {
            Some(favourite) if favourite != role => {
                return Err(format!(
                    "favours {}, but its role is {}; a calling's favourite is its role, its birth trade, or \"none\" for an all-rounder",
                    favourite.slug(),
                    role.slug()
                ))
            }
            Some(_) if !(self.focus > 0.0 && self.focus <= 1.0) => {
                return Err(format!("has a focus of {}; with a favourite it's more than 0, up to 1", self.focus))
            }
            None if self.focus != 0.0 => {
                return Err(format!("has a focus of {} and no favourite; an all-rounder's focus is 0", self.focus))
            }
            _ => {}
        }
        if !unit(self.wander) {
            return Err(format!("has a wander of {}; it runs 0 to 1", self.wander));
        }
        for (i, (job, lean)) in self.leanings.iter().enumerate() {
            if Some(*job) == self.favourite {
                return Err(format!("leans toward {} besides favouring it; its focus says how much", job.slug()));
            }
            if self.leanings[..i].iter().any(|(r, _)| r == job) {
                return Err(format!("leans toward {} twice", job.slug()));
            }
            if !unit(*lean) {
                return Err(format!("leans {lean} toward {}; a lean runs 0 to 1", job.slug()));
            }
        }
        if self.total() <= 0.0 {
            return Err("gives no time to anything; a calling needs a focus or a lean more than 0".into());
        }
        Ok(())
    }
}

impl fmt::Display for Calling {
    /// `specialist in gatherer (fiber), 71%`, or `all-rounder`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.favourite {
            None => write!(f, "{}", self.style()),
            Some(job) => write!(f, "{} in {job}, {:.0}%", self.style(), 100.0 * self.share(job)),
        }
    }
}

/// A calling as the roster writes it, and so a decision log (K1.1):
/// `{"favourite": "builder", "focus": 0.71, "wander": 0.4, "leanings":
/// {"crafter-planks": 0.12, ...}}`, the favourite [`NO_FAVOURITE`] for an
/// all-rounder. Read back, the leanings go in [`ROLES`] order, as a
/// roster's and a seeded calling's are, so a replay's shares are the same
/// to the bit.
#[derive(Serialize, Deserialize)]
struct Wire {
    favourite: String,
    focus: f32,
    wander: f32,
    leanings: BTreeMap<String, f32>,
}

impl From<Calling> for Wire {
    fn from(c: Calling) -> Wire {
        Wire {
            favourite: c.favourite.map_or(NO_FAVOURITE, |r| r.slug()).to_string(),
            focus: c.focus,
            wander: c.wander,
            leanings: c.leanings.iter().map(|(job, w)| (job.slug().to_string(), *w)).collect(),
        }
    }
}

impl TryFrom<Wire> for Calling {
    type Error = String;

    fn try_from(w: Wire) -> Result<Calling, String> {
        let job = |slug: &str| Role::from_slug(slug).ok_or_else(|| format!("{slug:?} is no job"));
        let favourite = match w.favourite.as_str() {
            NO_FAVOURITE => None,
            slug => Some(job(slug)?),
        };
        let mut leanings = w.leanings.iter().map(|(slug, lean)| Ok((job(slug)?, *lean))).collect::<Result<Vec<_>, String>>()?;
        leanings.sort_by_key(|(r, _)| ROLES.iter().position(|o| o == r));
        Ok(Calling {
            favourite,
            focus: w.focus,
            leanings,
            wander: w.wander,
        })
    }
}

/// Doing `job`, in words, as a card's line has it
/// ([`lately`](capitol_protocol::rules::lately)): `cutting grass`, `felling
/// trees`.
pub fn doing(job: Role) -> &'static str {
    match job {
        Role::Gatherer(Gather::Fiber) => "cutting grass",
        Role::Gatherer(Gather::Wood) => "felling trees",
        Role::Gatherer(Gather::Ore) => "mining ore",
        Role::Crafter(Craft::Planks) => "sawing planks",
        Role::Crafter(Craft::Ingots) => "smelting metal",
        Role::Crafter(Craft::Tools) => "making tools",
        Role::Builder => "building",
        Role::Merchant => "trading",
    }
}

/// [`doing`] as a verb: `cut grass`, `fell trees`.
pub fn to_do(job: Role) -> &'static str {
    match job {
        Role::Gatherer(Gather::Fiber) => "cut grass",
        Role::Gatherer(Gather::Wood) => "fell trees",
        Role::Gatherer(Gather::Ore) => "mine ore",
        Role::Crafter(Craft::Planks) => "saw planks",
        Role::Crafter(Craft::Ingots) => "smelt metal",
        Role::Crafter(Craft::Tools) => "make tools",
        Role::Builder => "build",
        Role::Merchant => "trade",
    }
}

/// The jobs kin to `role`'s: the ones it would be good for, either side of
/// it in the chain, or that use what it has. A specialist takes up two or
/// three of them.
fn kin(role: Role) -> [Role; 3] {
    use Role::{Builder, Crafter, Gatherer, Merchant};
    match role {
        // It has the Strands, and other ground to gather from
        Gatherer(Gather::Fiber) => [Gatherer(Gather::Wood), Gatherer(Gather::Ore), Merchant],
        Gatherer(Gather::Wood) => [Crafter(Craft::Planks), Gatherer(Gather::Fiber), Builder],
        Gatherer(Gather::Ore) => [Crafter(Craft::Ingots), Gatherer(Gather::Wood), Crafter(Craft::Tools)],
        Crafter(Craft::Planks) => [Gatherer(Gather::Wood), Builder, Crafter(Craft::Tools)],
        Crafter(Craft::Ingots) => [Gatherer(Gather::Ore), Crafter(Craft::Tools), Builder],
        Crafter(Craft::Tools) => [Crafter(Craft::Ingots), Crafter(Craft::Planks), Gatherer(Gather::Ore)],
        Builder => [Crafter(Craft::Planks), Crafter(Craft::Ingots), Gatherer(Gather::Wood)],
        // It buys and sells what the fiber, the sawbench and the forge make
        Merchant => [Gatherer(Gather::Fiber), Crafter(Craft::Planks), Crafter(Craft::Ingots)],
    }
}

/// `total` shared out over `weights`, each share to two places and at
/// least [`LEAST`].
fn spread(weights: &[(Role, f64)], total: f64) -> Vec<(Role, f32)> {
    let sum: f64 = weights.iter().map(|(_, w)| w).sum();
    weights
        .iter()
        .map(|&(r, w)| (r, hundredth((total * w / sum).max(LEAST))))
        .collect()
}

/// As the roster writes it: two places.
fn hundredth(v: f64) -> f32 {
    ((v * 100.0).round() / 100.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn town(n: u64, mix: &Mix) -> Vec<(Role, Calling)> {
        Role::assign(n as usize)
            .into_iter()
            .enumerate()
            .map(|(i, role)| (role, Calling::seeded(&mut Rng::new((i as u64).wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 91), mix, role)))
            .collect()
    }

    #[test]
    fn each_style_keeps_to_its_favourite_as_its_people_do() {
        for (role, calling) in town(600, &Mix::default()) {
            assert_eq!(calling.check(role), Ok(()), "{calling:?}");
            // Everyone leans at least a little toward every job, and wanders
            for job in ROLES {
                assert!(calling.share(job) > 0.0, "{calling:?} gives {job} nothing");
            }
            let total: f32 = ROLES.iter().map(|&j| calling.share(j)).sum();
            assert!((total - 1.0).abs() < 1e-4);
            assert!((0.0..=1.0).contains(&calling.wander));
            let top_other = ROLES
                .iter()
                .filter(|&&j| Some(j) != calling.favourite)
                .map(|&j| calling.share(j))
                .fold(0.0, f32::max);
            match calling.style() {
                Style::Specialist => {
                    assert_eq!(calling.favourite, Some(role));
                    let f = calling.share(role);
                    assert!((0.58..=0.82).contains(&f), "a specialist's {f}");
                    // Two or three jobs take most of the rest
                    let kin: f32 = kin(role).iter().map(|&j| calling.share(j)).sum();
                    assert!(kin >= 0.7 * (1.0 - f), "{calling:?}");
                    let real = calling.leanings.iter().filter(|(_, w)| *w >= 0.04).count();
                    assert!((2..=3).contains(&real), "{real} real leanings: {calling:?}");
                }
                Style::Leaner => {
                    assert_eq!(calling.favourite, Some(role));
                    let f = calling.share(role);
                    assert!((0.36..=0.54).contains(&f), "a leaner's {f}");
                    // Spread over most of the rest
                    let real = calling.leanings.iter().filter(|(_, w)| *w >= 0.04).count();
                    assert!(real >= 4, "{real} real leanings: {calling:?}");
                    assert!(top_other < f);
                }
                Style::AllRounder => {
                    assert_eq!(calling.favourite, None);
                    assert!(top_other <= 0.25, "{calling:?}");
                    assert!(ROLES.iter().all(|&j| calling.share(j) >= 0.08), "{calling:?}");
                }
            }
        }
    }

    #[test]
    fn a_mix_deals_its_styles_and_a_change_moves_few() {
        let mix = Mix::default();
        let callings = town(2000, &mix);
        let share = |s: Style| callings.iter().filter(|(_, c)| c.style() == s).count() as f64 / 2000.0;
        assert!((0.37..0.43).contains(&share(Style::Specialist)), "{}", share(Style::Specialist));
        assert!((0.37..0.43).contains(&share(Style::Leaner)));
        assert!((0.17..0.23).contains(&share(Style::AllRounder)));
        // A tweak of the mix re-deals only those near a boundary
        let more = Mix { specialist: 0.42, leaner: 0.38, ..mix };
        let moved = callings.iter().zip(town(2000, &more)).filter(|(a, b)| a.1.style() != b.1.style()).count();
        assert!(moved < 60, "{moved} moved");
        // One style alone deals only that
        for style in Style::ALL {
            assert!(town(50, &Mix::only(style)).iter().all(|(_, c)| c.style() == style));
            assert_eq!(Style::from_str(style.as_str()), Some(style));
        }
        assert_eq!(Mix { leaner: -1.0, ..mix }.check().unwrap_err(), "a style's weight in the mix is 0 or more");
        assert!(Mix { specialist: 0.0, leaner: 0.0, all_rounder: 0.0 }.check().is_err());
    }

    #[test]
    fn the_swarms_calling_is_its_one_job() {
        for role in ROLES {
            let one = Calling::single(role);
            assert_eq!(one.check(role), Ok(()));
            assert_eq!(one.share(role), 1.0);
            assert!(ROLES.iter().filter(|&&j| j != role).all(|&j| one.share(j) == 0.0));
            assert_eq!(one.style(), Style::Specialist);
            assert_eq!(one.wander, 0.0);
        }
        assert_eq!(Calling::single(Role::Builder).to_string(), "specialist in builder, 100%");
    }

    #[test]
    fn a_calling_goes_through_json_to_the_bit() {
        for (_, calling) in town(200, &Mix::default()) {
            let json = serde_json::to_string(&calling).unwrap();
            let back: Calling = serde_json::from_str(&json).unwrap();
            assert_eq!(back, calling, "{json}");
            for job in ROLES {
                assert_eq!(back.share(job).to_bits(), calling.share(job).to_bits(), "{json}");
            }
        }
        let json = serde_json::to_string(&Calling::single(Role::Builder)).unwrap();
        assert_eq!(json, r#"{"favourite":"builder","focus":1.0,"wander":0.0,"leanings":{}}"#);
        let all = r#"{"favourite":"none","focus":0.0,"wander":0.3,"leanings":{"merchant":0.5,"builder":0.5}}"#;
        let all: Calling = serde_json::from_str(all).unwrap();
        assert_eq!(all.leanings, [(Role::Builder, 0.5), (Role::Merchant, 0.5)], "in ROLES order");
        assert_eq!(all.style(), Style::AllRounder);
        assert!(serde_json::from_str::<Calling>(r#"{"favourite":"juggler","focus":1.0,"wander":0.0,"leanings":{}}"#).is_err());
        assert_eq!(serde_json::to_string(&Style::AllRounder).unwrap(), r#""all-rounder""#);
    }

    #[test]
    fn a_bad_calling_says_what_is_wrong() {
        let role = Role::Builder;
        let ok = Calling::seeded(&mut Rng::new(5), &Mix::only(Style::Leaner), role);
        let err = |c: Calling| c.check(role).unwrap_err();
        assert!(err(Calling { favourite: Some(Role::Merchant), ..ok.clone() }).contains("its role is builder"));
        assert!(err(Calling { focus: 0.0, ..ok.clone() }).contains("more than 0"));
        assert!(err(Calling { favourite: None, ..ok.clone() }).contains("all-rounder's focus is 0"));
        assert!(err(Calling { wander: 1.5, ..ok.clone() }).contains("wander"));
        let mut twice = ok.clone();
        twice.leanings.push(twice.leanings[0]);
        assert!(err(twice).contains("twice"));
        let mut own = ok.clone();
        own.leanings.push((role, 0.1));
        assert!(err(own).contains("besides favouring"));
        let none = Calling { favourite: None, focus: 0.0, leanings: vec![], wander: 0.2 };
        assert!(err(none).contains("no time"));
        assert!(Calling::seeded(&mut Rng::new(5), &Mix::only(Style::AllRounder), role).to_string() == "all-rounder");
    }
}
