//! The suburb's neighbourhoods (plan §4.1.1, §4.1.5, §4.1.7).
//!
//! - **Sectors.** Each 45° sector between two arterials is one neighbourhood,
//!   or two when a seeded draw splits it by angle along a mid-angle greenway.
//!   A split never goes by radius: 800 deep halves to about 375, which fits
//!   one street and wastes the rest.
//! - **Frames.** Each neighbourhood gets a `Frame` between its side lines,
//!   inset from the arterial by half its width plus the buffer, or from the
//!   greenway's centre line by half the greenway.
//! - **Streams.** Sector `s` draws its split from `stream(seed, [s])`, and its
//!   neighbourhood `n` everything else from `stream(seed, [s, n])`. Changing
//!   one neighbourhood then leaves every other frame as it was.
//! - **Entrances.** 2–4 per neighbourhood, at row ends on its arterial sides,
//!   at least 150 apart. Across an arterial, entrances either line up exactly
//!   or sit at least 80 apart along it.

use std::f64::consts::FRAC_PI_4;

use crate::models::geom::SplitMix64;
use crate::models::terrain::{NeighbourhoodParams, SuburbanGenParams};

use super::frame::{Frame, Side};

type P = (f64, f64);

/// How far apart one neighbourhood's entrances must be.
pub(crate) const ENTRANCE_SPACING: f64 = 150.0;
/// Entrances on the two sides of an arterial line up to this...
pub(crate) const ALIGNED: f64 = 1e-6;
/// ...or sit at least this far apart along it.
pub(crate) const ENTRANCE_OFFSET: f64 = 80.0;
pub(crate) const MIN_ENTRANCES: usize = 2;
pub(crate) const MAX_ENTRANCES: usize = 4;

/// One of a frame's two side lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge {
    Lo,
    Hi,
}

/// What a neighbourhood's side faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Facing {
    /// Arterial `n`, at 45n°.
    Arterial(u8),
    Greenway,
}

/// Green ground that isn't a lot or a road.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BufferKind {
    /// Between an arterial's edge and a side line.
    Arterial,
    /// Half a greenway, from its centre line to a side line.
    Greenway,
    /// Between the zone's edge and a back line, where the warp dips away.
    Edge,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Buffer {
    pub kind: BufferKind,
    pub polygon: Vec<P>,
}

/// A way in: a row end on an arterial side, and where it meets the arterial.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entrance {
    pub edge: Edge,
    pub arterial: u8,
    /// Which of the frame's rows ends here.
    pub row: usize,
    /// The row end, on the side line.
    pub gate: P,
    /// On the arterial's centre line, and its distance from the world centre.
    pub at: P,
    pub along: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct Neighbourhood {
    pub sector: u8,
    pub index: u8,
    pub frame: Frame,
    pub lo: Facing,
    pub hi: Facing,
    pub buffers: Vec<Buffer>,
    pub entrances: Vec<Entrance>,
    /// The neighbourhood's own stream, where the frame and entrances left
    /// it, for its pattern to go on drawing from.
    pub rng: SplitMix64,
}

impl Neighbourhood {
    pub(crate) fn facing(&self, edge: Edge) -> Facing {
        match edge {
            Edge::Lo => self.lo,
            Edge::Hi => self.hi,
        }
    }
}

/// Every neighbourhood in the suburb, in sector order, with entrances.
pub(crate) fn neighbourhoods(params: &SuburbanGenParams, center: P) -> Vec<Neighbourhood> {
    neighbourhoods_with(params, center, |s, n| SplitMix64::stream(params.seed, &[s as u64, n as u64]))
}

/// `neighbourhoods`, with each neighbourhood's stream from `stream`, so a
/// test can change one and watch the others.
pub(crate) fn neighbourhoods_with(
    params: &SuburbanGenParams,
    center: P,
    stream: impl Fn(u8, u8) -> SplitMix64,
) -> Vec<Neighbourhood> {
    let p = &params.neighbourhood;
    let zone = (params.inner_radius, params.outer_radius);
    let arterial_inset = params.arterial_width / 2.0 + p.buffer;
    let mut all = Vec::new();
    let mut candidates = Vec::new();
    for sector in 0..8u8 {
        let a0 = sector as f64 * FRAC_PI_4;
        let (lo, hi) = (Side { angle: a0, inset: arterial_inset }, Side { angle: a0 + FRAC_PI_4, inset: arterial_inset });
        let facings = (Facing::Arterial(sector), Facing::Arterial((sector + 1) % 8));
        let mid = Side { angle: a0 + FRAC_PI_4 / 2.0, inset: p.greenway_width / 2.0 };

        let split = SplitMix64::stream(params.seed, &[sector as u64]).next_f64() < p.split_chance;
        let halves = [(lo, mid, (facings.0, Facing::Greenway)), (mid, hi, (Facing::Greenway, facings.1))];
        let whole = [(lo, hi, facings)];
        // Both halves must fit rows and be wide enough, or the sector stays whole.
        let built = split
            .then(|| build(sector, &halves, center, zone, params, &stream))
            .filter(|b| b.iter().all(|(n, _)| fits(&n.frame, p)))
            .unwrap_or_else(|| build(sector, &whole, center, zone, params, &stream));
        for (n, c) in built {
            all.push(n);
            candidates.push(c);
        }
    }
    allocate_entrances(&mut all, candidates);
    all
}

/// Whether a frame is worth a neighbourhood of its own: rows fit, and at its
/// narrowest it's two street spacings across.
fn fits(frame: &Frame, p: &NeighbourhoodParams) -> bool {
    let spacing = 2.0 * p.lot_depth + p.local_width;
    let inner = frame.base - frame.warp.amplitude;
    frame.fit.is_some() && inner * frame.span(inner) >= 2.0 * spacing
}

/// A neighbourhood, its entrance candidates, and how many entrances it wants.
type Built = (Neighbourhood, (Vec<Entrance>, usize));

fn build(
    sector: u8,
    parts: &[(Side, Side, (Facing, Facing))],
    center: P,
    zone: (f64, f64),
    params: &SuburbanGenParams,
    stream: &impl Fn(u8, u8) -> SplitMix64,
) -> Vec<Built> {
    parts
        .iter()
        .enumerate()
        .filter_map(|(i, &(lo, hi, (lo_facing, hi_facing)))| {
            let mut rng = stream(sector, i as u8);
            let frame = Frame::new(center, lo, hi, zone, &params.neighbourhood, &mut rng)?;
            let mut n = Neighbourhood {
                sector,
                index: i as u8,
                buffers: Vec::new(),
                entrances: Vec::new(),
                lo: lo_facing,
                hi: hi_facing,
                frame,
                rng,
            };
            n.buffers = buffers(&n, zone, params.arterial_width);
            let c = entrance_candidates(&mut n);
            Some((n, c))
        })
        .collect()
}

/// The strips between the frame and what's round it: the side buffers out
/// to the arterial's edge or the greenway's centre, and the edge strips out
/// to the zone, where the warp pulls a back line away from it.
fn buffers(n: &Neighbourhood, (inner, outer): (f64, f64), arterial_width: f64) -> Vec<Buffer> {
    let f = &n.frame;
    let c = f.center;
    let at = |angle: f64, off: f64, r: f64| {
        // The point at radius r on the line parallel to `angle`, `off` towards the frame.
        let a = angle + (off / r).asin();
        (c.0 + r * a.cos(), c.1 + r * a.sin())
    };
    let mut out = Vec::new();
    for (side, facing, sign) in [(f.lo, n.lo, 1.0), (f.hi, n.hi, -1.0)] {
        let (kind, from) = match facing {
            Facing::Arterial(_) => (BufferKind::Arterial, arterial_width / 2.0),
            Facing::Greenway => (BufferKind::Greenway, 0.0),
        };
        let polygon = vec![
            at(side.angle, sign * from, inner),
            at(side.angle, sign * side.inset, inner),
            at(side.angle, sign * side.inset, outer),
            at(side.angle, sign * from, outer),
        ];
        out.push(Buffer { kind, polygon });
    }
    // Edge strips: the back line, then the zone's arc back between the side lines.
    let arc = |r: f64, from: f64, to: f64| -> Vec<P> {
        let steps = ((to - from).abs() * r / super::frame::STEP).ceil().max(1.0) as usize;
        (0..=steps)
            .map(|i| {
                let a = from + (to - from) * i as f64 / steps as f64;
                (c.0 + r * a.cos(), c.1 + r * a.sin())
            })
            .collect()
    };
    let lo_at = |r: f64| f.lo.angle + (f.lo.inset / r).asin();
    let hi_at = |r: f64| f.hi.angle - (f.hi.inset / r).asin();
    for (v, r) in [(0.0, inner), (f.depth, outer)] {
        let mut polygon = f.iso_v(v, 0.0, f.width);
        // The back line's ends sit on the side lines, at their own radii;
        // the arc runs between the side lines at the zone's.
        polygon.extend(arc(r, hi_at(r), lo_at(r)));
        out.push(Buffer { kind: BufferKind::Edge, polygon });
    }
    out
}

/// Every row end on an arterial side, shuffled by the neighbourhood's
/// stream, and how many entrances it wants. One end from each side comes
/// first, so `allocate_entrances` gives a whole sector a way in from both
/// arterials.
fn entrance_candidates(n: &mut Neighbourhood) -> (Vec<Entrance>, usize) {
    let f = &n.frame;
    let Some(fit) = &f.fit else { return (Vec::new(), 0) };
    let mut by_side = Vec::new();
    for (edge, side, u) in [(Edge::Lo, f.lo, 0.0), (Edge::Hi, f.hi, f.width)] {
        let Facing::Arterial(arterial) = n.facing(edge) else { continue };
        let dir = (side.angle.cos(), side.angle.sin());
        let mut ends: Vec<Entrance> = fit
            .rows
            .iter()
            .enumerate()
            .map(|(row, &v)| {
                let gate = f.to_world(u, v);
                let along = (gate.0 - f.center.0) * dir.0 + (gate.1 - f.center.1) * dir.1;
                let at = (f.center.0 + along * dir.0, f.center.1 + along * dir.1);
                Entrance { edge, arterial, row, gate, at, along }
            })
            .collect();
        shuffle(&mut ends, &mut n.rng);
        by_side.push(ends);
    }
    // One from each side, then the rest in the stream's order.
    let mut out: Vec<Entrance> = by_side.iter_mut().filter(|s| !s.is_empty()).map(|s| s.remove(0)).collect();
    let mut rest: Vec<Entrance> = by_side.into_iter().flatten().collect();
    shuffle(&mut rest, &mut n.rng);
    out.extend(rest);
    let want = MIN_ENTRANCES + n.rng.below((MAX_ENTRANCES - MIN_ENTRANCES + 1) as u64) as usize;
    (out, want)
}

fn shuffle<T>(v: &mut [T], rng: &mut SplitMix64) {
    for i in (1..v.len()).rev() {
        v.swap(i, rng.below(i as u64 + 1) as usize);
    }
}

fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Whether two entrances on opposite sides of one arterial clash: neither
/// lined up nor `ENTRANCE_OFFSET` apart.
pub(crate) fn clash(a: &Entrance, b: &Entrance) -> bool {
    let d = (a.along - b.along).abs();
    a.arterial == b.arterial && d > ALIGNED && d < ENTRANCE_OFFSET
}

/// Pick each neighbourhood's entrances from its candidates, in sector order.
/// An earlier neighbourhood's entrances stand; a later one skips candidates
/// that clash with them across the arterial. If that leaves it short of two,
/// it takes a clashing one and lines it up exactly with the one it clashed
/// with: the stub to the arterial slants, but no target breaks.
fn allocate_entrances(all: &mut [Neighbourhood], candidates: Vec<(Vec<Entrance>, usize)>) {
    let mut placed: Vec<Entrance> = Vec::new();
    for (n, (c, want)) in all.iter_mut().zip(candidates) {
        let mut mine: Vec<Entrance> = Vec::new();
        let spaced = |mine: &[Entrance], e: &Entrance| mine.iter().all(|m| dist(m.gate, e.gate) >= ENTRANCE_SPACING);
        for e in &c {
            if mine.len() < want && spaced(&mine, e) && !placed.iter().any(|p| clash(p, e)) {
                mine.push(e.clone());
            }
        }
        for e in &c {
            if mine.len() >= MIN_ENTRANCES {
                break;
            }
            if mine.contains(e) || !spaced(&mine, e) {
                continue;
            }
            let mut e = e.clone();
            if let Some(p) = placed.iter().find(|p| clash(p, &e)) {
                (e.at, e.along) = (p.at, p.along);
            }
            // Lining up with one may clash with another; leave it out then.
            if !placed.iter().any(|p| clash(p, &e)) {
                mine.push(e);
            }
        }
        placed.extend(mine.iter().cloned());
        n.entrances = mine;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::terrain::WorldGenParams;

    const C: P = (4000.0, 4000.0);

    fn params(seed: u64) -> SuburbanGenParams {
        WorldGenParams { seed, ..Default::default() }.suburban_params()
    }

    fn off_line(p: P, angle: f64) -> f64 {
        ((p.0 - C.0) * -angle.sin() + (p.1 - C.1) * angle.cos()).abs()
    }

    #[test]
    fn every_sector_is_covered_once_or_split_in_two() {
        for seed in [42, 7, 1234, 99999] {
            let all = neighbourhoods(&params(seed), C);
            for s in 0..8 {
                let mine: Vec<_> = all.iter().filter(|n| n.sector == s).collect();
                assert!(matches!(mine.len(), 1 | 2), "seed {seed} sector {s}: {} neighbourhoods", mine.len());
                assert!(mine.iter().all(|n| n.frame.fit.is_some()), "seed {seed} sector {s}: no rows fit");
                if mine.len() == 2 {
                    assert_eq!((mine[0].hi, mine[1].lo), (Facing::Greenway, Facing::Greenway));
                }
            }
        }
        let splits = |seed| neighbourhoods(&params(seed), C).len() - 8;
        assert!((0..20).map(splits).any(|s| s > 0) && (0..20).map(splits).any(|s| s < 8), "the split is a draw");
    }

    #[test]
    fn side_lines_sit_off_the_arterials_by_the_inset() {
        let p = params(42);
        let e = p.arterial_width / 2.0 + p.neighbourhood.buffer;
        for n in neighbourhoods(&p, C) {
            let f = &n.frame;
            for (edge, u, side) in [(Edge::Lo, 0.0, f.lo), (Edge::Hi, f.width, f.hi)] {
                let want = match n.facing(edge) {
                    Facing::Arterial(_) => e,
                    Facing::Greenway => p.neighbourhood.greenway_width / 2.0,
                };
                for j in 0..=10 {
                    let d = off_line(f.to_world(u, f.depth * j as f64 / 10.0), side.angle);
                    assert!((d - want).abs() < 0.5, "sector {} {edge:?}: {d} off, want {want}", n.sector);
                }
            }
        }
    }

    #[test]
    fn entrances_keep_their_spacing() {
        for seed in 0..200 {
            let all = neighbourhoods(&params(seed), C);
            let every: Vec<&Entrance> = all.iter().flat_map(|n| &n.entrances).collect();
            for n in &all {
                let k = n.entrances.len();
                assert!((MIN_ENTRANCES..=MAX_ENTRANCES).contains(&k), "seed {seed} s{}·{}: {k} entrances", n.sector, n.index);
                for (i, a) in n.entrances.iter().enumerate() {
                    assert!(matches!(n.facing(a.edge), Facing::Arterial(x) if x == a.arterial));
                    assert!(off_line(a.at, a.arterial as f64 * FRAC_PI_4) < 1e-6, "on the arterial");
                    for b in &n.entrances[i + 1..] {
                        assert!(dist(a.gate, b.gate) >= ENTRANCE_SPACING, "seed {seed}: entrances {} apart", dist(a.gate, b.gate));
                    }
                }
            }
            for (i, a) in every.iter().enumerate() {
                for b in &every[i + 1..] {
                    assert!(!clash(a, b), "seed {seed}: arterial {} entrances {} apart", a.arterial, (a.along - b.along).abs());
                }
            }
        }
    }

    #[test]
    fn one_neighbourhoods_stream_leaves_the_other_frames_alone() {
        let p = params(42);
        let base = neighbourhoods(&p, C);
        let (s, i) = (base[3].sector, base[3].index);
        let changed = neighbourhoods_with(&p, C, |a, b| {
            let key = if (a, b) == (s, i) { 0xdead_beef } else { b as u64 };
            SplitMix64::stream(p.seed, &[a as u64, key])
        });
        assert_eq!(base.len(), changed.len());
        for (x, y) in base.iter().zip(&changed) {
            let same = format!("{:?}", x.frame) == format!("{:?}", y.frame);
            assert_eq!(same, (x.sector, x.index) != (s, i), "s{}·{}", x.sector, x.index);
        }
    }

    #[test]
    fn neighbourhoods_are_deterministic() {
        let p = params(1234);
        let show = |all: Vec<Neighbourhood>| format!("{:?}", all.iter().map(|n| (&n.frame, &n.buffers, &n.entrances)).collect::<Vec<_>>());
        assert_eq!(show(neighbourhoods(&p, C)), show(neighbourhoods(&p, C)));
    }
}
