//! What you hear (P8.1). The sounds are made in [`library`] at start, off
//! the main thread, and handed to Flint from memory (`ctx.audio`, Flint
//! ADR 0084). [`Soundscape`] plays them: four looping beds whose levels
//! follow where you are, and one-shots it plays itself.
//!
//! **The beds** are `audio_source` entities on the music bus, spawned once
//! and never stopped; only their `volume` moves. [`mix`] sets it from how
//! far from the centre you are, how near the water, and how high the
//! camera is:
//! - city hum fades out across the Guild District and Urban,
//! - the breeze comes up through Urban, holds over the suburbs and the
//!   fields, and goes as the fields run out,
//! - wind takes over past the Rural edge,
//! - water laps within 25 m of a ring's edge, wherever that is,
//! - and climbing to the map lifts you out of all of it into a thin wind.
//!
//! The zone beds share one constant power, so a crossfade never dips or
//! swells, and their levels ease toward the mix over about a second so a
//! jump (a zoom, a sign-in) settles rather than snaps.
//!
//! **The volumes** (P8.4) are Flint's buses: the beds are on the music bus,
//! set to master × ambience, and the one-shots on the sfx bus, set to
//! master, each scaled by the effects or interface level.
//!
//! **One-shots** are played straight from `pre_update`. Something in the
//! world (a bird, a rustle) sounds where it is, and Flint pans it and fades
//! it with its distance from the camera, the listener. Its falloff is sized
//! to the rig (#94): full within [`NEAR`], about where the street camera
//! stands from you, then down like a sound in the open, within 1.5 dB
//! of `(32/d)^1.2` out to 300 m, the curve this soundscape was first tuned
//! by ear with. Past that it goes quicker, so the map view hears none of it.

pub mod dsp;
pub mod library;

use crate::settings::Volumes;
use dsp::{smoothstep, Rng};
use flint_core::{EntityId, Vec3};
use flint_player::{Bus, ExtensionCtx, SoundOptions};
use std::thread::JoinHandle;

/// Overall level of the beds, and of the one-shots.
const AMBIENCE: f32 = 0.8;
const EFFECTS: f32 = 1.0;

/// Each bed's level at full weight, by ear.
const BEDS: [(&str, &str, f32); 4] = [
    ("ambience_city", "amb_city", 0.55),
    ("ambience_breeze", "amb_breeze", 0.50),
    ("ambience_wind", "amb_wind", 0.50),
    ("ambience_water", "amb_water", 0.45),
];

/// How fast a bed's level follows the mix: a time constant, seconds.
const EASE: f32 = 0.8;

/// Something in the world is this many metres from the camera when it
/// sounds at full level, about where the street camera stands from you.
const NEAR: f32 = 32.0;

/// A placed one-shot is silent this far from the camera, metres.
const FAR: f32 = 1250.0;

/// Flint's falloff curve between [`NEAR`] and [`FAR`]
/// (`SoundOptions::rolloff`): fitted to `(NEAR/d)^1.2`.
const ROLLOFF: f32 = 0.6;

/// The beds' weights at one place and height. Each 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Mix {
    pub city: f32,
    pub breeze: f32,
    pub wind: f32,
    pub water: f32,
    /// How often birds call, 0 to 1. Not a bed: the soundscape scatters
    /// one-shots at this rate.
    pub birds: f32,
}

impl Mix {
    fn bed(&self, i: usize) -> f32 {
        [self.city, self.breeze, self.wind, self.water][i]
    }
}

/// The mix at `r` server units from the centre, `shore` units from the
/// nearest water's edge (0 over the water) and with the camera `height`
/// metres above the ground.
pub fn mix(r: f32, shore: f32, height: f32) -> Mix {
    let city = 1.0 - smoothstep(500.0, 1700.0, r);
    let breeze = smoothstep(900.0, 1800.0, r) * (1.0 - smoothstep(3000.0, 3600.0, r));
    let wind = smoothstep(2900.0, 3700.0, r);
    // Equal power across the three, whatever their sum.
    let sum = (city + breeze + wind).max(1e-6);
    let ground = 1.0 - smoothstep(80.0, 450.0, height);
    let air = 1.0 - ground;
    let birds = smoothstep(1000.0, 1900.0, r) * (1.0 - smoothstep(3000.0, 3600.0, r));
    Mix {
        city: (city / sum).sqrt() * ground,
        breeze: (breeze / sum).sqrt() * ground,
        // High over the map, a thin wind wherever you are.
        wind: ((wind / sum).sqrt() * ground).max(0.45 * air),
        water: (1.0 - smoothstep(0.0, 250.0, shore.max(0.0))) * ground,
        birds: birds * ground,
    }
}

/// A one-shot to play.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    /// Its name in [`library::RECIPES`].
    pub sound: String,
    pub volume: f32,
    /// Where to play it, world metres, or `None` for no place at all.
    pub at: Option<[f32; 3]>,
    pub pitch: f32,
    /// A panel's sound, not the world's: the interface volume sets it.
    pub interface: bool,
}

impl Cue {
    fn flat(sound: &str, volume: f32) -> Self {
        Self {
            sound: sound.to_owned(),
            volume: volume * EFFECTS,
            at: None,
            pitch: 1.0,
            interface: false,
        }
    }

    fn interface(self) -> Self {
        Self {
            interface: true,
            ..self
        }
    }

    /// How Flint plays it: on the sfx bus, placed or flat.
    pub fn options(&self) -> SoundOptions {
        let o = SoundOptions::new()
            .volume(self.volume as f64)
            .pitch(self.pitch as f64)
            .bus(Bus::Sfx);
        match self.at {
            Some([x, y, z]) => o
                .at(Vec3::new(x, y, z))
                .distances(NEAR, FAR)
                .rolloff(ROLLOFF),
            None => o,
        }
    }
}

/// What the world is doing that the soundscape listens to, this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ears {
    /// Where you are (or the camera looks, offline), world metres (X, Z).
    pub focus: [f32; 2],
    /// The camera, world metres.
    pub camera: [f32; 3],
    /// The camera's height above the ground, metres.
    pub height: f32,
}

/// One of the things that rustles: a tuft being worked.
#[derive(Debug, Clone)]
pub struct Rustling {
    pub id: String,
    /// World metres (X, Z).
    pub at: [f32; 2],
    /// You're the one working it.
    pub mine: bool,
}

pub struct Soundscape {
    muted: bool,
    /// The player's volumes (P8.4), and what the buses were last set to.
    volumes: Volumes,
    buses: Option<(f64, f64)>,
    /// The library being made, from the first `apply`; `None` once it's
    /// handed to Flint.
    making: Option<JoinHandle<Vec<(&'static str, library::Channels)>>>,
    /// The library is in Flint's cache, so the soundscape can play.
    loaded: bool,
    /// The bed entities, spawned on the first update.
    beds: Option<[EntityId; 4]>,
    /// Each bed's level now, easing toward the mix.
    levels: [f32; 4],
    /// What was last written to each bed's `audio_source.volume`.
    written: [f32; 4],
    /// The rings, server units: (radius, width).
    rings: Vec<(f32, f32)>,
    rng: Rng,
    /// Game time, seconds.
    now: f64,
    next_bird: f64,
    last_bird: usize,
    next_bell: f64,
    /// When each worked tuft next rustles.
    rustles: Vec<(String, f64)>,
    cues: Vec<Cue>,
    mix: Mix,
}

impl Soundscape {
    pub fn new(muted: bool) -> Self {
        Self {
            muted,
            volumes: Volumes::default(),
            buses: None,
            making: None,
            loaded: false,
            beds: None,
            levels: [0.0; 4],
            written: [-1.0; 4],
            rings: Vec::new(),
            rng: Rng::new(0xC0FFEE),
            now: 0.0,
            next_bird: 2.0,
            last_bird: 0,
            next_bell: 25.0,
            rustles: Vec::new(),
            cues: Vec::new(),
            mix: Mix::default(),
        }
    }

    pub fn set_rings(&mut self, rings: &[capitol_core::protocol::terrain::WaterRing]) {
        self.rings = rings
            .iter()
            .map(|r| (r.radius as f32, r.width as f32))
            .collect();
    }

    /// Set the player's volumes: the buses take them on the next `apply`.
    pub fn set_volumes(&mut self, volumes: Volumes) {
        self.volumes = volumes;
    }

    /// A button pressed in the UI.
    pub fn click(&mut self) {
        self.cue(Cue::flat("ui_click", 0.35).interface());
    }

    /// Something dropped into a slot.
    pub fn drop_in_slot(&mut self) {
        self.cue(Cue::flat("ui_drop", 0.4).interface());
    }

    /// A gather landed.
    pub fn gathered(&mut self) {
        self.cue(Cue::flat("pluck", 0.5));
    }

    /// A craft finished.
    pub fn crafted(&mut self) {
        self.cue(Cue::flat("chime", 0.55));
    }

    /// A plot is yours: the tower rings for you.
    pub fn claimed(&mut self) {
        self.cue(Cue::flat("bell", 0.6));
    }

    fn cue(&mut self, c: Cue) {
        if !self.muted {
            self.cues.push(c);
        }
    }

    /// Advance by `dt` seconds: move the beds toward the mix for `ears`,
    /// and decide which birds, rustles and bells sound. Returns the cues
    /// to play, the UI's and the game's among them, scaled by the effects
    /// or interface volume (master is the sfx bus's).
    pub fn update(&mut self, ears: Ears, working: &[Rustling], dt: f32) -> Vec<Cue> {
        self.now += dt as f64;
        let [sx, sy] = crate::coords::to_server(ears.focus[0], ears.focus[1]);
        let (dx, dy) = (
            sx as f32 - capitol_core::protocol::rules::CENTER_X as f32,
            sy as f32 - capitol_core::protocol::rules::CENTER_Y as f32,
        );
        let r = dx.hypot(dy);
        let shore = self
            .rings
            .iter()
            .map(|(radius, width)| (r - radius).abs() - width / 2.0)
            .fold(f32::INFINITY, f32::min);
        self.mix = mix(r, shore, ears.height);
        let k = 1.0 - (-dt / EASE).exp();
        for (i, level) in self.levels.iter_mut().enumerate() {
            let target = if self.muted {
                0.0
            } else {
                self.mix.bed(i) * BEDS[i].2 * AMBIENCE
            };
            *level += (target - *level) * k;
        }
        if !self.muted {
            self.birds(ears);
            self.bells();
            self.rustle(ears, working);
        }
        let volumes = self.volumes;
        let mut cues = std::mem::take(&mut self.cues);
        for c in &mut cues {
            c.volume *= if c.interface {
                volumes.interface
            } else {
                volumes.effects
            };
        }
        cues.retain(|c| c.volume > 0.0);
        cues
    }

    /// The music and sfx buses' volumes for the player's settings.
    pub fn bus_volumes(&self) -> (f64, f64) {
        let v = self.volumes;
        ((v.master * v.ambience) as f64, v.master as f64)
    }

    /// Hand the library to Flint once it's made, set the buses, play
    /// `cues`, spawn the beds once, then write any level that has moved.
    /// Until the library is in, nothing plays.
    pub fn apply(&mut self, ctx: &mut ExtensionCtx, cues: &[Cue]) {
        if self.muted || !self.load(ctx) {
            return;
        }
        let buses = self.bus_volumes();
        if self.buses != Some(buses) {
            ctx.audio.set_bus_volume(Bus::Music, buses.0);
            ctx.audio.set_bus_volume(Bus::Sfx, buses.1);
            self.buses = Some(buses);
        }
        for cue in cues {
            tracing::debug!("sound: {} at {:.2}", cue.sound, cue.volume);
            if let Err(e) = ctx.audio.play(&cue.sound, &cue.options()) {
                tracing::warn!("sound: {e:?}");
            }
        }
        let beds = match self.beds {
            Some(b) => b,
            None => {
                let mut ids = Vec::new();
                for (name, file, _) in BEDS {
                    let Ok(id) = ctx.world.spawn(name) else {
                        tracing::warn!("sound: couldn't spawn {name}");
                        return;
                    };
                    let mut src = toml::Table::new();
                    src.insert("file".into(), file.into());
                    src.insert("spatial".into(), false.into());
                    src.insert("loop".into(), true.into());
                    src.insert("autoplay".into(), true.into());
                    src.insert("volume".into(), 0.0.into());
                    src.insert("bus".into(), "music".into());
                    if let Err(e) = ctx.world.set_component(id, "audio_source", src.into()) {
                        tracing::warn!("sound: {name}: {e:?}");
                    }
                    ids.push(id);
                }
                let beds = [ids[0], ids[1], ids[2], ids[3]];
                self.beds = Some(beds);
                beds
            }
        };
        for (i, id) in beds.iter().enumerate() {
            let v = self.levels[i];
            if (v - self.written[i]).abs() < 0.002 {
                continue;
            }
            let mut t = toml::Table::new();
            t.insert("volume".into(), (v as f64).into());
            let _ = ctx.world.merge_component(*id, "audio_source", t.into());
            self.written[i] = v;
        }
    }

    /// Start making the library on the first call, and load it into
    /// Flint's cache once it's made. Returns whether it's loaded.
    fn load(&mut self, ctx: &mut ExtensionCtx) -> bool {
        if self.loaded {
            return true;
        }
        let making = self
            .making
            .get_or_insert_with(|| std::thread::spawn(library::make_all));
        if !making.is_finished() {
            return false;
        }
        let Some(making) = self.making.take() else {
            return false;
        };
        let started = std::time::Instant::now();
        for (name, channels) in making.join().unwrap_or_default() {
            let channels: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
            if let Err(e) = ctx.audio.load_samples(name, dsp::SR, &channels) {
                tracing::warn!("sound: {name}: {e:?}");
            }
        }
        tracing::info!("sound: the library is in, {} ms", started.elapsed().as_millis());
        self.loaded = true;
        true
    }

    /// How loud Flint will play a sound at `src`, from its distance to the
    /// camera: the engine's curve with this soundscape's falloff.
    fn heard(ears: Ears, src: [f32; 3]) -> f32 {
        let [cx, cy, cz] = ears.camera;
        let d = [src[0] - cx, src[1] - cy, src[2] - cz];
        falloff((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
    }

    fn birds(&mut self, ears: Ears) {
        let w = self.mix.birds;
        if self.now < self.next_bird {
            return;
        }
        // Fewer calls where there are fewer birds; none where there are none.
        self.next_bird = self.now + self.rng.range(1.2, 5.0) as f64 / (w as f64).max(0.15);
        if w < 0.05 {
            return;
        }
        let mut which = 1 + (self.rng.unit() * 4.0) as usize % 4;
        if which == self.last_bird {
            which = which % 4 + 1;
        }
        self.last_bird = which;
        let angle = self.rng.range(0.0, std::f32::consts::TAU);
        let dist = self.rng.range(8.0, 45.0);
        let src = [
            ears.focus[0] + angle.cos() * dist,
            crate::coords::layer::GROUND + self.rng.range(3.0, 10.0),
            ears.focus[1] + angle.sin() * dist,
        ];
        let volume = 0.45 * w * self.rng.range(0.55, 1.0);
        let pitch = self.rng.range(0.9, 1.12);
        self.cue(Cue {
            sound: format!("bird_{which}"),
            volume: volume * EFFECTS,
            at: Some(src),
            pitch,
            interface: false,
        });
    }

    /// Now and then, near the core, the tower's bell.
    fn bells(&mut self) {
        if self.now < self.next_bell {
            return;
        }
        self.next_bell = self.now + self.rng.range(70.0, 140.0) as f64;
        if self.mix.city > 0.3 {
            let v = 0.3 * self.mix.city;
            self.cue(Cue::flat("bell", v));
        }
    }

    /// Each tuft being worked rustles every half-second or so, yours
    /// louder than anyone else's.
    fn rustle(&mut self, ears: Ears, working: &[Rustling]) {
        self.rustles.retain(|(id, _)| working.iter().any(|w| &w.id == id));
        for w in working {
            let due = match self.rustles.iter_mut().find(|(id, _)| id == &w.id) {
                Some((_, t)) if self.now < *t => continue,
                Some((_, t)) => t,
                None => {
                    // Start each a little apart, so two tufts don't rustle as one.
                    let first = self.now + self.rng.range(0.0, 0.3) as f64;
                    self.rustles.push((w.id.clone(), first));
                    continue;
                }
            };
            *due = self.now + self.rng.range(0.45, 0.85) as f64;
            let src = [w.at[0], crate::coords::layer::GROUND + 0.5, w.at[1]];
            let base = if w.mine { 0.5 } else { 0.28 };
            let volume = base * self.rng.range(0.7, 1.0);
            if volume * Self::heard(ears, src) < 0.02 {
                continue;
            }
            let which = 1 + (self.rng.unit() * 3.0) as usize % 3;
            let pitch = self.rng.range(0.9, 1.1);
            self.cue(Cue {
                sound: format!("rustle_{which}"),
                volume: volume * EFFECTS,
                at: Some(src),
                pitch,
                interface: false,
            });
        }
    }
}

/// The level Flint plays a placed one-shot at, `d` metres from the
/// listener: Kira's spatial attenuation, `-60·r^ROLLOFF` dB, with `r` how far
/// `d` is from [`NEAR`] to [`FAR`].
fn falloff(d: f32) -> f32 {
    let r = ((d - NEAR) / (FAR - NEAR)).clamp(0.0, 1.0);
    10f32.powf(-3.0 * r.powf(ROLLOFF))
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::protocol::rules::Zone;

    fn zone_middle(z: Zone) -> f32 {
        let (a, b) = z.radii();
        ((a + b) / 2.0) as f32
    }

    const STREET: f32 = 25.0;
    const DRY: f32 = 1000.0;

    #[test]
    fn the_core_hums_and_the_wilderness_blows() {
        let core = mix(zone_middle(Zone::TradeDistrict), DRY, STREET);
        assert!(core.city > 0.99 && core.breeze < 0.01 && core.wind < 0.01, "{core:?}");
        let suburb = mix(zone_middle(Zone::Suburban), DRY, STREET);
        assert!(suburb.breeze > 0.99 && suburb.birds > 0.99, "{suburb:?}");
        let wild = mix(zone_middle(Zone::Wilderness), DRY, STREET);
        assert!(wild.wind > 0.95 && wild.city < 0.01 && wild.birds < 0.05, "{wild:?}");
    }

    #[test]
    fn walking_in_crossfades_at_constant_power_and_without_a_jump() {
        let mut last: Option<Mix> = None;
        for step in (0..=4000).rev() {
            let m = mix(step as f32, DRY, STREET);
            let power = m.city.powi(2) + m.breeze.powi(2) + m.wind.powi(2);
            assert!((power - 1.0).abs() < 1e-3, "r {step}: power {power}");
            if let Some(l) = last {
                for (a, b) in [(l.city, m.city), (l.breeze, m.breeze), (l.wind, m.wind)] {
                    assert!((a - b).abs() < 0.01, "r {step}: {a} -> {b}");
                }
            }
            last = Some(m);
        }
    }

    #[test]
    fn water_laps_near_a_ring_only() {
        assert!(mix(800.0, 0.0, STREET).water > 0.99);
        assert!(mix(800.0, 125.0, STREET).water > 0.4);
        assert_eq!(mix(800.0, 300.0, STREET).water, 0.0);
    }

    #[test]
    fn the_map_view_hears_only_a_thin_wind() {
        let m = mix(zone_middle(Zone::Suburban), 0.0, 620.0);
        assert_eq!((m.city, m.breeze, m.water, m.birds), (0.0, 0.0, 0.0, 0.0));
        assert!((m.wind - 0.45).abs() < 1e-3);
    }

    fn ears_at(r: f32) -> Ears {
        let [x, z] = crate::coords::to_world(4000.0 + r as f64, 4000.0);
        Ears {
            focus: [x, z],
            camera: [x, 25.0, z + 20.0],
            height: STREET,
        }
    }

    fn sounds(cues: &[Cue], prefix: &str) -> usize {
        cues.iter().filter(|c| c.sound.starts_with(prefix)).count()
    }

    fn run(s: &mut Soundscape, ears: Ears, working: &[Rustling], seconds: f32) -> Vec<Cue> {
        let mut all = Vec::new();
        for _ in 0..(seconds * 60.0) as usize {
            all.extend(s.update(ears, working, 1.0 / 60.0));
        }
        all
    }

    #[test]
    fn birds_sing_in_the_suburbs_and_not_at_the_core() {
        let mut s = Soundscape::new(false);
        let cues = run(&mut s, ears_at(zone_middle(Zone::Suburban)), &[], 60.0);
        assert!(sounds(&cues, "bird_") >= 10, "{}", sounds(&cues, "bird_"));
        // Every bird sings where it is, round you, in the trees.
        let focus = ears_at(zone_middle(Zone::Suburban)).focus;
        for c in cues.iter().filter(|c| c.sound.contains("bird")) {
            let [x, y, z] = c.at.unwrap();
            let d = ((x - focus[0]).powi(2) + (z - focus[1]).powi(2)).sqrt();
            assert!((7.9..45.1).contains(&d), "{d}");
            assert!(y > crate::coords::layer::GROUND + 2.9);
        }
        let mut s = Soundscape::new(false);
        let cues = run(&mut s, ears_at(100.0), &[], 60.0);
        assert_eq!(sounds(&cues, "bird_"), 0);
    }

    #[test]
    fn the_beds_ease_toward_the_mix() {
        let mut s = Soundscape::new(false);
        run(&mut s, ears_at(100.0), &[], 0.1);
        assert!(s.levels[0] < 0.2, "eases in, doesn't snap: {}", s.levels[0]);
        run(&mut s, ears_at(100.0), &[], 6.0);
        assert!((s.levels[0] - BEDS[0].2 * AMBIENCE).abs() < 0.01);
    }

    #[test]
    fn your_tuft_rustles_louder_than_a_neighbours() {
        let ears = ears_at(2000.0);
        let mine = Rustling { id: "a".into(), at: ears.focus, mine: true };
        let theirs = Rustling { id: "b".into(), at: ears.focus, mine: false };
        let mut s = Soundscape::new(false);
        let cues = run(&mut s, ears, &[mine, theirs], 5.0);
        let rustles: Vec<&Cue> = cues.iter().filter(|c| c.sound.contains("rustle")).collect();
        assert!(rustles.len() >= 10, "{}", rustles.len());
        let loudest_theirs = rustles.iter().filter(|c| c.volume < 0.3).count();
        assert!(loudest_theirs > 0 && loudest_theirs < rustles.len());
        // Stopped working, it stops.
        assert_eq!(sounds(&run(&mut s, ears, &[], 3.0), "rustle"), 0);
    }

    #[test]
    fn muted_is_silent() {
        let mut s = Soundscape::new(true);
        s.click();
        s.crafted();
        let ears = ears_at(2000.0);
        let mine = Rustling { id: "a".into(), at: ears.focus, mine: true };
        assert!(run(&mut s, ears, &[mine], 30.0).is_empty());
        assert_eq!(s.levels, [0.0; 4]);
    }

    #[test]
    fn the_volumes_scale_what_plays() {
        let mut s = Soundscape::new(false);
        s.set_volumes(Volumes {
            master: 0.5,
            ambience: 0.0,
            effects: 1.0,
            interface: 0.0,
        });
        s.click();
        s.gathered();
        let cues = run(&mut s, ears_at(2000.0), &[], 10.0);
        assert_eq!(sounds(&cues, "ui_click"), 0, "the interface is silent");
        let pluck = cues.iter().find(|c| c.sound == "pluck").unwrap();
        assert!((pluck.volume - 0.5 * EFFECTS).abs() < 1e-6, "master is the bus's");
        assert_eq!(s.bus_volumes(), (0.0, 0.5), "no ambience; master on the sfx bus");
        assert!(s.levels[1] > 0.1, "the beds follow the mix; the bus silences them");
    }

    #[test]
    fn a_cue_plays_on_the_sfx_bus_flat_or_placed() {
        let mut s = Soundscape::new(false);
        s.click();
        let cues = s.update(ears_at(100.0), &[], 0.016);
        let click = cues.iter().find(|c| c.sound == "ui_click").unwrap().options();
        assert_eq!((click.bus, click.at), (Bus::Sfx, None));
        assert!((click.volume - 0.35).abs() < 1e-6);
        let bird = Cue {
            sound: "bird_1".into(),
            volume: 0.2,
            at: Some([1.0, 2.0, 3.0]),
            pitch: 1.1,
            interface: false,
        }
        .options();
        assert_eq!(bird.at, Some(Vec3::new(1.0, 2.0, 3.0)));
        assert!((bird.pitch - 1.1).abs() < 1e-6);
        assert_eq!((bird.distances, bird.rolloff), ((NEAR, FAR), ROLLOFF));
    }

    #[test]
    fn the_engine_fades_a_sound_as_the_soundscape_was_tuned() {
        // The curve the levels were tuned by ear with, before #94.
        let tuned = |d: f32| (NEAR / d.max(NEAR)).powf(1.2);
        let db = |a: f32| 20.0 * a.log10();
        for d in (0..=300).step_by(4).map(|d| d as f32) {
            let off = (db(falloff(d)) - db(tuned(d))).abs();
            assert!(off < 1.5, "{off:.2} dB off at {d} m");
        }
        // The map view, a kilometre up, hears nothing.
        assert!(falloff(FAR) <= 0.001 + 1e-6);
    }

    #[test]
    fn every_cue_names_a_sound_the_library_makes() {
        let mut s = Soundscape::new(false);
        s.click();
        s.drop_in_slot();
        s.gathered();
        s.crafted();
        s.claimed();
        let ears = ears_at(2000.0);
        let mine = Rustling { id: "a".into(), at: ears.focus, mine: true };
        let mut cues = run(&mut s, ears, &[mine], 120.0);
        cues.extend(run(&mut s, ears_at(100.0), &[], 300.0));
        let files: Vec<String> = library::RECIPES.iter().map(|(f, _)| f.to_string()).collect();
        for c in &cues {
            assert!(files.contains(&c.sound), "{}", c.sound);
        }
        for (_, file, _) in BEDS {
            assert!(files.iter().any(|f| f == file), "{file}");
        }
        assert!(sounds(&cues, "bell") >= 2, "the deed, and the tower now and then");
    }
}
