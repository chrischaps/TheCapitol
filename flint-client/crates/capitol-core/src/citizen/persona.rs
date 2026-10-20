//! Who a citizen is. So far just its name (D5): a given name a grandparent
//! might have had, and a surname that says what the citizen does, as the
//! old trade names did. C1.2 (#250) adds the rest of the persona.

use crate::sim::{Craft, Gather, Role};

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
    }
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
    use crate::sim::MIX;
    use std::collections::HashSet;

    #[test]
    fn a_name_is_a_given_name_and_a_trade_name_and_always_the_same() {
        let ada = name(Role::Builder, 7);
        assert_eq!(ada, name(Role::Builder, 7));
        let (given, surname) = ada.split_once(' ').unwrap();
        assert!(GIVEN.contains(&given));
        assert!(surnames(Role::Builder).contains(&surname));
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
        for (role, _) in MIX {
            let names: HashSet<String> = (0..20).map(|n| name(role, n)).collect();
            // 48 × 6 names to draw from: twenty draws seldom meet twice
            assert!(names.len() >= 17, "{role}: {names:?}");
            // backend/src/citizens.rs, MAX_NAME_LEN
            assert!(names.iter().all(|n| n.chars().count() <= 32));
        }
    }
}
