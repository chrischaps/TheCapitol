//! `capitol-town` (C1.1): a roster of citizens, each living in the world on
//! its own thread, kept alive by a supervisor.
//!
//! - **The roster** is `town.toml`: a name, a [`Role`] and a key (or a key
//!   file) for each citizen, and its [`Persona`] (C1.2). It holds keys, so
//!   it's git-ignored; `town.example.toml` shows the shape. [`mint`] adds
//!   to it.
//! - **A day** (C1.2) is one life: the citizen signs in by its key at its
//!   [`Schedule`]'s `wake` and plays the role's loop ([`sim::run::live`]),
//!   keeping its market hours, until it goes home at dusk and logs out. It
//!   sleeps the night off the server and signs in again in the morning.
//!   A dropped connection or a server restart is ridden out inside the life
//!   (C0.6). A life that ends any other way (the server wasn't up to sign
//!   in to, a panic) starts again after the net layer's backoff. A refused
//!   key retires the citizen.
//! - **Health**: each citizen's [`Health`], one line apiece, which the town
//!   prints and writes beside the roster every minute, and its presence,
//!   appended to `town.presence.csv` (C1.2).
//! - **Decisions**: each citizen's decision log (C3.1,
//!   [`decide::log`](super::decide::log)), one JSONL file apiece in
//!   `town.decisions/` beside the roster, kept across its lives.
//!
//! [`Schedule`]: super::schedule::Schedule

use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use capitol_protocol::rest::{CreateCitizenRequest, LoginRequest};
use capitol_protocol::rules::{self, Zone};
use serde::Deserialize;

use super::decide::log::{self, Log};
use super::decide::{self, Cascade, Mind, SharedLog};
use super::persona::{self, Avenue, Home, Market, Persona, Temperament};
use super::schedule::{self, Phase, Schedule};
use crate::bot::{Bot, BotError};
use crate::net::{self, NetConfig, RestRequest, RestResponse};
use crate::sim::{self, Role};

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
    #[serde(default, rename = "citizen")]
    citizens: Vec<CitizenFile>,
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
    // The persona (C1.2). Any part left out is the one the name seeds.
    home: Option<HomeFile>,
    market: Option<String>,
    schedule: Option<ScheduleFile>,
    temperament: Option<TemperamentFile>,
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
    /// out.
    fn persona(&self, role: Role) -> Result<Persona, String> {
        let mut persona = Persona::seeded(role, seed(&self.name));
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
    pub citizens: Vec<Citizen>,
}

/// One citizen on the roster.
#[derive(Debug, Clone, PartialEq)]
pub struct Citizen {
    pub name: String,
    pub role: Role,
    /// Its decision layers (C1.3): its own, or the town's.
    pub layers: Vec<String>,
    pub persona: Persona,
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
        let mut citizens = Vec::with_capacity(file.citizens.len());
        for c in file.citizens {
            let what = |problem: &str| format!("{}: citizen {:?} {problem}", path.display(), c.name);
            let role = Role::from_slug(&c.role).ok_or_else(|| {
                let known: Vec<&str> = sim::ROLES.iter().map(|r| r.slug()).collect();
                what(&format!("has role {:?}; a role is one of {}", c.role, known.join(", ")))
            })?;
            let persona = c.persona(role).map_err(|e| what(&e))?;
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
            citizens.push(Citizen {
                name: c.name,
                role,
                layers: own,
                persona,
                key,
            });
        }
        Ok(Roster {
            path: path.to_path_buf(),
            server: file.server,
            layers,
            citizens,
        })
    }

    /// Adds a citizen to the end of the roster file, with the persona its
    /// name seeds written out in full, leaving what's there (and its
    /// comments) as it was.
    fn append(&mut self, name: &str, role: Role, secret: &str) -> std::io::Result<()> {
        use std::io::Write;
        let citizen = Citizen {
            name: name.to_string(),
            role,
            layers: self.layers.clone(),
            persona: Persona::seeded(role, seed(name)),
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
    } = &citizen.persona;
    format!(
        "\n[[citizen]]\nname = {}\nrole = {}\nkey = {}\n\
         home = {{ zone = {}, avenue = {}, depth = {:?} }}\n\
         market = {}\n\
         schedule = {{ wake = {}, market = {}, home = {}, bed = {} }}\n\
         temperament = {{ thrift = {:?}, risk = {:?}, sociability = {:?}, patience = {:?} }}\n",
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

/// Where the town writes its health: beside the roster, `town.status`.
pub fn status_path(roster: &Path) -> PathBuf {
    roster.with_extension("status")
}

/// Where the citizens' decision logs go (C3.1): beside the roster,
/// `town.decisions/`, a file a citizen.
pub fn decisions_path(roster: &Path) -> PathBuf {
    roster.with_extension("decisions")
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

/// An admin's sign-in, to mint citizens with.
#[derive(Debug, Clone)]
pub struct Admin {
    pub email: String,
    pub password: String,
}

/// Makes `count` citizens through `POST /admin/citizens` (C0.1) and adds each
/// to the roster as soon as it's made: its key's secret is shown only once.
/// Each is of `role`, or, with none given, roles continue the sim's mix from
/// the roster's size, so any town is a mix. Merchants (C1.6) are made only
/// by name. Names come from [`persona::name`]; one the server says is taken is passed
/// over for the next. Returns the citizens made; on an error, those made
/// before it are on the roster all the same.
pub fn mint(
    config: &NetConfig,
    admin: &Admin,
    roster: &mut Roster,
    count: usize,
    role: Option<Role>,
) -> Result<Vec<Citizen>, String> {
    let login = RestRequest::Login(LoginRequest {
        email: admin.email.clone(),
        password: admin.password.clone(),
    });
    let token = match net::call_once(config, None, &login) {
        Ok(RestResponse::Auth(auth)) => auth.token,
        Ok(other) => return Err(format!("signing in as {}: unexpected {other:?}", admin.email)),
        Err(e) => return Err(format!("signing in as {}: {e}", admin.email)),
    };
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
            .append(&created.citizen.name, role, &created.secret)
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
    /// Logged out for the night (C1.2), until its schedule wakes it.
    Asleep,
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
            Standing::Asleep => "asleep",
            Standing::Retired => "retired",
            Standing::Stopped => "stopped",
        })
    }
}

/// One citizen, as the town sees it.
#[derive(Debug, Clone)]
pub struct Health {
    pub name: String,
    pub role: Role,
    pub standing: Standing,
    /// Where in its day it is, by the clock (C1.2).
    pub phase: Option<Phase>,
    pub strands: Option<i64>,
    /// What it's doing: the step's label, as the sim's report files it.
    pub activity: String,
    /// The last thing that went wrong, and when.
    pub failure: Option<(Instant, String)>,
    /// Lives started after the first.
    pub restarts: u32,
}

impl Health {
    pub fn new(name: &str, role: Role) -> Health {
        Health {
            name: name.to_string(),
            role,
            standing: Standing::Starting,
            phase: None,
            strands: None,
            activity: "waiting its turn".into(),
            failure: None,
            restarts: 0,
        }
    }

    pub fn doing(&mut self, standing: Standing, activity: &str) {
        self.standing = standing;
        self.activity = activity.to_string();
    }

    pub fn failed(&mut self, why: String) {
        self.failure = Some((Instant::now(), why));
    }

    /// One line: who, what role, where its life stands and where in its
    /// day, Strands, what it's doing, and the last failure.
    pub fn line(&self, now: Instant) -> String {
        let strands = self.strands.map_or_else(|| "–".to_string(), |s| format!("{s} S"));
        let phase = self.phase.map_or("–", |p| p.as_str());
        let mut line = format!(
            "{:<20} {:<17} {:<8} {:<6} {:>9}  {}",
            self.name,
            self.role.to_string(),
            self.standing.to_string(),
            phase,
            strands,
            self.activity
        );
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

/// What a citizen's loop answers to: when to stop, and where to say what
/// it's doing ([`sim::run::live`]).
pub(crate) struct Tether<'a> {
    stop: &'a AtomicBool,
    health: &'a Mutex<Health>,
    /// The server's clock less ours, as the last life saw it, so the town
    /// wakes the citizen by the server's clock while it's signed out.
    clock_offset_ms: &'a AtomicI64,
}

impl Tether<'_> {
    pub(crate) fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub(crate) fn health(&self, change: impl FnOnce(&mut Health)) {
        change(&mut self.health.lock().unwrap_or_else(|e| e.into_inner()));
    }

    /// Notes the server's clock, as a life signed in reads it.
    pub(crate) fn server_time_was(&self, server_ms: Option<i64>) {
        if let Some(ms) = server_ms {
            self.clock_offset_ms.store(ms - local_unix_ms(), Ordering::Relaxed);
        }
    }

    /// The time of day on the server's clock, as last seen.
    fn time_of_day(&self) -> f64 {
        rules::time_of_day(local_unix_ms() + self.clock_offset_ms.load(Ordering::Relaxed))
    }
}

/// How a citizen's life ended, when it ended well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The town stopped.
    Stopped,
    /// It went home and logged out for the night (C1.2).
    Bedtime,
}

fn local_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

// ---------------------------------------------------------------------------
// The town
// ---------------------------------------------------------------------------

/// The running town: a thread a citizen, and their health.
pub struct Town {
    stop: Arc<AtomicBool>,
    health: Vec<Arc<Mutex<Health>>>,
    lives: Vec<thread::JoinHandle<()>>,
}

impl Town {
    /// Starts every citizen on the roster against `server`, [`STAGGER`]
    /// apart, each logging its decisions to `logs` if given. Setting `stop`
    /// ends them.
    pub fn start(roster: &Roster, server: &str, stop: Arc<AtomicBool>, logs: Option<&Logs>) -> std::io::Result<Town> {
        let mut town = Town {
            stop,
            health: Vec::new(),
            lives: Vec::new(),
        };
        for (n, citizen) in roster.citizens.iter().enumerate() {
            let health = Arc::new(Mutex::new(Health::new(&citizen.name, citizen.role)));
            let log = logs.and_then(|logs| open_log(citizen, logs));
            let (citizen, server, stop, h) = (citizen.clone(), server.to_string(), town.stop.clone(), health.clone());
            let turn = STAGGER * n as u32;
            let life = thread::Builder::new()
                .name(format!("citizen-{}", citizen.name))
                .spawn(move || live(&citizen, &server, turn, &stop, &h, log.as_ref()))?;
            town.health.push(health);
            town.lives.push(life);
        }
        Ok(town)
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
    /// [`PRESENCE_HEADER`]: what C1.2's chart of a day is drawn from.
    pub fn presence(&self, unix_ms: i64) -> Vec<String> {
        let time = schedule::clock(rules::time_of_day(unix_ms));
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
    match Log::open(&path, &citizen.name, citizen.role.slug(), &citizen.layers, logs.limit, logs.keep) {
        Ok(log) => Some(Arc::new(Mutex::new(log))),
        Err(e) => {
            eprintln!("{}: no decision log: {}: {e}", citizen.name, path.display());
            None
        }
    }
}

/// A citizen's lives, one after another, until the town stops or the server
/// refuses its key. A life is a day (C1.2): the citizen signs in only
/// between its `wake` and its `bed`, and sleeps the night through.
fn live(citizen: &Citizen, server: &str, turn: Duration, stop: &AtomicBool, health: &Mutex<Health>, log: Option<&SharedLog>) {
    let clock_offset_ms = AtomicI64::new(0);
    let tether = Tether {
        stop,
        health,
        clock_offset_ms: &clock_offset_ms,
    };
    let day = &citizen.persona.schedule;
    pause(stop, turn);
    let mut attempt = 0;
    while !tether.stopped() {
        let phase = day.phase(tether.time_of_day());
        tether.health(|h| h.phase = Some(phase));
        if phase == Phase::Night {
            sleep(&tether, day);
            continue;
        }
        tether.health(|h| h.doing(Standing::Starting, "signing in"));
        let began = Instant::now();
        let life = panic::catch_unwind(AssertUnwindSafe(|| one_life(citizen, server, &tether, log)));
        let why = match life {
            Ok(Ok(Ended::Stopped)) => break,
            Ok(Ok(Ended::Bedtime)) => {
                attempt = 0;
                sleep(&tether, day);
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
        if tether.stopped() {
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

/// One life: sign in by the key and play the role through the day, until
/// bedtime or the town stops. The bot goes with it, so its socket closes
/// and the avatar leaves the world.
fn one_life(citizen: &Citizen, server: &str, tether: &Tether, log: Option<&SharedLog>) -> Result<Ended, BotError> {
    let key = citizen.key().map_err(BotError::Failed)?;
    let bot = Bot::with_api_key(NetConfig::with_base_url(server), &key)?.quiet();
    tether.server_time_was(bot.state().server_time_ms());
    // The roster checked the layers
    let cascade = Cascade::from_names(&citizen.layers).map_err(BotError::Failed)?;
    let mut mind = Mind::new(cascade, citizen.persona.temperament);
    if let Some(log) = log {
        mind = mind.with_log(log.clone());
    }
    sim::run::live(bot, citizen.role, &citizen.persona, seed(&citizen.name), mind, tether)
}

/// Signed out until the morning: until `day`'s `wake` on the server's clock,
/// or the town stops.
fn sleep(tether: &Tether, day: &Schedule) {
    let wait = schedule::real(day.until_wake(tether.time_of_day()));
    tether.health(|h| {
        h.phase = Some(Phase::Night);
        h.doing(Standing::Asleep, &format!("asleep until {}", schedule::clock(day.wake)));
    });
    pause(tether.stop, wait);
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
        roster.append("Maud \"Mo\" Mason", Role::Builder, "cap_maud").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(text.starts_with(ROSTER_TEXT), "the comment and the rest stay");
        let again = Roster::parse(&text, &path).unwrap();
        assert_eq!(again.citizens.len(), 3);
        assert_eq!(again.citizens[2].name, "Maud \"Mo\" Mason");
        assert_eq!(again.citizens[2].key(), Ok("cap_maud".into()));
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
        assert!(with("home = { zone = \"wilderness\", avenue = \"south\", depth = 0.5 }").contains("rural or suburban"));
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
        for (role, _) in sim::MIX {
            for n in 0..30 {
                let name = persona::name(role, n);
                let citizen = Citizen {
                    name: name.clone(),
                    role,
                    layers: default_layers(),
                    persona: Persona::seeded(role, seed(&name)),
                    key: Key::Inline("k".into()),
                };
                let text = entry(&citizen, "k");
                let back = Roster::parse(&text, Path::new("town.toml")).unwrap();
                assert_eq!(back.citizens[0], citizen, "{text}");
            }
        }
    }

    /// The acceptance's day, as planned: twenty citizens minted as `mint`
    /// would, and who of them is where at each game quarter-hour.
    #[test]
    fn a_town_of_twenty_works_by_day_trades_in_the_afternoon_and_sleeps_at_night() {
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
        // Midnight and the small hours: everyone abed
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
    fn a_health_line_says_who_what_and_how_it_stands() {
        let mut h = Health::new("Ada Reeder", Role::Gatherer(Gather::Fiber));
        h.strands = Some(312);
        h.doing(Standing::Working, "gather fiber");
        h.phase = Some(Phase::Market);
        let now = Instant::now();
        let line = h.line(now);
        assert!(line.starts_with("Ada Reeder"), "{line}");
        for part in ["gatherer (fiber)", "working", "market", "312 S", "gather fiber"] {
            assert!(line.contains(part), "{line} lacks {part}");
        }
        assert!(!line.contains("failure"));
        h.failure = Some((now, "extract refused".into()));
        h.restarts = 1;
        let line = h.line(now + Duration::from_secs(240));
        assert!(line.contains("1 restart ·"), "{line}");
        assert!(line.ends_with("last failure 4m ago: extract refused"), "{line}");
    }

    #[test]
    fn a_pause_ends_when_the_town_stops() {
        let stop = AtomicBool::new(true);
        let start = Instant::now();
        pause(&stop, Duration::from_secs(30));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn durations_read_short() {
        assert_eq!(ago(Duration::from_secs(40)), "40s");
        assert_eq!(ago(Duration::from_secs(720)), "12m");
        assert_eq!(ago(Duration::from_secs(3 * 3600 + 5)), "3h");
    }
}
