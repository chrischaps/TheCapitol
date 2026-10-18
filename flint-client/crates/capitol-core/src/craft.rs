//! Crafting by hand (P5.5): which recipe the input slots make, what's still
//! missing, what the craft would send, its quality range, and whether the
//! output has anywhere to go. Then [`Crafting`], which follows one craft
//! from the ask to the settled inventory. React's rules
//! (`CraftingPanel.tsx`), kept out of the engine so they're plain tests.
//!
//! **At a station (P6.3)** the same rules hold, with the station's box as
//! the input box and the station's recipes ([`crate::station::recipes_at`])
//! in place of the hand ones. [`Crafting::start_at`] sends
//! `craft_at_station`. The output still lands in the player's own crafting
//! output box (`crafting.rs` on the server sets `crafting_output` whatever
//! the bench), so that's the box [`check`] weighs it against.
//!
//! - **The recipe.** A recipe picked by hand wins. Otherwise [`detect`]
//!   finds one the inputs satisfy: an exact match first, then the one that
//!   uses the most. Only hand recipes ([`hand_recipes`]); the server
//!   refuses a station recipe here.
//! - **What's sent.** [`Check::inputs`] holds the input stacks a craft
//!   needs, in slot order, and no more. The server takes what it needs from
//!   them and leaves the rest. React sends every stack in the inputs.
//! - **The quality range.** The server's own formula,
//!   `rules::quality::output_range`, over the units the craft would take
//!   and the recipe's weights from the catalog, so it's exact: the server
//!   rolls only the variance (E1.1). The server takes units from the stacks
//!   in the order they're sent, as [`check`] counts them.
//! - **Output with nowhere to go.** The server puts the output in the
//!   output box: onto a stack of the same type, else into a free slot, and
//!   if there's neither it's lost (`persist_items` in `engine/tick.rs`).
//!   [`Check::blocked`] names what's in the way, and the craft isn't sent.
//! - **A finished craft settles in two refetches.** The tick writes the
//!   output, then deletes the inputs, each with its own `inventory_updated`.
//!   A refetch between them shows the output with the inputs still there,
//!   so [`Crafting`] holds the next craft until the inputs it sent have
//!   changed, or [`SETTLE`] has passed.

use std::time::{Duration, Instant};

use capitol_protocol::rest::RecipeInfo;
use capitol_protocol::rules::quality;
use capitol_protocol::ws::ClientMessage;
use uuid::Uuid;

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
}

impl Check<'_> {
    /// Every input is there.
    pub fn satisfied(&self) -> bool {
        self.needs.iter().all(Need::met)
    }

    /// The craft can be sent: every input is there and the output has
    /// somewhere to go.
    pub fn ready(&self) -> bool {
        self.satisfied() && self.blocked.is_none()
    }

    /// Where the output's quality comes from: each input's weight and mean
    /// quality, then the station's.
    pub fn breakdown(&self) -> Vec<quality::Term> {
        quality::terms(&self.taken, self.station, 0, &self.recipe.quality_weights)
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

/// Weighs a hand `recipe` against the input box, and the output box if it's known.
pub fn check<'a>(
    recipe: &'a RecipeInfo,
    input: &Container,
    output: Option<&Container>,
) -> Check<'a> {
    check_at(recipe, input, output, quality::NO_STATION)
}

/// As [`check`], at a station of quality `station`: its share of the output
/// counts that quality, as the server's does.
pub fn check_at<'a>(
    recipe: &'a RecipeInfo,
    input: &Container,
    output: Option<&Container>,
    station: i32,
) -> Check<'a> {
    let mut needs = Vec::with_capacity(recipe.inputs.len());
    let mut inputs = Vec::new();
    let mut taken = quality::Inputs::default();
    for need in &recipe.inputs {
        let mut left = need.quantity;
        for item in input.items().filter(|i| i.item_type == need.item_type) {
            if left <= 0 {
                break;
            }
            let used = left.min(item.quantity);
            left -= used;
            taken.add(&item.item_type, used, item.quality);
            inputs.push(item.item_id);
        }
        needs.push(Need {
            item_type: need.item_type.clone(),
            item_name: need.item_name.clone(),
            have: count(input, &need.item_type),
            required: need.quantity,
        });
    }
    let satisfied = needs.iter().all(Need::met);
    let range = satisfied.then(|| {
        quality::output_range(&taken, station, 0, &recipe.quality_weights)
    });
    Check {
        recipe,
        needs,
        inputs,
        taken,
        range,
        blocked: output.and_then(|o| blocked(recipe, o)),
        station,
    }
}

/// What in `output` would leave one of the recipe's outputs nowhere to go:
/// a type already there merges, anything else takes a free slot.
fn blocked(recipe: &RecipeInfo, output: &Container) -> Option<Blocked> {
    let mut free = output.slot_count() - output.used_slots();
    for out in &recipe.outputs {
        if output.items().any(|i| i.item_type == out.item_type) {
            continue;
        }
        if free == 0 {
            let first = output.items().next()?;
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
/// slots run out. What's already in `input` counts.
pub fn fill(recipe: &RecipeInfo, from: &Container, input: &Container) -> Vec<Shift> {
    let mut empty = (0..input.slot_count()).filter(|&s| input.slot(s).is_none());
    let mut shifts = Vec::new();
    'needs: for need in &recipe.inputs {
        let mut left = need.quantity - count(input, &need.item_type);
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
/// the same type where there is one, else into an empty slot. Each target
/// is used once, so moves sent together can't race for a slot.
pub fn empty_into(from: &Container, to: &Container) -> Vec<Shift> {
    let mut taken = vec![false; to.slot_count()];
    let mut shifts = Vec::new();
    for item in from.items() {
        let onto = (0..to.slot_count())
            .find(|&s| !taken[s] && to.slot(s).is_some_and(|t| t.item_type == item.item_type))
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
        let rope = check(&r[0], &inp, None);
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
        let bench = check(&r[1], &inp, None);
        assert_eq!(bench.range, Some((40, 46)));
        assert_eq!(bench.inputs.len(), 3);

        // Short: no range, and not ready.
        let short = check(&r[1], &input(&[item(1, 0, "fiber", 90, 3)]), None);
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
        assert_eq!(check(&r[0], &inp, None).range, Some((48, 54)));
    }

    #[test]
    fn output_with_nowhere_to_go_blocks_the_craft() {
        let r = recipes();
        let inp = input(&[item(1, 0, "fiber", 50, 2)]);
        let out = |items: &[SlotItem]| boxed(OUT, "crafting_output", 1, items);

        // Empty: fine.
        assert!(check(&r[0], &inp, Some(&out(&[]))).ready());
        // Rope on rope merges.
        assert!(check(&r[0], &inp, Some(&out(&[item(9, 0, "rope", 70, 1)]))).ready());
        // A kit in the way: the rope would be lost.
        let c = check(&r[0], &inp, Some(&out(&[item(9, 0, "chest_kit", 70, 1)])));
        assert!(c.satisfied() && !c.ready());
        assert_eq!(
            c.blocked,
            Some(Blocked {
                item_id: Uuid::from_u128(9),
                item_name: "chest_kit".into(),
            })
        );
        // Not loaded yet: nothing to say.
        assert_eq!(check(&r[0], &inp, None).blocked, None);
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
            fill(&r[0], &inv, &input(&[])),
            [(Uuid::from_u128(3), IN, 0)]
        );
        // A workbench: 3 + 9; the slot already taken is skipped.
        assert_eq!(
            fill(&r[1], &inv, &input(&[item(7, 0, "rope", 50, 1)])),
            [(Uuid::from_u128(3), IN, 1), (Uuid::from_u128(4), IN, 2)]
        );
        // What's in the inputs already counts.
        assert!(fill(&r[0], &inv, &input(&[item(7, 2, "fiber", 50, 2)])).is_empty());
        // Out of slots: as far as it goes.
        let full = input(&[
            item(7, 0, "rope", 50, 1),
            item(8, 1, "rope", 50, 1),
            item(9, 2, "rope", 50, 1),
        ]);
        assert_eq!(fill(&r[1], &inv, &full), [(Uuid::from_u128(3), IN, 3)]);
    }

    #[test]
    fn emptying_merges_where_it_can_and_never_shares_a_target() {
        let inv = boxed(
            INV,
            "player_inventory",
            4,
            &[item(1, 0, "fiber", 40, 4), item(2, 2, "rope", 80, 1)],
        );
        let inp = input(&[
            item(5, 0, "fiber", 90, 3),
            item(6, 1, "fiber", 70, 2),
            item(7, 2, "chest_kit", 70, 1),
        ]);
        assert_eq!(
            empty_into(&inp, &inv),
            [
                (Uuid::from_u128(5), INV, 0), // onto the fiber
                (Uuid::from_u128(6), INV, 1), // that one's taken: an empty slot
                (Uuid::from_u128(7), INV, 3),
            ]
        );
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
            state.crafting_output(),
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
        let c = check_at(
            &forge,
            state.station_container().unwrap(),
            state.crafting_output(),
            90,
        );
        assert!(c.ready());
        // 0.56 × 70 + 0.24 × 50 + 0.2 × 90 (an A-grade bench) = 69.2.
        assert_eq!(c.range, Some((66, 72)));
        let points: f64 = c.breakdown().iter().map(quality::Term::points).sum();
        assert!((points - 69.2).abs() < 1e-9);
        // The same box at a D-grade bench comes out 10 points lower.
        let d = check_at(&forge, state.station_container().unwrap(), None, 40);
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
        let c = check(&r[0], state.crafting_input().unwrap(), None);

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
        let c = check(&r[0], state.crafting_input().unwrap(), None);
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
