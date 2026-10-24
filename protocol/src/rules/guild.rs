//! Guilds' rules (commons plan D10, S4.1): what a guild's name and tag may
//! be, what founding and a roster change cost, and who in a guild may do what.
//!
//! - A subscriber founds a guild at the Exchange hall for [`FOUNDING_FEE`]
//!   (GDD §8.4's registration). Citizens never can: they have no subscription.
//! - Each change to the roster costs [`ROSTER_FEE`]: someone joining, an
//!   expulsion, a new role, the lead handed on. Leaving is free, so nobody is
//!   kept in a guild they can't afford to leave.
//! - Roles rank leader, officer, member ([`GuildRole`]). An officer or the
//!   leader invites, and accepts an application; an officer expels members,
//!   the leader officers too; only the leader sets roles and hands on the lead.
//! - A leader whose subscription lapses keeps the guild but can't invite.
//!   After [`LEAD_LAPSE_DAYS`] the lead passes on (`engine::guilds`).
//! - The treasury (S4.2): any member deposits, an officer withdraws up to the
//!   weekly limit the leader sets, the leader without one. It pays the roster's
//!   fee when it can, and the week's dues, [`weekly_dues`], a Strand at a time
//!   through the hours. What it can't pay is debt; officers hear as the debt
//!   passes each week ([`debt_warning_due`]), and at [`dissolves_at`] the
//!   guild is dissolved, what's left shared among the members by tenure
//!   ([`share_by_tenure`]).
//!
//! The server checks exactly these, so a client's buttons are its answers.

use serde::{Deserialize, Serialize};

use super::talk::{self, TalkError};

/// What founding a guild costs, in Strands: a sink (GDD §8.4).
pub const FOUNDING_FEE: i64 = 500;
/// What a change to the roster costs, in Strands: a sink.
pub const ROSTER_FEE: i64 = 1;
/// How long a leader's subscription may stay lapsed before the lead passes
/// on: four weeks, real time, as subscriptions run.
pub const LEAD_LAPSE_DAYS: i64 = 28;
/// A guild's dues a week, in Strands: this, plus [`DUES_PER_MEMBER`] for
/// each member. A sink, the GDD's second recurring one after property tax.
pub const DUES_BASE: i64 = 50;
pub const DUES_PER_MEMBER: i64 = 5;
/// Weeks of dues a guild may owe before it's dissolved.
pub const DISSOLVE_AFTER_WEEKS: i64 = 4;
/// The days an officer's withdrawals count against the leader's limit: a
/// rolling week, real time.
pub const WITHDRAWAL_WINDOW_DAYS: i64 = 7;
/// The shortest and longest name, in characters.
pub const NAME_MIN: usize = 3;
pub const NAME_MAX: usize = 40;
/// The shortest and longest tag, in letters.
pub const TAG_MIN: usize = 2;
pub const TAG_MAX: usize = 5;

/// Words a guild's name can't use, in any case or with an s: the city's own
/// institutions, and anything that would pass for staff or the citizens' mark.
pub const RESERVED: &[&str] = &[
    "capitol", "gazette", "exchange", "court", "registry", "bazaar", "bureau", "government",
    "admin", "administrator", "moderator", "official", "staff", "citizen",
];

/// Tags no guild can wear, besides any [`RESERVED`] word that fits.
pub const RESERVED_TAGS: &[&str] = &["CAP", "GAZ", "EX", "EXC", "ADM", "MOD", "GM", "DEV", "CIT", "SYS"];

/// Why a guild's name, tag or charter can't be what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Text(TalkError),
    NameLength,
    /// A name needs a letter in it somewhere
    NoLetters,
    /// The reserved word it uses
    Reserved(String),
    TagLength,
    /// A tag is letters only
    TagLetters,
    ReservedTag,
}

impl NameError {
    /// Words a client can show.
    pub fn message(&self) -> String {
        match self {
            NameError::Text(e) => e.message(),
            NameError::NameLength => format!("A guild's name is {NAME_MIN} to {NAME_MAX} characters"),
            NameError::NoLetters => "A guild's name needs a letter in it".into(),
            NameError::Reserved(word) => format!("A guild's name can't use \"{word}\": it belongs to the city"),
            NameError::TagLength => format!("A tag is {TAG_MIN} to {TAG_MAX} letters"),
            NameError::TagLetters => "A tag is letters only".into(),
            NameError::ReservedTag => "That tag belongs to the city".into(),
        }
    }
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for NameError {}

/// The reserved word `word` is, if it's one, as itself or with an s.
fn reserved(word: &str) -> Option<&'static str> {
    let word = word.to_lowercase();
    RESERVED
        .iter()
        .find(|r| word == **r || word.strip_suffix('s') == Some(**r))
        .copied()
}

/// A guild's name as it's kept, or why it can't be: cleaned as a line of
/// talk is, then [`NAME_MIN`] to [`NAME_MAX`] characters with a letter, and
/// no [`RESERVED`] word. Names are unique ignoring case.
pub fn clean_name(name: &str) -> Result<String, NameError> {
    let name = talk::clean(name).map_err(NameError::Text)?;
    if !(NAME_MIN..=NAME_MAX).contains(&name.chars().count()) {
        return Err(NameError::NameLength);
    }
    if !name.chars().any(char::is_alphabetic) {
        return Err(NameError::NoLetters);
    }
    if let Some(word) = name.split(|c: char| !c.is_alphanumeric()).find_map(reserved) {
        return Err(NameError::Reserved(word.into()));
    }
    Ok(name)
}

/// A guild's tag as it's kept, in capitals, or why it can't be:
/// [`TAG_MIN`] to [`TAG_MAX`] ASCII letters, not a reserved one.
pub fn clean_tag(tag: &str) -> Result<String, NameError> {
    let tag = tag.trim();
    if !tag.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(NameError::TagLetters);
    }
    if !(TAG_MIN..=TAG_MAX).contains(&tag.len()) {
        return Err(NameError::TagLength);
    }
    let tag = tag.to_ascii_uppercase();
    if RESERVED_TAGS.contains(&tag.as_str()) || reserved(&tag).is_some() {
        return Err(NameError::ReservedTag);
    }
    Ok(tag)
}

/// A charter as it's kept: cleaned as a letter is, and it may be empty.
pub fn clean_charter(charter: &str) -> Result<String, NameError> {
    match talk::clean_letter(charter) {
        Ok(charter) => Ok(charter),
        Err(TalkError::Empty) => Ok(String::new()),
        Err(e) => Err(NameError::Text(e)),
    }
}

/// A week's dues for a guild of `members`.
pub fn weekly_dues(members: u32) -> i64 {
    DUES_BASE + DUES_PER_MEMBER * i64::from(members)
}

/// The debt at which a guild of `members` is dissolved: four weeks' dues.
pub fn dissolves_at(members: u32) -> i64 {
    weekly_dues(members) * DISSOLVE_AFTER_WEEKS
}

/// Whether debt going from `before` to `after` passed a whole week of a
/// guild of `members`' dues, short of dissolving it: its officers hear.
pub fn debt_warning_due(before: i64, after: i64, members: u32) -> bool {
    let weekly = weekly_dues(members);
    after > before
        && after < dissolves_at(members)
        && (1..DISSOLVE_AFTER_WEEKS).any(|k| before < k * weekly && after >= k * weekly)
}

/// `strands` shared among members who've served `tenures` (in any unit,
/// each counted as at least 1), in proportion, in whole Strands. The
/// Strands a fair share leaves over go one each to the largest remainders,
/// the earlier first on a tie, so the shares add up to `strands` exactly.
pub fn share_by_tenure(strands: i64, tenures: &[i64]) -> Vec<i64> {
    if strands <= 0 || tenures.is_empty() {
        return vec![0; tenures.len()];
    }
    let tenures: Vec<i128> = tenures.iter().map(|t| i128::from((*t).max(1))).collect();
    let total: i128 = tenures.iter().sum();
    let mut shares: Vec<i64> = tenures.iter().map(|t| (i128::from(strands) * t / total) as i64).collect();
    let mut left = strands - shares.iter().sum::<i64>();
    let mut by_remainder: Vec<usize> = (0..tenures.len()).collect();
    by_remainder.sort_by_key(|&i| std::cmp::Reverse(i128::from(strands) * tenures[i] % total));
    for i in by_remainder {
        if left == 0 {
            break;
        }
        shares[i] += 1;
        left -= 1;
    }
    shares
}

/// A place in a guild, highest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum GuildRole {
    Leader,
    Officer,
    Member,
}

impl GuildRole {
    /// The column's value, and the wire's.
    pub fn as_str(self) -> &'static str {
        match self {
            GuildRole::Leader => "leader",
            GuildRole::Officer => "officer",
            GuildRole::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<GuildRole> {
        match s {
            "leader" => Some(GuildRole::Leader),
            "officer" => Some(GuildRole::Officer),
            "member" => Some(GuildRole::Member),
            _ => None,
        }
    }

    fn rank(self) -> u8 {
        match self {
            GuildRole::Leader => 2,
            GuildRole::Officer => 1,
            GuildRole::Member => 0,
        }
    }

    /// May invite someone, or accept their application. A leader also needs
    /// a running subscription, which the server checks beside this.
    pub fn may_invite(self) -> bool {
        self != GuildRole::Member
    }

    /// May expel someone of role `them`: an officer expels members, the
    /// leader officers and members.
    pub fn may_expel(self, them: GuildRole) -> bool {
        self.may_invite() && self.rank() > them.rank()
    }

    /// May make a member an officer or an officer a member, or hand on the lead.
    pub fn may_set_roles(self) -> bool {
        self == GuildRole::Leader
    }

    /// May draw on the treasury: an officer up to the leader's weekly limit,
    /// the leader without one.
    pub fn may_withdraw(self) -> bool {
        self != GuildRole::Member
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_cleaned_and_measured() {
        assert_eq!(clean_name("  The   Rope\tWalkers ").unwrap(), "The Rope Walkers");
        assert_eq!(clean_name("Ox"), Err(NameError::NameLength));
        assert_eq!(clean_name(&"a".repeat(NAME_MAX + 1)), Err(NameError::NameLength));
        assert_eq!(clean_name("1234"), Err(NameError::NoLetters));
        assert_eq!(clean_name("   "), Err(NameError::Text(TalkError::Empty)));
        assert!(clean_name("Ölmühle Brüder").is_ok(), "any letters");
    }

    #[test]
    fn the_citys_words_are_reserved_in_any_case_or_plural() {
        for name in ["Capitol Weavers", "the gazette", "EXCHANGE men", "Court of Rope", "The Capitol's Own", "Citizens United", "Admins"] {
            assert!(matches!(clean_name(name), Err(NameError::Reserved(_))), "{name}");
        }
        assert_eq!(clean_name("Capitol Weavers"), Err(NameError::Reserved("capitol".into())));
        // Only whole words
        assert!(clean_name("Courtyard Masons").is_ok());
        assert!(clean_name("Exchangers of Grain").is_ok());
    }

    #[test]
    fn a_tag_is_two_to_five_letters_in_capitals() {
        assert_eq!(clean_tag(" rope ").unwrap(), "ROPE");
        assert_eq!(clean_tag("R"), Err(NameError::TagLength));
        assert_eq!(clean_tag("ROPERS"), Err(NameError::TagLength));
        assert_eq!(clean_tag("R0PE"), Err(NameError::TagLetters));
        assert_eq!(clean_tag("RÖPE"), Err(NameError::TagLetters));
        assert_eq!(clean_tag("cap"), Err(NameError::ReservedTag));
        assert_eq!(clean_tag("Court"), Err(NameError::ReservedTag), "a reserved word that fits");
    }

    #[test]
    fn a_charter_may_be_empty() {
        assert_eq!(clean_charter("  ").unwrap(), "");
        assert_eq!(clean_charter("We make rope.\n\n\n\nGood rope.").unwrap(), "We make rope.\n\nGood rope.");
        assert!(matches!(clean_charter(&"a".repeat(talk::MAX_LETTER + 1)), Err(NameError::Text(_))));
    }

    #[test]
    fn dues_are_fifty_and_five_a_member_and_four_weeks_dissolve() {
        assert_eq!(weekly_dues(0), 50);
        assert_eq!(weekly_dues(6), 80);
        assert_eq!(dissolves_at(6), 320);
    }

    #[test]
    fn officers_hear_as_the_debt_passes_each_week() {
        // 80 a week for six
        assert!(!debt_warning_due(0, 1, 6));
        assert!(!debt_warning_due(78, 79, 6));
        assert!(debt_warning_due(79, 80, 6), "a week");
        assert!(!debt_warning_due(80, 81, 6));
        assert!(debt_warning_due(159, 160, 6) && debt_warning_due(239, 240, 6));
        assert!(!debt_warning_due(319, 320, 6), "four weeks dissolve, not warn");
        assert!(!debt_warning_due(90, 80, 6), "paid down");
    }

    #[test]
    fn the_rest_is_shared_by_tenure_to_the_strand() {
        assert_eq!(share_by_tenure(100, &[3, 1]), vec![75, 25]);
        // 10 over 3 equal: 3 each and one over, to the first
        assert_eq!(share_by_tenure(10, &[5, 5, 5]), vec![4, 3, 3]);
        // The largest remainder takes the Strand left
        assert_eq!(share_by_tenure(10, &[1, 2]), vec![3, 7]);
        assert_eq!(share_by_tenure(7, &[0, 0]), vec![4, 3], "a tenure counts as at least 1");
        assert_eq!(share_by_tenure(0, &[3, 1]), vec![0, 0]);
        assert!(share_by_tenure(5, &[]).is_empty());
        for (strands, tenures) in [(999, vec![86_400, 3_600, 1]), (i64::MAX / 4, vec![i64::MAX / 2, 7])] {
            assert_eq!(share_by_tenure(strands, &tenures).iter().sum::<i64>(), strands);
        }
    }

    #[test]
    fn each_role_has_its_rights() {
        use GuildRole::*;
        assert!(Leader.may_invite() && Officer.may_invite() && !Member.may_invite());
        assert!(Leader.may_expel(Officer) && Leader.may_expel(Member));
        assert!(Officer.may_expel(Member) && !Officer.may_expel(Officer) && !Officer.may_expel(Leader));
        assert!(!Member.may_expel(Member));
        assert!(Leader.may_set_roles() && !Officer.may_set_roles() && !Member.may_set_roles());
        assert!(Leader.may_withdraw() && Officer.may_withdraw() && !Member.may_withdraw());
        for role in [Leader, Officer, Member] {
            assert_eq!(GuildRole::parse(role.as_str()), Some(role));
        }
    }
}
