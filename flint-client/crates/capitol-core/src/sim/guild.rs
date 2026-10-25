//! Guilds as a citizen weighs them (Commons S4.5): which guilds are for its
//! trade, what it pays in, and when it gives one up. Pure, so it's tested
//! without a server.
//!
//! - **For its trades.** A guild is on a citizen's menu when its name or
//!   charter names one of its willing jobs' trades (Callings K1.2) in a word
//!   of [`Role::trade_words`]: "ore" for mining, "planks" for sawing, as
//!   whole words, a plural or not. Its favourite's guilds come first.
//! - **Paying in.** On a guild day, once a game week ([`guild_day`]), a
//!   member deposits a tenth of what it gained over the day, never from
//!   what it keeps back ([`deposit`]).
//! - **Leaving.** A member leaves a guild whose debt for dues has passed
//!   [`LEAVE_AFTER_WEEKS`] weeks of them ([`too_deep`]): it stops paying into
//!   a guild that's going under, and two weeks still leaves the guild two to
//!   find the money before it's dissolved.

use std::collections::HashSet;

use capitol_protocol::rest::{GuildSummary, GuildTreasury};
use uuid::Uuid;

use super::Role;
use crate::citizen::decide::{Choice, GuildSeen};

/// A game week's days: one of them is the guild day.
pub const GUILD_WEEK_DAYS: i64 = 7;
/// The part of a day's gain a member pays in on a guild day: a tenth.
pub const DEPOSIT_PART: i64 = 10;
/// Weeks of dues owed past which a citizen leaves its guild.
pub const LEAVE_AFTER_WEEKS: i64 = 2;
/// The most of a charter a question carries: enough to say what it's for.
pub const CHARTER_SHOWN: usize = 200;
/// The most guilds on one question's menu.
pub const MENU_GUILDS: usize = 5;

/// The guilds a citizen who works at `jobs` is asked about (S4.5), from the
/// registry as `listed` has it (the largest first, each with its charter):
/// those for one of its jobs' trades, and those in `invited`, which come
/// first. Then those of its first job (its favourite, Callings K1.2), then
/// the next's. Guilds it has `applied` to already wait on their officers,
/// and aren't asked again. At most [`MENU_GUILDS`].
pub fn seen(jobs: &[Role], listed: &[(&GuildSummary, &str)], invited: &[Uuid], applied: &HashSet<Uuid>) -> Vec<(Uuid, GuildSeen)> {
    let mut seen: Vec<(usize, Uuid, GuildSeen)> = listed
        .iter()
        .filter(|(g, _)| !applied.contains(&g.id))
        .filter_map(|(g, charter)| {
            let rank = jobs.iter().position(|&job| names_trade(job, &g.name, charter));
            let trade = rank.is_some();
            let invited = invited.contains(&g.id);
            (trade || invited).then(|| {
                let guild = GuildSeen {
                    name: g.name.clone(),
                    tag: g.tag.clone(),
                    members: g.members,
                    charter: shown(charter),
                    invited,
                    trade,
                };
                (rank.unwrap_or(jobs.len()), g.id, guild)
            })
        })
        .collect();
    // Stable: those that asked it first, then by its jobs, then the
    // registry's order
    seen.sort_by_key(|(rank, _, g)| (!g.invited, *rank));
    seen.truncate(MENU_GUILDS);
    seen.into_iter().map(|(_, id, g)| (id, g)).collect()
}

/// The menu for [`seen`]: ask to join each, or stay out, last.
pub fn menu(seen: &[(Uuid, GuildSeen)]) -> Vec<Choice> {
    seen.iter().map(|(_, g)| Choice::Apply { guild: g.name.clone() }).chain([Choice::Stay]).collect()
}

/// Whether `name` or `charter` names `role`'s trade, as whole words,
/// ignoring case: "The Ore Guild", "for the miners of the ranges".
pub fn names_trade(role: Role, name: &str, charter: &str) -> bool {
    let words = role.trade_words();
    words_of(name).chain(words_of(charter)).any(|w| {
        words.iter().any(|t| w == *t || w.strip_suffix('s').is_some_and(|s| s == *t) || w.strip_suffix("es").is_some_and(|s| s == *t))
    })
}

/// The words of `text`, in lower case.
fn words_of(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase)
}

/// Whether game day `day` ([`rules::day_number`](capitol_protocol::rules::day_number))
/// is a guild day: the first of each game week.
pub fn guild_day(day: i64) -> bool {
    day.rem_euclid(GUILD_WEEK_DAYS) == 0
}

/// The game week `day` falls in.
pub fn week_of(day: i64) -> i64 {
    day.div_euclid(GUILD_WEEK_DAYS)
}

/// What a member pays in on a guild day: a tenth of `gained` over the day,
/// as far as `strands` reaches past `kept`, what it keeps back. 0 for a
/// day that gained nothing.
pub fn deposit(gained: i64, strands: i64, kept: i64) -> i64 {
    (gained / DEPOSIT_PART).min(strands - kept).max(0)
}

/// Whether a treasury owes more than [`LEAVE_AFTER_WEEKS`] weeks of its dues.
pub fn too_deep(treasury: &GuildTreasury) -> bool {
    treasury.owed > LEAVE_AFTER_WEEKS * treasury.weekly_dues
}

/// The start of `charter`, as a question carries it: up to
/// [`CHARTER_SHOWN`] characters, at a word's end, with an ellipsis if
/// there was more.
pub fn shown(charter: &str) -> String {
    let charter = charter.trim();
    if charter.chars().count() <= CHARTER_SHOWN {
        return charter.to_string();
    }
    let cut: String = charter.chars().take(CHARTER_SHOWN).collect();
    let cut = cut.rsplit_once(char::is_whitespace).map_or(cut.as_str(), |(head, _)| head);
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Craft, Gather, ROLES};

    #[test]
    fn a_guild_is_for_a_trade_its_name_or_charter_names() {
        let ore = Role::Gatherer(Gather::Ore);
        let planks = Role::Crafter(Craft::Planks);
        assert!(names_trade(ore, "The Ore Guild", ""));
        assert!(names_trade(ore, "Hill Folk", "For the miners of the ranges, and their ore."));
        assert!(names_trade(planks, "Sawyers", "We cut PLANKS."));
        assert!(names_trade(Role::Crafter(Craft::Tools), "The Toolmakers", "tools, handles"));
        // Whole words only: "store" isn't "ore", "explore" neither
        assert!(!names_trade(ore, "The Store", "We explore the city."));
        // Another trade's words aren't its
        assert!(!names_trade(planks, "The Ore Guild", "ore and ingots"));
        assert!(!names_trade(ore, "Sawyers", "planks"));
        // Every role has words, and its own guild takes it
        for role in ROLES {
            let word = role.trade_words()[0];
            assert!(names_trade(role, "", &format!("A guild of {word}s.")), "{role}");
        }
    }

    fn summary(n: u128, name: &str, members: u32) -> GuildSummary {
        GuildSummary {
            id: Uuid::from_u128(n),
            name: name.into(),
            tag: "TAG".into(),
            members,
            leader: capitol_protocol::rest::Correspondent {
                id: Uuid::from_u128(100),
                name: "Ada Reeder".into(),
                citizen: false,
                office: false,
            },
            founded_at: chrono::DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_menu_holds_its_trades_guilds_and_those_that_invited_it() {
        let ore = Role::Gatherer(Gather::Ore);
        let (diggers, ore_guild, sawyers, store) = (
            summary(1, "Diggers", 9),
            summary(2, "The Ore Guild", 4),
            summary(3, "Sawyers", 7),
            summary(4, "The Store", 2),
        );
        let listed = [(&diggers, "Miners of the ranges."), (&ore_guild, ""), (&sawyers, "Planks."), (&store, "Shopkeepers.")];
        let names = |seen: &[(Uuid, GuildSeen)]| seen.iter().map(|(_, g)| g.name.clone()).collect::<Vec<_>>();
        // Its trade's, the largest first: by charter, then by name
        let seen = seen(&[ore], &listed, &[], &HashSet::new());
        assert_eq!(names(&seen), ["Diggers", "The Ore Guild"]);
        assert!(seen.iter().all(|(_, g)| g.trade && !g.invited));
        // An invitation from another trade's guild comes first
        let asked = super::seen(&[ore], &listed, &[sawyers.id], &HashSet::new());
        assert_eq!(names(&asked), ["Sawyers", "Diggers", "The Ore Guild"]);
        assert!(asked[0].1.invited && !asked[0].1.trade);
        // One it applied to waits on its officers
        let applied = HashSet::from([diggers.id]);
        assert_eq!(names(&super::seen(&[ore], &listed, &[], &applied)), ["The Ore Guild"]);
        // A trade no guild names has none
        assert!(super::seen(&[Role::Builder], &listed, &[], &HashSet::new()).is_empty());
        // Several trades (Callings K1.2): every one's guilds, its favourite's first
        let planks = Role::Crafter(Craft::Planks);
        assert_eq!(names(&super::seen(&[planks, ore], &listed, &[], &HashSet::new())), ["Sawyers", "Diggers", "The Ore Guild"]);
        assert_eq!(names(&super::seen(&[ore, planks], &listed, &[], &HashSet::new())), ["Diggers", "The Ore Guild", "Sawyers"]);
        // Asking to join each, then staying out
        assert_eq!(
            menu(&seen),
            [Choice::Apply { guild: "Diggers".into() }, Choice::Apply { guild: "The Ore Guild".into() }, Choice::Stay]
        );
        // At most five
        let many: Vec<GuildSummary> = (0..8).map(|n| summary(10 + n, &format!("Ore Guild {n}"), 3)).collect();
        let listed: Vec<(&GuildSummary, &str)> = many.iter().map(|g| (g, "")).collect();
        assert_eq!(super::seen(&[ore], &listed, &[], &HashSet::new()).len(), MENU_GUILDS);
    }

    #[test]
    fn guild_day_comes_once_a_game_week() {
        let days: Vec<i64> = (0..21).filter(|d| guild_day(*d)).collect();
        assert_eq!(days, [0, 7, 14]);
        assert!(guild_day(-7));
        assert_eq!((week_of(6), week_of(7), week_of(-1)), (0, 1, -1));
    }

    #[test]
    fn a_member_pays_a_tenth_of_its_gain_from_what_it_can_spare() {
        // A tenth of the day's gain
        assert_eq!(deposit(250, 1000, 100), 25);
        // Never into what it keeps back
        assert_eq!(deposit(250, 110, 100), 10);
        assert_eq!(deposit(250, 90, 100), 0);
        // A day that gained nothing, or lost, pays nothing
        assert_eq!(deposit(0, 1000, 100), 0);
        assert_eq!(deposit(-40, 1000, 100), 0);
        assert_eq!(deposit(9, 1000, 100), 0);
    }

    #[test]
    fn a_member_leaves_past_two_weeks_of_debt() {
        let treasury = |owed| GuildTreasury {
            strands: 0,
            owed,
            weekly_dues: 60,
            dissolves_at: 240,
            withdrawal_limit: 0,
            withdrawn_this_week: 0,
        };
        assert!(!too_deep(&treasury(0)));
        assert!(!too_deep(&treasury(120)));
        assert!(too_deep(&treasury(121)));
    }

    #[test]
    fn a_long_charter_is_cut_at_a_word() {
        assert_eq!(shown("  Ore, and those who dig it. "), "Ore, and those who dig it.");
        let long = "ore ".repeat(80);
        let cut = shown(&long);
        assert!(cut.ends_with("ore…"), "{cut}");
        assert!(cut.chars().count() <= CHARTER_SHOWN + 1);
    }
}
