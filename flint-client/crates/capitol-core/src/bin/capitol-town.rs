//! `capitol-town`: the citizens of The Capitol (C1.1), AI players who live
//! in the world, run from a roster.
//!
//! ```text
//! capitol-town mint --count 20      # make twenty citizens, onto town.toml
//! capitol-town                      # live until Ctrl-C
//! capitol-town --status             # the running town's last health
//! ```
//!
//! Run it from `flint-client/` with `cargo run -p capitol-core --bin capitol-town -- <flags>`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capitol_core::citizen::town::{self, Admin, Roster, Town};
use capitol_core::net::NetConfig;

const USAGE: &str = "\
capitol-town: the citizens of The Capitol

USAGE:
    capitol-town [run]           Live: every citizen on the roster plays its
                                 role until Ctrl-C. A citizen whose life ends
                                 starts again after a backoff; one whose key
                                 is refused retires.
    capitol-town mint --count <n>
                                 Make n citizens through the admin endpoint
                                 and add them, with their keys, to the roster
    capitol-town status          Print the running town's last health
                                 (also --status)

FLAGS:
    --roster <path>   The roster [default: town.toml]
    --server <url>    Server root [default: $CAPITOL_SERVER, else the
                      roster's `server`, else http://localhost:3000]
    --every <secs>    How often a running town prints and writes its health
                      [default: 60]
    --count <n>       With mint, how many citizens to make
    --admin <email>   With mint, an admin account to make them as
                      [default: $CAPITOL_ADMIN_EMAIL]. Its password is read
                      from $CAPITOL_ADMIN_PASSWORD only, never a flag.
    -h, --help        Print this

The roster holds keys, so keep it out of git (flint-client/.gitignore has
town.toml); town.example.toml shows its shape. Health goes beside it, in
town.status. Ctrl-C lets each citizen finish its step and stops; any order
it has resting on the book stays up, as a market maker's does. A second
Ctrl-C stops at once.";

/// How long a stopping town waits for its citizens to finish their steps.
const GRACE: Duration = Duration::from_secs(20);

#[derive(Debug, PartialEq)]
enum Action {
    Run,
    Mint { count: usize },
    Status,
}

#[derive(Debug)]
struct Options {
    action: Action,
    roster: PathBuf,
    server: Option<String>,
    every: Duration,
    admin: Option<String>,
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut action = None;
    let mut count = None;
    let mut options = Options {
        action: Action::Run,
        roster: PathBuf::from(town::ROSTER),
        server: std::env::var("CAPITOL_SERVER").ok().filter(|s| !s.is_empty()),
        every: Duration::from_secs(60),
        admin: std::env::var("CAPITOL_ADMIN_EMAIL").ok().filter(|s| !s.is_empty()),
    };
    let mut admin_given = false;
    let mut every_given = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        let mut set = |a: Action| match action.replace(a) {
            Some(_) => Err("run, mint and status go one at a time".to_string()),
            None => Ok(()),
        };
        match arg.as_str() {
            "run" => set(Action::Run)?,
            "mint" => set(Action::Mint { count: 0 })?,
            "status" | "--status" => set(Action::Status)?,
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
            "--admin" => {
                options.admin = Some(value()?);
                admin_given = true;
            }
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    options.action = match (action.unwrap_or(Action::Run), count) {
        (Action::Mint { .. }, Some(count)) => Action::Mint { count },
        (Action::Mint { .. }, None) => return Err("mint needs --count".into()),
        (_, Some(_)) => return Err("--count goes with mint".into()),
        (action, None) => action,
    };
    if admin_given && !matches!(options.action, Action::Mint { .. }) {
        return Err("--admin goes with mint".into());
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
    );
    for citizen in &roster.citizens[had..] {
        println!("made {} ({})", citizen.name, citizen.role);
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
    let mut town = Town::start(&roster, &server, stop.clone()).map_err(|e| format!("starting the town: {e}"))?;
    println!(
        "capitol-town: {} citizens on {server}, health every {:.0?} to {}",
        roster.citizens.len(),
        options.every,
        town::status_path(&roster.path).display()
    );
    let status = town::status_path(&roster.path);
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
            report(&town, &server, &status);
            next += options.every;
        }
        thread::sleep(Duration::from_millis(250));
    }
    println!("capitol-town: stopping; each citizen finishes its step");
    let stuck = town.wait(GRACE);
    report(&town, &server, &status);
    if stuck > 0 {
        println!("capitol-town: {stuck} still mid-step after {GRACE:?}; leaving them");
    }
    Ok(())
}

/// Prints the health and writes it beside the roster.
fn report(town: &Town, server: &str, path: &Path) {
    let header = format!("capitol-town · {server} · {} · {}", now_utc(), town.tally());
    let lines = town.health_lines();
    println!("── {header}");
    for line in &lines {
        println!("   {line}");
    }
    let text = format!("{header}\n{}\n", lines.join("\n"));
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("capitol-town: writing {}: {e}", path.display());
    }
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
        assert_eq!(o.admin.as_deref(), Some("chris@example.com"));
        assert_eq!(o.roster, PathBuf::from("t/x.toml"));
        assert_eq!(parse(args("--status")).unwrap().unwrap().action, Action::Status);
        assert_eq!(parse(args("status")).unwrap().unwrap().action, Action::Status);
        let o = parse(args("run --every 5 --server http://x:3001")).unwrap().unwrap();
        assert_eq!(o.every, Duration::from_secs(5));
        assert_eq!(o.server.as_deref(), Some("http://x:3001"));
        assert!(parse(args("--help")).unwrap().is_none());
    }

    #[test]
    fn bad_arguments_are_reported() {
        assert!(parse(args("mint")).is_err(), "mint without --count");
        assert!(parse(args("mint --count 0")).is_err());
        assert!(parse(args("--count 3")).is_err(), "--count without mint");
        assert!(parse(args("run --admin a@b")).is_err());
        assert!(parse(args("mint status --count 2")).is_err());
        assert!(parse(args("status --every 5")).is_err());
        assert!(parse(args("--every 0")).is_err());
        assert!(parse(args("--fly")).is_err());
    }
}
