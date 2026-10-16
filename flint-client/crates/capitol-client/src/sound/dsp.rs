//! The small kit the sounds are built from (P8.1): a seeded noise source,
//! pink and brown noise, RBJ biquads, a Schroeder reverb, loop seaming and
//! level matching. Mono `f32` buffers at [`SR`] throughout.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Every sound's sample rate.
pub const SR: u32 = 44_100;

/// Seconds to samples.
pub fn samples(seconds: f32) -> usize {
    (seconds * SR as f32).round() as usize
}

/// xorshift64*: fast, seeded, and the same on every machine, so a sound is
/// the same sound wherever it's written.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// 0 to 1.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// -1 to 1.
    pub fn white(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// Pink noise (Paul Kellet's filter): even power per octave, the colour of
/// wind, surf and a distant crowd.
pub struct Pink {
    rng: Rng,
    b: [f32; 7],
}

impl Pink {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            b: [0.0; 7],
        }
    }

    pub fn next(&mut self) -> f32 {
        let w = self.rng.white();
        let b = &mut self.b;
        b[0] = 0.99886 * b[0] + w * 0.055_517_9;
        b[1] = 0.99332 * b[1] + w * 0.075_075_9;
        b[2] = 0.96900 * b[2] + w * 0.153_852;
        b[3] = 0.86650 * b[3] + w * 0.310_485_6;
        b[4] = 0.55000 * b[4] + w * 0.532_952_2;
        b[5] = -0.7616 * b[5] - w * 0.016_898;
        let out = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
        b[6] = w * 0.115_926;
        out * 0.11
    }
}

/// Brown noise: a leaky walk, nearly all rumble.
pub struct Brown {
    rng: Rng,
    y: f32,
}

impl Brown {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            y: 0.0,
        }
    }

    pub fn next(&mut self) -> f32 {
        self.y = (self.y + 0.02 * self.rng.white()) / 1.02;
        self.y * 3.5
    }
}

/// An RBJ biquad, transposed direct form II. Its coefficients can be
/// retuned between samples without losing its state, for swept filters.
#[derive(Clone)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

enum Shape {
    Low,
    High,
    Band,
}

impl Biquad {
    pub fn lowpass(hz: f32, q: f32) -> Self {
        Self::new(Shape::Low, hz, q)
    }

    pub fn highpass(hz: f32, q: f32) -> Self {
        Self::new(Shape::High, hz, q)
    }

    /// Band-pass with 0 dB at the centre.
    pub fn bandpass(hz: f32, q: f32) -> Self {
        Self::new(Shape::Band, hz, q)
    }

    fn new(shape: Shape, hz: f32, q: f32) -> Self {
        let mut f = Self {
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        };
        f.tune(shape, hz, q);
        f
    }

    fn tune(&mut self, shape: Shape, hz: f32, q: f32) {
        let hz = hz.clamp(10.0, SR as f32 * 0.45);
        let w0 = TAU * hz / SR as f32;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match shape {
            Shape::Low => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
            Shape::High => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
            Shape::Band => (alpha, 0.0, -alpha),
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    pub fn retune_lowpass(&mut self, hz: f32, q: f32) {
        self.tune(Shape::Low, hz, q);
    }

    pub fn retune_bandpass(&mut self, hz: f32, q: f32) {
        self.tune(Shape::Band, hz, q);
    }

    pub fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// A sine that runs a whole number of times round a loop of `n` samples,
/// so modulation built from it is seamless at the loop point.
pub fn lfo(i: usize, n: usize, cycles: f32, phase: f32) -> f32 {
    (TAU * cycles * i as f32 / n as f32 + phase).sin()
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A strike: a quarter-sine rise over `attack` seconds, then an exponential
/// fall with time constant `decay`. `t` in seconds.
pub fn strike(t: f32, attack: f32, decay: f32) -> f32 {
    if t < 0.0 {
        0.0
    } else if t < attack {
        (t / attack * FRAC_PI_2).sin()
    } else {
        (-(t - attack) / decay).exp()
    }
}

/// Make a loop of `n` samples from `n + fade` of them: the last `fade` are
/// faded in under the first `fade` with equal power, which suits noise,
/// whose two ends don't correlate.
pub fn seam(mut v: Vec<f32>, n: usize) -> Vec<f32> {
    let fade = v.len() - n;
    for i in 0..fade {
        let t = i as f32 / fade as f32 * FRAC_PI_2;
        v[i] = v[i] * t.sin() + v[n + i] * t.cos();
    }
    v.truncate(n);
    v
}

pub fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
}

pub fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, x| m.max(x.abs()))
}

/// Scale to an RMS of `target`.
pub fn to_rms(v: &mut [f32], target: f32) {
    let r = rms(v);
    if r > 1e-9 {
        v.iter_mut().for_each(|x| *x *= target / r);
    }
}

/// Scale to a peak of `target`.
pub fn to_peak(v: &mut [f32], target: f32) {
    let p = peak(v);
    if p > 1e-9 {
        v.iter_mut().for_each(|x| *x *= target / p);
    }
}

/// Round off anything over 0.9 so a rare crest can't clip.
pub fn soft_limit(v: &mut [f32]) {
    for x in v.iter_mut() {
        let a = x.abs();
        if a > 0.9 {
            *x = x.signum() * (0.9 + 0.1 * ((a - 0.9) / 0.1).tanh());
        }
    }
}

/// Fade the last `seconds` out along a quarter cosine, so a sound that's
/// still ringing when its buffer ends doesn't stop on a click.
pub fn fade_out(v: &mut [f32], seconds: f32) {
    let n = v.len();
    let fade = samples(seconds).min(n);
    for (k, x) in v[n - fade..].iter_mut().enumerate() {
        *x *= (k as f32 / fade as f32 * FRAC_PI_2).cos();
    }
}

/// Add `src` into `dst` at `gain`, from sample `at`.
pub fn mix_into(dst: &mut [f32], src: &[f32], at: usize, gain: f32) {
    for (d, s) in dst.iter_mut().skip(at).zip(src) {
        *d += s * gain;
    }
}

/// A sine whose frequency follows `hz(t)`, under the envelope `amp(t)`, for
/// `seconds`. Birds and bells are made of these.
pub fn voice(seconds: f32, hz: impl Fn(f32) -> f32, amp: impl Fn(f32) -> f32) -> Vec<f32> {
    let mut phase = 0.0f32;
    (0..samples(seconds))
        .map(|i| {
            let t = i as f32 / SR as f32;
            phase = (phase + TAU * hz(t) / SR as f32) % TAU;
            phase.sin() * amp(t)
        })
        .collect()
}

/// A Schroeder reverb (four damped combs into two all-passes, Freeverb's
/// tunings): a small stone room, or the air round a distant tower. Returns
/// the input with `wet` of the reverb added, `tail` seconds longer.
pub fn reverb(dry: &[f32], wet: f32, room: f32, tail: f32) -> Vec<f32> {
    struct Comb {
        buf: Vec<f32>,
        i: usize,
        store: f32,
    }
    let mut combs: Vec<Comb> = [1116, 1188, 1277, 1356, 1422, 1491]
        .iter()
        .map(|&n| Comb {
            buf: vec![0.0; n],
            i: 0,
            store: 0.0,
        })
        .collect();
    let mut passes: Vec<(Vec<f32>, usize)> = [556, 441, 341]
        .iter()
        .map(|&n| (vec![0.0; n], 0))
        .collect();
    let (feedback, damp) = (0.70 + 0.28 * room, 0.25);
    let len = dry.len() + samples(tail);
    (0..len)
        .map(|k| {
            let x = dry.get(k).copied().unwrap_or(0.0) * 0.15;
            let mut y = 0.0;
            for c in &mut combs {
                let out = c.buf[c.i];
                c.store = out * (1.0 - damp) + c.store * damp;
                c.buf[c.i] = x + c.store * feedback;
                c.i = (c.i + 1) % c.buf.len();
                y += out;
            }
            for (buf, i) in &mut passes {
                let b = buf[*i];
                buf[*i] = y + b * 0.5;
                y = b - y;
                *i = (*i + 1) % buf.len();
            }
            dry.get(k).copied().unwrap_or(0.0) + y * wet
        })
        .collect()
}

/// A plucked string (Karplus-Strong) at `hz`, `seconds` long, its
/// brightness (0 to 1) in the initial burst.
pub fn pluck(hz: f32, seconds: f32, brightness: f32, seed: u64) -> Vec<f32> {
    let period = (SR as f32 / hz).round().max(2.0) as usize;
    let mut rng = Rng::new(seed);
    let mut soft = Biquad::lowpass(1000.0 + 8000.0 * brightness, 0.7);
    let mut line: Vec<f32> = (0..period).map(|_| soft.run(rng.white())).collect();
    let mut out = Vec::with_capacity(samples(seconds));
    let mut i = 0;
    for _ in 0..samples(seconds) {
        let next = (i + 1) % period;
        let y = line[i];
        line[i] = 0.5 * (line[i] + line[next]) * 0.996;
        out.push(y);
        i = next;
    }
    fade_out(&mut out, 0.05);
    out
}

/// Half a cosine: 0 to 1 to 0 over `t` in 0..1. The envelope of a chirp.
pub fn hump(t: f32) -> f32 {
    if (0.0..=1.0).contains(&t) {
        (PI * t).sin()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_the_same_everywhere() {
        let a: Vec<u64> = {
            let mut r = Rng::new(7);
            (0..4).map(|_| r.next_u64()).collect()
        };
        let mut r = Rng::new(7);
        assert_eq!(a, (0..4).map(|_| r.next_u64()).collect::<Vec<_>>());
        let mut r = Rng::new(7);
        assert!((0..10_000).all(|_| (0.0..1.0).contains(&r.unit())));
    }

    #[test]
    fn a_lowpass_keeps_the_low_and_drops_the_high() {
        let tone = |hz: f32| -> f32 {
            let mut f = Biquad::lowpass(500.0, 0.707);
            let out: Vec<f32> = (0..SR as usize / 4)
                .map(|i| f.run((TAU * hz * i as f32 / SR as f32).sin()))
                .collect();
            rms(&out[SR as usize / 8..])
        };
        assert!(tone(100.0) > 0.65);
        assert!(tone(5000.0) < 0.02);
    }

    #[test]
    fn a_seamed_loop_meets_itself() {
        let mut pink = Pink::new(3);
        let n = samples(2.0);
        let v = seam((0..n + samples(0.5)).map(|_| pink.next()).collect(), n);
        assert_eq!(v.len(), n);
        // The step across the seam is no bigger than a step inside it.
        let across = (v[0] - v[n - 1]).abs();
        let inside = v.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(across <= inside, "{across} > {inside}");
    }
}
