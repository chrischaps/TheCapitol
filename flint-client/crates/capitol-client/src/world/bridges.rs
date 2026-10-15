//! The bridges (P3.5): a humped deck over each crossing, carried on an
//! arch with the canal running underneath, and rails along both sides.
//!
//! A deck is exactly its bridge's nominal width, with straight parallel
//! sides, and runs along the bridge's axis from road end to road end: the
//! roads stop at the kerb's land face (`water::kerbed`), and a short ramp
//! past each end climbs off the road onto the deck. Seen from above, the
//! deck covers what the 2D clients draw as the bridge. The server is more
//! generous: it lets a move cross anywhere within twice the width
//! (`Bridge::angular_half_width`), so everywhere a deck is drawn is
//! walkable.
//!
//! In profile the deck is one gentle hump from toe to toe, high enough to
//! clear the kerbs. Under it, an elliptical arch springs from the channel
//! walls just below the waves, so the canal runs on beneath the bridge and
//! the sea shows through it from the quay. The deck is the 2D clients'
//! bridge colour; the rails, posts and the faces of the arch are its edge
//! colour, and each rail ends at a newel post capped in the Capitol's gold.

use crate::coords::{layer, to_world, S};
use crate::world::mesh::{facing, MeshBuf};
use crate::world::palette::{self, hex, mix};
use crate::world::water;
use capitol_protocol::rules::{CENTER_X, CENTER_Y};
use capitol_protocol::terrain::{RoadType, WaterRing};
use flint_player::ExtensionGpu;
use std::f64::consts::PI;

/// The runtime mesh's name, as the scene's bridges entity names it.
pub const MESH: &str = "bridges";

/// The 2D clients' bridge fill and edge.
const DECK: u32 = 0x8b6914;
const EDGE: u32 = 0x5d4612;

/// How far each ramp reaches past the kerb onto the road, in server units.
const RAMP: f64 = 20.0;
/// Where a ramp's toe sits: on top of the greatest road it lands on.
fn toe() -> f32 {
    layer::road(RoadType::Arterial) + 0.02
}
/// The deck's thickness at the crown of its arch, in metres.
const CROWN_THICKNESS: f32 = 0.18;
/// Sample spacing along a deck's land ends, in server units.
const STEP: f64 = 2.0;
/// Samples across the arch, spaced evenly in angle so the springs, where
/// it curves fastest, get the most.
const ARCH_STEPS: usize = 24;

/// Rails: height of the top rail and the middle one above the deck, the
/// rail's square section, and how far posts stand in from the deck's edge
/// (metres). Posts are spaced about `POST_GAP` server units apart.
const RAIL_HEIGHT: f32 = 0.9;
const MID_RAIL: f32 = 0.45;
const RAIL: f32 = 0.08;
const POST: f32 = 0.11;
const NEWEL: f32 = 0.22;
const NEWEL_HEIGHT: f32 = 1.1;
const INSET: f32 = 0.12;
const POST_GAP: f64 = 12.0;

/// One bridge in its own frame: `s` runs out from the world centre along
/// its axis and `o` across it, both in server units.
#[derive(Debug, Clone, Copy)]
struct Span {
    /// The axis and the across direction, in server (x, y).
    axis: [f64; 2],
    across: [f64; 2],
    /// Half the bridge's width.
    half: f64,
    /// The toes of the two ramps.
    start: f64,
    end: f64,
    /// Where the channel's walls meet the deck's sides: the arch's springs.
    spring_in: f64,
    spring_out: f64,
}

impl Span {
    fn new(ring: &WaterRing, angle_degrees: f64, width: f64) -> Self {
        let a = angle_degrees.to_radians();
        let half = width * 0.5;
        let kerb = (water::KERB / S) as f64;
        let side = |r: f64| (r * r - half * half).max(0.0).sqrt();
        Self {
            axis: [a.cos(), a.sin()],
            across: [-a.sin(), a.cos()],
            half,
            start: ring.inner_radius() - kerb - RAMP,
            end: ring.outer_radius() + kerb + RAMP,
            spring_in: side(ring.inner_radius()),
            spring_out: side(ring.outer_radius()),
        }
    }

    /// World (X, Z) at `s` along the axis and `o` across it.
    fn at(&self, s: f64, o: f64) -> [f32; 2] {
        to_world(
            CENTER_X + self.axis[0] * s + self.across[0] * o,
            CENTER_Y + self.axis[1] * s + self.across[1] * o,
        )
    }

    fn point(&self, s: f64, o: f64, y: f32) -> [f32; 3] {
        let [x, z] = self.at(s, o);
        [x, y, z]
    }

    /// The axis and the across direction as world vectors.
    fn axis_w(&self) -> [f32; 3] {
        [self.axis[0] as f32, 0.0, self.axis[1] as f32]
    }
    fn across_w(&self) -> [f32; 3] {
        [self.across[0] as f32, 0.0, self.across[1] as f32]
    }

    /// The deck's top at `s`: one hump from toe to toe.
    fn top(&self, s: f64) -> f32 {
        let u = ((s - self.start) / (self.end - self.start)).clamp(0.0, 1.0);
        toe() + (layer::DECK - toe()) * (PI * u).sin() as f32
    }

    /// The slope of the top at `s`, in metres per metre.
    fn top_slope(&self, s: f64) -> f32 {
        let len = self.end - self.start;
        let u = ((s - self.start) / len).clamp(0.0, 1.0);
        let dy_ds = (layer::DECK - toe()) as f64 * PI / len * (PI * u).cos();
        (dy_ds / S as f64) as f32
    }

    /// The arch's centre and half its span, along the axis.
    fn arch(&self) -> (f64, f64) {
        (
            (self.spring_in + self.spring_out) * 0.5,
            (self.spring_out - self.spring_in) * 0.5,
        )
    }

    /// The underside of the arch at angle `phi` (0 at the inner spring, π
    /// at the outer): where along the axis, and how high.
    fn soffit(&self, phi: f64) -> (f64, f32) {
        let (mid, half) = self.arch();
        let crown = self.top(mid) - CROWN_THICKNESS;
        let s = mid - half * phi.cos();
        (
            s,
            layer::ARCH_SPRING + (crown - layer::ARCH_SPRING) * phi.sin() as f32,
        )
    }

    /// The samples along the deck: the land at each end every `STEP`, and
    /// the arch between, with the springs themselves shared.
    fn samples(&self) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let land = |a: f64, b: f64| {
            let n = ((b - a) / STEP).ceil().max(1.0) as usize;
            (0..=n)
                .map(|i| a + (b - a) * i as f64 / n as f64)
                .collect::<Vec<_>>()
        };
        let arch = (0..=ARCH_STEPS)
            .map(|i| self.soffit(PI * i as f64 / ARCH_STEPS as f64).0)
            .collect();
        (
            land(self.start, self.spring_in),
            arch,
            land(self.spring_out, self.end),
        )
    }
}

/// Every bridge in these rings, in its own frame.
fn spans(rings: &[WaterRing]) -> Vec<Span> {
    rings
        .iter()
        .flat_map(|ring| {
            ring.bridges
                .iter()
                .map(move |b| Span::new(ring, b.angle_degrees, b.width))
        })
        .collect()
}

fn normalise([x, y, z]: [f32; 3]) -> [f32; 3] {
    let len = (x * x + y * y + z * z).sqrt().max(1e-6);
    [x / len, y / len, z / len]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// A smooth sheet through `rows`, each a pair of points with the normal
/// they share. Consecutive rows make quads, wound to face their normals.
fn sheet(m: &mut MeshBuf, rows: &[([f32; 3], [f32; 3], [f32; 3])], colour: [f32; 4]) {
    for w in rows.windows(2) {
        let ((a, b, n0), (c, d, n1)) = (w[0], w[1]);
        let n = normalise(add(n0, n1));
        let [ia, ib] = [(a, n0), (b, n0)].map(|(p, n)| m.vertex(p, n, colour));
        let [ic, id] = [(c, n1), (d, n1)].map(|(p, n)| m.vertex(p, n, colour));
        // Round the quad: a, b, then d, c across the next row.
        if facing(a, b, d, n) {
            m.indices.extend([ia, ib, id, ia, id, ic]);
        } else {
            m.indices.extend([ia, id, ib, ia, ic, id]);
        }
    }
}

/// An upright box at (`s`, `o`), `half` metres either side in both
/// directions, from `y0` up to `y1`, with a top of its own colour.
fn post(
    m: &mut MeshBuf,
    span: &Span,
    (s, o): (f64, f64),
    half: f32,
    (y0, y1): (f32, f32),
    colour: [f32; 4],
    cap: [f32; 4],
) {
    let h = (half / S) as f64;
    let (ax, ac) = (span.axis_w(), span.across_w());
    let corner = |ds: f64, d_o: f64, y: f32| span.point(s + ds * h, o + d_o * h, y);
    let sides = [
        ((1.0, -1.0), (1.0, 1.0), ax),
        ((1.0, 1.0), (-1.0, 1.0), ac),
        ((-1.0, 1.0), (-1.0, -1.0), scale(ax, -1.0)),
        ((-1.0, -1.0), (1.0, -1.0), scale(ac, -1.0)),
    ];
    for ((s0, o0), (s1, o1), n) in sides {
        m.quad(
            [
                corner(s0, o0, y0),
                corner(s1, o1, y0),
                corner(s1, o1, y1),
                corner(s0, o0, y1),
            ],
            n,
            colour,
        );
    }
    m.quad(
        [
            corner(-1.0, -1.0, y1),
            corner(1.0, -1.0, y1),
            corner(1.0, 1.0, y1),
            corner(-1.0, 1.0, y1),
        ],
        [0.0, 1.0, 0.0],
        cap,
    );
}

/// A square rail `RAIL` thick swept along `s` from `s0` to `s1` at `o`,
/// its centre `height` above the deck.
fn rail(m: &mut MeshBuf, span: &Span, o: f64, (s0, s1): (f64, f64), height: f32, colour: [f32; 4]) {
    let n = ((s1 - s0) / STEP).ceil().max(1.0) as usize;
    let along: Vec<f64> = (0..=n)
        .map(|i| s0 + (s1 - s0) * i as f64 / n as f64)
        .collect();
    let r = (RAIL * 0.5 / S) as f64;
    let half = RAIL * 0.5;
    let (ax, ac) = (span.axis_w(), span.across_w());
    let up_at = |s: f64| normalise(add([0.0, 1.0, 0.0], scale(ax, -span.top_slope(s))));
    let y = |s: f64| span.top(s) + height;
    // The top and bottom faces, then each side.
    for (dy, facing_up) in [(half, 1.0), (-half, -1.0)] {
        let rows: Vec<_> = along
            .iter()
            .map(|&s| {
                (
                    span.point(s, o - r, y(s) + dy),
                    span.point(s, o + r, y(s) + dy),
                    scale(up_at(s), facing_up),
                )
            })
            .collect();
        sheet(m, &rows, colour);
    }
    for (side, n) in [(r, ac), (-r, scale(ac, -1.0))] {
        let rows: Vec<_> = along
            .iter()
            .map(|&s| {
                (
                    span.point(s, o + side, y(s) - half),
                    span.point(s, o + side, y(s) + half),
                    n,
                )
            })
            .collect();
        sheet(m, &rows, colour);
    }
}

/// One bridge: deck, arch, sides, ramps' ends and rails.
fn bridge(m: &mut MeshBuf, span: &Span) {
    let deck = hex(DECK);
    let edge = hex(EDGE);
    // The vault is in its own shadow.
    let vault = mix(edge, hex(palette::BACKGROUND), 0.35);
    let gold = hex(palette::GOLD);
    let (ax, ac) = (span.axis_w(), span.across_w());
    let h = span.half;
    let (land_in, arch, land_out) = span.samples();
    let all: Vec<f64> = land_in
        .iter()
        .chain(&arch[1..arch.len() - 1])
        .chain(&land_out)
        .copied()
        .collect();

    // The deck's top.
    let up_at = |s: f64| normalise(add([0.0, 1.0, 0.0], scale(ax, -span.top_slope(s))));
    let rows: Vec<_> = all
        .iter()
        .map(|&s| {
            (
                span.point(s, -h, span.top(s)),
                span.point(s, h, span.top(s)),
                up_at(s),
            )
        })
        .collect();
    sheet(m, &rows, deck);

    // The arch's underside, facing down into the opening.
    let rows: Vec<_> = (0..=ARCH_STEPS)
        .map(|i| {
            let phi = PI * i as f64 / ARCH_STEPS as f64;
            let (s, y) = span.soffit(phi);
            // The soffit's tangent in (along, up), in metres per radian,
            // turned to point down into the opening.
            let (mid, half) = span.arch();
            let rise = span.top(mid) - CROWN_THICKNESS - layer::ARCH_SPRING;
            let (ds, dy) = (
                (half as f32) * S * phi.sin() as f32,
                rise * phi.cos() as f32,
            );
            let n = normalise(add(scale(ax, dy), [0.0, -ds, 0.0]));
            (span.point(s, -h, y), span.point(s, h, y), n)
        })
        .collect();
    sheet(m, &rows, vault);

    // Each side: down to the land at the ends, down to the arch between.
    for (o, n) in [(h, ac), (-h, scale(ac, -1.0))] {
        let side =
            |s: f64, bottom: f32| (span.point(s, o, bottom), span.point(s, o, span.top(s)), n);
        let ground: Vec<_> = land_in.iter().map(|&s| side(s, layer::GROUND)).collect();
        sheet(m, &ground, edge);
        let over: Vec<_> = (0..=ARCH_STEPS)
            .map(|i| {
                let (s, y) = span.soffit(PI * i as f64 / ARCH_STEPS as f64);
                side(s, y)
            })
            .collect();
        sheet(m, &over, edge);
        let ground: Vec<_> = land_out.iter().map(|&s| side(s, layer::GROUND)).collect();
        sheet(m, &ground, edge);
    }

    // The ramps' toes.
    for (s, n) in [(span.start, scale(ax, -1.0)), (span.end, ax)] {
        m.quad(
            [
                span.point(s, -h, layer::GROUND),
                span.point(s, h, layer::GROUND),
                span.point(s, h, span.top(s)),
                span.point(s, -h, span.top(s)),
            ],
            n,
            edge,
        );
    }

    // Rails along both sides, between a newel post at each end, with
    // posts between. They start halfway up each ramp.
    let (r0, r1) = (span.start + RAMP * 0.5, span.end - RAMP * 0.5);
    let o = h - ((INSET + NEWEL * 0.5) / S) as f64;
    let foot = |s: f64| span.top(s) - 0.05;
    for o in [o, -o] {
        for s in [r0, r1] {
            post(
                m,
                span,
                (s, o),
                NEWEL * 0.5,
                (foot(s), span.top(s) + NEWEL_HEIGHT),
                edge,
                gold,
            );
        }
        let posts = ((r1 - r0) / POST_GAP).round().max(1.0) as usize;
        for i in 1..posts {
            let s = r0 + (r1 - r0) * i as f64 / posts as f64;
            post(
                m,
                span,
                (s, o),
                POST * 0.5,
                (foot(s), span.top(s) + RAIL_HEIGHT),
                edge,
                edge,
            );
        }
        rail(m, span, o, (r0, r1), RAIL_HEIGHT - RAIL * 0.5, edge);
        rail(m, span, o, (r0, r1), MID_RAIL, edge);
    }
}

/// Every bridge in these rings, as one mesh.
fn build(rings: &[WaterRing]) -> MeshBuf {
    let mut m = MeshBuf::default();
    for span in spans(rings) {
        bridge(&mut m, &span);
    }
    m
}

/// Upload the bridges for these rings (once per terrain).
pub fn upload(gpu: &mut ExtensionGpu, rings: &[WaterRing]) {
    build(rings).upload(gpu, MESH, 0.8);
}

/// Every deck's footprint, for standing on it (P4.2): an avatar on a
/// bridge stands on the hump, not at the road's height under it. Built
/// once per terrain, so a lookup doesn't allocate.
#[derive(Debug, Clone, Default)]
pub struct Decks(Vec<Span>);

impl Decks {
    pub fn new(rings: &[WaterRing]) -> Self {
        Self(spans(rings))
    }

    /// The deck's top in metres under server (x, y), if a deck is there:
    /// across the bridge's width, from ramp toe to ramp toe.
    pub fn top(&self, x: f64, y: f64) -> Option<f32> {
        let (dx, dy) = (x - CENTER_X, y - CENTER_Y);
        self.0.iter().find_map(|span| {
            let s = dx * span.axis[0] + dy * span.axis[1];
            let o = dx * span.across[0] + dy * span.across[1];
            (o.abs() <= span.half && (span.start..=span.end).contains(&s)).then(|| span.top(s))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::polar_to_world;
    use crate::world::mesh::probe::{covered, triangles};
    use std::path::Path;

    fn rings() -> Vec<WaterRing> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = capitol_core::terrain::read_terrain_file(&path).expect("fixture");
        terrain.water_rings
    }

    /// Only the faces that look up, as a walker sees the deck from above.
    fn from_above(m: &MeshBuf) -> MeshBuf {
        let mut out = MeshBuf::default();
        for tri in triangles(m) {
            if tri[0].normal[1] > 0.5 {
                let i = out.vertices.len() as u32;
                out.vertices.extend(tri);
                out.indices.extend([i, i + 1, i + 2]);
            }
        }
        out
    }

    #[test]
    fn each_deck_is_exactly_its_bridge_wide_from_toe_to_toe() {
        for ring in rings() {
            for b in &ring.bridges {
                let span = Span::new(&ring, b.angle_degrees, b.width);
                let mut m = MeshBuf::default();
                bridge(&mut m, &span);
                let top = from_above(&m);
                let h = span.half;
                for s in [
                    span.start + 0.5,
                    span.spring_in,
                    ring.radius,
                    span.spring_out,
                    span.end - 0.5,
                ] {
                    for side in [-1.0, 1.0] {
                        let what =
                            format!("{} bridge {} at {s}, side {side}", ring.id, b.angle_degrees);
                        assert!(
                            covered(&top, span.at(s, side * (h - 0.5))),
                            "deck at {what}"
                        );
                        assert!(
                            !covered(&m, span.at(s, side * (h + 0.5))),
                            "beside the deck at {what}"
                        );
                    }
                }
                for s in [span.start - 0.5, span.end + 0.5] {
                    assert!(
                        !covered(&m, span.at(s, 0.0)),
                        "{} past a toe at {s}",
                        ring.id
                    );
                }
            }
        }
    }

    #[test]
    fn decks_stand_only_at_bridges() {
        let rings = rings();
        let m = build(&rings);
        for ring in &rings {
            for step in 0..1440 {
                let deg = step as f64 * 0.25;
                let a = deg.to_radians();
                let (x, y) = (ring.radius * a.cos(), ring.radius * a.sin());
                // Distance outside the nearest bridge's sides.
                let off = ring
                    .bridges
                    .iter()
                    .map(|b| {
                        let (by, bx) = b.angle_degrees.to_radians().sin_cos();
                        if x * bx + y * by > 0.0 {
                            (x * by - y * bx).abs() - b.width * 0.5
                        } else {
                            f64::INFINITY
                        }
                    })
                    .fold(f64::INFINITY, f64::min);
                if off.abs() < 1.0 {
                    continue; // on a deck's edge
                }
                let p = polar_to_world(ring.radius, deg);
                assert_eq!(covered(&m, p), off < 0.0, "{} at {deg}°", ring.id);
            }
        }
    }

    #[test]
    fn a_deck_meets_the_road_ends_and_clears_the_kerbs() {
        for ring in rings() {
            let kerbed = water::kerbed(std::slice::from_ref(&ring))[0];
            for b in &ring.bridges {
                let span = Span::new(&ring, b.angle_degrees, b.width);
                // The roads end at the kerbed band; the ramps reach past it.
                assert!(span.start < kerbed.0 && span.end > kerbed.1);
                // From where the kerb starts on either side, to where it
                // ends, the deck is above it.
                let side = |r: f64| (r * r - span.half * span.half).sqrt();
                for i in 0..=100 {
                    let s = side(kerbed.0) + (kerbed.1 - side(kerbed.0)) * i as f64 / 100.0;
                    assert!(span.top(s) > layer::BANK + 0.05, "{} at {s}", ring.id);
                }
                assert!(span.top(span.start) > layer::road(RoadType::Arterial));
            }
        }
    }

    #[test]
    fn the_water_runs_under_an_arch_that_springs_below_the_waves() {
        for ring in rings() {
            for b in &ring.bridges {
                let span = Span::new(&ring, b.angle_degrees, b.width);
                let (spring_in, y_in) = span.soffit(0.0);
                let (spring_out, y_out) = span.soffit(PI);
                assert!((spring_in - span.spring_in).abs() < 1e-9);
                assert!((spring_out - span.spring_out).abs() < 1e-9);
                assert!(y_in < layer::SEA && y_out < layer::SEA);
                // Headroom over the water at the crown, and the deck is
                // never thinner than at its crown.
                assert!(span.soffit(PI * 0.5).1 > layer::SEA + 0.8, "{}", ring.id);
                for i in 0..=ARCH_STEPS {
                    let (s, y) = span.soffit(PI * i as f64 / ARCH_STEPS as f64);
                    assert!(
                        span.top(s) - y >= CROWN_THICKNESS - 1e-4,
                        "{} at {s}",
                        ring.id
                    );
                }
            }
        }
    }

    #[test]
    fn the_server_lets_you_cross_wherever_a_deck_is_drawn() {
        for ring in rings() {
            for b in &ring.bridges {
                let span = Span::new(&ring, b.angle_degrees, b.width);
                for s in [span.spring_in, ring.radius, span.spring_out] {
                    for o in [-span.half, span.half] {
                        let x = span.axis[0] * s + span.across[0] * o;
                        let y = span.axis[1] * s + span.across[1] * o;
                        let angle = y.atan2(x).to_degrees();
                        assert!(b.contains_angle(angle, ring.radius), "{} {angle}", ring.id);
                    }
                }
            }
        }
    }

    #[test]
    fn a_walker_stands_on_the_deck_from_toe_to_crown() {
        let rings = rings();
        let decks = Decks::new(&rings);
        let ring = &rings[0];
        let b = &ring.bridges[0];
        let span = Span::new(ring, b.angle_degrees, b.width);
        let at = |s: f64, o: f64| {
            (
                CENTER_X + span.axis[0] * s + span.across[0] * o,
                CENTER_Y + span.axis[1] * s + span.across[1] * o,
            )
        };
        // At the crown, on the deck's height, which is above the road's.
        let (x, y) = at(ring.radius, 0.0);
        let crown = decks.top(x, y).expect("on the deck");
        assert!(crown > layer::BANK, "{crown}");
        // At a toe, level with the road the ramp lands on.
        let (x, y) = at(span.start + 0.01, 0.0);
        assert!((decks.top(x, y).unwrap() - toe()).abs() < 0.01);
        // Past the toe, or over the rail, it's the road again.
        let (x, y) = at(span.start - 1.0, 0.0);
        assert_eq!(decks.top(x, y), None);
        let (x, y) = at(ring.radius, span.half + 1.0);
        assert_eq!(decks.top(x, y), None);
    }

    #[test]
    fn every_face_faces_its_normal() {
        let m = build(&rings());
        assert!(!m.indices.is_empty());
        for v in triangles(&m) {
            let n = normalise(add(add(v[0].normal, v[1].normal), v[2].normal));
            assert!(facing(v[0].position, v[1].position, v[2].position, n));
        }
    }
}
