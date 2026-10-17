//! The street network as a graph, for the plan metrics.
//!
//! `street_plan::weld_junctions` makes every junction a vertex that the roads
//! meeting there share, so the nodes are just the distinct road vertices and
//! the links run between neighbouring vertices of each road. A node is a
//! junction when three or more links meet there from at least two roads. A
//! lollipop's bulb closes on its own stem, which gives that vertex three
//! links from one road; that's the end of a cul-de-sac, not a T.

use std::collections::{HashMap, VecDeque};

use capitol_protocol::terrain::{RoadSegment, RoadType};

type P = (f64, f64);

/// Vertices closer than this are one node. Road points are at least
/// `street_plan::MIN_STEP` apart, and welded vertices are exact copies.
const SAME: f64 = 1e-4;

#[derive(Debug, Clone)]
pub struct Node {
    pub at: P,
    /// The neighbouring nodes, each with the road that links them. A
    /// neighbour reached by two roads (a shared stretch) is listed once.
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Link {
    pub to: usize,
    pub road: usize,
}

/// One road as a walk over the nodes.
#[derive(Debug, Clone)]
pub struct Walk {
    pub nodes: Vec<usize>,
    /// Distance along the road to each node.
    pub along: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JunctionKind {
    /// Three ways: a T, or a Y.
    Tee,
    /// Four or more ways.
    FourWay,
}

#[derive(Debug, Clone)]
pub struct Junction {
    pub node: usize,
    pub kind: JunctionKind,
    /// The narrowest angle between two neighbouring ways, in degrees.
    pub min_angle: f64,
}

/// A street that ends without coming back to the network: a dead end, or a
/// lollipop whose bulb closes on its own stem.
#[derive(Debug, Clone)]
pub struct CulDeSac {
    pub road: usize,
    /// From the network to the end of the street, or to where the bulb closes.
    pub stem: f64,
    /// The dead end, or the vertex where the bulb closes.
    pub end: P,
    pub bulb: bool,
}

pub struct Graph {
    pub nodes: Vec<Node>,
    pub walks: Vec<Walk>,
    pub road_types: Vec<RoadType>,
}

fn dist(a: P, b: P) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

impl Graph {
    pub fn build(roads: &[RoadSegment]) -> Self {
        let key = |p: P| ((p.0 / SAME).round() as i64, (p.1 / SAME).round() as i64);
        let mut index: HashMap<(i64, i64), usize> = HashMap::new();
        let mut nodes: Vec<Node> = Vec::new();
        let mut walks = Vec::with_capacity(roads.len());
        for (r, road) in roads.iter().enumerate() {
            let mut walk = Walk { nodes: Vec::new(), along: Vec::new() };
            let mut along = 0.0;
            for (k, &p) in road.points.iter().enumerate() {
                if k > 0 {
                    along += dist(road.points[k - 1], p);
                }
                let n = *index.entry(key(p)).or_insert_with(|| {
                    nodes.push(Node { at: p, links: Vec::new() });
                    nodes.len() - 1
                });
                if let Some(&prev) = walk.nodes.last() {
                    if prev != n {
                        for (a, b) in [(prev, n), (n, prev)] {
                            if !nodes[a].links.iter().any(|l| l.to == b) {
                                nodes[a].links.push(Link { to: b, road: r });
                            }
                        }
                    }
                }
                walk.nodes.push(n);
                walk.along.push(along);
            }
            walks.push(walk);
        }
        Self { nodes, walks, road_types: roads.iter().map(|r| r.road_type).collect() }
    }

    pub fn degree(&self, n: usize) -> usize {
        self.nodes[n].links.len()
    }

    /// The distinct roads with a link at `n`, in order.
    pub fn roads_at(&self, n: usize) -> Vec<usize> {
        let mut roads: Vec<usize> = self.nodes[n].links.iter().map(|l| l.road).collect();
        roads.sort_unstable();
        roads.dedup();
        roads
    }

    pub fn junction(&self, n: usize) -> Option<Junction> {
        let degree = self.degree(n);
        if degree < 3 || self.roads_at(n).len() < 2 {
            return None;
        }
        let at = self.nodes[n].at;
        let mut bearings: Vec<f64> = self.nodes[n]
            .links
            .iter()
            .map(|l| {
                let to = self.nodes[l.to].at;
                (to.1 - at.1).atan2(to.0 - at.0).to_degrees()
            })
            .collect();
        bearings.sort_by(f64::total_cmp);
        let min_angle = (0..bearings.len())
            .map(|i| {
                let next = if i + 1 < bearings.len() { bearings[i + 1] } else { bearings[0] + 360.0 };
                next - bearings[i]
            })
            .fold(f64::INFINITY, f64::min);
        let kind = if degree == 3 { JunctionKind::Tee } else { JunctionKind::FourWay };
        Some(Junction { node: n, kind, min_angle })
    }

    pub fn junctions(&self) -> Vec<Junction> {
        (0..self.nodes.len()).filter_map(|n| self.junction(n)).collect()
    }

    /// The gaps between neighbouring junctions along road `r`. Its own ends
    /// count when they're junctions, which they are where it joins its parent.
    pub fn junction_spacing(&self, r: usize) -> Vec<(f64, P)> {
        let walk = &self.walks[r];
        let mut gaps = Vec::new();
        let mut last: Option<f64> = None;
        for (k, &n) in walk.nodes.iter().enumerate() {
            if self.junction(n).is_none() {
                continue;
            }
            let here = walk.along[k];
            if let Some(prev) = last {
                if here > prev {
                    gaps.push((here - prev, self.nodes[n].at));
                }
            }
            last = Some(here);
        }
        gaps
    }

    /// Which nodes can be reached from the arterials.
    pub fn reached_from_arterials(&self) -> Vec<bool> {
        let mut seen = vec![false; self.nodes.len()];
        let mut queue = VecDeque::new();
        for (r, walk) in self.walks.iter().enumerate() {
            if self.road_types[r] != RoadType::Arterial {
                continue;
            }
            for &n in &walk.nodes {
                if !seen[n] {
                    seen[n] = true;
                    queue.push_back(n);
                }
            }
        }
        while let Some(n) = queue.pop_front() {
            for l in &self.nodes[n].links {
                if !seen[l.to] {
                    seen[l.to] = true;
                    queue.push_back(l.to);
                }
            }
        }
        seen
    }

    /// Road `r` as a cul-de-sac, if it is one: an end that is a dead end, or
    /// a bulb closing on the road's own stem, with the other end on the
    /// network.
    pub fn cul_de_sac(&self, r: usize) -> Option<CulDeSac> {
        let walk = &self.walks[r];
        let n = walk.nodes.len();
        if n < 2 {
            return None;
        }
        let total = walk.along[n - 1];
        // Last end first: streets are drawn from where they join.
        for from_start in [false, true] {
            let end = if from_start { walk.nodes[0] } else { walk.nodes[n - 1] };
            let other = if from_start { walk.nodes[n - 1] } else { walk.nodes[0] };
            if self.degree(other) < 2 && !walk.nodes[1..n - 1].contains(&other) {
                continue; // loose at both ends
            }
            // Where the bulb closes: the end node seen again inside the road.
            let close = if from_start {
                walk.nodes.iter().rposition(|&m| m == end).filter(|&k| k > 0)
            } else {
                walk.nodes[..n - 1].iter().position(|&m| m == end)
            };
            if let Some(k) = close {
                if self.roads_at(end).len() == 1 {
                    let stem = if from_start { total - walk.along[k] } else { walk.along[k] };
                    return Some(CulDeSac { road: r, stem, end: self.nodes[end].at, bulb: true });
                }
            } else if self.degree(end) == 1 {
                return Some(CulDeSac { road: r, stem: total, end: self.nodes[end].at, bulb: false });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn road(id: &str, road_type: RoadType, points: Vec<P>) -> RoadSegment {
        RoadSegment { id: id.into(), road_type, points, width: 16.0 }
    }

    fn node_at(g: &Graph, p: P) -> usize {
        g.nodes.iter().position(|n| dist(n.at, p) < 1e-9).expect("a node there")
    }

    #[test]
    fn a_t_is_one_tee_at_right_angles() {
        let roads = vec![
            road("main", RoadType::Arterial, vec![(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]),
            road("stem", RoadType::Suburban, vec![(50.0, 0.0), (50.0, 40.0), (50.0, 80.0)]),
        ];
        let g = Graph::build(&roads);
        let js = g.junctions();
        assert_eq!(js.len(), 1);
        assert_eq!(js[0].node, node_at(&g, (50.0, 0.0)));
        assert_eq!(js[0].kind, JunctionKind::Tee);
        assert!((js[0].min_angle - 90.0).abs() < 1e-9);
        assert_eq!(g.junction_spacing(0), vec![]);
        assert!(g.reached_from_arterials().iter().all(|&r| r));
        let c = g.cul_de_sac(1).expect("a dead end");
        assert!(!c.bulb && (c.stem - 80.0).abs() < 1e-9 && c.end == (50.0, 80.0));
        assert!(g.cul_de_sac(0).is_none(), "loose at both ends");
    }

    #[test]
    fn a_crossing_is_a_four_way() {
        let roads = vec![
            road("a", RoadType::Suburban, vec![(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]),
            road("b", RoadType::Suburban, vec![(50.0, -50.0), (50.0, 0.0), (60.0, 50.0)]),
        ];
        let g = Graph::build(&roads);
        let js = g.junctions();
        assert_eq!(js.len(), 1);
        assert_eq!(js[0].kind, JunctionKind::FourWay);
        let narrowest = (50.0_f64).atan2(10.0).to_degrees();
        assert!((js[0].min_angle - narrowest).abs() < 1e-9, "{}", js[0].min_angle);
        assert!(g.reached_from_arterials().iter().all(|&r| !r), "no arterial");
    }

    #[test]
    fn a_lollipop_closes_on_itself_without_a_tee() {
        // Joins main at (100, 0), runs up 60, then a bulb of radius 10 back to (100, 60).
        let mut pts = vec![(100.0, 0.0), (100.0, 30.0), (100.0, 60.0)];
        pts.extend(crate::models::street_plan::bulb((100.0, 60.0), (0.0, 1.0), 10.0, 12));
        let roads = vec![
            road("main", RoadType::Arterial, vec![(0.0, 0.0), (100.0, 0.0), (200.0, 0.0)]),
            road("sac", RoadType::Suburban, pts),
        ];
        let g = Graph::build(&roads);
        let neck = node_at(&g, (100.0, 60.0));
        assert_eq!(g.degree(neck), 3);
        assert!(g.junction(neck).is_none(), "the bulb's close is not a T");
        assert_eq!(g.junctions().len(), 1, "only the T on main");
        let c = g.cul_de_sac(1).expect("a lollipop");
        assert!(c.bulb && (c.stem - 60.0).abs() < 1e-9 && c.end == (100.0, 60.0), "{c:?}");

        // Spacing along main: the T and a second street 40 further on.
        let mut roads = roads;
        roads.push(road("next", RoadType::Suburban, vec![(140.0, 0.0), (140.0, -50.0)]));
        roads[0].points.insert(2, (140.0, 0.0));
        let g = Graph::build(&roads);
        let gaps: Vec<f64> = g.junction_spacing(0).iter().map(|g| g.0).collect();
        assert_eq!(gaps, vec![40.0]);
    }
}
