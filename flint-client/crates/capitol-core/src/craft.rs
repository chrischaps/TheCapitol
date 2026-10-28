//! Crafting by hand (P5.5): which recipe the input slots make, what's still
//! missing, what the craft would send, its quality range, and whether the
//! output has anywhere to go. Then [`Crafting`], which follows one craft
//! from the ask to the settled inventory. React's rules
//! (`CraftingPanel.tsx`), kept out of the engine so they're plain tests.
//!
//! **At a station (P6.3)** the same rules hold, with the station's box as
//! the input box and the station's recipes ([`crate::station::recipes_at`])
//! in place of the hand ones. [`Crafting::start_at`] sends
//! `craft_at_station`. The output stays at the station, in its box, as
//! yours (#489), so the box is the output box too, and [`check_at`] counts
//! the slots the craft's own inputs would empty as room for it. The box
//! holds other people's things too, and says whose each stack is (#498), so
//! [`Whose`] keeps the craft to the server's rule: it takes only your
//! stacks and joins only one of yours (on a wage shift, it takes anything
//! there and joins the employer's). Fill counts only yours, and Clear
//! leaves the rest in the box.
//!
//! - **The recipe.** A recipe picked by hand wins. Otherwise [`detect`]
//!   finds one the inputs satisfy: an exact match first, then the one that
//!   uses the most. Only hand recipes ([`hand_recipes`]); the server
//!   refuses a station recipe here.
//! - **The list (#490).** The inputs narrow it: [`shortlist`] puts the
//!   recipes they satisfy first, marked ready, then those that take
//!   something in them, then the rest. [`auto_label`] says in words what
//!   the automatic choice is ("Auto: rope, from what's in").
//! - **What's sent.** [`Check::inputs`] holds the input stacks a craft
//!   needs, in slot order, and no more. The server takes what it needs from
//!   them and leaves the rest. React sends every stack in the inputs.
//! - **The quality range.** The server's own formula,
//!   `rules::quality::output_range`, over the units the craft would take
//!   and the recipe's weights from the catalog, so it's exact: the server
//!   rolls only the variance (E1.1). The server takes units from the stacks
//!   in the order they're sent, and the best units of each stack first
//!   (`Pick::Best`, E1.8), as [`check`] counts them.
//! - **The tool (E1.5 to E1.7).** A recipe that names a `tool_type` takes
//!   the tool term from the best tool of that type in the inventory, by
//!   power times wear ([`tool_for`], the server's `best_tool`). At a
//!   station, a tool in its slot wins when its type matches
//!   (`tool_for_station_recipe`). Without one the server refuses, so
//!   [`Check::missing_tool`] holds the craft back. [`Check::lifts`] says
//!   how far an A-grade station or tool would raise the range's floor.
//! - **Output with nowhere to go.** The server puts the output in the
//!   output box: onto the first stack of its type, whatever its grade
//!   (every unit keeps its quality, E1.8), else into a free slot, and if
//!   there's neither it refuses the craft. [`Check::blocked`] names what's in
//!   the way, and the craft isn't sent.
//! - **A finished craft settles in two refetches.** The tick writes the
//!   output, then deletes the inputs, each with its own `inventory_updated`.
//!   A refetch between them shows the output with the inputs still there,
//!   so [`Crafting`] holds the next craft until the inputs it sent have
//!   changed, or [`SETTLE`] has passed.

use std::time::{Duration, Instant};

use capitol_protocol::rest::{RecipeInfo, SlotItem, StationTool};
use capitol_protocol::rules::quality;
use capitol_protocol::rules::stacking::Pick;
use capitol_protocol::rules::tools::{self, Tool};
use capitol_protocol::ws::ClientMessage;
use uuid::Uuid;

use crate::catalog::Catalog;
use crate::state::{Container, GameState, StateEvent};

/// How long a `craft` may go unanswered. The server always answers, with
/// `crafting_started` or `crafting_failed`, within a tick or two.
pub const UNANSWERED: Duration = Duration::from_secs(3);

/// The longest a finished craft holds the next one, waiting for the
/// inputs it used to leave the inventory.
pub const SETTLE: Duration = Duration::from_secs(2);

/// The recipes that need no station.
pub fn hand_recipes(recipes: &[RecipeInfo]) -> impl Iterator<Item = &RecipeInfo> {
    recipes.iter().filter(|r| r.required_station_type.is_none())
}

/// How many of `item_type` the container holds, across its stacks.
pub fn count(container: &Container, item_type: &str) -> i32 {
    container
        .items()
        .filter(|i| i.item_type == item_type)
        .map(|i| i.quantity)
        .sum()
}

/// As [`count`], of the stacks a craft may take ([`Whose::takes`]).
pub fn count_of(container: &Container, item_type: &str, whose: Whose) -> i32 {
    container
        .items()
        .filter(|i| i.item_type == item_type && whose.takes(i))
        .map(|i| i.quantity)
        .sum()
}

/// The recipe the inputs make, if any: React's auto-detect. One the inputs
/// satisfy exactly wins; among the rest, the one that uses the most.
pub fn detect<'a>(
    recipes: impl IntoIterator<Item = &'a RecipeInfo>,
    input: &Container,
) -> Option<&'a RecipeInfo> {
    if input.used_slots() == 0 {
        return None;
    }
    let mut best: Option<(&RecipeInfo, bool, i32)> = None;
    for recipe in recipes {
        if recipe.inputs.is_empty() {
            continue;
        }
        let mut exact = true;
        let mut uses = 0;
        let satisfied = recipe.inputs.iter().all(|need| {
            let have = count(input, &need.item_type);
            exact &= have == need.quantity;
            uses += need.quantity;
            have >= need.quantity
        });
        if !satisfied {
            continue;
        }
        let better = match best {
            None => true,
            Some((_, best_exact, best_uses)) => {
                (exact && !best_exact) || (exact == best_exact && uses > best_uses)
            }
        };
        if better {
            best = Some((recipe, exact, uses));
        }
    }
    best.map(|(recipe, ..)| recipe)
}

/// How a recipe stands against what's in the inputs, for the picker (#490).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// The inputs hold all it needs.
    Ready,
    /// It takes something that's in the inputs, but they're short of it.
    Uses,
    /// It takes nothing that's in the inputs, or the inputs are empty.
    Other,
}

/// One line of the picker: a recipe and how it fits the inputs.
#[derive(Debug, Clone, Copy)]
pub struct Listed<'a> {
    pub recipe: &'a RecipeInfo,
    pub fit: Fit,
}

/// The recipes as the picker lists them (#490): the inputs narrow the list
/// to what you could make from them. The ready ones first, then those that
/// take something in the inputs (the most kinds of it first), then the
/// rest. The catalog's order holds within each.
pub fn shortlist<'a>(
    recipes: impl IntoIterator<Item = &'a RecipeInfo>,
    input: &Container,
) -> Vec<Listed<'a>> {
    let mut listed: Vec<(Listed, usize)> = recipes
        .into_iter()
        .map(|recipe| {
            let shared = recipe
                .inputs
                .iter()
                .filter(|need| count(input, &need.item_type) > 0)
                .count();
            let ready = !recipe.inputs.is_empty()
                && recipe
                    .inputs
                    .iter()
                    .all(|need| count(input, &need.item_type) >= need.quantity);
            let fit = match (ready, shared) {
                (true, _) => Fit::Ready,
                (false, 0) => Fit::Other,
                (false, _) => Fit::Uses,
            };
            (Listed { recipe, fit }, shared)
        })
        .collect();
    listed.sort_by_key(|(l, shared)| (l.fit, std::cmp::Reverse(*shared)));
    listed.into_iter().map(|(l, _)| l).collect()
}

/// What the picker's automatic choice says, in words rather than
/// "auto-detect" (#490): what the inputs make, or why they make nothing.
pub fn auto_label(input: &Container, list: &[Listed]) -> String {
    if input.used_slots() == 0 {
        return "Pick one, or put items in to choose".to_owned();
    }
    if let Some(r) = detect(list.iter().map(|l| l.recipe), input) {
        return format!("Auto: {}, from what's in", r.name);
    }
    match list.iter().filter(|l| l.fit == Fit::Uses).count() {
        0 => "Auto: nothing takes what's in".to_owned(),
        1 => "Auto: not enough yet for the 1 that takes it".to_owned(),
        n => format!("Auto: not enough yet for the {n} that take it"),
    }
}

/// One of a recipe's inputs, against what the slots hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    pub item_type: String,
    pub item_name: String,
    pub have: i32,
    pub required: i32,
}

impl Need {
    pub fn met(&self) -> bool {
        self.have >= self.required
    }
}

/// The stack in the output box that leaves the craft's output nowhere to go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    pub item_id: Uuid,
    pub item_name: String,
}

/// The tool a craft would use, and where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolPick {
    pub item_id: Uuid,
    pub item_type: String,
    pub item_name: String,
    pub tool: Tool,
    /// It's the station's own, in its tool slot (E1.6), not yours.
    pub installed: bool,
}

impl ToolPick {
    /// The quality it brings to the tool term: its own, scaled by its wear
    /// (`rules::tools::term_quality`).
    pub fn term(&self) -> i32 {
        tools::term_quality(&self.tool)
    }
}

/// A tool as the rules see it: `item_type`'s power from the catalog, and
/// its own wear. `None` for anything that isn't a tool. A missing maximum
/// is worked out from the type and quality, as the server does.
pub fn as_tool(
    catalog: &Catalog,
    item_type: &str,
    quality: i32,
    durability: Option<i32>,
    max_durability: Option<i32>,
) -> Option<Tool> {
    let spec = catalog.item_type(item_type)?.tool.as_ref()?;
    Some(Tool {
        base_power: spec.base_power,
        quality,
        durability: durability.unwrap_or(0),
        max_durability: max_durability
            .unwrap_or_else(|| tools::max_durability(spec.max_durability, quality)),
    })
}

/// The tool `recipe` would be made with, as the server picks it: the
/// station's installed tool when it's of the recipe's type and has uses
/// left, else the best of that type in `inventory` (power times wear, ties
/// to the first slot). `None` when the recipe needs no tool, or there's none.
pub fn tool_for(
    catalog: &Catalog,
    recipe: &RecipeInfo,
    inventory: Option<&Container>,
    installed: Option<&StationTool>,
) -> Option<ToolPick> {
    let wanted = recipe.tool_type.as_deref()?;
    let of_type = |item_type: &str| {
        catalog
            .item_type(item_type)
            .and_then(|t| t.tool.as_ref())
            .is_some_and(|t| t.tool_type == wanted)
    };
    if let Some(slot) = installed.filter(|t| t.durability > 0 && of_type(&t.item_type)) {
        let tool = as_tool(
            catalog,
            &slot.item_type,
            slot.quality,
            Some(slot.durability),
            Some(slot.max_durability),
        )?;
        return Some(ToolPick {
            item_id: slot.item_id,
            item_type: slot.item_type.clone(),
            item_name: catalog.item_name(&slot.item_type),
            tool,
            installed: true,
        });
    }
    let held: Vec<(&SlotItem, Tool)> = inventory?
        .items()
        .filter(|i| of_type(&i.item_type))
        .filter_map(|i| {
            as_tool(catalog, &i.item_type, i.quality, i.durability, i.max_durability)
                .map(|tool| (i, tool))
        })
        .collect();
    let (best, tool) = tools::best_tool(held.iter().map(|&(i, tool)| ((i, tool), tool)))?;
    Some(ToolPick {
        item_id: best.item_id,
        item_type: best.item_type.clone(),
        item_name: best.item_name.clone(),
        tool,
        installed: false,
    })
}

/// [`tool_for`] from what `state` holds: your inventory, and at a station
/// (`at_station`) the open station's tool slot.
pub fn tool_in(state: &GameState, recipe: &RecipeInfo, at_station: bool) -> Option<ToolPick> {
    let installed = at_station
        .then(|| state.open_station_info().and_then(|s| s.tool.as_ref()))
        .flatten();
    tool_for(state.catalog(), recipe, state.inventory(), installed)
}

/// The quality [`Check::lifts`] imagines a better station or tool at: the
/// bottom of grade A.
pub const LIFT_AT: i32 = 90;

/// What a better station or tool would do for a craft.
#[derive(Debug, Clone, PartialEq)]
pub struct Lift {
    /// `Station` or `Tool`.
    pub source: quality::Source,
    /// The quality imagined, [`LIFT_AT`].
    pub at: i32,
    /// The range's floor with it.
    pub floor: i32,
    /// How much higher that is than now.
    pub by: i32,
}

/// A recipe weighed against the input and output boxes.
#[derive(Debug, Clone, PartialEq)]
pub struct Check<'a> {
    pub recipe: &'a RecipeInfo,
    pub needs: Vec<Need>,
    /// The stacks to send with `craft`, in slot order: enough of each
    /// input, and nothing the recipe doesn't use.
    pub inputs: Vec<Uuid>,
    /// The units the craft would take, by type.
    pub taken: quality::Inputs,
    /// The lowest and highest quality the output can come out at, once
    /// every need is met. The server's result is always in it.
    pub range: Option<(i32, i32)>,
    /// What's in the way in the output box, if the output would be lost.
    pub blocked: Option<Blocked>,
    /// The quality of the station it's made at (`quality::NO_STATION` by hand).
    pub station: i32,
    /// The tool it would use, for a recipe that names one.
    pub tool: Option<ToolPick>,
}

impl Check<'_> {
    /// Every input is there.
    pub fn satisfied(&self) -> bool {
        self.needs.iter().all(Need::met)
    }

    /// The craft can be sent: every input is there, so is its tool, and the
    /// output has somewhere to go.
    pub fn ready(&self) -> bool {
        self.satisfied() && self.missing_tool().is_none() && self.blocked.is_none()
    }

    /// The tool type the recipe needs and there's none of: "hammer".
    pub fn missing_tool(&self) -> Option<&str> {
        self.recipe.tool_type.as_deref().filter(|_| self.tool.is_none())
    }

    /// The quality the tool term counts: the tool's after wear, or 0.
    pub fn tool_term(&self) -> i32 {
        self.tool.as_ref().map_or(0, ToolPick::term)
    }

    /// Where the output's quality comes from: each input's weight and mean
    /// quality, then the station's and the tool's.
    pub fn breakdown(&self) -> Vec<quality::Term> {
        quality::terms(
            &self.taken,
            self.station,
            self.tool_term(),
            &self.recipe.quality_weights,
        )
    }

    /// How far an A-grade station, or an A-grade tool in good repair, would
    /// raise the range's floor, where one would. Nothing until every input
    /// is there.
    pub fn lifts(&self) -> Vec<Lift> {
        let Some((floor, _)) = self.range else {
            return Vec::new();
        };
        let weights = &self.recipe.quality_weights;
        let tool = self.tool_term();
        let candidates = [
            (quality::Source::Station, weights.station, self.station, (LIFT_AT, tool)),
            (quality::Source::Tool, weights.tool, tool, (self.station, LIFT_AT)),
        ];
        candidates
            .into_iter()
            .filter(|&(_, weight, now, _)| weight > 0.0 && now < LIFT_AT)
            .filter_map(|(source, _, _, (station, tool))| {
                let (lifted, _) = quality::output_range(&self.taken, station, tool, weights);
                (lifted > floor).then(|| Lift {
                    source,
                    at: LIFT_AT,
                    floor: lifted,
                    by: lifted - floor,
                })
            })
            .collect()
    }

    /// The `craft` message for it.
    pub fn message(&self) -> ClientMessage {
        ClientMessage::Craft {
            recipe_id: self.recipe.id.clone(),
            input_item_ids: self.inputs.iter().map(Uuid::to_string).collect(),
        }
    }

    /// The `craft_at_station` message for it, at `station`.
    pub fn message_at(&self, station: Uuid) -> ClientMessage {
        ClientMessage::CraftAtStation {
            station_id: station.to_string(),
            recipe_id: self.recipe.id.clone(),
            input_item_ids: self.inputs.iter().map(Uuid::to_string).collect(),
        }
    }
}

/// Weighs a hand `recipe` against the input box, and the output box if
/// it's known, with `tool` ([`tool_for`]) for a recipe that takes one.
pub fn check<'a>(
    recipe: &'a RecipeInfo,
    input: &Container,
    output: Option<&Container>,
    tool: Option<ToolPick>,
) -> Check<'a> {
    check_at(recipe, input, output, quality::NO_STATION, tool, Whose::Any)
}

/// Whose stacks a craft may take, and which its output may join (#498). A
/// station's box holds other people's things too (#80) and says whose each
/// stack is (`SlotItem::owner_id`); your own boxes don't, and needn't. A
/// stack that doesn't say is taken as yours, as it was before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Whose {
    /// Your own boxes: every stack is yours.
    Any,
    /// At a station, as `me`: the craft takes your stacks and joins one of
    /// yours, as the server's does (#489).
    Mine(Uuid),
    /// On a wage shift (Commons D7): the craft takes anything in the box,
    /// and its output is the employer's, so it joins only theirs.
    Shift { employer: Uuid },
}

impl Whose {
    /// At the open station: on `employer`'s shift if you're on one, else
    /// yours. `Any` when who you are isn't known yet.
    pub fn at(me: Option<Uuid>, employer: Option<Uuid>) -> Self {
        match (employer, me) {
            (Some(employer), _) => Whose::Shift { employer },
            (None, Some(me)) => Whose::Mine(me),
            (None, None) => Whose::Any,
        }
    }

    /// A craft may take `item`.
    pub fn takes(self, item: &SlotItem) -> bool {
        match self {
            Whose::Mine(me) => item.owner_id.is_none_or(|o| o == me),
            Whose::Any | Whose::Shift { .. } => true,
        }
    }

    /// A craft's output may join `item`, a stack of its type.
    pub fn joins(self, item: &SlotItem) -> bool {
        match self {
            Whose::Any => true,
            Whose::Mine(who) | Whose::Shift { employer: who } => item.owner_id.is_none_or(|o| o == who),
        }
    }
}

/// As [`check`], at a station of quality `station`: its share of the output
/// counts that quality, as the server's does. `whose` says which stacks in
/// the box the craft may take and join ([`Whose`]).
pub fn check_at<'a>(
    recipe: &'a RecipeInfo,
    input: &Container,
    output: Option<&Container>,
    station: i32,
    tool: Option<ToolPick>,
    whose: Whose,
) -> Check<'a> {
    let mut needs = Vec::with_capacity(recipe.inputs.len());
    let mut inputs = Vec::new();
    let mut taken = quality::Inputs::default();
    // The stacks the craft uses up whole: at a station they leave room in
    // the box for the output
    let mut emptied = Vec::new();
    for need in &recipe.inputs {
        let mut left = need.quantity;
        for item in input.items().filter(|i| i.item_type == need.item_type && whose.takes(i)) {
            if left <= 0 {
                break;
            }
            let Some(used) = item.units.pick(left.min(item.quantity), Pick::Best) else {
                continue;
            };
            left -= used.count() as i32;
            taken.add_units(&item.item_type, &used);
            inputs.push(item.item_id);
            if used.count() as i32 >= item.quantity {
                emptied.push(item.item_id);
            }
        }
        needs.push(Need {
            item_type: need.item_type.clone(),
            item_name: need.item_name.clone(),
            have: count_of(input, &need.item_type, whose),
            required: need.quantity,
        });
    }
    let satisfied = needs.iter().all(Need::met);
    let tool_term = tool.as_ref().map_or(0, ToolPick::term);
    let range = satisfied.then(|| {
        quality::output_range(&taken, station, tool_term, &recipe.quality_weights)
    });
    Check {
        recipe,
        needs,
        inputs,
        taken,
        range,
        // At a station the box is both, and what the craft empties is room
        blocked: output.and_then(|o| blocked(recipe, o, if o.id == input.id { &emptied } else { &[] }, whose)),
        station,
        tool,
    }
}

/// What in `output` would leave one of the recipe's outputs nowhere to go:
/// a stack of its type `whose` may join takes it, whatever the grade
/// (E1.8); anything else needs a free slot. The stacks in `emptied` are
/// gone by then (the server takes the inputs first), so their slots are
/// free. A stack of its type that isn't yours is what's named in the way.
fn blocked(recipe: &RecipeInfo, output: &Container, emptied: &[Uuid], whose: Whose) -> Option<Blocked> {
    let mut free = output.slot_count() - output.used_slots() + emptied.len();
    let staying: Vec<&SlotItem> = output.items().filter(|i| !emptied.contains(&i.item_id)).collect();
    for out in &recipe.outputs {
        let of_type = || staying.iter().filter(|i| i.item_type == out.item_type);
        if of_type().any(|i| whose.joins(i)) {
            continue;
        }
        if free == 0 {
            let first = of_type().next().or(staying.first())?;
            return Some(Blocked {
                item_id: first.item_id,
                item_name: first.item_name.clone(),
            });
        }
        free -= 1;
    }
    None
}

/// A move between boxes: the item, the box and the slot.
pub type Shift = (Uuid, Uuid, i32);

/// The moves that bring a recipe's inputs from `from` (the inventory) into
/// `input`, whole stacks into empty slots, until each need is met or the
/// slots run out. What's already in `input` counts, if the craft may take
/// it (`whose`: at a station, someone else's fiber isn't yours to use).
pub fn fill(recipe: &RecipeInfo, from: &Container, input: &Container, whose: Whose) -> Vec<Shift> {
    let mut empty = (0..input.slot_count()).filter(|&s| input.slot(s).is_none());
    let mut shifts = Vec::new();
    'needs: for need in &recipe.inputs {
        let mut left = need.quantity - count_of(input, &need.item_type, whose);
        // Best first: the craft uses the stacks in slot order.
        let mut stacks: Vec<_> = from
            .items()
            .filter(|i| i.item_type == need.item_type)
            .collect();
        stacks.sort_by_key(|i| std::cmp::Reverse(i.quality));
        for item in stacks {
            if left <= 0 {
                break;
            }
            let Some(slot) = empty.next() else {
                break 'needs;
            };
            shifts.push((item.item_id, input.id, slot as i32));
            left -= item.quantity;
        }
    }
    shifts
}

/// The moves that put everything in `from` back into `to`: onto a stack of
/// the same type where there is one, whatever its grade (every unit keeps
/// its quality, E1.8), else into an empty slot. Each target is used once, so
/// moves sent together can't race for a slot. Only what `whose` may take
/// moves: at a station, someone else's things stay in its box (#498).
pub fn empty_into(from: &Container, to: &Container, whose: Whose) -> Vec<Shift> {
    let mut taken = vec![false; to.slot_count()];
    let mut shifts = Vec::new();
    for item in from.items().filter(|i| whose.takes(i)) {
        let onto = (0..to.slot_count())
            .find(|&s| {
                !taken[s]
                    && to.slot(s).is_some_and(|t| t.item_type == item.item_type)
            })
            .or_else(|| (0..to.slot_count()).find(|&s| !taken[s] && to.slot(s).is_none()));
        let Some(slot) = onto else {
            break;
        };
        taken[slot] = true;
        shifts.push((item.item_id, to.id, slot as i32));
    }
    shifts
}

/// Where a craft stands, as far as the panel is concerned.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Nothing under way: a craft can be sent.
    Idle,
    /// `craft` went out; no answer yet.
    Asking { recipe_id: String, since: Instant },
    /// The server is crafting (`GameState::is_crafting`).
    Running,
    /// Done, and the inputs it used haven't left the inventory yet.
    Settling { since: Instant },
}

/// Something worth telling the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// The server refused, in its words ("Missing 1 more fiber").
    Failed(String),
    /// A `craft` went out and nothing came back.
    Unanswered,
}

/// Follows one craft: asked, running, then settling until the inventory
/// shows it. Feed it every batch of state changes and tick it each frame.
#[derive(Debug)]
pub struct Crafting {
    phase: Phase,
    /// The stacks the last craft was sent with, and their quantities then.
    sent: Vec<(Uuid, i32)>,
    notes: Vec<Note>,
}

impl Default for Crafting {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            sent: Vec::new(),
            notes: Vec::new(),
        }
    }
}

impl Crafting {
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// A craft can be sent now: nothing under way, and the local player
    /// isn't busy with anything else.
    pub fn can_start(&self, state: &GameState) -> bool {
        self.phase == Phase::Idle && state.local_activity().is_none()
    }

    /// Asks for `check`'s craft. Returns the message to send, or `None` if
    /// it isn't ready or something is under way.
    pub fn start(
        &mut self,
        check: &Check,
        state: &GameState,
        now: Instant,
    ) -> Option<ClientMessage> {
        self.begin(check, state, now).then(|| check.message())
    }

    /// As [`Crafting::start`], at `station`: `craft_at_station`.
    pub fn start_at(
        &mut self,
        check: &Check,
        station: Uuid,
        state: &GameState,
        now: Instant,
    ) -> Option<ClientMessage> {
        self.begin(check, state, now)
            .then(|| check.message_at(station))
    }

    /// Takes `check`'s craft as asked, if it can start.
    fn begin(&mut self, check: &Check, state: &GameState, now: Instant) -> bool {
        if !check.ready() || !self.can_start(state) {
            return false;
        }
        self.sent = check
            .inputs
            .iter()
            .filter_map(|&id| held(state, id).map(|q| (id, q)))
            .collect();
        self.phase = Phase::Asking {
            recipe_id: check.recipe.id.clone(),
            since: now,
        };
        true
    }

    /// Asks to stop the craft under way. `None` when there's none.
    pub fn cancel(&self, state: &GameState) -> Option<ClientMessage> {
        state.is_crafting().then_some(ClientMessage::CancelCrafting)
    }

    /// After every batch of state changes.
    pub fn observe(&mut self, changes: &[StateEvent], state: &GameState, now: Instant) {
        for change in changes {
            match change {
                StateEvent::CraftingStarted { .. } => self.phase = Phase::Running,
                StateEvent::CraftingCompleted { .. } => self.phase = Phase::Settling { since: now },
                StateEvent::CraftingCancelled => self.phase = Phase::Idle,
                StateEvent::CraftingFailed { reason } => {
                    self.phase = Phase::Idle;
                    self.notes.push(Note::Failed(reason.clone()));
                }
                // The server dropped and re-added us idle: nothing is running.
                StateEvent::SnapshotStale => self.phase = Phase::Idle,
                _ => {}
            }
        }
        if matches!(self.phase, Phase::Settling { .. })
            && changes.iter().any(|c| {
                matches!(
                    c,
                    StateEvent::ContainersUpdated | StateEvent::StationContainerUpdated
                )
            })
            && self.sent.iter().any(|&(id, q)| held(state, id) != Some(q))
        {
            self.phase = Phase::Idle;
        }
    }

    /// Once per frame: gives up on an unanswered ask, and on settling.
    pub fn tick(&mut self, now: Instant) {
        match self.phase {
            Phase::Asking { since, .. } if now.duration_since(since) >= UNANSWERED => {
                self.phase = Phase::Idle;
                self.notes.push(Note::Unanswered);
            }
            Phase::Settling { since } if now.duration_since(since) >= SETTLE => {
                self.phase = Phase::Idle;
            }
            _ => {}
        }
    }

    /// What's worth telling the player since the last call.
    pub fn take_notes(&mut self) -> Vec<Note> {
        std::mem::take(&mut self.notes)
    }
}

/// How many of a stack the player holds, in any box, the open station's too.
fn held(state: &GameState, item_id: Uuid) -> Option<i32> {
    state
        .containers()
        .values()
        .chain(state.station_container())
        .find_map(|c| c.find(item_id))
        .map(|i| i.quantity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{ConnectionState, RestResponse};
    use capitol_protocol::rest::{
        ContainerResponse, ContainersResponse, RecipeInputInfo, RecipeOutputInfo, SlotItem,
    };
    use capitol_protocol::rules::stacking::Units;
    use capitol_protocol::ws::ServerMessage as M;

    const ME: Uuid = Uuid::from_u128(1);
    const INV: Uuid = Uuid::from_u128(100);
    const IN: Uuid = Uuid::from_u128(101);
    const OUT: Uuid = Uuid::from_u128(102);

    /// A hand recipe, its inputs weighted evenly, give or take 3.
    fn recipe(id: &str, inputs: &[(&str, i32)], output: &str) -> RecipeInfo {
        RecipeInfo {
            tool_type: None,
            id: id.into(),
            name: id.into(),
            description: None,
            category: "hand_crafting".into(),
            inputs: inputs
                .iter()
                .map(|(t, q)| RecipeInputInfo {
                    item_type: (*t).into(),
                    item_name: (*t).into(),
                    quantity: *q,
                })
                .collect(),
            outputs: vec![RecipeOutputInfo {
                item_type: output.into(),
                item_name: output.into(),
                quantity_min: 1,
                quantity_max: 1,
            }],
            duration_ticks: 50,
            required_station_type: None,
            quality_weights: quality::Weights::for_recipe(
                inputs.iter().map(|(t, _)| (*t, 1.0)),
                None,
                None,
                false,
                -3,
                3,
            ),
        }
    }

    /// The server's three hand recipes.
    fn recipes() -> Vec<RecipeInfo> {
        vec![
            recipe("rope_basic", &[("fiber", 2)], "rope"),
            recipe("craft_workbench", &[("fiber", 10)], "workbench_kit"),
            recipe("craft_chest", &[("fiber", 8)], "chest_kit"),
        ]
    }

    /// (id, slot, type, quality, quantity)
    fn item(n: u128, slot: i32, kind: &str, quality: i32, quantity: i32) -> SlotItem {
        SlotItem::new(
            Uuid::from_u128(n),
            kind.into(),
            kind.into(),
            quality,
            quantity,
            slot,
        )
    }

    fn boxed(id: Uuid, kind: &str, slots: i32, items: &[SlotItem]) -> Container {
        Container::new(id, kind, slots, 2, items)
    }

    fn input(items: &[SlotItem]) -> Container {
        boxed(IN, "crafting_input", 4, items)
    }

    #[test]
    fn the_inputs_pick_the_recipe() {
        let r = recipes();
        let name = |items: &[SlotItem]| detect(&r, &input(items)).map(|r| r.id.as_str());
        assert_eq!(name(&[]), None);
        assert_eq!(name(&[item(1, 0, "fiber", 50, 1)]), None);
        // Exactly two: rope.
        assert_eq!(name(&[item(1, 0, "fiber", 50, 2)]), Some("rope_basic"));
        // Nine is exact for nothing; the chest uses the most it can.
        assert_eq!(name(&[item(1, 0, "fiber", 50, 9)]), Some("craft_chest"));
        // Ten, across two stacks, is exactly a workbench.
        assert_eq!(
            name(&[item(1, 0, "fiber", 50, 6), item(2, 3, "fiber", 50, 4)]),
            Some("craft_workbench")
        );
        // Eight is exactly a chest, though rope fits too.
        assert_eq!(name(&[item(1, 0, "fiber", 50, 8)]), Some("craft_chest"));
        // Rope in the inputs makes nothing.
        assert_eq!(name(&[item(1, 0, "rope", 50, 3)]), None);
    }

    #[test]
    fn the_inputs_narrow_the_list() {
        let mut r = recipes();
        r.push(recipe("bundle", &[("fiber", 1), ("stick", 2)], "bundle"));
        r.push(recipe("stew", &[("water", 1)], "stew"));
        let list = |items: &[SlotItem]| {
            shortlist(&r, &input(items))
                .into_iter()
                .map(|l| (l.recipe.id.as_str(), l.fit))
                .collect::<Vec<_>>()
        };
        // Empty: everything, in the catalog's order.
        let empty = list(&[]);
        assert!(empty.iter().all(|(_, f)| *f == Fit::Other));
        assert_eq!(empty[0].0, "rope_basic");
        // Three fiber: rope is ready; then what takes fiber, the bundle
        // first for taking it too; stew last.
        assert_eq!(
            list(&[item(1, 0, "fiber", 50, 3)]),
            [
                ("rope_basic", Fit::Ready),
                ("craft_workbench", Fit::Uses),
                ("craft_chest", Fit::Uses),
                ("bundle", Fit::Uses),
                ("stew", Fit::Other),
            ]
        );
        // A stick and a fiber: the bundle takes both kinds, so it leads.
        let mixed = list(&[item(1, 0, "fiber", 50, 1), item(2, 1, "stick", 50, 1)]);
        assert_eq!(mixed[0], ("bundle", Fit::Uses));
        assert_eq!(mixed.last(), Some(&("stew", Fit::Other)));
    }

    #[test]
    fn the_automatic_choice_says_what_it_is() {
        let r = recipes();
        let label = |items: &[SlotItem]| {
            let inp = input(items);
            auto_label(&inp, &shortlist(&r, &inp))
        };
        assert_eq!(label(&[]), "Pick one, or put items in to choose");
        assert_eq!(
            label(&[item(1, 0, "fiber", 50, 2)]),
            "Auto: rope_basic, from what's in"
        );
        assert_eq!(
            label(&[item(1, 0, "fiber", 50, 1)]),
            "Auto: not enough yet for the 3 that take it"
        );
        assert_eq!(
            label(&[item(1, 0, "rope", 50, 1)]),
            "Auto: nothing takes what's in"
        );
    }

    #[test]
    fn station_recipes_are_left_out() {
        let mut r = recipes();
        r[1].required_station_type = Some("workbench".into());
        let hand: Vec<_> = hand_recipes(&r).map(|r| r.id.as_str()).collect();
        assert_eq!(hand, ["rope_basic", "craft_chest"]);
    }

    #[test]
    fn a_check_counts_what_is_there_and_sends_only_what_is_used() {
        let r = recipes();
        let inp = input(&[
            item(1, 0, "fiber", 90, 1),
            item(2, 1, "rope", 40, 1),
            item(3, 2, "fiber", 60, 5),
            item(4, 3, "fiber", 10, 5),
        ]);
        let rope = check(&r[0], &inp, None, None);
        assert_eq!(rope.needs[0].have, 11);
        assert!(rope.ready());
        // Slot order, enough and no more: one from the first, one from the second.
        assert_eq!(rope.inputs, [Uuid::from_u128(1), Uuid::from_u128(3)]);
        // (90 + 60) / 2 = 75, give or take 3.
        assert_eq!(rope.range, Some((72, 78)));
        assert_eq!(
            rope.message(),
            ClientMessage::Craft {
                recipe_id: "rope_basic".into(),
                input_item_ids: vec![
                    Uuid::from_u128(1).to_string(),
                    Uuid::from_u128(3).to_string()
                ],
            }
        );

        // Ten for the workbench: 1 × 90 + 5 × 60 + 4 × 10 = 430, over 10.
        let bench = check(&r[1], &inp, None, None);
        assert_eq!(bench.range, Some((40, 46)));
        assert_eq!(bench.inputs.len(), 3);

        // Short: no range, and not ready.
        let short = check(&r[1], &input(&[item(1, 0, "fiber", 90, 3)]), None, None);
        assert_eq!(
            short.needs[0],
            Need {
                item_type: "fiber".into(),
                item_name: "fiber".into(),
                have: 3,
                required: 10,
            }
        );
        assert!(!short.needs[0].met() && !short.ready());
        assert_eq!(short.range, None);
    }

    #[test]
    fn the_range_rounds_half_up() {
        let r = recipes();
        let inp = input(&[item(1, 0, "fiber", 50, 1), item(2, 1, "fiber", 51, 1)]);
        // 50.5 rounds to 51, then ±3.
        assert_eq!(check(&r[0], &inp, None, None).range, Some((48, 54)));
    }

    #[test]
    fn output_with_nowhere_to_go_blocks_the_craft() {
        let r = recipes();
        let inp = input(&[item(1, 0, "fiber", 50, 2)]);
        let out = |items: &[SlotItem]| boxed(OUT, "crafting_output", 1, items);

        // Empty: fine.
        assert!(check(&r[0], &inp, Some(&out(&[])), None).ready());
        // Rope joins rope of any grade, and keeps its quality (E1.8)
        for q in [40, 70] {
            assert!(check(&r[0], &inp, Some(&out(&[item(9, 0, "rope", q, 1)])), None).ready());
        }
        let mixed = item(9, 0, "rope", 40, 1).with_units(Units::from_pairs([(20, 1), (95, 2)]));
        assert!(check(&r[0], &inp, Some(&out(&[mixed])), None).ready());
        // A kit in the way: the rope would be lost.
        let c = check(&r[0], &inp, Some(&out(&[item(9, 0, "chest_kit", 70, 1)])), None);
        assert!(c.satisfied() && !c.ready());
        assert_eq!(
            c.blocked,
            Some(Blocked {
                item_id: Uuid::from_u128(9),
                item_name: "chest_kit".into(),
            })
        );
        // Not loaded yet: nothing to say.
        assert_eq!(check(&r[0], &inp, None, None).blocked, None);
    }

    /// At a station the box is the input and the output (#489): a full box
    /// still takes the rope where the craft uses up a stack of its own
    #[test]
    fn a_full_station_box_has_room_where_the_craft_empties_a_stack() {
        let r = recipes();
        let bench = |fiber: i32| {
            let mut items = vec![item(1, 0, "fiber", 50, fiber)];
            items.extend((1..4).map(|s| item(10 + s as u128, s, "chest_kit", 70, 1)));
            boxed(Uuid::from_u128(40), "station_inventory", 4, &items)
        };
        // Two fiber, both used: their slot takes the rope
        let used_up = bench(2);
        assert!(check_at(&r[0], &used_up, Some(&used_up), 50, None, Whose::Any).ready());
        // Three: one stays, and there's no room
        let left = bench(3);
        let c = check_at(&r[0], &left, Some(&left), 50, None, Whose::Any);
        assert_eq!(c.blocked.map(|b| b.item_id), Some(Uuid::from_u128(1)));
    }

    /// Rope that wants a sickle: fiber 70% and the tool 30%, give or take 3.
    fn sickle_rope() -> RecipeInfo {
        let mut r = recipe("rope_cut", &[("fiber", 2)], "rope");
        r.tool_type = Some("sickle".into());
        r.quality_weights = quality::Weights::for_recipe([("fiber", 1.0)], None, Some(0.3), false, -3, 3);
        r
    }

    /// A stone sickle (power 6, 60 uses at Q50) with `left` of `max` uses.
    fn sickle(n: u128, slot: i32, quality: i32, left: i32, max: i32) -> SlotItem {
        item(n, slot, "stone_sickle", quality, 1).with_durability(left, max)
    }

    /// The tool term comes from the best sickle by power times wear, as the
    /// server's `best_tool` picks it (E1.5)
    #[test]
    fn the_best_tool_by_power_and_wear_sets_the_tool_term() {
        let catalog = Catalog::fallback();
        let r = sickle_rope();
        let inp = input(&[item(1, 0, "fiber", 80, 2)]);
        // A Q90 sickle with 2 of 84 uses left is on the cliff: 6 × 1.4 × 0.1
        // is under the fresh Q50's 6 × 1.0.
        let worn = sickle(10, 0, 90, 2, 84);
        let fresh = sickle(11, 1, 50, 60, 60);
        let inv = boxed(INV, "player_inventory", 24, &[worn, fresh, item(12, 2, "rope", 99, 1)]);
        let tool = tool_for(&catalog, &r, Some(&inv), None).unwrap();
        assert_eq!((tool.item_id, tool.installed, tool.term()), (Uuid::from_u128(11), false, 50));

        let c = check(&r, &inp, None, Some(tool));
        assert!(c.ready());
        // 0.7 × 80 + 0.3 × 50 = 71.
        assert_eq!(c.range, Some((68, 74)));
        let tool_term = c.breakdown().into_iter().find(|t| t.source == quality::Source::Tool);
        assert_eq!(tool_term.map(|t| t.quality), Some(50.0));
        // An A-grade sickle would lift the floor by 0.3 × 40 = 12; the
        // recipe gives a station nothing.
        assert_eq!(
            c.lifts(),
            [Lift { source: quality::Source::Tool, at: LIFT_AT, floor: 80, by: 12 }]
        );

        // No sickle: held back, as the server would refuse it.
        let bare = boxed(INV, "player_inventory", 24, &[]);
        let c = check(&r, &inp, None, tool_for(&catalog, &r, Some(&bare), None));
        assert_eq!(c.missing_tool(), Some("sickle"));
        assert!(c.satisfied() && !c.ready());
        // A recipe that needs no tool never picks one.
        assert_eq!(tool_for(&catalog, &recipes()[0], Some(&inv), None), None);
    }

    /// At a station its own tool wins when its type matches, whatever the
    /// one in your hands could do (E1.6)
    #[test]
    fn a_stations_tool_wins_when_its_type_matches() {
        let catalog = Catalog::fallback();
        let r = sickle_rope();
        let inv = boxed(INV, "player_inventory", 24, &[sickle(11, 0, 95, 60, 87)]);
        let installed = |item_type: &str, durability| StationTool {
            item_id: Uuid::from_u128(20),
            item_type: item_type.into(),
            quality: 40,
            durability,
            max_durability: 54,
            owner_id: None,
        };
        let pick = |slot: StationTool| {
            tool_for(&catalog, &r, Some(&inv), Some(&slot)).map(|t| (t.item_id, t.installed))
        };
        assert_eq!(pick(installed("stone_sickle", 30)), Some((Uuid::from_u128(20), true)));
        // Spent, or not a sickle: your own.
        assert_eq!(pick(installed("stone_sickle", 0)), Some((Uuid::from_u128(11), false)));
        assert_eq!(pick(installed("rope", 30)), Some((Uuid::from_u128(11), false)));
        // Worn into the decline, its term drops: 40 × 0.5 at 5% left.
        let low = tool_for(&catalog, &r, None, Some(&StationTool { durability: 3, max_durability: 60, ..installed("stone_sickle", 0) }));
        assert_eq!(low.map(|t| t.term()), Some(20));
    }

    #[test]
    fn fill_brings_the_best_stacks_into_empty_slots() {
        let r = recipes();
        let inv = boxed(
            INV,
            "player_inventory",
            24,
            &[
                item(1, 0, "fiber", 40, 4),
                item(2, 1, "rope", 80, 1),
                item(3, 5, "fiber", 90, 3),
                item(4, 6, "fiber", 70, 9),
            ],
        );
        // Rope: the 90s alone are enough.
        assert_eq!(
            fill(&r[0], &inv, &input(&[]), Whose::Any),
            [(Uuid::from_u128(3), IN, 0)]
        );
        // A workbench: 3 + 9; the slot already taken is skipped.
        assert_eq!(
            fill(&r[1], &inv, &input(&[item(7, 0, "rope", 50, 1)]), Whose::Any),
            [(Uuid::from_u128(3), IN, 1), (Uuid::from_u128(4), IN, 2)]
        );
        // What's in the inputs already counts.
        assert!(fill(&r[0], &inv, &input(&[item(7, 2, "fiber", 50, 2)]), Whose::Any).is_empty());
        // Out of slots: as far as it goes.
        let full = input(&[
            item(7, 0, "rope", 50, 1),
            item(8, 1, "rope", 50, 1),
            item(9, 2, "rope", 50, 1),
        ]);
        assert_eq!(fill(&r[1], &inv, &full, Whose::Any), [(Uuid::from_u128(3), IN, 3)]);
    }

    #[test]
    fn emptying_merges_where_it_can_and_never_shares_a_target() {
        let inv = boxed(
            INV,
            "player_inventory",
            6,
            &[item(1, 0, "fiber", 40, 4), item(2, 2, "rope", 80, 1)],
        );
        let inp = input(&[
            item(5, 0, "fiber", 45, 3),
            item(6, 1, "fiber", 50, 2),
            item(7, 2, "fiber", 90, 1),
            item(8, 3, "chest_kit", 70, 1),
        ]);
        assert_eq!(
            empty_into(&inp, &inv, Whose::Any),
            [
                (Uuid::from_u128(5), INV, 0), // onto the fiber, a D as it is
                (Uuid::from_u128(6), INV, 1), // that one's taken: an empty slot
                (Uuid::from_u128(7), INV, 3), // the fiber's taken too: an empty slot
                (Uuid::from_u128(8), INV, 4),
            ]
        );
    }

    #[test]
    fn emptying_joins_a_stack_of_any_grade() {
        let mixed = item(1, 0, "fiber", 40, 4).with_units(Units::from_pairs([(10, 1), (50, 3)]));
        let inv = boxed(INV, "player_inventory", 4, &[mixed]);
        let inp = input(&[item(5, 0, "fiber", 95, 3)]);
        assert_eq!(empty_into(&inp, &inv, Whose::Any), [(Uuid::from_u128(5), INV, 0)]);
    }

    /// A station's box holds other people's things too, and says whose
    /// (#498): a guest's rope joins only the guest's own rope, so the
    /// owner's stack is no room for it, and is what's named in the way
    #[test]
    fn a_guests_craft_has_no_room_on_the_owners_stack() {
        let r = recipes();
        let (owner, guest) = (Uuid::from_u128(500), Uuid::from_u128(501));
        let bench = |rope_owner: Uuid| {
            let items = [
                item(1, 0, "fiber", 50, 3).with_owner(Some(guest)),
                item(2, 1, "rope", 70, 4).with_owner(Some(rope_owner)),
                item(3, 2, "chest_kit", 70, 1).with_owner(Some(owner)),
                item(4, 3, "chest_kit", 70, 1).with_owner(Some(owner)),
            ];
            boxed(Uuid::from_u128(40), "station_inventory", 4, &items)
        };
        // Full but for the owner's rope: the server would refuse, so it's held back
        let full = bench(owner);
        let c = check_at(&r[0], &full, Some(&full), 50, None, Whose::Mine(guest));
        assert!(c.satisfied() && !c.ready());
        assert_eq!(c.blocked.map(|b| b.item_id), Some(Uuid::from_u128(2)));
        // The owner's own craft joins it, as it did before
        let theirs = bench(guest);
        assert!(check_at(&r[0], &theirs, Some(&theirs), 50, None, Whose::Mine(guest)).ready());
        // A box that doesn't say whose is taken as yours, as before
        assert!(check_at(&r[0], &full, Some(&full), 50, None, Whose::Any).ready());
    }

    /// The craft takes only your stacks at a station (#498): the server
    /// refuses another's as an input. On a wage shift it takes anything in
    /// the box, and the output joins the employer's.
    #[test]
    fn a_station_craft_takes_only_yours_and_a_shift_the_employers() {
        let r = recipes();
        let (owner, guest) = (Uuid::from_u128(500), Uuid::from_u128(501));
        let items = [
            item(1, 0, "fiber", 90, 2).with_owner(Some(owner)),
            item(2, 1, "fiber", 50, 1).with_owner(Some(guest)),
            item(3, 2, "rope", 70, 1).with_owner(Some(owner)),
        ];
        let bench = boxed(Uuid::from_u128(40), "station_inventory", 3, &items);
        // The guest has 1 of the 2, and the owner's best fiber isn't theirs
        let c = check_at(&r[0], &bench, Some(&bench), 50, None, Whose::Mine(guest));
        assert_eq!((c.needs[0].have, c.inputs.clone()), (1, vec![Uuid::from_u128(2)]));
        assert!(!c.satisfied());
        // The owner's craft takes the owner's, and joins the owner's rope
        let c = check_at(&r[0], &bench, Some(&bench), 50, None, Whose::Mine(owner));
        assert!(c.ready());
        assert_eq!(c.inputs, vec![Uuid::from_u128(1)]);
        // On the owner's shift the guest takes the best there, whoever's, and
        // the rope joins the owner's
        let shift = Whose::at(Some(guest), Some(owner));
        assert_eq!(shift, Whose::Shift { employer: owner });
        let c = check_at(&r[0], &bench, Some(&bench), 50, None, shift);
        assert!(c.ready());
        assert_eq!(c.inputs, vec![Uuid::from_u128(1)]);
        // Whoever you are, unknown: as before
        assert_eq!(Whose::at(None, None), Whose::Any);
    }

    /// Fill counts only what you may use, and Clear moves only yours (#498)
    #[test]
    fn fill_and_clear_at_a_station_keep_to_yours() {
        let r = recipes();
        let (owner, guest) = (Uuid::from_u128(500), Uuid::from_u128(501));
        let inv = boxed(INV, "player_inventory", 4, &[item(9, 0, "fiber", 60, 2)]);
        let bench = boxed(
            Uuid::from_u128(40),
            "station_inventory",
            4,
            &[
                item(1, 0, "fiber", 90, 2).with_owner(Some(owner)),
                item(2, 1, "rope", 70, 1).with_owner(Some(guest)),
            ],
        );
        // The owner's fiber is no use to the guest: Fill brings theirs
        assert_eq!(
            fill(&r[0], &inv, &bench, Whose::Mine(guest)),
            [(Uuid::from_u128(9), Uuid::from_u128(40), 2)]
        );
        assert!(fill(&r[0], &inv, &bench, Whose::Mine(owner)).is_empty());
        // Clear takes the guest's rope and leaves the owner's fiber
        assert_eq!(
            empty_into(&bench, &inv, Whose::Mine(guest)),
            [(Uuid::from_u128(2), INV, 1)]
        );
    }

    /// The craft takes the best units of each stack first, as the server
    /// does, so a mixed stack's range is over its best (E1.8)
    #[test]
    fn the_range_counts_the_best_units_of_a_mixed_stack() {
        let r = recipes();
        let mixed = item(1, 0, "fiber", 70, 4).with_units(Units::from_pairs([(50, 2), (90, 2)]));
        let nineties = item(2, 0, "fiber", 90, 2);
        assert_eq!(check(&r[0], &input(&[mixed]), None, None).range, check(&r[0], &input(&[nineties]), None, None).range);
    }

    /// Connected, with an inventory, the input box holding `items`, and an
    /// empty output.
    fn world(items: &[SlotItem]) -> GameState {
        let mut state = GameState::new();
        state.apply_connection(&ConnectionState::Connected { player_id: ME });
        set_boxes(&mut state, items);
        state
    }

    fn set_boxes(state: &mut GameState, items: &[SlotItem]) -> Vec<StateEvent> {
        let c = |id, kind: &str, slot_count, slots: &[SlotItem]| ContainerResponse {
            id,
            container_type: kind.into(),
            slot_count,
            layout_columns: 2,
            slots: slots.to_vec(),
        };
        state.apply_rest(&RestResponse::Containers(ContainersResponse {
            containers: vec![
                c(INV, "player_inventory", 24, &[]),
                c(IN, "crafting_input", 4, items),
                c(OUT, "crafting_output", 1, &[]),
            ],
        }))
    }

    fn started() -> M {
        M::CraftingStarted {
            player_id: ME,
            operation_id: Uuid::from_u128(50),
            recipe_id: "rope_basic".into(),
            recipe_name: "Basic Rope".into(),
            duration_ticks: 50,
        }
    }

    fn completed() -> M {
        M::CraftingCompleted {
            tool: None,
            player_id: ME,
            operation_id: Uuid::from_u128(50),
            item_type: "rope".into(),
            item_name: "Basic Rope".into(),
            quantity: 1,
            quality: 61,
        }
    }

    #[test]
    fn a_craft_runs_then_settles_once_its_inputs_are_used() {
        let r = recipes();
        let t0 = Instant::now();
        let fiber = [item(1, 0, "fiber", 60, 3)];
        let mut state = world(&fiber);
        let mut crafting = Crafting::default();

        let c = check(
            &r[0],
            state.crafting_input().unwrap(),
            state.crafting_output(), None
        );
        assert!(crafting.start(&c, &state, t0).is_some());
        assert!(matches!(crafting.phase(), Phase::Asking { .. }));
        // Asked: a second craft isn't sent.
        assert!(crafting.start(&c, &state, t0).is_none());

        let changes = state.apply_ws(&started());
        crafting.observe(&changes, &state, t0);
        assert_eq!(crafting.phase(), &Phase::Running);
        assert_eq!(crafting.cancel(&state), Some(ClientMessage::CancelCrafting));

        let changes = state.apply_ws(&completed());
        crafting.observe(&changes, &state, t0);
        assert!(matches!(crafting.phase(), Phase::Settling { .. }));

        // The first refetch: rope written, fiber still there. Still settling.
        let changes = set_boxes(&mut state, &fiber);
        crafting.observe(&changes, &state, t0);
        assert!(matches!(crafting.phase(), Phase::Settling { .. }));
        // The second: the fiber is down to one.
        let changes = set_boxes(&mut state, &[item(1, 0, "fiber", 60, 1)]);
        crafting.observe(&changes, &state, t0);
        assert_eq!(crafting.phase(), &Phase::Idle);
        assert!(crafting.take_notes().is_empty());
    }

    #[test]
    fn a_station_craft_takes_from_the_station_box_and_settles_on_it() {
        use capitol_protocol::rest::StationContainerResponse;
        const BENCH: Uuid = Uuid::from_u128(30);
        const BOX: Uuid = Uuid::from_u128(31);
        let mut forge = recipe("craft_forge", &[("rope", 4), ("fiber", 6)], "forge_kit");
        forge.required_station_type = Some("workbench".into());
        forge.quality_weights = quality::Weights::for_recipe(
            [("rope", 0.7), ("fiber", 0.3)],
            None,
            None,
            true,
            -3,
            3,
        );
        let t0 = Instant::now();
        let mut state = world(&[]);
        state.apply_ws(&M::StationOpened {
            player_id: ME,
            station_id: BENCH,
            station_type: "workbench".into(),
            name: "Workbench".into(),
            container_id: BOX,
        });
        let fill = |state: &mut GameState, rope: i32| {
            state.apply_rest(&RestResponse::StationContainer(StationContainerResponse {
                id: BOX,
                container_type: "station_inventory".into(),
                slot_count: 16,
                layout_columns: 4,
                slots: vec![item(1, 0, "rope", 70, rope), item(2, 1, "fiber", 50, 6)],
            }))
        };
        fill(&mut state, 5);
        let mut crafting = Crafting::default();
        let bench = state.station_container().unwrap();
        let c = check_at(&forge, bench, Some(bench), 90, None, Whose::Any);
        assert!(c.ready());
        // 0.56 × 70 + 0.24 × 50 + 0.2 × 90 (an A-grade bench) = 69.2.
        assert_eq!(c.range, Some((66, 72)));
        let points: f64 = c.breakdown().iter().map(quality::Term::points).sum();
        assert!((points - 69.2).abs() < 1e-9);
        // The same box at a D-grade bench comes out 10 points lower.
        let d = check_at(&forge, state.station_container().unwrap(), None, 40, None, Whose::Any);
        assert_eq!(d.range, Some((56, 62)));
        assert_eq!(
            crafting.start_at(&c, BENCH, &state, t0),
            Some(ClientMessage::CraftAtStation {
                station_id: BENCH.to_string(),
                recipe_id: "craft_forge".into(),
                input_item_ids: vec![
                    Uuid::from_u128(1).to_string(),
                    Uuid::from_u128(2).to_string()
                ],
            })
        );
        let changes = state.apply_ws(&started());
        crafting.observe(&changes, &state, t0);
        let changes = state.apply_ws(&completed());
        crafting.observe(&changes, &state, t0);
        assert!(matches!(crafting.phase(), Phase::Settling { .. }));
        // The station box is refetched with the rope used up.
        let changes = fill(&mut state, 1);
        crafting.observe(&changes, &state, t0);
        assert_eq!(crafting.phase(), &Phase::Idle);
    }

    #[test]
    fn settling_gives_up_after_a_while() {
        let t0 = Instant::now();
        let mut state = world(&[item(1, 0, "fiber", 60, 2)]);
        let mut crafting = Crafting::default();
        let changes = state.apply_ws(&completed());
        crafting.observe(&changes, &state, t0);
        crafting.tick(t0 + SETTLE / 2);
        assert!(matches!(crafting.phase(), Phase::Settling { .. }));
        crafting.tick(t0 + SETTLE);
        assert_eq!(crafting.phase(), &Phase::Idle);
    }

    #[test]
    fn refusals_and_silence_are_noted() {
        let r = recipes();
        let t0 = Instant::now();
        let mut state = world(&[item(1, 0, "fiber", 60, 2)]);
        let mut crafting = Crafting::default();
        let c = check(&r[0], state.crafting_input().unwrap(), None, None);

        crafting.start(&c, &state, t0).unwrap();
        let changes = state.apply_ws(&M::CraftingFailed {
            player_id: ME,
            reason: "Missing 1 more fiber".into(),
        });
        crafting.observe(&changes, &state, t0);
        assert_eq!(crafting.phase(), &Phase::Idle);
        assert_eq!(
            crafting.take_notes(),
            [Note::Failed("Missing 1 more fiber".into())]
        );

        crafting.start(&c, &state, t0).unwrap();
        crafting.tick(t0 + UNANSWERED);
        assert_eq!(crafting.phase(), &Phase::Idle);
        assert_eq!(crafting.take_notes(), [Note::Unanswered]);
    }

    #[test]
    fn nothing_starts_while_busy_or_blocked() {
        let r = recipes();
        let t0 = Instant::now();
        let mut state = world(&[item(1, 0, "fiber", 60, 2)]);
        let mut crafting = Crafting::default();
        state.apply_ws(&M::ExtractionStarted {
            player_id: ME,
            node_id: Uuid::from_u128(9),
            duration_ticks: 30,
        });
        let c = check(&r[0], state.crafting_input().unwrap(), None, None);
        assert!(!crafting.can_start(&state));
        assert!(crafting.start(&c, &state, t0).is_none());
        assert_eq!(crafting.cancel(&state), None);
    }

    #[test]
    fn a_reconnect_clears_a_craft() {
        let t0 = Instant::now();
        let mut state = world(&[]);
        let mut crafting = Crafting::default();
        let changes = state.apply_ws(&started());
        crafting.observe(&changes, &state, t0);
        let changes = state.apply_resync();
        crafting.observe(&changes, &state, t0);
        assert_eq!(crafting.phase(), &Phase::Idle);
    }

    #[test]
    fn a_cancel_returns_to_idle() {
        let t0 = Instant::now();
        let mut state = world(&[]);
        let mut crafting = Crafting::default();
        let changes = state.apply_ws(&started());
        crafting.observe(&changes, &state, t0);
        let changes = state.apply_ws(&M::CraftingCancelled {
            player_id: ME,
            operation_id: Uuid::from_u128(50),
        });
        crafting.observe(&changes, &state, t0);
        assert_eq!(crafting.phase(), &Phase::Idle);
    }
}
