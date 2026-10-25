//! A town's worth of situations, made up (C3.3): every role's work, sell,
//! buy, bid, stall and free-time questions over a spread of holdings, books
//! and temperaments, and the hall's board's since S2.5: a commission for
//! what it makes, and a post for what it needs. Since S2.8, work offered
//! at its trade, at a spread of rates, and since S4.5 a guild for its trade
//! to join, some of them having invited it. Since Callings K1.1 each
//! citizen has a calling, every birth trade in every style, and is asked
//! what to work at next over its calling's jobs: at the start of a day,
//! with nothing done lately or at its shares, and pulled away mid-stint.
//! Since K1.5 a detour or two is on its menu as a stint starts or ends,
//! and on some of its free time's. A fresh live town asks only work and
//! bank questions (#294), so this is what a new layer is measured on until
//! the logs hold the rest.
//! `capitol-town corpus` writes it as a rules-only citizen's log.

use std::path::Path;
use std::sync::{Arc, Mutex};

use capitol_protocol::rules::court;
use capitol_protocol::rules::wages::Work;
use uuid::Uuid;

use std::collections::BTreeMap;

use super::log::{self, Fate, Log};
use super::{Cascade, Choice, GuildSeen, Held, Mind, Place, Question, Quote, Situation, Temperament, When, Who, Why};
use crate::catalog::Catalog;
use crate::citizen::calling::{self, Calling, Style};
use crate::citizen::detour::{Detour, DetourKind};
use crate::citizen::persona::Persona;
use crate::sim::plan::Holdings;
use crate::sim::wage::{self, Hire};
use crate::sim::{bid_menu, post_menu, sell_menu, stall_menu, stint, surplus, Commitment, Role, Wanted, LOT, ROLES};

/// The name the corpus's citizen logs under.
pub const CITIZEN: &str = "Percy Sawyer";

/// Every role's work, sell, buy, bid, stall and stint questions over a
/// spread of holdings, books, temperaments and callings: what a town asks
/// in a day.
pub fn situations() -> Vec<Situation> {
    round(0)
}

/// `rounds` of [`situations`], each with other temperaments, and its books
/// and stock at another scale: a bigger town, to measure a layer's
/// calibration on.
pub fn town(rounds: u32) -> Vec<Situation> {
    (0..rounds.max(1)).flat_map(round).collect()
}

/// Round 0 is [`situations`].
fn round(r: u32) -> Vec<Situation> {
    let scale = 1 + (r % 3) as i64;
    let catalog = Catalog::fallback();
    let mut out = Vec::new();
    let mut n = 0u32;
    for (k, role) in ROLES.into_iter().enumerate() {
        // Each birth trade in each style, round by round
        let style = Style::ALL[(k + r as usize) % Style::ALL.len()];
        for (strands, fiber, product, last) in [(0, 0, 0, None), (40, 120, 12, Some(3)), (300, 260, 45, Some(9)), (900, 30, 80, Some(1))] {
            n += 1;
            let calling = Persona::seeded_in(role, u64::from(n) << 16 | u64::from(r), &Default::default(), &calling::Mix::only(style)).calling;
            let t = |k: u32| ((n * 37 + k * 11 + r * 53 + r * k * 7) % 10) as f32 / 9.0;
            let (product, last) = (product * scale as i32, last.map(|l: i64| l * scale));
            let temperament = Temperament {
                thrift: t(1),
                risk: t(2),
                sociability: t(3),
                patience: t(4),
            };
            let mut h = Holdings {
                strands,
                deed_cost: 250,
                ..Holdings::default()
            };
            h.items.insert("fiber".into(), fiber);
            if let Some(p) = role.product() {
                h.items.insert(p.into(), product);
            }
            let quote = |item: &str| Quote {
                item: item.into(),
                bid: last.map(|l: i64| l - 1),
                ask: last.map(|l: i64| l + 2),
                last,
            };
            let market: Vec<Quote> = ["log", "plank", "iron_ore", "iron_ingot", "brick", "stone"].iter().map(|i| quote(i)).collect();
            let s = |question, menu: Vec<Choice>, carry_on| Situation {
                who: Who {
                    name: format!("Citizen {n}"),
                    role: role.slug().into(),
                    temperament,
                    calling: Some(calling.clone()),
                },
                at: Place {
                    x: 6000.0 + n as f64 * 13.7,
                    y: 6000.0 - n as f64 * 3.1,
                    zone: "urban".into(),
                },
                when: When {
                    time_of_day: Some(0.3 + n as f64 * 0.01),
                    hours: n as f64 * 0.37,
                },
                holdings: (&h).into(),
                market: market.clone(),
                question,
                menu,
                carry_on: Some(carry_on),
            };
            out.push(stint(&calling, role, n, &h, &catalog, s));
            let Ok(errands) = role.errands(&catalog, &h) else { continue };
            out.push(s(Question::Work, errands.iter().map(|e| e.choice(&catalog)).collect(), 0));
            let mut free = vec![Choice::Rest, Choice::Hall];
            if n % 2 == 1 {
                free.extend(detours(n));
            }
            out.push(s(Question::FreeTime, free, 0));
            // What it has spare at its birth trade, across its calling's
            // jobs (Callings K1.4)
            let goal = errands.first().and_then(|e| e.goal.as_ref());
            let willing = stint::willing(&calling, role, false);
            let spare = surplus::spare(&h, role, surplus::selling(role, &h, goal), &willing);
            if let Some(&(item, n)) = spare.first() {
                if let Some(menu) = sell_menu(&h, item, n, last.map(|l| l - 1), last.map(|l| l + 2)) {
                    out.push(s(Question::Sell { item: item.into() }, menu, 1));
                }
                if let Some(menu) = stall_menu(&h, item, n, last, false) {
                    out.push(s(Question::Sell { item: item.into() }, menu, 1));
                }
            }
            for e in &errands {
                if let Some(menu) = role.buy_menu(&h, &e.step, last.map(|l| l + 2)) {
                    let item = menu.iter().find_map(|c| match c {
                        Choice::Buy { item, .. } => Some(item.clone()),
                        _ => None,
                    });
                    out.push(s(Question::Buy { item: item.unwrap_or_default() }, menu, 1));
                }
            }
            let wants = surplus::wants(&calling, role);
            for &item in &wants {
                if let Some(menu) = bid_menu(&h, item, &wants, last, false) {
                    out.push(s(Question::Buy { item: item.to_string() }, menu, 1));
                }
                if let Some(menu) = post_menu(&h, item, &wants, last.unwrap_or(4), 0) {
                    out.push(s(Question::Buy { item: item.to_string() }, menu, 1));
                }
            }
            // A commission for what it makes, at a price round its books,
            // and every third citizen with a claim on it already
            for (k, item) in role.answers().iter().enumerate() {
                let wanted = Wanted {
                    item,
                    unclaimed: 12 * scale as i32,
                    owed: if n.is_multiple_of(3) { 6 } else { 0 },
                    meeting: h.count(item),
                    day_work: LOT,
                    can_make: true,
                    due_soon: false,
                };
                let Some(menu) = role.commission_menu(&wanted) else { continue };
                let question = Question::Commission {
                    id: Uuid::from_u128(u128::from(n) << 8 | k as u128),
                    item: item.to_string(),
                    grade: "F".into(),
                    wanted: court::deliverable(wanted.unclaimed, wanted.owed),
                    claimed: wanted.owed,
                    price: last.map_or(5, |l| l - 1 + (n % 4) as i64),
                    due: 30.0,
                };
                let pass = menu.len() - 1;
                out.push(s(question, menu, pass));
            }
            // Work offered at its trade (S2.8), at a rate round its books
            if let Some(work) = own_work(&catalog, role) {
                let job = wage::job(&catalog, &work).expect("a job the catalog knows");
                let menu = role.wage_menu(&Hire { job, left: 40, tooled: true }).expect("its own trade's work");
                let rate = last.map_or(2, |l| (l * (1 + (n % 3) as i64) / 2).max(1));
                let books = |item: &str| super::rules::worth_in(&market, item);
                let question = Question::Wage {
                    id: Uuid::from_u128(u128::from(n) << 8 | 0xff),
                    employer: "Ada Reeder".into(),
                    work: wage::words(&catalog, &work),
                    rate,
                    left: 40,
                    ends: 24.0,
                    pays: wage::pays(&catalog, &work, rate).unwrap_or(0),
                    earns: wage::earns(role, &catalog, books),
                };
                out.push(s(question, menu, 1));
            }
            // Hiring (Callings K2.4): its lot's grass, and planks sawn at a
            // bench of its from a dozen logs, at its line by its thrift
            let mut stocked = h.clone();
            stocked.strands += 400 * scale;
            stocked.items.insert("log".into(), 12);
            let openings = [
                wage::Opening {
                    work: Work::Gather {
                        plot_id: Uuid::nil(),
                        resource: "grass".into(),
                    },
                    most: wage::GATHER_CAP,
                    stock: Vec::new(),
                },
                wage::Opening {
                    work: Work::Craft {
                        station_id: Uuid::nil(),
                        recipe_id: "plank".into(),
                    },
                    most: 36,
                    stock: vec![("log".into(), 12)],
                },
            ];
            let line = wage::hire_line(wage::HIRE_LINE, temperament.thrift);
            for opening in &openings {
                let books = |item: &str| super::rules::worth_in(&market, item);
                if let Some((question, menu)) = wage::hire_menu(opening, &catalog, &stocked, line, books) {
                    let mut situation = s(question, menu, 1);
                    situation.holdings = (&stocked).into();
                    out.push(situation);
                }
            }
            // A guild for its trade (S4.5): some invited it, and some have
            // a rival for its trade too
            let word = role.trade_words()[0];
            let mut guilds = vec![GuildSeen {
                name: format!("The {} Guild", capitalised(word)),
                tag: word.chars().take(4).collect::<String>().to_uppercase(),
                members: 1 + n % 6,
                charter: format!("For everyone in the {word} trade."),
                invited: n.is_multiple_of(3),
                trade: true,
            }];
            if n % 4 == 1 {
                guilds.push(GuildSeen {
                    name: format!("{}s Together", capitalised(word)),
                    tag: "TOG".into(),
                    members: 3,
                    charter: String::new(),
                    invited: false,
                    trade: true,
                });
            }
            let mut menu: Vec<Choice> = guilds.iter().map(|g| Choice::Apply { guild: g.name.clone() }).collect();
            menu.push(Choice::Stay);
            let stay = menu.len() - 1;
            out.push(s(Question::Guild { guilds }, menu, stay));
        }
    }
    out
}

/// What `n`, a citizen of `calling` born to `role`, works at next (K1.1),
/// over its calling's jobs as it holds `h`: its favourite as a whole
/// ladder, the rest as a dabble, one now and then pulled on. By turns it's
/// starting its day with nothing done lately, done with a stint of its
/// birth trade, at its calling's shares, or pulled away from its favourite
/// with too little of it done. `s` makes the situation.
fn stint(
    calling: &Calling,
    role: Role,
    n: u32,
    h: &Holdings,
    catalog: &Catalog,
    s: impl Fn(Question, Vec<Choice>, usize) -> Situation,
) -> Situation {
    let jobs: Vec<Role> = ROLES.into_iter().filter(|&job| calling.share(job) > 0.0).collect();
    let menu: Vec<Choice> = jobs
        .iter()
        .enumerate()
        .filter_map(|(j, job)| {
            let commitment = if calling.favourite == Some(*job) { Commitment::Full } else { Commitment::Dabble };
            let pays = if (n as usize + j).is_multiple_of(7) { 40 + 20 * (n % 5) as i64 } else { 0 };
            job.take(catalog, h, commitment, pays)
        })
        .collect();
    let favourite = calling.favourite.unwrap_or(role);
    let (why, doing, run, lately): (Why, Option<Role>, f32, BTreeMap<String, f32>) = match n % 4 {
        0 => (Why::Start, None, 0.0, BTreeMap::new()),
        1 => (Why::Done, Some(role), 1.0 + (n % 3) as f32, BTreeMap::from([(role.slug().to_string(), 1.0)])),
        2 => {
            let at = jobs.iter().map(|&job| (job.slug().to_string(), calling.share(job))).collect();
            (Why::Done, Some(favourite), 2.0, at)
        }
        _ => {
            let short = jobs
                .iter()
                .map(|&job| {
                    let share = calling.share(job);
                    (job.slug().to_string(), if job == favourite { share * 0.5 } else { share * 1.3 })
                })
                .collect();
            (Why::Pull, Some(favourite), 0.2 * (n % 4) as f32, short)
        }
    };
    let mut menu = menu;
    // As a stint starts or ends, a wander off may come first (K1.5)
    if why != Why::Pull && !menu.is_empty() {
        menu.extend(detours(n));
    }
    let carry_on = doing.and_then(|d| menu.iter().position(|c| matches!(c, Choice::Take { job, .. } if job == d.slug())));
    let question = Question::Stint {
        why,
        doing: doing.map(|d| d.slug().to_string()),
        run,
        lately,
    };
    let mut situation = s(question, menu, 0);
    situation.carry_on = carry_on;
    situation
}

/// The detours open to the `n`th citizen (K1.5): the lane and the paper,
/// and by turns a lake to see, a recipe to try and a neighbour to call on.
fn detours(n: u32) -> Vec<Choice> {
    let mut out = vec![
        Detour {
            kind: DetourKind::Browse,
            to: "lane".into(),
            does: "browse the bazaar lane".into(),
        },
        Detour {
            kind: DetourKind::Read,
            to: "gazette".into(),
            does: "read the Gazette at the hall".into(),
        },
    ];
    match n % 3 {
        0 => out.push(Detour {
            kind: DetourKind::See,
            to: format!("wild_spur_{n}"),
            does: "see a lake in the hills to the north-east".into(),
        }),
        1 => out.push(Detour {
            kind: DetourKind::Try,
            to: "rope_basic".into(),
            does: "try making rope".into(),
        }),
        _ => out.push(Detour {
            kind: DetourKind::Call,
            to: format!("sub_1_0_{n}_L_2"),
            does: "call on a neighbour".into(),
        }),
    }
    out.iter().map(Detour::choice).collect()
}

/// `word` with a capital: `Ore`.
fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// Work at `role`'s own trade, as an employer would offer it: a craft at
/// the first station it works at, or a gather of its good. `None` for a
/// role that does neither.
fn own_work(catalog: &Catalog, role: Role) -> Option<Work> {
    let item = *role.answers().first()?;
    if let Some(&station) = role.crafts_at().first() {
        let recipe = catalog.recipes().iter().find(|r| r.required_station_type.as_deref() == Some(station))?;
        return Some(Work::Craft {
            station_id: Uuid::nil(),
            recipe_id: recipe.id.clone(),
        });
    }
    let resource = catalog.resource_types().iter().find(|r| r.yields_item == item)?;
    Some(Work::Gather {
        plot_id: Uuid::nil(),
        resource: resource.id.clone(),
    })
}

/// Writes `situations` to `path` as a rules-only citizen's log, each
/// decision done.
pub fn record(path: &Path, situations: &[Situation]) -> std::io::Result<()> {
    let log = Log::open(path, CITIZEN, Role::Builder.slug(), None, &["rules".into()], log::LOG_LIMIT, log::LOG_KEEP)?;
    let mut mind = Mind::new(Cascade::rules(), Temperament::default()).with_log(Arc::new(Mutex::new(log)));
    for s in situations {
        let mark = mind.mark();
        mind.ask(s);
        mind.settle(mark, &Held::default(), |_| Fate::Done);
    }
    Ok(())
}
