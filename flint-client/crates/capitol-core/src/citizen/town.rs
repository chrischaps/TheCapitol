//! `capitol-town` (C1.1): a roster of citizens, each living in the world on
//! its own thread, kept alive by a supervisor.
//!
//! - **The roster** is `town.toml`: a name, a [`Role`] and a key (or a key
//!   file) for each citizen. It holds keys, so it's git-ignored;
//!   `town.example.toml` shows the shape. [`mint`] adds to it.
//! - **A life** signs in by the key and plays the role's loop
//!   ([`sim::run::live`]) until the town stops. A dropped connection or a
//!   server restart is ridden out inside the life (C0.6). A life that ends
//!   any other way (the server wasn't up to sign in to, a panic) starts
//!   again after the net layer's backoff. A refused key retires the citizen.
//! - **Health**: each citizen's [`Health`], one line apiece, which the town
//!   prints and writes beside the roster every minute.

use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use capitol_protocol::rest::{CreateCitizenRequest, LoginRequest};
use serde::Deserialize;

use super::persona;
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
}

/// The town's citizens, and the server they live on.
#[derive(Debug, Clone, PartialEq)]
pub struct Roster {
    pub path: PathBuf,
    /// The server named in the roster, if it names one.
    pub server: Option<String>,
    pub citizens: Vec<Citizen>,
}

/// One citizen on the roster.
#[derive(Debug, Clone, PartialEq)]
pub struct Citizen {
    pub name: String,
    pub role: Role,
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
                citizens: Vec::new(),
            })
        }
    }

    /// Parses a roster read from `path`.
    pub fn parse(text: &str, path: &Path) -> Result<Roster, String> {
        let file: RosterFile = toml::from_str(text).map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut citizens = Vec::with_capacity(file.citizens.len());
        for c in file.citizens {
            let what = |problem: &str| format!("{}: citizen {:?} {problem}", path.display(), c.name);
            let role = Role::from_slug(&c.role).ok_or_else(|| {
                let known: Vec<&str> = sim::MIX.iter().map(|(r, _)| r.slug()).collect();
                what(&format!("has role {:?}; a role is one of {}", c.role, known.join(", ")))
            })?;
            let key = match (c.key, c.key_file) {
                (Some(secret), None) => Key::Inline(secret),
                (None, Some(file)) => Key::File(dir.join(file)),
                _ => return Err(what("needs a key or a key_file, and only one")),
            };
            if citizens.iter().any(|other: &Citizen| other.name.eq_ignore_ascii_case(&c.name)) {
                return Err(what("is on the roster twice"));
            }
            citizens.push(Citizen { name: c.name, role, key });
        }
        Ok(Roster {
            path: path.to_path_buf(),
            server: file.server,
            citizens,
        })
    }

    /// Adds a citizen to the end of the roster file, leaving what's there
    /// (and its comments) as it was.
    fn append(&mut self, name: &str, role: Role, secret: &str) -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        file.write_all(entry(name, role, secret).as_bytes())?;
        self.citizens.push(Citizen {
            name: name.to_string(),
            role,
            key: Key::Inline(secret.to_string()),
        });
        Ok(())
    }
}

/// A citizen's `[[citizen]]` table.
fn entry(name: &str, role: Role, secret: &str) -> String {
    let quote = |s: &str| toml::Value::String(s.to_string()).to_string();
    format!(
        "\n[[citizen]]\nname = {}\nrole = {}\nkey = {}\n",
        quote(name),
        quote(role.slug()),
        quote(secret)
    )
}

/// Where the town writes its health: beside the roster, `town.status`.
pub fn status_path(roster: &Path) -> PathBuf {
    roster.with_extension("status")
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
/// Roles continue the sim's mix from the roster's size, so any town is a mix.
/// Names come from [`persona::name`]; one the server says is taken is passed
/// over for the next. Returns the citizens made; on an error, those made
/// before it are on the roster all the same.
pub fn mint(config: &NetConfig, admin: &Admin, roster: &mut Roster, count: usize) -> Result<Vec<Citizen>, String> {
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
    let roles = Role::assign(had + count);
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

    /// One line: who, what role, where its life stands, Strands, what it's
    /// doing, and the last failure.
    pub fn line(&self, now: Instant) -> String {
        let strands = self.strands.map_or_else(|| "–".to_string(), |s| format!("{s} S"));
        let mut line = format!(
            "{:<20} {:<17} {:<8} {:>9}  {}",
            self.name,
            self.role.to_string(),
            self.standing.to_string(),
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
}

impl Tether<'_> {
    pub(crate) fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub(crate) fn health(&self, change: impl FnOnce(&mut Health)) {
        change(&mut self.health.lock().unwrap_or_else(|e| e.into_inner()));
    }
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
    /// apart. Setting `stop` ends them.
    pub fn start(roster: &Roster, server: &str, stop: Arc<AtomicBool>) -> std::io::Result<Town> {
        let mut town = Town {
            stop,
            health: Vec::new(),
            lives: Vec::new(),
        };
        for (n, citizen) in roster.citizens.iter().enumerate() {
            let health = Arc::new(Mutex::new(Health::new(&citizen.name, citizen.role)));
            let (citizen, server, stop, h) = (citizen.clone(), server.to_string(), town.stop.clone(), health.clone());
            let turn = STAGGER * n as u32;
            let life = thread::Builder::new()
                .name(format!("citizen-{}", citizen.name))
                .spawn(move || live(&citizen, &server, turn, &stop, &h))?;
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

/// A citizen's lives, one after another, until the town stops or the server
/// refuses its key.
fn live(citizen: &Citizen, server: &str, turn: Duration, stop: &AtomicBool, health: &Mutex<Health>) {
    let tether = Tether { stop, health };
    pause(stop, turn);
    let mut attempt = 0;
    while !tether.stopped() {
        tether.health(|h| h.doing(Standing::Starting, "signing in"));
        let began = Instant::now();
        let life = panic::catch_unwind(AssertUnwindSafe(|| one_life(citizen, server, &tether)));
        let why = match life {
            Ok(Ok(())) => break,
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

/// One life: sign in by the key and play the role until the town stops.
fn one_life(citizen: &Citizen, server: &str, tether: &Tether) -> Result<(), BotError> {
    let key = citizen.key().map_err(BotError::Failed)?;
    let bot = Bot::with_api_key(NetConfig::with_base_url(server), &key)?.quiet();
    sim::run::live(bot, citizen.role, seed(&citizen.name), tether)
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
    }

    #[test]
    fn a_health_line_says_who_what_and_how_it_stands() {
        let mut h = Health::new("Ada Reeder", Role::Gatherer(Gather::Fiber));
        h.strands = Some(312);
        h.doing(Standing::Working, "gather fiber");
        let now = Instant::now();
        let line = h.line(now);
        assert!(line.starts_with("Ada Reeder"), "{line}");
        for part in ["gatherer (fiber)", "working", "312 S", "gather fiber"] {
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
