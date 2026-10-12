//! `capitol-bot`: headless players for The Capitol.
//!
//! ```text
//! capitol-bot                          # Bot1 gathers fiber, crafts rope, reports
//! capitol-bot --accounts 5 --wander    # five players strolling about, until Ctrl-C
//! capitol-bot --accounts 5 --wander --near Chris   # ...around Chris, following them
//! ```
//!
//! Run it from `flint-client/` with `cargo run -p capitol-core --bin capitol-bot -- <flags>`.

use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use capitol_core::bot::{Account, Bot, BotError};
use capitol_core::net::NetConfig;

const USAGE: &str = "\
capitol-bot: headless players for The Capitol

USAGE:
    capitol-bot [FLAGS]

FLAGS:
    --server <url>      Server root [default: $CAPITOL_SERVER, else http://localhost:3000]
    --accounts <n>      How many bots, each on its own account [default: 1]
    --prefix <name>     Accounts are <name>1@bots.capitol.local and up, players
                        <Name>1 and up. Registered on first use. [default: bot]
    --wander            Stroll around instead of running the script
    --near <player>     Wander around this player, and follow them
    --radius <units>    How far a wanderer strays from where it started [default: 120]
    --seconds <s>       Stop wandering after this long [default: never]
    -h, --help          Print this

The script logs in (or registers), scouts for fiber, walks to the nearest
node, extracts until it holds enough for rope_basic, crafts the rope and
reports. The server allows one session per account, so two copies with the
same --prefix log each other out.";

struct Options {
    server: String,
    accounts: usize,
    prefix: String,
    wander: bool,
    near: Option<String>,
    radius: f64,
    seconds: Option<f64>,
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options {
        server: std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into()),
        accounts: 1,
        prefix: "bot".into(),
        wander: false,
        near: None,
        radius: 120.0,
        seconds: None,
    };
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--server" => options.server = value()?,
            "--accounts" => {
                options.accounts = number(&flag, &value()?)?;
                if options.accounts == 0 {
                    return Err("--accounts must be at least 1".into());
                }
            }
            "--prefix" => {
                options.prefix = value()?;
                if options.prefix.is_empty() || !options.prefix.chars().all(char::is_alphanumeric) {
                    return Err("--prefix must be letters and digits".into());
                }
            }
            "--wander" => options.wander = true,
            "--near" => options.near = Some(value()?),
            "--radius" => options.radius = number(&flag, &value()?)?,
            "--seconds" => options.seconds = Some(number(&flag, &value()?)?),
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if options.near.is_some() && !options.wander {
        return Err("--near goes with --wander".into());
    }
    Ok(Some(options))
}

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{flag}: {value:?} isn't a number"))
}

fn run(options: &Options, n: usize) -> Result<(), BotError> {
    let account = Account::numbered(&options.prefix, n);
    let mut bot = Bot::sign_in(NetConfig::with_base_url(&options.server), &account)?;
    if options.wander {
        let deadline = options
            .seconds
            .map(|s| Instant::now() + Duration::from_secs_f64(s));
        bot.wander(options.radius, options.near.as_deref(), deadline)
    } else {
        println!("{}", bot.run_script()?);
        Ok(())
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
            eprintln!("capitol-bot: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let options = &options;
    let failures = thread::scope(|scope| {
        let bots: Vec<_> = (1..=options.accounts)
            .map(|n| {
                // Staggered, so a fleet doesn't hash its passwords all at once.
                thread::sleep(Duration::from_millis(150));
                scope.spawn(move || {
                    let result = run(options, n);
                    if let Err(e) = &result {
                        eprintln!("{}: {e}", Account::numbered(&options.prefix, n).name);
                    }
                    result.is_err()
                })
            })
            .collect();
        bots.into_iter()
            .map(|bot| bot.join().unwrap_or(true))
            .filter(|&failed| failed)
            .count()
    });
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        eprintln!("capitol-bot: {failures} of {} failed", options.accounts);
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn flags_parse() {
        let o = parse(args(
            "--accounts 5 --wander --radius 80 --seconds 30 --prefix scout --near Ada",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(o.accounts, 5);
        assert!(o.wander);
        assert_eq!(o.radius, 80.0);
        assert_eq!(o.seconds, Some(30.0));
        assert_eq!(o.prefix, "scout");
        assert_eq!(o.near.as_deref(), Some("Ada"));
        assert!(parse(args("--help")).unwrap().is_none());
    }

    #[test]
    fn bad_flags_are_reported() {
        assert!(parse(args("--accounts")).is_err());
        assert!(parse(args("--accounts zero")).is_err());
        assert!(parse(args("--accounts 0")).is_err());
        assert!(parse(args("--prefix a@b")).is_err());
        assert!(parse(args("--fly")).is_err());
        assert!(
            parse(args("--near Ada")).is_err(),
            "--near without --wander"
        );
    }
}
