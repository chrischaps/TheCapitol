//! `capitol-town` (C1.1): a roster of citizens, each living in the world on
//! its own thread, kept alive by a supervisor.
//!
//! - **The roster** is `town.toml`: a name, a [`Role`] and a key (or a key
//!   file) for each citizen, and its [`Persona`] (C1.2). It holds keys, so
//!   it's git-ignored; `town.example.toml` shows the shape. [`mint`] adds
//!   to it.
//! - **A session** (#276) is one life: the citizen signs in by its key when
//!   its [`Habit`] says, on the real clock, and plays the role's loop
//!   ([`sim::run::live`]) through its [`Schedule`]'s day (C1.2): work, its
//!   market hours, home at dusk, and the night's work from home. When the
//!   session's over it logs out where it stands, and is away until the
//!   next. `--always-on` ([`Hours::AlwaysOn`]) keeps every citizen on, for
//!   a soak of the whole pool. A dropped connection or a server restart is
//!   ridden out inside the life (C0.6). A life that ends any other way (the
//!   server wasn't up to sign in to, a panic) starts again after the net
//!   layer's backoff. A refused key retires the citizen.
//! - **Health**: each citizen's [`Health`], one line apiece, which the town
//!   prints and writes beside the roster every minute, and its presence,
//!   appended to `town.presence.csv` (C1.2).
//! - **Stints** (Callings K1.2): the job each citizen is at, how long it's
//!   run and what it has done lately ([`Kept`]), saved with the health to
//!   `town.stints.json` beside the roster and read back as the town starts,
//!   so a casual citizen's half-hour sessions still add up to a mix. The
//!   roster itself is never rewritten.
//! - **Decisions**: each citizen's decision log (C3.1,
//!   [`decide::log`](super::decide::log)), one JSONL file apiece in
//!   `town.decisions/` beside the roster, kept across its lives.
//! - **The ledger** (C1.7): each citizen's [`Book`], what it made, traded,
//!   spent its time on and got stuck on, as the sim's report entries, and how
//!   its questions were settled. [`Town::runs`] hands them to the report
//!   ([`super::report`]).
//! - **The server's console** (D4.1): each citizen's health and its mind's
//!   [`Pulse`](super::decide::pulse::Pulse), which a [`Reporter`] makes into
//!   a `TownReport` each game minute, for `capitol-town` to post with the
//!   town's key ([`report_key_path`]).
//!
//! [`Schedule`]: super::schedule::Schedule
//! [`Habit`]: super::habit::Habit

use std::collections::BTreeMap;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use capitol_protocol::ledger::Ledger;
use capitol_protocol::metrics::{
    CallingReport, CitizenReport, DecisionBrief, RoadInReport, StintReport, TownFailure, TownReport, TOWN_FAILURES,
};
use capitol_protocol::rest::{ClockResponse, CreateCitizenRequest, LoginRequest, SetClockRequest};
use capitol_protocol::rules::lately::Lately;
use capitol_protocol::rules::{self, Zone};
use capitol_protocol::tunables::{TunablesResponse, TunablesUpdate};
use serde::Deserialize;
use uuid::Uuid;

use super::decide::log::{self, Log};
use super::decide::pulse::{Pulse, SharedPulse};
use super::decide::{self, Cascade, Mind, SharedLog};
use super::calling::{self, Calling, Style, NO_FAVOURITE};
use super::habit::{Habit, Kind, Mix, Presence};
use super::persona::{self, Avenue, Home, Market, Persona, Temperament};
use super::schedule::{self, Phase, Schedule};
use super::voice;
use crate::bot::{Bot, BotError};
use crate::net::{self, NetConfig, RestRequest, RestResponse};
use crate::sim::report::{BotRun, Entry};
use crate::sim::patron;
use crate::sim::stint::{self, Kept};
use crate::sim::wage;
use crate::sim::{self, Activity, Role};

/// The roster, beside wherever the town is run from.
pub const ROSTER: &str = "town.toml";
/// Between one citizen's start and the next, so a town doesn't arrive all
/// at once.
pub const STAGGER: Duration = Duration::from_millis(500);
/// The first wait before a citizen whose life ended starts again. It doubles
/// with each start that fails soon after, up to [`RESTART_MAX`].
const RESTART_FIRST: Duration = Duration::from_secs(2);
const RESTART_MAX: Duration = Duration::from_secs(60);
/// A life this long resets the backoff: it was going fine.
const LIVED_WELL: Duration = Duration::from_secs(600);
/// How often a waiting citizen looks to see if the town has stopped.
const STOP_CHECK: Duration = Duration::from_millis(250);
/// Names a mint tries for each citizen before it gives up on one.
const NAME_TRIES: u64 = 24;

// ---------------------------------------------------------------------------
// The roster
// ---------------------------------------------------------------------------

/// `town.toml`, as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RosterFile {
    server: Option<String>,
    /// The decision layers every citizen has, bottom first (C1.3).
    layers: Option<Vec<String>>,
    /// Who finds every citizen's words (#311): `phrasebook` unless it says.
    voice: Option<String>,
    /// When citizens play (#276): the town's mix of habits.
    habits: Option<HabitsFile>,
    /// Who citizens are, as work goes (Callings K0.2): the town's mix of
    /// styles.
    callings: Option<CallingsFile>,
    /// When citizens hire (Callings K2.4): `line = 600`, the Strands a
    /// middling citizen keeps back before it offers work.
    hiring: Option<HiringFile>,
    /// When citizens trade more (Callings K2.2): `line = 1200`, the Strands
    /// past which a middling citizen's calling leans further to trading.
    trading: Option<TradingFile>,
    /// When citizens post for their homes (Callings K2.3): `line = 1500`,
    /// the Strands a middling citizen keeps back before it does.
    patrons: Option<PatronsFile>,
    #[serde(default, rename = "citizen")]
    citizens: Vec<CitizenFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HiringFile {
    line: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TradingFile {
    line: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatronsFile {
    line: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HabitsFile {
    mix: Option<MixFile>,
}

/// Each kind's weight: `{ dedicated = 0.05, regular = 0.3, casual = 0.55, lapsed = 0.1 }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MixFile {
    dedicated: f64,
    regular: f64,
    casual: f64,
    lapsed: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallingsFile {
    mix: Option<CallingMixFile>,
}

/// Each style's weight: `{ specialist = 0.4, leaner = 0.4, all-rounder = 0.2 }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallingMixFile {
    specialist: f64,
    leaner: f64,
    #[serde(rename = "all-rounder")]
    all_rounder: f64,
}

/// `{ favourite = "builder", focus = 0.71, wander = 0.4, leanings = {
/// crafter-planks = 0.12, ... } }`: its favourite a role, or `"none"` for an
/// all-rounder, whose focus is 0.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallingFile {
    favourite: String,
    focus: f32,
    wander: f32,
    leanings: BTreeMap<String, f32>,
}

/// `{ kind = "casual", zone = -5 }`: the zone in hours from UTC.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HabitFile {
    kind: String,
    zone: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CitizenFile {
    name: String,
    role: String,
    key: Option<String>,
    key_file: Option<PathBuf>,
    /// This citizen's layers, in place of the town's.
    layers: Option<Vec<String>>,
    /// This citizen's voice, in place of the town's.
    voice: Option<String>,
    // The persona (C1.2). Any part left out is the one the name seeds.
    home: Option<HomeFile>,
    market: Option<String>,
    schedule: Option<ScheduleFile>,
    temperament: Option<TemperamentFile>,
    habit: Option<HabitFile>,
    /// Its calling (K0.2). A roster from before callings has none, and
    /// draws one from the name, by the town's mix.
    calling: Option<CallingFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HomeFile {
    zone: String,
    avenue: String,
    depth: f64,
}

/// Game clocks: `"05:30"`, and the market's `"14:05-15:50"`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleFile {
    wake: String,
    market: String,
    home: String,
    bed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TemperamentFile {
    thrift: f32,
    risk: f32,
    sociability: f32,
    patience: f32,
}

impl CitizenFile {
    /// The persona the file gives, the name's seed filling what it leaves
    /// out, its habit's kind by the town's `mix` and its calling's style by
    /// its `callings`.
    fn persona(&self, role: Role, mix: &Mix, callings: &calling::Mix) -> Result<Persona, String> {
        let mut persona = Persona::seeded_in(role, seed(&self.name), mix, callings);
        if let Some(home) = &self.home {
            persona.home = Home {
                zone: Zone::from_str(&home.zone).ok_or_else(|| format!("has a home in {:?}, not a zone", home.zone))?,
                avenue: Avenue::from_str(&home.avenue).ok_or_else(|| {
                    format!("lives off the {:?} avenue; an avenue is a compass point, as \"south-west\"", home.avenue)
                })?,
                depth: home.depth,
            };
            persona.home.check()?;
        }
        if let Some(market) = &self.market {
            persona.market = Market::from_str(market)
                .ok_or_else(|| format!("keeps the {market:?} market; a market is \"hall\" or \"bazaar\""))?;
        }
        if let Some(day) = &self.schedule {
            let at = |what: &str, text: &str| {
                schedule::parse_clock(text).ok_or_else(|| format!("has {what} {text:?}; a time is a game clock, as \"05:30\""))
            };
            let (market_open, market_close) = schedule::parse_hours(&day.market)
                .ok_or_else(|| format!("has market {:?}; market hours are as \"14:00-15:30\"", day.market))?;
            persona.schedule = Schedule {
                wake: at("wake", &day.wake)?,
                market_open,
                market_close,
                home: at("home", &day.home)?,
                bed: at("bed", &day.bed)?,
            };
            persona.schedule.check()?;
        }
        if let Some(t) = &self.temperament {
            persona.temperament = Temperament {
                thrift: t.thrift,
                risk: t.risk,
                sociability: t.sociability,
                patience: t.patience,
            };
            persona.temperament.check()?;
        }
        if let Some(habit) = &self.habit {
            let kind = Kind::from_str(&habit.kind).ok_or_else(|| {
                format!("has the habit {:?}; a habit is dedicated, regular, casual or lapsed", habit.kind)
            })?;
            persona.habit = Habit::new(kind, habit.zone, seed(&self.name));
            persona.habit.check()?;
        }
        if let Some(c) = &self.calling {
            let job = |slug: &str| {
                Role::from_slug(slug).ok_or_else(|| {
                    let known: Vec<&str> = sim::ROLES.iter().map(|r| r.slug()).collect();
                    format!("has a calling naming {slug:?}; a job is one of {}", known.join(", "))
                })
            };
            let favourite = match c.favourite.as_str() {
                NO_FAVOURITE => None,
                slug => Some(job(slug)?),
            };
            let mut leanings = c.leanings.iter().map(|(slug, w)| Ok((job(slug)?, *w))).collect::<Result<Vec<_>, String>>()?;
            leanings.sort_by_key(|(r, _)| sim::ROLES.iter().position(|o| o == r));
            persona.calling = Calling {
                favourite,
                focus: c.focus,
                leanings,
                wander: c.wander,
            };
            persona.calling.check(role).map_err(|e| format!("has a calling that won't do: it {e}"))?;
        }
        Ok(persona)
    }
}

/// The town's citizens, and the server they live on.
#[derive(Debug, Clone, PartialEq)]
pub struct Roster {
    pub path: PathBuf,
    /// The server named in the roster, if it names one.
    pub server: Option<String>,
    /// The town's decision layers, bottom first: `["rules"]` unless it says.
    pub layers: Vec<String>,
    /// Who finds the town's words (#311): `phrasebook` unless it says.
    pub voice: String,
    /// The town's mix of habits (#276), for the citizens it seeds.
    pub mix: Mix,
    /// The town's mix of callings' styles (K0.2), for the citizens it seeds.
    pub callings: calling::Mix,
    /// The Strands a middling citizen keeps back before it hires (K2.4):
    /// `[hiring] line`, [`wage::HIRE_LINE`] unless it says.
    pub hire_line: i64,
    /// The Strands past which a middling citizen leans further to trading
    /// (K2.2): `[trading] line`, [`stint::TRADE_LINE`] unless it says.
    pub trade_line: i64,
    /// The Strands a middling citizen keeps back before it posts for its
    /// home (K2.3): `[patrons] line`, [`patron::PATRON_LINE`] unless it says.
    pub patron_line: i64,
    pub citizens: Vec<Citizen>,
}

/// One citizen on the roster.
#[derive(Debug, Clone, PartialEq)]
pub struct Citizen {
    pub name: String,
    /// Its birth trade: it gave the citizen its name and its home, and it's
    /// its calling's favourite unless it's an all-rounder (K0.2).
    pub role: Role,
    /// Its decision layers (C1.3): its own, or the town's.
    pub layers: Vec<String>,
    /// Its voice (#311): its own, or the town's.
    pub voice: String,
    pub persona: Persona,
    /// The town's line for hiring (K2.4), which its thrift weighs.
    pub hire_line: i64,
    /// The town's line for trading (K2.2), which its patience weighs.
    pub trade_line: i64,
    /// The town's line for posting for its home (K2.3), which its thrift
    /// weighs.
    pub patron_line: i64,
    key: Key,
}

/// Where a citizen's key is.
#[derive(Debug, Clone, PartialEq)]
enum Key {
    Inline(String),
    /// Relative paths are from the roster's directory.
    File(PathBuf),
}

impl Citizen {
    /// The key's secret. A key file is read each time the citizen starts,
    /// so a new key in it is taken up at the next restart.
    pub fn key(&self) -> Result<String, String> {
        match &self.key {
            Key::Inline(secret) => Ok(secret.clone()),
            Key::File(path) => std::fs::read_to_string(path)
                .map(|s| s.trim().to_string())
                .map_err(|e| format!("reading {}: {e}", path.display())),
        }
    }
}

impl Roster {
    /// Reads the roster at `path`.
    pub fn load(path: &Path) -> Result<Roster, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
        Roster::parse(&text, path)
    }

    /// The roster at `path`, or an empty one if there's no file yet.
    pub fn load_or_new(path: &Path) -> Result<Roster, String> {
        if path.exists() {
            Roster::load(path)
        } else {
            Ok(Roster {
                path: path.to_path_buf(),
                server: None,
                layers: default_layers(),
                voice: voice::DEFAULT_VOICE.to_string(),
                mix: Mix::default(),
                callings: calling::Mix::default(),
                hire_line: wage::HIRE_LINE,
                trade_line: stint::TRADE_LINE,
                patron_line: patron::PATRON_LINE,
                citizens: Vec::new(),
            })
        }
    }

    /// Parses a roster read from `path`.
    pub fn parse(text: &str, path: &Path) -> Result<Roster, String> {
        let file: RosterFile = toml::from_str(text).map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let layers = file.layers.unwrap_or_else(default_layers);
        decide::check_layers(&layers).map_err(|e| format!("{}: layers: {e}", path.display()))?;
        let town_voice = file.voice.unwrap_or_else(|| voice::DEFAULT_VOICE.to_string());
        voice::check_voice(&town_voice).map_err(|e| format!("{}: voice: {e}", path.display()))?;
        let mix = match file.habits.and_then(|h| h.mix) {
            Some(m) => Mix {
                dedicated: m.dedicated,
                regular: m.regular,
                casual: m.casual,
                lapsed: m.lapsed,
            },
            None => Mix::default(),
        };
        mix.check().map_err(|e| format!("{}: habits: {e}", path.display()))?;
        let callings = match file.callings.and_then(|c| c.mix) {
            Some(m) => calling::Mix {
                specialist: m.specialist,
                leaner: m.leaner,
                all_rounder: m.all_rounder,
            },
            None => calling::Mix::default(),
        };
        callings.check().map_err(|e| format!("{}: callings: {e}", path.display()))?;
        let hire_line = file.hiring.and_then(|h| h.line).unwrap_or(wage::HIRE_LINE);
        if hire_line < 0 {
            return Err(format!("{}: hiring: line is {hire_line}; it must be 0 or more", path.display()));
        }
        let trade_line = file.trading.and_then(|t| t.line).unwrap_or(stint::TRADE_LINE);
        if trade_line < 0 {
            return Err(format!("{}: trading: line is {trade_line}; it must be 0 or more", path.display()));
        }
        let patron_line = file.patrons.and_then(|p| p.line).unwrap_or(patron::PATRON_LINE);
        if patron_line < 0 {
            return Err(format!("{}: patrons: line is {patron_line}; it must be 0 or more", path.display()));
        }
        let mut citizens = Vec::with_capacity(file.citizens.len());
        for c in file.citizens {
            let what = |problem: &str| format!("{}: citizen {:?} {problem}", path.display(), c.name);
            let role = Role::from_slug(&c.role).ok_or_else(|| {
                let known: Vec<&str> = sim::ROLES.iter().map(|r| r.slug()).collect();
                what(&format!("has role {:?}; a role is one of {}", c.role, known.join(", ")))
            })?;
            let persona = c.persona(role, &mix, &callings).map_err(|e| what(&e))?;
            let key = match (c.key, c.key_file) {
                (Some(secret), None) => Key::Inline(secret),
                (None, Some(file)) => Key::File(dir.join(file)),
                _ => return Err(what("needs a key or a key_file, and only one")),
            };
            if citizens.iter().any(|other: &Citizen| other.name.eq_ignore_ascii_case(&c.name)) {
                return Err(what("is on the roster twice"));
            }
            let own = c.layers.unwrap_or_else(|| layers.clone());
            decide::check_layers(&own).map_err(|e| what(&format!("has layers that won't do: {e}")))?;
            let own_voice = c.voice.unwrap_or_else(|| town_voice.clone());
            voice::check_voice(&own_voice).map_err(|e| what(&format!("has a voice that won't do: {e}")))?;
            citizens.push(Citizen {
                name: c.name,
                role,
                layers: own,
                voice: own_voice,
                persona,
                hire_line,
                trade_line,
                patron_line,
                key,
            });
        }
        Ok(Roster {
            path: path.to_path_buf(),
            server: file.server,
            layers,
            voice: town_voice,
            mix,
            callings,
            hire_line,
            trade_line,
            patron_line,
            citizens,
        })
    }

    /// Adds a citizen to the end of the roster file, with the persona its
    /// name seeds written out in full, leaving what's there (and its
    /// comments) as it was. Its calling is of `style`, or by the town's mix
    /// without one.
    fn append(&mut self, name: &str, role: Role, style: Option<Style>, secret: &str) -> std::io::Result<()> {
        use std::io::Write;
        let callings = style.map_or(self.callings, calling::Mix::only);
        let citizen = Citizen {
            name: name.to_string(),
            role,
            layers: self.layers.clone(),
            voice: self.voice.clone(),
            persona: Persona::seeded_in(role, seed(name), &self.mix, &callings),
            hire_line: self.hire_line,
            trade_line: self.trade_line,
            patron_line: self.patron_line,
            key: Key::Inline(secret.to_string()),
        };
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        file.write_all(entry(&citizen, secret).as_bytes())?;
        self.citizens.push(citizen);
        Ok(())
    }
}

fn default_layers() -> Vec<String> {
    decide::DEFAULT_LAYERS.iter().map(|l| l.to_string()).collect()
}

/// A citizen's `[[citizen]]` table, persona and all. Its layers are the
/// town's, so they're left out.
fn entry(citizen: &Citizen, secret: &str) -> String {
    let quote = |s: &str| toml::Value::String(s.to_string()).to_string();
    let Persona {
        temperament: t,
        home,
        market,
        schedule: day,
        habit,
        calling,
    } = &citizen.persona;
    let leanings: Vec<String> = calling.leanings.iter().map(|(job, w)| format!("{} = {w:?}", job.slug())).collect();
    format!(
        "\n[[citizen]]\nname = {}\nrole = {}\nkey = {}\n\
         home = {{ zone = {}, avenue = {}, depth = {:?} }}\n\
         market = {}\n\
         schedule = {{ wake = {}, market = {}, home = {}, bed = {} }}\n\
         temperament = {{ thrift = {:?}, risk = {:?}, sociability = {:?}, patience = {:?} }}\n\
         habit = {{ kind = {}, zone = {} }}\n\
         calling = {{ favourite = {}, focus = {:?}, wander = {:?}, leanings = {{ {} }} }}\n",
        quote(&citizen.name),
        quote(citizen.role.slug()),
        quote(secret),
        quote(home.zone.as_str()),
        quote(home.avenue.as_str()),
        home.depth,
        quote(market.as_str()),
        quote(&schedule::clock(day.wake)),
        quote(&day.market()),
        quote(&schedule::clock(day.home)),
        quote(&schedule::clock(day.bed)),
        t.thrift,
        t.risk,
        t.sociability,
        t.patience,
        quote(habit.kind.as_str()),
        habit.zone,
        quote(calling.favourite.map_or(NO_FAVOURITE, |r| r.slug())),
        calling.focus,
        calling.wander,
        leanings.join(", "),
    )
}

/// Where the town appends who was where in their day: beside the roster,
/// `town.presence.csv`.
pub fn presence_path(roster: &Path) -> PathBuf {
    roster.with_extension("presence.csv")
}

/// The presence log's columns. `game_time` is the server's clock as this
/// machine has it; `standing` is [`Standing`], and a citizen is in the
/// world while it's `working` or `resting`.
pub const PRESENCE_HEADER: &str = "unix_ms,game_time,name,role,phase,standing";

/// Where the town keeps its citizens' stints (Callings K1.2): beside the
/// roster, `town.stints.json`, an entry a citizen by name.
pub fn stints_path(roster: &Path) -> PathBuf {
    roster.with_extension("stints.json")
}

/// The stints kept at `path`, by name. A file that's missing or won't read
/// is no stints: each citizen asks afresh, which costs it its mix so far
/// and nothing else.
pub fn read_stints(path: &Path) -> BTreeMap<String, Kept> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("capitol-town: {} doesn't read, so every citizen starts its stints afresh: {e}", path.display());
        BTreeMap::new()
    })
}

/// Writes `stints` to `path`, by way of a file beside it, so a town stopped
/// mid-write keeps the last whole one.
pub fn write_stints(path: &Path, stints: &BTreeMap<String, Kept>) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(stints).map_err(std::io::Error::other)?;
    let part = path.with_extension("json.part");
    std::fs::write(&part, text)?;
    std::fs::rename(&part, path)
}

/// Where the town writes its health: beside the roster, `town.status`.
pub fn status_path(roster: &Path) -> PathBuf {
    roster.with_extension("status")
}

/// Where the citizens' decision logs go (C3.1): beside the roster,
/// `town.decisions/`, a file a citizen.
pub fn decisions_path(roster: &Path) -> PathBuf {
    roster.with_extension("decisions")
}

/// Where the town keeps its key for its reports to the server (D4.1):
/// beside the roster, `town.report-key`. It holds a secret, like the roster.
pub fn report_key_path(roster: &Path) -> PathBuf {
    roster.with_extension("report-key")
}

/// Where `capitol-town corpus` writes its made-up town's log (C3.3).
pub fn corpus_path(roster: &Path) -> PathBuf {
    roster.with_extension("corpus")
}

/// Where and how big the decision logs are (C3.1).
#[derive(Debug, Clone, PartialEq)]
pub struct Logs {
    pub dir: PathBuf,
    /// Bytes a log grows to before it rotates
    pub limit: u64,
    /// Rotated files kept
    pub keep: usize,
}

// ---------------------------------------------------------------------------
// Minting
// ---------------------------------------------------------------------------

/// An admin's sign-in, to mint citizens with, and to read the world's books
/// in a soak.
#[derive(Debug, Clone)]
pub struct Admin {
    pub email: String,
    pub password: String,
}

impl Admin {
    /// Signs in over REST alone, with no socket, so it takes no avatar.
    /// Returns the access token.
    pub fn sign_in(&self, config: &NetConfig) -> Result<String, String> {
        let login = RestRequest::Login(LoginRequest {
            email: self.email.clone(),
            password: self.password.clone(),
        });
        match net::call_once(config, None, &login) {
            Ok(RestResponse::Auth(auth)) => Ok(auth.token),
            Ok(other) => Err(format!("signing in as {}: unexpected {other:?}", self.email)),
            Err(e) => Err(format!("signing in as {}: {e}", self.email)),
        }
    }

    /// A new key for the town's reports (D4.1), `POST /admin/town/key`,
    /// which revokes the one before. Returns its secret.
    pub fn town_key(&self, config: &NetConfig) -> Result<String, String> {
        let token = self.sign_in(config)?;
        match net::call_once(config, Some(&token), &RestRequest::TownKey) {
            Ok(RestResponse::CreatedApiKey(created)) => Ok(created.secret),
            Ok(other) => Err(format!("minting the town's key: unexpected {other:?}")),
            Err(e) if e.status() == Some(403) => Err(format!("minting the town's key: {e}. Is {} an admin?", self.email)),
            Err(e) => Err(format!("minting the town's key: {e}")),
        }
    }

    /// The world's books now, `GET /admin/ledger` (C1.7). It signs in each
    /// time: a soak reads once a minute for hours, longer than a token lasts.
    pub fn ledger(&self, config: &NetConfig) -> Result<Ledger, String> {
        let token = self.sign_in(config)?;
        match net::call_once(config, Some(&token), &RestRequest::Ledger) {
            Ok(RestResponse::Ledger(ledger)) => Ok(*ledger),
            Ok(other) => Err(format!("reading the ledger: unexpected {other:?}")),
            Err(e) if e.status() == Some(403) => Err(format!("reading the ledger: {e}. Is {} an admin?", self.email)),
            Err(e) => Err(format!("reading the ledger: {e}")),
        }
    }

    /// Sets a dev server's day clock to read `hour` now, `PUT /admin/clock`,
    /// for a soak that starts at an hour of its own (#427). Refused unless
    /// the server was started with `DEV_CLOCK=on`.
    pub fn set_clock(&self, config: &NetConfig, hour: f64) -> Result<ClockResponse, String> {
        let token = self.sign_in(config)?;
        match net::call_once(config, Some(&token), &RestRequest::SetClock(SetClockRequest { hour })) {
            Ok(RestResponse::Clock(clock)) => Ok(clock),
            Ok(other) => Err(format!("setting the day clock: unexpected {other:?}")),
            Err(e) if e.status() == Some(403) => {
                Err(format!("setting the day clock: {e}. Is {} an admin, and the server started with DEV_CLOCK=on?", self.email))
            }
            Err(e) => Err(format!("setting the day clock: {e}")),
        }
    }

    /// What each of `players`' cards says they've lately been busy with
    /// (Callings K3.1), `GET /players/{id}/card`, as anyone reading it would
    /// see: for the soak's §10 measures (#553). `None` for a quiet card, and
    /// for one that couldn't be read.
    pub fn cards(&self, config: &NetConfig, players: &[Uuid]) -> Result<Vec<Option<Lately>>, String> {
        let token = self.sign_in(config)?;
        Ok(players
            .iter()
            .map(|id| match net::call_once(config, Some(&token), &RestRequest::PlayerCard { player_id: *id }) {
                Ok(RestResponse::Card(card)) => card.lately,
                Ok(_) | Err(_) => None,
            })
            .collect())
    }

    /// Every live knob, `GET /admin/tunables` (D6.1)
    pub fn tunables(&self, config: &NetConfig) -> Result<TunablesResponse, String> {
        self.tunables_call(config, &RestRequest::Tunables, "reading the tunables")
    }

    /// Sets knobs, all of them or none, `PUT /admin/tunables`, for a
    /// balance run's preset (D6.3). Returns every knob as it is now.
    pub fn set_tunables(&self, config: &NetConfig, update: TunablesUpdate) -> Result<TunablesResponse, String> {
        self.tunables_call(config, &RestRequest::SetTunables(update), "setting the tunables")
    }

    fn tunables_call(&self, config: &NetConfig, request: &RestRequest, doing: &str) -> Result<TunablesResponse, String> {
        let token = self.sign_in(config)?;
        match net::call_once(config, Some(&token), request) {
            Ok(RestResponse::Tunables(tunables)) => Ok(tunables),
            Ok(other) => Err(format!("{doing}: unexpected {other:?}")),
            Err(e) if e.status() == Some(403) => Err(format!("{doing}: {e}. Is {} an admin?", self.email)),
            Err(e) => Err(format!("{doing}: {e}")),
        }
    }
}

/// Makes `count` citizens through `POST /admin/citizens` (C0.1) and adds each
/// to the roster as soon as it's made: its key's secret is shown only once.
/// Each is of `role`, or, with none given, roles continue the sim's mix from
/// the roster's size, so any town is a mix. Merchants (C1.6) are made only
/// by name. Each one's calling (K0.2) is of `style`, or by the roster's mix
/// of callings. Names come from [`persona::name`]; one the server says is taken is passed
/// over for the next. Returns the citizens made; on an error, those made
/// before it are on the roster all the same.
pub fn mint(
    config: &NetConfig,
    admin: &Admin,
    roster: &mut Roster,
    count: usize,
    role: Option<Role>,
    style: Option<Style>,
) -> Result<Vec<Citizen>, String> {
    let token = admin.sign_in(config)?;
    let had = roster.citizens.len();
    let roles = match role {
        Some(role) => vec![role; had + count],
        None => Role::assign(had + count),
    };
    let mut made = Vec::with_capacity(count);
    for (i, &role) in roles.iter().enumerate().skip(had) {
        let mut tries = (0..NAME_TRIES)
            .map(|k| persona::name(role, i as u64 + k * 1000))
            .filter(|name| !roster.citizens.iter().any(|c| c.name.eq_ignore_ascii_case(name)));
        let created = loop {
            let Some(name) = tries.next() else {
                return Err(format!("every name tried for citizen {} is taken", i + 1));
            };
            let request = RestRequest::CreateCitizen(CreateCitizenRequest { name: name.clone() });
            match net::call_once(config, Some(&token), &request) {
                Ok(RestResponse::Citizen(created)) => break created,
                Ok(other) => return Err(format!("minting {name}: unexpected {other:?}")),
                // Someone already goes by it
                Err(e) if e.status() == Some(409) => continue,
                Err(e) if e.status() == Some(403) => {
                    return Err(format!("minting {name}: {e}. Is {} an admin?", admin.email))
                }
                Err(e) => return Err(format!("minting {name}: {e}")),
            }
        };
        roster
            .append(&created.citizen.name, role, style, &created.secret)
            .map_err(|e| {
                format!(
                    "{} was made, but writing it to {} failed: {e}. Its key is {}",
                    created.citizen.name,
                    roster.path.display(),
                    created.secret
                )
            })?;
        made.push(roster.citizens.last().cloned().expect("just appended"));
    }
    Ok(made)
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

/// Where a citizen's life stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Waiting its turn to start, or signing in.
    Starting,
    /// At a step of its role.
    Working,
    /// Sitting out a minute: nothing to do, or too many failures in a row.
    Resting,
    /// The connection dropped; waiting for the server to come back.
    Offline,
    /// Its life ended; starting again after a backoff.
    Waiting,
    /// Logged out by its habit (#276), until its next session.
    Away,
    /// The server refused its key. It won't start again.
    Retired,
    Stopped,
}

impl fmt::Display for Standing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Standing::Starting => "starting",
            Standing::Working => "working",
            Standing::Resting => "resting",
            Standing::Offline => "offline",
            Standing::Waiting => "waiting",
            Standing::Away => "away",
            Standing::Retired => "retired",
            Standing::Stopped => "stopped",
        })
    }
}

/// One citizen, as the town sees it.
#[derive(Debug, Clone)]
pub struct Health {
    pub name: String,
    /// Its birth trade: the roster's role
    pub role: Role,
    /// Its calling (Callings K0.2), once known.
    pub calling: Option<Calling>,
    /// The job of the moment (K1.2): its stint's, once it has one.
    pub stint: Option<Role>,
    /// Game hours the stint has run, over its sessions, as last seen.
    pub stint_hours: f64,
    /// Game hours on each activity since the town started (K0.1), free
    /// time among them, for the report to the server (K3.3).
    pub time: BTreeMap<Activity, f64>,
    pub standing: Standing,
    /// Where in its day it is, by the clock (C1.2).
    pub phase: Option<Phase>,
    pub strands: Option<i64>,
    /// What it's doing: the step's label, as the sim's report files it.
    pub activity: String,
    /// The last thing that went wrong, and when.
    pub failure: Option<(Instant, String)>,
    /// Things that went wrong, all told.
    pub failures: u64,
    /// Lives started after the first.
    pub restarts: u32,
    /// Where it is on the road in (#450), as it last heard
    pub road_in: Option<RoadInReport>,
}

impl Health {
    pub fn new(name: &str, role: Role) -> Health {
        Health {
            name: name.to_string(),
            role,
            calling: None,
            stint: None,
            stint_hours: 0.0,
            time: BTreeMap::new(),
            standing: Standing::Starting,
            phase: None,
            strands: None,
            activity: "waiting its turn".into(),
            failure: None,
            failures: 0,
            restarts: 0,
            road_in: None,
        }
    }

    pub fn doing(&mut self, standing: Standing, activity: &str) {
        self.standing = standing;
        self.activity = activity.to_string();
    }

    pub fn failed(&mut self, why: String) {
        self.failure = Some((Instant::now(), why));
        self.failures += 1;
    }

    /// The bucket its reports count it in (Callings K3.3): its calling's,
    /// its favourite job's slug or `all-rounder`; its role's slug until its
    /// calling is known.
    pub fn bucket(&self) -> &'static str {
        self.calling.as_ref().map_or(self.role.slug(), Calling::bucket)
    }

    /// Adds the time in `entries` to its time by activity.
    fn spent(&mut self, entries: &[Entry]) {
        for entry in entries {
            let (activity, hours) = match entry {
                Entry::Spent { activity, hours, .. } => (*activity, *hours),
                Entry::Idle { hours, .. } => (Activity::Free, *hours),
                _ => continue,
            };
            *self.time.entry(activity).or_insert(0.0) += hours;
        }
    }

    /// What the town's report to the server says of it (D4.1): its health
    /// line's parts, and since Callings K3.3 its calling, its stint and its
    /// time by activity.
    fn report(&self, failure: Option<TownFailure>, decisions: Vec<DecisionBrief>) -> CitizenReport {
        // Two places is plenty for the console, and keeps the report small
        let two = |v: f64| (v * 100.0).round() / 100.0;
        CitizenReport {
            name: self.name.clone(),
            role: self.bucket().to_string(),
            standing: self.standing.to_string(),
            phase: self.phase.map(|p| p.as_str().to_string()),
            strands: self.strands,
            activity: self.activity.clone(),
            restarts: self.restarts,
            failure,
            decisions,
            road_in: self.road_in.clone(),
            calling: self.calling.as_ref().map(Calling::report),
            doing: self.stint.map(|job| StintReport {
                job: job.slug().to_string(),
                hours: two(self.stint_hours),
            }),
            time: self
                .time
                .iter()
                .filter(|(_, h)| **h > 0.0)
                .map(|(a, h)| (a.slug().to_string(), two(*h)))
                .collect(),
        }
    }

    /// One line: who, what role and calling, where its life stands and
    /// where in its day, Strands, the job of the moment and what it's doing
    /// at it, and the last failure.
    pub fn line(&self, now: Instant) -> String {
        let strands = self.strands.map_or_else(|| "–".to_string(), |s| format!("{s} S"));
        let phase = self.phase.map_or("–", |p| p.as_str());
        let style = self.calling.as_ref().map_or("–", |c| c.style().as_str());
        let stint = self.stint.map_or(String::new(), |job| format!("{}: ", calling::doing(job)));
        let mut line = format!(
            "{:<20} {:<17} {:<11} {:<8} {:<6} {:>9}  {stint}{}",
            self.name,
            self.bucket(),
            style,
            self.standing.to_string(),
            phase,
            strands,
            self.activity
        );
        if let Some(road) = self.road_in.as_ref().filter(|r| !r.finished()) {
            line.push_str(&format!(" · road in {}/{}", road.done, road.of));
        }
        if self.restarts > 0 {
            line.push_str(&format!(" · {} restart{}", self.restarts, if self.restarts == 1 { "" } else { "s" }));
        }
        if let Some((at, why)) = &self.failure {
            line.push_str(&format!(" · last failure {} ago: {why}", ago(now.saturating_duration_since(*at))));
        }
        line
    }
}

/// A short duration: `40s`, `12m`, `3h`.
fn ago(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=99 => format!("{s}s"),
        100..=5999 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

/// What a citizen did while the town ran, for its report (C1.7): kept across
/// its lives.
#[derive(Debug, Clone, Default)]
pub struct Book {
    /// The sim's report entries: what it made, traded, deposited and bought,
    /// how long each step took, and why steps failed or it was stuck
    pub entries: Vec<Entry>,
    /// How its questions were settled ([`By::label`](super::decide::By::label))
    pub rulings: BTreeMap<String, u64>,
    /// Its Strands when the town first saw them, and last
    pub strands_first: Option<i64>,
    pub strands_last: Option<i64>,
}

impl Book {
    fn record(&mut self, entries: Vec<Entry>, rulings: BTreeMap<String, u64>) {
        self.entries.extend(entries);
        for (by, n) in rulings {
            *self.rulings.entry(by).or_insert(0) += n;
        }
    }

    fn saw_strands(&mut self, strands: Option<i64>) {
        if strands.is_some() {
            self.strands_first = self.strands_first.or(strands);
            self.strands_last = strands;
        }
    }
}

/// What a citizen's loop answers to: when to stop, where to say what it's
/// doing, and where to write down what it did ([`sim::run::live`]).
pub(crate) struct Tether<'a> {
    stop: &'a AtomicBool,
    health: &'a Mutex<Health>,
    book: &'a Mutex<Book>,
    /// Its stints, kept across its lives and the town's (Callings K1.2).
    kept: &'a Mutex<Kept>,
    /// The server's clock less ours, as the last life saw it.
    clock_offset_ms: &'a AtomicI64,
    /// When this session ends (#276), in unix ms on our clock.
    session_end: &'a AtomicI64,
}

impl Tether<'_> {
    /// Whether to stop: the town says so, or the session's over (#276).
    /// Every loop in a life checks it.
    pub(crate) fn stopped(&self) -> bool {
        self.town_stopped() || local_unix_ms() >= self.session_end.load(Ordering::Relaxed)
    }

    /// Whether its sessions end when its habit says (#276), not on all
    /// the time: `--always-on` never sets an end.
    pub(crate) fn keeps_hours(&self) -> bool {
        self.session_end.load(Ordering::Relaxed) != i64::MAX
    }

    /// Whether the town says stop.
    fn town_stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// How a life that stopped ended: the town stopped, or its session's
    /// over.
    pub(crate) fn ended(&self) -> Ended {
        if self.town_stopped() {
            Ended::Stopped
        } else {
            Ended::Away
        }
    }

    pub(crate) fn health(&self, change: impl FnOnce(&mut Health)) {
        let mut health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut health);
        let strands = health.strands;
        drop(health);
        self.book.lock().unwrap_or_else(|e| e.into_inner()).saw_strands(strands);
    }

    /// Writes down what the citizen did since it last did, for the report,
    /// and its time for the report to the server.
    pub(crate) fn record(&self, entries: Vec<Entry>, rulings: BTreeMap<String, u64>) {
        self.health.lock().unwrap_or_else(|e| e.into_inner()).spent(&entries);
        self.book.lock().unwrap_or_else(|e| e.into_inner()).record(entries, rulings);
    }

    /// Its stints as last kept (Callings K1.2).
    pub(crate) fn kept(&self) -> Kept {
        self.kept.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Keeps its stints, for its next life and the town's side file.
    pub(crate) fn keep(&self, kept: Kept) {
        *self.kept.lock().unwrap_or_else(|e| e.into_inner()) = kept;
    }

    /// Notes the server's day clock, as a life signed in reads it: its
    /// clock and the day's offset (#427).
    pub(crate) fn day_clock_was(&self, server_ms: Option<i64>) {
        if let Some(ms) = server_ms {
            self.clock_offset_ms.store(ms - local_unix_ms(), Ordering::Relaxed);
        }
    }

    /// The time of day on the server's day clock, as last seen.
    fn time_of_day(&self) -> f64 {
        rules::time_of_day(local_unix_ms() + self.clock_offset_ms.load(Ordering::Relaxed))
    }
}

/// How a citizen's life ended, when it ended well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The town stopped.
    Stopped,
    /// Its session was over (#276): it logged out where it stood.
    Away,
}

/// When a town's citizens are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hours {
    /// By each one's [`Habit`] (#276): a few hours some days, on the real
    /// clock.
    Habits,
    /// Every citizen, all the time, through day and night: for a soak that
    /// wants the whole pool at work (`--always-on`).
    AlwaysOn,
}

fn local_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

// ---------------------------------------------------------------------------
// The town
// ---------------------------------------------------------------------------

/// The running town: a thread a citizen, their health and their books.
pub struct Town {
    stop: Arc<AtomicBool>,
    health: Vec<Arc<Mutex<Health>>>,
    books: Vec<Arc<Mutex<Book>>>,
    /// Each citizen's stints (Callings K1.2), by name, and where they're kept
    kept: Vec<(String, Arc<Mutex<Kept>>)>,
    stints_path: PathBuf,
    pulses: Vec<SharedPulse>,
    lives: Vec<thread::JoinHandle<()>>,
}

impl Town {
    /// Starts every citizen on the roster against `server`, those on now
    /// [`STAGGER`] apart, each logging its decisions to `logs` if given.
    /// Setting `stop` ends them.
    pub fn start(
        roster: &Roster,
        server: &str,
        stop: Arc<AtomicBool>,
        logs: Option<&Logs>,
        hours: Hours,
    ) -> std::io::Result<Town> {
        let stints_path = stints_path(&roster.path);
        let mut stints = read_stints(&stints_path);
        let mut town = Town {
            stop,
            health: Vec::new(),
            books: Vec::new(),
            kept: Vec::new(),
            stints_path,
            pulses: Vec::new(),
            lives: Vec::new(),
        };
        let now = local_unix_ms();
        let mut arriving = 0;
        for citizen in &roster.citizens {
            let kept = stints.remove(&citizen.name).unwrap_or_default();
            let mut health = Health::new(&citizen.name, citizen.role);
            health.calling = Some(citizen.persona.calling.clone());
            health.stint = kept.job.as_deref().and_then(Role::from_slug);
            health.stint_hours = kept.run.max(0.0);
            let health = Arc::new(Mutex::new(health));
            let book = Arc::new(Mutex::new(Book::default()));
            let kept = Arc::new(Mutex::new(kept));
            let pulse = Pulse::shared();
            let log = logs.and_then(|logs| open_log(citizen, logs));
            town.kept.push((citizen.name.clone(), kept.clone()));
            let (citizen, server, stop, h, b, k, p) =
                (citizen.clone(), server.to_string(), town.stop.clone(), health.clone(), book.clone(), kept, pulse.clone());
            // Only those signing in now wait their turn
            let on = hours == Hours::AlwaysOn || matches!(citizen.persona.habit.at(now), Presence::On { .. });
            let turn = if on { STAGGER * arriving } else { Duration::ZERO };
            arriving += u32::from(on);
            let life = thread::Builder::new()
                .name(format!("citizen-{}", citizen.name))
                .spawn(move || live(&citizen, &server, turn, hours, &stop, &h, &b, &k, &p, log.as_ref()))?;
            town.health.push(health);
            town.books.push(book);
            town.pulses.push(pulse);
            town.lives.push(life);
        }
        Ok(town)
    }

    /// Each citizen's run so far, as the sim's report reads one, in roster
    /// order, with how its questions were settled. Its Strands are the
    /// first and last the town saw.
    pub fn runs(&self) -> Vec<(BotRun, BTreeMap<String, u64>)> {
        self.health
            .iter()
            .zip(&self.books)
            .map(|(health, book)| {
                let (name, role) = {
                    let h = health.lock().unwrap_or_else(|e| e.into_inner());
                    (h.name.clone(), h.bucket())
                };
                let book = book.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let run = BotRun {
                    name,
                    role: role.to_string(),
                    entries: book.entries,
                    strands_start: book.strands_first.unwrap_or(0),
                    strands_end: book.strands_last.unwrap_or(0),
                    error: None,
                };
                (run, book.rulings)
            })
            .collect()
    }

    /// Each citizen's calling, by name, as the console's roster says it
    /// (Callings K3.3): for the soak's §10 measures (#553), which a run
    /// doesn't carry.
    pub fn callings(&self) -> BTreeMap<String, CallingReport> {
        self.health
            .iter()
            .filter_map(|health| {
                let h = health.lock().unwrap_or_else(|e| e.into_inner());
                Some((h.name.clone(), h.calling.as_ref()?.report()))
            })
            .collect()
    }

    /// Writes each citizen's stints beside the roster (Callings K1.2),
    /// keeping those kept there of anyone not on this roster.
    pub fn save_stints(&self) -> std::io::Result<()> {
        let mut stints = read_stints(&self.stints_path);
        for (name, kept) in &self.kept {
            stints.insert(name.clone(), kept.lock().unwrap_or_else(|e| e.into_inner()).clone());
        }
        write_stints(&self.stints_path, &stints)
    }

    /// Its citizens' health and pulses, for a [`Reporter`] on another
    /// thread.
    pub fn view(&self) -> TownView {
        TownView {
            health: self.health.clone(),
            pulses: self.pulses.clone(),
        }
    }

    /// A line a citizen, in roster order.
    pub fn health_lines(&self) -> Vec<String> {
        let now = Instant::now();
        self.health
            .iter()
            .map(|h| h.lock().unwrap_or_else(|e| e.into_inner()).line(now))
            .collect()
    }

    /// How many citizens stand each way, for a summary: `18 working, 2 resting`.
    pub fn tally(&self) -> String {
        let mut counts: Vec<(Standing, usize)> = Vec::new();
        for h in &self.health {
            let standing = h.lock().unwrap_or_else(|e| e.into_inner()).standing;
            match counts.iter_mut().find(|(s, _)| *s == standing) {
                Some((_, n)) => *n += 1,
                None => counts.push((standing, 1)),
            }
        }
        counts.iter().map(|(s, n)| format!("{n} {s}")).collect::<Vec<_>>().join(", ")
    }

    /// Who was where in their day at `unix_ms`, a CSV row a citizen under
    /// [`PRESENCE_HEADER`]: what C1.2's chart of a day is drawn from. Its
    /// game time is on the server's day clock, `day_offset_ms` on (#427).
    pub fn presence(&self, unix_ms: i64, day_offset_ms: i64) -> Vec<String> {
        let time = schedule::clock(rules::time_of_day(unix_ms + day_offset_ms));
        self.health
            .iter()
            .map(|h| {
                let h = h.lock().unwrap_or_else(|e| e.into_inner());
                let phase = h.phase.map_or("", |p| p.as_str());
                format!("{unix_ms},{time},{},{},{phase},{}", h.name, h.role.slug(), h.standing)
            })
            .collect()
    }

    /// Whether every citizen's life is over: stopped or retired.
    pub fn is_empty(&self) -> bool {
        self.lives.iter().all(|l| l.is_finished())
    }

    /// Waits up to `grace` for the citizens to stop, after `stop` was set.
    /// Their resting orders stay up, as the market makers' do; the next run
    /// finds them through the resync. Returns how many hadn't stopped.
    pub fn wait(&mut self, grace: Duration) -> usize {
        let end = Instant::now() + grace;
        while Instant::now() < end && !self.is_empty() {
            thread::sleep(STOP_CHECK);
        }
        let (over, going): (Vec<_>, Vec<_>) = std::mem::take(&mut self.lives).into_iter().partition(|l| l.is_finished());
        for life in over {
            let _ = life.join();
        }
        let stuck = going.len();
        self.lives = going;
        stuck
    }
}

/// A citizen's decision log, opened for this run. A log that won't open is
/// said and done without: a citizen never stops for its log.
fn open_log(citizen: &Citizen, logs: &Logs) -> Option<SharedLog> {
    let path = log::file_for(&logs.dir, &citizen.name);
    match Log::open(&path, &citizen.name, citizen.role.slug(), Some(&citizen.persona.calling), &citizen.layers, logs.limit, logs.keep) {
        Ok(log) => Some(Arc::new(Mutex::new(log))),
        Err(e) => {
            eprintln!("{}: no decision log: {}: {e}", citizen.name, path.display());
            None
        }
    }
}

/// A citizen's lives, one after another, until the town stops or the server
/// refuses its key. A life is a session (#276): the citizen signs in when
/// its habit says, plays through whatever the game's clock has it doing, and
/// logs out when the session's over. With [`Hours::AlwaysOn`] it never is.
#[allow(clippy::too_many_arguments)]
fn live(
    citizen: &Citizen,
    server: &str,
    turn: Duration,
    hours: Hours,
    stop: &AtomicBool,
    health: &Mutex<Health>,
    book: &Mutex<Book>,
    kept: &Mutex<Kept>,
    pulse: &SharedPulse,
    log: Option<&SharedLog>,
) {
    let clock_offset_ms = AtomicI64::new(0);
    let session_end = AtomicI64::new(i64::MAX);
    let tether = Tether {
        stop,
        health,
        kept,
        book,
        clock_offset_ms: &clock_offset_ms,
        session_end: &session_end,
    };
    let day = &citizen.persona.schedule;
    let habit = &citizen.persona.habit;
    pause(stop, turn);
    let mut attempt = 0;
    while !tether.town_stopped() {
        if hours == Hours::Habits {
            match habit.at(local_unix_ms()) {
                Presence::On { until } => session_end.store(until, Ordering::Relaxed),
                Presence::Away { until } => {
                    away(&tether, habit, until);
                    continue;
                }
            }
        }
        let phase = day.phase(tether.time_of_day());
        tether.health(|h| {
            h.phase = Some(phase);
            h.doing(Standing::Starting, "signing in");
        });
        let began = Instant::now();
        let life = panic::catch_unwind(AssertUnwindSafe(|| one_life(citizen, server, &tether, pulse, log)));
        let why = match life {
            Ok(Ok(Ended::Stopped)) => break,
            // The presence check at the top has it away
            Ok(Ok(Ended::Away)) => {
                attempt = 0;
                continue;
            }
            Ok(Err(e)) if e.is_final() => {
                tether.health(|h| {
                    h.failed(e.to_string());
                    h.doing(Standing::Retired, "its key was refused");
                });
                eprintln!("{}: retired: {e}", citizen.name);
                return;
            }
            Ok(Err(e)) => e.to_string(),
            Err(panicked) => format!("panicked: {}", panic_message(&*panicked)),
        };
        if tether.town_stopped() {
            break;
        }
        if began.elapsed() >= LIVED_WELL {
            attempt = 0;
        }
        attempt += 1;
        let wait = net::backoff(attempt, RESTART_FIRST, RESTART_MAX, net::jitter());
        eprintln!("{}: {why}; starting again in {:.0?}", citizen.name, wait);
        tether.health(|h| {
            h.failed(why);
            h.restarts += 1;
            h.doing(Standing::Waiting, &format!("starting again in {}s", wait.as_secs().max(1)));
        });
        pause(stop, wait);
    }
    tether.health(|h| h.doing(Standing::Stopped, "stopped"));
}

/// One life: sign in by the key and play the role through the session,
/// until it's over or the town stops. The bot goes with it, so its socket
/// closes and the avatar leaves the world.
fn one_life(
    citizen: &Citizen,
    server: &str,
    tether: &Tether,
    pulse: &SharedPulse,
    log: Option<&SharedLog>,
) -> Result<Ended, BotError> {
    let key = citizen.key().map_err(BotError::Failed)?;
    let bot = Bot::with_api_key(NetConfig::with_base_url(server), &key)?.quiet();
    tether.day_clock_was(bot.state().day_clock_ms());
    // The roster checked the layers
    let cascade = Cascade::from_names(&citizen.layers).map_err(BotError::Failed)?;
    let mut mind = Mind::new(cascade, citizen.persona.temperament).with_pulse(pulse.clone());
    if let Some(log) = log {
        mind = mind.with_log(log.clone());
    }
    let who = decide::Who {
        name: citizen.name.clone(),
        role: citizen.role.slug().to_string(),
        temperament: citizen.persona.temperament,
        calling: Some(citizen.persona.calling.clone()),
    };
    // The roster checked the voice
    let voice = voice::voice(&citizen.voice, who, seed(&citizen.name)).map_err(BotError::Failed)?;
    sim::run::live(
        bot,
        citizen.role,
        &citizen.persona,
        seed(&citizen.name),
        mind,
        voice,
        citizen.hire_line,
        citizen.trade_line,
        citizen.patron_line,
        tether,
    )
}

/// Signed out until its next session (`until`, unix ms), or the town stops.
/// With none in sight, it looks again in a day.
fn away(tether: &Tether, habit: &Habit, until: Option<i64>) {
    let now = local_unix_ms();
    let (wait, back) = match until {
        Some(at) => (
            Duration::from_millis((at - now).max(0) as u64),
            format!("away until {} {}", habit.local_clock(at), habit.zone_name()),
        ),
        None => (Duration::from_secs(24 * 3600), "away for weeks".to_string()),
    };
    tether.health(|h| {
        h.phase = None;
        h.doing(Standing::Away, &back);
    });
    pause(tether.stop, wait);
}

// ---------------------------------------------------------------------------
// The report to the server (D4.1)
// ---------------------------------------------------------------------------

/// The town's citizens as a [`Reporter`] reads them: their health and
/// their minds' pulses, shared with their lives.
#[derive(Clone)]
pub struct TownView {
    health: Vec<Arc<Mutex<Health>>>,
    pulses: Vec<SharedPulse>,
}

/// Makes the town's reports to the server (D4.1): what each citizen is
/// doing and has newly decided, the layers' turns and cost, and what went
/// wrong, each over the window since the report before.
pub struct Reporter {
    view: TownView,
    started_ms: i64,
    last_ms: i64,
    /// Each citizen's failures, as last counted
    seen: Vec<u64>,
    /// The newest failures, oldest first
    failures: Vec<TownFailure>,
}

impl Reporter {
    /// For the town in `view`, which started at `started_ms` (Unix ms).
    pub fn new(view: TownView, started_ms: i64) -> Reporter {
        let seen = vec![0; view.health.len()];
        Reporter {
            view,
            started_ms,
            last_ms: started_ms,
            seen,
            failures: Vec::new(),
        }
    }

    /// The town at `now_ms` (Unix ms); `stopping` makes it the last.
    pub fn report(&mut self, now_ms: i64, stopping: bool) -> TownReport {
        let now = Instant::now();
        let mut report = TownReport {
            at_ms: now_ms,
            started_ms: self.started_ms,
            since_ms: now_ms - self.last_ms,
            stopping,
            ..TownReport::default()
        };
        self.last_ms = now_ms;
        let mut spent: BTreeMap<String, f64> = BTreeMap::new();
        for (i, (health, pulse)) in self.view.health.iter().zip(&self.view.pulses).enumerate() {
            let h = health.lock().unwrap_or_else(|e| e.into_inner());
            let failure = h.failure.as_ref().map(|(at, why)| TownFailure {
                at_ms: now_ms - now.saturating_duration_since(*at).as_millis() as i64,
                citizen: h.name.clone(),
                why: why.clone(),
            });
            // A citizen that failed twice since the last report shows only
            // its last, but both count
            let new = h.failures - self.seen[i];
            self.seen[i] = h.failures;
            report.failed += new;
            if let Some(failure) = failure.as_ref().filter(|_| new > 0) {
                self.failures.push(failure.clone());
            }
            let beat = pulse.lock().unwrap_or_else(|e| e.into_inner()).take();
            for (name, layer) in &beat.layers {
                *spent.entry(name.clone()).or_insert(0.0) += layer.cents_total;
                report.layers.entry(name.clone()).or_default().add(layer);
            }
            for (by, n) in beat.rulings {
                *report.rulings.entry(by).or_insert(0) += n;
            }
            report.citizens.push(h.report(failure, beat.decisions));
        }
        // A layer's cost so far is every citizen's together
        for (name, total) in spent {
            report.layers.entry(name).or_default().cents_total = total;
        }
        self.failures.sort_by_key(|f| f.at_ms);
        let over = self.failures.len().saturating_sub(TOWN_FAILURES);
        self.failures.drain(..over);
        report.failures = self.failures.clone();
        report
    }
}

/// Sleeps for `how_long`, or until `stop`.
fn pause(stop: &AtomicBool, how_long: Duration) {
    let end = Instant::now() + how_long;
    while !stop.load(Ordering::Relaxed) {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        thread::sleep(left.min(STOP_CHECK));
    }
}

/// FNV-1a of a name: a citizen's own random stream, the same every run.
fn seed(name: &str) -> u64 {
    name.bytes()
        .fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Craft, Gather};

    const ROSTER_TEXT: &str = r#"
# a comment
server = "http://localhost:3001"

[[citizen]]
name = "Ada Reeder"
role = "gatherer-fiber"
key = "cap_ada"

[[citizen]]
name = "Percy Sawyer"
role = "crafter-planks"
key_file = "keys/percy.key"
layers = ["rules"]
voice = "claude-haiku"
"#;

    #[test]
    fn a_roster_reads_names_roles_and_keys() {
        let roster = Roster::parse(ROSTER_TEXT, Path::new("town/town.toml")).unwrap();
        assert_eq!(roster.server.as_deref(), Some("http://localhost:3001"));
        assert_eq!(roster.citizens.len(), 2);
        assert_eq!(roster.citizens[0].name, "Ada Reeder");
        assert_eq!(roster.citizens[0].role, Role::Gatherer(Gather::Fiber));
        assert_eq!(roster.citizens[0].key(), Ok("cap_ada".into()));
        assert_eq!(roster.citizens[1].role, Role::Crafter(Craft::Planks));
        // The town's layers, which Percy changes
        assert_eq!(roster.layers, ["rules"]);
        assert_eq!(roster.citizens[0].layers, ["rules"]);
        // The town's voice, which Percy changes
        assert_eq!(roster.voice, "phrasebook");
        assert_eq!(roster.citizens[0].voice, "phrasebook");
        assert_eq!(roster.citizens[1].voice, "claude-haiku");
        // From the roster's own directory
        assert_eq!(roster.citizens[1].key, Key::File(Path::new("town").join("keys/percy.key")));
    }

    #[test]
    fn a_bad_roster_says_what_is_wrong() {
        let path = Path::new("town.toml");
        let err = |text: &str| Roster::parse(text, path).unwrap_err();
        assert!(err("[[citizen]]\nname = \"A\"\nrole = \"poet\"\nkey = \"k\"").contains("gatherer-fiber"));
        assert!(err("[[citizen]]\nname = \"A\"\nrole = \"builder\"").contains("needs a key"));
        assert!(err("[[citizen]]\nname = \"A\"\nrole = \"builder\"\nkey = \"k\"\nkey_file = \"f\"").contains("only one"));
        assert!(err(
            "[[citizen]]\nname = \"A\"\nrole = \"builder\"\nkey = \"k\"\n[[citizen]]\nname = \"a\"\nrole = \"builder\"\nkey = \"j\""
        )
        .contains("twice"));
        assert!(err("[[citizen]]\nname = \"A\"\nrole = \"builder\"\nkey = \"k\"\nhat = 1").contains("hat"));
        assert!(Roster::parse("", path).unwrap().citizens.is_empty());
        assert!(err("layers = [\"rules\", \"oracle\"]").contains("\"oracle\" is no layer"));
        assert!(err("layers = []").contains("at least one"));
        assert!(err("voice = \"oracle\"").contains("\"oracle\" is no voice"));
        assert!(err("[[citizen]]
name = \"A\"
role = \"builder\"
key = \"k\"
voice = \"x\"").contains("has a voice"));
        assert!(err("[[citizen]]
name = \"A\"
role = \"builder\"
key = \"k\"
layers = [\"x\"]").contains("citizen \"A\""));
    }

    #[test]
    fn an_appended_citizen_reads_back_and_the_rest_is_kept() {
        let dir = std::env::temp_dir().join(format!("capitol-town-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("town.toml");
        std::fs::write(&path, ROSTER_TEXT).unwrap();
        let mut roster = Roster::load(&path).unwrap();
        roster.append("Maud \"Mo\" Mason", Role::Builder, None, "cap_maud").unwrap();
        roster.append("Ivor Tyler", Role::Builder, Some(Style::AllRounder), "cap_ivor").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(text.starts_with(ROSTER_TEXT), "the comment and the rest stay");
        let again = Roster::parse(&text, &path).unwrap();
        assert_eq!(again.citizens.len(), 4);
        assert_eq!(again.citizens[2].name, "Maud \"Mo\" Mason");
        assert_eq!(again.citizens[2].key(), Ok("cap_maud".into()));
        // Ivor was dealt the style he was minted with, and it's written down
        assert_eq!(again.citizens[3].persona.calling.style(), Style::AllRounder);
        assert_eq!(again.citizens, roster.citizens);
    }

    #[test]
    fn the_example_roster_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../town.example.toml");
        let roster = Roster::load(&path).unwrap();
        assert!(!roster.citizens.is_empty());
        // Ada's persona is written out; Percy's is left to his name's seed
        let ada = &roster.citizens[0].persona;
        assert_eq!(ada.home.avenue.as_str(), "south-west");
        assert_eq!(ada.market, Market::Bazaar);
        assert_eq!(schedule::clock(ada.schedule.wake), "05:40");
        assert_eq!(ada.calling.favourite, Some(Role::Gatherer(Gather::Fiber)));
        assert_eq!(ada.calling.style(), Style::Specialist);
        assert!((ada.calling.share(Role::Gatherer(Gather::Wood)) - 0.12).abs() < 1e-3);
        let percy = &roster.citizens[1];
        assert_eq!(percy.persona, Persona::seeded(percy.role, seed(&percy.name)));
    }

    #[test]
    fn a_roster_reads_a_persona_and_seeds_what_it_leaves_out() {
        let text = r#"
[[citizen]]
name = "Ada Reeder"
role = "gatherer-fiber"
key = "k"
home = { zone = "suburban", avenue = "north-east", depth = 0.25 }
schedule = { wake = "06:10", market = "09:30-11:00", home = "18:50", bed = "21:15" }
"#;
        let roster = Roster::parse(text, Path::new("town.toml")).unwrap();
        let ada = &roster.citizens[0].persona;
        let seeded = Persona::seeded(Role::Gatherer(Gather::Fiber), seed("Ada Reeder"));
        assert_eq!(ada.home.zone, Zone::Suburban);
        assert_eq!(ada.home.avenue, Avenue::from_str("north-east").unwrap());
        assert_eq!(ada.schedule.market(), "09:30-11:00");
        assert_eq!(schedule::clock(ada.schedule.bed), "21:15");
        // The market and the temperament come from her name
        assert_eq!(ada.market, seeded.market);
        assert_eq!(ada.temperament, seeded.temperament);
    }

    #[test]
    fn a_bad_persona_says_what_is_wrong() {
        let path = Path::new("town.toml");
        let with = |line: &str| {
            Roster::parse(&format!("[[citizen]]\nname = \"A\"\nrole = \"builder\"\nkey = \"k\"\n{line}"), path).unwrap_err()
        };
        assert!(with("market = \"pub\"").contains("\"hall\" or \"bazaar\""));
        assert!(with("home = { zone = \"wilderness\", avenue = \"south\", depth = 0.5 }").contains("rural, suburban or urban"));
        assert!(with("home = { zone = \"rural\", avenue = \"up\", depth = 0.5 }").contains("compass point"));
        assert!(with("home = { zone = \"rural\", avenue = \"south\", depth = 1.5 }").contains("depth"));
        assert!(with("schedule = { wake = \"6am\", market = \"09:30-11:00\", home = \"18:50\", bed = \"21:15\" }")
            .contains("game clock"));
        assert!(with("schedule = { wake = \"06:00\", market = \"19:30-20:00\", home = \"18:50\", bed = \"21:15\" }")
            .contains("must run wake < market < home < bed"));
        assert!(with("temperament = { thrift = 2.0, risk = 0.5, sociability = 0.5, patience = 0.5 }").contains("thrift"));
        assert!(with("temperament = { thrift = 0.5 }").contains("missing field"));
    }

    #[test]
    fn a_minted_entry_reads_back_the_persona_its_name_seeds() {
        for role in sim::ROLES {
            for n in 0..30 {
                let name = persona::name(role, n);
                // Each style in turn, as `mint --calling` deals them
                let style = Style::ALL[n as usize % 3];
                let citizen = Citizen {
                    name: name.clone(),
                    role,
                    layers: default_layers(),
                    voice: voice::DEFAULT_VOICE.to_string(),
                    persona: Persona::seeded_in(role, seed(&name), &Mix::default(), &calling::Mix::only(style)),
                    hire_line: wage::HIRE_LINE,
                    trade_line: stint::TRADE_LINE,
                    patron_line: patron::PATRON_LINE,
                    key: Key::Inline("k".into()),
                };
                let text = entry(&citizen, "k");
                let back = Roster::parse(&text, Path::new("town.toml")).unwrap();
                assert_eq!(back.citizens[0], citizen, "{text}");
            }
        }
    }

    /// Callings K2.4: the town's line for hiring, on each citizen.
    #[test]
    fn a_roster_reads_the_towns_line_for_hiring() {
        let ada = "[[citizen]]\nname = \"Ada Reeder\"\nrole = \"builder\"\nkey = \"k\"\n";
        let path = Path::new("t.toml");
        let roster = Roster::parse(&format!("[hiring]\nline = 1500\n\n{ada}"), path).unwrap();
        assert_eq!((roster.hire_line, roster.citizens[0].hire_line), (1500, 1500));
        assert_eq!(Roster::parse(ada, path).unwrap().citizens[0].hire_line, wage::HIRE_LINE);
        assert!(Roster::parse("[hiring]\nline = -1\n", path).unwrap_err().contains("0 or more"));
        assert!(Roster::parse("[hiring]\nfloor = 3\n", path).is_err());
    }

    /// Callings K2.2: the town's line for trading, on each citizen.
    #[test]
    fn a_roster_reads_the_towns_line_for_trading() {
        let ada = "[[citizen]]\nname = \"Ada Reeder\"\nrole = \"builder\"\nkey = \"k\"\n";
        let path = Path::new("t.toml");
        let roster = Roster::parse(&format!("[trading]\nline = 3000\n\n{ada}"), path).unwrap();
        assert_eq!((roster.trade_line, roster.citizens[0].trade_line), (3000, 3000));
        assert_eq!(Roster::parse(ada, path).unwrap().citizens[0].trade_line, stint::TRADE_LINE);
        assert!(Roster::parse("[trading]\nline = -1\n", path).unwrap_err().contains("0 or more"));
        assert!(Roster::parse("[trading]\nfloat = 3\n", path).is_err());
    }

    /// Callings K2.3: the town's line for patrons, on each citizen.
    #[test]
    fn a_roster_reads_the_towns_line_for_patrons() {
        let ada = "[[citizen]]\nname = \"Ada Reeder\"\nrole = \"builder\"\nkey = \"k\"\n";
        let path = Path::new("t.toml");
        let roster = Roster::parse(&format!("[patrons]\nline = 4000\n\n{ada}"), path).unwrap();
        assert_eq!((roster.patron_line, roster.citizens[0].patron_line), (4000, 4000));
        assert_eq!(Roster::parse(ada, path).unwrap().citizens[0].patron_line, patron::PATRON_LINE);
        assert!(Roster::parse("[patrons]\nline = -1\n", path).unwrap_err().contains("0 or more"));
        assert!(Roster::parse("[patrons]\nfloor = 3\n", path).is_err());
    }

    /// A roster from before callings (K0.2): each citizen keeps the name,
    /// temperament, home, market, day and habit it had, pinned here as
    /// they were drawn before callings, and draws a calling after them.
    #[test]
    fn an_old_roster_keeps_its_people_and_draws_each_a_calling() {
        let text = r#"
[[citizen]]
name = "Ada Rushton"
role = "gatherer-fiber"
key = "a"

[[citizen]]
name = "Dorothy Builder"
role = "builder"
key = "b"

[[citizen]]
name = "Nora Arkwright"
role = "crafter-tools"
key = "c"

[[citizen]]
name = "Mabel Draper"
role = "merchant"
key = "d"
"#;
        let roster = Roster::parse(text, Path::new("town.toml")).unwrap();
        // Their names are still what mint names them
        let names: Vec<String> = [(Role::Gatherer(Gather::Fiber), 0), (Role::Builder, 7), (Role::Crafter(Craft::Tools), 31), (Role::Merchant, 400)]
            .into_iter()
            .map(|(role, n)| persona::name(role, n))
            .collect();
        assert_eq!(names, roster.citizens.iter().map(|c| c.name.clone()).collect::<Vec<_>>());
        let pinned = [
            ((0.12, 0.54, 0.64, 0.2), (Zone::Suburban, 7, 0.42), Market::Hall, 0.22569444444444445, (Kind::Regular, 0)),
            ((0.32, 0.85, 0.67, 0.53), (Zone::Suburban, 1, 0.68), Market::Hall, 0.2513888888888889, (Kind::Casual, -6)),
            ((0.87, 0.34, 0.61, 0.72), (Zone::Suburban, 6, 0.68), Market::Bazaar, 0.2708333333333333, (Kind::Casual, -5)),
            ((0.23, 0.63, 0.4, 0.61), (Zone::Urban, 7, 0.24), Market::Hall, 0.2361111111111111, (Kind::Casual, 1)),
        ];
        for (citizen, (t, (zone, avenue, depth), market, wake, (kind, tz))) in roster.citizens.iter().zip(pinned) {
            let p = &citizen.persona;
            let who = &citizen.name;
            assert_eq!((p.temperament.thrift, p.temperament.risk, p.temperament.sociability, p.temperament.patience), t, "{who}");
            assert_eq!((p.home.zone, p.home.avenue, p.home.depth), (zone, Avenue(avenue), depth), "{who}");
            assert_eq!(p.market, market, "{who}");
            assert_eq!(p.schedule.wake, wake, "{who}");
            assert_eq!((p.habit.kind, p.habit.zone), (kind, tz), "{who}");
            // And a calling, its favourite its birth trade unless it has none
            assert_eq!(p.calling.check(citizen.role), Ok(()), "{who}");
            assert!(p.calling.favourite.is_none_or(|f| f == citizen.role), "{who}");
            assert_eq!(p.calling, Persona::seeded(citizen.role, seed(who)).calling);
        }
    }

    /// `mint`'s town of a thousand, its roles by the sim's mix and its names
    /// as mint gives them, is dealt each style within 3% of the default mix.
    #[test]
    fn a_thousand_minted_are_dealt_the_mix_of_callings() {
        let town: Vec<Persona> = Role::assign(1000)
            .into_iter()
            .enumerate()
            .map(|(i, role)| Persona::seeded(role, seed(&persona::name(role, i as u64))))
            .collect();
        let mix = calling::Mix::default();
        for style in Style::ALL {
            let share = town.iter().filter(|p| p.calling.style() == style).count() as f64 / 1000.0;
            assert!((share - mix.weight(style)).abs() <= 0.03, "{style}: {share}");
        }
    }

    #[test]
    fn a_roster_reads_a_calling_and_the_towns_mix() {
        let text = r#"
[callings]
mix = { specialist = 0.0, leaner = 0.0, all-rounder = 1.0 }

[[citizen]]
name = "Ada Reeder"
role = "gatherer-fiber"
key = "k"
calling = { favourite = "gatherer-fiber", focus = 0.5, wander = 0.9, leanings = { builder = 0.25, merchant = 0.25 } }

[[citizen]]
name = "Percy Sawyer"
role = "crafter-planks"
key = "j"
"#;
        let roster = Roster::parse(text, Path::new("town.toml")).unwrap();
        let ada = &roster.citizens[0].persona.calling;
        assert_eq!(ada.style(), Style::Leaner);
        assert_eq!(ada.share(Role::Gatherer(Gather::Fiber)), 0.5);
        assert_eq!(ada.share(Role::Merchant), 0.25);
        assert_eq!(ada.share(Role::Gatherer(Gather::Wood)), 0.0);
        assert_eq!(ada.wander, 0.9);
        // In the jobs' own order, whatever order the roster wrote them in
        assert_eq!(ada.leanings, vec![(Role::Builder, 0.25), (Role::Merchant, 0.25)]);
        // Percy's is drawn from his name, by the town's mix: no favourite
        assert_eq!(roster.citizens[1].persona.calling.style(), Style::AllRounder);
        assert_eq!(roster.callings, calling::Mix::only(Style::AllRounder));
        assert_eq!(Roster::parse("", Path::new("t.toml")).unwrap().callings, calling::Mix::default());
        let path = Path::new("town.toml");
        let err = |text: &str| Roster::parse(text, path).unwrap_err();
        assert!(err("[callings]\nmix = { specialist = -1.0, leaner = 0.3, all-rounder = 0.5 }").contains("0 or more"));
        assert!(err("[callings]\nmix = { specialist = 0.1 }").contains("missing field"));
        let with = |calling: &str| err(&format!("[[citizen]]\nname = \"A\"\nrole = \"builder\"\nkey = \"k\"\ncalling = {calling}"));
        assert!(with("{ favourite = \"merchant\", focus = 0.7, wander = 0.1, leanings = {} }").contains("its role is builder"));
        assert!(with("{ favourite = \"poet\", focus = 0.7, wander = 0.1, leanings = {} }").contains("a job is one of"));
        assert!(with("{ favourite = \"builder\", focus = 0.7, wander = 0.1, leanings = { hats = 0.1 } }").contains("\"hats\""));
        assert!(with("{ favourite = \"none\", focus = 0.7, wander = 0.1, leanings = { builder = 0.1 } }").contains("all-rounder's focus is 0"));
        assert!(with("{ favourite = \"builder\", focus = 0.7, wander = 0.1, leanings = { builder = 0.1 } }").contains("besides favouring"));
        assert!(with("{ favourite = \"builder\", focus = 0.7 }").contains("missing field"));
        // Its wander runs 0 to 1, as everything else does
        let all = with("{ favourite = \"none\", focus = 0.0, wander = 2.0, leanings = { builder = 0.5, merchant = 0.5 } }");
        assert!(all.contains("wander"), "{all}");
    }

    /// C1.2's day, as planned: twenty citizens minted as `mint` would, and
    /// who of them is where at each game quarter-hour, while they're on.
    #[test]
    fn a_town_of_twenty_works_by_day_trades_in_the_afternoon_and_works_the_night_from_home() {
        let roles = Role::assign(20);
        let town: Vec<Persona> = roles
            .iter()
            .enumerate()
            .map(|(i, &role)| Persona::seeded(role, seed(&persona::name(role, i as u64))))
            .collect();
        let count = |clock: &str, phase: Phase| {
            let t = schedule::parse_clock(clock).unwrap();
            town.iter().filter(|p| p.schedule.phase(t) == phase).count()
        };
        // Midnight and the small hours: everyone at the night's work
        for clock in ["00:00", "03:00", "04:30", "22:30"] {
            assert_eq!(count(clock, Phase::Night), 20, "{clock}");
        }
        // Mid-morning and early afternoon, most at work
        assert!(count("08:00", Phase::Work) >= 18);
        // The afternoon market draws most of the town; the morning's fewer
        assert!(count("15:00", Phase::Market) >= 9, "{}", count("15:00", Phase::Market));
        assert!(count("10:00", Phase::Market) < count("15:00", Phase::Market));
        // At dusk they're on their way home, and nobody's at work
        assert!(count("19:45", Phase::Home) >= 15);
        assert_eq!(count("19:45", Phase::Work) + count("19:45", Phase::Market), 0);
        // Spread: nobody wakes at once with everyone else
        let first = town.iter().map(|p| p.schedule.wake).fold(1.0, f64::min);
        let last = town.iter().map(|p| p.schedule.wake).fold(0.0, f64::max);
        assert!((last - first) * 24.0 >= 0.75, "the town wakes over three quarters of an hour or more");
    }

    #[test]
    fn a_roster_reads_habits_and_the_towns_mix() {
        let text = r#"
[habits]
mix = { dedicated = 1.0, regular = 0.0, casual = 0.0, lapsed = 0.0 }

[[citizen]]
name = "Ada Reeder"
role = "gatherer-fiber"
key = "k"
habit = { kind = "lapsed", zone = 10 }

[[citizen]]
name = "Percy Sawyer"
role = "crafter-planks"
key = "j"
"#;
        let roster = Roster::parse(text, Path::new("town.toml")).unwrap();
        let ada = roster.citizens[0].persona.habit;
        assert_eq!((ada.kind, ada.zone), (Kind::Lapsed, 10));
        // Her days are her name's, as the seeded habit's would be
        assert_eq!(ada, Habit::new(Kind::Lapsed, 10, 0).with_seed(seed("Ada Reeder")));
        assert_eq!(ada.with_seed(0), Habit::new(Kind::Lapsed, 10, 0));
        // Percy's is drawn from his name, by the town's mix: all dedicated
        assert_eq!(roster.citizens[1].persona.habit.kind, Kind::Dedicated);
        assert_eq!(roster.mix.dedicated, 1.0);
        // The default mix, without one
        assert_eq!(Roster::parse("", Path::new("t.toml")).unwrap().mix, Mix::default());
        let path = Path::new("town.toml");
        let err = |text: &str| Roster::parse(text, path).unwrap_err();
        assert!(err("[habits]\nmix = { dedicated = -1.0, regular = 0.3, casual = 0.5, lapsed = 0.1 }").contains("0 or more"));
        assert!(err("[habits]\nmix = { dedicated = 0.1 }").contains("missing field"));
        let with = |line: &str| err(&format!("[[citizen]]\nname = \"A\"\nrole = \"builder\"\nkey = \"k\"\n{line}"));
        assert!(with("habit = { kind = \"keen\", zone = 0 }").contains("dedicated, regular, casual or lapsed"));
        assert!(with("habit = { kind = \"casual\", zone = 20 }").contains("-12 to 14"));
    }

    #[test]
    fn a_life_stops_when_its_session_is_over() {
        let (stop, health, book, offset) = (
            AtomicBool::new(false),
            Mutex::new(Health::new("Ada Reeder", Role::Builder)),
            Mutex::new(Book::default()),
            AtomicI64::new(0),
        );
        let kept = Mutex::new(Kept::default());
        let session_end = AtomicI64::new(local_unix_ms() + 60_000);
        let tether = Tether {
            stop: &stop,
            health: &health,
            kept: &kept,
            book: &book,
            clock_offset_ms: &offset,
            session_end: &session_end,
        };
        assert!(!tether.stopped());
        session_end.store(local_unix_ms() - 1, Ordering::Relaxed);
        assert!(tether.stopped());
        assert_eq!(tether.ended(), Ended::Away);
        assert!(!tether.town_stopped(), "the town lives on");
        stop.store(true, Ordering::Relaxed);
        assert_eq!(tether.ended(), Ended::Stopped);
    }

    #[test]
    fn a_health_line_says_who_what_and_how_it_stands() {
        let mut h = Health::new("Ada Reeder", Role::Gatherer(Gather::Fiber));
        h.strands = Some(312);
        h.doing(Standing::Working, "gather fiber");
        h.phase = Some(Phase::Market);
        let now = Instant::now();
        let line = h.line(now);
        assert!(line.starts_with("Ada Reeder"), "{line}");
        for part in ["gatherer-fiber", "working", "market", "312 S", "gather fiber"] {
            assert!(line.contains(part), "{line} lacks {part}");
        }
        // Its calling's style and the job of the moment (Callings K1.2)
        let mut leaner = Calling::single(Role::Gatherer(Gather::Fiber));
        leaner.focus = 0.45;
        leaner.leanings = vec![(Role::Crafter(Craft::Planks), 0.55)];
        h.calling = Some(leaner);
        h.stint = Some(Role::Crafter(Craft::Planks));
        let line = h.line(now);
        assert!(line.contains("leaner") && line.contains("sawing planks: gather fiber"), "{line}");
        // An all-rounder goes by its bucket, as the reports do (K3.3)
        h.calling = Some(Calling { favourite: None, focus: 0.0, ..h.calling.clone().unwrap() });
        assert!(h.line(now).starts_with("Ada Reeder           all-rounder "), "{}", h.line(now));
        assert!(!line.contains("failure"));
        h.failure = Some((now, "extract refused".into()));
        h.restarts = 1;
        let line = h.line(now + Duration::from_secs(240));
        assert!(line.contains("1 restart ·"), "{line}");
        assert!(line.ends_with("last failure 4m ago: extract refused"), "{line}");
    }

    /// The side file (Callings K1.2) keeps each citizen's stint by name, and
    /// one that's missing or torn is no stints, never a town that won't start.
    #[test]
    fn stints_are_kept_beside_the_roster_and_read_back() {
        let dir = std::env::temp_dir().join(format!("capitol-stints-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = stints_path(&dir.join("town.toml"));
        assert_eq!(path.file_name().unwrap(), "town.stints.json");
        let _ = std::fs::remove_file(&path);
        assert!(read_stints(&path).is_empty());
        let mut lately = sim::stint::Lately::default();
        lately.add(Role::Builder, 1.25);
        let ada = Kept { job: Some("builder".into()), run: 0.5, lately, seen: ["wild_spur_4".to_string()].into(), ..Kept::default() };
        let stints = BTreeMap::from([("Ada Reeder".to_string(), ada.clone()), ("Bo Tanner".to_string(), Kept::default())]);
        write_stints(&path, &stints).unwrap();
        assert_eq!(read_stints(&path), stints);
        assert!(!path.with_extension("json.part").exists());
        std::fs::write(&path, "{ torn").unwrap();
        assert!(read_stints(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pause_ends_when_the_town_stops() {
        let stop = AtomicBool::new(true);
        let start = Instant::now();
        pause(&stop, Duration::from_secs(30));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    /// A rules mind with `pulse` decides `n` work questions
    fn decide(pulse: &SharedPulse, n: usize) {
        use decide::{Choice, Held, Place, Question, Situation, When, Who};
        let s = Situation {
            who: Who { name: "Ada Reeder".into(), role: "gatherer-wood".into(), temperament: Temperament::default(), calling: None },
            at: Place { x: 0.0, y: 0.0, zone: "rural".into() },
            when: When { time_of_day: Some(0.5), hours: 1.0 },
            holdings: Held::default(),
            market: Vec::new(),
            question: Question::Work,
            menu: vec![Choice::Work { goal: "log".into(), step: "more log".into() }, Choice::Rest],
            carry_on: Some(0),
        };
        let mut mind = Mind::new(Cascade::rules(), Temperament::default()).with_pulse(pulse.clone());
        for _ in 0..n {
            mind.ask(&s);
        }
    }

    /// Callings K3.3: each citizen's bucket, calling, stint and time by
    /// activity, as the console reads them.
    #[test]
    fn a_report_says_each_citizens_calling_stint_and_time() {
        use crate::sim::{Activity, ROLES};
        let wood = Role::Gatherer(Gather::Wood);
        let planks = Role::Crafter(Craft::Planks);
        let ada = Arc::new(Mutex::new(Health::new("Ada Reeder", wood)));
        let cora = Arc::new(Mutex::new(Health::new("Cora Vane", planks)));
        let view = TownView { health: vec![ada.clone(), cora.clone()], pulses: vec![Pulse::shared(), Pulse::shared()] };
        let mut reporter = Reporter::new(view, 1_000);
        let report = reporter.report(2_000, false);
        assert_eq!(report.citizens[1].role, "crafter-planks", "its role until its calling is known");
        assert!(report.citizens[1].calling.is_none() && report.citizens[1].doing.is_none() && report.citizens[1].time.is_empty());

        {
            let mut a = ada.lock().unwrap();
            a.calling = Some(Calling {
                favourite: Some(wood),
                focus: 0.7,
                leanings: vec![
                    (Role::Gatherer(Gather::Fiber), 0.04),
                    (Role::Gatherer(Gather::Ore), 0.03),
                    (planks, 0.14),
                    (Role::Builder, 0.09),
                ],
                wander: 0.3,
            });
            a.stint = Some(planks);
            a.stint_hours = 0.756;
            let spent = |activity, hours| Entry::Spent { label: "x".into(), hours, ok: true, activity, setup: false, day: 0 };
            a.spent(&[spent(Activity::Wood, 1.5), Entry::Idle { hours: 0.5, day: 0 }, spent(Activity::Planks, 0.25), Entry::Stuck { reason: "no".into() }]);
            a.spent(&[spent(Activity::Wood, 0.5)]);
        }
        cora.lock().unwrap().calling = Some(Calling {
            favourite: None,
            focus: 0.0,
            leanings: ROLES.into_iter().map(|r| (r, 0.125)).collect(),
            wander: 0.5,
        });
        let report = reporter.report(7_000, false);
        let a = &report.citizens[0];
        assert_eq!(a.role, "gatherer-wood");
        let calling = a.calling.as_ref().unwrap();
        assert_eq!((calling.style.as_str(), calling.favourite.as_deref(), calling.focus), ("specialist", Some("gatherer-wood"), 0.7));
        // Its top leanings, the most first, none under 5%
        let leans: Vec<(&str, f64)> = calling.leanings.iter().map(|l| (l.job.as_str(), l.share)).collect();
        assert_eq!(leans, [("crafter-planks", 0.14), ("builder", 0.09)]);
        let doing = a.doing.as_ref().unwrap();
        assert_eq!((doing.job.as_str(), doing.hours), ("crafter-planks", 0.76));
        assert_eq!(a.time, BTreeMap::from([("free".to_string(), 0.5), ("planks".to_string(), 0.25), ("wood".to_string(), 2.0)]));
        let c = &report.citizens[1];
        assert_eq!(c.role, "all-rounder");
        let calling = c.calling.as_ref().unwrap();
        assert_eq!((calling.style.as_str(), calling.favourite.as_deref(), calling.focus), ("all-rounder", None, 0.0));
        assert_eq!(calling.leanings.len(), 3, "the top three of eight even ones");
        assert!(c.doing.is_none(), "no stint yet");
        // The health line buckets it the same way
        assert!(cora.lock().unwrap().line(Instant::now()).contains(" all-rounder "));
    }

    #[test]
    fn a_report_says_what_each_citizen_did_since_the_last() {
        let ada = Arc::new(Mutex::new(Health::new("Ada Reeder", Role::Gatherer(Gather::Wood))));
        let bo = Arc::new(Mutex::new(Health::new("Bo Tanner", Role::Merchant)));
        let (ada_pulse, bo_pulse) = (Pulse::shared(), Pulse::shared());
        let view = TownView { health: vec![ada.clone(), bo.clone()], pulses: vec![ada_pulse.clone(), bo_pulse.clone()] };
        let mut reporter = Reporter::new(view, 1_000);

        ada.lock().unwrap().doing(Standing::Working, "more log");
        decide(&ada_pulse, 2);
        decide(&bo_pulse, 1);
        bo.lock().unwrap().failed("placing a bid: refused".into());
        bo.lock().unwrap().failed("placing a bid: refused again".into());
        let report = reporter.report(6_000, false);
        assert_eq!((report.started_ms, report.since_ms, report.stopping), (1_000, 5_000, false));
        assert_eq!(report.citizens[0].standing, "working");
        assert_eq!(report.citizens[0].role, "gatherer-wood");
        assert_eq!(report.citizens[0].decisions.len(), 2);
        assert_eq!(report.layers["rules"].asked, 3, "both citizens' turns");
        assert_eq!(report.rulings.values().sum::<u64>(), 3);
        assert_eq!(report.failed, 2, "both failures count");
        assert_eq!(report.failures.len(), 1, "the last of them shows");
        assert_eq!(report.failures[0].why, "placing a bid: refused again");
        assert_eq!(report.citizens[1].failure.as_ref().unwrap().citizen, "Bo Tanner");

        // Nothing new: no decisions sent, no failures counted, the last kept
        let report = reporter.report(11_000, true);
        assert_eq!((report.since_ms, report.stopping), (5_000, true));
        assert!(report.citizens.iter().all(|c| c.decisions.is_empty()));
        assert!(report.rulings.is_empty());
        assert_eq!((report.failed, report.failures.len()), (0, 1));
    }

    #[test]
    fn durations_read_short() {
        assert_eq!(ago(Duration::from_secs(40)), "40s");
        assert_eq!(ago(Duration::from_secs(720)), "12m");
        assert_eq!(ago(Duration::from_secs(3 * 3600 + 5)), "3h");
    }
}
