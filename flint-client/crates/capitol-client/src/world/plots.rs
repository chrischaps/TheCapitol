//! The plots (P3.6): every `/terrain` lot as a low slab a step above the
//! street, edged with a curb, in its zone's tint.
//!
//! - **Each lot is drawn as the server sends it.** Since B7 (#42) the
//!   server clears every lot off the roads and out of the water and its
//!   kerb (`backend/src/models/lot_clearance.rs`), so the client trusts the
//!   quad. It used to clip lots to the road ribbons itself, but on cleared
//!   lots that only shaved up to 5% off those by a bend in the road (#84).
//!   `lots_keep_off_the_roads_and_the_water` checks the fixture still holds.
//! - **Inset.** The lot is pulled in by its zone's `inset`, so
//!   neighbours sharing an edge stand apart with a seam of dark ground
//!   between them, and a lot by a road has a gutter.
//! - **Curb lip.** A band `curb` wide round the edge rises `lip` above the
//!   lot: pale stone in town, a dark hedge bank round the rural fields.
//!   The lot itself is at `layer::PLOT`, above the roads, like a pavement.
//! - **Tint.** Each zone's token pulled toward the background, a little
//!   brighter than the ground round it, and each lot a shade lighter or
//!   darker than the next (from its id), so the parcels read one by one.
//! - **One mesh per sector** (`sector`), each lot in the sector of its
//!   centre, so each part culls on its own. `Lot` records where each lot's
//!   vertices and indices sit in its sector's mesh, for P6.4 to find and
//!   highlight one. Lots are keyed by their `TerrainPlot::id`, which is
//!   unique since B7 and which a claimable plot names as
//!   `PlotInfo::terrain_id`.
//! - **Yours are gold** (P6.4, [`owned`]). Each lot you own gets a second
//!   mesh laid just over it: the curb's top in glowing gold, its outer face
//!   in dim gold, and a warm wash over the lot that's strongest at the curb
//!   and gone by its middle, as if lit from its edge. It's one small mesh,
//!   `plots_owned`, rebuilt when your plots change, so the sector meshes
//!   are never touched. `Lot` keeps the drawn outline for it.

use std::collections::HashMap;
use std::ops::Range;

use crate::coords::{layer, to_world};
use crate::world::geom::*;
use crate::world::mesh::{facing, MeshBuf, UP};
use crate::world::palette::{self, hex, mix};
use crate::world::sector::Sector;
use capitol_protocol::rules::Zone;
use capitol_protocol::terrain::TerrainPlot;
use flint_import::ImportedMaterial;
use flint_player::ExtensionGpu;

/// Points closer than this, in server units, are one point.
const MIN_STEP: f64 = 0.5;
/// How far each lot's shade wanders from its zone's, either way.
const JITTER: f32 = 0.07;

/// How one zone's lots are drawn. Lengths are server units, heights metres.
struct Style {
    /// Gap between the lot's edge and its curb.
    inset: f64,
    /// The curb's width.
    curb: f64,
    /// The curb's height above the lot.
    lip: f32,
    /// How far the lot is pulled from the background toward its zone token.
    strength: f32,
    /// The curb's colour.
    curb_colour: [f32; 4],
}

fn style(zone: Zone) -> Style {
    let stone = |k: f32| mix(hex(palette::BACKGROUND), hex(palette::DIM_TEXT), k);
    match zone {
        Zone::Suburban => Style {
            inset: 2.5,
            curb: 2.0,
            lip: 0.05,
            strength: 0.15,
            curb_colour: mix(stone(0.55), hex(palette::zone(Zone::Suburban)), 0.08),
        },
        Zone::Rural => Style {
            inset: 4.0,
            curb: 3.5,
            lip: 0.14,
            strength: 0.13,
            // A hedge bank: the zone's green, darker than the field.
            curb_colour: mix(hex(palette::BACKGROUND), hex(0x3f6212), 0.55),
        },
        // Urban, and anything else that ever carries lots.
        _ => Style {
            inset: 1.5,
            curb: 1.2,
            lip: 0.06,
            strength: 0.27,
            curb_colour: stone(0.48),
        },
    }
}

/// Where one lot's geometry sits in its sector's mesh, and the shape it
/// was drawn with.
#[derive(Clone, Debug, PartialEq)]
pub struct Lot {
    pub sector: Sector,
    pub vertices: Range<u32>,
    pub indices: Range<u32>,
    /// The drawn edge, server units, anticlockwise: inset.
    pub outline: Vec<P>,
    pub zone: Zone,
    /// The lot's own colour.
    pub colour: [f32; 4],
}

/// Every lot, built: one mesh per sector, and each lot's place in it.
#[derive(Default)]
pub struct Plots {
    pub meshes: HashMap<Sector, MeshBuf>,
    /// Where each drawn lot is, by its `TerrainPlot::id`. A lot too thin
    /// to survive its inset isn't drawn, so isn't here.
    pub lots: HashMap<String, Lot>,
}

fn centroid(poly: &[P]) -> P {
    let sum = poly.iter().fold([0.0, 0.0], |s, &p| add(s, p));
    scale(sum, 1.0 / poly.len() as f64)
}

/// The convex hull of `points`, anticlockwise in maths axes (positive
/// shoelace area), with near-duplicates merged.
fn hull(points: &[P]) -> Vec<P> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup_by(|a, b| dist(*a, *b) < MIN_STEP);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: P, a: P, b: P| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<P> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<P> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The part of the convex polygon `poly` where `dot(n, p) >= c`.
fn clip(poly: &[P], n: P, c: f64) -> Vec<P> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (dot(n, a) - c, dot(n, b) - c);
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            out.push(lerp(a, b, da / (da - db)));
        }
    }
    out.dedup_by(|a, b| dist(*a, *b) < MIN_STEP);
    if out.len() > 1 && dist(out[0], out[out.len() - 1]) < MIN_STEP {
        out.pop();
    }
    out
}

/// The anticlockwise convex polygon `poly` with every edge moved `d` in.
fn inset(poly: &[P], d: f64) -> Vec<P> {
    let mut out = poly.to_vec();
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let n = unit(left(sub(b, a)));
        out = clip(&out, n, dot(n, a) + d);
        if out.len() < 3 {
            return Vec::new();
        }
    }
    out
}

/// `poly` (anticlockwise, convex) with each corner mitred `d` in, corner
/// for corner, or `None` where an edge would vanish or turn over.
fn mitred_in(poly: &[P], d: f64) -> Option<Vec<P>> {
    let n = poly.len();
    let edge = |i: usize| unit(sub(poly[(i + 1) % n], poly[i]));
    let inner: Vec<P> = (0..n)
        .map(|i| {
            let (n0, n1) = (left(edge((i + n - 1) % n)), left(edge(i)));
            let k = 1.0 + dot(n0, n1);
            (k > 0.2).then(|| add(poly[i], scale(add(n0, n1), d / k)))
        })
        .collect::<Option<_>>()?;
    let keeps_shape = (0..n).all(|i| dot(sub(inner[(i + 1) % n], inner[i]), edge(i)) > MIN_STEP);
    keeps_shape.then_some(inner)
}

/// A number in [-1, 1] from a lot's id, the same on every run.
fn jitter(id: &str) -> f32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in id.bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    (h >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
}

fn shaded(c: [f32; 4], k: f32) -> [f32; 4] {
    [c[0] * k, c[1] * k, c[2] * k, c[3]]
}

/// A world point over server `p`, at height `y`.
fn lift(p: P, y: f32) -> [f32; 3] {
    let [x, z] = to_world(p[0], p[1]);
    [x, y, z]
}

/// A normal for the server direction `d`, level.
fn level(d: P) -> [f32; 3] {
    [d[0] as f32, 0.0, d[1] as f32]
}

/// Draws one lot, whose drawn edge is `outer`, into `m`.
fn lot(m: &mut MeshBuf, outer: &[P], s: &Style, colour: [f32; 4]) {
    let (pad, lip) = (layer::PLOT, layer::PLOT + s.lip);
    let n = outer.len();
    let walls = |m: &mut MeshBuf, poly: &[P], lo: f32, hi: f32, outward: bool, c: [f32; 4]| {
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let inward = unit(left(sub(b, a)));
            let normal = level(if outward { scale(inward, -1.0) } else { inward });
            m.quad(
                [lift(a, lo), lift(b, lo), lift(b, hi), lift(a, hi)],
                normal,
                c,
            );
        }
    };
    let fan = |m: &mut MeshBuf, poly: &[P], y: f32, c: [f32; 4]| {
        let mut rim: Vec<[f32; 3]> = poly.iter().map(|&p| lift(p, y)).collect();
        rim.push(rim[0]);
        m.fan(lift(centroid(poly), y), &rim, UP, c);
    };
    match mitred_in(outer, s.curb) {
        Some(inner) => {
            let curb = s.curb_colour;
            walls(m, outer, layer::GROUND, lip, true, shaded(curb, 0.7));
            for i in 0..n {
                let j = (i + 1) % n;
                m.quad(
                    [
                        lift(outer[i], lip),
                        lift(outer[j], lip),
                        lift(inner[j], lip),
                        lift(inner[i], lip),
                    ],
                    UP,
                    curb,
                );
            }
            walls(m, &inner, pad, lip, false, shaded(curb, 0.85));
            fan(m, &inner, pad, colour);
        }
        // Too small or too sharp for a curb: just the slab.
        None => {
            walls(m, outer, layer::GROUND, pad, true, shaded(colour, 0.7));
            fan(m, outer, pad, colour);
        }
    }
}

/// Every lot in `plots`.
pub fn build(plots: &[TerrainPlot]) -> Plots {
    let mut out = Plots::default();
    for plot in plots {
        let zone = Zone::from_str(&plot.zone).unwrap_or(Zone::Urban);
        let s = style(zone);
        let quad = hull(&plot.corners.map(|(x, y)| [x, y]));
        if quad.len() < 3 {
            continue;
        }
        let outer = inset(&quad, s.inset);
        if outer.len() < 3 {
            continue;
        }
        let base = mix(
            hex(palette::BACKGROUND),
            hex(palette::zone(zone)),
            s.strength,
        );
        let colour = shaded(base, 1.0 + JITTER * jitter(&plot.id));
        let sector = Sector::of(centroid(&outer));
        let m = out.meshes.entry(sector).or_default();
        let (v0, i0) = (m.vertices.len() as u32, m.indices.len() as u32);
        lot(m, &outer, &s, colour);
        let lot = Lot {
            sector,
            vertices: v0..m.vertices.len() as u32,
            indices: i0..m.indices.len() as u32,
            outline: outer,
            zone,
            colour,
        };
        out.lots.insert(plot.id.clone(), lot);
    }
    out
}

/// The mesh that marks your own lots in gold.
pub const OWNED_MESH: &str = "plots_owned";

/// How far over the lot and its curb the gold is laid, metres: enough to
/// win the depth test at the map's zoom, well under the curb's lip.
const OVERLAY: f32 = 0.02;
/// How bright the curb's gold glows (`palette::glow`).
const EDGE_GLOW: f32 = 0.5;
/// How far the wash comes in from the curb, as a share of the way to the
/// lot's middle, and how much gold it carries at the curb.
const WASH_REACH: f64 = 0.35;
const WASH: f32 = 0.26;
/// The edge band's width on a lot too small for a curb, server units.
const BARE_EDGE: f64 = 1.5;
/// How far the gold curb face stands out from the curb's own, server units.
const FACE_OUT: f64 = 0.3;

/// `poly` pulled toward its centroid, keeping `k` of each point's distance.
fn shrunk(poly: &[P], k: f64) -> Vec<P> {
    let c = centroid(poly);
    poly.iter().map(|&p| add(c, scale(sub(p, c), k))).collect()
}

/// A flat band round the closed ring `outer`, in to `inner` (as many
/// points), at height `y`, facing up: `colours.0` at the outer edge and
/// `colours.1` at the inner.
fn band(m: &mut MeshBuf, outer: &[P], inner: &[P], y: f32, colours: ([f32; 4], [f32; 4])) {
    let n = outer.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let corners = [
            lift(outer[i], y),
            lift(outer[j], y),
            lift(inner[j], y),
            lift(inner[i], y),
        ];
        let tints = [colours.0, colours.0, colours.1, colours.1];
        let [a, b, c, d]: [u32; 4] = std::array::from_fn(|k| m.vertex(corners[k], UP, tints[k]));
        if facing(corners[0], corners[1], corners[2], UP) {
            m.indices.extend([a, b, c, a, c, d]);
        } else {
            m.indices.extend([a, c, b, a, d, c]);
        }
    }
}

/// The gold over each of `lots`: see the module docs.
pub fn owned<'a>(lots: impl IntoIterator<Item = &'a Lot>) -> MeshBuf {
    let gold = hex(palette::GOLD);
    let glow = palette::glow(gold, EDGE_GLOW);
    let mut lit = MeshBuf::default();
    let mut light = MeshBuf::default();
    for lot in lots {
        let s = style(lot.zone);
        let outer = &lot.outline;
        let (pad, lip) = (layer::PLOT + OVERLAY, layer::PLOT + s.lip + OVERLAY);
        // The curb's top glows; a lot too small for a curb gets a thin band.
        let edge = match mitred_in(outer, s.curb) {
            Some(inner) => {
                band(&mut light, outer, &inner, lip, (glow, glow));
                // Just outside the curb's own face, so the two don't fight.
                let skin = mitred_in(outer, -FACE_OUT).unwrap_or_else(|| outer.clone());
                let face = mix(lot.colour, hex(palette::GOLD_DIM), 0.8);
                for i in 0..skin.len() {
                    let (a, b) = (skin[i], skin[(i + 1) % skin.len()]);
                    let normal = level(scale(unit(left(sub(b, a))), -1.0));
                    lit.quad(
                        [
                            lift(a, layer::GROUND),
                            lift(b, layer::GROUND),
                            lift(b, lip),
                            lift(a, lip),
                        ],
                        normal,
                        face,
                    );
                }
                inner
            }
            None => {
                let Some(inner) = mitred_in(outer, BARE_EDGE) else {
                    continue;
                };
                band(&mut light, outer, &inner, pad, (glow, glow));
                inner
            }
        };
        // The wash: gold at the curb, the lot's own colour by the middle.
        let warm = mix(lot.colour, gold, WASH);
        let core = shrunk(&edge, 1.0 - WASH_REACH);
        band(&mut lit, &edge, &core, pad, (warm, lot.colour));
        let mut rim: Vec<[f32; 3]> = core.iter().map(|&p| lift(p, pad)).collect();
        rim.push(rim[0]);
        lit.fan(lift(centroid(&core), pad), &rim, UP, lot.colour);
    }
    lit.append(light);
    lit
}

/// Builds and uploads the gold over `lots` as [`OWNED_MESH`]. With no lots,
/// the mesh is removed.
pub fn upload_owned<'a>(gpu: &mut ExtensionGpu, lots: impl IntoIterator<Item = &'a Lot>) {
    owned(lots).upload_material(
        gpu,
        ImportedMaterial {
            roughness: 0.9,
            ..ImportedMaterial::vertex_colored(OWNED_MESH)
        },
    );
}

/// The runtime mesh for `sector`'s lots.
pub fn mesh_name(sector: Sector) -> String {
    format!("plots_{}", sector.tag())
}

/// Builds and uploads every sector's lots. Returns the names of the meshes
/// with anything in them, and where each lot is (`Plots::lots`).
pub fn upload(
    gpu: &mut ExtensionGpu,
    plots: &[TerrainPlot],
) -> (Vec<String>, HashMap<String, Lot>) {
    let Plots { mut meshes, lots } = build(plots);
    let mut drawn = Vec::new();
    let mut vertices = 0;
    for s in Sector::all() {
        let name = mesh_name(s);
        let mesh = meshes.remove(&s).unwrap_or_default();
        if !mesh.indices.is_empty() {
            vertices += mesh.vertices.len();
            drawn.push(name.clone());
        }
        mesh.upload_material(
            gpu,
            ImportedMaterial {
                roughness: 0.9,
                ..ImportedMaterial::vertex_colored(&name)
            },
        );
    }
    tracing::info!(
        "plots: {} of {} lots in {} meshes, {vertices} vertices",
        lots.len(),
        plots.len(),
        drawn.len()
    );
    (drawn, lots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::to_server;
    use crate::world::mesh::facing;
    use crate::world::mesh::probe::covered;
    use crate::world::sector::WEDGES;
    use crate::world::water;
    use capitol_protocol::rules::{CENTER_X, CENTER_Y};
    use capitol_protocol::terrain::{RoadSegment, TerrainResponse};
    use std::path::Path;
    use std::sync::OnceLock;

    /// The fixture's terrain and its lots, built once for every test.
    fn fixture() -> &'static (TerrainResponse, Plots) {
        static BUILT: OnceLock<(TerrainResponse, Plots)> = OnceLock::new();
        BUILT.get_or_init(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
            let t = capitol_core::terrain::read_terrain_file(&path).expect("fixture");
            let plots = build(&t.terrain_plots);
            (t, plots)
        })
    }

    fn shoelace(poly: &[P]) -> f64 {
        let n = poly.len();
        (0..n)
            .map(|i| {
                let (a, b) = (poly[i], poly[(i + 1) % n]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            * 0.5
    }

    /// A road, with its box for skipping lots nowhere near it.
    struct Road {
        points: Vec<P>,
        half: f64,
        min: P,
        max: P,
    }

    impl Road {
        fn new(r: &RoadSegment) -> Self {
            let points: Vec<P> = r.points.iter().map(|&(x, y)| [x, y]).collect();
            let half = r.width * 0.5;
            let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in &points {
                for k in 0..2 {
                    min[k] = min[k].min(p[k] - half);
                    max[k] = max[k].max(p[k] + half);
                }
            }
            Road {
                points,
                half,
                min,
                max,
            }
        }
    }

    fn server(position: [f32; 3]) -> P {
        to_server(position[0], position[2])
    }

    fn square(side: f64) -> Vec<P> {
        vec![[0.0, 0.0], [side, 0.0], [side, side], [0.0, side]]
    }

    /// The box round a mesh's vertices, which is what the renderer culls.
    fn bounds(vertices: &[flint_render::Vertex]) -> ([f32; 3], [f32; 3]) {
        let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for v in vertices {
            for k in 0..3 {
                min[k] = min[k].min(v.position[k]);
                max[k] = max[k].max(v.position[k]);
            }
        }
        (min, max)
    }

    /// The vertices of one lot.
    fn lot_vertices<'a>(plots: &'a Plots, lot: &Lot) -> &'a [flint_render::Vertex] {
        &plots.meshes[&lot.sector].vertices[lot.vertices.start as usize..lot.vertices.end as usize]
    }

    #[test]
    fn clipping_keeps_one_side_of_a_line() {
        let half = clip(&square(10.0), [1.0, 0.0], 4.0);
        assert!((shoelace(&half) - 60.0).abs() < 1e-9);
        assert!(half.iter().all(|p| p[0] >= 4.0));
        assert_eq!(clip(&square(10.0), [1.0, 0.0], 11.0).len(), 0);
        assert_eq!(clip(&square(10.0), [1.0, 0.0], -1.0).len(), 4);
    }

    #[test]
    fn the_hull_runs_anticlockwise_whichever_way_the_corners_do() {
        let mut corners = square(10.0);
        assert!(shoelace(&hull(&corners)) > 0.0);
        corners.reverse();
        assert!((shoelace(&hull(&corners)) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn inset_and_mitre_agree_and_a_sliver_takes_no_curb() {
        let inner = inset(&square(10.0), 1.0);
        assert!((shoelace(&inner) - 64.0).abs() < 1e-9);
        let mitred = mitred_in(&square(10.0), 1.0).expect("a square takes a curb");
        assert!((shoelace(&mitred) - 64.0).abs() < 1e-9);
        let sliver = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 1.0], [0.0, 1.0]];
        assert!(mitred_in(&sliver, 1.0).is_none());
    }

    #[test]
    fn every_lot_is_drawn_under_its_own_id() {
        let (t, plots) = fixture();
        let n = t.terrain_plots.len();
        assert!(n > 2000);
        assert_eq!(plots.lots.len(), n);
        assert!(t
            .terrain_plots
            .iter()
            .all(|p| plots.lots.contains_key(&p.id)));
    }

    #[test]
    fn a_lot_is_drawn_whole_less_its_inset() {
        // Nothing is clipped off: a square lot is the square, inset.
        let plot = TerrainPlot {
            corners: [(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)],
            ..fixture().0.terrain_plots[0].clone()
        };
        let lot = &build(std::slice::from_ref(&plot)).lots[&plot.id];
        let side = 40.0 - 2.0 * style(lot.zone).inset;
        assert!((shoelace(&lot.outline) - side * side).abs() < 1e-6);
    }

    #[test]
    fn sector_bounds_contain_their_vertices() {
        // A mesh is culled by the box round its vertices, so a sector's
        // lots must keep to the sector, give or take one lot, or its box
        // spreads over its neighbours' and stops culling.
        const LOT: f64 = 260.0; // more than the widest rural lot's diagonal
        let (_, plots) = fixture();
        for (s, m) in &plots.meshes {
            let ([lo, hi], [a0, a1]) = s.span();
            for v in &m.vertices {
                let p = server(v.position);
                let r = radius(p);
                assert!(r > lo - LOT && r < hi + LOT, "{s:?}: r {r}");
                let deg = (p[1] - CENTER_Y).atan2(p[0] - CENTER_X).to_degrees();
                let slack = (LOT / r).to_degrees();
                let d = (deg - a0 + slack).rem_euclid(360.0);
                assert!(d < a1 - a0 + 2.0 * slack, "{s:?}: {deg} deg at r {r}");
            }
        }
        // And each lot is inside the box of the mesh it's recorded in.
        for lot in plots.lots.values() {
            let (min, max) = bounds(&plots.meshes[&lot.sector].vertices);
            for v in lot_vertices(plots, lot) {
                assert!((0..3).all(|k| v.position[k] >= min[k] && v.position[k] <= max[k]));
            }
        }
    }

    #[test]
    fn each_lot_is_found_in_its_sector_mesh() {
        let (t, plots) = fixture();
        for plot in &t.terrain_plots {
            let lot = &plots.lots[&plot.id];
            let m = &plots.meshes[&lot.sector];
            assert!(lot.vertices.end as usize <= m.vertices.len());
            assert!(lot.indices.end as usize <= m.indices.len());
            let own = MeshBuf {
                vertices: m.vertices.clone(),
                indices: m.indices[lot.indices.start as usize..lot.indices.end as usize].to_vec(),
            };
            assert!(
                own.indices.iter().all(|i| lot.vertices.contains(i)),
                "{}",
                plot.id
            );
            // Its own triangles cover its middle, and that's in the plot.
            let ps: Vec<P> = lot_vertices(plots, lot)
                .iter()
                .map(|v| server(v.position))
                .collect();
            let mid = centroid(&ps);
            assert!(covered(&own, to_world(mid[0], mid[1])), "{}", plot.id);
            let quad = hull(&plot.corners.map(|(x, y)| [x, y]));
            assert!((0..quad.len()).all(|i| {
                let (a, b) = (quad[i], quad[(i + 1) % quad.len()]);
                dot(left(sub(b, a)), sub(mid, a)) > 0.0
            }));
        }
    }

    #[test]
    fn lots_keep_off_the_roads_and_the_water() {
        let (t, plots) = fixture();
        let bands = water::kerbed(&t.water_rings);
        let roads: Vec<Road> = t.roads.iter().map(Road::new).collect();
        for m in plots.meshes.values() {
            for v in &m.vertices {
                let p = server(v.position);
                let r = radius(p);
                for &(lo, hi) in &bands {
                    assert!(r <= lo || r >= hi, "a lot in the water at r {r}");
                }
                // Outside a road's box is more than half its width away.
                let near =
                    |road: &&Road| (0..2).all(|k| p[k] >= road.min[k] && p[k] <= road.max[k]);
                for road in roads.iter().filter(near) {
                    for w in road.points.windows(2) {
                        let d = distance_to_segment(p, w[0], w[1]);
                        assert!(
                            d >= road.half - 0.01,
                            "a lot on a road at {p:?}: {d} < {}",
                            road.half
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn neighbours_stand_apart() {
        // Every vertex is at least its zone's inset inside the lot's quad,
        // so two lots sharing an edge have a seam of ground between them.
        let (t, plots) = fixture();
        for plot in &t.terrain_plots {
            let lot = &plots.lots[&plot.id];
            let s = style(Zone::from_str(&plot.zone).unwrap());
            let quad = hull(&plot.corners.map(|(x, y)| [x, y]));
            for v in lot_vertices(plots, lot) {
                let p = server(v.position);
                for i in 0..quad.len() {
                    let (a, b) = (quad[i], quad[(i + 1) % quad.len()]);
                    let inward = dot(unit(left(sub(b, a))), sub(p, a));
                    assert!(inward >= s.inset - 0.01, "{}: {inward}", plot.id);
                }
            }
        }
    }

    #[test]
    fn every_face_faces_its_normal_and_the_tops_are_level() {
        let (_, plots) = fixture();
        for m in plots.meshes.values() {
            for t in m.indices.chunks_exact(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]);
                let n = a.normal;
                assert!(facing(a.position, b.position, c.position, n));
                if n == UP {
                    let y = a.position[1];
                    assert!((layer::PLOT..layer::BANK).contains(&y), "a top at {y}");
                    assert!([b, c].iter().all(|v| v.position[1] == y));
                } else {
                    assert_eq!(n[1], 0.0, "walls stand upright");
                }
            }
        }
    }

    #[test]
    fn at_street_zoom_most_plot_sectors_are_culled() {
        // P3.9's rig at street level, about 50 degrees down from 25 m, over
        // a player in the urban ring, then in the suburbs, then out in the
        // fields. The scene's near and far planes, the player's 70 degree
        // field of view and a 16:9 window.
        let (_, plots) = fixture();
        let total = plots.meshes.len();
        assert!(
            total >= 3 * WEDGES as usize - 4,
            "only {total} sectors hold lots"
        );
        for (r, deg) in [(1300.0, 90.0), (2000.0, 200.0), (2900.0, 315.0)] {
            let [x, z] = crate::coords::polar_to_world(r, deg);
            let mut camera = flint_render::Camera::default();
            (camera.position.x, camera.position.y, camera.position.z) = (x, 25.0, z + 21.0);
            (camera.target.x, camera.target.y, camera.target.z) = (x, 0.0, z);
            camera.fov = 70.0;
            camera.near = 1.0;
            camera.far = 4000.0;
            camera.aspect = 16.0 / 9.0;
            let frustum =
                flint_render::Frustum::from_view_projection(&camera.view_projection_matrix());
            let drawn = plots
                .meshes
                .values()
                .filter(|m| {
                    let (min, max) = bounds(&m.vertices);
                    frustum.aabb_visible(min, max)
                })
                .count();
            assert!(
                drawn * 2 < total,
                "at r {r}: {drawn} of {total} sectors drawn"
            );
        }
    }

    #[test]
    fn an_owned_lot_is_edged_and_washed_in_gold_over_its_own_ground() {
        use crate::world::mesh::is_light;
        let (_, plots) = fixture();
        let lot = plots
            .lots
            .values()
            .find(|l| l.zone == Zone::Suburban)
            .unwrap();
        let m = owned([lot]);
        assert!(!m.indices.is_empty());
        let s = style(lot.zone);
        // On the drawn lot (the gold face just outside its curb), and just
        // over what it covers.
        let n = lot.outline.len();
        for v in &m.vertices {
            let p = server(v.position);
            for i in 0..n {
                let (a, b) = (lot.outline[i], lot.outline[(i + 1) % n]);
                let inward = dot(unit(left(sub(b, a))), sub(p, a));
                assert!(
                    inward >= -FACE_OUT - 0.01,
                    "{p:?} outside the lot: {inward}"
                );
            }
            assert!(v.position[1] <= layer::PLOT + s.lip + OVERLAY + 1e-4);
        }
        // Its tops face up (the glow keeps its winding), and the glow is gold.
        for t in m.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]);
            if a.normal[1] != 0.0 {
                assert!(facing(a.position, b.position, c.position, UP));
            }
        }
        let glows: Vec<_> = m.vertices.iter().filter(|v| is_light(v)).collect();
        assert!(!glows.is_empty());
        assert!(
            glows.iter().all(|v| v.color[0] > v.color[2]),
            "gold is warm"
        );
        // The middle is the lot's own colour, so the wash fades into it.
        let mid = centroid(&lot.outline);
        let near = |v: &&flint_render::Vertex| dist(server(v.position), mid);
        let middle = m
            .vertices
            .iter()
            .min_by(|a, b| near(a).total_cmp(&near(b)))
            .unwrap();
        assert_eq!(middle.color, lot.colour);
        assert!(owned([]).indices.is_empty());
    }

    #[test]
    fn every_sector_has_its_own_mesh_name() {
        let mut names: Vec<_> = Sector::all().map(mesh_name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Sector::all().count());
    }
}
