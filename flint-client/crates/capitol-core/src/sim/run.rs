//! The swarm: one thread a bot, each signing in, waiting for the others, then
//! playing its [`Role`] until the game hours are up. The market makers run
//! alongside, one thread each, from a grant.
//!
//! `capitol-town`'s citizens (C1.1) play the same loop, one at a time
//! through [`live`], for as long as their town runs. They also meet people
//! (C1.5, [`people`]): they answer trades, leave standing bids for what
//! their trade needs, and keep a stall when they make more than the hall
//! takes. The swarm's bots don't, so the E2.7 sim's numbers stay comparable.
//! Nor do they have an idle life (C2.1, [`idle`]): a citizen spends its
//! free time and a noon break somewhere worth being, keeps a spot on the
//! hall's steps or the bazaar's lane at market hours, drifts toward company,
//! turns to whoever it trades with, and stops a moment beside a person it
//! passes. Lingering among other citizens, it may ask one for a trade
//! (#295). A town's merchants (C1.6, [`merchant`]) earn a float and make a
//! market. And every citizen walks the road in, the tutorial, and claims
//! its deed at the end of it (#450, [`road_in`]). They answer and post
//! commissions on the hall's board too (Commons S2.5, [`board`]), take
//! work for a wage in their working hours (S2.8, [`wages`]), and join the
//! guilds of their trades and pay into them (S4.5, [`guilds`]). In their
//! working hours they wander off now and then, for a quarter hour or so
//! (Callings K1.5, [`detour`]). One whose work climbs toward a tool buys
//! one when it can spare the Strands (K2.1, [`tools`]).

mod board;
mod detour;
mod guilds;
mod hire;
mod idle;
mod merchant;
mod people;
pub mod road_in;
mod stint;
mod tools;
mod wages;

pub use road_in::walk_the_road;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use capitol_protocol::land;
use capitol_protocol::metrics::RoadInReport;
use capitol_protocol::rest::{PlaceOrder, ResourceNodeInfo, StationInfo};
use capitol_protocol::rules::market::{OrderSide, OrderStatus};
use capitol_protocol::rules::Zone;
use uuid::Uuid;

use super::activity::{self, Activity};
use super::lot;
use super::plan::{Goal, Holdings, Step, Upward, FLOAT};
use super::report::{BotRun, Entry, Report, MAKER_ROLE};
use super::{is_tool, watched, Errand, Role, Trade, MARKET_ITEMS};
use crate::bot::{Account, Bot, BotError, NO_WAY};
use crate::catalog::Catalog;
use crate::citizen::calling::Calling;
use crate::citizen::decide::log::Fate;
use crate::citizen::decide::{Choice, Decision, Held, Mind, Place, Question, Situation, When, Who};
use crate::citizen::detour::Detour;
use crate::citizen::meet;
use crate::citizen::persona::{Market, Persona};
use crate::citizen::phrases::Voice;
use crate::citizen::schedule::Phase;
use crate::citizen::town::{Ended, Standing, Tether};
use crate::market::MarketEvent;
use crate::market_maker::{self, Maker, MakerConfig};
use crate::net::NetConfig;
use crate::poll::Fetch;
use crate::state::StateEvent;

/// A deed's price until a plot listing says otherwise: any home zone's,
/// since they cost the same (#292).
pub(super) const DEFAULT_DEED: i64 = 500;
/// Where builders look for land, nearest the rural spawn first.
const PLOT_ZONES: [&str; 2] = ["rural", "suburban"];
/// The most spots on its lot a bot walks to before it gives up on placing.
const PLACE_TRIES: usize = 8;
/// A citizen home for the night repairs its stations on its lot under this,
/// when it holds what the repair takes (Plots v2 §5.8, #478). Under 25 its
/// work at one makes the goods first ([`Holdings::worn`]).
const MEND_BELOW: i32 = 50;
/// How long a station whose repair the server refused is left alone, in
/// game hours (#478).
const REPAIR_RETRY_HOURS: f64 = 1.0;
/// Walks out to look for a resource before the gather gives up.
const MAX_EXPLORES: usize = 6;
/// Spots an explore picks before it gives up on ones it has no way to.
const EXPLORE_TRIES: usize = 4;
/// Explores nearer in that find nothing before a gather tries the wilds
/// (#336).
const WILD_AFTER: usize = 2;
/// The [`risk`](crate::citizen::decide::Temperament::risk) from which a
/// citizen explores the wilds first: about a third of them, since a
/// temperament is two draws averaged.
const VENTURE_RISK: f32 = 0.6;
/// Spots an explore for ore looks over, to walk to the highest (#336).
const HIGH_GROUND_LOOKS: usize = 6;
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
/// How far a citizen goes back for a station of its own that it's leaving
/// off its lot, to take it down (#488). Further off, it's left for a day
/// that ends nearer.
const PACK_UP_REACH: f64 = 200.0;
/// How many of its stations off its lot and further than [`PACK_UP_REACH`]
/// a citizen with a lot goes back for of an evening (#500): one, so a pile
/// by the spawn clears over a few days and no evening is all walking.
const FETCH_A_DAY: usize = 1;

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
                            role: role.slug().to_string(),
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

/// A citizen's session (C1.1, C1.2, #276): `bot` plays `role` as `persona`
/// would, with `mind` making its decisions (C1.3), until its session's over
/// or its town says stop. It trades with whatever is on the book, at the hall, as the swarm
/// does with its makers; the town grants nobody anything (D2), so the book
/// holds only what players and citizens put there.
///
/// The day follows the persona's schedule on the server's clock: work, the
/// market's hours at the hall or the bazaar, work again, home at dusk,
/// buying a lot to live on first if it has saved enough, and then the
/// night's work from home. `Ok` once stopped, or away; `Err` when the bot
/// couldn't start, or the server won't have it back.
#[allow(clippy::too_many_arguments)]
pub(crate) fn live(
    mut bot: Bot,
    role: Role,
    persona: &Persona,
    seed: u64,
    mut mind: Mind,
    voice: Voice,
    hire_line: i64,
    trade_line: i64,
    patron_line: i64,
    town: &Tether,
) -> Result<Ended, BotError> {
    let scale = bot.health()?.sim_tick_scale.unwrap_or(1.0);
    mind.set_scale(scale);
    // The hall's goods and the tools (K2.1)
    for item in watched() {
        bot.watch_market(item)?;
    }
    // Its stall, if it kept one in an earlier life, and its standing bids
    bot.my_stall()?;
    bot.my_orders()?;
    bot.answer_trades(true);
    bot.greet_people(true);
    bot.speak_with(voice);
    let strands = bot.state().strands();
    town.health(|h| h.strands = strands);
    let mut citizen = SimBot::new(bot, role, seed, scale, Instant::now(), true, mind);
    citizen.people = true;
    citizen.persona = Some(persona.clone());
    // The town's line for hiring (K2.4), which its thrift weighs
    citizen.hiring.base_line = hire_line;
    // And for posting for its home (K2.3)
    citizen.board.patron_line = patron_line;
    // Its calling, and the stint it was on and what it did lately, from
    // the side file, so a short session still adds up to a mix (K1.2)
    citizen.calling = persona.calling.clone();
    citizen.stints = stint::Stints::from_kept(&town.kept(), citizen.now());
    // Every citizen walks the road in, and earns its deed (#450)
    citizen.road = Some(road_in::Road::default());
    // Its goods to trade in, from its calling (Callings K2.2): trading is a
    // leaning anyone may take up once it has its float
    citizen.merchant = Some(merchant::Merchant::new(&citizen.calling, seed, trade_line));
    // Its stations wherever they stand, not just those near it (#500)
    if let Err(e) = citizen.learn_own() {
        if e.is_final() {
            return Err(e);
        }
        town.health(|h| h.failed(format!("listing its stations: {e}")));
    }
    let ended = citizen.work(None, Some(town));
    // A shift it's on ends with its session, paid for what it made, rather
    // than holding the work from others until the server sees it gone
    if ended.is_ok() {
        if let Err(e) = citizen.end_shift("signed_off") {
            town.health(|h| h.failed(format!("stopping its shift: {e}")));
        }
        // Its offers of work too (K2.4), and what they left in its boxes,
        // unless the town is stopping
        let mut hires = citizen.end_hires("signed_off");
        if hires.is_ok() && !town.stopped() {
            hires = citizen.collect_boxes();
        }
        if let Err(e) = hires {
            town.health(|h| h.failed(format!("taking its offers of work down: {e}")));
        }
        // A guild day's deposit it hasn't made, before it goes (S4.5)
        if let Err(e) = citizen.pay_in_on_leaving() {
            town.health(|h| h.failed(format!("paying into its guild: {e}")));
        }
    }
    // What the day's last steps did: its walk home, and its deed
    citizen.journal();
    town.record(std::mem::take(&mut citizen.entries), citizen.mind.take_rulings());
    citizen.credit();
    town.keep(citizen.stints.kept(citizen.now()));
    ended
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

/// What came of a decision that picked `choice`, when `action` was what the
/// round did (`None`: free time) and `result` how it went (C3.1).
fn fate(choice: &Choice, action: Option<&Action>, result: &Result<(), BotError>) -> Fate {
    let acted = match (choice, action) {
        (Choice::Work { .. } | Choice::Bank, Some(Action::Step(_)))
        | (Choice::Sell { .. }, Some(Action::Trade(Trade::Sell { .. })))
        | (Choice::Buy { .. }, Some(Action::Trade(Trade::Buy { .. } | Trade::Shop { .. })))
        | (Choice::Bid { .. }, Some(Action::Trade(Trade::Bid { .. })))
        | (Choice::Stock { .. }, Some(Action::Trade(Trade::Stock { .. })))
        | (Choice::MakeMarket { .. }, Some(Action::Trade(Trade::MakeMarket)))
        | (Choice::Road { .. }, Some(Action::Road(_)))
        | (Choice::Claim { .. } | Choice::Deliver { .. }, Some(Action::Board(_)))
        | (Choice::TakeWork, Some(Action::Wage(_)))
        | (Choice::Hire { .. }, Some(Action::Hire(_)))
        | (Choice::Apply { .. }, Some(Action::Guild(_)))
        | (Choice::Post { .. }, Some(Action::Trade(Trade::Post { .. })))
        | (Choice::Rest | Choice::Hall, None) => true,
        // A stint is taken up as it's chosen (Callings K1.2)
        (Choice::Take { .. }, _) => true,
        // A detour, from the stint menu or in free time (K1.5)
        (Choice::Detour { .. }, Some(Action::Detour(_)) | None) => true,
        (Choice::Keep | Choice::Gather { .. } | Choice::Make { .. } | Choice::Pass | Choice::Stay, _) => return Fate::Passed,
        _ => false,
    };
    match result {
        _ if !acted => Fate::Superseded,
        Ok(()) => Fate::Done,
        Err(e) => Fate::Failed { why: e.to_string() },
    }
}

/// A node the bot has seen, and when.
#[derive(Debug, Clone)]
struct Seen {
    node: ResourceNodeInfo,
    at: Instant,
    /// The server refused us it, or the walk to it was blocked.
    shunned_until: Option<Instant>,
}

/// One bot playing its role, or a citizen its calling (Callings K1.2).
struct SimBot {
    bot: Bot,
    /// Its birth trade: the swarm's role, a citizen's roster `role`. Its
    /// name and home zone go by it; its lot by its calling (K1.3,
    /// [`Self::lot_for`]); its work by the job of the moment
    /// ([`Self::job`]).
    role: Role,
    /// What it would rather be doing (K0.2): a citizen's persona's, or its
    /// role alone for a swarm bot ([`Calling::single`]).
    calling: Calling,
    /// Its stints: the job of the moment, what it has done lately, and
    /// what pulls on it. A swarm bot never takes one.
    stints: stint::Stints,
    scale: f64,
    start: Instant,
    entries: Vec<Entry>,
    /// Every node seen this run. The store only holds what's in range.
    atlas: HashMap<Uuid, Seen>,
    /// Stations the bot owns, by id (#500): a second of a type, and one far
    /// off that `/stations/nearby` has lost, count too. Its working one of a
    /// type is [`Self::own`].
    stations: HashMap<Uuid, StationInfo>,
    /// The day it last went back for a far station of its own (#500).
    fetched_day: Option<i64>,
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
    /// Home from the day's work, on the night's (#276): set by the walk
    /// home at dusk, until the morning's work.
    night_shift: bool,
    /// A citizen's goods to trade in and its market (C1.6, Callings K2.2).
    /// Every citizen has them; a swarm bot doesn't trade.
    merchant: Option<merchant::Merchant>,
    /// Where it's lingering, and which way it looks (C2.1).
    spot: Option<((f64, f64), (f64, f64))>,
    /// The day it last took its noon break (C2.1).
    noon_day: Option<i64>,
    /// The citizens it has asked to trade lately, and when (#295).
    asked: HashMap<Uuid, Instant>,
    /// The cards it has read of those it thought of asking, and the game
    /// day it read each (Callings K3.2): `lately`, or `None` for a quiet one.
    cards: HashMap<Uuid, (i64, Option<capitol_protocol::rules::lately::Lately>)>,
    /// A citizen walks the road in (#450): the steps it's waiting on. A
    /// swarm bot doesn't, so the E2.7 sim's numbers stay comparable.
    road: Option<road_in::Road>,
    /// Its last round was a step on the road: what carrying on means there.
    on_road: bool,
    /// The hall's board as it last read it, and its part in it (S2.5). Only
    /// a citizen reads it.
    board: board::Board,
    /// The work offered as it last read it, and its shift (S2.8). Only a
    /// citizen works for a wage.
    wages: wages::Wages,
    /// Its guild as it last read it, and what it has done about guilds
    /// (S4.5). Only a citizen joins one.
    guilds: guilds::Guilds,
    /// Its offers of work, and the boxes they left (Callings K2.4). Only a
    /// citizen hires.
    hiring: hire::Hiring,
    /// The lot it last took its stations down on to pack again (#405):
    /// once a lot, so a packing that comes out no better doesn't go round.
    repacked: Option<Uuid>,
    /// When it last set off on a detour, in the run's game hours (K1.5).
    last_detour: Option<f64>,
    /// When it last read the bazaar's signs for a tool, in the run's game
    /// hours (K2.1).
    tools_looked: Option<f64>,
    /// When it last found every stall at the bazaar taken, with none of its
    /// own, in the run's game hours (#551): it isn't asked to stock until
    /// it looks again.
    stalls_full: Option<f64>,
    /// Tools its standing bids bought since it last collected its market
    /// box (K2.1): they wait there, not in its pack.
    boxed_tools: std::collections::HashSet<String>,
    /// Its stations whose repair the server refused, by type, and when, in
    /// the run's game hours (#478): left for [`REPAIR_RETRY_HOURS`], so a
    /// refusal isn't asked again every round.
    refused_repairs: HashMap<String, f64>,
}

/// What a bot does next.
enum Action {
    Step(Step),
    Trade(Trade),
    /// A citizen's market hours with no trade to make: to its market, and
    /// a game minute there.
    Market,
    /// A bit of the road in's next step (#450).
    Road(capitol_protocol::tutorial::Step),
    /// A claim, or a delivery, at the hall's board (S2.5).
    Board(board::BoardAct),
    /// Work taken for a wage, or a unit of its shift (S2.8).
    Wage(wages::WageAct),
    /// Work offered at a place of its own, or a box a hire left brought
    /// home (Callings K2.4).
    Hire(hire::HireAct),
    /// Joining a guild, paying in or leaving (S4.5), between its other
    /// business: never a round of its own.
    Guild(guilds::GuildAct),
    /// A quarter hour off its work for something unrelated (K1.5), taken
    /// as a stint ends.
    Detour(Detour),
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
            .map(|s| (s.id, s.clone()))
            .collect();
        SimBot {
            bot,
            role,
            calling: Calling::single(role),
            stints: stint::Stints::default(),
            scale,
            start,
            entries: Vec::new(),
            atlas: HashMap::new(),
            stations,
            fetched_day: None,
            deed_cost: DEFAULT_DEED,
            rng: 0x9E37_79B9_7F4A_7C15 ^ (n + 1).wrapping_mul(0xBF58_476D_1CE4_E5B9),
            trading,
            mind,
            doing: None,
            people: false,
            persona: None,
            at_market: false,
            night_shift: false,
            merchant: None,
            spot: None,
            noon_day: None,
            asked: HashMap::new(),
            cards: HashMap::new(),
            road: None,
            on_road: false,
            board: board::Board::new(),
            wages: wages::Wages::new(),
            guilds: guilds::Guilds::new(),
            hiring: hire::Hiring::new(),
            repacked: None,
            last_detour: None,
            tools_looked: None,
            stalls_full: None,
            boxed_tools: Default::default(),
            refused_repairs: HashMap::new(),
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
            role: self.role.slug().to_string(),
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
                    // Dusk, the change of shift: home, then the night's
                    // work. A session that starts at night goes home first
                    // too
                    Phase::Home | Phase::Night if !self.night_shift => {
                        self.go_home(town)?;
                        continue;
                    }
                    Phase::Home | Phase::Night => {}
                    Phase::Market => self.night_shift = false,
                    Phase::Work => {
                        self.night_shift = false;
                        if self.at_market {
                            self.at_market = false;
                            self.spot = None;
                        }
                        if self.noon_break(Some(town)) {
                            continue;
                        }
                    }
                }
            }
            let catalog = self.bot.state().catalog().clone();
            // What it owes on the board shapes its work (S2.5)
            self.read_board()?;
            self.read_wages()?;
            // Its guild (S4.5): joined, paid into or left from where it
            // stands, before the round's own business
            self.read_guild()?;
            self.mind_guild(phase, town)?;
            // Its offers of work (K2.4): any whose place its stint wants
            // back, or all once its money runs low, come down
            match self.mind_hires() {
                Ok(()) => {}
                Err(e) if e.is_final() => return Err(e),
                Err(e) => {
                    if let Some(town) = town {
                        town.health(|h| h.failed(format!("taking an offer of work down: {e}")));
                    }
                }
            }
            let holdings = self.holdings();
            // A lot with no room for the rest of its chain, that would have
            // it packed again, is (#405): planned again with its kits home
            match self.repack(&holdings) {
                Ok(0) => {}
                Ok(_) => continue,
                Err(e) if e.is_final() => return Err(e),
                Err(e) => {
                    if let Some(town) = town {
                        town.health(|h| h.failed(format!("repacking its lot: {e}")));
                    }
                }
            }
            // The round's decisions, whose outcomes are logged after it (C3.1)
            let mark = self.mind.mark();
            // What to work at for the next while (Callings K1.2): asked as
            // a session starts, as a stint runs its length, and on a pull.
            // As one starts or ends, it may wander off first (K1.5)
            let detour = self.mind_stint(&holdings, phase);
            // A promise first, then the board, before the road or a trade
            let board = if self.people && detour.is_none() { self.choose_board(&holdings) } else { None };
            // Then work for a wage (S2.8): its shift, or work it takes
            let wage = if board.is_none() && detour.is_none() { self.choose_wage(&holdings, phase) } else { None };
            // Then hiring (K2.4): a box a hire left, or work it offers
            let hire = if board.is_none() && wage.is_none() && detour.is_none() { self.choose_hire(&holdings, phase) } else { None };
            // What to work on, at the job of the moment, then whether to
            // trade first (C1.3). On a shift, the employer's work is its work
            let errand = match self.job().errands_at(&catalog, &holdings, self.commitment()) {
                _ if wage.is_some() || detour.is_some() => Ok(None),
                Ok(errands) if errands.is_empty() => Ok(None),
                Ok(errands) => Ok(Some(self.choose_errand(&catalog, &holdings, errands))),
                Err(stuck) => Err(stuck),
            };
            // The road in first, while a step on it is open (#450): by day and
            // at market hours, and in the night's work what needs no walk
            let road = match &errand {
                _ if self.road.is_none() || board.is_some() || wage.is_some() || detour.is_some() => None,
                Ok(errand) => self.choose_road(&holdings, errand.as_ref()),
                Err(_) => self.choose_road(&holdings, None),
            };
            let trade = match &errand {
                _ if road.is_some() || board.is_some() || hire.is_some() => None,
                // No market in the night's work (#276)
                Ok(Some(errand)) if self.trading && !self.night_shift => self.choose_trade(&holdings, errand),
                _ => None,
            };
            // At market hours a citizen trades what it has, and otherwise
            // keeps its market rather than its work. The errand stays what
            // it carries on with after.
            // What its errand is for, which its step's and its trade's time
            // are filed under (Callings K0.1)
            let goal = errand.as_ref().ok().and_then(Option::as_ref).map(|e| e.goal.clone());
            let job = match &goal {
                Some(goal) => Activity::of_goal(&catalog, goal.as_ref(), self.job()),
                None => Activity::of_role(self.job()),
            };
            let step = errand.map(|e| e.map(|e| e.step));
            let errand_label = step.as_ref().ok().and_then(|s| s.as_ref()).map(|s| s.label(&catalog));
            let action = match (board, wage, road, step, trade) {
                _ if detour.is_some() => Ok(detour.map(Action::Detour)),
                (Some(act), ..) => Ok(Some(Action::Board(act))),
                (None, Some(act), ..) => Ok(Some(Action::Wage(act))),
                _ if hire.is_some() => Ok(hire.map(Action::Hire)),
                (None, None, Some(lesson), _, _) => Ok(Some(Action::Road(lesson))),
                (None, None, None, _, Some(trade)) => Ok(Some(Action::Trade(trade))),
                _ if phase == Some(Phase::Market) => Ok(Some(Action::Market)),
                (None, None, None, step, None) => step.map(|s| s.map(Action::Step)),
            };
            match action {
                Ok(Some(action)) => {
                    let label = self.label(&action, &catalog);
                    let (activity, setup) = Self::activity(&action, job, goal.as_ref().and_then(Option::as_ref), &catalog);
                    let day = self.day();
                    if let Some(town) = town {
                        // The stint it's at now, which this round's question
                        // may have changed, so a market spell under a stint
                        // just taken up reads as trading
                        let stint = self.stints.on.map(|s| s.job);
                        town.health(|h| {
                            if stint.is_some() {
                                h.stint = stint;
                            }
                            h.doing(Standing::Working, &label);
                        });
                    }
                    let began = self.now();
                    let result = match &action {
                        Action::Trade(Trade::MakeMarket) => self.make_market(town),
                        Action::Trade(trade) => self.trade(trade),
                        Action::Step(step) => self.take(step),
                        Action::Market => self.keep_market(town),
                        Action::Road(lesson) => self.walk_road(*lesson),
                        Action::Board(act) => self.board_act(act),
                        Action::Wage(act) => self.wage_act(act),
                        Action::Hire(act) => self.hire_act(act),
                        Action::Guild(act) => self.guild_act(act),
                        Action::Detour(d) => self.take_detour(d, town),
                    };
                    if errand_label.is_some() {
                        self.doing = errand_label;
                    }
                    self.journal();
                    self.settle(mark, Some(&action), &result);
                    let ok = result.is_ok();
                    self.entries.push(Entry::Spent {
                        label: label.clone(),
                        hours: self.now() - began,
                        ok,
                        activity,
                        setup,
                        day,
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
                // Nothing to do at the job of the moment: another, next round
                Ok(None) => {
                    self.stint_stuck();
                    self.idle(town);
                    self.settle(mark, None, &Ok(()));
                }
                Err(stuck) => {
                    self.entries.push(Entry::Stuck { reason: stuck.0 });
                    self.stint_stuck();
                    self.idle(town);
                    self.settle(mark, None, &Ok(()));
                }
            }
            if let Some(town) = town {
                // A citizen's entries go to its town's ledger (C1.7)
                self.journal();
                town.record(std::mem::take(&mut self.entries), self.mind.take_rulings());
                let strands = self.bot.state().strands();
                let road = self.road_progress().map(|p| RoadInReport::of(&p));
                let now = self.now();
                let stint = self.stints.on.map(|s| (s.job, s.run(now)));
                town.health(|h| {
                    h.strands = strands;
                    h.road_in = road.or(h.road_in.take());
                    if let Some((job, run)) = stint {
                        h.stint = Some(job);
                        h.stint_hours = run;
                    } else {
                        h.stint = None;
                    }
                });
                // Its stint and what it did lately, for its next session
                self.credit();
                town.keep(self.stints.kept(self.now()));
            }
        }
        Ok(town.map_or(Ended::Stopped, Tether::ended))
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
            Action::Road(step) => format!("the road in: {}", road_in::does(*step)),
            Action::Board(act) => act.label(),
            Action::Wage(act) => act.label(),
            Action::Hire(act) => act.label(),
            Action::Guild(act) => act.label(),
            Action::Detour(d) => format!("a detour: {}", d.does),
        }
    }

    /// What the report files `action`'s time under (Callings K0.1, plan
    /// D2), and whether it's setup. A step, a sale, a buy and a bid are its
    /// errand's `job`, and a tool bought is setup, as making it would be
    /// (K2.1); a stall and a market are trading; the board, posts and
    /// wages are the Commons; market hours with nothing to trade are free.
    fn activity(action: &Action, job: Activity, goal: Option<&Goal>, catalog: &Catalog) -> (Activity, bool) {
        match action {
            Action::Step(step) => (job, activity::is_setup(catalog, goal, step)),
            Action::Trade(Trade::Buy { item, .. } | Trade::Bid { item, .. } | Trade::Shop { item, .. }) => (job, is_tool(item)),
            Action::Trade(Trade::Collect) => (job, true),
            Action::Trade(Trade::Sell { .. }) => (job, false),
            Action::Trade(Trade::Stock { .. } | Trade::MakeMarket) => (Activity::Trading, false),
            Action::Trade(Trade::Post { .. }) | Action::Board(_) | Action::Wage(_) | Action::Hire(_) | Action::Guild(_) => {
                (Activity::Commons, false)
            }
            Action::Road(_) => (Activity::RoadIn, false),
            Action::Market => (Activity::Free, false),
            Action::Detour(_) => (Activity::Detour, false),
        }
    }

    /// The day's clock's day now ([`rules::day_number`]), by the server's
    /// clock once it's known.
    fn day(&self) -> i64 {
        let now = self.bot.state().server_time_ms().unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64)
        });
        capitol_protocol::rules::day_number(now)
    }

    /// Its free time, and an entry for how long it took (K0.1), less a
    /// detour taken in it, which has its own (K1.5).
    fn idle(&mut self, town: Option<&Tether>) {
        let (began, day) = (self.now(), self.day());
        let detour = self.free_time(town);
        self.entries.push(Entry::Idle {
            hours: (self.now() - began - detour).max(0.0),
            day,
        });
    }

    /// Its guild's business, if any is due (S4.5): the decision to join is
    /// settled against the call that came of it. A call that fails is on its
    /// health line, and only a final error ends the round.
    fn mind_guild(&mut self, phase: Option<Phase>, town: Option<&Tether>) -> Result<(), BotError> {
        let mark = self.mind.mark();
        let holdings = self.holdings();
        let act = match self.choose_guild(&holdings, phase) {
            Ok(act) => act,
            Err(e) if e.is_final() => return Err(e),
            Err(_) => None,
        };
        let Some(act) = act else {
            self.settle(mark, None, &Ok(()));
            return Ok(());
        };
        let result = self.guild_act(&act);
        let label = act.label();
        self.settle(mark, Some(&Action::Guild(act)), &result);
        match result {
            Ok(()) => Ok(()),
            Err(e) if e.is_final() => Err(e),
            Err(e) => {
                self.entries.push(Entry::Failed {
                    label: label.clone(),
                    why: e.to_string(),
                });
                if let Some(town) = town {
                    town.health(|h| h.failed(format!("{label}: {e}")));
                }
                Ok(())
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
            self.spot = None;
        }
        // A spot on the steps or along the lane (C2.1), and a game minute
        // there. Someone asking to trade cuts it short: answered at the top
        let bazaar = self.persona.as_ref().is_some_and(|p| p.market == Market::Bazaar);
        self.market_spot(town, bazaar)
    }

    /// Dusk: buys a home if the citizen has saved enough and has none, takes
    /// down its benches near it off its lot ([`Self::pack_up`], #488), and
    /// walks home along the avenues, for the night's work (#276). Something
    /// going wrong on the way doesn't keep it from that: it works the night
    /// from where it stands.
    fn go_home(&mut self, town: &Tether) -> Result<(), BotError> {
        town.health(|h| h.doing(Standing::Working, "going home"));
        // The end of its day ends a shift it's on (S2.8)
        match self.end_shift("day") {
            Ok(()) => {}
            Err(e) if e.is_final() => return Err(e),
            Err(e) => town.health(|h| h.failed(format!("stopping its shift: {e}"))),
        }
        // and the work it offered (K2.4), and what that left in its boxes
        // comes home with it
        match self.end_hires("day").and_then(|()| self.collect_boxes()) {
            Ok(()) => {}
            Err(e) if e.is_final() => return Err(e),
            Err(e) => town.health(|h| h.failed(format!("taking its offers of work down: {e}"))),
        }
        self.at_market = false;
        self.spot = None;
        self.night_shift = true;
        // A citizen on the road in doesn't save for a home: the road's deed
        // is its first (#450)
        if self.bot.state().plots().is_empty() && !self.awaits_road_deed() {
            match self.buy_home() {
                Ok(()) => {}
                Err(e) if e.is_final() => return Err(e),
                Err(e) => town.health(|h| h.failed(format!("buying a home: {e}"))),
            }
        }
        // Its benches off its lot come home in its pack (#488), those it
        // had lost sight of too (#500): the near ones, and a far one
        match self.learn_own().and_then(|()| self.pack_up()).and_then(|_| self.fetch_far()) {
            Ok(_) => {}
            Err(e) if e.is_final() => return Err(e),
            Err(e) => town.health(|h| h.failed(format!("packing up: {e}"))),
        }
        let (x, y) = self.home();
        // A while on a bridge first, if one's near the way (C2.1)
        self.by_the_bridge(town, (x, y));
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
        // Home, its stations there that the weather has worn (#478)
        if !town.stopped() {
            match self.mend() {
                Ok(_) => {}
                Err(e) if e.is_final() => return Err(e),
                Err(e) => town.health(|h| h.failed(format!("repairing its stations: {e}"))),
            }
        }
        Ok(())
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
    /// what its thrift wants in hand for one, and its [`FLOAT`] past the deed.
    fn buy_home(&mut self) -> Result<(), BotError> {
        let Some(persona) = self.persona.clone() else {
            return Ok(());
        };
        let strands = self.bot.state().strands().unwrap_or(0);
        let fund = |cost: i64| persona.temperament.home_fund(cost).max(cost + FLOAT);
        if strands < fund(self.deed_cost) {
            return Ok(());
        }
        self.claim_near(&[persona.home.zone.as_str()], persona.home.spot(), 1, false, |cost| fund(cost) <= strands, false)
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

    /// Asks the mind what to work on, from the job of the moment's errands.
    /// While a layer thinks, it carries on with the last step's errand.
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

    /// Asks the mind whether to sell what it has spare before `errand`
    /// (Callings K1.4: whatever job it came of), and if not, whether to buy
    /// the tool its step climbs toward (K2.1), or what it would gather.
    /// `None`: just do it. A merchant makes its market on a stint at
    /// trading.
    fn choose_trade(&mut self, holdings: &Holdings, errand: &Errand) -> Option<Trade> {
        let job = self.job();
        if self.merchant.is_some() && job == Role::Merchant {
            return self.choose_market(holdings, errand);
        }
        // What a claim of its own wants isn't spare (S2.5): one question at
        // the hall, for the first good someone bids for, then one about a
        // stall, for the most it has spare of what's enough for one (a
        // good's STALL_AT, or tools, K2.1)
        let spare = self.spare(holdings, errand.goal.as_ref());
        let sale = spare.iter().find_map(|&(item, n)| {
            let (bid, ask) = self.best(item);
            Some((item, super::sell_menu(holdings, item, n, bid, ask)?))
        });
        if let Some((item, menu)) = sale {
            let question = Question::Sell { item: item.into() };
            if let Some((d, Choice::Sell { item, .. })) = self.decide(question, menu, 1, holdings) {
                return Some(Trade::Sell {
                    item,
                    quantity: d.quantity?,
                    price: d.price?,
                });
            }
        }
        let stall = spare.iter().find(|&&(item, n)| n >= (super::stall_at(item) / 2).max(1));
        if let Some(&(item, n)) = stall {
            if let Some(stock) = self.choose_stall(holdings, item, n) {
                return Some(stock);
            }
        }
        if let Some(tool) = self.choose_tool(holdings, errand) {
            return Some(tool);
        }
        let Step::Gather { item, .. } = &errand.step else {
            return None;
        };
        let (_, ask) = self.best(item);
        let Some(menu) = job.buy_menu(holdings, &errand.step, ask) else {
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

    /// Time on its hands: rest where it is, or pass it at the hall. A
    /// citizen has more places to be (C2.1, [`idle`]), and may wander off
    /// on a detour (K1.5): the game hours that took.
    fn free_time(&mut self, town: Option<&Tether>) -> f64 {
        let holdings = self.holdings();
        if self.night_shift {
            self.night_in(town);
            return 0.0;
        }
        if self.persona.is_some() {
            return self.idle_time(town, &holdings, None);
        }
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
                self.rest(town, "resting at the hall");
                return 0.0;
            }
        }
        self.rest(town, "resting");
        0.0
    }

    /// Logs what came of the decisions since `mark` (C3.1), when `action`
    /// was what they came to (`None`: free time) and `result` how it went.
    fn settle(&mut self, mark: usize, action: Option<&Action>, result: &Result<(), BotError>) {
        if self.mind.mark() <= mark {
            return;
        }
        let after = Held::from(&self.holdings());
        self.mind.settle(mark, &after, |choice| fate(choice, action, result));
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
                calling: self.persona.as_ref().map(|p| p.calling.clone()),
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

    /// The fewest uses left a buy of the tool `item` at `price` asks for
    /// (#547): those of the ask on its book that's the cheapest a use at
    /// the price, so it gets that tool or a fresher one, never a worn one at
    /// the same price; with none, [`fewest_uses`](super::fewest_uses). `None`
    /// for a good.
    fn uses_wanted(&self, item: &str, price: i64) -> Option<i32> {
        let state = self.bot.state();
        let half = super::fewest_uses(state.catalog(), item)?;
        let life = state.catalog().item_type(item)?.tool.as_ref()?.max_durability;
        let on_book = state.market().book(item).and_then(|b| b.cheapest_use(life, Some(price)));
        Some(on_book.map_or(half, |(_, uses)| uses.max(1)))
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
            // A standing bid's fills, and a stall's sales, go in the
            // decision log against what put them up (C3.1)
            match &change {
                StateEvent::Market(event) => match &**event {
                    MarketEvent::OrderFilled { order, trade } => {
                        if order.side == OrderSide::Buy {
                            self.board.filled(&order.item_type);
                            if is_tool(&order.item_type) {
                                self.boxed_tools.insert(order.item_type.clone());
                            }
                        }
                        self.mind.filled(
                            order.id,
                            trade.quantity as i64,
                            trade.price * trade.quantity as i64,
                            order.status != OrderStatus::Open,
                        )
                    }
                    MarketEvent::OrderClosed(order) => self.mind.closed(order.id),
                    _ => {}
                },
                StateEvent::BazaarSale {
                    item_type, quantity, paid, ..
                } => self.mind.sold(item_type, *quantity, *paid),
                _ => {}
            }
            match change {
                // A failed pull (D5, #206)
                StateEvent::ExtractionCompleted { item, .. } if item.quantity == 0 => {
                    self.entries.push(Entry::Missed { item: item.item_type, at })
                }
                // What it makes on a shift is the employer's, not a day's
                // work of its own (S2.8)
                StateEvent::ExtractionCompleted { item, .. } => {
                    if !self.wages.on_shift() {
                        self.board.made(&item.item_type, item.quantity);
                    }
                    self.entries.push(Entry::Made {
                        item: item.item_type,
                        quantity: item.quantity,
                        quality: item.quality,
                        gathered: true,
                        at,
                    })
                }
                StateEvent::CraftingCompleted { item } => {
                    if !self.wages.on_shift() {
                        self.board.made(&item.item_type, item.quantity);
                    }
                    self.entries.push(Entry::Made {
                        item: item.item_type,
                        quantity: item.quantity,
                        quality: item.quality,
                        gathered: false,
                        at,
                    })
                }
                // The board's notices (S2.5). A fill a person delivered is
                // counted here; one a citizen made, by its delivery
                StateEvent::CommissionDelivered {
                    item_type,
                    deliverer_citizen,
                    left,
                    ..
                } => {
                    self.board.stale();
                    if left == 0 && !deliverer_citizen {
                        self.entries.push(Entry::Filled { item: item_type });
                    }
                }
                StateEvent::CommissionExpired { item_type, refunded, .. } => {
                    self.board.stale();
                    self.entries.push(Entry::Expired {
                        item: item_type,
                        refunded,
                    });
                }
                StateEvent::ClaimLapsed {
                    item_type,
                    claimant_id,
                    quantity,
                    delivered,
                    ..
                } => {
                    self.board.stale();
                    if Some(claimant_id) == self.bot.state().me() {
                        self.entries.push(Entry::Lapsed {
                            item: item_type,
                            quantity: quantity - delivered,
                        });
                    }
                }
                // Wages (S2.8): the board's offers, its pay, and a shift's end
                StateEvent::WageBoard(offer) => {
                    self.hire_heard(&offer);
                    self.wages.update(offer)
                }
                StateEvent::WagePaid {
                    item_type, units, paid, ..
                } => self.entries.push(Entry::Earned {
                    item: item_type,
                    units,
                    paid,
                }),
                StateEvent::WageShiftEnded { offer_id, shift, .. } => {
                    if Some(shift.worker_id) == self.bot.state().me() {
                        let why = shift.end.map_or("ended", |e| e.as_str());
                        self.shift_ended(offer_id, why, shift.made);
                    }
                }
                StateEvent::ToolBroke { item_type, .. } => {
                    self.entries.push(Entry::ToolBroke { item: item_type, at })
                }
                // Guilds (S4.5): read again, and a guild it's out of
                StateEvent::GuildChanged { .. }
                | StateEvent::GuildNotice { .. }
                | StateEvent::GuildDues { .. }
                | StateEvent::GuildDissolved { .. } => self.guild_event(&change),
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
            fees: state.fees(),
            road_deed: self.road_deed(),
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
        h.stations.extend(self.stations.values().map(|s| s.station_type.clone()));
        // What fits on the lot, not just its capacity: a narrow suburban lot
        // runs out of ground first (#305). One that would take the rest of
        // its chain packed again is ([`Self::repack`], #405). Its floors
        // that are up have their own room, where no fire goes (#478)
        let now = self.server_now();
        if let Some(plot) = state.plots().first() {
            let left = (plot.station_capacity - plot.station_count).max(0);
            let upstairs = lot::room_upstairs(plot, &self.stations_placed(), plot.open_floors(now));
            let room = (lot::room(plot, &self.stations_at()) + upstairs).min(left);
            h.plot_room = Some(room);
            h.upstairs = upstairs.min(room);
            h.floor = self.upward(plot, now);
        }
        // Its own under 25: the work at one mends it first (#478), but not
        // one whose repair was refused lately
        let now_hours = self.now();
        for s in self.working() {
            let kind = &s.station_type;
            let condition = state.station(s.id).map_or(s.condition, |s| s.condition);
            let refused = self.refused_repairs.get(kind).is_some_and(|&at| now_hours - at < REPAIR_RETRY_HOURS);
            if condition < capitol_protocol::rules::weather::NEEDS_REPAIR && !refused {
                let bill = repair_bill(catalog, kind, condition);
                if !bill.is_empty() {
                    h.worn.insert(kind.clone(), bill);
                }
            }
        }
        // Past what its calling still wants for its own, the room a
        // workbench may take, so it stays home (#499), and a dabble's
        // station outside its calling (D6)
        let one_job = self.persona.is_none();
        let (lot_for, spare) = lot_kept(&self.calling, one_job, |kind| self.owns(kind), h.plot_room);
        h.lot_for = lot_for;
        h.plot_spare = spare;
        h.owed = self.owed();
        h.promised = self.board.promised();
        // A kit a claim of its own has promised is a patron's (K2.3): not
        // its own to put down, so a builder never places the kit it owes
        for (item, n) in &h.promised {
            if catalog.station_for_kit(item).is_some() {
                if let Some(held) = h.items.get_mut(item) {
                    *held = (*held - n).max(0);
                }
            }
        }
        // Its tools at its stall and on the book (K2.1)
        let stalled: i64 = state
            .bazaar()
            .my_shop()
            .map_or(0, |shop| shop.listings.iter().filter(|l| is_tool(&l.item_type)).map(|l| l.units.count()).sum());
        let booked: i32 = state
            .market()
            .open_orders()
            .iter()
            .filter(|o| o.side == OrderSide::Sell && is_tool(&o.item_type))
            .map(|o| o.quantity_remaining)
            .sum();
        h.tools_on_sale = stalled as i32 + booked;
        h
    }

    /// The next floor its `plot` may go up by (Plots v2 §5.8, #478): a
    /// citizen's urban lot with a building, and none going up. Not outside
    /// the city, where the yard is cheap room, nor for a swarm bot, so the
    /// E2.7 sim's numbers stay comparable.
    fn upward(&self, plot: &capitol_protocol::rest::PlotInfo, now: chrono::DateTime<chrono::Utc>) -> Option<Upward> {
        if self.persona.is_none() || Zone::from_str(&plot.zone_id) != Some(Zone::Urban) || plot.open_floors(now) < plot.floors {
            return None;
        }
        let bill = plot.next_floor.as_ref()?;
        Some(Upward {
            permit: bill.permit,
            goods: bill.goods.iter().map(|g| (g.item_type.clone(), g.quantity)).collect(),
            room: plot.indoor_room.unwrap_or(0).max(0),
        })
    }

    /// The server's clock now, or this machine's before it's known.
    fn server_now(&self) -> chrono::DateTime<chrono::Utc> {
        let ms = self.bot.state().server_time_ms().unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64)
        });
        chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default()
    }

    /// How many of its calling's stations for a lot it has yet to place
    /// ([`still_wanted`]).
    fn still_wanted(&self) -> i32 {
        still_wanted(&self.calling, |kind| self.owns(kind))
    }

    /// Takes down its own stations on its lot, so the planner puts them back
    /// where [`lot::spots`] packs them (#405), when the lot has no room left
    /// for the rest of its role's chain but would, packed again. A builder
    /// whose kiln went in the middle of a narrow lot before #305 stayed
    /// stuck at the forge. One at work stays, and the others are packed
    /// round it. Once a lot ([`Self::repacked`]). One on a lot too small
    /// even empty stays stuck: a deed can't be given up (#519). How many
    /// came down.
    fn repack(&mut self, holdings: &Holdings) -> Result<usize, BotError> {
        let state = self.bot.state();
        let Some(plot) = state.plots().first().cloned() else {
            return Ok(0);
        };
        let still = self.still_wanted();
        let short = holdings.plot_room.is_some_and(|room| room < still);
        if !short || self.repacked == Some(plot.id) {
            return Ok(0);
        }
        // What the store last heard of each, for a job set since it was placed
        let ours: Vec<(String, StationInfo)> = self
            .stations
            .values()
            .map(|s| (s.station_type.clone(), state.station(s.id).cloned().unwrap_or_else(|| s.clone())))
            .collect();
        let (moving, staying): (Vec<_>, Vec<_>) =
            ours.into_iter().partition(|(_, s)| idle(s) && s.floor == 0 && plot.contains(s.x, s.y));
        if moving.is_empty() {
            return Ok(0);
        }
        let mine = |id: Uuid| self.stations.contains_key(&id);
        let fixed: Vec<(f64, f64)> = state
            .stations()
            .values()
            .filter(|s| !mine(s.id))
            .chain(staying.iter().map(|(_, s)| s))
            .filter(|s| s.floor == 0)
            .map(|s| (s.x, s.y))
            .collect();
        if !lot::repack_frees(&plot, &fixed, moving.len(), still as usize) {
            return Ok(0);
        }
        self.repacked = Some(plot.id);
        let (began, day) = (self.now(), self.day());
        let mut down = 0;
        for (_, s) in moving {
            self.bot.walk_to(s.x, s.y + 10.0)?;
            match self.bot.remove_station(s.id) {
                Ok(()) => {
                    self.stations.remove(&s.id);
                    down += 1;
                }
                // The pack has no room for the kit and what's in its box:
                // left standing, and the others packed round it
                Err(BotError::Rest { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        // In the report's steps, so a soak shows whether any lot was packed
        // up, and when (K1.3: never on a change of stint)
        if down > 0 {
            self.entries.push(Entry::Spent {
                label: format!("pack its lot up again ({down} down)"),
                hours: self.now() - began,
                ok: true,
                activity: Activity::Building,
                setup: true,
                day,
            });
        }
        Ok(down)
    }

    fn take(&mut self, step: &Step) -> Result<(), BotError> {
        match step {
            Step::Gather { resource, .. } => self.gather(resource),
            Step::Craft { recipe, station } => {
                let at = match station {
                    Some(kind) => {
                        let s = self.own(kind).cloned().ok_or_else(|| {
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
                // Made once, it's no longer something new to try (K1.5)
                self.stints.made.insert(recipe.clone());
                self.bot.collect_output()
            }
            Step::Place {
                kit,
                station,
                on_plot,
            } => self.place(kit, station, *on_plot),
            Step::Claim => self.claim(),
            Step::Floor => self.build_floor(),
            Step::Repair { station } => self.repair(station),
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
        // These collect the market box, tools and all
        if matches!(trade, Trade::Sell { .. } | Trade::Buy { .. } | Trade::Bid { .. } | Trade::Collect) {
            self.boxed_tools.clear();
        }
        match trade {
            Trade::MakeMarket => return self.make_market(None),
            Trade::Stock { item, quantity, price } => return self.keep_stall(item, *quantity, *price),
            Trade::Bid { item, quantity, price } => {
                self.bot.go_to_exchange()?;
                self.bid(item, *quantity, *price)?;
                self.bot.collect_market_box()?;
            }
            Trade::Post { item, quantity, price } => {
                self.bot.go_to_exchange()?;
                self.post(item, *quantity, *price)?;
            }
            Trade::Shop { stall, item, price } => return self.buy_at_a_stall(stall, item, *price),
            Trade::Collect => {
                self.bot.go_to_exchange()?;
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
                // The biggest stack; of tools, which don't stack, the
                // freshest, so it keeps its most worn to use up. The book
                // shows a tool's wear (#547), so it's priced by the share of
                // its uses left, as at a stall
                let (item_id, held, price) = self
                    .bot
                    .state()
                    .inventory()
                    .and_then(|inv| {
                        inv.items()
                            .filter(|i| &i.item_type == item)
                            .max_by_key(|i| (i.quantity, i.durability.unwrap_or(0)))
                    })
                    .map(|i| (i.item_id, i.quantity, people::worn_price(*price, i)))
                    .ok_or_else(|| BotError::Failed(format!("no {item} to sell")))?;
                PlaceOrder {
                    side: OrderSide::Sell,
                    item_type: item.clone(),
                    price,
                    quantity: (*quantity).min(held),
                    item_id: Some(item_id),
                    qualities: None,
                    min_grade: None,
                    grade_rule: None,
                    min_durability: None,
                }
            }
            Trade::Bid { .. } | Trade::Stock { .. } | Trade::MakeMarket | Trade::Post { .. } | Trade::Shop { .. } | Trade::Collect => {
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
                min_durability: self.uses_wanted(item, *price),
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

    /// One pull from a node of `resource`, or of any resource that yields
    /// the same with the same tool (#336), finding one first.
    ///
    /// For a claim whose goods come from the wilds (S2.5), only a node out
    /// there will do, and for a claim at a grade, only a node at it (#504).
    fn gather(&mut self, resource: &str) -> Result<(), BotError> {
        let kinds = kinds(self.bot.state().catalog(), resource);
        let yields = self.bot.state().catalog().resource_type(resource).map(|r| r.yields_item.clone());
        let wild = yields.as_ref().is_some_and(|item| self.board.wild(item));
        let at_least = yields.as_ref().and_then(|item| self.board.grade_wanted(item));
        let mut explored = 0;
        for _ in 0..MAX_EXPLORES {
            self.learn_nodes();
            let pick = self.random();
            let Some(node) = self.nearest(&kinds, pick, wild, at_least) else {
                self.explore(&kinds, explored)?;
                explored += 1;
                continue;
            };
            match self.bot.extract(&node) {
                Ok(_) => return Ok(()),
                // A blocked walk: the walk graph has no way to the node (one
                // wild node in 25 is in a pocket walled by crests, #280), or
                // the server stopped it. The node is shunned and another tried.
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

    /// A near node of one of `kinds` that nobody is gathering and that's
    /// available, or was depleted long enough ago to have grown back. One of
    /// the three nearest, picked by `pick` (in [0, 1)), so a crowd spreads
    /// out rather than queueing at one patch. With `wild`, only one in the
    /// Wilderness. With `at_least`, only one whose quality is that or more:
    /// a gather never comes out better than its node (GDD §6.2), so a node
    /// under a claim's grade is a wasted pull (#504). A node from a server
    /// that doesn't say its quality is taken on trust.
    fn nearest(&self, kinds: &[String], pick: f64, wild: bool, at_least: Option<i32>) -> Option<ResourceNodeInfo> {
        let regrown: HashMap<&str, Duration> = kinds
            .iter()
            .map(|kind| {
                let regrow = self
                    .bot
                    .state()
                    .catalog()
                    .resource_type(kind)
                    .map_or(600.0, |r| r.regeneration_ticks as f64 / 10.0);
                (kind.as_str(), self.wall(regrow))
            })
            .collect();
        let (x, y) = self.bot.position();
        let now = Instant::now();
        let state = self.bot.state();
        let mut near: Vec<&ResourceNodeInfo> = self
            .atlas
            .values()
            .filter(|s| s.shunned_until.is_none_or(|t| now >= t))
            .filter(|s| {
                regrown.get(s.node.resource_type.as_str()).is_some_and(|regrown| {
                    s.node.state == "available" || now.duration_since(s.at) >= *regrown
                })
            })
            .filter(|s| state.harvester(s.node.id).is_none())
            .filter(|s| worth_a_pull(&s.node, wild, at_least))
            .map(|s| &s.node)
            .collect();
        near.sort_by(|a, b| (a.x - x).hypot(a.y - y).total_cmp(&(b.x - x).hypot(b.y - y)));
        near.truncate(3);
        let i = (pick * near.len() as f64) as usize;
        near.get(i.min(near.len().saturating_sub(1))).map(|n| (*n).clone())
    }

    /// Walks somewhere new where one of `kinds` grows, then asks what's
    /// around: in the nearest zone it grows in, or out in the wilds (#336),
    /// first for a citizen who [ventures](Self::ventures), and for anyone
    /// once [`WILD_AFTER`] explores nearer in have come to nothing. This
    /// gather has explored `explored` times already.
    fn explore(&mut self, kinds: &[String], explored: usize) -> Result<(), BotError> {
        let (x, y) = self.bot.position();
        let (cx, cy) = (Zone::CENTER_X, Zone::CENTER_Y);
        let r_now = (x - cx).hypot(y - cy);
        let a_now = (y - cy).atan2(x - cx);
        let catalog = self.bot.state().catalog();
        let zones: Vec<Zone> = kinds
            .iter()
            .filter_map(|kind| catalog.resource_type(kind))
            .flat_map(|r| r.spawns.iter())
            .filter(|s| s.density > 0.0)
            .filter_map(|s| Zone::from_str(&s.zone))
            .collect();
        // Ore is in the rock: for what a pickaxe works, it looks up at the
        // high ground, which in the wilds finds iron twice as often (#336)
        let dug = kinds
            .iter()
            .filter_map(|kind| catalog.resource_type(kind))
            .any(|r| r.tool_type.as_ref().or(r.required_tool.as_ref()).is_some_and(|t| t == "pickaxe"));
        let wilds = self.ventures() || explored >= WILD_AFTER;
        let (inner, outer) = search_band(&zones, r_now, wilds)
            .unwrap_or((rules_spawn_radius() - 300.0, rules_spawn_radius() + 300.0));
        let walks = if dug { self.bot.walks().ok() } else { None };
        for _ in 0..EXPLORE_TRIES {
            let looks = if walks.is_some() { HIGH_GROUND_LOOKS } else { 1 };
            let mut spots = Vec::with_capacity(looks);
            for _ in 0..looks {
                let r = inner + 60.0 + (outer - inner - 120.0).max(0.0) * self.random();
                // Along the ring from here, a little either way.
                let a = a_now + (self.random() - 0.5) * 0.6;
                spots.push((cx + r * a.cos(), cy + r * a.sin()));
            }
            let height = |&(x, y): &(f64, f64)| {
                walks.as_ref().map_or(0.0, |w| land::height_at(x, y, w.ground().land()))
            };
            let (x, y) = spots
                .into_iter()
                .max_by(|a, b| height(a).total_cmp(&height(b)))
                .expect("at least one spot");
            match self.bot.walk_to(x, y) {
                // A spot the walk graph can't reach, in a lake, say: it never
                // set out, so another is tried (#280)
                Err(BotError::Blocked { reason, .. }) if reason == NO_WAY => continue,
                Ok(()) | Err(BotError::Blocked { .. }) => break,
                Err(e) => return Err(e),
            }
        }
        self.bot.refetch(Fetch::NearbyNodes);
        self.bot.settle()
    }

    /// Whether it looks in the wilds first when it explores (#336): a
    /// citizen who takes chances, about a third of them, walks out to the
    /// ranges for their better grade. A swarm bot, with no persona, doesn't.
    ///
    /// A claim on ore from the wilds sends anyone out there (S2.5).
    fn ventures(&self) -> bool {
        self.board.wild("iron_ore")
            || self
                .persona
                .as_ref()
                .is_some_and(|p| p.temperament.risk >= VENTURE_RISK)
    }

    fn place(&mut self, kit: &str, station: &str, on_plot: bool) -> Result<(), BotError> {
        let kit_id = self
            .bot
            .state()
            .inventory()
            .and_then(|inv| inv.items().find(|i| i.item_type == kit).map(|i| i.item_id))
            .ok_or_else(|| BotError::Failed(format!("no {kit} in the inventory")))?;
        let spots: Vec<lot::Spot> = if on_plot {
            let plot = self
                .bot
                .state()
                .plots()
                .first()
                .cloned()
                .ok_or_else(|| BotError::Failed("no plot".into()))?;
            // The lot's own shape, not its box, packed round what's on it
            // (#305): indoors, then upstairs, then the yard (#478), and the
            // ground's last indoor spots left for the fires still to come
            let catalog = self.bot.state().catalog();
            let fire = |kind: &str| catalog.station_type(kind).is_some_and(|t| t.ground_only);
            let keep = super::stint::lot_for(&self.calling)
                .into_iter()
                .filter(|&kind| kind != station && fire(kind) && !self.owns(kind))
                .count();
            let open = plot.open_floors(self.server_now());
            let mut spots = lot::spots_for(&plot, &self.stations_placed(), open, fire(station), keep);
            spots.truncate(PLACE_TRIES);
            spots
        } else {
            off_lot_spots(self.bot.position(), &self.stations_at()).into_iter().map(|p| (p, 0)).collect()
        };
        if spots.is_empty() {
            return Err(BotError::Failed(format!("no room on the plot for a {station}")));
        }
        // Why each spot was passed over, for the error if none takes (#305)
        let mut passed: Vec<String> = Vec::new();
        let tried = spots.len();
        let mut last = None;
        for ((x, y), floor) in spots {
            match self.bot.walk_to(x, y) {
                Ok(()) => {}
                Err(e @ BotError::Blocked { .. }) => {
                    passed.push(e.to_string());
                    last = Some(e);
                    continue;
                }
                Err(e) => return Err(e),
            }
            match self.bot.place_station_on(station, kit_id, x, y, floor) {
                Ok(placed) => {
                    self.stations.insert(placed.id, placed);
                    return Ok(());
                }
                Err(e @ BotError::Rest { .. }) => {
                    passed.push(e.to_string());
                    last = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        passed.sort();
        passed.dedup();
        match last {
            Some(e) if tried == 1 => Err(e),
            _ => Err(BotError::Failed(format!(
                "nowhere to put a {station}, {tried} spots tried: {}",
                passed.join("; ")
            ))),
        }
    }

    /// Where the stations it knows of stand on the ground: its own, and
    /// those it has seen nearby. One upstairs (V3.3) is in a place of its
    /// own, over the ground's spots ([`Self::stations_placed`]).
    fn stations_at(&self) -> Vec<(f64, f64)> {
        self.stations_placed().into_iter().filter(|s| s.1 == 0).map(|s| s.0).collect()
    }

    /// Where the stations it knows of stand, each on its storey (#478).
    fn stations_placed(&self) -> Vec<lot::Spot> {
        known_stations(self.bot.state().stations(), self.stations.values())
    }

    /// Builds its lot's next floor (Plots v2 §5.8, #478), for a station its
    /// chain needs and its lot has no room for.
    fn build_floor(&mut self) -> Result<(), BotError> {
        let plot = self.bot.state().plots().first().map(|p| p.id).ok_or_else(|| BotError::Failed("no plot".into()))?;
        self.bot.build_floor(plot)?;
        Ok(())
    }

    /// Repairs its own `kind` of station, at it (#478).
    fn repair(&mut self, kind: &str) -> Result<(), BotError> {
        let s = self.own(kind).cloned().ok_or_else(|| BotError::Failed(format!("no {kind} of ours is known")))?;
        self.bot.walk_to(s.x, s.y + 10.0)?;
        let condition = match self.bot.repair_station(s.id) {
            Ok(condition) => condition,
            Err(e @ BotError::Rest { .. }) => {
                self.refused_repairs.insert(kind.to_string(), self.now());
                return Err(e);
            }
            Err(e) => return Err(e),
        };
        self.refused_repairs.remove(kind);
        if let Some(ours) = self.stations.get_mut(&s.id) {
            ours.condition = condition;
        }
        Ok(())
    }

    /// Home for the night, repairs its stations on its lot that are under
    /// [`MEND_BELOW`], those it holds the goods for (Plots v2 §5.8, #478).
    /// One under 25 it hasn't the goods for waits for its work, which makes
    /// them ([`Holdings::worn`]). How many it repaired.
    fn mend(&mut self) -> Result<usize, BotError> {
        let state = self.bot.state();
        let Some(plot) = state.plots().first().cloned() else {
            return Ok(0);
        };
        let catalog = state.catalog().clone();
        let holdings = self.holdings();
        let mut due: Vec<(String, i32)> = self
            .working()
            .into_iter()
            .map(|s| state.station(s.id).cloned().unwrap_or(s))
            .filter(|s| s.condition < MEND_BELOW && plot.contains(s.x, s.y))
            .map(|s| (s.station_type, s.condition))
            .collect();
        // The most worn first, while the goods last
        due.sort_by_key(|&(_, condition)| condition);
        let (began, day) = (self.now(), self.day());
        let mut held = holdings.items;
        let mut mended = 0;
        for (kind, condition) in due {
            let bill = repair_bill(&catalog, &kind, condition);
            if bill.is_empty() || bill.iter().any(|(item, n)| held.get(item).copied().unwrap_or(0) < *n) {
                continue;
            }
            match self.repair(&kind) {
                Ok(()) => {
                    for (item, n) in &bill {
                        *held.entry(item.clone()).or_insert(0) -= n;
                    }
                    mended += 1;
                }
                // Refused (its bill came out one more than ours, say): left
                // for another night
                Err(BotError::Rest { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        // In the report's steps, so a soak shows repairs and when
        if mended > 0 {
            self.entries.push(Entry::Spent {
                label: format!("repair its stations ({mended})"),
                hours: self.now() - began,
                ok: true,
                activity: Activity::Building,
                setup: true,
                day,
            });
        }
        Ok(mended)
    }

    /// Takes down its own stations near it that stand off its lot (#488).
    /// The whole kit comes back, so a bench it's done with goes in its pack
    /// rather than standing by the road for good; the spawn filled up with
    /// Arrival's benches. One set to work stays. What's in a box comes home
    /// with the kit (#489). How many came down.
    pub(super) fn pack_up(&mut self) -> Result<usize, BotError> {
        let state = self.bot.state();
        let plots = state.plots();
        let known = self.known_own();
        let leaving = leaving(known, self.bot.position(), |x, y| plots.iter().any(|p| p.contains(x, y)));
        let mut down = 0;
        for (_, s) in leaving {
            self.bot.walk_to(s.x, s.y + 10.0)?;
            match self.bot.remove_station(s.id) {
                Ok(()) => {
                    self.stations.remove(&s.id);
                    down += 1;
                }
                // The pack has no room for the kit and what's in its box:
                // left standing, for another day
                Err(BotError::Rest { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(down)
    }

    /// Goes back for one of its stations off its lot beyond
    /// [`PACK_UP_REACH`] and takes it down (#500), once a day, nearest
    /// first: a bench from Arrival by the spawn, left there before it had a
    /// lot or before #499, that crowded new citizens out of Arrival's bench
    /// step. Only once it has a lot, where the planner puts the kit back
    /// down. How many came down.
    fn fetch_far(&mut self) -> Result<usize, BotError> {
        let day = self.day();
        let state = self.bot.state();
        let plots = state.plots();
        if plots.is_empty() || self.fetched_day == Some(day) {
            return Ok(0);
        }
        let known = self.known_own();
        let far = strays(known, self.bot.position(), |x, y| plots.iter().any(|p| p.contains(x, y)));
        self.fetched_day = Some(day);
        let began = self.now();
        let mut down = 0;
        for (_, s) in far.into_iter().take(FETCH_A_DAY) {
            self.bot.walk_to(s.x, s.y + 10.0)?;
            match self.bot.remove_station(s.id) {
                Ok(()) => {
                    self.stations.remove(&s.id);
                    down += 1;
                }
                // Its pack has no room, or it's gone: for another day, or
                // the next list of its own
                Err(BotError::Rest { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        // In the report's steps, so a soak shows the pile going
        if down > 0 {
            self.entries.push(Entry::Spent {
                label: format!("fetch a far station ({down} down)"),
                hours: self.now() - began,
                ok: true,
                activity: Activity::Building,
                setup: true,
                day,
            });
        }
        Ok(down)
    }

    /// Learns its own stations wherever they stand, from the server (#500):
    /// one the store has lost sight of (after a restart, or a second of a
    /// type, far off) is known again, and one gone (taken down another way)
    /// is forgotten.
    pub(super) fn learn_own(&mut self) -> Result<(), BotError> {
        let mine = self.bot.my_stations()?;
        self.stations = mine.into_iter().map(|s| (s.id, s)).collect();
        Ok(())
    }

    /// Its own stations by type, each as the store last heard of it if it
    /// has, for a job set since it was placed.
    fn known_own(&self) -> Vec<(String, StationInfo)> {
        let state = self.bot.state();
        self.stations
            .values()
            .map(|s| (s.station_type.clone(), state.station(s.id).cloned().unwrap_or_else(|| s.clone())))
            .collect()
    }

    /// Whether it owns a `kind` of station, anywhere.
    fn owns(&self, kind: &str) -> bool {
        self.stations.values().any(|s| s.station_type == kind)
    }

    /// Its working `kind` of station (#500): of two, the one on its lot,
    /// else the nearer.
    fn own(&self, kind: &str) -> Option<&StationInfo> {
        let plots = self.bot.state().plots();
        let (x, y) = self.bot.position();
        let rank = |s: &StationInfo| (!plots.iter().any(|p| p.contains(s.x, s.y)), (s.x - x).hypot(s.y - y));
        self.stations.values().filter(|s| s.station_type == kind).min_by(|a, b| {
            let (ra, rb) = (rank(a), rank(b));
            ra.0.cmp(&rb.0).then(ra.1.total_cmp(&rb.1)).then(a.id.cmp(&b.id))
        })
    }

    /// Its working station of each type it owns ([`Self::own`]).
    fn working(&self) -> Vec<StationInfo> {
        let mut kinds: Vec<&str> = self.stations.values().map(|s| s.station_type.as_str()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        kinds.into_iter().filter_map(|kind| self.own(kind).cloned()).collect()
    }

    /// Buys the nearest unclaimed plot it can afford with its [`FLOAT`] left
    /// over (#294) and room for its calling's stations (#419, Callings D6):
    /// for a citizen,
    /// near its home (C1.2), so the lot it builds on is the one it lives on.
    /// A citizen whose home zone has such a lot it can't afford yet saves for
    /// it rather than buying elsewhere; only a home zone with no lot that
    /// takes its stations sends it to the others.
    ///
    /// A citizen on the road in whose deed is open claims with it instead
    /// ([`road_in`]), as its first plot (#450).
    fn claim(&mut self) -> Result<(), BotError> {
        if self.road_deed() == Some(capitol_protocol::tutorial::Grant::Open) {
            return self.claim_deed();
        }
        let strands = self.bot.state().strands().unwrap_or(0) - FLOAT;
        // The room its calling wants, not its job of the moment (D6)
        let wanted = super::stint::room_for(&self.calling);
        match self.persona.as_ref().map(|p| p.home) {
            Some(home) => {
                let mut zones = vec![home.zone.as_str()];
                zones.extend(PLOT_ZONES.iter().filter(|z| **z != home.zone.as_str()));
                self.claim_near(&zones, home.spot(), wanted, true, |cost| cost <= strands, false)
            }
            None => {
                let here = self.bot.position();
                self.claim_near(&PLOT_ZONES, here, wanted, false, |cost| cost <= strands, false)
            }
        }
    }

    /// Buys the unclaimed plot nearest `(x, y)` whose deed it can `afford`,
    /// from the first of `zones` that has one with room for `wanted`
    /// stations, or failing that the first with any (#305). The deed it
    /// saves for is the first zone's (#419). With `wait_for_first`, a first
    /// zone that has a lot with the room, but none it can afford, is waited
    /// for: it buys nothing. With `granted`, it claims with the road in's
    /// deed (#450) and pays nothing.
    fn claim_near(
        &mut self,
        zones: &[&str],
        (x, y): (f64, f64),
        wanted: i32,
        wait_for_first: bool,
        afford: impl Fn(i64) -> bool,
        granted: bool,
    ) -> Result<(), BotError> {
        let strands = self.bot.state().strands().unwrap_or(0);
        let mut listed: Vec<Vec<capitol_protocol::rest::PlotInfo>> = Vec::new();
        for (i, &zone) in zones.iter().enumerate() {
            let mut plots = self.bot.unclaimed_plots(zone, (x, y))?;
            plots.retain(|p| p.deed_cost.is_some());
            let roomy = || plots.iter().filter(|p| lot::takes(p, wanted));
            if i == 0 {
                if let Some(cost) = roomy().filter_map(|p| p.deed_cost).min() {
                    self.deed_cost = cost as i64;
                }
                let affordable = roomy().any(|p| p.deed_cost.is_some_and(|c| afford(c as i64)));
                if wait_for_first && roomy().next().is_some() && !affordable {
                    return Err(BotError::Failed(format!(
                        "saving for a lot in {zone}: {} Strands, holding {strands}",
                        self.deed_cost
                    )));
                }
            }
            plots.retain(|p| p.deed_cost.is_some_and(|c| afford(c as i64)));
            listed.push(plots);
        }
        for wanted in [wanted, 1] {
            for plots in &listed {
                let mut plots: Vec<_> = plots.iter().filter(|p| lot::takes(p, wanted)).collect();
                plots.sort_by(|a, b| {
                    (a.world_x - x)
                        .hypot(a.world_y - y)
                        .total_cmp(&(b.world_x - x).hypot(b.world_y - y))
                });
                for plot in plots.iter().take(5) {
                    let claimed = if granted { self.bot.claim_with_grant(plot.id) } else { self.bot.claim_plot(plot.id) };
                    match claimed {
                        Ok(_) => {
                            let cost = if granted { 0 } else { plot.deed_cost.unwrap_or(0) as i64 };
                            if !granted {
                                self.deed_cost = cost;
                            }
                            self.entries.push(Entry::Claimed { cost });
                            self.bot.refetch(Fetch::Plots);
                            self.bot.settle()?;
                            return Ok(());
                        }
                        // The deed refused: not earned, or spent (#440). It
                        // would be refused on every lot, so say why
                        Err(e @ BotError::Rest { .. }) if granted && !e.to_string().contains("already claimed") => {
                            return Err(e)
                        }
                        // Someone else got it first.
                        Err(BotError::Rest { .. }) => continue,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        if granted {
            return Err(BotError::Failed(format!("no free lot in {} for the road in's deed", zones.join(", "))));
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

/// Which of `stations` (each by type) a citizen at `at` takes down as it
/// packs up (#488): those not on a lot of its own (`own_lot`) and within
/// [`PACK_UP_REACH`], nearest first. Further off, it goes back for one a
/// day ([`strays`], #500). One at work stays where its job is; a job set
/// but stopped holds nothing there, and came down with it (#499).
fn leaving(
    stations: Vec<(String, StationInfo)>,
    (x, y): (f64, f64),
    own_lot: impl Fn(f64, f64) -> bool,
) -> Vec<(String, StationInfo)> {
    let away = |s: &StationInfo| (s.x - x).hypot(s.y - y);
    let mut leaving: Vec<_> = stations
        .into_iter()
        .filter(|(_, s)| idle(s) && !own_lot(s.x, s.y) && away(s) <= PACK_UP_REACH)
        .collect();
    leaving.sort_by(|a, b| away(&a.1).total_cmp(&away(&b.1)));
    leaving
}

/// Which of `stations` (each by type) off a lot of its own (`own_lot`) and
/// beyond [`PACK_UP_REACH`] of `at` a citizen goes back for (#500), nearest
/// first: those [`leaving`] leaves. One at work stays.
fn strays(
    stations: Vec<(String, StationInfo)>,
    (x, y): (f64, f64),
    own_lot: impl Fn(f64, f64) -> bool,
) -> Vec<(String, StationInfo)> {
    let away = |s: &StationInfo| (s.x - x).hypot(s.y - y);
    let mut far: Vec<_> = stations
        .into_iter()
        .filter(|(_, s)| idle(s) && !own_lot(s.x, s.y) && away(s) > PACK_UP_REACH)
        .collect();
    far.sort_by(|a, b| away(&a.1).total_cmp(&away(&b.1)));
    far
}

/// Whether a station may come down: no job on it, or one stopped (#499).
fn idle(s: &StationInfo) -> bool {
    use capitol_protocol::rest::AutomationState;
    s.automation.as_ref().is_none_or(|a| a.state == AutomationState::Stopped)
}

/// How far `r` lies outside the band `(inner, outer)`.
fn band_gap((inner, outer): (f64, f64), r: f64) -> f64 {
    (inner - r).max(r - outer).max(0.0)
}

/// `resource` and every other resource that yields the same item with the
/// same tool: a log from a tree or from old growth (#336).
fn kinds(catalog: &Catalog, resource: &str) -> Vec<String> {
    let Some(this) = catalog.resource_type(resource) else {
        return vec![resource.to_string()];
    };
    let mut kinds = vec![this.id.clone()];
    kinds.extend(
        catalog
            .resource_types()
            .iter()
            .filter(|r| r.id != this.id)
            .filter(|r| r.yields_item == this.yields_item && r.required_tool == this.required_tool)
            .map(|r| r.id.clone()),
    );
    kinds
}

/// The band an explore looks in, from `r_now`, among the `zones` a resource
/// grows in: the nearest of them short of the Wilderness, or with `wilds`
/// the Wilderness, if it grows there (#336). A resource that grows only in
/// the wilds is looked for there either way. `None` when it grows nowhere.
fn search_band(zones: &[Zone], r_now: f64, wilds: bool) -> Option<(f64, f64)> {
    let wild = zones.contains(&Zone::Wilderness).then(|| Zone::Wilderness.radii());
    let near = zones
        .iter()
        .filter(|z| **z != Zone::Wilderness)
        .map(|z| z.radii())
        .min_by(|a, b| band_gap(*a, r_now).total_cmp(&band_gap(*b, r_now)));
    match (near, wild) {
        (_, Some(wild)) if wilds => Some(wild),
        (Some(near), _) => Some(near),
        (None, wild) => wild,
    }
}

fn rules_spawn_radius() -> f64 {
    use capitol_protocol::rules::{SPAWN_X, SPAWN_Y};
    (SPAWN_X - Zone::CENTER_X).hypot(SPAWN_Y - Zone::CENTER_Y)
}

/// Where a station may go off any lot, round `(x, y)`, nearest first: rings
/// out to 210, leaving out those too close to a station it knows of. The
/// spawn crowds with benches, and two rings of 30 and 90 weren't enough for
/// Arrival's (#499).
fn off_lot_spots((x, y): (f64, f64), known: &[(f64, f64)]) -> Vec<(f64, f64)> {
    use capitol_protocol::rules::STATION_MIN_SPACING;
    let mut spots = ring(x, y, 30.0);
    for r in [90.0, 150.0, 210.0] {
        spots.extend(ring(x, y, r).into_iter().skip(1));
    }
    spots.retain(|&(sx, sy)| known.iter().all(|&(kx, ky)| (sx - kx).hypot(sy - ky) >= STATION_MIN_SPACING));
    spots
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

/// How many of its calling's stations for a lot a citizen has yet to place
/// (Callings D6), with `owned` the station types it owns: of the room its
/// lot was bought for ([`stint::room_for`](super::stint::room_for)), less
/// those of [`lot_for`](super::stint::lot_for) it has. Of the job of the
/// moment, nothing, so a switch of stint never changes it, and never packs
/// its lot up ([`SimBot::repack`]). A swarm bot's calling is its role, so
/// it's its role's stations, as before callings.
fn still_wanted(calling: &Calling, owned: impl Fn(&str) -> bool) -> i32 {
    let placed = super::stint::lot_for(calling).into_iter().filter(|kind| owned(kind)).count() as i32;
    (super::stint::room_for(calling) - placed).max(0)
}

/// What a repair of a `station` type at `condition` takes, as the server
/// will bill it or more (#478): the condition shown is the server's rounded
/// up, so a bill at the shown one can come out one under the server's
/// (`station::repair`), and a citizen holding just that would be refused
/// every round. Billed a point more worn, it never is.
fn repair_bill(catalog: &Catalog, station: &str, condition: i32) -> Vec<(String, i32)> {
    crate::station::repair::bill(catalog, catalog.recipes(), station, (condition - 1).max(0))
}

/// A lot as a round's holdings see it (Callings D6): the stations its room
/// is kept for ([`Holdings::lot_for`]) and how much of its `room` the
/// calling doesn't want ([`Holdings::plot_spare`]), with `owned` the
/// station types it owns.
///
/// A swarm bot (`one_job`) keeps its lot for nothing in particular, as
/// before callings (plan §5): it has no dabble whose station could take
/// its calling's room, and its role's ladder climbs through stations its
/// role doesn't work at. A plank bot's sawbench kit wants the kiln's
/// bricks and the forge's nails, and with its lot kept for the sawbench
/// alone, it never bought the lot to put the kiln on (#552).
fn lot_kept(
    calling: &Calling,
    one_job: bool,
    owned: impl Fn(&str) -> bool,
    room: Option<i32>,
) -> (Option<std::collections::HashSet<String>>, i32) {
    let kept = (!one_job).then(|| super::stint::lot_for(calling).into_iter().map(String::from).collect());
    let spare = room.map_or(0, |room| (room - still_wanted(calling, owned)).max(0));
    (kept, spare)
}

/// Where the stations a citizen knows of stand, each on its storey: those
/// the store has `seen`, and those of its `own` the store hasn't heard of
/// (one placed this round, say), each once (#549). Its own are in the store
/// too once seen, and counted twice they filled an urban lot's 4 indoors
/// with 2, since a place's room is counted by station (V3.1): no urban
/// builder got past its kiln and forge.
fn known_stations<'a>(seen: &'a HashMap<Uuid, StationInfo>, own: impl IntoIterator<Item = &'a StationInfo>) -> Vec<lot::Spot> {
    seen.values()
        .chain(own.into_iter().filter(|s| !seen.contains_key(&s.id)))
        .map(|s| ((s.x, s.y), s.floor))
        .collect()
}

/// Whether `node` is one [`SimBot::nearest`] may send a gatherer to: out in
/// the Wilderness for a claim from the wilds, and at `at_least` or better for
/// a claim at a grade (#504). A node whose quality the server didn't say is
/// taken on trust.
fn worth_a_pull(node: &ResourceNodeInfo, wild: bool, at_least: Option<i32>) -> bool {
    (!wild || Zone::from_position(node.x, node.y) == Zone::Wilderness)
        && at_least.is_none_or(|q| node.base_quality.is_none_or(|b| b >= q))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A claim at a grade passes over the nodes under it, wherever they are,
    /// and a claim from the wilds over the nodes nearer in (#504).
    #[test]
    fn a_claim_at_a_grade_passes_over_the_nodes_under_it() {
        let vein = |x: f64, quality: Option<i32>| ResourceNodeInfo {
            id: Uuid::from_u128(x as u128),
            resource_type: "iron_vein".into(),
            x,
            y: Zone::CENTER_Y,
            state: "available".into(),
            base_quality: quality,
        };
        let rural = vein(Zone::CENTER_X + 5000.0, Some(70));
        let wild = vein(Zone::CENTER_X + 7000.0, Some(70));
        // Any vein will do for a claim with no grade
        assert!(worth_a_pull(&rural, false, None));
        // B (75) isn't to be had at a 70 vein, wild or not
        assert!(!worth_a_pull(&rural, false, Some(75)));
        assert!(!worth_a_pull(&wild, true, Some(75)));
        assert!(worth_a_pull(&vein(Zone::CENTER_X + 7000.0, Some(80)), true, Some(75)));
        // C (60) is, but a claim from the wilds keeps to the wilds
        assert!(worth_a_pull(&rural, false, Some(60)));
        assert!(!worth_a_pull(&rural, true, Some(60)));
        // A server that doesn't say is taken on trust
        assert!(worth_a_pull(&vein(Zone::CENTER_X + 7000.0, None), true, Some(75)));
    }

    fn station(kind: &str, x: f64, y: f64) -> (String, StationInfo) {
        let info = StationInfo {
            id: Uuid::from_u128((x as u128) << 16 | y as u128),
            station_type: kind.into(),
            name: kind.into(),
            category: "crafting".into(),
            icon: None,
            x,
            y,
            owner_id: Uuid::nil(),
            container_id: None,
            interaction_range: 75.0,
            quality: 50,
            tool: None,
            automation: None,
            production: Vec::new(),
            indoors: false,
            condition: 100,
            floor: 0,
        };
        (kind.into(), info)
    }

    /// Packing up (#488) takes down what stands off its own lot and near,
    /// nearest first, and leaves its lot's, the far and one at work.
    #[test]
    fn packing_up_takes_down_the_near_benches_off_its_lot() {
        use capitol_protocol::rest::{AutomationMode, AutomationState, StationAutomation};
        let job = |kind: &str, x: f64, state: AutomationState| {
            let mut s = station(kind, x, 0.0);
            s.1.automation = Some(StationAutomation {
                recipe_id: "nails".into(),
                mode: AutomationMode::Forever,
                state,
                done: 0,
            });
            s
        };
        let stations = vec![
            station("workbench", 120.0, 0.0),
            station("kiln", 40.0, 0.0),
            station("loom", 500.0, 0.0),
            station("forge", 0.0, 60.0),
            job("anvil", 30.0, AutomationState::Running),
            // A job set and stopped holds nothing there (#499)
            job("sawbench", 80.0, AutomationState::Stopped),
        ];
        // Its lot is everything north of y = 50
        let leaving = leaving(stations, (0.0, 0.0), |_, y| y > 50.0);
        let kinds: Vec<&str> = leaving.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(kinds, ["kiln", "sawbench", "workbench"]);
    }

    /// Going back for its far stations (#500) takes what packing up leaves
    /// off its lot, nearest first: not its lot's, nor one at work.
    #[test]
    fn going_back_takes_the_far_ones_off_its_lot() {
        use capitol_protocol::rest::{AutomationMode, AutomationState, StationAutomation};
        let mut working = station("anvil", 900.0, 0.0);
        working.1.automation = Some(StationAutomation {
            recipe_id: "nails".into(),
            mode: AutomationMode::Forever,
            state: AutomationState::Running,
            done: 0,
        });
        let stations = vec![
            station("workbench", 5000.0, 0.0),
            station("workbench", 120.0, 0.0),
            station("kiln", 300.0, 0.0),
            station("loom", 400.0, 400.0),
            working,
        ];
        // Its lot is everything north of y = 50
        let far = strays(stations.clone(), (0.0, 0.0), |_, y| y > 50.0);
        let at: Vec<(&str, f64)> = far.iter().map(|(k, s)| (k.as_str(), s.x)).collect();
        assert_eq!(at, [("kiln", 300.0), ("workbench", 5000.0)]);
        // Packing up and going back never both take one
        let near = leaving(stations, (0.0, 0.0), |_, y| y > 50.0);
        assert!(near.iter().all(|(_, n)| far.iter().all(|(_, f)| f.id != n.id)));
        assert_eq!(near.len(), 1);
    }

    /// Holdings on a lot of `room` that owns `stations`, as a round sees
    /// them for `calling` ([`SimBot::holdings`]): rich, with a lot.
    fn on_lot(calling: &Calling, room: i32, stations: &[&str]) -> Holdings {
        let mut h = Holdings {
            strands: 10_000,
            plot_room: Some(room),
            stations: stations.iter().map(|s| s.to_string()).collect(),
            ..Holdings::default()
        };
        kept_again(calling, &mut h);
        h
    }

    /// What a round's holdings say of the lot, again ([`lot_kept`]).
    fn kept_again(calling: &Calling, h: &mut Holdings) {
        let owned = h.stations.clone();
        let (lot_for, spare) = lot_kept(calling, false, |k| owned.contains(k), h.plot_room);
        h.lot_for = lot_for;
        h.plot_spare = spare;
    }

    fn wanted(calling: &Calling, h: &Holdings) -> i32 {
        still_wanted(calling, |k| h.stations.contains(k))
    }

    /// An urban builder's kiln and forge, known to the store and as its
    /// own, are two stations, not four: its lot has room for the sawbench
    /// and the anvil (#549). Counted twice, the building was full.
    #[test]
    fn its_own_stations_count_once_on_its_lot() {
        let mut plot = lot::tests::home("urban", 90.0, 93.0, 4, 0);
        let spots = lot::spots(&plot, &[]);
        let (kiln, forge) = (station("kiln", spots[0].0, spots[0].1), station("forge", spots[1].0, spots[1].1));
        plot.station_count = 2;
        let seen: HashMap<Uuid, StationInfo> = [&kiln, &forge].map(|(_, s)| (s.id, s.clone())).into_iter().collect();
        let own: HashMap<String, StationInfo> = [kiln.clone(), forge.clone()].into_iter().collect();
        let known = known_stations(&seen, own.values());
        assert_eq!(known.len(), 2);
        let ground: Vec<_> = known.iter().map(|s| s.0).collect();
        assert_eq!(lot::room(&plot, &ground), 2);
        // One placed this round, not yet in the store, still counts
        let known = known_stations(&HashMap::new(), own.values());
        assert_eq!(known.len(), 2);
    }

    /// K1.3's Verify: a citizen's stints, played out on the planner against
    /// its lot. Whatever job it switches to, what its lot should hold stays
    /// the same, and no job's work leaves the lot short of room for its
    /// calling's stations, which is when it would pack its lot up
    /// ([`SimBot::repack`]). For every birth trade in every style, on lots
    /// of 1 to 6.
    #[test]
    fn a_switch_of_stint_never_changes_what_the_lot_should_hold() {
        use crate::citizen::calling::{Mix, Style};
        use crate::citizen::persona::Rng;
        use crate::sim::{plan, stint, MIX};
        let catalog = Catalog::fallback();
        let mut rng = Rng::new(532);
        let mut placed = 0;
        for style in Style::ALL {
            for role in MIX.map(|(r, _)| r) {
                let calling = Calling::seeded(&mut rng, &Mix::only(style), role);
                for room in [1, 2, 4, 6] {
                    let mut h = on_lot(&calling, room, &[]);
                    let short_at_first = room < wanted(&calling, &h);
                    for job in stint::menu_jobs(&calling, false) {
                        kept_again(&calling, &mut h);
                        let commitment = stint::commitment(&calling, job);
                        if job.take(&catalog, &h, commitment, 0).is_none() {
                            continue;
                        }
                        // A stint of it, up to the planks' 500 steps or so
                        for _ in 0..700 {
                            kept_again(&calling, &mut h);
                            let Ok(errands) = job.errands_at(&catalog, &h, commitment) else { break };
                            let Some(errand) = errands.into_iter().next() else { break };
                            let (still, room) = (wanted(&calling, &h), h.plot_room.unwrap_or(0));
                            plan::take(&catalog, &mut h, &errand.step);
                            if let Step::Place { station, on_plot: true, .. } = &errand.step {
                                placed += 1;
                                let after = (wanted(&calling, &h), h.plot_room.unwrap_or(0));
                                assert!(
                                    after.1 >= after.0 || short_at_first,
                                    "{calling} on a lot of {room}: {job}'s {station} left room {} for {} wanted (was {room} for {still})",
                                    after.1,
                                    after.0
                                );
                                // Only its calling's own stations change what's wanted
                                let kept = stint::lot_for(&calling).contains(&station.as_str());
                                assert_eq!(after.0, if kept { (still - 1).max(0) } else { still }, "{calling}: {job}'s {station}");
                            }
                        }
                    }
                }
            }
        }
        assert!(placed > 50, "the stints placed {placed} stations");
    }

    /// The #519 guard: a lot that can't take a job's stations leaves the
    /// job off the stint menu, unless it owns them already. An urban
    /// citizen on a lot of 1 never takes up building; the sawbench, which
    /// fits, it does once it has its kit.
    #[test]
    fn a_lot_of_one_never_takes_up_building_unless_it_owns_the_stations() {
        use crate::sim::{stint, Commitment, Craft, Gather, LOT_STATIONS};
        let catalog = Catalog::fallback();
        let builder = Calling::single(Role::Builder);
        let all_rounder = Calling {
            favourite: None,
            focus: 0.0,
            leanings: crate::sim::ROLES.into_iter().map(|r| (r, 0.125)).collect(),
            wander: 0.0,
        };
        for calling in [&builder, &all_rounder] {
            let h = on_lot(calling, 1, &["workbench"]);
            assert_eq!(h.plot_spare, 0, "its calling wants 4");
            for commitment in [Commitment::Full, Commitment::Dabble] {
                assert_eq!(Role::Builder.take(&catalog, &h, commitment, 0), None, "{calling}");
                // Nor for what pays for it
                assert_eq!(Role::Builder.take(&catalog, &h, commitment, 1_000), None, "{calling}");
            }
            let menu: Vec<Role> = stint::menu_jobs(calling, false)
                .into_iter()
                .filter(|&j| j.take(&catalog, &h, stint::commitment(calling, j), 0).is_some())
                .collect();
            assert!(!menu.contains(&Role::Builder), "{calling}: {menu:?}");
            if calling.favourite.is_none() {
                // The sawbench's kit wants the kiln's bricks and the forge's
                // nails, which don't fit either, so it gathers
                let gathers = [Role::Gatherer(Gather::Fiber), Role::Gatherer(Gather::Wood), Role::Gatherer(Gather::Ore)];
                assert_eq!(menu, gathers);
                // With a sawbench's kit bought in, the sawbench fits, and it saws
                let mut kit = h.clone();
                let sawbench = catalog.station_type("sawbench").and_then(|t| t.kit.clone()).expect("the sawbench's kit");
                kit.items.insert(sawbench, 1);
                kit.items.insert("log".into(), 10);
                let planks = Role::Crafter(Craft::Planks);
                assert!(planks.take(&catalog, &kit, stint::commitment(calling, planks), 0).is_some());
            }
            // Owning a builder's stations already (put down before the lot
            // was resized, say), it builds on: no room for its chest, so
            // bricks, with its axe
            let mut owns: Vec<&str> = LOT_STATIONS.to_vec();
            owns.push("workbench");
            let mut full = on_lot(calling, 0, &owns);
            let axe = catalog.item_type("stone_axe").and_then(|t| t.tool.clone()).expect("an axe");
            full.tools.insert(axe.tool_type);
            assert!(Role::Builder.take(&catalog, &full, stint::commitment(calling, Role::Builder), 0).is_some(), "{calling}");
        }
        // Before it has a lot, the one it would buy is its calling's
        let homeless = Holdings {
            strands: 10_000,
            lot_for: Some(Default::default()),
            ..Holdings::default()
        };
        assert!(!Role::Crafter(Craft::Planks).fits(&homeless), "a fiber gatherer's lot keeps no room");
        assert!(Role::Gatherer(Gather::Wood).fits(&homeless));
        let builds = Holdings {
            lot_for: Some(stint::lot_for(&builder).into_iter().map(String::from).collect()),
            ..homeless
        };
        assert!(Role::Builder.fits(&builds));
    }

    /// A dabble's station never takes the room its calling wants (K0.3's
    /// note): a smelter's builder stint doesn't put a chest where its
    /// forge is to go, but it may put one in room its calling doesn't want.
    #[test]
    fn a_dabbles_station_goes_only_into_room_the_calling_does_not_want() {
        use crate::sim::{plan, Craft};
        let catalog = Catalog::fallback();
        let smelter = Calling::single(Role::Crafter(Craft::Ingots));
        let chest = Goal::Station("storage_chest".into());
        let kit = catalog.station_type("storage_chest").and_then(|t| t.kit.clone()).expect("the chest has a kit");
        let mut h = on_lot(&smelter, 2, &["workbench"]);
        h.items.insert(kit.clone(), 1);
        assert!(plan::next(&catalog, &h, &chest).is_err(), "{:?}", plan::next(&catalog, &h, &chest));
        let mut roomy = on_lot(&smelter, 3, &["workbench"]);
        roomy.items.insert(kit, 1);
        assert_eq!(roomy.plot_spare, 1);
        assert!(matches!(plan::next(&catalog, &roomy, &chest), Ok(Some(Step::Place { on_plot: true, .. }))));
        // Its own forge goes into any of it
        let forge = Goal::Station("forge".into());
        let kit = catalog.station_type("forge").and_then(|t| t.kit.clone()).expect("the forge has a kit");
        h.items.insert(kit, 1);
        assert!(matches!(plan::next(&catalog, &h, &forge), Ok(Some(Step::Place { on_plot: true, .. }))));
    }

    /// The lot is bought for the calling: a swarm bot's is its role's, as
    /// before callings, and the stations nest within a builder's 4.
    #[test]
    fn the_lot_is_bought_for_the_calling_not_the_stint() {
        use crate::sim::{stint, Craft, Gather, ROLES};
        for role in ROLES {
            assert_eq!(stint::room_for(&Calling::single(role)), role.room_wanted(), "{role}");
            assert_eq!(stint::lot_for(&Calling::single(role)), role.lot_stations(), "{role}");
        }
        // Planks and tools want the sawbench beside the forge, kiln and anvil
        let mixed = Calling {
            favourite: Some(Role::Crafter(Craft::Tools)),
            focus: 0.7,
            leanings: vec![(Role::Crafter(Craft::Planks), 0.2), (Role::Gatherer(Gather::Fiber), 0.1)],
            wander: 0.0,
        };
        assert_eq!(stint::room_for(&mixed), 4);
        // A leaning under the willing share keeps no room
        let fiber = Calling {
            favourite: Some(Role::Gatherer(Gather::Fiber)),
            leanings: vec![(Role::Gatherer(Gather::Wood), 0.29), (Role::Builder, 0.01)],
            ..mixed
        };
        assert_eq!(stint::lot_for(&fiber), Vec::<&str>::new());
        assert_eq!(stint::room_for(&fiber), 1);
    }

    /// The swarm is one job (plan §5, #552): from nothing, every `MIX`
    /// role climbs its whole ladder on the planner, buying the lot for
    /// whatever station the climb wants first. A plank bot's sawbench kit
    /// wants the kiln's bricks, so its lot goes to the kiln too; kept for
    /// the sawbench alone, as a citizen's calling keeps it, it was stuck.
    #[test]
    fn a_swarm_bot_buys_its_lot_for_its_whole_ladder() {
        use crate::sim::{plan, Commitment, Craft, MIX};
        let catalog = Catalog::fallback();
        // Its role's work played out from nothing, the lot as a round sees
        // it after each step: what it put down, or where it was stuck
        let climb = |role: Role, one_job: bool| -> Result<(Holdings, Vec<String>), String> {
            let calling = Calling::single(role);
            let mut h = Holdings {
                strands: 10_000,
                ..Holdings::default()
            };
            let mut placed = Vec::new();
            for _ in 0..2000 {
                let owned = h.stations.clone();
                let (lot_for, spare) = lot_kept(&calling, one_job, |k| owned.contains(k), h.plot_room);
                h.lot_for = lot_for;
                h.plot_spare = spare;
                let errands = role
                    .errands_at(&catalog, &h, Commitment::Full)
                    .map_err(|stuck| format!("{role} stuck after placing {placed:?}: {}", stuck.0))?;
                let Some(errand) = errands.into_iter().next() else { break };
                if let Step::Place { station, .. } = &errand.step {
                    placed.push(station.clone());
                }
                plan::take(&catalog, &mut h, &errand.step);
            }
            Ok((h, placed))
        };
        let planks = Role::Crafter(Craft::Planks);
        let stuck = climb(planks, false).err();
        assert!(
            stuck.as_deref().is_some_and(|s| s.contains("not a kiln")),
            "kept for its sawbench alone: {stuck:?}"
        );
        for role in MIX.map(|(r, _)| r) {
            let (h, placed) = climb(role, true).unwrap_or_else(|stuck| panic!("{stuck}"));
            for station in role.lot_stations() {
                assert!(h.stations.contains(*station), "{role} never put down its {station}: {placed:?}");
            }
            if role == planks {
                assert!(placed.iter().any(|s| s == "kiln"), "{placed:?}");
            }
        }
    }

    #[test]
    fn a_band_gap_is_zero_inside_and_the_distance_outside() {
        assert_eq!(band_gap((3000.0, 4600.0), 3350.0), 0.0);
        assert_eq!(band_gap((1800.0, 3000.0), 3350.0), 350.0);
        assert_eq!(band_gap((3000.0, 4600.0), 2500.0), 500.0);
    }

    #[test]
    fn a_log_comes_from_a_tree_or_old_growth_and_ore_from_its_own_vein() {
        let catalog = Catalog::fallback();
        assert_eq!(kinds(&catalog, "tree"), ["tree", "old_growth"]);
        assert_eq!(kinds(&catalog, "old_growth"), ["old_growth", "tree"]);
        assert_eq!(kinds(&catalog, "iron_vein"), ["iron_vein"]);
        assert_eq!(kinds(&catalog, "no_such"), ["no_such"]);
    }

    #[test]
    fn an_explore_goes_to_the_wilds_when_asked_or_when_nothing_else_grows_it() {
        let (rural, wilderness) = (Zone::Rural.radii(), Zone::Wilderness.radii());
        let ore = [Zone::Rural, Zone::Wilderness];
        let r_home = (rural.0 + rural.1) / 2.0;
        assert_eq!(search_band(&ore, r_home, false), Some(rural));
        assert_eq!(search_band(&ore, r_home, true), Some(wilderness));
        // Copper and tin grow only in the wilds
        assert_eq!(search_band(&[Zone::Wilderness], r_home, false), Some(wilderness));
        // Grass doesn't grow there at all
        assert_eq!(search_band(&[Zone::Rural], r_home, true), Some(rural));
        assert_eq!(search_band(&[], r_home, true), None);
    }

    #[test]
    fn about_a_third_of_citizens_venture_out_first() {
        let venturers = (0..1000u64)
            .filter(|&seed| {
                let persona = Persona::seeded(Role::Gatherer(super::super::Gather::Ore), seed);
                persona.temperament.risk >= VENTURE_RISK
            })
            .count();
        assert!((250..=420).contains(&venturers), "{venturers} of 1000 venture out first");
    }

    /// Off a lot (#499): four rings, nearest first, and none by a station.
    #[test]
    fn off_lot_spots_reach_further_and_keep_clear_of_stations() {
        use capitol_protocol::rules::STATION_MIN_SPACING;
        let all = off_lot_spots((0.0, 0.0), &[]);
        assert_eq!(all.len(), 25);
        assert_eq!(all[0], (0.0, 0.0));
        let r = |&(x, y): &(f64, f64)| x.hypot(y);
        assert!(all.windows(2).all(|w| r(&w[0]) <= r(&w[1]) + 1e-9));
        assert!((r(all.last().unwrap()) - 210.0).abs() < 1e-9);
        let known = [(0.0, 0.0), (90.0, 0.0)];
        let clear = off_lot_spots((0.0, 0.0), &known);
        // The centre's station takes the whole ring of 30, the other its spot
        assert_eq!(clear.len(), 17);
        assert!(clear.iter().all(|s| known.iter().all(|k| (s.0 - k.0).hypot(s.1 - k.1) >= STATION_MIN_SPACING)));
    }

    #[test]
    fn a_ring_is_seven_spots_apart_enough_for_stations() {
        let spots = ring(100.0, 100.0, lot::SPOT_SPACING);
        assert_eq!(spots.len(), 7);
        for (i, a) in spots.iter().enumerate() {
            for b in &spots[i + 1..] {
                assert!((a.0 - b.0).hypot(a.1 - b.1) >= capitol_protocol::rules::STATION_MIN_SPACING);
            }
        }
    }
}
