//! `capitol-town`: the citizens of The Capitol (C1.1), AI players who live
//! in the world, run from a roster.
//!
//! ```text
//! capitol-town mint --count 20      # make twenty citizens, onto town.toml
//! capitol-town mint --count 2 --role merchant   # and two trading favourites (C1.6, K2.2)
//! capitol-town mint --count 3 --calling all-rounder   # three with no favourite (K0.2)
//! capitol-town mint --online 20     # enough citizens that 20 are on, on average (#276)
//! capitol-town habits               # who's on when, over the coming weeks (#276)
//! capitol-town road-in              # where each citizen is on the road in (#450)
//! capitol-town                      # live until Ctrl-C
//! capitol-town --always-on          # every citizen on, day and night (#276)
//! capitol-town --report             # and write what the town did when it stops (C1.7)
//! capitol-town soak --hours 24      # a day, then the report, load and conservation (C1.7)
//! capitol-town books town.soak-….ledgers.jsonl  # a soak's books, even one that was killed
//! capitol-town soak --hours 2 --tunables tax-high.toml  # a soak under a preset (D6.3)
//! capitol-town compare a.balance.json b.balance.json    # two soaks side by side (D6.3)
//! capitol-town --status             # the running town's last health
//! capitol-town replay --layers rules # the decision logs, replayed (C3.1)
//! capitol-town corpus               # a made-up town's log to replay (C3.3)
//! capitol-town replay town.corpus --layers rules,jev   # Jev measured on it
//! ```
//!
//! Run it from `flint-client/` with `cargo run -p capitol-core --bin capitol-town -- <flags>`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capitol_core::citizen::balance::{self, Balance, Preset};
use capitol_core::citizen::decide::{corpus, log};
use capitol_core::citizen::decide::replay::Replayer;
use capitol_core::citizen::decide::{self, Cascade};
use capitol_core::citizen::departed;
use capitol_core::citizen::habit::{self, Habit, Kind};
use capitol_core::citizen::measures;
use capitol_core::citizen::report::{Soak, TownReport};
use capitol_core::citizen::schedule;
use capitol_core::citizen::voice;
use capitol_core::citizen::town::{self, Admin, Hours, Logs, Reporter, Roster, Town};
use capitol_core::catalog::Catalog;
use capitol_core::net::{self, NetConfig, RestRequest, RestResponse};
use capitol_core::protocol::ledger::Ledger;
use capitol_core::protocol::metrics::{self, TOWN_REPORT_MS};
use capitol_core::citizen::calling::Style;
use capitol_core::sim::Role;

const USAGE: &str = "\
capitol-town: the citizens of The Capitol

USAGE:
    capitol-town [run]           Live until Ctrl-C: each citizen on the
                                 roster signs in when its habit says, on
                                 the real clock (a few hours some days, in
                                 its own evenings), and plays the game's
                                 day while it's on: its trade, its market
                                 hours, home at dusk, and the night's work
                                 from there. When its session's over it
                                 logs out until the next. A citizen whose
                                 life ends starts again after a backoff;
                                 one whose key is refused retires.
    capitol-town soak --hours <h>
                                 Live as run does for h hours of the day's
                                 clock (24 is a day, two real hours), then
                                 stop and write the town's report with the
                                 server's load, read every --sample, and
                                 whether every Strand and unit is accounted
                                 for. It reads the world's books as an admin
                                 (--admin), so run it on a database of its
                                 own. Exits 1 if they drifted. With
                                 --tunables, it sets a preset first.
    capitol-town compare <a.balance.json> <b.balance.json> [--out <md>]
                                 Hold two soaks up against each other: the
                                 knobs each ran under, whether their books
                                 balanced, the citizens' Strands by trade,
                                 the Gini, the faucets and sinks, and prices
                                 against their targets. Printed, or written
                                 to --out. scripts/balance.ps1 runs the two
                                 soaks, each on a new town, and this. Exits
                                 1 if either's books drifted.
    capitol-town books <ledgers.jsonl>
                                 Reconcile the books a soak wrote down as it
                                 read them, first reading against last, and
                                 print its conservation and load: for a soak
                                 that stopped before its report. Exits 1 if
                                 they drifted.
    capitol-town mint --count <n> [--role <role>] [--calling <style>]
    capitol-town mint --online <n> [--role <role>] [--calling <style>]
                                 Make n citizens through the admin endpoint,
                                 or with --online enough that n of them are
                                 on at once on average, by the roster's mix
                                 of habits, and add them, with their keys
                                 and their seeded personas, callings and
                                 all, to the roster
    capitol-town habits          Who on the roster is on when, over the
                                 next four weeks: its habits, the average
                                 online, the fewest and most, and the
                                 average by hour of the day (UTC)
    capitol-town road-in         Where each citizen on the roster is on the
                                 road in, the tutorial every citizen walks
                                 (#450): steps done, its chapter, its next
                                 step and its deed, asked of the server
                                 with its own key, so a running town plays
                                 on. Exits 1 if one couldn't be asked
    capitol-town status         Print the running town's last health
                                 (also --status)
    capitol-town replay [<log>...] --layers <a,b>
                                 Run a stack of decision layers over the
                                 citizens' decision logs, offline, and
                                 report its agreement with what they chose,
                                 its escalation rate, latency and cost. A
                                 log is a file or a directory of them
                                 [default: town.decisions beside the roster]
    capitol-town corpus [<dir>] [--rounds <n>]
                                 Write a log of made-up situations, every
                                 role's work, sell, buy, bid, stall and
                                 free-time questions, as the rules decide
                                 them, to replay a new layer over
                                 [default: town.corpus beside the roster].
                                 Each round more varies the temperaments
                                 and the scale of books and stock
                                 [default: 1 round, about 100]
    capitol-town voice [--voice <name>]
                                 Hear a voice (#311): three made-up
                                 citizens, reserved, even and warm, each
                                 say a line for every occasion (a greeting,
                                 the words round a trade, a sale, an answer
                                 and a letter's reply), with how long each
                                 took and what they cost. A model's line
                                 is waited for here, where a town says the
                                 phrasebook's after 2 s [default:
                                 claude-haiku]

FLAGS:
    --roster <path>   The roster [default: town.toml]
    --server <url>    Server root [default: $CAPITOL_SERVER, else the
                      roster's `server`, else http://localhost:3000]
    --every <secs>    How often a running town prints and writes its health
                      [default: 60]
    --report          With run, write the town's report beside the roster
                      when it stops: each citizen's and each trade's
                      wealth, output, prices and what held it up, and how
                      its questions were settled
    --hours <h>       With soak, how long, in hours of the day's clock
    --sample <secs>   With soak, how often it reads the server's load
                      [default: 60]
    --tunables <toml> With soak, a preset to run under (D6.3): it's set on
                      the server before the books are read, and every knob
                      it doesn't name goes back to its default. One knob a
                      line, `property_tax_rate = 0.2`; a family's by its id,
                      `\"yield.grass\" = 1.5` or under `[yield]`
    --start-at <hour> With soak, set the server's day clock to this hour
                      first, 9 or 13:30, so the soak lives the hours it
                      asks for whenever it starts (#427). Needs a server
                      started with DEV_CLOCK=on
    --out <md>        With compare, where to write it [default: printed]
    --count <n>       With mint, how many citizens to make
    --online <n>      With mint, make enough citizens to keep n on, on
                      average: about 32 for each with the default mix
    --always-on       With run or soak, every citizen stays on through
                      day and night, whatever its habit: for a soak of the
                      whole pool
    --role <role>     With mint, make every one of them this: gatherer-fiber,
                      gatherer-wood, gatherer-ore, crafter-planks,
                      crafter-ingots, crafter-tools, builder or merchant
                      [default: the sim's mix, which has no merchants].
                      It's the citizen's birth trade: its name, its home,
                      and its calling's favourite. A merchant is a trading
                      favourite, never an all-rounder unless --calling
                      says; anyone else trades too once it has its float
    --calling <style> With mint, deal every one of them this style of
                      calling: specialist (about 70% of its work on its
                      favourite), leaner (about 45%) or all-rounder (no
                      favourite) [default: the roster's [callings] mix,
                      40 / 40 / 20]
    --admin <email>   With mint or soak, an admin account to act as
                      [default: $CAPITOL_ADMIN_EMAIL]. Its password is read
                      from $CAPITOL_ADMIN_PASSWORD only, never a flag.
    --log-mb <n>      With run or soak, how big a citizen's decision log
                      grows before it rotates (two old ones are kept); 0
                      logs nothing [default: 4]
    --layers <a,b>    With replay, the stack to run, bottom first:
                      rules, jev (TypeSafe; $TYPESAFE_API_KEY),
                      jev-router (OpenRouter; $OPENROUTER_API_KEY),
                      claude-haiku or claude-sonnet (Anthropic;
                      $ANTHROPIC_API_KEY). A layer may set its threshold:
                      jev@0.8 [default: rules]
    --stride <n>      With replay, replay every nth decision only, to
                      sample a costly layer [default: 1]
    --in-flight <n>   With replay, how many decisions may wait on a slow
                      layer at once; 1 replays one at a time [default: 8]
    --voice <name>    With voice, whose words: phrasebook, claude-haiku or
                      claude-sonnet ($ANTHROPIC_API_KEY). A roster gives
                      its citizens one with `voice = \"claude-haiku\"`
                      [default: phrasebook]
    -h, --help        Print this

The roster holds keys, so keep it out of git (flint-client/.gitignore has
town.toml); town.example.toml shows its shape. Health goes beside it, in
town.status, and who was where in their day is appended to
town.presence.csv. Each citizen logs its decisions, and what came of them,
to town.decisions/<name>.jsonl. Reports go beside it too, as
town.report-<when>.md, and a soak's as town.soak-<when>.md with its load,
a minute a row, in town.soak-<when>.csv, each citizen's time by
activity, a day a row, in town.soak-<when>.activity.csv (and
town.report-<when>.activity.csv), and every reading of the
server's books, as it's taken, in town.soak-<when>.ledgers.jsonl, and
what a balance run compares in town.soak-<when>.balance.json. A
running town also reports to the server each game minute, for its admin
console, with the key in town.report-key: a run with
$CAPITOL_ADMIN_EMAIL and $CAPITOL_ADMIN_PASSWORD set mints one if
there's none. The key of every citizen the town has run or minted is kept
in town.known.json beside the roster, so one dropped from the roster is
settled as the town next starts: it signs in once, closes its stall and
takes its orders, commissions, wage offers and claims down (#585), and is
forgotten. Ctrl-C lets each citizen finish
its step and stops; any order it has resting on the book stays up, as a
market maker's does. A second Ctrl-C stops at once.";

/// How long a stopping town waits for its citizens to finish their steps.
const GRACE: Duration = Duration::from_secs(20);
/// After a soak's citizens stop, how long it waits before reading the books
/// at the end, so the market's matching (each second) and the writes under
/// way land first.
const SETTLE: Duration = Duration::from_secs(5);
/// How long a report to the server may take before it's given up on
const POST_TIMEOUT: Duration = Duration::from_secs(5);
/// A decision log's size before it rotates, in MB, unless told otherwise.
const LOG_MB: u64 = log::LOG_LIMIT >> 20;
/// How many decisions a replay lets wait on a slow layer at once, unless
/// told otherwise (#301): a network layer replays this many times as fast,
/// and stays under its API's rate limits.
const IN_FLIGHT: usize = 8;

#[derive(Debug, PartialEq)]
enum Action {
    Run,
    Soak { hours: f64 },
    Books { file: PathBuf },
    Compare { a: PathBuf, b: PathBuf, out: Option<PathBuf> },
    Mint { size: Size },
    Habits,
    RoadIn,
    Status,
    Replay {
        logs: Vec<PathBuf>,
        layers: Vec<String>,
        stride: usize,
        in_flight: usize,
    },
    Corpus { dir: Option<PathBuf>, rounds: u32 },
    Voice { name: String },
}

/// How many citizens a mint makes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Size {
    Count(usize),
    /// Enough to keep this many on, on average (#276)
    Online(f64),
}

#[derive(Debug)]
struct Options {
    action: Action,
    roster: PathBuf,
    server: Option<String>,
    every: Duration,
    admin: Option<String>,
    /// With mint, the role to make every citizen
    role: Option<Role>,
    /// With mint, the style of calling to deal every citizen (K0.2)
    calling: Option<Style>,
    /// 0: no decision logs
    log_mb: u64,
    /// With run, write the report when it stops
    report: bool,
    /// With soak, how often it reads the ledger
    sample: Duration,
    /// With run or soak, everyone on all the time, whatever their habits
    always_on: bool,
    /// With soak, the preset to run under (D6.3)
    tunables: Option<PathBuf>,
    /// With soak, the hour to set the server's day clock to first (#427)
    start_at: Option<f64>,
}

impl Options {
    /// Whether the town lives: run, or soak.
    fn lives(&self) -> bool {
        matches!(self.action, Action::Run | Action::Soak { .. })
    }
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut action = None;
    let mut count = None;
    let mut online = None;
    let mut hours = None;
    let mut logs = Vec::new();
    let mut layers = None;
    let mut stride = None;
    let mut in_flight = None;
    let mut corpus_dir = None;
    let mut rounds = None;
    let mut log_mb_given = false;
    let mut sample_given = false;
    let mut compared: Vec<PathBuf> = Vec::new();
    let mut out = None;
    let mut voice_name = None;
    let mut options = Options {
        action: Action::Run,
        roster: PathBuf::from(town::ROSTER),
        server: std::env::var("CAPITOL_SERVER").ok().filter(|s| !s.is_empty()),
        every: Duration::from_secs(60),
        admin: std::env::var("CAPITOL_ADMIN_EMAIL").ok().filter(|s| !s.is_empty()),
        role: None,
        calling: None,
        log_mb: LOG_MB,
        report: false,
        sample: Duration::from_secs(60),
        always_on: false,
        tunables: None,
        start_at: None,
    };
    let mut admin_given = false;
    let mut every_given = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        let replaying = matches!(action, Some(Action::Replay { .. }));
        let corpus = matches!(action, Some(Action::Corpus { .. }));
        let reading_books = matches!(&action, Some(Action::Books { file }) if file.as_os_str().is_empty());
        let comparing = matches!(action, Some(Action::Compare { .. }));
        let mut set = |a: Action| match action.replace(a) {
            Some(_) => Err(
                "run, soak, books, compare, mint, habits, road-in, status, replay, corpus and voice go one at a time".to_string(),
            ),
            None => Ok(()),
        };
        match arg.as_str() {
            "run" => set(Action::Run)?,
            "soak" => set(Action::Soak { hours: 0.0 })?,
            "books" => set(Action::Books { file: PathBuf::new() })?,
            "compare" => set(Action::Compare {
                a: PathBuf::new(),
                b: PathBuf::new(),
                out: None,
            })?,
            "--tunables" => options.tunables = Some(value()?.into()),
            "--out" => out = Some(PathBuf::from(value()?)),
            "mint" => set(Action::Mint { size: Size::Count(0) })?,
            "habits" => set(Action::Habits)?,
            "road-in" => set(Action::RoadIn)?,
            "--always-on" => options.always_on = true,
            "--online" => {
                let n: f64 = number(&arg, &value()?)?;
                if n.is_nan() || n <= 0.0 {
                    return Err("--online must be more than 0".into());
                }
                online = Some(n);
            }
            "status" | "--status" => set(Action::Status)?,
            "replay" => set(Action::Replay {
                logs: Vec::new(),
                layers: Vec::new(),
                stride: 1,
                in_flight: IN_FLIGHT,
            })?,
            "corpus" => set(Action::Corpus { dir: None, rounds: 1 })?,
            "voice" => set(Action::Voice { name: String::new() })?,
            "--voice" => {
                let name = value()?;
                voice::check_voice(&name)?;
                voice_name = Some(name);
            }
            "--rounds" => {
                let n: u32 = number(&arg, &value()?)?;
                if n == 0 {
                    return Err("--rounds must be at least 1".into());
                }
                rounds = Some(n);
            }
            "--stride" => {
                let n: usize = number(&arg, &value()?)?;
                if n == 0 {
                    return Err("--stride must be at least 1".into());
                }
                stride = Some(n);
            }
            "--in-flight" => {
                let n: usize = number(&arg, &value()?)?;
                if n == 0 {
                    return Err("--in-flight must be at least 1".into());
                }
                in_flight = Some(n);
            }
            "--layers" => {
                let names: Vec<String> = value()?.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
                decide::check_layers(&names)?;
                layers = Some(names);
            }
            "--log-mb" => {
                options.log_mb = number(&arg, &value()?)?;
                log_mb_given = true;
            }
            "--report" => options.report = true,
            "--start-at" => options.start_at = Some(hour(&arg, &value()?)?),
            "--hours" => {
                let h: f64 = number(&arg, &value()?)?;
                if h.is_nan() || h <= 0.0 {
                    return Err("--hours must be more than 0".into());
                }
                hours = Some(h);
            }
            "--sample" => {
                let secs: f64 = number(&arg, &value()?)?;
                if secs.is_nan() || secs < 5.0 {
                    return Err("--sample takes at least 5 seconds".into());
                }
                options.sample = Duration::from_secs_f64(secs);
                sample_given = true;
            }
            "--roster" => options.roster = value()?.into(),
            "--server" => options.server = Some(value()?),
            "--every" => {
                let secs: f64 = number(&arg, &value()?)?;
                if secs.is_nan() || secs < 1.0 {
                    return Err("--every takes at least 1 second".into());
                }
                options.every = Duration::from_secs_f64(secs);
                every_given = true;
            }
            "--count" => {
                let n: usize = number(&arg, &value()?)?;
                if n == 0 {
                    return Err("--count must be at least 1".into());
                }
                count = Some(n);
            }
            "--role" => {
                let slug = value()?;
                options.role = Some(Role::from_slug(&slug).ok_or_else(|| format!("--role: {slug:?} is no role"))?);
            }
            "--calling" => {
                let style = value()?;
                options.calling = Some(Style::from_str(&style).ok_or_else(|| {
                    format!("--calling: {style:?} is no style; a calling is specialist, leaner or all-rounder")
                })?);
            }
            "--admin" => {
                options.admin = Some(value()?);
                admin_given = true;
            }
            "-h" | "--help" => return Ok(None),
            log if replaying && !log.starts_with('-') => logs.push(PathBuf::from(log)),
            dir if corpus && !dir.starts_with('-') && corpus_dir.is_none() => corpus_dir = Some(PathBuf::from(dir)),
            file if reading_books && !file.starts_with('-') => action = Some(Action::Books { file: file.into() }),
            file if comparing && !file.starts_with('-') && compared.len() < 2 => compared.push(file.into()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let size = match (count, online) {
        (Some(_), Some(_)) => return Err("mint takes --count or --online, not both".into()),
        (Some(n), None) => Some(Size::Count(n)),
        (None, Some(n)) => Some(Size::Online(n)),
        (None, None) => None,
    };
    if online.is_some() && !matches!(action, Some(Action::Mint { .. })) {
        return Err("--online goes with mint".into());
    }
    options.action = match (action.unwrap_or(Action::Run), size, hours) {
        (Action::Mint { .. }, Some(size), None) => Action::Mint { size },
        (Action::Mint { .. }, None, _) => return Err("mint needs --count or --online".into()),
        (_, Some(_), _) => return Err("--count goes with mint".into()),
        (Action::Soak { .. }, None, Some(hours)) => Action::Soak { hours },
        (Action::Soak { .. }, None, None) => return Err("soak needs --hours".into()),
        (_, None, Some(_)) => return Err("--hours goes with soak".into()),
        (Action::Books { file }, None, None) if file.as_os_str().is_empty() => {
            return Err("books needs the file of a soak's books".into())
        }
        (Action::Compare { .. }, None, None) => match <[PathBuf; 2]>::try_from(std::mem::take(&mut compared)) {
            Ok([a, b]) => Action::Compare { a, b, out: out.take() },
            Err(_) => return Err("compare needs two soaks' .balance.json files".into()),
        },
        (Action::Replay { .. }, None, None) => Action::Replay {
            logs,
            layers: layers.take().unwrap_or_else(|| decide::DEFAULT_LAYERS.map(String::from).to_vec()),
            stride: stride.take().unwrap_or(1),
            in_flight: in_flight.take().unwrap_or(IN_FLIGHT),
        },
        (Action::Corpus { .. }, None, None) => Action::Corpus {
            dir: corpus_dir,
            rounds: rounds.take().unwrap_or(1),
        },
        (Action::Voice { .. }, None, None) => Action::Voice {
            name: voice_name.take().unwrap_or_else(|| "claude-haiku".into()),
        },
        (action, None, None) => action,
    };
    if layers.is_some() {
        return Err("--layers goes with replay".into());
    }
    if voice_name.is_some() {
        return Err("--voice goes with voice".into());
    }
    if rounds.is_some() {
        return Err("--rounds goes with corpus".into());
    }
    if stride.is_some() {
        return Err("--stride goes with replay".into());
    }
    if in_flight.is_some() {
        return Err("--in-flight goes with replay".into());
    }
    if log_mb_given && !options.lives() {
        return Err("--log-mb goes with run or soak".into());
    }
    if admin_given && !matches!(options.action, Action::Mint { .. } | Action::Soak { .. }) {
        return Err("--admin goes with mint or soak".into());
    }
    if options.always_on && !options.lives() {
        return Err("--always-on goes with run or soak".into());
    }
    if options.role.is_some() && !matches!(options.action, Action::Mint { .. }) {
        return Err("--role goes with mint".into());
    }
    if options.calling.is_some() && !matches!(options.action, Action::Mint { .. }) {
        return Err("--calling goes with mint".into());
    }
    if every_given && !options.lives() {
        return Err("--every goes with run or soak".into());
    }
    if options.report && options.action != Action::Run {
        return Err("--report goes with run (a soak always reports)".into());
    }
    if sample_given && !matches!(options.action, Action::Soak { .. }) {
        return Err("--sample goes with soak".into());
    }
    if options.tunables.is_some() && !matches!(options.action, Action::Soak { .. }) {
        return Err("--tunables goes with soak".into());
    }
    if options.start_at.is_some() && !matches!(options.action, Action::Soak { .. }) {
        return Err("--start-at goes with soak".into());
    }
    if out.is_some() {
        return Err("--out goes with compare".into());
    }
    Ok(Some(options))
}

/// An hour of the day, 0 to 24: `9`, `13.5` or `13:30`
fn hour(flag: &str, value: &str) -> Result<f64, String> {
    let h = match value.split_once(':') {
        Some((h, m)) => {
            let (h, m): (f64, f64) = (number(flag, h)?, number(flag, m)?);
            if !(0.0..60.0).contains(&m) {
                return Err(format!("{flag}: {value:?} has no such minute"));
            }
            h + m / 60.0
        }
        None => number(flag, value)?,
    };
    if !(0.0..=24.0).contains(&h) {
        return Err(format!("{flag}: {value:?} is outside 0 to 24"));
    }
    Ok(h)
}

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{flag}: {value:?} isn't a number"))
}

/// `--server`, else `$CAPITOL_SERVER` (both in `options.server`), else the
/// roster's, else the local stack.
fn server(options: &Options, roster: &Roster) -> String {
    options
        .server
        .clone()
        .or_else(|| roster.server.clone())
        .unwrap_or_else(|| "http://localhost:3000".into())
}

/// The admin to act as: `--admin` or `$CAPITOL_ADMIN_EMAIL`, with
/// `$CAPITOL_ADMIN_PASSWORD`.
fn admin(options: &Options, doing: &str) -> Result<Admin, String> {
    let email = options
        .admin
        .clone()
        .ok_or_else(|| format!("{doing} needs an admin: --admin <email> or $CAPITOL_ADMIN_EMAIL"))?;
    let password = std::env::var("CAPITOL_ADMIN_PASSWORD")
        .ok()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| format!("{doing} reads the admin's password from $CAPITOL_ADMIN_PASSWORD"))?;
    Ok(Admin { email, password })
}

fn mint(options: &Options, size: Size) -> Result<(), String> {
    let admin = admin(options, "mint")?;
    let mut roster = Roster::load_or_new(&options.roster)?;
    let server = server(options, &roster);
    let had = roster.citizens.len();
    let count = match size {
        Size::Count(n) => n,
        // A pool of the mix is on its share of the time
        Size::Online(n) => (n / roster.mix.share()).ceil().max(1.0) as usize,
    };
    let result = town::mint(&NetConfig::with_base_url(&server), &admin, &mut roster, count, options.role, options.calling);
    // Known from the start, so one dropped before the town runs is settled too (#585)
    if let Err(e) = departed::remember(&roster) {
        eprintln!("capitol-town: writing {}: {e}", departed::known_path(&roster.path).display());
    }
    for citizen in &roster.citizens[had..] {
        let p = &citizen.persona;
        println!(
            "made {} ({}, {}), of the {} {} avenue, at the {} {}, {} in {}",
            citizen.name,
            citizen.role,
            p.calling,
            p.home.zone,
            p.home.avenue,
            p.market,
            p.schedule.market(),
            p.habit.kind,
            p.habit.zone_name()
        );
    }
    let made: Vec<Habit> = roster.citizens[had..].iter().map(|c| c.persona.habit).collect();
    let all: Vec<Habit> = roster.citizens.iter().map(|c| c.persona.habit).collect();
    let (from, hours) = (unix_ms(), HABIT_WEEKS * 7 * 24);
    println!(
        "{}: {} citizens on the roster; the {} made keep {:.1} on, on average, and the roster {:.1}",
        roster.path.display(),
        roster.citizens.len(),
        made.len(),
        habit::expected_online(&made, from, hours),
        habit::expected_online(&all, from, hours)
    );
    result.map(|_| ())
}

/// How far ahead `habits` and `mint --online` look.
const HABIT_WEEKS: usize = 4;

/// Who on the roster is on when (#276): its habits, and how many are on by
/// hour over the next [`HABIT_WEEKS`].
fn habits(options: &Options) -> Result<(), String> {
    let roster = living_roster(options)?;
    let all: Vec<Habit> = roster.citizens.iter().map(|c| c.persona.habit).collect();
    let kinds: Vec<String> = Kind::ALL
        .iter()
        .map(|&k| format!("{} {k}", all.iter().filter(|h| h.kind == k).count()))
        .collect();
    println!("{}: {} citizens: {}", roster.path.display(), all.len(), kinds.join(", "));
    let mut zones: Vec<(i32, usize)> = Vec::new();
    for h in &all {
        match zones.iter_mut().find(|(z, _)| *z == h.zone) {
            Some((_, n)) => *n += 1,
            None => zones.push((h.zone, 1)),
        }
    }
    zones.sort();
    let zones: Vec<String> = zones
        .iter()
        .map(|&(z, n)| format!("{n} in {}", Habit::new(Kind::Casual, z, 0).zone_name()))
        .collect();
    println!("living {}", zones.join(", "));
    // From the top of this hour, so each row is a whole hour of the clock
    let hour_ms = 3_600_000;
    let from = unix_ms().div_euclid(hour_ms) * hour_ms;
    let hours = HABIT_WEEKS * 7 * 24;
    let by_hour = habit::online_by_hour(&all, from, hours);
    let mean = habit::expected_online(&all, from, hours);
    let on_now = all.iter().filter(|h| matches!(h.at(unix_ms()), habit::Presence::On { .. })).count();
    println!(
        "over the next {HABIT_WEEKS} weeks: {mean:.1} on, on average ({:.1}% of the pool; the mix alone says {:.1}%), fewest {}, most {}; {on_now} on now",
        100.0 * mean / all.len().max(1) as f64,
        100.0 * roster.mix.share(),
        by_hour.iter().min().unwrap_or(&0),
        by_hour.iter().max().unwrap_or(&0)
    );
    // Each hour of the day (UTC), averaged over the weeks
    let first = (from / hour_ms).rem_euclid(24) as usize;
    let mut sums = [0usize; 24];
    for (i, n) in by_hour.iter().enumerate() {
        sums[(first + i) % 24] += n;
    }
    let days = (hours / 24) as f64;
    let peak = sums.iter().copied().max().unwrap_or(0).max(1) as f64 / days;
    println!("by the hour (UTC), on average:");
    for (hour, sum) in sums.iter().enumerate() {
        let avg = *sum as f64 / days;
        let bar = "█".repeat((avg / peak * 40.0).round() as usize);
        println!("  {hour:02}:00  {avg:>6.1}  {bar}");
    }
    Ok(())
}

/// Where each citizen on the roster is on the road in (#450), as the server
/// keeps it: asked with each one's key, by REST alone, so a running town's
/// citizens are left playing.
fn road_in(options: &Options) -> Result<(), String> {
    let roster = living_roster(options)?;
    let config = NetConfig::with_base_url(server(options, &roster));
    println!("{}: the road in, citizen by citizen", roster.path.display());
    println!("{:<20} {:<17} {:<9} {:>7}  {:<13} {:<8} next", "citizen", "trade", "home", "steps", "chapter", "deed");
    let (mut walked, mut claimed, mut unread) = (0, 0, 0);
    for citizen in &roster.citizens {
        let progress = citizen.key().and_then(|key| {
            match net::call_once(&config, Some(&key), &RestRequest::Tutorial).map_err(|e| e.to_string())? {
                RestResponse::Tutorial(progress) => Ok(metrics::RoadInReport::of(&progress)),
                other => Err(format!("the server answered {other:?}")),
            }
        });
        let home = citizen.persona.home.zone.as_str();
        match progress {
            Ok(road) => {
                walked += usize::from(road.finished());
                claimed += usize::from(road.grant == capitol_core::protocol::tutorial::Grant::Spent);
                println!(
                    "{:<20} {:<17} {:<9} {:>7}  {:<13} {:<8} {}",
                    citizen.name,
                    citizen.role.to_string(),
                    home,
                    format!("{}/{}", road.done, road.of),
                    road.chapter.as_deref().unwrap_or("walked"),
                    format!("{:?}", road.grant).to_lowercase(),
                    road.next.as_deref().unwrap_or("–"),
                );
            }
            Err(e) => {
                unread += 1;
                println!("{:<20} {:<17} {:<9} couldn't ask: {e}", citizen.name, citizen.role.to_string(), home);
            }
        }
    }
    let n = roster.citizens.len();
    println!("{walked} of {n} have walked the whole road; {claimed} have claimed their deed");
    if unread > 0 {
        return Err(format!("{unread} of {n} couldn't be asked"));
    }
    Ok(())
}

fn status(options: &Options) -> Result<(), String> {
    let path = town::status_path(&options.roster);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("reading {}: {e}. Has the town run here?", path.display()))?;
    print!("{text}");
    let age = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok());
    if let Some(age) = age {
        let secs = age.as_secs();
        if secs > 180 {
            println!("(written {} min ago: is the town still running?)", secs / 60);
        } else {
            println!("(written {secs} s ago)");
        }
    }
    Ok(())
}

/// The roster, which must have citizens to live.
fn living_roster(options: &Options) -> Result<Roster, String> {
    let roster = Roster::load(&options.roster)?;
    if roster.citizens.is_empty() {
        return Err(format!(
            "{} has no citizens: make some with `capitol-town mint --count <n>`",
            roster.path.display()
        ));
    }
    Ok(roster)
}

fn run(options: &Options) -> Result<(), String> {
    let roster = living_roster(options)?;
    let server = server(options, &roster);
    let (town, wall) = live(options, &roster, &server, None, || {})?;
    if options.report {
        let report = TownReport::build(&town.runs(), wall.as_secs_f64(), tick_scale(&server), None);
        let path = beside(&roster.path, "report", "md");
        std::fs::write(&path, report.to_markdown()).map_err(|e| format!("writing {}: {e}", path.display()))?;
        let csv = path.with_extension("activity.csv");
        std::fs::write(&csv, report.activity_csv()).map_err(|e| format!("writing {}: {e}", csv.display()))?;
        println!("capitol-town: the report is in {}", path.display());
    }
    Ok(())
}

/// A soak (C1.7): the town lives for `hours` of the day's clock while the
/// server's books are read every `--sample`, then it stops and reports.
/// `Ok(false)` when the books drifted.
fn soak(options: &Options, hours: f64) -> Result<bool, String> {
    let roster = living_roster(options)?;
    let server = server(options, &roster);
    let admin = admin(options, "soak")?;
    let config = NetConfig::with_base_url(&server);
    // The preset is set before the books are read, so the whole soak runs
    // under it (D6.3)
    let preset = match &options.tunables {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
            Some((path, Preset::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?))
        }
        None => None,
    };
    let mut tunables = admin.tunables(&config)?;
    if let Some((path, preset)) = &preset {
        let update = preset.update(&tunables);
        if !update.values.is_empty() {
            tunables = admin.set_tunables(&config, update)?;
        }
        println!("capitol-town: the knobs are {}'s", path.display());
    }
    // Before the books and before any citizen signs in, so each hears the
    // day clock in its auth_ok and the whole soak lives from that hour
    if let Some(hour) = options.start_at {
        let clock = admin.set_clock(&config, hour)?;
        println!("capitol-town: the day's clock reads {} (#427)", schedule::clock(clock.time_of_day));
    }
    let knobs = balance::off_default(&tunables);
    for (key, value) in &knobs {
        println!("    {key} = {value}");
    }
    let start = admin.ledger(&config)?;
    // The period is the tick over the scale, as a float: rounded back to
    // what was set, 30 and not 30.000003
    let scale = (capitol_core::protocol::rules::TICK_MS as f64 / start.load.tick_period_ms * 100.0).round() / 100.0;
    let length = schedule::real(hours / 24.0);
    println!(
        "capitol-town: a soak of {hours} hours of the day's clock ({:.0} real minutes) at {scale}×, from books of {} accounts, {} Strands and {} units",
        length.as_secs_f64() / 60.0,
        start.accounts.len(),
        start.accounts.iter().map(|a| a.strands).sum::<i64>() + start.escrow_strands,
        start.units.values().sum::<i64>()
    );
    // Every reading goes to disk as it's read, so a soak that's killed
    // still leaves its books for `capitol-town books` to reconcile
    let md = beside(&roster.path, "soak", "md");
    let ledgers = md.with_extension("ledgers.jsonl");
    keep(&ledgers, &start)?;
    println!("capitol-town: the books, as they're read, to {}", ledgers.display());
    let mut samples = vec![start.clone()];
    let until = Instant::now() + length;
    let (town, wall) = live(options, &roster, &server, Some(until), || match admin.ledger(&config) {
        Ok(ledger) => {
            println!("── {}", load_line(samples.last().unwrap_or(&ledger), &ledger));
            if let Err(e) = keep(&ledgers, &ledger) {
                eprintln!("capitol-town: {e}");
            }
            samples.push(ledger);
        }
        Err(e) => eprintln!("capitol-town: {e}"),
    })?;
    thread::sleep(SETTLE);
    let end = admin.ledger(&config)?;
    keep(&ledgers, &end)?;
    samples.push(end.clone());
    // The cards' "lately busy with" are read with the end's books, before
    // the run's database goes (§10, #553)
    let catalog = catalog(&config);
    let cards = cards(&admin, &config, &end, &catalog);
    let report = TownReport::build(&town.runs(), wall.as_secs_f64(), scale, Some(Soak { start, end, samples }))
        .with_callings(&town.callings())
        .with_cards(&cards);
    std::fs::write(&md, report.to_markdown()).map_err(|e| format!("writing {}: {e}", md.display()))?;
    if let Some(csv) = report.load_csv() {
        let path = md.with_extension("csv");
        std::fs::write(&path, csv).map_err(|e| format!("writing {}: {e}", path.display()))?;
    }
    let path = md.with_extension("activity.csv");
    std::fs::write(&path, report.activity_csv()).map_err(|e| format!("writing {}: {e}", path.display()))?;
    let books = report.soak.as_ref().map(Soak::reconciliation);
    if let Some(books) = &books {
        println!(
            "capitol-town: Strands {} (drift {}, {} account{} unexplained); units {}",
            if books.strands.holds() { "conserved" } else { "DRIFTED" },
            books.strands.drift,
            books.strands.accounts.len(),
            if books.strands.accounts.len() == 1 { "" } else { "s" },
            if books.units.void {
                "unchecked: the server restarted".to_string()
            } else if books.units.holds() {
                "conserved".to_string()
            } else {
                format!("DRIFTED in {}", books.units.drift.iter().map(|(i, n)| format!("{i} {n:+}")).collect::<Vec<_>>().join(", "))
            }
        );
    }
    let label = preset.as_ref().map_or("the server's knobs".to_string(), |(path, _)| {
        path.file_name().map_or(path.display().to_string(), |n| n.to_string_lossy().into_owned())
    });
    if let Some(mut run) = Balance::of(&report, &label, knobs, &targets(&catalog)) {
        run.day_offset_ms = day_offset(&server);
        run.earning = town.earning();
        let path = md.with_extension("balance.json");
        let json = capitol_core::serde_json::to_string_pretty(&run).map_err(|e| format!("writing the balance: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("writing {}: {e}", path.display()))?;
        println!("capitol-town: what a balance run compares is in {}", path.display());
    }
    println!("capitol-town: the soak's report is in {}", md.display());
    Ok(report.conserved())
}

/// The server's catalog, or `None` if it can't be read
fn catalog(config: &NetConfig) -> Option<Catalog> {
    match net::call_once(config, None, &RestRequest::Catalog) {
        Ok(RestResponse::Catalog(catalog)) => Some(Catalog::new(*catalog)),
        Ok(_) | Err(_) => {
            eprintln!(
                "capitol-town: couldn't read the catalog, so prices are held up against the sim's D5 values, and the cards are read by the built-in one"
            );
            None
        }
    }
}

/// Each item's target price from the catalog (D2.3); none without one
fn targets(catalog: &Option<Catalog>) -> std::collections::BTreeMap<String, i64> {
    catalog.as_ref().map_or_else(Default::default, |catalog| {
        catalog.item_types().iter().filter_map(|t| Some((t.id.clone(), t.target_price?))).collect()
    })
}

/// What each citizen's card says it's lately been busy with, by name, as
/// the jobs it names (§10, #553): an empty list for a quiet card. A card
/// that couldn't be read is left out, and the report says it went unread.
fn cards(admin: &Admin, config: &NetConfig, books: &Ledger, catalog: &Option<Catalog>) -> std::collections::BTreeMap<String, Vec<String>> {
    let citizens: Vec<_> = books.accounts.iter().filter(|a| a.citizen).collect();
    let ids: Vec<_> = citizens.iter().map(|a| a.player_id).collect();
    let fallback = Catalog::fallback();
    let catalog = catalog.as_ref().unwrap_or(&fallback);
    match admin.cards(config, &ids) {
        Ok(cards) => citizens
            .iter()
            .zip(cards)
            .map(|(a, card)| (a.name.clone(), card.map_or_else(Vec::new, |c| measures::card_jobs(&c, catalog))))
            .collect(),
        Err(e) => {
            eprintln!("capitol-town: couldn't read the citizens' cards ({e}), so \"lately busy with\" goes unmeasured");
            Default::default()
        }
    }
}

/// Two soaks' balances side by side (D6.3), printed or written to `out`.
/// `Ok(false)` when either's books drifted.
fn compare(a: &Path, b: &Path, out: Option<&Path>) -> Result<bool, String> {
    let read = |path: &Path| -> Result<Balance, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
        capitol_core::serde_json::from_str(&text).map_err(|e| format!("{} isn't a soak's balance: {e}", path.display()))
    };
    let (a, b) = (read(a)?, read(b)?);
    let md = balance::compare(&a, &b);
    match out {
        Some(path) => {
            std::fs::write(path, &md).map_err(|e| format!("writing {}: {e}", path.display()))?;
            println!("capitol-town: the comparison is in {}", path.display());
        }
        None => print!("{md}"),
    }
    Ok(a.conserved() && b.conserved())
}

/// Appends `ledger` to a soak's books, a line of JSON.
fn keep(path: &Path, ledger: &Ledger) -> Result<(), String> {
    use std::io::Write;
    let line = capitol_core::serde_json::to_string(ledger).map_err(|e| format!("writing the books: {e}"))?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{line}"))
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Reconciles the books a soak kept, first reading against last, and prints
/// their conservation and load. `Ok(false)` when they drifted.
fn books(file: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("reading {}: {e}", file.display()))?;
    let soak = Soak::from_jsonl(&text)?.ok_or_else(|| format!("{} holds fewer than two readings", file.display()))?;
    print!("{}", soak.to_markdown());
    Ok(soak.reconciliation().holds())
}

/// The load since `before`, in a line.
fn load_line(before: &Ledger, now: &Ledger) -> String {
    let secs = (now.at_ms - before.at_ms) as f64 / 1000.0;
    let w = capitol_core::protocol::ledger::LoadWindow::between(&before.load, &now.load, secs);
    let p99 = w.tick_p99_ms.map_or("over 1 s".to_string(), |b| format!("≤ {b} ms"));
    format!(
        "load: ticks {:.2} ms mean, p99 {p99} of {:.0} ms, {} over; {} in the world, {} sockets, {} dropped; {:.0} DB transactions/s",
        w.tick_mean_ms, w.tick_period_ms, w.ticks_over, w.players_in_world, w.sockets_open, w.sockets_dropped, w.db_transactions_per_sec
    )
}

/// What the server's day clock adds to its clock (#427), from its health;
/// 0 if it won't say.
fn day_offset(server: &str) -> i64 {
    match net::call_once(&NetConfig::with_base_url(server), None, &RestRequest::Health) {
        Ok(RestResponse::Health(health)) => health.day_offset_ms,
        _ => 0,
    }
}

/// The server's `SIM_TICK_SCALE`, from its health; 1 if it won't say.
fn tick_scale(server: &str) -> f64 {
    match net::call_once(&NetConfig::with_base_url(server), None, &RestRequest::Health) {
        Ok(RestResponse::Health(health)) => health.sim_tick_scale.unwrap_or(1.0),
        _ => 1.0,
    }
}

/// Lives the town until Ctrl-C, `until`, or every citizen retires,
/// printing its health every `--every` and calling `sample` every
/// `--sample`. Returns the town, stopped, and how long it lived.
fn live(
    options: &Options,
    roster: &Roster,
    server: &str,
    until: Option<Instant>,
    mut sample: impl FnMut(),
) -> Result<(Town, Duration), String> {
    let stop = Arc::new(AtomicBool::new(false));
    on_ctrl_c(stop.clone());
    let logs = (options.log_mb > 0).then(|| Logs {
        dir: town::decisions_path(&roster.path),
        limit: options.log_mb << 20,
        keep: log::LOG_KEEP,
    });
    let hours = if options.always_on { Hours::AlwaysOn } else { Hours::Habits };
    // Those who've left the roster take down what they left first (#585)
    for line in departed::settle_departed(roster, server) {
        println!("capitol-town: {line}");
    }
    let mut town = Town::start(roster, server, stop.clone(), logs.as_ref(), hours)
        .map_err(|e| format!("starting the town: {e}"))?;
    println!(
        "capitol-town: {} citizens on {server}, health every {:.0?} to {}, and who's where to {}",
        roster.citizens.len(),
        options.every,
        town::status_path(&roster.path).display(),
        town::presence_path(&roster.path).display()
    );
    if let Some(logs) = &logs {
        println!("capitol-town: decisions to {}", logs.dir.display());
    }
    let poster = report_to_server(options, roster, server, &town);
    let status = &roster.path;
    let day_offset = day_offset(server);
    let began = Instant::now();
    let mut next = began + options.every;
    let mut next_sample = began + options.sample;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if town.is_empty() {
            println!("capitol-town: every citizen has retired");
            break;
        }
        let now = Instant::now();
        if until.is_some_and(|until| now >= until) {
            println!("capitol-town: the soak's hours are up");
            break;
        }
        if now >= next {
            report(&town, server, status, day_offset);
            next += options.every;
        }
        if until.is_some() && now >= next_sample {
            sample();
            next_sample += options.sample;
        }
        thread::sleep(Duration::from_millis(250));
    }
    stop.store(true, Ordering::Relaxed);
    println!("capitol-town: stopping; each citizen finishes its step");
    let stuck = town.wait(GRACE);
    report(&town, server, status, day_offset);
    if let Some(poster) = poster {
        poster.finish();
    }
    if stuck > 0 {
        println!("capitol-town: {stuck} still mid-step after {GRACE:?}; leaving them");
    }
    Ok((town, began.elapsed()))
}

/// The town's reports to the server's console (D4.1), each game minute, on
/// a thread of their own, so a slow server never holds the town up.
struct Poster {
    stop: mpsc::Sender<()>,
    thread: thread::JoinHandle<()>,
}

impl Poster {
    /// Sends the town's last report, saying it's stopping, and waits a
    /// moment for it to go.
    fn finish(self) {
        let _ = self.stop.send(());
        let end = Instant::now() + POST_TIMEOUT + Duration::from_secs(1);
        while !self.thread.is_finished() && Instant::now() < end {
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Starts reporting the town to `server`, if it has a key to report with.
fn report_to_server(options: &Options, roster: &Roster, server: &str, town: &Town) -> Option<Poster> {
    let mut config = NetConfig::with_base_url(server);
    config.rest_timeout = POST_TIMEOUT;
    let admin = admin(options, "reporting").ok();
    let key_path = town::report_key_path(&roster.path);
    let mut key = report_key(&key_path, admin.as_ref(), &config)?;
    let mut reporter = Reporter::new(town.view(), unix_ms());
    let (stop, stopped) = mpsc::channel::<()>();
    let every = Duration::from_millis(TOWN_REPORT_MS);
    let spawned = thread::Builder::new().name("town-report".into()).spawn(move || {
        // A report the server didn't take goes again with the next
        let mut unsent: Option<metrics::TownReport> = None;
        loop {
            let last = !matches!(stopped.recv_timeout(every), Err(mpsc::RecvTimeoutError::Timeout));
            let report = reporter.report(unix_ms(), last);
            let report = match unsent.take() {
                Some(earlier) => earlier.then(report),
                None => report,
            };
            unsent = post(&config, &mut key, admin.as_ref(), &key_path, report, unsent.is_some());
            if last {
                return;
            }
        }
    });
    match spawned {
        Ok(thread) => {
            println!("capitol-town: the town reports to {server}/admin/town/report each game minute");
            Some(Poster { stop, thread })
        }
        Err(e) => {
            eprintln!("capitol-town: can't report to the server: {e}");
            None
        }
    }
}

/// The town's report key: the one beside the roster, else, with an admin to
/// hand, a new one, kept there. `None` if it has none, and won't report.
fn report_key(path: &Path, admin: Option<&Admin>, config: &NetConfig) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let key = text.trim();
        if !key.is_empty() {
            return Some(key.to_string());
        }
    }
    let Some(admin) = admin else {
        println!(
            "capitol-town: no {}, so the server's console won't hear from the town. Run it once with \
             $CAPITOL_ADMIN_EMAIL and $CAPITOL_ADMIN_PASSWORD set and it mints one",
            path.display()
        );
        return None;
    };
    match mint_report_key(admin, config, path) {
        Ok(key) => Some(key),
        Err(e) => {
            eprintln!("capitol-town: {e}; the town won't report to the server");
            None
        }
    }
}

/// A new report key from `admin`, kept at `path`.
fn mint_report_key(admin: &Admin, config: &NetConfig, path: &Path) -> Result<String, String> {
    let key = admin.town_key(config)?;
    std::fs::write(path, format!("{key}\n")).map_err(|e| format!("keeping the town's key in {}: {e}", path.display()))?;
    println!("capitol-town: minted the town's report key, kept in {}", path.display());
    Ok(key)
}

/// Posts `report`. A key the server no longer knows (revoked, or the town's
/// database made anew) is replaced, with an admin to hand, and the report
/// sent again. Returns the report if the server didn't answer, or failed, to
/// go again with the next; one it refused is dropped. A failure is said
/// once (`failing` says the last post failed), until a report gets through.
fn post(
    config: &NetConfig,
    key: &mut String,
    admin: Option<&Admin>,
    key_path: &Path,
    report: metrics::TownReport,
    failing: bool,
) -> Option<metrics::TownReport> {
    let request = RestRequest::TownReport(Box::new(report));
    let mut result = net::call_once(config, Some(key), &request);
    if let (Err(e), Some(admin)) = (&result, admin) {
        if e.is_unauthorized() {
            match mint_report_key(admin, config, key_path) {
                Ok(new) => {
                    *key = new;
                    result = net::call_once(config, Some(key), &request);
                }
                Err(e) => eprintln!("capitol-town: {e}"),
            }
        }
    }
    let error = match result {
        Ok(_) => {
            if failing {
                println!("capitol-town: the server takes the town's reports again");
            }
            return None;
        }
        Err(e) => e,
    };
    if !failing {
        eprintln!("capitol-town: the server didn't take the town's report: {error}");
    }
    let RestRequest::TownReport(report) = request else { unreachable!("it was made a report") };
    error.status().is_none_or(|status| status >= 500).then_some(*report)
}

/// Replays `logs` (the roster's decision logs if none) with the stack
/// `layers`, and prints the report (C3.1).
fn replay(options: &Options, logs: &[PathBuf], layers: &[String], stride: usize, in_flight: usize) -> Result<(), String> {
    let logs = match logs {
        [] => vec![town::decisions_path(&options.roster)],
        logs => logs.to_vec(),
    };
    let files = log::files(&logs).map_err(|e| format!("finding the logs: {e}"))?;
    if files.is_empty() {
        return Err(format!("no decision logs in {}", logs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")));
    }
    // check_layers passed them when they were parsed
    let names = layers.to_vec();
    let mut replayer = Replayer::new(move || Cascade::from_names(&names).expect("checked layers"))
        .stride(stride)
        .in_flight(in_flight);
    for file in &files {
        let records = log::read(file).map_err(|e| format!("reading {}: {e}", file.display()))?;
        replayer.log(&file.display().to_string(), records);
    }
    print!("{}", replayer.finish());
    Ok(())
}

/// Writes the made-up town's log (C3.3) to `dir`, a fresh file each time.
fn corpus(options: &Options, dir: Option<&Path>, rounds: u32) -> Result<(), String> {
    let dir = dir.map_or_else(|| town::corpus_path(&options.roster), Path::to_path_buf);
    let path = log::file_for(&dir, corpus::CITIZEN);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("replacing {}: {e}", path.display()))?;
    }
    let situations = corpus::town(rounds);
    corpus::record(&path, &situations).map_err(|e| format!("writing {}: {e}", path.display()))?;
    println!("{}: {} decisions, as the rules made them", path.display(), situations.len());
    Ok(())
}

/// Three made-up citizens say a line for every occasion in `name`'s voice,
/// and what each took and cost (#311).
fn audition(name: &str) -> Result<(), String> {
    use capitol_core::citizen::phrases::{Moment, Occasion, Words};
    if name != voice::DEFAULT_VOICE && std::env::var("ANTHROPIC_API_KEY").map_or(true, |k| k.trim().is_empty()) {
        return Err(format!("{name} needs $ANTHROPIC_API_KEY"));
    }
    let citizens = [
        ("Wren Ashby", "gatherer-ore", 0.15, 0.8),
        ("Percy Sawyer", "crafter-planks", 0.5, 0.5),
        ("Ada Reeder", "merchant", 0.9, 0.2),
    ];
    let mut cents = 0.0;
    for (n, (citizen, role, sociability, thrift)) in citizens.into_iter().enumerate() {
        let who = decide::Who {
            name: citizen.into(),
            role: role.into(),
            temperament: decide::Temperament {
                sociability,
                thrift,
                ..Default::default()
            },
            calling: None,
        };
        println!("── {citizen}, {role}, sociability {sociability}");
        let mut speaker = voice::speaker(name, who, n as u64)?;
        let first = citizen.split_whitespace().next().unwrap_or(citizen);
        for occasion in Occasion::ALL {
            let mut moment = Moment::with("Bram Cole", uuid::Uuid::from_u128(7 + n as u128), Some(unix_ms()));
            moment.heard = match occasion {
                Occasion::Answer => Some(format!("{first}, are you selling today?")),
                Occasion::Letter => Some(format!(
                    "Dear {first},\nThank you for your help at the hall last week. Will you be there tomorrow?\nBram"
                )),
                _ => None,
            };
            let asked = Instant::now();
            let words = match speaker.line(occasion, &moment) {
                Words::Pending(ticket) => loop {
                    if let Some(words) = speaker.poll(ticket) {
                        break words;
                    }
                    if asked.elapsed() > decide::remote::TIMEOUT {
                        break Words::Nothing;
                    }
                    thread::sleep(Duration::from_millis(20));
                },
                words => words,
            };
            let took = asked.elapsed().as_secs_f64();
            match words {
                Words::Line(line) => println!("   {occasion:?} ({took:.1} s): {}", line.replace('\n', " / ")),
                _ => println!("   {occasion:?} ({took:.1} s): nothing, so the phrasebook speaks"),
            }
        }
        cents += speaker.spent();
    }
    println!("spent {cents:.4}¢ on {} lines", citizens.len() * Occasion::ALL.len());
    Ok(())
}

/// Prints the health and writes it beside the roster, appends who was
/// where in their day to the presence log (C1.2), and keeps the citizens'
/// stints (Callings K1.2).
fn report(town: &Town, server: &str, roster: &Path, day_offset_ms: i64) {
    let now_ms = unix_ms();
    let game = schedule::clock(capitol_core::protocol::rules::time_of_day(now_ms + day_offset_ms));
    let header = format!("capitol-town · {server} · {} · game {game} · {}", now_utc(), town.tally());
    let lines = town.health_lines();
    println!("── {header}");
    for line in &lines {
        println!("   {line}");
    }
    let status = town::status_path(roster);
    let text = format!("{header}\n{}\n", lines.join("\n"));
    if let Err(e) = std::fs::write(&status, text) {
        eprintln!("capitol-town: writing {}: {e}", status.display());
    }
    let presence = town::presence_path(roster);
    if let Err(e) = append_presence(&presence, &town.presence(now_ms, day_offset_ms)) {
        eprintln!("capitol-town: writing {}: {e}", presence.display());
    }
    // Each citizen's stint and what it did lately, for its next session
    if let Err(e) = town.save_stints() {
        eprintln!("capitol-town: writing {}: {e}", town::stints_path(roster).display());
    }
}

/// Appends `rows` to the presence log, with its header if it's new.
fn append_presence(path: &Path, rows: &[String]) -> std::io::Result<()> {
    use std::io::Write;
    let new = !path.exists();
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let mut text = String::new();
    if new {
        text.push_str(town::PRESENCE_HEADER);
        text.push('\n');
    }
    for row in rows {
        text.push_str(row);
        text.push('\n');
    }
    file.write_all(text.as_bytes())
}

/// A file beside the roster, stamped with the time: `town.soak-20261020-153000.md`.
fn beside(roster: &Path, what: &str, extension: &str) -> PathBuf {
    let stamp = capitol_core::chrono::DateTime::from_timestamp(unix_ms() / 1000, 0)
        .unwrap_or_default()
        .format("%Y%m%d-%H%M%S");
    roster.with_extension(format!("{what}-{stamp}.{extension}"))
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The time now, as `2026-10-20 14:02:31 UTC`.
fn now_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    capitol_core::chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .format("%Y-%m-%d %H:%M:%S UTC")
        .to_string()
}

/// The first Ctrl-C sets `stop`; a second ends the process at once.
fn on_ctrl_c(stop: Arc<AtomicBool>) {
    let spawned = thread::Builder::new().name("ctrl-c".into()).spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
            eprintln!("capitol-town: can't listen for Ctrl-C; stop it by closing it");
            return;
        };
        runtime.block_on(async {
            while tokio::signal::ctrl_c().await.is_ok() {
                if stop.swap(true, Ordering::Relaxed) {
                    eprintln!("capitol-town: stopped at once");
                    std::process::exit(130);
                }
            }
        });
    });
    if let Err(e) = spawned {
        eprintln!("capitol-town: can't listen for Ctrl-C ({e}); stop it by closing it");
    }
}

fn main() -> ExitCode {
    let options = match parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("capitol-town: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match options.action {
        Action::Run => run(&options),
        Action::Soak { hours } => match soak(&options, hours) {
            Ok(true) => Ok(()),
            Ok(false) => Err("the soak's books drifted".to_string()),
            Err(e) => Err(e),
        },
        Action::Books { ref file } => match books(file) {
            Ok(true) => Ok(()),
            Ok(false) => Err("the books drifted".to_string()),
            Err(e) => Err(e),
        },
        Action::Compare { ref a, ref b, ref out } => match compare(a, b, out.as_deref()) {
            Ok(true) => Ok(()),
            Ok(false) => Err("a run's books drifted".to_string()),
            Err(e) => Err(e),
        },
        Action::Mint { size } => mint(&options, size),
        Action::Habits => habits(&options),
        Action::RoadIn => road_in(&options),
        Action::Status => status(&options),
        Action::Replay {
            ref logs,
            ref layers,
            stride,
            in_flight,
        } => replay(&options, logs, layers, stride, in_flight),
        Action::Corpus { ref dir, rounds } => corpus(&options, dir.as_deref(), rounds),
        Action::Voice { ref name } => audition(name),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("capitol-town: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn actions_and_flags_parse() {
        let o = parse(args("")).unwrap().unwrap();
        assert_eq!(o.action, Action::Run);
        assert_eq!(o.roster, PathBuf::from("town.toml"));
        assert!(!o.report);
        let o = parse(args("mint --count 20 --admin chris@example.com --roster t/x.toml"))
            .unwrap()
            .unwrap();
        assert_eq!(o.action, Action::Mint { size: Size::Count(20) });
        assert_eq!(o.role, None);
        assert_eq!(o.admin.as_deref(), Some("chris@example.com"));
        assert_eq!(o.roster, PathBuf::from("t/x.toml"));
        let o = parse(args("mint --count 2 --role merchant")).unwrap().unwrap();
        assert_eq!(o.role, Some(Role::Merchant));
        assert_eq!(o.calling, None);
        let o = parse(args("mint --count 2 --role builder --calling all-rounder")).unwrap().unwrap();
        assert_eq!((o.role, o.calling), (Some(Role::Builder), Some(Style::AllRounder)));
        assert!(parse(args("mint --count 2 --calling hermit")).unwrap_err().contains("specialist, leaner or all-rounder"));
        assert!(parse(args("run --calling leaner")).is_err(), "--calling without mint");
        let o = parse(args("mint --online 18")).unwrap().unwrap();
        assert_eq!(o.action, Action::Mint { size: Size::Online(18.0) });
        assert_eq!(parse(args("habits")).unwrap().unwrap().action, Action::Habits);
        assert_eq!(parse(args("road-in --roster t/x.toml")).unwrap().unwrap().action, Action::RoadIn);
        assert!(!parse(args("run")).unwrap().unwrap().always_on);
        assert!(parse(args("run --always-on")).unwrap().unwrap().always_on);
        assert!(parse(args("soak --hours 2 --always-on")).unwrap().unwrap().always_on);
        // A soak's day clock, from an hour or a time of day (#427)
        assert_eq!(parse(args("soak --hours 2 --start-at 9")).unwrap().unwrap().start_at, Some(9.0));
        assert_eq!(parse(args("soak --hours 2 --start-at 13:30")).unwrap().unwrap().start_at, Some(13.5));
        assert!(parse(args("soak --hours 2 --start-at 25")).is_err());
        assert!(parse(args("soak --hours 2 --start-at 9:75")).is_err());
        assert!(parse(args("run --start-at 9")).is_err(), "--start-at goes with soak");
        assert_eq!(parse(args("--status")).unwrap().unwrap().action, Action::Status);
        assert_eq!(parse(args("status")).unwrap().unwrap().action, Action::Status);
        let o = parse(args("run --every 5 --server http://x:3001")).unwrap().unwrap();
        assert_eq!(o.every, Duration::from_secs(5));
        assert_eq!(o.server.as_deref(), Some("http://x:3001"));
        assert!(parse(args("--help")).unwrap().is_none());
        assert_eq!(parse(args("run --log-mb 0")).unwrap().unwrap().log_mb, 0);
        assert_eq!(parse(args("")).unwrap().unwrap().log_mb, LOG_MB);
        assert!(parse(args("--report")).unwrap().unwrap().report);
    }

    #[test]
    fn a_soak_takes_its_hours_and_its_sample() {
        let o = parse(args("soak --hours 24 --log-mb 1 --admin a@b --every 30")).unwrap().unwrap();
        assert_eq!(o.action, Action::Soak { hours: 24.0 });
        assert_eq!((o.log_mb, o.sample), (1, Duration::from_secs(60)));
        assert_eq!(o.admin.as_deref(), Some("a@b"));
        assert_eq!(parse(args("soak --hours 0.5 --sample 10")).unwrap().unwrap().sample, Duration::from_secs(10));
        assert!(parse(args("soak")).unwrap_err().contains("--hours"));
        assert!(parse(args("soak --hours 0")).is_err());
        assert!(parse(args("run --hours 2")).is_err(), "--hours without soak");
        assert!(parse(args("run --sample 30")).is_err(), "--sample without soak");
        assert!(parse(args("soak --hours 2 --sample 1")).is_err());
        assert!(parse(args("soak --hours 2 --report")).is_err(), "a soak always reports");
        assert!(parse(args("soak mint --hours 2")).is_err());
    }

    #[test]
    fn a_soak_takes_a_preset_and_compare_two_runs() {
        let o = parse(args("soak --hours 2 --tunables tax-high.toml")).unwrap().unwrap();
        assert_eq!(o.tunables, Some(PathBuf::from("tax-high.toml")));
        assert!(parse(args("run --tunables tax-high.toml")).unwrap_err().contains("--tunables goes with soak"));
        let o = parse(args("compare a.balance.json b.balance.json --out c.md")).unwrap().unwrap();
        assert_eq!(
            o.action,
            Action::Compare {
                a: "a.balance.json".into(),
                b: "b.balance.json".into(),
                out: Some("c.md".into())
            }
        );
        assert!(parse(args("compare a.balance.json")).unwrap_err().contains("two"));
        assert!(parse(args("compare a b c")).is_err(), "a third file");
        assert!(parse(args("soak --hours 2 --out c.md")).unwrap_err().contains("--out goes with compare"));
    }

    #[test]
    fn books_take_a_file() {
        let o = parse(args("books town.soak-1.ledgers.jsonl")).unwrap().unwrap();
        assert_eq!(o.action, Action::Books { file: "town.soak-1.ledgers.jsonl".into() });
        assert!(parse(args("books")).unwrap_err().contains("file"));
        assert!(parse(args("books a.jsonl b.jsonl")).is_err(), "one file");
    }

    #[test]
    fn replay_takes_logs_and_a_stack() {
        let o = parse(args("replay")).unwrap().unwrap();
        assert_eq!(
            o.action,
            Action::Replay {
                logs: Vec::new(),
                layers: vec!["rules".into()],
                stride: 1,
                in_flight: IN_FLIGHT
            }
        );
        let o = parse(args("replay a.jsonl town.decisions --layers rules")).unwrap().unwrap();
        assert_eq!(
            o.action,
            Action::Replay {
                logs: vec!["a.jsonl".into(), "town.decisions".into()],
                layers: vec!["rules".into()],
                stride: 1,
                in_flight: IN_FLIGHT
            }
        );
        let o = parse(args("replay town.corpus --layers rules,jev@0.8 --stride 5")).unwrap().unwrap();
        assert_eq!(
            o.action,
            Action::Replay {
                logs: vec!["town.corpus".into()],
                layers: vec!["rules".into(), "jev@0.8".into()],
                stride: 5,
                in_flight: IN_FLIGHT
            }
        );
        let o = parse(args("replay --layers rules,jev-router --in-flight 32")).unwrap().unwrap();
        assert!(matches!(o.action, Action::Replay { in_flight: 32, .. }), "{:?}", o.action);
        assert!(parse(args("replay --layers jev@2")).is_err(), "a threshold over 1");
        assert!(parse(args("replay --stride 0")).is_err());
        assert!(parse(args("--stride 3")).is_err(), "--stride without replay");
        assert!(parse(args("replay --in-flight 0")).is_err());
        assert!(parse(args("--in-flight 4")).is_err(), "--in-flight without replay");
        assert_eq!(parse(args("corpus")).unwrap().unwrap().action, Action::Corpus { dir: None, rounds: 1 });
        assert_eq!(
            parse(args("voice")).unwrap().unwrap().action,
            Action::Voice { name: "claude-haiku".into() }
        );
        assert_eq!(
            parse(args("voice --voice phrasebook")).unwrap().unwrap().action,
            Action::Voice { name: "phrasebook".into() }
        );
        assert!(parse(args("voice --voice oracle")).unwrap_err().contains("is no voice"));
        assert!(parse(args("run --voice claude-haiku")).unwrap_err().contains("goes with voice"));
        assert_eq!(
            parse(args("corpus here --rounds 5")).unwrap().unwrap().action,
            Action::Corpus {
                dir: Some("here".into()),
                rounds: 5
            }
        );
        assert!(parse(args("replay --rounds 5")).is_err(), "--rounds without corpus");
        assert!(parse(args("replay --layers rules,oracle")).unwrap_err().contains("oracle"));
        assert!(parse(args("--layers rules")).is_err(), "--layers without replay");
        assert!(parse(args("a.jsonl replay")).is_err(), "a log before replay");
        assert!(parse(args("replay --log-mb 2")).is_err());
    }

    #[test]
    fn bad_arguments_are_reported() {
        assert!(parse(args("mint")).is_err(), "mint without --count");
        assert!(parse(args("mint --count 2 --online 3")).is_err(), "both");
        assert!(parse(args("mint --online 0")).is_err());
        assert!(parse(args("run --online 3")).is_err(), "--online without mint");
        assert!(parse(args("habits --always-on")).is_err(), "--always-on without run or soak");
        assert!(parse(args("mint --count 0")).is_err());
        assert!(parse(args("--count 3")).is_err(), "--count without mint");
        assert!(parse(args("run --admin a@b")).is_err());
        assert!(parse(args("mint status --count 2")).is_err());
        assert!(parse(args("mint --count 2 --role poet")).is_err());
        assert!(parse(args("run --role merchant")).is_err(), "--role without mint");
        assert!(parse(args("status --every 5")).is_err());
        assert!(parse(args("status --report")).is_err());
        assert!(parse(args("--every 0")).is_err());
        assert!(parse(args("--fly")).is_err());
    }
}
