//! How you've played each control mode (controls T3.1): numbers to sit
//! beside the feel when T3.2 decides between point-and-walk and over the
//! shoulder. Kept in `save/controls_stats.toml`, by server and player as
//! `save/gazette.toml` is, and nowhere else: they never leave your machine.
//!
//! For each mode: the minutes played in it, the distance walked (from the
//! server's snapshots of you, not the prediction), the interactions started
//! from a distance (walk there, then act) and those in reach, the times the
//! water or a slope stopped you (`movement_blocked`), the prediction's
//! corrections, the gathers, and the switches into it. The Settings panel's
//! Controls section shows them, and `--controls-report` prints them.
//!
//! Captures and offline runs keep none, so a recipe never writes the file.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::ControlMode;

/// A frame longer than this counts as this long: a window dragged, or a
/// machine asleep, isn't play.
const LONGEST_FRAME: f64 = 0.5;
/// A move between two frames longer than this is a jump (a reconnect, a
/// presence taken back), not a walk.
const LONGEST_STEP: f64 = 200.0;
/// How often the file is written while there's something new.
const SAVE_EVERY: Duration = Duration::from_secs(30);

/// One mode's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeStats {
    pub seconds: f64,
    /// In server units
    pub distance: f64,
    /// Walk there, then act
    pub acts_from_afar: u64,
    pub acts_in_reach: u64,
    pub blocked: u64,
    pub corrections: u64,
    pub gathers: u64,
    /// Switches into this mode, by F8 or the panel
    pub switched_to: u64,
}

impl ModeStats {
    pub fn minutes(&self) -> f64 {
        self.seconds / 60.0
    }

    /// `None` until there's a minute to go on.
    pub fn gathers_per_hour(&self) -> Option<f64> {
        (self.seconds >= 60.0).then(|| self.gathers as f64 * 3600.0 / self.seconds)
    }
}

/// Both modes' numbers, for one account.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Played {
    pub click: ModeStats,
    pub direct: ModeStats,
}

impl Played {
    fn mode_mut(&mut self, mode: ControlMode) -> &mut ModeStats {
        match mode {
            ControlMode::Click => &mut self.click,
            ControlMode::Direct => &mut self.direct,
        }
    }

    /// The report as rows: a name, then Click's and Direct's figures.
    pub fn rows(&self) -> Vec<(&'static str, String, String)> {
        let row = |name, f: &dyn Fn(&ModeStats) -> String| (name, f(&self.click), f(&self.direct));
        vec![
            row("Minutes", &|m| format!("{:.0}", m.minutes())),
            row("Walked", &|m| format!("{:.0}", m.distance)),
            row("Acts from afar", &|m| m.acts_from_afar.to_string()),
            row("Acts in reach", &|m| m.acts_in_reach.to_string()),
            row("Stopped", &|m| m.blocked.to_string()),
            row("Corrections", &|m| m.corrections.to_string()),
            row("Gathers", &|m| m.gathers.to_string()),
            row("Gathers an hour", &|m| m.gathers_per_hour().map_or("–".into(), |g| format!("{g:.0}"))),
            row("Switched to", &|m| m.switched_to.to_string()),
        ]
    }

    /// The rows as plain text, for `--controls-report`.
    pub fn report(&self) -> String {
        let mut out = format!("{:<16} {:>10} {:>10}\n", "", "click", "direct");
        for (name, click, direct) in self.rows() {
            out += &format!("{name:<16} {click:>10} {direct:>10}\n");
        }
        out
    }
}

/// What the session saw since the last frame took it: the interactions
/// you started, the blocks and the gathers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tally {
    pub from_afar: u32,
    pub in_reach: u32,
    pub blocked: u32,
    pub gathers: u32,
}

impl Tally {
    /// An interaction started: `walks` if it needs a walk there first.
    pub fn acted(&mut self, walks: bool) {
        if walks {
            self.from_afar += 1;
        } else {
            self.in_reach += 1;
        }
    }
}

/// One frame of play, as the session has it.
pub struct Frame<'a> {
    pub server: &'a str,
    /// `None` before you're signed in: nothing counts then.
    pub me: Option<Uuid>,
    /// Your last server position
    pub at: Option<(f64, f64)>,
    /// The prediction's corrections this session, a running count
    pub corrections: u32,
    pub tally: Tally,
    /// Seconds since the last frame
    pub dt: f64,
}

/// `save/controls_stats.toml` and the running count into it.
pub struct ControlsStats {
    path: PathBuf,
    accounts: BTreeMap<String, Played>,
    /// The account being played, by [`key`]
    account: Option<String>,
    last_at: Option<(f64, f64)>,
    last_corrections: u32,
    /// When there was first something unsaved
    dirty: Option<Instant>,
}

impl ControlsStats {
    pub const PATH: &'static str = "save/controls_stats.toml";

    /// Reads `path`. A missing or unreadable file starts at zero.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let accounts = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("controls stats: ignoring {}: {e}", path.display());
                BTreeMap::new()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                tracing::warn!("controls stats: ignoring {}: {e}", path.display());
                BTreeMap::new()
            }
        };
        Self {
            path,
            accounts,
            account: None,
            last_at: None,
            last_corrections: 0,
            dirty: None,
        }
    }

    /// Every account in the file, by "server player".
    pub fn accounts(&self) -> &BTreeMap<String, Played> {
        &self.accounts
    }

    /// The account being played, if anyone is signed in.
    pub fn current(&self) -> Option<&Played> {
        self.accounts.get(self.account.as_ref()?)
    }

    /// Count one frame of play in `mode`, then write the file if it's due.
    pub fn frame(&mut self, now: Instant, mode: ControlMode, frame: Frame) {
        let Some(me) = frame.me else {
            self.account = None;
            self.last_at = None;
            return;
        };
        let account = key(frame.server, me);
        if self.account.as_ref() != Some(&account) {
            // A new sign-in, or a new session: its corrections start again
            self.account = Some(account.clone());
            self.last_at = None;
            self.last_corrections = frame.corrections;
        }
        let corrections = match frame.corrections.checked_sub(self.last_corrections) {
            Some(n) => n,
            None => frame.corrections,
        };
        self.last_corrections = frame.corrections;
        let walked = match (self.last_at, frame.at) {
            (Some((x0, y0)), Some((x1, y1))) => Some((x1 - x0).hypot(y1 - y0)).filter(|d| *d <= LONGEST_STEP),
            _ => None,
        };
        if frame.at.is_some() {
            self.last_at = frame.at;
        }
        let m = self.accounts.entry(account).or_default().mode_mut(mode);
        m.seconds += frame.dt.clamp(0.0, LONGEST_FRAME);
        m.distance += walked.unwrap_or(0.0);
        m.corrections += u64::from(corrections);
        let t = frame.tally;
        m.acts_from_afar += u64::from(t.from_afar);
        m.acts_in_reach += u64::from(t.in_reach);
        m.blocked += u64::from(t.blocked);
        m.gathers += u64::from(t.gathers);
        let since = *self.dirty.get_or_insert(now);
        if now.duration_since(since) >= SAVE_EVERY {
            self.save();
        }
    }

    /// The mode just switched to `mode`, for whoever's signed in.
    pub fn switched(&mut self, mode: ControlMode) {
        if let Some(account) = self.account.clone() {
            self.accounts.entry(account).or_default().mode_mut(mode).switched_to += 1;
        }
    }

    /// Writes the file now, if there's anything new.
    pub fn save(&mut self) {
        if self.dirty.is_none() {
            return;
        }
        if let Some(dir) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!("controls stats: can't create {}: {e}", dir.display());
                return;
            }
        }
        let text = match toml::to_string(&self.accounts) {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!("controls stats: can't write them: {e}");
                return;
            }
        };
        // Written beside, then moved over, so a crash mid-write keeps the old.
        let tmp = self.path.with_extension("toml.tmp");
        match std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &self.path)) {
            Ok(()) => self.dirty = None,
            Err(e) => tracing::warn!("controls stats: can't write {}: {e}", self.path.display()),
        }
    }
}

impl Drop for ControlsStats {
    fn drop(&mut self) {
        self.save();
    }
}

/// An account's key in the file: the server, then the player id.
pub fn key(server: &str, me: Uuid) -> String {
    format!("{server} {me}")
}

/// `--controls-report`: every account in the file, or a line saying there
/// are none.
pub fn report(path: impl Into<PathBuf>) -> String {
    let path = path.into();
    let stats = ControlsStats::load(&path);
    if stats.accounts().is_empty() {
        return format!("No play recorded in {} yet.\n", path.display());
    }
    let mut out = String::new();
    for (account, played) in stats.accounts() {
        out += &format!("{account}\n{}\n", played.report());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("capitol-controls-stats-{}-{name}.toml", std::process::id()))
    }

    fn frame(me: Uuid, at: (f64, f64), corrections: u32, tally: Tally) -> Frame<'static> {
        Frame {
            server: "http://s",
            me: Some(me),
            at: Some(at),
            corrections,
            tally,
            dt: 0.1,
        }
    }

    #[test]
    fn the_stats_round_trip() {
        let path = temp("round-trip");
        let me = Uuid::from_u128(7);
        let now = Instant::now();
        {
            let mut stats = ControlsStats::load(&path);
            let mut tally = Tally::default();
            tally.acted(true);
            tally.acted(false);
            tally.blocked = 2;
            tally.gathers = 3;
            stats.frame(now, ControlMode::Direct, frame(me, (0.0, 0.0), 0, Tally::default()));
            stats.frame(now, ControlMode::Direct, frame(me, (3.0, 4.0), 1, tally));
            stats.switched(ControlMode::Click);
            // Written as it drops
        }
        let stats = ControlsStats::load(&path);
        let played = stats.accounts()[&key("http://s", me)];
        let d = played.direct;
        assert!((d.seconds - 0.2).abs() < 1e-9);
        assert_eq!(d.distance, 5.0);
        assert_eq!((d.acts_from_afar, d.acts_in_reach, d.blocked, d.gathers, d.corrections), (1, 1, 2, 3, 1));
        assert_eq!(played.click.switched_to, 1);
        assert_eq!(played.click.seconds, 0.0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn an_old_save_starts_at_zero() {
        // No file at all
        let stats = ControlsStats::load(temp("missing"));
        assert!(stats.accounts().is_empty());
        // A file from before a number was kept, and one that isn't ours
        let path = temp("old");
        std::fs::write(&path, "[\"http://s 7\".direct]\nseconds = 90.0\n").unwrap();
        let stats = ControlsStats::load(&path);
        let played = stats.accounts()["http://s 7"];
        assert_eq!(played.direct.seconds, 90.0);
        assert_eq!(played.direct.switched_to, 0);
        assert_eq!(played.click, ModeStats::default());
        std::fs::write(&path, "not = [toml").unwrap();
        assert!(ControlsStats::load(&path).accounts().is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn minutes_go_to_the_mode_in_use_across_a_switch() {
        let path = temp("switch");
        let me = Uuid::from_u128(9);
        let now = Instant::now();
        let mut stats = ControlsStats::load(&path);
        for _ in 0..30 {
            stats.frame(now, ControlMode::Click, frame(me, (0.0, 0.0), 0, Tally::default()));
        }
        stats.switched(ControlMode::Direct);
        for _ in 0..10 {
            stats.frame(now, ControlMode::Direct, frame(me, (0.0, 0.0), 0, Tally::default()));
        }
        let played = *stats.current().unwrap();
        assert!((played.click.seconds - 3.0).abs() < 1e-9);
        assert!((played.direct.seconds - 1.0).abs() < 1e-9);
        assert_eq!((played.click.switched_to, played.direct.switched_to), (0, 1));
        drop(stats);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_jump_isnt_walked_and_a_new_session_counts_its_corrections_afresh() {
        let me = Uuid::from_u128(3);
        let now = Instant::now();
        let path = temp("jump");
        let mut stats = ControlsStats::load(&path);
        stats.frame(now, ControlMode::Click, frame(me, (0.0, 0.0), 5, Tally::default()));
        stats.frame(now, ControlMode::Click, frame(me, (1000.0, 0.0), 6, Tally::default()));
        stats.frame(now, ControlMode::Click, frame(me, (1010.0, 0.0), 2, Tally::default()));
        let click = stats.current().unwrap().click;
        assert_eq!(click.distance, 10.0);
        // 5 was the session's own before; then 1, then a new count of 2
        assert_eq!(click.corrections, 3);
        // Signed out, nothing counts
        let mut out = frame(me, (0.0, 0.0), 0, Tally::default());
        out.me = None;
        stats.frame(now, ControlMode::Click, out);
        assert!(stats.current().is_none());
        drop(stats);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn gathers_an_hour_wait_for_a_minute() {
        let mut m = ModeStats { seconds: 30.0, gathers: 5, ..Default::default() };
        assert_eq!(m.gathers_per_hour(), None);
        m.seconds = 1800.0;
        assert_eq!(m.gathers_per_hour(), Some(10.0));
        assert!(Played::default().report().contains("Gathers an hour"));
    }
}
