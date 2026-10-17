//! The suburb, rebuilt as post-war neighbourhoods (`the-capitol-suburb-v2.md`).
//!
//! - `neighbourhood`: the sectors, their seeded splits, buffers and entrances (#106);
//! - `frame`: each neighbourhood's `F(u, v) → world` and its row fit (#106);
//! - `patterns`: the streets each neighbourhood is filled with, and their
//!   `StreetMeta` (pattern (a), #107);
//! - `graph`: the street network as a graph;
//! - `metrics`: the layout numbers and the targets they're held to;
//! - `render`: the plan-view SVG and the `suburb_plan_report` test.
//!
//! Until something outside the tests uses `graph` and `metrics`, they're
//! test-only.

use capitol_protocol::terrain::RoadSegment;

use super::terrain::SuburbanGenParams;

// Patterns (#107 on) and lots v2 (#108) use what the server build doesn't yet.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod frame;
#[cfg(test)]
pub(crate) mod graph;
#[cfg(test)]
pub(crate) mod metrics;
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod neighbourhood;
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod patterns;
#[cfg(test)]
mod render;

use neighbourhood::Neighbourhood;
use patterns::StreetMeta;

/// The Neighbourhood layout of the suburb.
pub(crate) struct SuburbPlan {
    pub roads: Vec<RoadSegment>,
    /// One for each of `roads`, in the same order.
    #[cfg_attr(not(test), allow(dead_code))] // lots v2 reads it (#108)
    pub meta: Vec<StreetMeta>,
    #[cfg_attr(not(test), allow(dead_code))] // the patterns and lots v2 read them
    pub neighbourhoods: Vec<Neighbourhood>,
}

/// Lay out the suburb round `center`: the neighbourhoods, and each one's
/// streets in the pattern its own stream picks.
pub(crate) fn plan(params: &SuburbanGenParams, center: (f64, f64)) -> SuburbPlan {
    let mut neighbourhoods = neighbourhood::neighbourhoods(params, center);
    let (mut roads, mut meta) = (Vec::new(), Vec::new());
    for n in &mut neighbourhoods {
        let laid = patterns::lay(n, &params.neighbourhood);
        roads.extend(laid.roads);
        meta.extend(laid.meta);
    }
    SuburbPlan { roads, meta, neighbourhoods }
}
