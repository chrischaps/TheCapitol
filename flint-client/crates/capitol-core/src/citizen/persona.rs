//! Who a citizen is (D5, C1.2):
//!
//! - **a name**: a given name a grandparent might have had, and a surname
//!   that says what the citizen does, as the old trade names did;
//! - **a temperament**: `thrift`, `risk`, `sociability` and `patience`,
//!   each 0 to 1. Thrift says how much a citizen keeps back before it buys
//!   a home, and sociability how long it stays at the market and whether it
//!   keeps the bazaar. The rules layer (C1.3) weighs all four, and they
//!   colour a model layer's prompt;
//! - **a home**: a lot it claims once it can afford one, and until then a
//!   spot on an avenue in its zone, where it goes at dusk;
//! - **a market**: the hall or the bazaar, where it spends its market hours;
//! - **a [`Schedule`]**: its day on the server's clock;
//! - **a [`Habit`]**: when it plays, on the real clock (#276).
//!
//! All of it is drawn from the citizen's own stream, seeded by its name, so
//! a roster that leaves any of it out reads back the persona `mint` wrote.

use std::f64::consts::FRAC_PI_4;
use std::fmt;

use capitol_protocol::rules::Zone;

use super::habit::{Habit, Mix};
use super::schedule::Schedule;
use crate::sim::{Craft, Gather, Role};

/// Who a citizen is, beyond its name and its trade.
#[derive(Debug, Clone, PartialEq)]
pub struct Persona {
    pub temperament: Temperament,
    pub home: Home,
    pub market: Market,
    pub schedule: Schedule,
    pub habit: Habit,
}

impl Persona {
    /// The persona a citizen of `role` with the stream `seed` is born with,
    /// in a town of the default [`Mix`] of habits. The same seed always
    /// gives the same persona.
    pub fn seeded(role: Role, seed: u64) -> Persona {
        Persona::seeded_in(role, seed, &Mix::default())
    }

    /// As [`seeded`](Persona::seeded), its habit's kind drawn from `mix`.
    pub fn seeded_in(role: Role, seed: u64, mix: &Mix) -> Persona {
        let mut rng = Rng::new(seed);
        let temperament = Temperament::seeded(&mut rng);
        let home = Home::seeded(&mut rng, role);
        let market = Market::seeded(&mut rng, &temperament);
        let schedule = Schedule::seeded(&mut rng, &temperament);
        // Drawn last, so the persona C1.2 drew is the same
        let habit = Habit::seeded(&mut rng, mix, seed);
        Persona {
            temperament,
            home,
            market,
            schedule,
            habit,
        }
    }
}

/// The decision seam's (C1.3); the persona seeds each citizen's own.
pub use super::decide::Temperament;

impl Temperament {
    pub(crate) fn seeded(rng: &mut Rng) -> Temperament {
        // Two draws averaged: most citizens are middling, a few aren't
        let mut draw = || to_hundredth((rng.unit() + rng.unit()) / 2.0) as f32;
        Temperament {
            thrift: draw(),
            risk: draw(),
            sociability: draw(),
            patience: draw(),
        }
    }

    /// Checks each trait is 0 to 1.
    pub fn check(&self) -> Result<(), String> {
        let traits = [
            ("thrift", self.thrift),
            ("risk", self.risk),
            ("sociability", self.sociability),
            ("patience", self.patience),
        ];
        match traits.iter().find(|(_, v)| !(0.0..=1.0).contains(v)) {
            Some((name, v)) => Err(format!("its {name} is {v}; a temperament's traits run 0 to 1")),
            None => Ok(()),
        }
    }

    /// The Strands a citizen wants in hand before it pays `deed` for a
    /// home: the deed alone for the least thrifty, three deeds' worth for
    /// the most.
    pub fn home_fund(&self, deed: i64) -> i64 {
        deed + (deed as f64 * 2.0 * f64::from(self.thrift)).round() as i64
    }
}

/// Where a citizen lives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Home {
    /// The zone its lot will be in.
    pub zone: Zone,
    /// The avenue it lives off, and walks home along.
    pub avenue: Avenue,
    /// How far out along the avenue, through the zone: 0 its inner edge,
    /// 1 its outer.
    pub depth: f64,
}

/// The zones citizens live in: those with lots the sim's builders buy.
pub const HOME_ZONES: [Zone; 2] = [Zone::Rural, Zone::Suburban];

impl Home {
    fn seeded(rng: &mut Rng, role: Role) -> Home {
        // Gatherers live out by the land they work; crafters and merchants
        // nearer town
        let rural = match role {
            Role::Gatherer(_) => 0.7,
            Role::Crafter(_) | Role::Merchant => 0.3,
            Role::Builder => 0.5,
        };
        let zone = if rng.unit() < rural { Zone::Rural } else { Zone::Suburban };
        Home {
            zone,
            avenue: Avenue((rng.unit() * 8.0) as u8 % 8),
            depth: to_hundredth(rng.between(0.15, 0.85)),
        }
    }

    pub fn check(&self) -> Result<(), String> {
        if !HOME_ZONES.contains(&self.zone) {
            return Err(format!("its home is in {}; a home is rural or suburban", self.zone));
        }
        if !(0.0..=1.0).contains(&self.depth) {
            return Err(format!("its home's depth is {}; it runs 0 to 1", self.depth));
        }
        Ok(())
    }

    /// The spot it goes home to until it has a lot: on its avenue's crown,
    /// `depth` of the way out through its zone.
    pub fn spot(&self) -> (f64, f64) {
        let (inner, outer) = self.zone.radii();
        let r = inner + (outer - inner) * self.depth;
        let a = self.avenue.angle();
        (Zone::CENTER_X + r * a.cos(), Zone::CENTER_Y + r * a.sin())
    }
}

/// One of the eight arterials, every 45° round the city, by its compass
/// point. East is +x and south +y, as the spawn on the south arterial has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Avenue(pub u8);

impl Avenue {
    const NAMES: [&'static str; 8] = [
        "east",
        "south-east",
        "south",
        "south-west",
        "west",
        "north-west",
        "north",
        "north-east",
    ];

    /// Its bearing from the centre, in radians from +x toward +y.
    pub fn angle(&self) -> f64 {
        self.0 as f64 * FRAC_PI_4
    }

    pub fn as_str(&self) -> &'static str {
        Avenue::NAMES[self.0 as usize % 8]
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(name: &str) -> Option<Avenue> {
        Avenue::NAMES.iter().position(|n| *n == name).map(|i| Avenue(i as u8))
    }
}

impl fmt::Display for Avenue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a citizen spends its market hours. Either way its orders go on
/// the book at the hall (D6), which faces the bazaar across the avenue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    Hall,
    Bazaar,
}

impl Market {
    fn seeded(rng: &mut Rng, temperament: &Temperament) -> Market {
        // The bazaar's the livelier: one in ten of the shyest keep it, half
        // the most sociable
        if rng.unit() < 0.1 + 0.4 * f64::from(temperament.sociability) {
            Market::Bazaar
        } else {
            Market::Hall
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Market::Hall => "hall",
            Market::Bazaar => "bazaar",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(name: &str) -> Option<Market> {
        [Market::Hall, Market::Bazaar].into_iter().find(|m| m.as_str() == name)
    }
}

impl fmt::Display for Market {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A citizen's own random stream: SplitMix64.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// Uniform in [0, 1).
    pub(crate) fn unit(&mut self) -> f64 {
        let x = mix(self.0);
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [lo, hi).
    pub(crate) fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

/// As the roster writes it: two places.
fn to_hundredth(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Given names, a mix of the ordinary ones of the middle of the last century.
const GIVEN: [&str; 48] = [
    "Ada", "Agnes", "Albert", "Alice", "Arthur", "Beatrice", "Bernard", "Cecil", "Clara", "Clement",
    "Doris", "Dorothy", "Edith", "Edwin", "Elsie", "Ernest", "Florence", "Frank", "Gilbert", "Grace",
    "Harold", "Hazel", "Herbert", "Hilda", "Ida", "Iris", "Irene", "Ivor", "Jean", "Leonard",
    "Lillian", "Lionel", "Mabel", "Maud", "Mildred", "Nora", "Norman", "Olive", "Percy", "Phyllis",
    "Reginald", "Rose", "Ruth", "Sidney", "Stanley", "Vera", "Walter", "Winifred",
];

/// Trade surnames for each role: what the work would have called you.
fn surnames(role: Role) -> &'static [&'static str] {
    match role {
        Role::Gatherer(Gather::Fiber) => &["Reeder", "Roper", "Fielding", "Hayward", "Rushton", "Flaxman"],
        Role::Gatherer(Gather::Wood) => &["Woodward", "Forester", "Hewitt", "Lumber", "Ashby", "Underwood"],
        Role::Gatherer(Gather::Ore) => &["Collier", "Miner", "Delve", "Stoner", "Quarry", "Leadbetter"],
        Role::Crafter(Craft::Planks) => &["Sawyer", "Joiner", "Carpenter", "Turner", "Boardman", "Plank"],
        Role::Crafter(Craft::Ingots) => &["Smith", "Ironside", "Brazier", "Furness", "Bloom", "Ironmonger"],
        Role::Crafter(Craft::Tools) => &["Wright", "Cutler", "Toller", "Hammond", "Arkwright", "Wainwright"],
        Role::Builder => &["Mason", "Tyler", "Slater", "Bricker", "Thatcher", "Builder"],
        Role::Merchant => &["Chapman", "Mercer", "Marchant", "Packman", "Spicer", "Draper"],
    }
}

/// The trade a name's surname names, if it's one of the trade names: how
/// one citizen tells what another makes, as anyone in the town could, from
/// the name over its head (#295). Each surname belongs to one trade.
pub fn trade_of(name: &str) -> Option<Role> {
    let surname = name.rsplit(' ').next()?;
    crate::sim::ROLES.into_iter().find(|role| surnames(*role).contains(&surname))
}

/// The `n`th name for a citizen of `role`. Given names step through the
/// list a stride at a time, so any 48 citizens in a row have 48 different
/// ones; the surname is a hashed pick among the role's. A mint that finds a
/// name taken asks for another `n`. The same `n` and role always give the
/// same name.
pub fn name(role: Role, n: u64) -> String {
    let surnames = surnames(role);
    let given = GIVEN[((n * GIVEN_STRIDE) % GIVEN.len() as u64) as usize];
    let h = mix(n ^ role.slug().bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64)));
    let surname = surnames[(h % surnames.len() as u64) as usize];
    format!("{given} {surname}")
}

/// Coprime to 48, and far enough round the list that neighbours don't
/// share an initial.
const GIVEN_STRIDE: u64 = 29;

/// SplitMix64's finaliser: spreads consecutive `n` over the whole range.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::ROLES;
    use std::collections::HashSet;

    #[test]
    fn a_persona_is_the_same_from_the_same_seed_and_differs_from_another() {
        let ada = Persona::seeded(Role::Builder, 7);
        assert_eq!(ada, Persona::seeded(Role::Builder, 7));
        assert_ne!(ada, Persona::seeded(Role::Builder, 8));
        assert_eq!(ada.temperament.check(), Ok(()));
        assert_eq!(ada.home.check(), Ok(()));
        assert_eq!(ada.schedule.check(), Ok(()));
        assert_eq!(ada.habit.check(), Ok(()));
    }

    #[test]
    fn a_town_of_personas_is_spread_and_mostly_middling() {
        let town: Vec<(Role, Persona)> = Role::assign(400)
            .into_iter()
            .enumerate()
            .map(|(n, role)| (role, Persona::seeded(role, n as u64 * 7919)))
            .collect();
        let share = |f: &dyn Fn(&(Role, Persona)) -> bool| town.iter().filter(|p| f(p)).count() as f64 / 400.0;
        for a in 0..8 {
            assert!(share(&|(_, p)| p.home.avenue == Avenue(a)) > 0.06, "every avenue has its people: {a}");
        }
        // Gatherers mostly live out in the country, crafters mostly nearer in
        let rural = |want: fn(&Role) -> bool| {
            let of: Vec<_> = town.iter().filter(|(r, _)| want(r)).collect();
            of.iter().filter(|(_, p)| p.home.zone == Zone::Rural).count() as f64 / of.len() as f64
        };
        assert!(rural(|r| matches!(r, Role::Gatherer(_))) > 0.55);
        assert!(rural(|r| matches!(r, Role::Crafter(_))) < 0.45);
        let bazaar = share(&|(_, p)| p.market == Market::Bazaar);
        assert!((0.15..0.5).contains(&bazaar), "some keep the bazaar, most the hall: {bazaar}");
        // Traits lean to the middle but reach the ends
        let thrift: Vec<f32> = town.iter().map(|(_, p)| p.temperament.thrift).collect();
        let middling = thrift.iter().filter(|t| (0.25..=0.75).contains(*t)).count();
        assert!(middling > 260, "{middling} of 400 middling");
        assert!(thrift.iter().any(|t| *t < 0.15) && thrift.iter().any(|t| *t > 0.85));
    }

    #[test]
    fn a_home_spot_is_on_its_avenue_within_its_zone() {
        let home = Home {
            zone: Zone::Rural,
            avenue: Avenue::from_str("south").unwrap(),
            depth: 0.5,
        };
        let (x, y) = home.spot();
        let (inner, outer) = Zone::Rural.radii();
        assert!((x - Zone::CENTER_X).abs() < 1e-6, "on the south arterial");
        assert!((y - Zone::CENTER_Y - (inner + outer) / 2.0).abs() < 1e-6);
        for a in 0..8 {
            let avenue = Avenue(a);
            assert_eq!(Avenue::from_str(avenue.as_str()), Some(avenue));
        }
    }

    #[test]
    fn a_thrifty_citizen_saves_longer_for_its_home() {
        let mut t = Temperament {
            thrift: 0.0,
            risk: 0.5,
            sociability: 0.5,
            patience: 0.5,
        };
        assert_eq!(t.home_fund(250), 250);
        t.thrift = 1.0;
        assert_eq!(t.home_fund(250), 750);
        t.thrift = 1.2;
        assert!(t.check().unwrap_err().contains("thrift"));
    }

    #[test]
    fn a_name_is_a_given_name_and_a_trade_name_and_always_the_same() {
        let ada = name(Role::Builder, 7);
        assert_eq!(ada, name(Role::Builder, 7));
        let (given, surname) = ada.split_once(' ').unwrap();
        assert!(GIVEN.contains(&given));
        assert!(surnames(Role::Builder).contains(&surname));
    }

    #[test]
    fn a_trade_name_says_the_trade_and_belongs_to_one() {
        for role in ROLES {
            for n in 0..40 {
                assert_eq!(trade_of(&name(role, n)), Some(role));
            }
        }
        assert_eq!(trade_of("Ada Lovelace"), None);
        assert_eq!(trade_of("Sawyer"), Some(Role::Crafter(Craft::Planks)));
        assert_eq!(trade_of(""), None);
    }

    #[test]
    fn forty_eight_citizens_in_a_row_have_different_given_names() {
        let given: HashSet<String> = (100..148)
            .map(|n| name(Role::assign(148)[n as usize], n).split(' ').next().unwrap().to_string())
            .collect();
        assert_eq!(given.len(), GIVEN.len());
    }

    #[test]
    fn names_fit_the_servers_limit_and_mostly_differ() {
        for role in ROLES {
            let names: HashSet<String> = (0..20).map(|n| name(role, n)).collect();
            // 48 × 6 names to draw from: twenty draws seldom meet twice
            assert!(names.len() >= 17, "{role}: {names:?}");
            // backend/src/citizens.rs, MAX_NAME_LEN
            assert!(names.iter().all(|n| n.chars().count() <= 32));
        }
    }
}
