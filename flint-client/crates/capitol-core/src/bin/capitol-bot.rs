//! `capitol-bot`: headless players for The Capitol.
//!
//! ```text
//! capitol-bot                          # Bot1 gathers fiber, crafts rope, reports, with its round trip
//! capitol-bot --accounts 5 --wander    # five players strolling about, until Ctrl-C
//! capitol-bot --accounts 5 --wander --near Chris   # ...around Chris, following them
//! capitol-bot --prefix trader --trade Chris --ask  # a trading partner for Chris
//! capitol-bot --api-key cap_…          # the script, as the key's own player
//! capitol-bot --subscribe 30 --wander             # a subscriber, for a month
//! capitol-bot --wander --listen --hail          # among citizens, printing what they say (S1.6)
//! capitol-bot --sim 20 --hours 2               # the economy sim (E2.7), with a report
//! capitol-bot --prefix maker --market fiber,plank,iron_ingot --fund   # a market maker (E3.8)
//! capitol-bot --prefix newcomer --tutorial      # walk the road in and claim its deed (#450)
//! capitol-bot --prefix poster --grant 2000 --commission log:20:9 --commission iron_ore:10:30:B
//!                                               # a person posting on the hall's board (S2.5)
//! ```
//!
//! Run it from `flint-client/` with `cargo run -p capitol-core --bin capitol-bot -- <flags>`.

use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use capitol_core::bot::{Account, Bot, BotError, TradeOffer};
use capitol_core::market_maker::{self, Maker, MakerConfig};
use capitol_core::net::NetConfig;
use capitol_core::protocol::rest::{GrantRequest, PostCommission};
use capitol_core::protocol::rules::market::grade_floor;
use capitol_core::protocol::tutorial;
use capitol_core::state::StateEvent;
use capitol_core::sim::run::{self as sim, SimConfig};
use capitol_core::sim::{Gather, Role};

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
    --at <x,y>          Walk here first, round the water
    --listen            Print the talk the bot hears, local and Capitol
    --hail              Say hello by name to a citizen within earshot, every
                        20 s at most, to hear it answer (with --listen)
    --near <player>     Wander around this player, and follow them
    --radius <units>    How far a wanderer strays from where it started [default: 120]
    --seconds <s>       Stop wandering, or making a market, after this long
                        [default: never]
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
    --market <items>    Make a market in these items (comma-separated): quote
                        both sides round the last trade, or D5's value before
                        one, and re-quote as orders fill. Stops after --seconds,
                        if given, and takes its orders down; Ctrl-C leaves them
                        up, and the next run picks them up again.
    --spread <pct>      With --market, the whole spread in percent [default: 10]
    --depth <n>         With --market, price levels a side [default: 2]
    --size <n>          With --market, units a level, at least; a cheap item's
                        levels are bigger, so each order is worth 100 Strands
                        [default: 10]
    --target <n>        With --market, units of each item to hold: it bids up
                        to twice this, and leans its prices to get back to it
                        [default: 40]
    --fund              With --market, top up to 20000 Strands and --target
                        units of each item first, as a grant. Only a server
                        with DEV_GRANTS=on gives one: never a shared server.
    --tutorial          Walk the road in, the tutorial, on the account as a
                        citizen does (#450): every step the server judges,
                        the few only a screen shows reported, each said as
                        it's done, to the claim with the deed it earns at
                        the Deeds Office and the steps of settling in. Stops
                        once it's walked, or after --seconds, and fails if
                        the deed wasn't claimed. Its card step wants someone
                        else about: run it in a town
    --commission <item:quantity:price[:grade]>
                        Post a commission on the hall's board as a person
                        (Commons S2.5): quantity of item at price Strands
                        each, of grade A to F or better [default grade: F],
                        due in 3 days. Repeat it for more. Then stay, printing
                        each claim, delivery, lapse and expiry, until every
                        one is filled or --seconds is up
    --grant <strands>   Top up with this many Strands first, as a grant. Only
                        a server with DEV_GRANTS=on gives one
    --sim <bots>        Run the economy sim: this many bots, each with a role
                        (gatherer, crafter or builder), gather, craft and build
                        for --hours game hours, then write a report to --out.
                        Start the backend with SIM_TICK_SCALE=10 so a game hour
                        takes six minutes, and RATE_LIMITS=off, since bots at
                        that pace go over every limit. Accounts are
                        <prefix>1@bots.capitol.local and up, prefix `sim` unless
                        --prefix says otherwise.
    --hours <h>         With --sim, game hours to run [default: 2]
    --out <dir>         With --sim, where the report goes [default: renders/sim]
    --makers <n>        With --sim, market makers to join the swarm, on
                        accounts <prefix>maker1 and up. The swarm sells to them
                        and buys from them. They need DEV_GRANTS=on for their
                        stock; 0 runs the swarm with no market [default: 2]
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
    at: Option<(f64, f64)>,
    listen: bool,
    hail: bool,
    near: Option<String>,
    radius: f64,
    seconds: Option<f64>,
    gather: Option<i32>,
    trade: Option<String>,
    ask: bool,
    offer: TradeOffer,
    subscribe: Option<i32>,
    sim: Option<usize>,
    hours: f64,
    out: String,
    makers: usize,
    market: Option<MakerConfig>,
    fund: bool,
    tutorial: bool,
    commissions: Vec<PostCommission>,
    grant: Option<i64>,
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options {
        server: std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into()),
        api_key: std::env::var("CAPITOL_API_KEY").ok().filter(|k| !k.is_empty()),
        accounts: 1,
        prefix: "bot".into(),
        wander: false,
        at: None,
        listen: false,
        hail: false,
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
        sim: None,
        hours: 2.0,
        out: "renders/sim".into(),
        makers: 2,
        market: None,
        fund: false,
        tutorial: false,
        commissions: Vec::new(),
        grant: None,
    };
    let mut maker = MakerConfig::default();
    let mut maker_tuned = false;
    let mut prefix_given = false;
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
                prefix_given = true;
                options.prefix = value()?;
                if options.prefix.is_empty() || !options.prefix.chars().all(char::is_alphanumeric) {
                    return Err("--prefix must be letters and digits".into());
                }
            }
            "--wander" => options.wander = true,
            "--at" => {
                let value = value()?;
                let (x, y) = value.split_once(',').ok_or("--at takes x,y")?;
                options.at = Some((number(&flag, x.trim())?, number(&flag, y.trim())?));
            }
            "--listen" => options.listen = true,
            "--hail" => options.hail = true,
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
            "--sim" => {
                let bots = number(&flag, &value()?)?;
                if bots == 0 {
                    return Err("--sim needs at least 1 bot".into());
                }
                options.sim = Some(bots);
            }
            "--hours" => {
                options.hours = number(&flag, &value()?)?;
                if !(options.hours > 0.0 && options.hours <= 48.0) {
                    return Err("--hours takes more than 0 and at most 48".into());
                }
            }
            "--out" => options.out = value()?,
            "--makers" => options.makers = number(&flag, &value()?)?,
            "--market" => {
                let items: Vec<String> = value()?
                    .split(',')
                    .map(str::trim)
                    .filter(|i| !i.is_empty())
                    .map(String::from)
                    .collect();
                if items.is_empty() {
                    return Err("--market needs at least one item".into());
                }
                maker.items = items;
            }
            "--spread" => {
                let pct: f64 = number(&flag, &value()?)?;
                if !(pct > 0.0 && pct <= 100.0) {
                    return Err("--spread takes more than 0 and at most 100 percent".into());
                }
                maker.spread = pct / 100.0;
                maker_tuned = true;
            }
            "--depth" => {
                maker.depth = number(&flag, &value()?)?;
                if !(1..=10).contains(&maker.depth) {
                    return Err("--depth takes 1 to 10".into());
                }
                maker_tuned = true;
            }
            "--size" => {
                maker.size = number(&flag, &value()?)?;
                if maker.size < 1 {
                    return Err("--size must be at least 1".into());
                }
                maker_tuned = true;
            }
            "--target" => {
                maker.target = number(&flag, &value()?)?;
                if maker.target < 1 {
                    return Err("--target must be at least 1".into());
                }
                maker_tuned = true;
            }
            "--fund" => {
                options.fund = true;
                maker_tuned = true;
            }
            "--tutorial" => options.tutorial = true,
            "--commission" => options.commissions.push(commission(&value()?)?),
            "--grant" => options.grant = Some(number(&flag, &value()?)?),
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if !maker.items.is_empty() {
        options.market = Some(maker);
    } else if maker_tuned {
        return Err("--spread, --depth, --size, --target and --fund go with --market".into());
    }
    if options.market.is_some()
        && (options.sim.is_some() || options.wander || options.gather.is_some() || options.trade.is_some())
    {
        return Err("--market is a bot of its own: not with --sim, --wander, --gather or --trade".into());
    }
    if options.tutorial
        && (options.sim.is_some()
            || options.market.is_some()
            || options.wander
            || options.gather.is_some()
            || options.trade.is_some())
    {
        return Err("--tutorial walks the road on its own: not with --sim, --market, --wander, --gather or --trade".into());
    }
    if options.sim.is_some() {
        if options.api_key.is_some()
            || options.accounts != 1
            || options.wander
            || options.gather.is_some()
            || options.trade.is_some()
            || options.subscribe.is_some()
        {
            return Err("--sim runs its own bots; it only goes with --server, --prefix, --hours, --out and --makers".into());
        }
        if !prefix_given {
            options.prefix = "sim".into();
        }
    } else if options.hours != 2.0 || options.out != "renders/sim" || options.makers != 2 {
        return Err("--hours, --out and --makers go with --sim".into());
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

/// Days a commission posted from the command line runs.
const COMMISSION_DAYS: i64 = 3;

/// `log:20:9` or `iron_ore:10:30:B`, as `--commission` takes it.
fn commission(value: &str) -> Result<PostCommission, String> {
    let parts: Vec<&str> = value.split(':').collect();
    let (item, quantity, price, grade) = match parts[..] {
        [item, quantity, price] => (item, quantity, price, "F"),
        [item, quantity, price, grade] => (item, quantity, price, grade),
        _ => return Err(format!("--commission: {value:?} isn't item:quantity:price[:grade]")),
    };
    let min_quality = grade_floor(grade).ok_or_else(|| format!("--commission: {grade:?} isn't a grade, A to F"))?;
    Ok(PostCommission {
        item_type: item.to_string(),
        min_quality,
        quantity: number("--commission", quantity)?,
        price: number("--commission", price)?,
        due_days: COMMISSION_DAYS,
    })
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
    bot.print_talk(options.listen);
    bot.hail_citizens(options.hail);
    if let Some((x, y)) = options.at {
        bot.walk_to(x, y)?;
    }
    let deadline = options
        .seconds
        .map(|s| Instant::now() + Duration::from_secs_f64(s));
    if let Some(config) = &options.market {
        return make_market(&mut bot, config, options.fund, deadline);
    }
    if options.tutorial {
        return walk_the_road(bot, n, deadline);
    }
    if let Some(strands) = options.grant {
        let granted = bot.grant(GrantRequest { strands, items: Vec::new() })?;
        bot.log(format!("granted {} Strands", granted.strands));
    }
    if !options.commissions.is_empty() {
        return post_commissions(&mut bot, &options.commissions, deadline);
    }
    // These loops can start over, so they carry on through a dropped
    // connection or a server restart (C0.6).
    if let Some(name) = &options.trade {
        bot.stay_up(deadline, |bot| bot.trade_with(name, &options.offer, options.ask, deadline))
    } else if options.wander {
        bot.stay_up(deadline, |bot| bot.wander(options.radius, options.near.as_deref(), deadline))
    } else if let Some(want) = options.gather {
        bot.gather("fiber", want)?;
        bot.log(format!("holding {want} fiber or more"));
        Ok(())
    } else {
        println!("{}", bot.run_script()?);
        println!("{}: {}", bot.name(), bot.net_stats());
        Ok(())
    }
}

/// The road in, walked as a fiber gatherer would, living where its seed
/// says (#450): the steps it did, and whether it claimed its deed.
fn walk_the_road(bot: Bot, n: usize, deadline: Option<Instant>) -> Result<(), BotError> {
    let name = bot.name().to_string();
    let seed = name.bytes().fold(n as u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    let progress = sim::walk_the_road(bot, Role::Gatherer(Gather::Fiber), seed, deadline)?;
    let done = progress.done.iter().count();
    let of = tutorial::Step::ALL.len();
    let claimed = progress.done.has(tutorial::Step::ClaimHome) && progress.grant == tutorial::Grant::Spent;
    println!(
        "{name}: {done} of {of} steps of the road in; {}",
        if claimed { "its deed is claimed" } else { "its deed isn't claimed" }
    );
    if let Some(step) = progress.done.current() {
        println!("{name}: still to do: {}", sim::road_in::does(step));
    }
    if claimed {
        Ok(())
    } else {
        Err(BotError::Failed("the road in ended without a claim".into()))
    }
}

/// Posts `commissions` at the hall, as a person would, then prints what
/// comes of them until each is filled or `deadline` (S2.5).
fn post_commissions(bot: &mut Bot, commissions: &[PostCommission], deadline: Option<Instant>) -> Result<(), BotError> {
    bot.go_to_exchange()?;
    bot.record();
    let mut open = 0;
    for post in commissions {
        let posted = bot.post_commission(post.clone())?;
        bot.log(format!("fee {}, due {}", posted.fee, posted.due_at.format("%Y-%m-%d %H:%M UTC")));
        open += 1;
    }
    while open > 0 && deadline.is_none_or(|d| Instant::now() < d) {
        bot.idle(Duration::from_secs(1))?;
        for change in bot.take_journal() {
            let line = match change {
                StateEvent::CommissionClaimed {
                    item_type,
                    claimant_name,
                    claimant_citizen,
                    quantity,
                    ..
                } => format!("{claimant_name}{} claimed {quantity} {item_type}", mark(claimant_citizen)),
                StateEvent::CommissionDelivered {
                    item_type,
                    deliverer_name,
                    deliverer_citizen,
                    units,
                    paid,
                    left,
                } => {
                    if left == 0 {
                        open -= 1;
                    }
                    format!(
                        "{deliverer_name}{} delivered {} {item_type} for {paid} Strands; {}",
                        mark(deliverer_citizen),
                        units.count(),
                        if left == 0 { "filled".to_string() } else { format!("{left} left") }
                    )
                }
                StateEvent::ClaimLapsed {
                    item_type,
                    claimant_name,
                    quantity,
                    delivered,
                    ..
                } => format!("{claimant_name}'s claim on {quantity} {item_type} lapsed, {delivered} in"),
                StateEvent::CommissionExpired {
                    item_type, delivered, quantity, refunded,
                } => {
                    open -= 1;
                    format!("the commission for {item_type} ran out, {delivered} of {quantity} in; {refunded} Strands back")
                }
                _ => continue,
            };
            bot.log(line);
        }
    }
    bot.log(if open == 0 { "every commission is filled or done".to_string() } else { format!("{open} still open") });
    Ok(())
}

fn mark(citizen: bool) -> &'static str {
    if citizen {
        " (a citizen)"
    } else {
        ""
    }
}

/// Strands a funded maker starts with.
const FUND_STRANDS: i64 = 20_000;
/// The quality of a funded maker's stock: Q50 (grade D), ordinary stuff.
const FUND_QUALITY: i32 = 50;

fn make_market(bot: &mut Bot, config: &MakerConfig, fund: bool, deadline: Option<Instant>) -> Result<(), BotError> {
    if fund {
        match market_maker::fund(bot, config, FUND_STRANDS, FUND_QUALITY)? {
            Some(granted) => bot.log(format!("funded: {} Strands, stock in the market box", granted.strands)),
            None => bot.log("already funded"),
        }
    }
    let mut maker = Maker::new(config.clone());
    // Orders are placed at the Exchange hall (D6)
    bot.go_to_exchange()?;
    maker.start(bot)?;
    for item in &config.items {
        bot.log(format!("{item} round {}", maker.reference(item).unwrap_or(0)));
    }
    bot.stay_up(deadline, |bot| maker.run(bot, deadline))?;
    maker.withdraw(bot)?;
    let s = &maker.stats;
    bot.log(format!(
        "done: {} orders placed, {} cancelled, {} units bought, {} sold",
        s.placed, s.cancelled, s.bought, s.sold
    ));
    Ok(())
}

fn run_sim(options: &Options, bots: usize) -> ExitCode {
    let config = SimConfig {
        server: options.server.clone(),
        bots,
        hours: options.hours,
        prefix: options.prefix.clone(),
        out_dir: options.out.clone().into(),
        makers: options.makers,
    };
    println!(
        "capitol-bot: {bots} bots for {} game hours against {}",
        options.hours, options.server
    );
    match sim::run(&config) {
        Ok((report, md, csv)) => {
            println!("{}", report.to_markdown());
            println!("wrote {} and {}", md.display(), csv.display());
            if report.errors.is_empty() {
                ExitCode::SUCCESS
            } else {
                eprintln!("capitol-bot: {} bots stopped early", report.errors.len());
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("capitol-bot: {e}");
            ExitCode::FAILURE
        }
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
    if let Some(bots) = options.sim {
        return run_sim(&options, bots);
    }
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
    fn a_commission_is_item_quantity_price_and_a_grade() {
        let o = parse(args("--grant 500 --commission log:20:9 --commission iron_ore:10:30:B")).unwrap().unwrap();
        assert_eq!(o.grant, Some(500));
        assert_eq!(o.commissions.len(), 2);
        assert_eq!((o.commissions[0].item_type.as_str(), o.commissions[0].quantity, o.commissions[0].price), ("log", 20, 9));
        assert_eq!(o.commissions[0].min_quality, 0);
        assert_eq!(o.commissions[1].min_quality, 75);
        assert!(parse(args("--commission log:20")).is_err());
        assert!(parse(args("--commission log:20:9:Z")).is_err());
        assert!(parse(args("--commission log:many:9")).is_err());
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
    fn sim_flags_parse() {
        let o = parse(args("--sim 20 --hours 0.5 --out x/y")).unwrap().unwrap();
        assert_eq!(o.sim, Some(20));
        assert_eq!(o.hours, 0.5);
        assert_eq!(o.out, "x/y");
        assert_eq!(o.prefix, "sim");
        let o = parse(args("--sim 3 --prefix swarm")).unwrap().unwrap();
        assert_eq!((o.prefix.as_str(), o.hours), ("swarm", 2.0));
        assert!(parse(args("--sim 0")).is_err());
        assert!(parse(args("--sim 2 --hours 0")).is_err());
        assert!(parse(args("--sim 2 --wander")).is_err());
        assert!(parse(args("--sim 2 --accounts 4")).is_err());
        assert!(parse(args("--hours 3")).is_err(), "--hours without --sim");
    }

    #[test]
    fn market_flags_parse() {
        let o = parse(args("--market fiber,plank,iron_ingot --spread 20 --depth 3 --fund"))
            .unwrap()
            .unwrap();
        let m = o.market.unwrap();
        assert_eq!(m.items, ["fiber", "plank", "iron_ingot"]);
        assert_eq!((m.spread, m.depth), (0.2, 3));
        assert!(o.fund);
        assert!(parse(args("--spread 20")).is_err(), "--spread without --market");
        assert!(parse(args("--fund")).is_err(), "--fund without --market");
        assert!(parse(args("--market ,")).is_err());
        assert!(parse(args("--market plank --spread 0")).is_err());
        assert!(parse(args("--market plank --depth 0")).is_err());
        assert!(parse(args("--market plank --wander")).is_err());
        assert_eq!(parse(args("--sim 4 --makers 0")).unwrap().unwrap().makers, 0);
        assert!(parse(args("--makers 3")).is_err(), "--makers without --sim");
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
