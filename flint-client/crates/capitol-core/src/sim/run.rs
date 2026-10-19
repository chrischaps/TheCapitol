//! The swarm: one thread a bot, each signing in, waiting for the others, then
//! playing its [`Role`] until the game hours are up. The market makers run
//! alongside, one thread each, from a grant.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use capitol_protocol::rest::{PlaceOrder, ResourceNodeInfo, StationInfo};
use capitol_protocol::rules::market::{OrderSide, OrderStatus};
use capitol_protocol::rules::{self, Zone};
use uuid::Uuid;

use super::plan::{self, Holdings, Step};
use super::report::{BotRun, Entry, Report, MAKER_ROLE};
use super::{Role, Trade, MARKET_ITEMS};
use crate::bot::{Account, Bot, BotError};
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
                        Ok(bot) => SimBot::new(bot, role, i as u64, scale, started, trading).play(config.hours),
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

/// A maker's run: quote until `deadline`, take the orders down, and count
/// what filled.
fn make_market(mut bot: Bot, mut maker: Maker, deadline: Instant, started: Instant, scale: f64) -> BotRun {
    bot.record();
    let strands_start = bot.state().strands().unwrap_or(0);
    let result = maker.run(&mut bot, Some(deadline)).and_then(|()| maker.withdraw(&mut bot));
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
}

impl SimBot {
    fn new(mut bot: Bot, role: Role, n: u64, scale: f64, start: Instant, trading: bool) -> SimBot {
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
        let mut failures = 0;
        let mut error = None;
        while Instant::now() < deadline {
            let catalog = self.bot.state().catalog().clone();
            let holdings = self.holdings();
            let step = if self.role.banks(&holdings) {
                Ok(Some(Step::Deposit))
            } else {
                plan::next(&catalog, &holdings, &self.role.goal(&catalog, &holdings))
            };
            let trade = match &step {
                Ok(Some(step)) if self.trading => self.role.trade(&catalog, &holdings, step, |item, side| {
                    let book = self.bot.state().market().book(item)?;
                    match side {
                        OrderSide::Buy => book.best_bid(),
                        OrderSide::Sell => book.best_ask(),
                    }
                }),
                _ => None,
            };
            match step {
                Ok(Some(step)) => {
                    let label = trade.as_ref().map_or_else(|| step.label(&catalog), Trade::label);
                    let began = self.now();
                    let result = match &trade {
                        Some(trade) => self.trade(trade),
                        None => self.take(&step),
                    };
                    self.journal();
                    let ok = result.is_ok();
                    self.entries.push(Entry::Spent {
                        label: label.clone(),
                        hours: self.now() - began,
                        ok,
                    });
                    match result {
                        Ok(()) => failures = 0,
                        Err(BotError::AuthFailed(e)) => {
                            error = Some(format!("the server ended the session: {e}"));
                            break;
                        }
                        Err(e) => {
                            self.entries.push(Entry::Failed {
                                label: label.clone(),
                                why: e.to_string(),
                            });
                            eprintln!("{}: {label} failed: {e}", self.bot.name());
                            failures += 1;
                            if failures >= FAILURES_BEFORE_REST {
                                failures = 0;
                                self.rest();
                            }
                        }
                    }
                }
                Ok(None) => self.rest(),
                Err(stuck) => {
                    self.entries.push(Entry::Stuck { reason: stuck.0 });
                    self.rest();
                }
            }
        }
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

    /// Sits out a game minute.
    fn rest(&mut self) {
        let _ = self.bot.idle(self.wall(60.0));
    }

    /// Turns what the bot saw into entries.
    fn journal(&mut self) {
        let at = self.now();
        for change in self.bot.take_journal() {
            match change {
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
    /// [`FILL_WAIT`] for it to fill, takes down what's left, collects what it
    /// bought, and walks back out.
    ///
    /// The way in is the one bots arrive by: back to the spawn on the south
    /// arterial, then up it to the hall's door, so the canals are crossed on
    /// its bridges. A straight line from the fields would meet water (#211).
    fn trade(&mut self, trade: &Trade) -> Result<(), BotError> {
        self.bot.walk_to(rules::SPAWN_X, rules::SPAWN_Y)?;
        self.bot.go_to_exchange()?;
        let traded = self.trade_at_the_hall(trade);
        // Out the way it came, whatever happened inside
        let out = self.bot.walk_to(rules::SPAWN_X, rules::SPAWN_Y);
        traded.and(out)
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

    /// Buys the nearest affordable unclaimed plot.
    fn claim(&mut self) -> Result<(), BotError> {
        let (x, y) = self.bot.position();
        let strands = self.bot.state().strands().unwrap_or(0);
        for zone in PLOT_ZONES {
            let mut plots = self.bot.unclaimed_plots(zone)?;
            plots.retain(|p| p.deed_cost.is_some());
            if let Some(cost) = plots.iter().filter_map(|p| p.deed_cost).min() {
                self.deed_cost = cost as i64;
            }
            plots.retain(|p| p.deed_cost.is_some_and(|c| c as i64 <= strands));
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
