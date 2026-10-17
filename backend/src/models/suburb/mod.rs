//! The suburb, rebuilt as post-war neighbourhoods (`the-capitol-suburb-v2.md`).
//!
//! - `neighbourhood`: the sectors, their seeded splits, buffers and entrances (#106);
//! - `frame`: each neighbourhood's `F(u, v) → world` and its row fit (#106);
//! - `graph`: the street network as a graph;
//! - `metrics`: the layout numbers and the targets they're held to;
//! - `render`: the plan-view SVG and the `suburb_plan_report` test.
//!
//! Streets come with the patterns, from #107. Until something outside the
//! tests uses `graph` and `metrics`, they're test-only.

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
#[cfg(test)]
mod render;

use neighbourhood::Neighbourhood;

/// The Neighbourhood layout of the suburb.
pub(crate) struct SuburbPlan {
    pub roads: Vec<RoadSegment>,
    #[cfg_attr(not(test), allow(dead_code))] // the patterns and lots v2 read them
    pub neighbourhoods: Vec<Neighbourhood>,
}

/// Lay out the suburb round `center`. So far, the neighbourhoods and no roads.
pub(crate) fn plan(params: &SuburbanGenParams, center: (f64, f64)) -> SuburbPlan {
    SuburbPlan { roads: Vec::new(), neighbourhoods: neighbourhood::neighbourhoods(params, center) }
}
