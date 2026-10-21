//! `capitol-town`: the citizens of The Capitol (C1.1), AI players who live
//! in the world, run from a roster.
//!
//! ```text
//! capitol-town mint --count 20      # make twenty citizens, onto town.toml
//! capitol-town mint --count 2 --role merchant   # and two merchants (C1.6)
//! capitol-town                      # live until Ctrl-C
//! capitol-town --status             # the running town's last health
//! capitol-town replay --layers rules # the decision logs, replayed (C3.1)
//! ```
//!
//! Run it from `flint-client/` with `cargo run -p capitol-core --bin capitol-town -- <flags>`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capitol_core::citizen::decide::log;
use capitol_core::citizen::decide::replay::Replayer;
use capitol_core::citizen::decide::{self, Cascade};
use capitol_core::citizen::schedule;
use capitol_core::citizen::town::{self, Admin, Logs, Roster, Town};
use capitol_core::net::NetConfig;
use capitol_core::sim::Role;

const USAGE: &str = "\
capitol-town: the citizens of The Capitol

USAGE:
    capitol-town [run]           Live: every citizen on the roster lives its
                                 day until Ctrl-C: up at its wake, its trade,
                                 its market hours, home at dusk, and logged
                                 out for the night. A citizen whose life
                                 ends starts again after a backoff; one
                                 whose key is refused retires.
    capitol-town mint --count <n> [--role <role>]
                                 Make n citizens through the admin endpoint
                                 and add them, with their keys and their
                                 seeded personas, to the roster
    capitol-town status          Print the running town's last health
                                 (also --status)
    capitol-town replay [<log>...] --layers <a,b>
                                 Run a stack of decision layers over the
                                 citizens' decision logs, offline, and
                                 report its agreement with what they chose,
                                 its escalation rate, latency and cost. A
                                 log is a file or a directory of them
                                 [default: town.decisions beside the roster]

FLAGS:
    --roster <path>   The roster [default: town.toml]
    --server <url>    Server root [default: $CAPITOL_SERVER, else the
                      roster's `server`, else http://localhost:3000]
    --every <secs>    How often a running town prints and writes its health
                      [default: 60]
    --count <n>       With mint, how many citizens to make
    --role <role>     With mint, make every one of them this: gatherer-fiber,
                      gatherer-wood, gatherer-ore, crafter-planks,
                      crafter-ingots, crafter-tools, builder or merchant
                      [default: the sim's mix, which has no merchants]
    --admin <email>   With mint, an admin account to make them as
                      [default: $CAPITOL_ADMIN_EMAIL]. Its password is read
                      from $CAPITOL_ADMIN_PASSWORD only, never a flag.
    --log-mb <n>      With run, how big a citizen's decision log grows
                      before it rotates (two old ones are kept); 0 logs
                      nothing [default: 4]
    --layers <a,b>    With replay, the stack to run, bottom first
                      [default: rules]
    -h, --help        Print this

The roster holds keys, so keep it out of git (flint-client/.gitignore has
town.toml); town.example.toml shows its shape. Health goes beside it, in
town.status, and who was where in their day is appended to
town.presence.csv. Each citizen logs its decisions, and what came of them,
to town.decisions/<name>.jsonl. Ctrl-C lets each citizen finish its step and stops; any order
it has resting on the book stays up, as a market maker's does. A second
Ctrl-C stops at once.";

/// How long a stopping town waits for its citizens to finish their steps.
const GRACE: Duration = Duration::from_secs(20);
/// A decision log's size before it rotates, in MB, unless told otherwise.
const LOG_MB: u64 = log::LOG_LIMIT >> 20;

#[derive(Debug, PartialEq)]
enum Action {
    Run,
    Mint { count: usize },
    Status,
    Replay { logs: Vec<PathBuf>, layers: Vec<String> },
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
    /// 0: no decision logs
    log_mb: u64,
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut action = None;
    let mut count = None;
    let mut logs = Vec::new();
    let mut layers = None;
    let mut log_mb_given = false;
    let mut options = Options {
        action: Action::Run,
        roster: PathBuf::from(town::ROSTER),
        server: std::env::var("CAPITOL_SERVER").ok().filter(|s| !s.is_empty()),
        every: Duration::from_secs(60),
        admin: std::env::var("CAPITOL_ADMIN_EMAIL").ok().filter(|s| !s.is_empty()),
        role: None,
        log_mb: LOG_MB,
    };
    let mut admin_given = false;
    let mut every_given = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        let replaying = matches!(action, Some(Action::Replay { .. }));
        let mut set = |a: Action| match action.replace(a) {
            Some(_) => Err("run, mint, status and replay go one at a time".to_string()),
            None => Ok(()),
        };
        match arg.as_str() {
            "run" => set(Action::Run)?,
            "mint" => set(Action::Mint { count: 0 })?,
            "status" | "--status" => set(Action::Status)?,
            "replay" => set(Action::Replay {
                logs: Vec::new(),
                layers: Vec::new(),
            })?,
            "--layers" => {
                let names: Vec<String> = value()?.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
                decide::check_layers(&names)?;
                layers = Some(names);
            }
            "--log-mb" => {
                options.log_mb = number(&arg, &value()?)?;
                log_mb_given = true;
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
            "--admin" => {
                options.admin = Some(value()?);
                admin_given = true;
            }
            "-h" | "--help" => return Ok(None),
            log if replaying && !log.starts_with('-') => logs.push(PathBuf::from(log)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    options.action = match (action.unwrap_or(Action::Run), count) {
        (Action::Mint { .. }, Some(count)) => Action::Mint { count },
        (Action::Mint { .. }, None) => return Err("mint needs --count".into()),
        (_, Some(_)) => return Err("--count goes with mint".into()),
        (Action::Replay { .. }, None) => Action::Replay {
            logs,
            layers: layers.take().unwrap_or_else(|| decide::DEFAULT_LAYERS.map(String::from).to_vec()),
        },
        (action, None) => action,
    };
    if layers.is_some() {
        return Err("--layers goes with replay".into());
    }
    if log_mb_given && options.action != Action::Run {
        return Err("--log-mb goes with run".into());
    }
    if admin_given && !matches!(options.action, Action::Mint { .. }) {
        return Err("--admin goes with mint".into());
    }
    if options.role.is_some() && !matches!(options.action, Action::Mint { .. }) {
        return Err("--role goes with mint".into());
    }
    if every_given && options.action != Action::Run {
        return Err("--every goes with run".into());
    }
    Ok(Some(options))
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

fn mint(options: &Options, count: usize) -> Result<(), String> {
    let email = options
        .admin
        .clone()
        .ok_or("mint needs an admin: --admin <email> or $CAPITOL_ADMIN_EMAIL")?;
    let password = std::env::var("CAPITOL_ADMIN_PASSWORD")
        .ok()
        .filter(|p| !p.is_empty())
        .ok_or("mint reads the admin's password from $CAPITOL_ADMIN_PASSWORD")?;
    let mut roster = Roster::load_or_new(&options.roster)?;
    let server = server(options, &roster);
    let had = roster.citizens.len();
    let result = town::mint(
        &NetConfig::with_base_url(&server),
        &Admin { email, password },
        &mut roster,
        count,
        options.role,
    );
    for citizen in &roster.citizens[had..] {
        let p = &citizen.persona;
        println!(
            "made {} ({}), of the {} {} avenue, up at {}, at the {} {}",
            citizen.name,
            citizen.role,
            p.home.zone,
            p.home.avenue,
            schedule::clock(p.schedule.wake),
            p.market,
            p.schedule.market()
        );
    }
    println!(
        "{}: {} citizens on the roster",
        roster.path.display(),
        roster.citizens.len()
    );
    result.map(|_| ())
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

fn run(options: &Options) -> Result<(), String> {
    let roster = Roster::load(&options.roster)?;
    if roster.citizens.is_empty() {
        return Err(format!(
            "{} has no citizens: make some with `capitol-town mint --count <n>`",
            roster.path.display()
        ));
    }
    let server = server(options, &roster);
    let stop = Arc::new(AtomicBool::new(false));
    on_ctrl_c(stop.clone());
    let logs = (options.log_mb > 0).then(|| Logs {
        dir: town::decisions_path(&roster.path),
        limit: options.log_mb << 20,
        keep: log::LOG_KEEP,
    });
    let mut town = Town::start(&roster, &server, stop.clone(), logs.as_ref()).map_err(|e| format!("starting the town: {e}"))?;
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
    let status = &roster.path;
    let mut next = Instant::now() + options.every;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if town.is_empty() {
            println!("capitol-town: every citizen has retired");
            break;
        }
        if Instant::now() >= next {
            report(&town, &server, status);
            next += options.every;
        }
        thread::sleep(Duration::from_millis(250));
    }
    println!("capitol-town: stopping; each citizen finishes its step");
    let stuck = town.wait(GRACE);
    report(&town, &server, status);
    if stuck > 0 {
        println!("capitol-town: {stuck} still mid-step after {GRACE:?}; leaving them");
    }
    Ok(())
}

/// Replays `logs` (the roster's decision logs if none) with the stack
/// `layers`, and prints the report (C3.1).
fn replay(options: &Options, logs: &[PathBuf], layers: &[String]) -> Result<(), String> {
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
    let mut replayer = Replayer::new(move || Cascade::from_names(&names).expect("checked layers"));
    for file in &files {
        let records = log::read(file).map_err(|e| format!("reading {}: {e}", file.display()))?;
        replayer.log(&file.display().to_string(), records);
    }
    print!("{}", replayer.finish());
    Ok(())
}

/// Prints the health and writes it beside the roster, and appends who was
/// where in their day to the presence log (C1.2).
fn report(town: &Town, server: &str, roster: &Path) {
    let now_ms = unix_ms();
    let game = schedule::clock(capitol_core::protocol::rules::time_of_day(now_ms));
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
    if let Err(e) = append_presence(&presence, &town.presence(now_ms)) {
        eprintln!("capitol-town: writing {}: {e}", presence.display());
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
        Action::Mint { count } => mint(&options, count),
        Action::Status => status(&options),
        Action::Replay { ref logs, ref layers } => replay(&options, logs, layers),
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
        let o = parse(args("mint --count 20 --admin chris@example.com --roster t/x.toml"))
            .unwrap()
            .unwrap();
        assert_eq!(o.action, Action::Mint { count: 20 });
        assert_eq!(o.role, None);
        assert_eq!(o.admin.as_deref(), Some("chris@example.com"));
        assert_eq!(o.roster, PathBuf::from("t/x.toml"));
        let o = parse(args("mint --count 2 --role merchant")).unwrap().unwrap();
        assert_eq!(o.role, Some(Role::Merchant));
        assert_eq!(parse(args("--status")).unwrap().unwrap().action, Action::Status);
        assert_eq!(parse(args("status")).unwrap().unwrap().action, Action::Status);
        let o = parse(args("run --every 5 --server http://x:3001")).unwrap().unwrap();
        assert_eq!(o.every, Duration::from_secs(5));
        assert_eq!(o.server.as_deref(), Some("http://x:3001"));
        assert!(parse(args("--help")).unwrap().is_none());
        assert_eq!(parse(args("run --log-mb 0")).unwrap().unwrap().log_mb, 0);
        assert_eq!(parse(args("")).unwrap().unwrap().log_mb, LOG_MB);
    }

    #[test]
    fn replay_takes_logs_and_a_stack() {
        let o = parse(args("replay")).unwrap().unwrap();
        assert_eq!(
            o.action,
            Action::Replay {
                logs: Vec::new(),
                layers: vec!["rules".into()]
            }
        );
        let o = parse(args("replay a.jsonl town.decisions --layers rules")).unwrap().unwrap();
        assert_eq!(
            o.action,
            Action::Replay {
                logs: vec!["a.jsonl".into(), "town.decisions".into()],
                layers: vec!["rules".into()]
            }
        );
        assert!(parse(args("replay --layers rules,oracle")).unwrap_err().contains("oracle"));
        assert!(parse(args("--layers rules")).is_err(), "--layers without replay");
        assert!(parse(args("a.jsonl replay")).is_err(), "a log before replay");
        assert!(parse(args("replay --log-mb 2")).is_err());
    }

    #[test]
    fn bad_arguments_are_reported() {
        assert!(parse(args("mint")).is_err(), "mint without --count");
        assert!(parse(args("mint --count 0")).is_err());
        assert!(parse(args("--count 3")).is_err(), "--count without mint");
        assert!(parse(args("run --admin a@b")).is_err());
        assert!(parse(args("mint status --count 2")).is_err());
        assert!(parse(args("mint --count 2 --role poet")).is_err());
        assert!(parse(args("run --role merchant")).is_err(), "--role without mint");
        assert!(parse(args("status --every 5")).is_err());
        assert!(parse(args("--every 0")).is_err());
        assert!(parse(args("--fly")).is_err());
    }
}
