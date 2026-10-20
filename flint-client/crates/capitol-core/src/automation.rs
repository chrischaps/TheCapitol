//! A station's automation as a player sees it (E4.4): where it stands, a
//! pause in plain words with what fixes it, and the form that sets it up.
//! The server's side is E4.2 and E4.3; this only reads what it sends
//! (`StationInfo.automation`, `station_automation_changed`) and says what to
//! send back.

use crate::catalog::Catalog;
use capitol_protocol::rest::{AutomationMode, AutomationState, PauseReason, RecipeInfo, StationAutomation};
use capitol_protocol::ws::ClientMessage;
use uuid::Uuid;

/// The most a count can ask for: the server's `automation::MAX_COUNT`.
pub const MAX_COUNT: u32 = 10_000;

/// Where a station's automation stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Never set up.
    Unset,
    /// Set up, and stopped by its owner (or never started).
    Stopped,
    /// A count that made all it was asked for. The server stops it.
    Finished,
    Running,
    /// It means to run but can't, for this reason. It goes on by itself
    /// once that clears.
    Paused(PauseReason),
}

impl Status {
    pub fn of(automation: Option<&StationAutomation>) -> Status {
        let Some(a) = automation else {
            return Status::Unset;
        };
        match &a.state {
            AutomationState::Running => Status::Running,
            AutomationState::Paused { reason } => Status::Paused(reason.clone()),
            AutomationState::Stopped if a.is_finished() => Status::Finished,
            AutomationState::Stopped => Status::Stopped,
        }
    }
}

/// How a station looks in the world: lit and working, dimmed and waiting,
/// or as it always is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Work {
    #[default]
    Idle,
    Running,
    Paused,
}

impl Work {
    pub fn of(automation: Option<&StationAutomation>) -> Work {
        match automation.map(|a| &a.state) {
            Some(AutomationState::Running) => Work::Running,
            Some(AutomationState::Paused { .. }) => Work::Paused,
            _ => Work::Idle,
        }
    }
}

/// A pause, said plainly: what's wrong, and what puts it right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Words {
    /// "Out of iron ore"
    pub what: String,
    /// "Put more iron ore in its box"
    pub fix: String,
}

/// `reason` in words. `recipe` is the one it runs, for the tool it wants.
pub fn pause_words(reason: &PauseReason, catalog: &Catalog, recipe: Option<&RecipeInfo>) -> Words {
    let tool = || {
        recipe
            .and_then(|r| r.tool_type.as_deref())
            .map_or_else(|| "tool".to_owned(), |t| t.replace('_', " "))
    };
    let (what, fix) = match reason {
        PauseReason::MissingInput { item_type } => {
            let name = catalog.item_name(item_type).to_lowercase();
            (format!("Out of {name}"), format!("Put more {name} in its box"))
        }
        PauseReason::OutputFull => (
            "Its box is full".to_owned(),
            "Take out some of what it made".to_owned(),
        ),
        PauseReason::ToolBroken => {
            let tool = tool();
            (format!("Its {tool} broke"), format!("Put another {tool} in its tool slot"))
        }
        PauseReason::ToolMissing => {
            let tool = tool();
            (
                format!("It has no {tool}"),
                format!("Put {} {tool} in its tool slot", article(&tool)),
            )
        }
        PauseReason::OwnerOffline => (
            "Waiting for you".to_owned(),
            "It goes on when you're back. A subscription keeps it working while you're away"
                .to_owned(),
        ),
    };
    Words { what, fix }
}

fn article(word: &str) -> &'static str {
    match word.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}

/// How far a run has got: "12 of 50", or "12 made" when it runs forever.
pub fn progress(a: &StationAutomation) -> String {
    match a.mode {
        AutomationMode::Count(n) => format!("{} of {n}", a.done),
        AutomationMode::Forever => format!("{} made", a.done),
    }
}

/// The automation form: what to make and for how long.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub recipe_id: Option<String>,
    pub forever: bool,
    /// The count, kept while `forever` is on so switching back finds it.
    pub count: u32,
}

impl Default for Draft {
    fn default() -> Self {
        Draft {
            recipe_id: None,
            forever: false,
            count: 10,
        }
    }
}

impl Draft {
    /// The form as the station is set, or the defaults with `recipe` (the
    /// recipe the box makes, say) when it was never set up.
    pub fn from(automation: Option<&StationAutomation>, recipe: Option<&str>) -> Draft {
        match automation {
            Some(a) => Draft {
                recipe_id: Some(a.recipe_id.clone()),
                forever: a.mode == AutomationMode::Forever,
                count: match a.mode {
                    AutomationMode::Count(n) => n,
                    AutomationMode::Forever => Draft::default().count,
                },
            },
            None => Draft {
                recipe_id: recipe.map(str::to_owned),
                ..Draft::default()
            },
        }
    }

    pub fn mode(&self) -> AutomationMode {
        if self.forever {
            AutomationMode::Forever
        } else {
            AutomationMode::Count(self.count.clamp(1, MAX_COUNT))
        }
    }

    /// The form says something other than what the station is set to.
    pub fn differs(&self, automation: Option<&StationAutomation>) -> bool {
        match automation {
            None => true,
            Some(a) => self.recipe_id.as_deref() != Some(a.recipe_id.as_str()) || self.mode() != a.mode,
        }
    }

    /// What Start sends: the setting first when the form changed it (the
    /// server stops the station and counts from 0 again), then the start.
    /// Nothing until a recipe is picked.
    pub fn start(&self, station: Uuid, automation: Option<&StationAutomation>) -> Vec<ClientMessage> {
        let Some(recipe_id) = &self.recipe_id else {
            return Vec::new();
        };
        let station_id = station.to_string();
        let mut out = Vec::with_capacity(2);
        if self.differs(automation) {
            out.push(ClientMessage::ConfigureAutomation {
                station_id: station_id.clone(),
                recipe_id: recipe_id.clone(),
                mode: self.mode(),
            });
        }
        out.push(ClientMessage::StartAutomation { station_id });
        out
    }
}

/// The commands a refusal of which belongs to the automation tab.
pub fn is_automation_command(command: &str) -> bool {
    matches!(
        command,
        "configure_automation" | "start_automation" | "stop_automation"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(mode: AutomationMode, done: u32) -> StationAutomation {
        StationAutomation {
            recipe_id: "smelt_iron".into(),
            mode,
            state: AutomationState::Running,
            done,
        }
    }

    #[test]
    fn a_finished_count_reads_as_finished_not_stopped() {
        let mut a = running(AutomationMode::Count(5), 5);
        a.state = AutomationState::Stopped;
        assert_eq!(Status::of(Some(&a)), Status::Finished);
        a.done = 3;
        assert_eq!(Status::of(Some(&a)), Status::Stopped);
        assert_eq!(Status::of(None), Status::Unset);
        a.state = AutomationState::Paused { reason: PauseReason::OutputFull };
        assert_eq!(Work::of(Some(&a)), Work::Paused);
        assert_eq!(Work::of(None), Work::Idle);
    }

    #[test]
    fn every_pause_says_what_fixes_it() {
        let catalog = Catalog::fallback();
        let mut recipe = catalog.recipes().first().cloned().expect("a recipe");
        recipe.tool_type = Some("hammer".into());
        let reasons = [
            PauseReason::MissingInput { item_type: "fiber".into() },
            PauseReason::OutputFull,
            PauseReason::ToolBroken,
            PauseReason::ToolMissing,
            PauseReason::OwnerOffline,
        ];
        for reason in &reasons {
            let w = pause_words(reason, &catalog, Some(&recipe));
            assert!(!w.what.is_empty() && !w.fix.is_empty(), "{reason:?}");
        }
        let w = pause_words(&reasons[3], &catalog, Some(&recipe));
        assert_eq!(w.fix, "Put a hammer in its tool slot");
        let w = pause_words(&reasons[0], &catalog, None);
        assert!(w.what.starts_with("Out of "), "{}", w.what);
    }

    #[test]
    fn start_sets_it_up_only_when_the_form_changed_it() {
        let station = Uuid::from_u128(7);
        let set = running(AutomationMode::Count(10), 4);
        let draft = Draft::from(Some(&set), None);
        assert!(!draft.differs(Some(&set)));
        assert!(matches!(
            draft.start(station, Some(&set)).as_slice(),
            [ClientMessage::StartAutomation { .. }]
        ));
        let forever = Draft { forever: true, ..draft.clone() };
        assert!(matches!(
            forever.start(station, Some(&set)).as_slice(),
            [
                ClientMessage::ConfigureAutomation { mode: AutomationMode::Forever, .. },
                ClientMessage::StartAutomation { .. }
            ]
        ));
        // Switching back finds the count it had
        assert_eq!(Draft { forever: false, ..forever }.mode(), AutomationMode::Count(10));
        assert!(Draft::default().start(station, None).is_empty());
        let fresh = Draft::from(None, Some("plank"));
        assert_eq!(fresh.start(station, None).len(), 2);
    }

    #[test]
    fn a_count_stays_in_the_servers_range() {
        let d = |count| Draft { recipe_id: None, forever: false, count };
        assert_eq!(d(0).mode(), AutomationMode::Count(1));
        assert_eq!(d(99_999).mode(), AutomationMode::Count(MAX_COUNT));
        assert_eq!(progress(&running(AutomationMode::Count(50), 12)), "12 of 50");
        assert_eq!(progress(&running(AutomationMode::Forever, 3)), "3 made");
    }
}
