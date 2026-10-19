//! Every sound the Capitol plays (P8.1), made here from nothing but noise
//! and sines, so there are no recordings to licence and none in the repo.
//! [`make_all`] makes them at start, off the main thread, and the
//! [`Soundscape`](super::Soundscape) hands them to Flint from memory
//! (Flint ADR 0084). Nothing is written to disk: `--write-sounds <dir>`
//! writes them as WAVs ([`write_all`]) for listening or spectrograms.
//! `audio/CREDITS.md` says where each came from.
//!
//! The beds loop: four ambiences the soundscape crossfades by zone, city
//! hum at the core, a breeze through the suburbs and fields, wind past the
//! edge, and water lapping at the rings (and the wilds' lakes), and a fifth
//! that rises by the wilds' running water (W10). The rest are one-shots.

use super::dsp::*;
use std::f32::consts::TAU;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// One sound: one channel, or two (left, right), at [`SR`].
pub type Channels = Vec<Vec<f32>>;

/// What makes one sound.
pub type Recipe = fn() -> Channels;

/// Each sound by name, with the recipe that makes it.
pub const RECIPES: &[(&str, Recipe)] = &[
    ("amb_city", city),
    ("amb_breeze", breeze),
    ("amb_wind", wind),
    ("amb_water", water),
    ("amb_stream", stream),
    ("ui_click", || vec![ui_click()]),
    ("ui_drop", || vec![ui_drop()]),
    ("rustle_1", || vec![rustle(1)]),
    ("rustle_2", || vec![rustle(2)]),
    ("rustle_3", || vec![rustle(3)]),
    ("pluck", || vec![gathered()]),
    ("chime", || vec![chime()]),
    ("bell", || vec![bell()]),
    ("bird_1", || vec![bird(1)]),
    ("bird_2", || vec![bird(2)]),
    ("bird_3", || vec![bird(3)]),
    ("bird_4", || vec![bird(4)]),
];

/// Make every sound, a thread each (the recipes share nothing).
pub fn make_all() -> Vec<(&'static str, Channels)> {
    std::thread::scope(|s| {
        let made: Vec<_> = RECIPES
            .iter()
            .map(|(name, make)| (*name, s.spawn(make)))
            .collect();
        made.into_iter()
            .filter_map(|(name, m)| match m.join() {
                Ok(channels) => Some((name, channels)),
                Err(_) => {
                    tracing::warn!("sound: the {name} recipe panicked");
                    None
                }
            })
            .collect()
    })
}

/// Write every sound into `dir` as `<name>.wav` (`--write-sounds`).
pub fn write_all(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    for (name, channels) in make_all() {
        write_wav(&dir.join(format!("{name}.wav")), &channels)?;
    }
    Ok(())
}

/// 16-bit PCM WAV, channels interleaved.
pub fn write_wav(path: &Path, channels: &[Vec<f32>]) -> io::Result<()> {
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    let ch = channels.len() as u16;
    let data_len = n as u32 * ch as u32 * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&ch.to_le_bytes());
    out.extend_from_slice(&SR.to_le_bytes());
    out.extend_from_slice(&(SR * ch as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(ch * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..n {
        for c in channels {
            let s = (c[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    let mut f = fs::File::create(path)?;
    f.write_all(&out)
}

// ── Beds ──────────────────────────────────────────────────────────────────
//
// Each bed is stereo: the two sides are the same recipe from different
// seeds, so the texture is wide while the swells, which come from loop-
// periodic LFOs, move together. Each part is levelled to unit RMS before
// it's weighed into the mix, so the weights mean what they say, and the bed
// is levelled to `BED_RMS`; the soundscape sets the balance between beds.

const BED_RMS: f32 = 0.16;

fn bed(seconds: f32, fade: f32, side: impl Fn(u64, usize, usize) -> Vec<f32>) -> Vec<Vec<f32>> {
    bed_over(seconds, fade, side, |_| None)
}

/// A bed with a tone under it. The tone runs whole cycles round the loop,
/// so it's added after the seam: faded across itself it would swell.
fn bed_over(
    seconds: f32,
    fade: f32,
    side: impl Fn(u64, usize, usize) -> Vec<f32>,
    tone: impl Fn(usize) -> Option<(Vec<f32>, f32)>,
) -> Vec<Vec<f32>> {
    let n = samples(seconds);
    let total = n + samples(fade);
    let mut sides: Vec<Vec<f32>> = [11u64, 29]
        .iter()
        .map(|&seed| {
            let mut v = seam(side(seed, n, total), n);
            if let Some((t, w)) = tone(n) {
                mix_into(&mut v, &t, 0, w);
            }
            v
        })
        .collect();
    let r = (rms(&sides[0]) + rms(&sides[1])) / 2.0;
    for s in &mut sides {
        s.iter_mut().for_each(|x| *x *= BED_RMS / r);
        soft_limit(s);
    }
    sides
}

/// A part of a bed: `total` samples, levelled to unit RMS.
fn part(total: usize, mut f: impl FnMut(usize) -> f32) -> Vec<f32> {
    let mut v: Vec<f32> = (0..total).map(&mut f).collect();
    to_rms(&mut v, 1.0);
    v
}

fn weigh(parts: &[(Vec<f32>, f32)]) -> Vec<f32> {
    let n = parts[0].0.len();
    (0..n)
        .map(|i| parts.iter().map(|(p, w)| p[i] * w).sum())
        .collect()
}

/// The core: the low rumble of traffic swelling and easing, the murmur of
/// a crowd somewhere out of sight, and under it a warm drone on A, its
/// octave and fifth, with a second octave a quarter-hertz sharp so it
/// slowly shimmers. Every tone runs a whole number of cycles round the
/// 12 s loop.
fn city() -> Vec<Vec<f32>> {
    let drone = |n: usize| {
        let tone = part(n, |i| {
            let t = i as f32 / SR as f32;
            [(55.0, 1.0), (110.0, 0.5), (110.25, 0.35), (82.5, 0.4), (165.0, 0.18)]
                .iter()
                .map(|(hz, a)| a * (TAU * hz * t).sin())
                .sum()
        });
        Some((tone, 0.30))
    };
    bed_over(12.0, 1.5, |seed, n, total| {
        let mut brown = Brown::new(seed);
        let mut lp = Biquad::lowpass(170.0, 0.7);
        let rumble = part(total, |i| {
            let swell = 0.7 + 0.2 * lfo(i, n, 3.0, 0.0) + 0.12 * lfo(i, n, 7.0, 1.1);
            lp.run(brown.next()) * swell
        });
        let mut pink = Pink::new(seed + 1);
        let mut band = Biquad::bandpass(520.0, 0.8);
        let mut chatter_noise = Rng::new(seed + 2);
        let mut chatter = Biquad::lowpass(5.0, 0.7);
        let murmur = part(total, |i| {
            let flicker = 0.4 + chatter.run(chatter_noise.white()).abs() * 6.0;
            band.run(pink.next()) * flicker * (0.8 + 0.2 * lfo(i, n, 2.0, 0.4))
        });
        let mut air = Pink::new(seed + 3);
        let (mut hp, mut lp2) = (Biquad::highpass(2500.0, 0.7), Biquad::lowpass(6000.0, 0.7));
        let hiss = part(total, |_| lp2.run(hp.run(air.next())));
        weigh(&[(rumble, 1.0), (murmur, 0.32), (hiss, 0.05)])
    }, drone)
}

/// Suburbs and fields: soft air moving, leaves stirring when it gusts, and
/// a low hush. The birds are separate one-shots, scattered round you by
/// the soundscape, so they never repeat.
fn breeze() -> Vec<Vec<f32>> {
    bed(14.0, 2.0, |seed, n, total| {
        let gust = |i: usize| 0.65 + 0.25 * lfo(i, n, 2.0, 0.3) + 0.12 * lfo(i, n, 5.0, 2.0);
        let mut pink = Pink::new(seed);
        let mut lp = Biquad::lowpass(900.0, 0.6);
        let air = part(total, |i| lp.run(pink.next()) * gust(i));
        // Leaves: soft swells of filtered noise a few tens of milliseconds
        // long, overlapping, more of them as it gusts. Not clicks.
        let mut rng = Rng::new(seed + 1);
        let mut leaf_noise = Pink::new(seed + 4);
        let mut band = Biquad::bandpass(3000.0, 0.8);
        let mut top = Biquad::lowpass(5500.0, 0.7);
        let mut stir = 0.0f32;
        let mut target = 0.0f32;
        let (rise, fall) = (
            1.0 - (-1.0 / (0.012 * SR as f32)).exp(),
            1.0 - (-1.0 / (0.060 * SR as f32)).exp(),
        );
        let leaves = part(total, |i| {
            let g = gust(i);
            if rng.unit() < 0.0004 * g * g * g {
                target += rng.range(0.3, 1.0);
            }
            target *= 1.0 - fall;
            stir += (target - stir) * rise;
            top.run(band.run(leaf_noise.next())) * stir
        });
        let mut brown = Brown::new(seed + 2);
        let mut low = Biquad::lowpass(120.0, 0.7);
        let hush = part(total, |_| low.run(brown.next()));
        weigh(&[(air, 1.0), (leaves, 0.22), (hush, 0.3)])
    })
}

/// Past the last fields: wind through two resonant bands whose pitch rides
/// the gusts, so it whistles a little as it rises, over a low buffeting.
fn wind() -> Vec<Vec<f32>> {
    bed(16.0, 2.0, |seed, n, total| {
        let gust = |i: usize| {
            (0.55 + 0.30 * lfo(i, n, 1.0, 0.0) + 0.15 * lfo(i, n, 3.0, 1.3)
                + 0.08 * lfo(i, n, 7.0, 0.4))
            .max(0.1)
        };
        let mut pink = Pink::new(seed);
        let mut bp1 = Biquad::bandpass(400.0, 4.0);
        let howl = part(total, |i| {
            let g = gust(i);
            if i % 32 == 0 {
                bp1.retune_bandpass(380.0 + 260.0 * g, 4.0);
            }
            bp1.run(pink.next()) * g
        });
        let mut pink2 = Pink::new(seed + 1);
        let mut bp2 = Biquad::bandpass(900.0, 7.0);
        let whistle = part(total, |i| {
            let g = gust(i);
            if i % 32 == 0 {
                let drift = 0.5 + 0.5 * lfo(i, n, 2.0, 0.7);
                bp2.retune_bandpass(900.0 + 500.0 * drift * g, 7.0);
            }
            bp2.run(pink2.next()) * g * g
        });
        let mut pink3 = Pink::new(seed + 2);
        let mut lp = Biquad::lowpass(500.0, 0.7);
        let body = part(total, |i| lp.run(pink3.next()) * gust(i));
        let mut brown = Brown::new(seed + 3);
        let mut low = Biquad::lowpass(90.0, 0.7);
        let buffet = part(total, |i| low.run(brown.next()) * (0.5 + 0.5 * gust(i)));
        weigh(&[(howl, 1.0), (whistle, 0.35), (body, 0.6), (buffet, 0.5)])
    })
}

/// At the rings: an unhurried wash, laps against the stone at uneven
/// intervals, each opening the filter as it breaks and closing as it
/// draws back, and now and then a bubble.
fn water() -> Vec<Vec<f32>> {
    bed(10.0, 1.0, |seed, n, total| {
        let mut brown = Brown::new(seed);
        let mut lp = Biquad::lowpass(350.0, 0.7);
        let wash = part(total, |i| lp.run(brown.next()) * (0.8 + 0.2 * lfo(i, n, 3.0, 0.0)));

        let mut rng = Rng::new(seed + 1);
        let mut laps = vec![0.0f32; total];
        let mut t = rng.range(0.1, 0.4);
        while samples(t) < total {
            let a = rng.range(0.5, 1.0);
            let (attack, decay) = (rng.range(0.10, 0.18), rng.range(0.25, 0.45));
            let start = samples(t);
            for (k, e) in laps.iter_mut().enumerate().skip(start).take(samples(1.6)) {
                *e += a * strike((k - start) as f32 / SR as f32, attack, decay);
            }
            t += rng.range(0.55, 1.5);
        }
        let mut pink = Pink::new(seed + 2);
        let mut sweep = Biquad::lowpass(300.0, 0.9);
        let lapping = part(total, |i| {
            let e = laps[i].min(1.4);
            if i % 16 == 0 {
                sweep.retune_lowpass(250.0 + 1300.0 * e, 0.9);
            }
            sweep.run(pink.next()) * e
        });

        let mut bubbles = vec![0.0f32; total];
        let mut t = rng.range(0.0, 0.3);
        while samples(t) < total {
            let f0 = rng.range(500.0, 1300.0);
            let blip = voice(
                0.06,
                |s| f0 * (1.0 + 1.5 * s / 0.04),
                |s| strike(s, 0.002, 0.012),
            );
            mix_into(&mut bubbles, &blip, samples(t), rng.range(0.3, 1.0));
            t += rng.range(0.15, 0.6);
        }
        to_rms(&mut bubbles, 1.0);
        weigh(&[(wash, 0.6), (lapping, 1.0), (bubbles, 0.06)])
    })
}

/// By a river or stream in the wilds (W10): water running over stones.
/// A bright rush of noise that never settles, fluttering fast as the
/// surface breaks, a low burble under it, and many small plinks, each a
/// bubble's chirp rising as it closes, thick enough that they blur into
/// the babble of a brook.
fn stream() -> Vec<Vec<f32>> {
    bed(9.0, 1.0, |seed, n, total| {
        let mut pink = Pink::new(seed);
        let mut band = Biquad::bandpass(1600.0, 0.7);
        let mut soft = Biquad::lowpass(4500.0, 0.7);
        let mut flutter_noise = Rng::new(seed + 1);
        let mut flutter = Biquad::lowpass(14.0, 0.7);
        let rush = part(total, |i| {
            let f = 0.6 + flutter.run(flutter_noise.white()).abs() * 5.0;
            soft.run(band.run(pink.next())) * f.min(1.6) * (0.9 + 0.1 * lfo(i, n, 3.0, 0.2))
        });
        let mut brown = Brown::new(seed + 2);
        let mut lp = Biquad::lowpass(260.0, 0.8);
        let burble = part(total, |i| lp.run(brown.next()) * (0.85 + 0.15 * lfo(i, n, 5.0, 1.7)));

        let mut rng = Rng::new(seed + 3);
        let mut plinks = vec![0.0f32; total];
        let mut t = rng.range(0.0, 0.02);
        while samples(t) < total {
            let f0 = rng.range(700.0, 2600.0);
            let len = rng.range(0.025, 0.06);
            let blip = voice(
                len + 0.01,
                |s| f0 * (1.0 + 0.9 * s / len),
                |s| strike(s, 0.0015, len * 0.35),
            );
            mix_into(&mut plinks, &blip, samples(t), rng.range(0.2, 1.0));
            t += rng.range(0.008, 0.05);
        }
        to_rms(&mut plinks, 1.0);
        weigh(&[(rush, 1.0), (burble, 0.35), (plinks, 0.45)])
    })
}

// ── One-shots ────────────────────────────────────────────────────────────

/// A button: a small wooden tock.
fn ui_click() -> Vec<f32> {
    let mut rng = Rng::new(5);
    let mut hp = Biquad::highpass(3000.0, 0.7);
    let mut v: Vec<f32> = (0..samples(0.06))
        .map(|i| {
            let t = i as f32 / SR as f32;
            (TAU * 1850.0 * t).sin() * (-t / 0.010).exp()
                + 0.2 * (TAU * 3700.0 * t).sin() * (-t / 0.006).exp()
                + 0.2 * hp.run(rng.white()) * (-t / 0.0015).exp()
        })
        .collect();
    to_peak(&mut v, 0.8);
    v
}

/// Something set down in a slot: lower and rounder than the click.
fn ui_drop() -> Vec<f32> {
    let mut rng = Rng::new(6);
    let mut lp = Biquad::lowpass(1500.0, 0.7);
    let mut body = voice(0.12, |t| 620.0 - 600.0 * t, |t| (-t / 0.025).exp());
    let over = voice(0.12, |t| 1240.0 - 1200.0 * t, |t| 0.25 * (-t / 0.015).exp());
    mix_into(&mut body, &over, 0, 1.0);
    for (i, x) in body.iter_mut().enumerate() {
        let t = i as f32 / SR as f32;
        *x += 0.5 * lp.run(rng.white()) * (-t / 0.004).exp();
    }
    to_peak(&mut body, 0.8);
    body
}

/// Grass being worked: a handful of dry crackles over a soft swish.
fn rustle(variant: u64) -> Vec<f32> {
    let mut rng = Rng::new(100 + variant);
    let len = samples(0.4);
    let mut env = vec![0.0f32; len];
    let grains = 18 + (rng.unit() * 12.0) as usize;
    for _ in 0..grains {
        let at = samples(0.28 * rng.unit().powf(1.5));
        let (a, d) = (rng.range(0.3, 1.0), rng.range(0.005, 0.014));
        for (k, e) in env.iter_mut().enumerate().skip(at) {
            *e += a * strike((k - at) as f32 / SR as f32, 0.001, d);
        }
    }
    let mut hi = Biquad::bandpass(rng.range(2400.0, 3200.0), 1.1);
    let mut lo = Biquad::bandpass(1100.0, 0.8);
    let mut dark = Biquad::lowpass(4500.0, 0.7);
    let mut pink = Pink::new(200 + variant);
    let mut v: Vec<f32> = (0..len)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let swish = strike(t, 0.06, 0.09);
            let n = pink.next();
            dark.run(hi.run(n) * env[i] + 0.45 * lo.run(n) * swish)
        })
        .collect();
    to_peak(&mut v, 0.8);
    v
}

/// A yield: two soft plucked notes, G then the D above.
fn gathered() -> Vec<f32> {
    let mut v = pluck(783.99, 0.7, 0.18, 21);
    v.resize(samples(0.75), 0.0);
    let d = pluck(1174.66, 0.65, 0.15, 22);
    mix_into(&mut v, &d, samples(0.07), 0.7);
    let mut v = reverb(&v, 0.25, 0.4, 0.4);
    to_peak(&mut v, 0.8);
    v
}

/// A struck bar: a fundamental with the free bar's inharmonic overtones,
/// each dying faster than the one below.
fn bar(hz: f32, seconds: f32) -> Vec<f32> {
    let mut v = vec![0.0; samples(seconds)];
    for (ratio, amp, decay) in [(1.0, 1.0, 1.4), (2.756, 0.35, 0.5), (5.404, 0.15, 0.25), (8.933, 0.06, 0.12)] {
        let f = hz * ratio;
        if f > 12_000.0 {
            continue;
        }
        let partial = voice(seconds, |_| f, |t| amp * strike(t, 0.002, decay));
        mix_into(&mut v, &partial, 0, 1.0);
    }
    fade_out(&mut v, seconds * 0.4);
    v
}

/// A craft finished: D, F# and A struck in turn, a little gold arpeggio,
/// ringing in a small room.
fn chime() -> Vec<f32> {
    let mut v = vec![0.0; samples(3.5)];
    for (k, hz) in [1174.66, 1479.98, 1760.0].iter().enumerate() {
        mix_into(&mut v, &bar(*hz, 3.2), samples(0.11 * k as f32), 1.0 - 0.15 * k as f32);
    }
    let mut v = reverb(&v, 0.35, 0.6, 1.0);
    to_peak(&mut v, 0.8);
    v
}

/// The tower's bell, heard across the city: a minor-third church bell on G
/// (hum, prime, tierce, quint, nominal and up), the strike softened by
/// distance and the hum hanging on longest.
fn bell() -> Vec<f32> {
    let hz = 196.0;
    let seconds = 9.0;
    let mut v = vec![0.0; samples(seconds)];
    for (ratio, amp, decay) in [
        (0.5, 0.6, 5.0),
        (1.0, 0.8, 3.5),
        (1.2, 0.5, 2.8),
        (1.5, 0.3, 2.2),
        (2.0, 0.7, 2.0),
        (2.5, 0.25, 1.4),
        (2.67, 0.15, 1.1),
        (4.0, 0.1, 0.7),
    ] {
        // Each partial a pair a fraction of a hertz apart, so it beats.
        for detune in [-0.35, 0.35] {
            let f = hz * ratio + detune;
            let p = voice(seconds, |_| f, |t| 0.5 * amp * strike(t, 0.004, decay));
            mix_into(&mut v, &p, 0, 1.0);
        }
    }
    let mut rng = Rng::new(9);
    let mut lp = Biquad::lowpass(2000.0, 0.7);
    for (i, x) in v.iter_mut().enumerate().take(samples(0.03)) {
        *x += 0.3 * lp.run(rng.white()) * (-(i as f32) / (0.006 * SR as f32)).exp();
    }
    fade_out(&mut v, 2.5);
    let mut far = Biquad::lowpass(2600.0, 0.6);
    let v: Vec<f32> = v.iter().map(|x| far.run(*x)).collect();
    let mut v = reverb(&v, 0.5, 0.9, 1.5);
    to_peak(&mut v, 0.8);
    v
}

/// One whistled note: a sine (and a little of its octave) gliding from
/// `from` to `to` Hz over `seconds`, with a touch of vibrato.
fn chirp(from: f32, to: f32, seconds: f32, vibrato: f32) -> Vec<f32> {
    let hz = move |t: f32| {
        let s = (t / seconds).clamp(0.0, 1.0);
        from + (to - from) * s + vibrato * (TAU * 28.0 * t).sin()
    };
    let mut v = voice(seconds, hz, |t| hump(t / seconds).powf(0.7));
    let octave = voice(seconds, move |t| 2.0 * hz(t), |t| 0.12 * hump(t / seconds));
    mix_into(&mut v, &octave, 0, 1.0);
    v
}

/// Four songbirds. Each call is a few chirps; the soundscape plays them at
/// different pitches and places, so a handful make a garden.
fn bird(variant: u32) -> Vec<f32> {
    let mut v = vec![0.0; samples(1.6)];
    let mut at = 0.0f32;
    let mut put = |v: &mut Vec<f32>, c: Vec<f32>, gap: f32, gain: f32| {
        mix_into(v, &c, samples(at), gain);
        at += c.len() as f32 / SR as f32 + gap;
    };
    match variant {
        // Three quick falling whistles.
        1 => {
            for k in 0..3 {
                let top = 3300.0 - 120.0 * k as f32;
                put(&mut v, chirp(top, top - 650.0, 0.08, 20.0), 0.06, 1.0 - 0.15 * k as f32);
            }
        }
        // A trill.
        2 => {
            for k in 0..10 {
                let hz = if k % 2 == 0 { 4000.0 } else { 4600.0 };
                put(&mut v, chirp(hz, hz - 200.0, 0.03, 0.0), 0.012, 0.6 + 0.04 * k as f32);
            }
        }
        // Tee-yoo: up, then a long fall.
        3 => {
            put(&mut v, chirp(2400.0, 3600.0, 0.15, 30.0), 0.03, 0.8);
            put(&mut v, chirp(3800.0, 2200.0, 0.22, 40.0), 0.0, 1.0);
        }
        // Fee-bee: two held notes, the second a step down.
        _ => {
            put(&mut v, chirp(3950.0, 3900.0, 0.28, 25.0), 0.08, 1.0);
            put(&mut v, chirp(3450.0, 3380.0, 0.30, 25.0), 0.0, 0.9);
        }
    }
    v.truncate(samples(at + 0.05).min(v.len()));
    let mut v = reverb(&v, 0.05, 0.2, 0.3);
    fade_out(&mut v, 0.1);
    to_peak(&mut v, 0.8);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_audible_and_never_clips() {
        for (file, make) in RECIPES {
            let channels = make();
            assert!(!channels.is_empty(), "{file}");
            for c in &channels {
                assert!(c.iter().all(|x| x.is_finite()), "{file}: not finite");
                assert!(peak(c) <= 1.0, "{file}: peak {}", peak(c));
                assert!(rms(c) > 0.01, "{file}: silent");
            }
        }
    }

    #[test]
    fn the_beds_are_stereo_and_loop_without_a_seam() {
        for (file, make) in RECIPES.iter().filter(|(f, _)| f.starts_with("amb_")) {
            let channels = make();
            assert_eq!(channels.len(), 2, "{file}");
            for c in &channels {
                let n = c.len();
                // The first and last quarter-second are about as loud.
                let q = samples(0.25);
                let (head, tail) = (rms(&c[..q]), rms(&c[n - q..]));
                assert!(head / tail < 2.5 && tail / head < 2.5, "{file}: {head} vs {tail}");
                // And the step across the loop point is an ordinary step.
                let across = (c[0] - c[n - 1]).abs();
                let typical = c.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / n as f32;
                assert!(across < typical * 12.0, "{file}: {across} vs {typical}");
            }
        }
    }

    #[test]
    fn write_all_writes_every_sound_as_a_wav() {
        let dir = std::env::temp_dir().join(format!("capitol-sounds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write_all(&dir).unwrap();
        for (name, _) in RECIPES {
            let wav = fs::read(dir.join(format!("{name}.wav"))).unwrap();
            assert_eq!(&wav[..4], b"RIFF", "{name}");
            assert_eq!(&wav[8..16], b"WAVEfmt ", "{name}");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
