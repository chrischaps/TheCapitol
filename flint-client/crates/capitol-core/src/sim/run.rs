//! The swarm: one thread a bot, each signing in, waiting for the others, then
//! playing its [`Role`] until the game hours are up. The market makers run
//! alongside, one thread each, from a grant.
//!
//! `capitol-town`'s citizens (C1.1) play the same loop, one at a time
//! through [`live`], for as long as their town runs. They also meet people
//! (C1.5, [`people`]): they answer trades, leave standing bids for what
//! their trade needs, and keep a stall when they make more than the hall
//! takes. The swarm's bots don't, so the E2.7 sim's numbers stay comparable.
//! A town's merchants (C1.6, [`merchant`]) earn a float and make a market.

mod merchant;
mod people;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use capitol_protocol::rest::{PlaceOrder, ResourceNodeInfo, StationInfo};
use capitol_protocol::rules::market::{OrderSide, OrderStatus};
use capitol_protocol::rules::Zone;
use uuid::Uuid;

use super::plan::{Holdings, Step};
use super::report::{BotRun, Entry, Report, MAKER_ROLE};
use super::{Errand, Role, Trade, MARKET_ITEMS};
use crate::bot::{Account, Bot, BotError};
use crate::catalog::Catalog;
use crate::citizen::decide::{Choice, Decision, Mind, Place, Question, Situation, When, Who};
use crate::citizen::meet;
use crate::citizen::persona::{Market, Persona};
use crate::citizen::schedule::Phase;
use crate::citizen::town::{Ended, Standing, Tether};
use crate::market::MarketEvent;
use crate::market_maker::{self, Maker, MakerConfig};
use crate::net::NetConfig;
use crate::poll::Fetch;
use crate::state::StateEvent;

/// A deed's price until a plot listing says otherwise: Rural's.
const DEFAULT_DEED: i64 = 250;
/// Where builders look for land, nearest the rural spawn first.
const PLOT_ZONES: [&str; 2] = ["rural", "suburban"];
/// How far apart stations go on a plot: just over `STATION_MIN_SPACING`.
const SPOT_SPACING: f64 = 55.0;
/// Walks out to look for a resource before the gather gives up.
const MAX_EXPLORES: usize = 6;
/// Failures in a row before a bot sits out a game minute.
const FAILURES_BEFORE_REST: u32 = 5;
/// How long a bot's order waits for a fill before it's taken down. Matching
/// runs on the wall clock (1 s), not the game's.
const FILL_WAIT: Duration = Duration::from_secs(3);
/// What a sim maker is granted: Strands, and units of each item at Q50.
const MAKER_STRANDS: i64 = 50_000;
const MAKER_QUALITY: i32 = 50;
/// How long a citizen waits, at most, before it notices its town saying stop.
const STOP_CHECK: Duration = Duration::from_secs(1);

/// How a sim is run.
#[derive(Debug, Clone)]
pub struct SimConfig {
    pub server: String,
    pub bots: usize,
    /// Game hours.
    pub hours: f64,
    /// Accounts are `<prefix>1@bots.capitol.local` and up.
    pub prefix: String,
    /// Where `sim-<stamp>.md` and `.csv` go.
    pub out_dir: PathBuf,
    /// Market makers, on accounts `<prefix>maker1@…` and up. With none,
    /// nobody trades.
    pub makers: usize,
}

/// How the sim's makers quote: [`MARKET_ITEMS`], a 10% spread, two levels.
fn maker_config() -> MakerConfig {
    MakerConfig {
        items: MARKET_ITEMS.iter().map(|i| i.to_string()).collect(),
        ..MakerConfig::default()
    }
}

/// Runs the swarm and writes the report. Returns it, with the paths written.
pub fn run(config: &SimConfig) -> Result<(Report, PathBuf, PathBuf), String> {
    let net = || NetConfig::with_base_url(&config.server);
    let roles = Role::assign(config.bots);
    // The scale, from the first bot's health check (all share one server).
    let scale = Arc::new(OnceLock::<f64>::new());
    let start = Arc::new(OnceLock::<Instant>::new());
    let ready = Arc::new(Barrier::new(config.bots + config.makers));
    let trading = config.makers > 0;
    let runs: Vec<BotRun> = thread::scope(|scope| {
        let mut handles: Vec<_> = (1..=config.makers)
            .map(|n| {
                let (scale, start, ready) = (scale.clone(), start.clone(), ready.clone());
                let account = Account::numbered(&format!("{}maker", config.prefix), n);
                thread::sleep(Duration::from_millis(150));
                scope.spawn(move || {
                    let hours = config.hours;
                    let funded = Bot::sign_in(net(), &account).and_then(|bot| {
                        let mut bot = bot.quiet();
                        market_maker::fund(&mut bot, &maker_config(), MAKER_STRANDS, MAKER_QUALITY)?;
                        let mut maker = Maker::new(maker_config());
                        bot.go_to_exchange()?;
                        maker.start(&mut bot)?;
                        Ok((bot, maker))
                    });
                    ready.wait();
                    let started = *start.get_or_init(Instant::now);
                    match funded {
                        Ok((bot, maker)) => {
                            // The swarm's first bot sets the scale before the barrier
                            let scale = scale.get().copied().unwrap_or(1.0);
                            make_market(bot, maker, started + Duration::from_secs_f64(hours * 3600.0 / scale), started, scale)
                        }
                        Err(e) => BotRun {
                            name: account.name.clone(),
                            role: MAKER_ROLE.into(),
                            entries: Vec::new(),
                            strands_start: 0,
                            strands_end: 0,
                            error: Some(format!("starting (does the server have DEV_GRANTS=on?): {e}")),
                        },
                    }
                })
            })
            .collect();
        handles.extend(roles
            .iter()
            .enumerate()
            .map(|(i, &role)| {
                let (scale, start, ready) = (scale.clone(), start.clone(), ready.clone());
                let account = Account::numbered(&config.prefix, i + 1);
                // Staggered, so the swarm doesn't hash its passwords at once.
                thread::sleep(Duration::from_millis(150));
                scope.spawn(move || {
                    let signed_in = Bot::sign_in(net(), &account).and_then(|mut bot| {
                        let health = bot.health()?;
                        let _ = scale.set(health.sim_tick_scale.unwrap_or(1.0));
                        if trading {
                            for item in MARKET_ITEMS {
                                bot.watch_market(item)?;
                            }
                        }
                        Ok(bot.quiet())
                    });
                    ready.wait();
                    let started = *start.get_or_init(Instant::now);
                    let scale = scale.get().copied().unwrap_or(1.0);
                    match signed_in {
                        Ok(bot) => SimBot::new(bot, role, i as u64, scale, started, trading, Mind::rules()).play(config.hours),
                        Err(e) => BotRun {
                            name: account.name.clone(),
                            role: role.to_string(),
                            entries: Vec::new(),
                            strands_start: 0,
                            strands_end: 0,
                            error: Some(format!("signing in: {e}")),
                        },
                    }
                })
            }));
        handles
            .into_iter()
            .map(|h| h.join().expect("a sim bot panicked"))
            .collect()
    });
    let scale = scale.get().copied().unwrap_or(1.0);
    let wall = start.get().map_or(0.0, |s| s.elapsed().as_secs_f64());
    let report = Report::build(&runs, config.hours, scale, wall);
    let (md, csv) = write(&report, &config.out_dir).map_err(|e| format!("writing the report: {e}"))?;
    Ok((report, md, csv))
}

/// A citizen's day (C1.1, C1.2): `bot` plays `role` as `persona` would,
/// with `mind` making its decisions (C1.3), until bedtime or its town says
/// stop. It trades with whatever is on the book, at the hall, as the swarm
/// does with its makers; the town grants nobody anything (D2), so the book
/// holds only what players and citizens put there.
///
/// The day follows the persona's schedule on the server's clock: work, the
/// market's hours at the hall or the bazaar, work again, and home at dusk,
/// buying a lot to live on first if it has saved enough. `Ok` once home for
/// the night or stopped; `Err` when the bot couldn't start, or the server
/// won't have it back.
pub(crate) fn live(mut bot: Bot, role: Role, persona: &Persona, seed: u64, mind: Mind, town: &Tether) -> Result<Ended, BotError> {
    let scale = bot.health()?.sim_tick_scale.unwrap_or(1.0);
    for item in MARKET_ITEMS {
        bot.watch_market(item)?;
    }
    // Its stall, if it kept one in an earlier life, and its standing bids
    bot.my_stall()?;
    bot.my_orders()?;
    bot.answer_trades(true);
    let strands = bot.state().strands();
    town.health(|h| h.strands = strands);
    let mut citizen = SimBot::new(bot, role, seed, scale, Instant::now(), true, mind);
    citizen.people = true;
    citizen.persona = Some(persona.clone());
    if role == Role::Merchant {
        citizen.merchant = Some(merchant::Merchant::new(seed));
    }
    citizen.work(None, Some(town))
}

/// A maker's run: quote until `deadline`, take the orders down, and count
/// what filled.
fn make_market(mut bot: Bot, mut maker: Maker, deadline: Instant, started: Instant, scale: f64) -> BotRun {
    bot.record();
    let strands_start = bot.state().strands().unwrap_or(0);
    let result = bot
        .stay_up(Some(deadline), |bot| maker.run(bot, Some(deadline)))
        .and_then(|()| maker.withdraw(&mut bot));
    // Let the last balance change land
    let _ = bot.idle(Duration::from_millis(500));
    let at = started.elapsed().as_secs_f64() * scale / 3600.0;
    let entries = bot.take_journal().into_iter().filter_map(|e| market_entry(e, at)).collect();
    BotRun {
        name: bot.name().to_string(),
        role: MAKER_ROLE.into(),
        entries,
        strands_start,
        strands_end: bot.state().strands().unwrap_or(0),
        error: result.err().map(|e| e.to_string()),
    }
}

/// The report's entry for a market event in the journal: a fill, or a fee.
fn market_entry(change: StateEvent, at: f64) -> Option<Entry> {
    let StateEvent::Market(event) = change else {
        return None;
    };
    match *event {
        MarketEvent::OrderFilled { order, trade } => Some(Entry::Traded {
            item: trade.item_type,
            sold: order.side == OrderSide::Sell,
            quantity: trade.quantity,
            price: trade.price,
            at,
        }),
        MarketEvent::OrderPlaced(order) => Some(Entry::Fee { strands: order.fee }),
        _ => None,
    }
}

fn write(report: &Report, dir: &Path) -> std::io::Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let stamp = chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .format("%Y%m%d-%H%M%S");
    let md = dir.join(format!("sim-{stamp}.md"));
    let csv = dir.join(format!("sim-{stamp}.csv"));
    std::fs::write(&md, report.to_markdown())?;
    std::fs::write(&csv, report.to_csv())?;
    Ok((md, csv))
}

/// A node the bot has seen, and when.
#[derive(Debug, Clone)]
struct Seen {
    node: ResourceNodeInfo,
    at: Instant,
    /// The server refused us it, or the walk to it was blocked.
    shunned_until: Option<Instant>,
}

/// One bot playing its role.
struct SimBot {
    bot: Bot,
    role: Role,
    scale: f64,
    start: Instant,
    entries: Vec<Entry>,
    /// Every node seen this run. The store only holds what's in range.
    atlas: HashMap<Uuid, Seen>,
    /// Stations the bot owns, by type.
    stations: HashMap<String, StationInfo>,
    deed_cost: i64,
    rng: u64,
    /// Whether there are makers to trade with
    trading: bool,
    /// What decides at each decision point (C1.3). The sim's bots have the
    /// rules alone, middling in everything.
    mind: Mind,
    /// The last step's label: what "carry on" means on a work menu.
    doing: Option<String>,
    /// A citizen meets people (C1.5): it answers trades, bids for what its
    /// trade needs, and keeps a stall. A swarm bot doesn't.
    people: bool,
    /// A citizen's persona (C1.2), which gives it a day. A swarm bot works
    /// round the clock.
    persona: Option<Persona>,
    /// At its market this market's hours.
    at_market: bool,
    /// A merchant's goods and its market (C1.6).
    merchant: Option<merchant::Merchant>,
}

/// What a bot does next.
enum Action {
    Step(Step),
    Trade(Trade),
    /// A citizen's market hours with no trade to make: to its market, and
    /// a game minute there.
    Market,
}

impl SimBot {
    fn new(mut bot: Bot, role: Role, n: u64, scale: f64, start: Instant, trading: bool, mind: Mind) -> SimBot {
        bot.record();
        let me = bot.state().me();
        // Stations kept from an earlier run on the same accounts.
        let stations = bot
            .state()
            .stations()
            .values()
            .filter(|s| Some(s.owner_id) == me)
            .map(|s| (s.station_type.clone(), s.clone()))
            .collect();
        SimBot {
            bot,
            role,
            scale,
            start,
            entries: Vec::new(),
            atlas: HashMap::new(),
            stations,
            deed_cost: DEFAULT_DEED,
            rng: 0x9E37_79B9_7F4A_7C15 ^ (n + 1).wrapping_mul(0xBF58_476D_1CE4_E5B9),
            trading,
            mind,
            doing: None,
            people: false,
            persona: None,
            at_market: false,
            merchant: None,
        }
    }

    /// Game hours since the swarm started.
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * self.scale / 3600.0
    }

    /// A game duration as wall time.
    fn wall(&self, game_secs: f64) -> Duration {
        Duration::from_secs_f64(game_secs / self.scale)
    }

    fn play(mut self, hours: f64) -> BotRun {
        let strands_start = self.bot.state().strands().unwrap_or(0);
        let deadline = self.start + self.wall(hours * 3600.0);
        let error = self.work(Some(deadline), None).err().map(|e| e.to_string());
        // Let the last balance change land.
        let _ = self.bot.idle(Duration::from_millis(500));
        self.journal();
        BotRun {
            name: self.bot.name().to_string(),
            role: self.role.to_string(),
            strands_start,
            strands_end: self.bot.state().strands().unwrap_or(0),
            entries: self.entries,
            error,
        }
    }

    /// Plays the role until `deadline`, or for a citizen (C1.1) until its
    /// town says stop or its day ends (C1.2). `Err` is what ended it early:
    /// the server won't have the bot back.
    fn work(&mut self, deadline: Option<Instant>, town: Option<&Tether>) -> Result<Ended, BotError> {
        let stopped = || town.is_some_and(Tether::stopped);
        let mut failures = 0;
        while deadline.is_none_or(|d| Instant::now() < d) && !stopped() {
            // Someone asked to trade: that first, where it stands
            if self.people && self.bot.asked_to_trade() {
                match self.meet(town) {
                    Err(e) if e.is_final() => return Err(e),
                    Err(BotError::Offline(_)) => match self.recover(deadline, town) {
                        Err(e) if e.is_final() => return Err(e),
                        Err(_) => break,
                        Ok(()) => {}
                    },
                    _ => {}
                }
                continue;
            }
            let phase = self.phase();
            if let (Some(town), Some(phase)) = (town, phase) {
                town.health(|h| h.phase = Some(phase));
                town.server_time_was(self.bot.state().server_time_ms());
                match phase {
                    Phase::Home => return self.go_home(town),
                    // Past bedtime: it sleeps where it stands
                    Phase::Night => return Ok(Ended::Bedtime),
                    Phase::Market => {}
                    Phase::Work => self.at_market = false,
                }
            }
            let catalog = self.bot.state().catalog().clone();
            let holdings = self.holdings();
            // What to work on, then whether to trade first (C1.3)
            let errand = match self.role.errands(&catalog, &holdings) {
                Ok(errands) if errands.is_empty() => Ok(None),
                Ok(errands) => Ok(Some(self.choose_errand(&catalog, &holdings, errands))),
                Err(stuck) => Err(stuck),
            };
            let trade = match &errand {
                Ok(Some(errand)) if self.trading => self.choose_trade(&holdings, errand),
                _ => None,
            };
            // At market hours a citizen trades what it has, and otherwise
            // keeps its market rather than its work. The errand stays what
            // it carries on with after.
            let step = errand.map(|e| e.map(|e| e.step));
            let errand_label = step.as_ref().ok().and_then(|s| s.as_ref()).map(|s| s.label(&catalog));
            let action = match (step, trade) {
                (_, Some(trade)) => Ok(Some(Action::Trade(trade))),
                _ if phase == Some(Phase::Market) => Ok(Some(Action::Market)),
                (step, None) => step.map(|s| s.map(Action::Step)),
            };
            match action {
                Ok(Some(action)) => {
                    let label = self.label(&action, &catalog);
                    if let Some(town) = town {
                        town.health(|h| h.doing(Standing::Working, &label));
                    }
                    let began = self.now();
                    let result = match &action {
                        Action::Trade(Trade::MakeMarket) => self.make_market(town),
                        Action::Trade(trade) => self.trade(trade),
                        Action::Step(step) => self.take(step),
                        Action::Market => self.keep_market(town),
                    };
                    if errand_label.is_some() {
                        self.doing = errand_label;
                    }
                    self.journal();
                    let ok = result.is_ok();
                    self.entries.push(Entry::Spent {
                        label: label.clone(),
                        hours: self.now() - began,
                        ok,
                    });
                    if let (Some(town), Err(e)) = (town, &result) {
                        // Being asked to trade is no failure
                        if !matches!(e, BotError::Interrupted(_)) {
                            town.health(|h| h.failed(format!("{label}: {e}")));
                        }
                    }
                    match result {
                        Ok(()) => failures = 0,
                        Err(e) if e.is_final() => return Err(e),
                        // Someone asked to trade: answered at the top
                        Err(BotError::Interrupted(_)) => {}
                        // The step went with the connection (C0.6): wait for
                        // the server and plan again from what it says we hold.
                        Err(e @ BotError::Offline(_)) => {
                            self.entries.push(Entry::Failed {
                                label: label.clone(),
                                why: e.to_string(),
                            });
                            if let Some(town) = town {
                                town.health(|h| h.doing(Standing::Offline, "waiting for the server"));
                            }
                            match self.recover(deadline, town) {
                                Ok(()) => {}
                                Err(e) if e.is_final() => return Err(e),
                                // Out of time, or told to stop
                                Err(_) => break,
                            }
                        }
                        Err(e) => {
                            self.entries.push(Entry::Failed {
                                label: label.clone(),
                                why: e.to_string(),
                            });
                            // A citizen's last failure is on its health line
                            if town.is_none() {
                                eprintln!("{}: {label} failed: {e}", self.bot.name());
                            }
                            failures += 1;
                            if failures >= FAILURES_BEFORE_REST {
                                failures = 0;
                                self.rest(town, "resting");
                            }
                        }
                    }
                }
                Ok(None) => self.free_time(town),
                Err(stuck) => {
                    self.entries.push(Entry::Stuck { reason: stuck.0 });
                    self.free_time(town);
                }
            }
            if let Some(town) = town {
                // The report's entries are the sim's; a citizen lives on
                // and would only pile them up. C1.7's ledger is their place.
                self.journal();
                self.entries.clear();
                let strands = self.bot.state().strands();
                town.health(|h| h.strands = strands);
            }
        }
        Ok(Ended::Stopped)
    }

    /// Where in its day a citizen is, by the server's clock. A swarm bot has
    /// no day.
    fn phase(&self) -> Option<Phase> {
        let persona = self.persona.as_ref()?;
        Some(self.bot.state().time_of_day().map_or(Phase::Work, |t| persona.schedule.phase(t)))
    }

    /// What the health line and the report call `action`.
    fn label(&self, action: &Action, catalog: &crate::catalog::Catalog) -> String {
        match action {
            Action::Step(step) => step.label(catalog),
            Action::Trade(trade) => trade.label(),
            Action::Market => {
                let place = self.persona.as_ref().map_or(Market::Hall, |p| p.market);
                let at = if self.at_market { "at" } else { "to" };
                format!("{at} the {place}")
            }
        }
    }

    /// A citizen's market hours with nothing to trade: to the hall or the
    /// bazaar, then a game minute there among the others.
    fn keep_market(&mut self, town: Option<&Tether>) -> Result<(), BotError> {
        if !self.at_market {
            match self.persona.as_ref().map(|p| p.market) {
                Some(Market::Bazaar) => self.bot.go_to_bazaar()?,
                _ => {
                    self.bot.go_to_exchange()?;
                    // As in its free time at the hall (C1.5)
                    if self.people {
                        let _ = self.bot.collect_market_box().and_then(|_| self.post_demand());
                    }
                }
            }
            self.at_market = true;
            return Ok(());
        }
        // Someone asking to trade cuts it short: answered at the top
        let end = Instant::now() + self.wall(60.0);
        while !town.is_some_and(Tether::stopped) && !self.bot.asked_to_trade() {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            self.bot.idle(left.min(STOP_CHECK))?;
        }
        Ok(())
    }

    /// Dusk: buys a home if the citizen has saved enough and has none, walks
    /// home along the avenues, and goes to bed. Something going wrong on the
    /// way doesn't keep it up: it sleeps where it stands.
    fn go_home(&mut self, town: &Tether) -> Result<Ended, BotError> {
        town.health(|h| h.doing(Standing::Working, "going home"));
        self.at_market = false;
        if self.bot.state().plots().is_empty() {
            match self.buy_home() {
                Ok(()) => {}
                Err(e) if e.is_final() => return Err(e),
                Err(e) => town.health(|h| h.failed(format!("buying a home: {e}"))),
            }
        }
        let (x, y) = self.home();
        while !town.stopped() {
            match self.bot.walk_to(x, y) {
                Ok(()) => break,
                Err(e) if e.is_final() => return Err(e),
                // Someone asked to trade on the way (C1.5): answered, then
                // on home
                Err(BotError::Interrupted(_)) => match self.meet(Some(town)) {
                    Err(e) if e.is_final() => return Err(e),
                    _ => {}
                },
                // From a steep pocket in the wilds, or beside a rural lake,
                // the walk graph can find no way home (#280)
                Err(e) => {
                    town.health(|h| h.failed(format!("going home: {e}")));
                    break;
                }
            }
        }
        Ok(Ended::Bedtime)
    }

    /// Where a citizen lives: its lot, once it has one, and until then its
    /// spot on its avenue.
    fn home(&self) -> (f64, f64) {
        match (self.bot.state().plots().first(), &self.persona) {
            (Some(plot), _) => (plot.world_x, plot.world_y),
            (None, Some(persona)) => persona.home.spot(),
            (None, None) => self.bot.position(),
        }
    }

    /// Claims a lot to live on, near its spot in its home zone, if it holds
    /// what its thrift wants in hand for one.
    fn buy_home(&mut self) -> Result<(), BotError> {
        let Some(persona) = self.persona.clone() else {
            return Ok(());
        };
        let strands = self.bot.state().strands().unwrap_or(0);
        let fund = |cost: i64| persona.temperament.home_fund(cost);
        if strands < fund(self.deed_cost) {
            return Ok(());
        }
        self.claim_near(&[persona.home.zone.as_str()], persona.home.spot(), |cost| fund(cost) <= strands)
    }

    /// Waits for the server after a drop: until `deadline`, or for a citizen
    /// until it's back or its town says stop.
    fn recover(&mut self, deadline: Option<Instant>, town: Option<&Tether>) -> Result<(), BotError> {
        let Some(town) = town else {
            return self.bot.recover(deadline);
        };
        loop {
            match self.bot.recover(Some(Instant::now() + STOP_CHECK)) {
                Err(BotError::Timeout { .. }) if !town.stopped() => continue,
                result => return result,
            }
        }
    }

    /// Asks the mind what to work on, from the role's errands. While a
    /// layer thinks, it carries on with the last step's errand.
    fn choose_errand(&mut self, catalog: &Catalog, holdings: &Holdings, mut errands: Vec<Errand>) -> Errand {
        let menu: Vec<Choice> = errands.iter().map(|e| e.choice(catalog)).collect();
        let carry_on = self
            .doing
            .as_ref()
            .and_then(|doing| errands.iter().position(|e| &e.step.label(catalog) == doing))
            .unwrap_or(0);
        let pick = self
            .decide(Question::Work, menu, carry_on, holdings)
            .map_or(0, |(d, _)| d.pick);
        errands.swap_remove(pick)
    }

    /// Asks the mind whether to sell the role's product before `errand`,
    /// and if not, whether to buy what it would gather. `None`: just do it.
    fn choose_trade(&mut self, holdings: &Holdings, errand: &Errand) -> Option<Trade> {
        if self.merchant.is_some() {
            return self.choose_market(holdings, errand);
        }
        if let Some(product) = self.role.product() {
            let (bid, ask) = self.best(product);
            if let Some(menu) = self.role.sell_menu(holdings, errand.goal.as_ref(), bid, ask) {
                let question = Question::Sell { item: product.into() };
                if let Some((d, Choice::Sell { item, .. })) = self.decide(question, menu, 1, holdings) {
                    return Some(Trade::Sell {
                        item,
                        quantity: d.quantity?,
                        price: d.price?,
                    });
                }
            }
            if let Some(stock) = self.choose_stall(holdings, product) {
                return Some(stock);
            }
        }
        let Step::Gather { item, .. } = &errand.step else {
            return None;
        };
        let (_, ask) = self.best(item);
        let Some(menu) = self.role.buy_menu(holdings, &errand.step, ask) else {
            // Nobody selling: leave a bid for it, if the trade needs it
            return self.choose_bid(holdings, item);
        };
        let question = Question::Buy { item: item.clone() };
        match self.decide(question, menu, 1, holdings)? {
            (d, Choice::Buy { item, .. }) => Some(Trade::Buy {
                item,
                quantity: d.quantity?,
                price: d.price?,
            }),
            _ => None,
        }
    }

    /// Time on its hands: rest where it is, or pass it at the hall.
    fn free_time(&mut self, town: Option<&Tether>) {
        let holdings = self.holdings();
        let menu = vec![Choice::Rest, Choice::Hall];
        if let Some((_, Choice::Hall)) = self.decide(Question::FreeTime, menu, 0, &holdings) {
            if let Some(town) = town {
                town.health(|h| h.doing(Standing::Working, "walk to the hall"));
            }
            // A walk that doesn't get there rests where it got to; a lost
            // connection shows at the next step.
            if self.bot.go_to_exchange().is_ok() {
                if self.people {
                    let _ = self.bot.collect_market_box().and_then(|_| self.post_demand());
                }
                return self.rest(town, "resting at the hall");
            }
        }
        self.rest(town, "resting");
    }

    /// The mind's answer to `question` from `menu`: the decision, and the
    /// entry it picked. `carry_on` is the entry that means carrying on.
    fn decide(&mut self, question: Question, menu: Vec<Choice>, carry_on: usize, holdings: &Holdings) -> Option<(Decision, Choice)> {
        let s = self.situation(question, menu, carry_on, holdings);
        let d = self.mind.ask(&s).decision?;
        Some((d, s.choice(&d).clone()))
    }

    /// What a layer knows at a decision point.
    fn situation(&self, question: Question, menu: Vec<Choice>, carry_on: usize, holdings: &Holdings) -> Situation {
        let state = self.bot.state();
        let (x, y) = self.bot.position();
        let market = if self.trading { meet::quotes(state) } else { Vec::new() };
        Situation {
            who: Who {
                name: self.bot.name().to_string(),
                role: self.role.slug().to_string(),
                temperament: self.mind.temperament,
            },
            at: Place {
                x,
                y,
                zone: Zone::from_position(x, y).as_str().to_string(),
            },
            when: When {
                time_of_day: state.time_of_day(),
                hours: self.now(),
            },
            holdings: holdings.into(),
            market,
            question,
            menu,
            carry_on: Some(carry_on),
        }
    }

    /// The best bid and ask on `item`'s book.
    fn best(&self, item: &str) -> (Option<i64>, Option<i64>) {
        let book = self.bot.state().market().book(item);
        (book.and_then(|b| b.best_bid()), book.and_then(|b| b.best_ask()))
    }

    /// Sits out a game minute, or less if the town says stop.
    fn rest(&mut self, town: Option<&Tether>, activity: &str) {
        let Some(town) = town else {
            let _ = self.bot.idle(self.wall(60.0));
            return;
        };
        town.health(|h| h.doing(Standing::Resting, activity));
        let end = Instant::now() + self.wall(60.0);
        while !town.stopped() && !self.bot.asked_to_trade() {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() || self.bot.idle(left.min(STOP_CHECK)).is_err() {
                return;
            }
        }
    }

    /// Turns what the bot saw into entries.
    fn journal(&mut self) {
        let at = self.now();
        for change in self.bot.take_journal() {
            match change {
                // A failed pull (D5, #206)
                StateEvent::ExtractionCompleted { item, .. } if item.quantity == 0 => {
                    self.entries.push(Entry::Missed { item: item.item_type, at })
                }
                StateEvent::ExtractionCompleted { item, .. } => self.entries.push(Entry::Made {
                    item: item.item_type,
                    quantity: item.quantity,
                    quality: item.quality,
                    gathered: true,
                    at,
                }),
                StateEvent::CraftingCompleted { item } => self.entries.push(Entry::Made {
                    item: item.item_type,
                    quantity: item.quantity,
                    quality: item.quality,
                    gathered: false,
                    at,
                }),
                StateEvent::ToolBroke { item_type, .. } => {
                    self.entries.push(Entry::ToolBroke { item: item_type, at })
                }
                change => self.entries.extend(market_entry(change, at)),
            }
        }
    }

    fn holdings(&self) -> Holdings {
        let state = self.bot.state();
        let catalog = state.catalog();
        let mut h = Holdings {
            strands: state.strands().unwrap_or(0),
            deed_cost: self.deed_cost,
            ..Holdings::default()
        };
        for container in [state.inventory(), state.crafting_output()].into_iter().flatten() {
            for item in container.items() {
                *h.items.entry(item.item_type.clone()).or_insert(0) += item.quantity;
            }
        }
        if let Some(inventory) = state.inventory() {
            for item in inventory.items() {
                let tool = catalog.item_type(&item.item_type).and_then(|t| t.tool.as_ref());
                if let Some(tool) = tool {
                    if item.durability.is_none_or(|d| d > 0) {
                        h.tools.insert(tool.tool_type.clone());
                    }
                }
            }
        }
        h.stations.extend(self.stations.keys().cloned());
        h.plot_room = state
            .plots()
            .first()
            .map(|p| p.station_capacity - p.station_count);
        h
    }

    fn take(&mut self, step: &Step) -> Result<(), BotError> {
        match step {
            Step::Gather { resource, .. } => self.gather(resource),
            Step::Craft { recipe, station } => {
                let at = match station {
                    Some(kind) => {
                        let s = self.stations.get(kind).cloned().ok_or_else(|| {
                            BotError::Failed(format!("no {kind} of ours is known"))
                        })?;
                        self.bot.walk_to(s.x, s.y + 10.0)?;
                        Some(s.id)
                    }
                    None => None,
                };
                // Anything left in the output would block a different craft.
                self.bot.collect_output()?;
                self.bot.craft_at(recipe, at)?;
                self.bot.collect_output()
            }
            Step::Place {
                kit,
                station,
                on_plot,
            } => self.place(kit, station, *on_plot),
            Step::Claim => self.claim(),
            Step::Deposit => {
                let deposit = self.bot.deposit_fiber()?;
                self.entries.push(Entry::Deposited {
                    fiber: deposit.deposited_quantity,
                    strands: deposit.strands_received,
                });
                Ok(())
            }
        }
    }

    /// Walks to the Exchange hall, puts the trade's order on the book, waits
    /// [`FILL_WAIT`] for it to fill, takes down what's left, and collects
    /// what it bought. The walk goes round the canals by their bridges
    /// (#211), and the next step's walk takes the bot back out.
    fn trade(&mut self, trade: &Trade) -> Result<(), BotError> {
        match trade {
            Trade::MakeMarket => return self.make_market(None),
            Trade::Stock { item, quantity, price } => return self.keep_stall(item, *quantity, *price),
            Trade::Bid { item, quantity, price } => {
                self.bot.go_to_exchange()?;
                self.bid(item, *quantity, *price)?;
                self.bot.collect_market_box()?;
            }
            Trade::Sell { .. } | Trade::Buy { .. } => {
                self.bot.go_to_exchange()?;
                self.trade_at_the_hall(trade)?;
            }
        }
        // At the hall anyway: what else its trade wants
        if self.people {
            self.post_demand()?;
        }
        Ok(())
    }

    fn trade_at_the_hall(&mut self, trade: &Trade) -> Result<(), BotError> {
        let order = match trade {
            Trade::Sell { item, quantity, price } => {
                let (item_id, held) = self
                    .bot
                    .state()
                    .inventory()
                    .and_then(|inv| inv.items().filter(|i| &i.item_type == item).max_by_key(|i| i.quantity))
                    .map(|i| (i.item_id, i.quantity))
                    .ok_or_else(|| BotError::Failed(format!("no {item} to sell")))?;
                PlaceOrder {
                    side: OrderSide::Sell,
                    item_type: item.clone(),
                    price: *price,
                    quantity: (*quantity).min(held),
                    item_id: Some(item_id),
                    qualities: None,
                    min_grade: None,
                    grade_rule: None,
                }
            }
            Trade::Bid { .. } | Trade::Stock { .. } | Trade::MakeMarket => {
                return Err(BotError::Failed(format!("{} isn't a trade at the hall", trade.label())))
            }
            Trade::Buy { item, quantity, price } => PlaceOrder {
                side: OrderSide::Buy,
                item_type: item.clone(),
                price: *price,
                quantity: *quantity,
                item_id: None,
                qualities: None,
                min_grade: None,
                grade_rule: None,
            },
        };
        let placed = self.bot.place_order(order)?;
        let id = placed.id;
        let closed = |s: &crate::state::GameState| s.market().order(id).is_some_and(|o| o.status != OrderStatus::Open);
        if self.bot.until("a fill", FILL_WAIT, |s, _| closed(s)).is_err() {
            match self.bot.cancel_order(id) {
                Ok(_) => {}
                // It filled as we asked
                Err(BotError::Rest { source, .. }) if source.status().is_some_and(|s| s < 500) => {}
                Err(e) => return Err(e),
            }
        }
        // Bought units, or an unsold remainder, are in the market box
        self.bot.collect_market_box()?;
        Ok(())
    }

    /// One pull from a node of `resource`, finding one first.
    fn gather(&mut self, resource: &str) -> Result<(), BotError> {
        for _ in 0..MAX_EXPLORES {
            self.learn_nodes();
            let pick = self.random();
            let Some(node) = self.nearest(resource, pick) else {
                self.explore(resource)?;
                continue;
            };
            match self.bot.extract(&node) {
                Ok(_) => return Ok(()),
                // A blocked walk is water in the way: bots walk straight lines
                // (#211), so the node is shunned and another tried.
                Err(e @ (BotError::Refused { .. } | BotError::Blocked { .. })) => {
                    let shun = self.wall(120.0);
                    if let Some(seen) = self.atlas.get_mut(&node.id) {
                        seen.shunned_until = Some(Instant::now() + shun);
                    }
                    if matches!(e, BotError::Blocked { .. }) {
                        continue;
                    }
                    return Err(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(BotError::Failed(format!("found no {resource} to gather")))
    }

    fn learn_nodes(&mut self) {
        let now = Instant::now();
        for node in self.bot.state().nodes().values() {
            let shunned_until = self.atlas.get(&node.id).and_then(|s| s.shunned_until);
            self.atlas.insert(
                node.id,
                Seen {
                    node: node.clone(),
                    at: now,
                    shunned_until,
                },
            );
        }
    }

    /// A near node of `resource` that nobody is gathering and that's available,
    /// or was depleted long enough ago to have grown back. One of the three
    /// nearest, picked by `pick` (in [0, 1)), so a crowd spreads out rather
    /// than queueing at one patch.
    fn nearest(&self, resource: &str, pick: f64) -> Option<ResourceNodeInfo> {
        let regrow = self
            .bot
            .state()
            .catalog()
            .resource_type(resource)
            .map_or(600.0, |r| r.regeneration_ticks as f64 / 10.0);
        let regrown = self.wall(regrow);
        let (x, y) = self.bot.position();
        let now = Instant::now();
        let state = self.bot.state();
        let mut near: Vec<&ResourceNodeInfo> = self
            .atlas
            .values()
            .filter(|s| s.node.resource_type == resource)
            .filter(|s| s.shunned_until.is_none_or(|t| now >= t))
            .filter(|s| s.node.state == "available" || now.duration_since(s.at) >= regrown)
            .filter(|s| state.harvester(s.node.id).is_none())
            .map(|s| &s.node)
            .collect();
        near.sort_by(|a, b| (a.x - x).hypot(a.y - y).total_cmp(&(b.x - x).hypot(b.y - y)));
        near.truncate(3);
        let i = (pick * near.len() as f64) as usize;
        near.get(i.min(near.len().saturating_sub(1))).map(|n| (*n).clone())
    }

    /// Walks somewhere new in the nearest zone `resource` grows in, then asks
    /// what's around.
    fn explore(&mut self, resource: &str) -> Result<(), BotError> {
        let (x, y) = self.bot.position();
        let (cx, cy) = (Zone::CENTER_X, Zone::CENTER_Y);
        let r_now = (x - cx).hypot(y - cy);
        let a_now = (y - cy).atan2(x - cx);
        let bands: Vec<(f64, f64)> = self
            .bot
            .state()
            .catalog()
            .resource_type(resource)
            .map(|r| {
                r.spawns
                    .iter()
                    .filter(|s| s.density > 0.0 && s.zone != Zone::Wilderness.as_str())
                    .filter_map(|s| Zone::from_str(&s.zone))
                    .map(|z| z.radii())
                    .collect()
            })
            .unwrap_or_default();
        let (inner, outer) = bands
            .into_iter()
            .min_by(|a, b| band_gap(*a, r_now).total_cmp(&band_gap(*b, r_now)))
            .unwrap_or((rules_spawn_radius() - 300.0, rules_spawn_radius() + 300.0));
        let r = inner + 60.0 + (outer - inner - 120.0).max(0.0) * self.random();
        // Along the ring from here, a little either way.
        let a = a_now + (self.random() - 0.5) * 0.6;
        match self.bot.walk_to(cx + r * a.cos(), cy + r * a.sin()) {
            Ok(()) | Err(BotError::Blocked { .. }) => {}
            Err(e) => return Err(e),
        }
        self.bot.refetch(Fetch::NearbyNodes);
        self.bot.settle()
    }

    fn place(&mut self, kit: &str, station: &str, on_plot: bool) -> Result<(), BotError> {
        let kit_id = self
            .bot
            .state()
            .inventory()
            .and_then(|inv| inv.items().find(|i| i.item_type == kit).map(|i| i.item_id))
            .ok_or_else(|| BotError::Failed(format!("no {kit} in the inventory")))?;
        let spots: Vec<(f64, f64)> = if on_plot {
            let plot = self
                .bot
                .state()
                .plots()
                .first()
                .cloned()
                .ok_or_else(|| BotError::Failed("no plot".into()))?;
            let b = plot.bounds;
            let inside = |(x, y): &(f64, f64)| {
                *x > b.min_x + 15.0 && *x < b.max_x - 15.0 && *y > b.min_y + 15.0 && *y < b.max_y - 15.0
            };
            ring(plot.world_x, plot.world_y, SPOT_SPACING)
                .into_iter()
                .filter(inside)
                .filter(|s| !self.near_a_station(*s))
                .collect()
        } else {
            let (x, y) = self.bot.position();
            let mut spots = ring(x, y, 30.0);
            spots.extend(ring(x, y, 90.0));
            spots
        };
        let mut last = None;
        for (x, y) in spots {
            match self.bot.walk_to(x, y) {
                Ok(()) => {}
                Err(e @ BotError::Blocked { .. }) => {
                    last = Some(e);
                    continue;
                }
                Err(e) => return Err(e),
            }
            match self.bot.place_station(station, kit_id, x, y) {
                Ok(placed) => {
                    self.stations.insert(station.to_string(), placed);
                    return Ok(());
                }
                Err(e @ BotError::Rest { .. }) => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| BotError::Failed(format!("nowhere to put a {station}"))))
    }

    fn near_a_station(&self, (x, y): (f64, f64)) -> bool {
        self.bot
            .state()
            .stations()
            .values()
            .any(|s| (s.x - x).hypot(s.y - y) < capitol_protocol::rules::STATION_MIN_SPACING)
    }

    /// Buys the nearest affordable unclaimed plot: for a citizen, near its
    /// home (C1.2), so the lot it builds on is the one it lives on.
    fn claim(&mut self) -> Result<(), BotError> {
        let strands = self.bot.state().strands().unwrap_or(0);
        match self.persona.as_ref().map(|p| p.home) {
            Some(home) => {
                let mut zones = vec![home.zone.as_str()];
                zones.extend(PLOT_ZONES.iter().filter(|z| **z != home.zone.as_str()));
                self.claim_near(&zones, home.spot(), |cost| cost <= strands)
            }
            None => {
                let here = self.bot.position();
                self.claim_near(&PLOT_ZONES, here, |cost| cost <= strands)
            }
        }
    }

    /// Buys the unclaimed plot nearest `(x, y)` whose deed it can `afford`,
    /// from the first of `zones` that has one.
    fn claim_near(&mut self, zones: &[&str], (x, y): (f64, f64), afford: impl Fn(i64) -> bool) -> Result<(), BotError> {
        let strands = self.bot.state().strands().unwrap_or(0);
        for &zone in zones {
            let mut plots = self.bot.unclaimed_plots(zone)?;
            plots.retain(|p| p.deed_cost.is_some());
            if let Some(cost) = plots.iter().filter_map(|p| p.deed_cost).min() {
                self.deed_cost = cost as i64;
            }
            plots.retain(|p| p.deed_cost.is_some_and(|c| afford(c as i64)));
            plots.sort_by(|a, b| {
                (a.world_x - x)
                    .hypot(a.world_y - y)
                    .total_cmp(&(b.world_x - x).hypot(b.world_y - y))
            });
            for plot in plots.iter().take(5) {
                match self.bot.claim_plot(plot.id) {
                    Ok(_) => {
                        self.entries.push(Entry::Claimed {
                            cost: plot.deed_cost.unwrap_or(0) as i64,
                        });
                        self.bot.refetch(Fetch::Plots);
                        self.bot.settle()?;
                        return Ok(());
                    }
                    // Someone else got it first.
                    Err(BotError::Rest { .. }) => continue,
                    Err(e) => return Err(e),
                }
            }
        }
        Err(BotError::Failed(format!(
            "no plot for sale at {strands} Strands or less"
        )))
    }

    /// Uniform in [0, 1). xorshift64*.
    fn random(&mut self) -> f64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// How far `r` lies outside the band `(inner, outer)`.
fn band_gap((inner, outer): (f64, f64), r: f64) -> f64 {
    (inner - r).max(r - outer).max(0.0)
}

fn rules_spawn_radius() -> f64 {
    use capitol_protocol::rules::{SPAWN_X, SPAWN_Y};
    (SPAWN_X - Zone::CENTER_X).hypot(SPAWN_Y - Zone::CENTER_Y)
}

/// `(x, y)` and six points round it at radius `r`.
fn ring(x: f64, y: f64, r: f64) -> Vec<(f64, f64)> {
    let mut spots = vec![(x, y)];
    for k in 0..6 {
        let a = std::f64::consts::TAU * k as f64 / 6.0;
        spots.push((x + r * a.cos(), y + r * a.sin()));
    }
    spots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_band_gap_is_zero_inside_and_the_distance_outside() {
        assert_eq!(band_gap((3000.0, 3800.0), 3350.0), 0.0);
        assert_eq!(band_gap((1800.0, 3000.0), 3350.0), 350.0);
        assert_eq!(band_gap((3000.0, 3800.0), 2500.0), 500.0);
    }

    #[test]
    fn a_ring_is_seven_spots_apart_enough_for_stations() {
        let spots = ring(100.0, 100.0, SPOT_SPACING);
        assert_eq!(spots.len(), 7);
        for (i, a) in spots.iter().enumerate() {
            for b in &spots[i + 1..] {
                assert!((a.0 - b.0).hypot(a.1 - b.1) >= capitol_protocol::rules::STATION_MIN_SPACING);
            }
        }
    }
}
