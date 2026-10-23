//! The road in (#450, `the-capitol-road-in.md` D3): citizens walk the
//! tutorial too, and earn the same deed. An earned deed isn't a grant, so
//! it keeps the citizens' rule (they earn everything, Citizens D2).
//!
//! The road is a task in the citizen's day like any other, decided through
//! the cascade: each round, while a step is open to it, it's asked
//! [`Question::Road`] with the step ([`Choice::Road`]) beside what it was
//! doing, and the rules take the step ([`rules`](crate::citizen::decide::rules)).
//! Each round takes one bit of the step (a pull, a craft, a walk, a look),
//! as an errand does, so it still answers a trade, keeps its market hours
//! and goes home at dusk, and a road walked across its sessions (#276)
//! picks up where it left off: the server keeps where it is.
//!
//! The steps come from `protocol::tutorial` in the tour's order, and the
//! server judges them as it judges anyone's (#439): the citizen does what a
//! player would, by the same routes. The few only a screen shows (the pack,
//! the zone card, the office, the tax) it reports with `tutorial_seen`, as
//! the client does, and the market box it waives only when nothing has
//! filled (#446). The card it opens is someone's it can see, or a name in
//! the Gazette (#447): that step waits for a person (D10).
//!
//! The deed: at the Deeds Office it claims with the grant in its home zone
//! ([`SimBot::claim_deed`]), the lot its trade's claim would have taken, so
//! it never saves for a first deed ([`Holdings::road_deed`]). A citizen that
//! already owns a home claims a second, as a person who owns one does.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use capitol_protocol::rest::{AutomationMode, FreeLotsQuery, LetterBox, LettersQuery, PlaceOrder, PositionQuery};
use capitol_protocol::rules::market::OrderSide;
use capitol_protocol::rules::{talk, Zone, SPAWN_X, SPAWN_Y};
use capitol_protocol::tutorial::{Chapter, Grant, Judge, Step as Lesson, Steps, TutorialProgress};
use capitol_protocol::ws::{Channel, ClientMessage};

use super::super::plan::{self, Goal, Holdings, Step};
use super::super::{Errand, Role};
use super::{SimBot, PLOT_ZONES};
use crate::bot::{Bot, BotError};
use crate::citizen::decide::{Choice, Mind, Question};
use crate::citizen::meet;
use crate::citizen::persona::{Persona, HOME_ZONES};
use crate::citizen::phrases::{Occasion, Voice};
use crate::net::{RestRequest, RestResponse};
use crate::exchange::FIBER;

/// How long a step that couldn't be taken waits before it's tried again, in
/// game seconds: a letter not come yet, nobody about whose card to read.
const BLOCKED_FOR: f64 = 120.0;
/// How long the server has to say a step it was told of counted.
const HEARD_WITHIN: Duration = Duration::from_secs(5);
/// How long to wait for the server to judge a step the citizen just took.
const JUDGED_WITHIN: Duration = Duration::from_secs(2);
/// How far the first walk goes, up the road toward the hall.
const FIRST_WALK: f64 = 40.0;
/// How near the spawn a citizen sets its first bench up, on Arrival's
/// ground, which is clear of every lot (#443).
const SPAWN_GROUND: f64 = 150.0;
/// The job Settle in teaches (#449): knapped stone, for ever, at a
/// workbench. Rope is a hand recipe, which no station can be set to.
const SETTLE_IN_JOB: &str = "knapped_stone";
/// What a citizen says in local talk when nobody near has a name to greet.
const LINES: [&str; 4] = [
    "Good day to you.",
    "Fine morning for the road in.",
    "Busy at the hall today.",
    "Long walk up from the farms, that.",
];

/// A citizen's walk on the road in: the steps it couldn't take, and when
/// each may be tried again.
#[derive(Debug, Default)]
pub(super) struct Road {
    blocked: HashMap<Lesson, Instant>,
}

/// What taking `step` is, in a few words: the health line's, the log's and
/// a model's.
pub fn does(step: Lesson) -> &'static str {
    use Lesson::*;
    match step {
        Walk => "walk a little way up the road",
        GatherGrass => "pull grass by hand",
        OpenPack => "look in its pack",
        TwistRope => "twist a rope",
        MakeWorkbench => "make a workbench kit",
        PlaceWorkbench => "set the workbench up off any lot",
        GatherBranches => "pick up fallen branches",
        GatherStone => "gather stone by hand",
        MakeHandle => "make a handle at the bench",
        KnapStone => "knap stone at the bench",
        MakeAxe => "make a stone axe",
        FellTree => "fell a tree",
        SeeRuralLot => "look at a farm by the road",
        SeeSuburbanLot => "look at a suburban lot by the road",
        SeeUrbanLot => "look at an urban yard by the road",
        ReadZoneCard => "read the zone card",
        Deposit => "change fiber for Strands at the Exchange",
        SellOrder => "leave a sell order on the book",
        BuyOrder => "leave a buy order on the book",
        CollectBox => "see to the market box",
        VisitStall => "look in at a bazaar stall",
        ReadGazette => "read the Gazette",
        SayLocal => "say a word to whoever's near",
        OpenCard => "read someone's card",
        ReadLetter => "read the Registrar's letter",
        OpenOffice => "look in at the Deeds Office",
        ClaimHome => "claim a home with its deed",
        PlaceAtHome => "set a station up at home",
        SetAutomation => "give a station a job",
        ReadTax => "read what tax asks",
    }
}

/// Whether `step` can be taken now, on a road with `done` behind it: its
/// chapter has come (the road is walked in order, as the card shows it),
/// and what it needs is there.
fn ready(step: Lesson, progress: &TutorialProgress, role: Role, home: Option<&HomeFacts>) -> bool {
    let done = progress.done;
    if done.has(step) || !done.chapter_open(step.chapter()) {
        return false;
    }
    match step {
        // Both orders on the book first: the box is theirs (#446)
        Lesson::CollectBox => done.has(Lesson::SellOrder) && done.has(Lesson::BuyOrder),
        // The Registrar writes as People opens (#447), which chapter_open says
        Lesson::ClaimHome => progress.grant == Grant::Open,
        // A trade that builds on its lot sets its own stations there
        Lesson::PlaceAtHome => home.is_some_and(|h| h.room > 0) && !builds_at_home(role),
        Lesson::SetAutomation => home.is_some_and(|h| h.crafting_station),
        _ => true,
    }
}

/// Whether `step` can be taken in the night's work, at home (#276): what
/// only a screen shows, but the office, which is by the hall. The rest
/// waits for the day, though a citizen's own trade often does its Arrival
/// steps at night anyway (a pull, an axe), and the server counts them.
fn at_home_tonight(step: Lesson) -> bool {
    step.judge() == Judge::Client && step != Lesson::OpenOffice
}

/// A trade whose work puts stations on its lot: its first one there is its
/// station at home, and a workbench beside them would take their room.
fn builds_at_home(role: Role) -> bool {
    matches!(role, Role::Crafter(_) | Role::Builder)
}

/// What [`ready`] needs to know of a citizen's home.
struct HomeFacts {
    /// More stations its lot takes
    room: i32,
    /// It owns a crafting station, which takes a job
    crafting_station: bool,
}

impl SimBot {
    /// Where it is on the road in, if it walks it and the server has said.
    pub(super) fn road_progress(&self) -> Option<TutorialProgress> {
        self.road.as_ref()?;
        self.bot.state().tutorial()
    }

    /// Whether its first plot waits on the road's deed: it walks the road
    /// and hasn't spent the deed (#450). It doesn't buy a home meanwhile.
    pub(super) fn awaits_road_deed(&self) -> bool {
        self.road.is_some() && self.bot.state().tutorial().is_none_or(|t| t.grant != Grant::Spent)
    }

    /// The deed, as the planner sees it ([`Holdings::road_deed`]).
    pub(super) fn road_deed(&self) -> Option<Grant> {
        self.road.as_ref()?;
        Some(self.bot.state().tutorial().map_or(Grant::Unearned, |t| t.grant))
    }

    /// The next step on the road it can take now: the first not done, in
    /// the tour's order, that's [`ready`] and not waiting after a failure.
    pub(super) fn next_road_step(&self) -> Option<Lesson> {
        let road = self.road.as_ref()?;
        let progress = self.bot.state().tutorial()?;
        let now = Instant::now();
        let home = self.bot.state().plots().first().map(|_| HomeFacts {
            room: self.holdings().plot_room.unwrap_or(0),
            crafting_station: self.crafting_station().is_some(),
        });
        Lesson::ALL
            .into_iter()
            .filter(|s| road.blocked.get(s).is_none_or(|until| now >= *until))
            // In the night's work, from home (#276), only what takes no walk
            .filter(|&s| !self.night_shift || at_home_tonight(s))
            .find(|&s| ready(s, &progress, self.role, home.as_ref()))
    }

    /// Asks the mind whether to take the road's next step, or go on with
    /// `errand` (`None`: rest). `None`: not now, or nothing open on it.
    pub(super) fn choose_road(&mut self, holdings: &Holdings, errand: Option<&Errand>) -> Option<Lesson> {
        let step = self.next_road_step()?;
        let catalog = self.bot.state().catalog().clone();
        let menu = vec![
            errand.map_or(Choice::Rest, |e| e.choice(&catalog)),
            Choice::Road {
                step: step.name().into(),
                chapter: step.chapter().name().into(),
                does: does(step).into(),
            },
        ];
        // Carrying on is the road, once it's on it
        let carry_on = if self.on_road { 1 } else { 0 };
        let question = Question::Road { step: step.name().into() };
        let take = matches!(self.decide(question, menu, carry_on, holdings), Some((_, Choice::Road { .. })));
        self.on_road = take;
        take.then_some(step)
    }

    /// Takes a bit of `step`: the step itself, or one toward it (the fiber
    /// a rope needs, the walk to the hall). A step that couldn't be taken
    /// waits [`BLOCKED_FOR`] before it's tried again.
    pub(super) fn walk_road(&mut self, step: Lesson) -> Result<(), BotError> {
        let result = match self.road_step(step) {
            // It should count: a moment for the server to say so
            Ok(true) => {
                let judged = |s: &crate::state::GameState, _: &[_]| s.tutorial().is_some_and(|t| t.done.has(step));
                match self.bot.until("the road in's step", JUDGED_WITHIN, judged) {
                    Ok(_) | Err(BotError::Timeout { .. }) => Ok(()),
                    Err(e) => Err(e),
                }
            }
            Ok(false) => Ok(()),
            Err(e) => Err(e),
        };
        if let Err(e) = &result {
            if !matches!(e, BotError::Interrupted(_) | BotError::Offline(_)) && !e.is_final() {
                let until = Instant::now() + self.wall(BLOCKED_FOR);
                if let Some(road) = self.road.as_mut() {
                    road.blocked.insert(step, until);
                }
            }
        }
        result
    }

    /// One bit of `step`. `Ok(true)` when it's the step itself, which the
    /// server should count; `Ok(false)` when it was a step toward it.
    fn road_step(&mut self, step: Lesson) -> Result<bool, BotError> {
        use Lesson::*;
        match step {
            Walk => {
                // Up the road, toward the hall
                let (x, y) = self.bot.position();
                let (dx, dy) = (Zone::CENTER_X - x, Zone::CENTER_Y - y);
                let d = dx.hypot(dy).max(1.0);
                self.bot.walk_to(x + dx / d * FIRST_WALK, y + dy / d * FIRST_WALK)?;
                Ok(true)
            }
            GatherGrass => self.one_more(FIBER),
            GatherBranches => self.one_more("sticks"),
            GatherStone => self.one_more("stone"),
            FellTree => self.one_more("log"),
            TwistRope => self.make("rope_basic"),
            MakeWorkbench => self.make("craft_workbench"),
            MakeHandle => self.make("tool_handle"),
            KnapStone => self.make("knapped_stone"),
            MakeAxe => self.make("stone_axe"),
            // A bench of its own, off any lot (D10), whatever it has at home
            PlaceWorkbench => self.set_up_workbench(false),
            PlaceAtHome => self.set_up_workbench(true),
            // Arrival's bench comes down before the road, as the card asks
            // a player (#488): the kit comes back whole, for its lot
            SeeRuralLot => {
                if self.pack_up()? > 0 {
                    return Ok(false);
                }
                self.see_stop(Zone::Rural)
            }
            SeeSuburbanLot => self.see_stop(Zone::Suburban),
            SeeUrbanLot => self.see_stop(Zone::Urban),
            ReadZoneCard => {
                // What the card shows, read as a player's panel reads it
                self.bot.call("reading the zone card", RestRequest::ZoneFacts)?;
                self.report(ReadZoneCard)
            }
            Deposit => {
                if self.holdings().count(FIBER) == 0 {
                    return self.one_more(FIBER);
                }
                self.bot.go_to_exchange()?;
                self.take(&Step::Deposit)?;
                Ok(true)
            }
            SellOrder => self.sell_one(),
            BuyOrder => {
                self.bot.go_to_exchange()?;
                let price = meet::worth(self.bot.state(), FIBER).unwrap_or(1).max(1);
                self.bot.place_order(order(OrderSide::Buy, FIBER, price, None))?;
                Ok(true)
            }
            CollectBox => self.see_to_the_box(),
            VisitStall => self.look_in_at_a_stall(),
            ReadGazette => {
                self.bot.call("reading the Gazette", RestRequest::Gazette { day: None })?;
                Ok(true)
            }
            SayLocal => self.say_a_word(),
            OpenCard => self.read_a_card(),
            ReadLetter => self.read_the_letter(),
            OpenOffice => {
                // The Deeds Office is the desk by the hall (#448); what it
                // shows is its home zone's free lots
                self.bot.go_to_exchange()?;
                let zone = self.home_zone().as_str().to_string();
                self.bot.call("reading the free lots", RestRequest::FreeLots(FreeLotsQuery { zone }))?;
                if !self.bot.state().tutorial().is_some_and(|t| t.done.has(ReadZoneCard)) {
                    self.report(ReadZoneCard)?;
                }
                self.report(OpenOffice)
            }
            ClaimHome => {
                self.claim_deed()?;
                Ok(true)
            }
            SetAutomation => self.give_a_job(),
            // What only a screen shows (the pack, the tax), told as the
            // client tells it; and a step this build has no way to take
            step if step.judge().client_reports() => self.report(step),
            step => Err(BotError::Failed(format!("the road in's {} isn't one a citizen can take yet", step.name()))),
        }
    }

    /// Tells the server it saw `step` (`tutorial_seen`, #441), as the
    /// client does for what only a screen shows, and waits to hear it count.
    fn report(&mut self, step: Lesson) -> Result<bool, BotError> {
        self.bot.send(ClientMessage::TutorialSeen { step });
        let heard = |s: &crate::state::GameState, _: &[_]| s.tutorial().is_some_and(|t| t.done.has(step));
        match self.bot.until("the road in's step", HEARD_WITHIN, heard) {
            Ok(_) => Ok(false),
            Err(BotError::Timeout { .. }) => {
                Err(BotError::Failed(format!("the server didn't take {} as seen", step.name())))
            }
            Err(e) => Err(e),
        }
    }

    /// A step toward holding one more `item` than it does: a pull, or what
    /// the pull needs first (an axe for a log).
    fn one_more(&mut self, item: &str) -> Result<bool, BotError> {
        let catalog = self.bot.state().catalog().clone();
        let holdings = self.holdings();
        let goal = Goal::hold(item, holdings.count(item) + 1);
        let step = plan::next(&catalog, &holdings, &goal)
            .map_err(|stuck| BotError::Failed(stuck.0))?
            .ok_or_else(|| BotError::Failed(format!("holding one more {item} needs nothing")))?;
        let counts = matches!(&step, Step::Gather { item: got, .. } if got == item);
        self.take(&step)?;
        Ok(counts)
    }

    /// A step toward crafting `recipe` itself (not another that makes the
    /// same): its station, then its inputs, then the craft.
    fn make(&mut self, recipe: &str) -> Result<bool, BotError> {
        let catalog = self.bot.state().catalog().clone();
        let r = catalog
            .recipes()
            .iter()
            .find(|r| r.id == recipe)
            .cloned()
            .ok_or_else(|| BotError::Failed(format!("no recipe {recipe}")))?;
        let holdings = self.holdings();
        let station = r.required_station_type.clone().map(Goal::Station);
        let inputs = r.inputs.iter().map(|i| Goal::hold(&i.item_type, i.quantity));
        for goal in station.into_iter().chain(inputs) {
            if let Some(step) = plan::next(&catalog, &holdings, &goal).map_err(|stuck| BotError::Failed(stuck.0))? {
                self.take(&step)?;
                return Ok(false);
            }
        }
        self.take(&Step::Craft {
            recipe: r.id.clone(),
            station: r.required_station_type.clone(),
        })?;
        Ok(true)
    }

    /// A workbench kit, then the bench set up off any lot, by the spawn
    /// where the road begins, or on its own lot. At home the kit is the one
    /// it took down by the road, if it did ([`SimBot::pack_up`], #488), or
    /// a bench of its own near it off its lot comes down for it.
    fn set_up_workbench(&mut self, at_home: bool) -> Result<bool, BotError> {
        const KIT: &str = "workbench_kit";
        if self.holdings().count(KIT) == 0 {
            if at_home && self.pack_up()? > 0 {
                return Ok(false);
            }
            self.make("craft_workbench")?;
            return Ok(false);
        }
        if !at_home {
            // Arrival's ground by the spawn is clear of every lot (#443); a
            // citizen elsewhere, on its own lot say, goes back to it
            let (x, y) = self.bot.position();
            if (x - SPAWN_X).hypot(y - SPAWN_Y) > SPAWN_GROUND {
                self.bot.walk_to(SPAWN_X, SPAWN_Y)?;
                return Ok(false);
            }
        }
        self.place(KIT, "workbench", at_home)?;
        Ok(true)
    }

    /// Walks up the south arterial to `zone`'s stop (#445) and looks at its
    /// lot (`GET /plots/at`), the lot a player's ring is round.
    fn see_stop(&mut self, zone: Zone) -> Result<bool, BotError> {
        let walks = self.bot.walks()?;
        let stop = {
            let claimed = self.bot.state().claimed_lots();
            walks.road_stop(zone, |id| claimed.contains_key(id))
        }
        .ok_or_else(|| BotError::Failed(format!("the road in has no stop in the {zone} zone")))?;
        self.bot.walk_to(stop.road.0, stop.road.1)?;
        let _ = self.bot.face(stop.middle.0, stop.middle.1);
        let at = PositionQuery {
            x: stop.middle.0,
            y: stop.middle.1,
        };
        self.bot.call("looking at a lot", RestRequest::PlotAt(at))?;
        Ok(true)
    }

    /// At the hall, one unit of what it made on the road, a rope if it has
    /// one, on the book at about its worth.
    fn sell_one(&mut self) -> Result<bool, BotError> {
        self.bot.go_to_exchange()?;
        // A rope just twisted is in crafting's output, not the pack
        self.bot.collect_output()?;
        let catalog = self.bot.state().catalog().clone();
        let pick = {
            let inventory = self.bot.state().inventory();
            let goods: Vec<_> = inventory
                .map(|inv| inv.items().filter(|i| i.quantity > 0).cloned().collect())
                .unwrap_or_default();
            let not_a_tool = |t: &str| catalog.item_type(t).is_none_or(|t| t.tool.is_none());
            goods
                .iter()
                .find(|i| i.item_type == "rope")
                .or_else(|| goods.iter().filter(|i| not_a_tool(&i.item_type)).max_by_key(|i| i.quantity))
                .map(|i| (i.item_type.clone(), i.item_id))
        };
        let Some((item, item_id)) = pick else {
            return self.one_more(FIBER);
        };
        let worth = meet::worth(self.bot.state(), &item).or_else(|| catalog.target_price(&item)).unwrap_or(2);
        self.bot.place_order(order(OrderSide::Sell, &item, worth.max(1), Some(item_id)))?;
        Ok(true)
    }

    /// Collects from the market box if something filled, and otherwise
    /// says so (`tutorial_seen`), which the server takes only with the box
    /// empty (#446, D10).
    fn see_to_the_box(&mut self) -> Result<bool, BotError> {
        self.bot.go_to_exchange()?;
        // Matching runs each second: a moment for a fill
        self.bot.idle(super::FILL_WAIT)?;
        let collected = self.bot.collect_market_box()?;
        if collected.collected.is_empty() {
            self.report(Lesson::CollectBox)
        } else {
            Ok(true)
        }
    }

    /// Looks in at a stall someone else keeps, or a bare one if none is
    /// kept: a bare one counts as well (D10).
    fn look_in_at_a_stall(&mut self) -> Result<bool, BotError> {
        let me = self.bot.state().me();
        let directory = self.bot.bazaar()?;
        let stall = directory
            .stalls
            .iter()
            .find(|s| s.shop.as_ref().is_some_and(|shop| Some(shop.owner_id) != me))
            .or_else(|| directory.stalls.iter().find(|s| s.shop.is_none()))
            .map(|s| s.stall_id.clone())
            .ok_or_else(|| BotError::Failed("the bazaar has no stall to look in at".into()))?;
        self.bot.go_to_stall(&stall)?;
        match self.bot.call("looking in at a stall", RestRequest::Shop { stall_id: stall }) {
            Ok(_) => Ok(true),
            // Nobody keeps it: it looked all the same, and that counts
            Err(BotError::Rest { source, .. }) if source.status() == Some(404) => Ok(true),
            Err(e) => Err(e),
        }
    }

    /// A word in local talk: a greeting to whoever's nearest, in its voice,
    /// among people at the hall if nobody's near where it is.
    fn say_a_word(&mut self) -> Result<bool, BotError> {
        if self.nearest_person(Some(talk::HEARD)).is_none() {
            self.bot.go_to_exchange()?;
            self.bot.idle(Duration::from_millis(500))?;
        }
        let spoke = self
            .nearest_person(Some(talk::HEARD))
            .is_some_and(|(id, _)| self.bot.speak(Occasion::Greeting, id));
        if !spoke {
            let line = LINES[(self.random() * LINES.len() as f64) as usize % LINES.len()];
            self.bot.say(Channel::Local, line);
        }
        Ok(true)
    }

    /// Reads the card of whoever's nearest it can see, or of a name in the
    /// Gazette (#447). With nobody at all, the step waits for a person
    /// (D10, #484).
    fn read_a_card(&mut self) -> Result<bool, BotError> {
        if self.nearest_person(None).is_none() {
            self.bot.go_to_exchange()?;
            self.bot.idle(Duration::from_millis(500))?;
        }
        if let Some((player_id, _)) = self.nearest_person(None) {
            self.bot.call("reading a card", RestRequest::PlayerCard { player_id })?;
            return Ok(true);
        }
        let me = self.bot.name().to_string();
        let name = match self.bot.call("reading the Gazette", RestRequest::Gazette { day: None })? {
            RestResponse::Gazette(g) => g
                .edition
                .iter()
                .flat_map(|e| e.producers.iter())
                .filter_map(|p| p.name.clone())
                .find(|n| *n != me),
            _ => None,
        };
        let name = name.ok_or_else(|| BotError::Failed("nobody about, nor in the Gazette, whose card to read".into()))?;
        self.bot.call("reading a card", RestRequest::PlayerCardByName { name })?;
        Ok(true)
    }

    /// The nearest other player it can see, within `within` if given.
    fn nearest_person(&self, within: Option<f64>) -> Option<(uuid::Uuid, f64)> {
        let state = self.bot.state();
        let me = state.me()?;
        let (x, y) = self.bot.position();
        state
            .players()
            .values()
            .filter(|p| p.id != me)
            .map(|p| (p.id, (p.x - x).hypot(p.y - y)))
            .filter(|(_, d)| within.is_none_or(|w| *d <= w))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Reads the Registrar's letter (#447), or any unread one.
    fn read_the_letter(&mut self) -> Result<bool, BotError> {
        let query = LettersQuery {
            which: LetterBox::In,
            before: None,
        };
        let letters = match self.bot.call("reading the letters", RestRequest::Letters(query))? {
            RestResponse::Mailbox(mailbox) => mailbox.letters,
            _ => Vec::new(),
        };
        // An office's letter takes no reply, and a citizen sends none (#432)
        let letter = letters
            .iter()
            .find(|l| l.from.office)
            .or_else(|| letters.iter().find(|l| l.read_at.is_none()))
            .or(letters.first())
            .ok_or_else(|| BotError::Failed("no letter has come yet".into()))?;
        let letter_id = letter.id;
        self.bot.call("reading a letter", RestRequest::ReadLetter { letter_id })?;
        Ok(true)
    }

    /// A job for a crafting station of its own (#449): the one Settle in
    /// teaches, knapped stone for ever at a workbench ([`SETTLE_IN_JOB`]),
    /// or at another station the first thing made there with no tool. It's
    /// set, not started, so it doesn't eat the stone its trade gathers.
    fn give_a_job(&mut self) -> Result<bool, BotError> {
        let (station_id, recipe) = self
            .crafting_station()
            .ok_or_else(|| BotError::Failed("no station of its own to give a job".into()))?;
        self.bot.configure_automation(station_id, &recipe, AutomationMode::Forever)?;
        Ok(true)
    }

    /// A crafting station of its own, a workbench first, and a recipe of
    /// that station's own it can make with no tool: a hand recipe isn't a
    /// job (E4.2). The same one each time.
    fn crafting_station(&self) -> Option<(uuid::Uuid, String)> {
        let catalog = self.bot.state().catalog();
        let mut owned: Vec<_> = self.stations.values().filter(|s| s.category == "crafting").collect();
        owned.sort_by_key(|s| (s.station_type != "workbench", s.station_type.clone()));
        owned.into_iter().find_map(|s| {
            let here = |r: &&capitol_protocol::rest::RecipeInfo| {
                r.required_station_type.as_deref() == Some(s.station_type.as_str()) && r.tool_type.is_none()
            };
            let recipes = catalog.recipes();
            let recipe = recipes
                .iter()
                .filter(here)
                .find(|r| r.id == SETTLE_IN_JOB)
                .or_else(|| recipes.iter().find(here))?;
            Some((s.id, recipe.id.clone()))
        })
    }

    /// Its home zone: its persona's, else where it stands if that's one.
    fn home_zone(&self) -> Zone {
        match &self.persona {
            Some(p) => p.home.zone,
            None => {
                let (x, y) = self.bot.position();
                Some(Zone::from_position(x, y)).filter(|z| HOME_ZONES.contains(z)).unwrap_or(Zone::Rural)
            }
        }
    }

    /// Claims a lot with the deed the road in earned (#440, #448): in its
    /// home zone, near its spot there, with room for its trade's stations,
    /// the lot its trade's claim would have taken ([`SimBot::claim`]), or
    /// failing that in the other home zones. The deed is any lot's (D6), so
    /// it needn't afford one.
    pub(super) fn claim_deed(&mut self) -> Result<(), BotError> {
        let home = self.home_zone();
        let spot = self.persona.as_ref().map_or_else(|| self.bot.position(), |p| p.home.spot());
        let mut zones = vec![home.as_str()];
        zones.extend(PLOT_ZONES.iter().copied().filter(|z| *z != home.as_str()));
        zones.extend(HOME_ZONES.iter().map(|z| z.as_str()).filter(|z| !zones.contains(z)).collect::<Vec<_>>());
        self.claim_near(&zones, spot, self.role.room_wanted(), false, |_| true, true)
    }
}

/// A one-unit order on `item`'s book.
fn order(side: OrderSide, item: &str, price: i64, item_id: Option<uuid::Uuid>) -> PlaceOrder {
    PlaceOrder {
        side,
        item_type: item.to_string(),
        price,
        quantity: 1,
        item_id,
        qualities: None,
        min_grade: None,
        grade_rule: None,
    }
}

/// A road in walked by a bot of its own (`capitol-bot --tutorial`, plan
/// §10): `bot` takes each step as a citizen would, as `role` and the
/// persona `seed` gives it, until the road is walked or `deadline` passes.
/// It says each step as it's done. The progress it ended on.
pub fn walk_the_road(bot: Bot, role: Role, seed: u64, deadline: Option<Instant>) -> Result<TutorialProgress, BotError> {
    let mut bot = bot;
    let scale = bot.health()?.sim_tick_scale.unwrap_or(1.0);
    let persona = Persona::seeded(role, seed);
    bot.speak_with(Voice::phrasebook(&persona.temperament, seed));
    let name = bot.name().to_string();
    let mut me = SimBot::new(bot, role, seed, scale, Instant::now(), true, Mind::rules());
    me.persona = Some(persona);
    me.road = Some(Road::default());
    let mut said = Steps::NONE;
    loop {
        let Some(progress) = me.bot.state().tutorial() else {
            me.bot.idle(Duration::from_millis(500))?;
            continue;
        };
        for step in progress.done.iter().filter(|s| !said.has(*s)) {
            let n = progress.done.iter().count();
            println!("{name}: {} ({n} of {})", does(step), Lesson::ALL.len());
        }
        said = progress.done;
        let walked = progress.finished() || (progress.grant == Grant::Spent && progress.done.chapter_done(Chapter::DeedsOffice));
        if progress.finished() || deadline.is_some_and(|d| Instant::now() >= d) {
            return Ok(progress);
        }
        let holdings = me.holdings();
        match me.choose_road(&holdings, None) {
            Some(step) => match me.walk_road(step) {
                Ok(()) => {}
                Err(e) if e.is_final() => return Err(e),
                Err(BotError::Offline(_)) => me.bot.recover(deadline)?,
                Err(e) => println!("{name}: {}: {e}", does(step)),
            },
            // Waiting on something: a letter, a person, a station's room
            None if walked => return Ok(progress),
            None => {
                me.bot.idle(Duration::from_secs(1))?;
            }
        }
        me.bot.take_journal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Craft, Gather};

    fn road(done: impl IntoIterator<Item = Lesson>, grant: Grant) -> TutorialProgress {
        TutorialProgress {
            done: done.into_iter().collect(),
            grant,
            ..Default::default()
        }
    }

    fn chapters(chapters: &[Chapter]) -> Vec<Lesson> {
        chapters.iter().flat_map(|c| c.steps()).collect()
    }

    /// The first step `ready` takes, as `next_road_step` finds it
    fn first(progress: &TutorialProgress, role: Role, home: Option<&HomeFacts>) -> Option<Lesson> {
        Lesson::ALL.into_iter().find(|&s| ready(s, progress, role, home))
    }

    const GATHERER: Role = Role::Gatherer(Gather::Fiber);

    #[test]
    fn the_road_is_walked_in_the_tours_order_a_chapter_at_a_time() {
        let fresh = road([], Grant::Unearned);
        assert_eq!(first(&fresh, GATHERER, None), Some(Lesson::Walk));
        // Nothing of a later chapter before this one's done
        assert!(!ready(Lesson::SeeRuralLot, &fresh, GATHERER, None));
        assert!(!ready(Lesson::Deposit, &fresh, GATHERER, None));
        let arrived = road(chapters(&[Chapter::Arrival]), Grant::Unearned);
        assert_eq!(first(&arrived, GATHERER, None), Some(Lesson::SeeRuralLot));
        // Within a chapter, a step done out of order is passed over
        let mut done = chapters(&[Chapter::Arrival]);
        done.push(Lesson::SeeSuburbanLot);
        let skipped = road(done, Grant::Unearned);
        assert!(!ready(Lesson::SeeSuburbanLot, &skipped, GATHERER, None));
        assert!(ready(Lesson::SeeUrbanLot, &skipped, GATHERER, None));
    }

    #[test]
    fn the_market_box_waits_for_both_orders() {
        let mut done = chapters(&[Chapter::Arrival, Chapter::RoadIn]);
        done.push(Lesson::Deposit);
        done.push(Lesson::SellOrder);
        assert!(!ready(Lesson::CollectBox, &road(done.clone(), Grant::Unearned), GATHERER, None));
        done.push(Lesson::BuyOrder);
        assert!(ready(Lesson::CollectBox, &road(done, Grant::Unearned), GATHERER, None));
    }

    #[test]
    fn the_claim_waits_for_an_open_deed_and_a_spent_one_claims_nothing() {
        let walked = chapters(&[Chapter::Arrival, Chapter::RoadIn, Chapter::Hall, Chapter::People]);
        assert!(ready(Lesson::ClaimHome, &road(walked.clone(), Grant::Open), GATHERER, None));
        assert!(!ready(Lesson::ClaimHome, &road(walked.clone(), Grant::Spent), GATHERER, None));
        // One step short of the road, and the office is shut to it
        let short: Vec<Lesson> = walked.iter().copied().filter(|s| *s != Lesson::OpenCard).collect();
        assert!(!ready(Lesson::OpenOffice, &road(short, Grant::Unearned), GATHERER, None));
        assert!(ready(Lesson::OpenOffice, &road(walked, Grant::Open), GATHERER, None));
    }

    #[test]
    fn settling_in_wants_a_home_and_a_builder_builds_its_own() {
        let claimed = chapters(&[
            Chapter::Arrival,
            Chapter::RoadIn,
            Chapter::Hall,
            Chapter::People,
            Chapter::DeedsOffice,
        ]);
        let p = road(claimed, Grant::Spent);
        assert!(!ready(Lesson::PlaceAtHome, &p, GATHERER, None), "no lot yet");
        let home = HomeFacts {
            room: 3,
            crafting_station: false,
        };
        assert!(ready(Lesson::PlaceAtHome, &p, GATHERER, Some(&home)));
        // A trade that builds on its lot sets its own station there first
        assert!(!ready(Lesson::PlaceAtHome, &p, Role::Builder, Some(&home)));
        assert!(!ready(Lesson::PlaceAtHome, &p, Role::Crafter(Craft::Planks), Some(&home)));
        let full = HomeFacts {
            room: 0,
            crafting_station: true,
        };
        assert!(!ready(Lesson::PlaceAtHome, &p, GATHERER, Some(&full)));
        assert!(!ready(Lesson::SetAutomation, &p, GATHERER, Some(&home)));
        assert!(ready(Lesson::SetAutomation, &p, GATHERER, Some(&full)));
        // Only the screen's step needs nothing
        assert!(ready(Lesson::ReadTax, &p, GATHERER, None));
    }

    #[test]
    fn a_walked_road_has_nothing_left() {
        let all = road(Lesson::ALL, Grant::Spent);
        assert_eq!(first(&all, GATHERER, None), None);
    }

    #[test]
    fn every_step_says_what_taking_it_is() {
        let words: std::collections::HashSet<&str> = Lesson::ALL.into_iter().map(does).collect();
        assert_eq!(words.len(), Lesson::ALL.len(), "two steps read the same");
    }

    #[test]
    fn the_night_takes_only_what_needs_no_walk() {
        let tonight: Vec<Lesson> = Lesson::ALL.into_iter().filter(|s| at_home_tonight(*s)).collect();
        assert_eq!(tonight, [Lesson::OpenPack, Lesson::ReadZoneCard, Lesson::ReadTax]);
    }
}
