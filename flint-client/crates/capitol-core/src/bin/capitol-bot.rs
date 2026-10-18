//! `capitol-bot`: headless players for The Capitol.
//!
//! ```text
//! capitol-bot                          # Bot1 gathers fiber, crafts rope, reports
//! capitol-bot --accounts 5 --wander    # five players strolling about, until Ctrl-C
//! capitol-bot --accounts 5 --wander --near Chris   # ...around Chris, following them
//! capitol-bot --prefix trader --trade Chris --ask  # a trading partner for Chris
//! capitol-bot --api-key cap_…          # the script, as the key's own player
//! capitol-bot --subscribe 30 --wander             # a subscriber, for a month
//! ```
//!
//! Run it from `flint-client/` with `cargo run -p capitol-core --bin capitol-bot -- <flags>`.

use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use capitol_core::bot::{Account, Bot, BotError, TradeOffer};
use capitol_core::net::NetConfig;

const USAGE: &str = "\
capitol-bot: headless players for The Capitol

USAGE:
    capitol-bot [FLAGS]

FLAGS:
    --server <url>      Server root [default: $CAPITOL_SERVER, else http://localhost:3000]
    --api-key <secret>  Play as the account that minted this key (POST
                        /account/api-keys) instead of a bot account. Its
                        scopes decide what it may do: the script needs play.
                        [default: $CAPITOL_API_KEY]
    --accounts <n>      How many bots, each on its own account [default: 1]
    --prefix <name>     Accounts are <name>1@bots.capitol.local and up, players
                        <Name>1 and up. Registered on first use. [default: bot]
    --wander            Stroll around instead of running the script
    --near <player>     Wander around this player, and follow them
    --radius <units>    How far a wanderer strays from where it started [default: 120]
    --seconds <s>       Stop wandering after this long [default: never]
    --gather <n>        Instead of the script, gather until the inventory holds
                        n fiber, and stop (to stock an account for a test)
    --trade <player>    Stay beside this player and trade with them: answer
                        their requests, offer, and accept once they offer
    --ask               With --trade, ask them too whenever idle beside them
    --offer-strands <n> With --trade, Strands to offer [default: 10]
    --offer <item_type> With --trade, also offer every stack of this type
    --subscribe <days>  Subscribe each bot for this many days first, with the
                        server's mock activation (MOCK_SUBSCRIPTIONS, on by
                        default in a debug build of the backend)
    -h, --help          Print this

The script logs in (or registers), scouts for fiber, walks to the nearest
node, extracts until it holds enough for rope_basic, crafts the rope and
reports. The server allows one session per account, so two copies with the
same --prefix log each other out. A key isn't a session: it leaves its
owner signed in, though a play key's socket still takes the player over
from their client (#151).";

struct Options {
    server: String,
    api_key: Option<String>,
    accounts: usize,
    prefix: String,
    wander: bool,
    near: Option<String>,
    radius: f64,
    seconds: Option<f64>,
    gather: Option<i32>,
    trade: Option<String>,
    ask: bool,
    offer: TradeOffer,
    subscribe: Option<i32>,
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options {
        server: std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into()),
        api_key: std::env::var("CAPITOL_API_KEY").ok().filter(|k| !k.is_empty()),
        accounts: 1,
        prefix: "bot".into(),
        wander: false,
        near: None,
        radius: 120.0,
        seconds: None,
        gather: None,
        trade: None,
        ask: false,
        offer: TradeOffer {
            strands: 10,
            item_type: None,
        },
        subscribe: None,
    };
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--server" => options.server = value()?,
            "--api-key" => options.api_key = Some(value()?),
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
            "--gather" => options.gather = Some(number(&flag, &value()?)?),
            "--trade" => options.trade = Some(value()?),
            "--ask" => options.ask = true,
            "--offer-strands" => options.offer.strands = number(&flag, &value()?)?,
            "--offer" => options.offer.item_type = Some(value()?),
            "--subscribe" => {
                let days = number(&flag, &value()?)?;
                if !(1..=3650).contains(&days) {
                    return Err("--subscribe takes 1 to 3650 days".into());
                }
                options.subscribe = Some(days);
            }
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if options.api_key.is_some() && options.accounts != 1 {
        return Err("--api-key plays one account; it doesn't go with --accounts".into());
    }
    if options.api_key.is_some() && options.subscribe.is_some() {
        return Err("--subscribe needs a sign-in; an API key can't subscribe its account".into());
    }
    if options.near.is_some() && !options.wander {
        return Err("--near goes with --wander".into());
    }
    if options.trade.is_none()
        && (options.ask || options.offer.item_type.is_some() || options.offer.strands != 10)
    {
        return Err("--ask, --offer and --offer-strands go with --trade".into());
    }
    Ok(Some(options))
}

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{flag}: {value:?} isn't a number"))
}

fn run(options: &Options, n: usize) -> Result<(), BotError> {
    let config = NetConfig::with_base_url(&options.server);
    let mut bot = match &options.api_key {
        Some(key) => Bot::with_api_key(config, key)?,
        None => Bot::sign_in(config, &Account::numbered(&options.prefix, n))?,
    };
    if let Some(days) = options.subscribe {
        bot.subscribe(days)?;
    }
    let deadline = options
        .seconds
        .map(|s| Instant::now() + Duration::from_secs_f64(s));
    if let Some(name) = &options.trade {
        bot.trade_with(name, &options.offer, options.ask, deadline)
    } else if options.wander {
        bot.wander(options.radius, options.near.as_deref(), deadline)
    } else if let Some(want) = options.gather {
        bot.gather("fiber", want)?;
        bot.say(format!("holding {want} fiber or more"));
        Ok(())
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
                        let who = match &options.api_key {
                            Some(key) => key.chars().take(12).collect(),
                            None => Account::numbered(&options.prefix, n).name,
                        };
                        eprintln!("{who}: {e}");
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
        assert_eq!(
            parse(args("--gather 12")).unwrap().unwrap().gather,
            Some(12)
        );
        assert_eq!(o.near.as_deref(), Some("Ada"));
        let k = parse(args("--api-key cap_abc --wander")).unwrap().unwrap();
        assert_eq!(k.api_key.as_deref(), Some("cap_abc"));
        assert!(parse(args("--help")).unwrap().is_none());
        let t = parse(args("--trade Ada --ask --offer rope --offer-strands 25"))
            .unwrap()
            .unwrap();
        assert_eq!(t.trade.as_deref(), Some("Ada"));
        assert!(t.ask);
        assert_eq!(t.offer.strands, 25);
        assert_eq!(t.offer.item_type.as_deref(), Some("rope"));
        assert_eq!(
            parse(args("--subscribe 30 --wander"))
                .unwrap()
                .unwrap()
                .subscribe,
            Some(30)
        );
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
        assert!(parse(args("--ask")).is_err(), "--ask without --trade");
        assert!(
            parse(args("--api-key cap_x --accounts 2")).is_err(),
            "a key is one account"
        );
        assert!(parse(args("--subscribe 0")).is_err());
        assert!(parse(args("--subscribe forever")).is_err());
        assert!(
            parse(args("--api-key cap_x --subscribe 30")).is_err(),
            "a key can't subscribe its account"
        );
    }
}
