//! Offline, every kind of node to look at (`--node-sample`, E2.5): a row
//! of each kind and a wood of trees, laid out round where the camera
//! first looks, and taken through a day's work on a fixed clock so a
//! capture can catch each look.
//!
//! | Seconds | What happens |
//! |---|---|
//! | 0 to 4 | everything at rest, baked into its sector |
//! | 4 to 7 | every other node worked: tips lit, swaying, particles |
//! | 7 | they're taken: tufts cut, trees felled, rocks broken |
//! | 7 to 13 | spent: stubble, stumps, rubble, dug-out banks |
//! | 13 | they come back |
//!
//! Then it starts again, so a long capture loops.

use super::Work;
use capitol_protocol::rest::ResourceNodeInfo;
use std::collections::HashMap;
use uuid::Uuid;

/// When work starts, the node is taken, and it comes back; and the loop.
const WORK_AT: f32 = 4.0;
const TAKEN_AT: f32 = 7.0;
const BACK_AT: f32 = 13.0;
const LOOP: f32 = 18.0;
/// The sample's worker, nobody in particular.
const WORKER: Uuid = Uuid::from_u128(0x5a3d1e);

/// The kinds in rows, by resource id, with how far apart they stand
/// (server units). The last is one the client doesn't know.
const ROWS: [(&str, f64); 9] = [
    ("grass", 30.0),
    ("fallen_branches", 34.0),
    ("rock_outcrop", 40.0),
    ("iron_vein", 40.0),
    ("copper_vein", 40.0),
    ("tin_vein", 40.0),
    ("clay_bank", 44.0),
    ("reed_bed", 30.0),
    ("moonstone", 30.0),
];
const PER_ROW: usize = 5;
/// The wood behind the rows: this many trees.
const TREES: usize = 36;

pub struct Sample {
    nodes: HashMap<Uuid, ResourceNodeInfo>,
    /// The ones taken through the day's work.
    worked: Vec<Uuid>,
    time: f32,
}

impl Sample {
    /// Lays the sample out round the server point `focus`.
    pub fn new(focus: [f64; 2]) -> Self {
        let mut nodes = HashMap::new();
        let mut worked = Vec::new();
        let mut add = |n: u128, kind: &str, x: f64, y: f64, work: bool| {
            let id = Uuid::from_u128(0xe25_0000_0000 + n * 0x9e37_79b9_7f4a_7c15);
            nodes.insert(
                id,
                ResourceNodeInfo {
                    id,
                    resource_type: kind.into(),
                    x,
                    y,
                    state: "available".into(),
                    base_quality: None,
                },
            );
            if work {
                worked.push(id);
            }
        };
        let mut n = 0;
        for (row, (kind, gap)) in ROWS.iter().enumerate() {
            let y = focus[1] - 20.0 + row as f64 * 42.0;
            for k in 0..PER_ROW {
                let x = focus[0] + (k as f64 - (PER_ROW - 1) as f64 / 2.0) * gap;
                add(n, kind, x, y, k % 2 == 1);
                n += 1;
            }
        }
        // A wood beyond the rows, on a jittered grid.
        let mut h: u64 = 0x2545_f491_4f6c_dd1d;
        let mut unit = || {
            h ^= h << 13;
            h ^= h >> 7;
            h ^= h << 17;
            (h >> 11) as f64 / (1u64 << 53) as f64
        };
        let side = 6;
        for k in 0..TREES {
            let (i, j) = ((k % side) as f64, (k / side) as f64);
            let x = focus[0] + (i - 2.5) * 55.0 + (unit() - 0.5) * 30.0;
            let y = focus[1] - 120.0 - j * 50.0 + (unit() - 0.5) * 30.0;
            add(n, "tree", x, y, k % 5 == 2);
            n += 1;
        }
        Sample {
            nodes,
            worked,
            time: 0.0,
        }
    }

    /// Moves the sample's clock on, and the worked nodes with it.
    pub fn update(&mut self, dt: f32) {
        self.time += dt;
        let state = match self.time % LOOP {
            t if t < WORK_AT => "available",
            t if t < TAKEN_AT => "harvesting",
            t if t < BACK_AT => "depleted",
            _ => "available",
        };
        for id in &self.worked {
            if let Some(n) = self.nodes.get_mut(id) {
                if n.state != state {
                    n.state = state.into();
                }
            }
        }
    }

    pub fn nodes(&self) -> &HashMap<Uuid, ResourceNodeInfo> {
        &self.nodes
    }

    /// The work on `id`, while it's being worked.
    pub fn work(&self, id: Uuid) -> Option<Work> {
        let t = self.time % LOOP;
        if !(WORK_AT..TAKEN_AT).contains(&t) || !self.worked.contains(&id) {
            return None;
        }
        let duration = ((TAKEN_AT - WORK_AT) * 10.0) as u32;
        Some(Work {
            player: WORKER,
            progress: ((t - WORK_AT) * 10.0) as u32,
            duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_has_every_kind_and_works_some() {
        let mut s = Sample::new([3400.0, 4000.0]);
        let kinds: std::collections::HashSet<&str> = s
            .nodes()
            .values()
            .map(|n| n.resource_type.as_str())
            .collect();
        for (kind, _) in ROWS {
            assert!(kinds.contains(kind), "{kind}");
        }
        assert!(kinds.contains("tree"));
        let worked = s.worked[0];
        s.update(5.0);
        assert_eq!(s.nodes()[&worked].state, "harvesting");
        assert!(s.work(worked).is_some());
        s.update(3.0);
        assert_eq!(s.nodes()[&worked].state, "depleted");
        assert!(s.work(worked).is_none());
        s.update(6.0);
        assert_eq!(s.nodes()[&worked].state, "available");
    }
}
