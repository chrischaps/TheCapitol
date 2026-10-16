//! The player's settings (P8.4): how the world is drawn, how loud it is,
//! and how the camera answers. They're kept in `save/settings.toml`
//! ([`Settings::PATH`]), written a moment after each change, and applied
//! live: the graphics to Flint's renderer ([`Graphics::apply`]), the
//! volumes to the soundscape (`sound::Soundscape::set_volumes`) and the
//! camera to `scripts/camera.rhai` through `cap_setting`.
//!
//! The server URL isn't here. It belongs with the session, in
//! `save/session.toml`, because a token is only good on its own server.
//! The settings panel changes it the way the login form does.
//!
//! A file of its own, not the engine's `save/persist.toml`, because
//! extensions can't reach that store (issue #71). Scripts read what they
//! need through `cap_setting`.
//!
//! A missing or broken file is the defaults, and any field it lacks or gets
//! wrong is that field's default, so an old file or a hand edit never stops
//! the game.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long after the last change the file is written, so dragging a
/// slider doesn't write it every frame.
const SAVE_AFTER: Duration = Duration::from_millis(600);

/// The slowest and fastest the camera turns and zooms, as a multiple of the
/// rig's own rates.
pub const SENSITIVITY: std::ops::RangeInclusive<f32> = 0.25..=3.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub graphics: Graphics,
    pub audio: Volumes,
    pub camera: CameraFeel,
}

/// How the world is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Graphics {
    /// Screen-space ambient occlusion: the soft dark where walls meet.
    pub ssao: bool,
    /// The glow round the tower's lantern and the forge's fire.
    pub bloom: bool,
    pub shadows: Shadows,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            ssao: true,
            bloom: true,
            shadows: Shadows::Medium,
        }
    }
}

/// The sun's shadow map: off, or its size on a side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Shadows {
    Off,
    Low,
    /// Flint's own size.
    #[default]
    Medium,
    High,
}

impl Shadows {
    pub const ALL: [Shadows; 4] = [Shadows::Off, Shadows::Low, Shadows::Medium, Shadows::High];

    /// Texels on a side, or `None` for no shadows.
    pub fn resolution(self) -> Option<u32> {
        match self {
            Shadows::Off => None,
            Shadows::Low => Some(1024),
            Shadows::Medium => Some(2048),
            Shadows::High => Some(4096),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Shadows::Off => "Off",
            Shadows::Low => "Low",
            Shadows::Medium => "Medium",
            Shadows::High => "High",
        }
    }
}

/// How loud each part of the sound is, 0 to 1. `master` scales the rest.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Volumes {
    pub master: f32,
    /// The beds: city hum, breeze, wind, water.
    pub ambience: f32,
    /// The world's one-shots: gathers, crafts, birds, the bell.
    pub effects: f32,
    /// Clicks and drops in the panels.
    pub interface: f32,
}

impl Default for Volumes {
    fn default() -> Self {
        Self {
            master: 1.0,
            ambience: 1.0,
            effects: 1.0,
            interface: 1.0,
        }
    }
}

impl Volumes {
    /// What a bed's level is multiplied by.
    pub fn ambience_gain(&self) -> f32 {
        self.master * self.ambience
    }

    /// What a one-shot's level is multiplied by: an interface sound's, or
    /// the world's.
    pub fn effect_gain(&self, interface: bool) -> f32 {
        self.master
            * if interface {
                self.interface
            } else {
                self.effects
            }
    }
}

/// How the camera rig answers you.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraFeel {
    /// Turning and zooming, as a multiple of the rig's rates.
    pub sensitivity: f32,
    /// Q/E and a right-drag turn the other way.
    pub invert_orbit: bool,
}

impl Default for CameraFeel {
    fn default() -> Self {
        Self {
            sensitivity: 1.0,
            invert_orbit: false,
        }
    }
}

impl Settings {
    pub const PATH: &str = "save/settings.toml";

    /// Every number in range, and a number for anything that isn't one.
    pub fn clamped(mut self) -> Self {
        let unit = |v: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                1.0
            }
        };
        let a = &mut self.audio;
        a.master = unit(a.master);
        a.ambience = unit(a.ambience);
        a.effects = unit(a.effects);
        a.interface = unit(a.interface);
        let c = &mut self.camera;
        c.sensitivity = if c.sensitivity.is_finite() {
            c.sensitivity
                .clamp(*SENSITIVITY.start(), *SENSITIVITY.end())
        } else {
            1.0
        };
        self
    }

    /// Reads TOML, field by field: anything missing or wrong is its default.
    pub fn parse(text: &str) -> Self {
        let Ok(table) = text.parse::<toml::Table>() else {
            return Self::default();
        };
        let section = |name: &str| {
            table
                .get(name)
                .cloned()
                .unwrap_or(toml::Value::Table(Default::default()))
        };
        Settings {
            graphics: lenient(section("graphics")),
            audio: lenient(section("audio")),
            camera: lenient(section("camera")),
        }
        .clamped()
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

/// A section as written, or, if any field won't read, the section with
/// just the fields that do.
fn lenient<T: Default + Serialize + for<'de> Deserialize<'de>>(value: toml::Value) -> T {
    if let Ok(t) = value.clone().try_into() {
        return t;
    }
    let mut out =
        toml::Value::try_from(T::default()).unwrap_or(toml::Value::Table(Default::default()));
    if let (toml::Value::Table(given), toml::Value::Table(fields)) = (value, &mut out) {
        for (key, v) in given {
            let Some(old) = fields.get(&key).cloned() else {
                continue;
            };
            fields.insert(key.clone(), v);
            if toml::Value::Table(fields.clone()).try_into::<T>().is_err() {
                fields.insert(key, old);
            }
        }
    }
    out.try_into().unwrap_or_default()
}

/// The settings and the file they're kept in.
pub struct SettingsFile {
    path: PathBuf,
    pub settings: Settings,
    /// What's on disk, to tell whether there's anything to write.
    saved: Settings,
    /// When the settings last changed from what's saved.
    changed: Option<Instant>,
}

impl SettingsFile {
    /// Reads `path`. A missing or unreadable file is the defaults.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let settings = match std::fs::read_to_string(&path) {
            Ok(text) => Settings::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                tracing::warn!("settings: ignoring {}: {e}", path.display());
                Settings::default()
            }
        };
        Self {
            path,
            saved: settings.clone(),
            settings,
            changed: None,
        }
    }

    /// Call once a frame: writes the file once the settings have held still
    /// for a moment.
    pub fn update(&mut self, now: Instant) {
        if self.settings == self.saved {
            self.changed = None;
            return;
        }
        let since = *self.changed.get_or_insert(now);
        if now.duration_since(since) >= SAVE_AFTER {
            self.save();
        }
    }

    /// Writes the file now, if anything changed.
    pub fn save(&mut self) {
        self.settings = self.settings.clone().clamped();
        if self.settings == self.saved {
            return;
        }
        if let Some(dir) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!("settings: can't create {}: {e}", dir.display());
                return;
            }
        }
        // Written beside, then moved over, so a crash mid-write keeps the old.
        let tmp = self.path.with_extension("toml.tmp");
        let written = std::fs::write(&tmp, self.settings.to_toml())
            .and_then(|()| std::fs::rename(&tmp, &self.path));
        match written {
            Ok(()) => {
                tracing::info!("settings: saved {}", self.path.display());
                self.saved = self.settings.clone();
                self.changed = None;
            }
            Err(e) => tracing::warn!("settings: can't write {}: {e}", self.path.display()),
        }
    }
}

impl Drop for SettingsFile {
    fn drop(&mut self) {
        self.save();
    }
}

impl Graphics {
    /// Make the renderer draw as these say. Only what differs from
    /// `before` is touched, and the shadow map is only made again when its
    /// size changes.
    pub fn apply(&self, before: Option<&Graphics>, gpu: &mut flint_player::ExtensionGpu) {
        let renderer = &mut *gpu.renderer;
        if before.map(|b| (b.ssao, b.bloom)) != Some((self.ssao, self.bloom)) {
            let mut config = renderer.post_process_config().clone();
            config.ssao_enabled = self.ssao;
            config.bloom_enabled = self.bloom;
            renderer.set_post_process_config(config);
        }
        if before.map(|b| b.shadows) != Some(self.shadows) {
            match self.shadows.resolution() {
                None => renderer.set_shadows(false),
                Some(res) => {
                    if renderer.shadow_resolution() != res {
                        renderer.set_shadow_resolution(gpu.device, res);
                    }
                    renderer.set_shadows(true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_toml() {
        let s = Settings {
            graphics: Graphics {
                ssao: false,
                bloom: true,
                shadows: Shadows::High,
            },
            audio: Volumes {
                master: 0.5,
                ambience: 0.25,
                effects: 0.75,
                interface: 0.0,
            },
            camera: CameraFeel {
                sensitivity: 1.5,
                invert_orbit: true,
            },
        };
        assert_eq!(Settings::parse(&s.to_toml()), s);
    }

    #[test]
    fn a_missing_or_broken_file_is_the_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert_eq!(Settings::parse("this isn't = = toml"), Settings::default());
    }

    #[test]
    fn a_bad_field_falls_back_alone() {
        let s = Settings::parse(
            "[graphics]\nssao = false\nshadows = \"enormous\"\n[audio]\nmaster = \"loud\"\neffects = 0.3\n[camera]\nsensitivity = 40.0\n",
        );
        assert!(!s.graphics.ssao);
        assert_eq!(s.graphics.shadows, Shadows::Medium);
        assert_eq!(s.audio.master, 1.0);
        assert_eq!(s.audio.effects, 0.3);
        assert_eq!(s.camera.sensitivity, *SENSITIVITY.end());
    }

    #[test]
    fn the_file_is_written_once_the_settings_settle() {
        let dir = std::env::temp_dir().join(format!("capitol-settings-{}", std::process::id()));
        let path = dir.join("settings.toml");
        let _ = std::fs::remove_file(&path);
        let t0 = Instant::now();
        {
            let mut file = SettingsFile::load(&path);
            assert_eq!(file.settings, Settings::default());
            file.settings.audio.master = 0.4;
            file.update(t0);
            assert!(!path.exists(), "written while still changing");
            file.update(t0 + SAVE_AFTER);
            assert!(path.exists());
            file.settings.camera.invert_orbit = true;
            // Dropped before it settles: written on the way out.
        }
        let file = SettingsFile::load(&path);
        assert_eq!(file.settings.audio.master, 0.4);
        assert!(file.settings.camera.invert_orbit);
        drop(file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gains_multiply_through_master() {
        let v = Volumes {
            master: 0.5,
            ambience: 0.5,
            effects: 1.0,
            interface: 0.0,
        };
        assert_eq!(v.ambience_gain(), 0.25);
        assert_eq!(v.effect_gain(false), 0.5);
        assert_eq!(v.effect_gain(true), 0.0);
    }
}
