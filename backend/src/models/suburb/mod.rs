//! The suburb, rebuilt as post-war neighbourhoods (`the-capitol-suburb-v2.md`).
//!
//! So far this is the harness that measures a plan and draws it (#104):
//! - `graph`: the street network as a graph;
//! - `metrics`: the layout numbers and the targets they're held to;
//! - `render`: the plan-view SVG and the `suburb_plan_report` test.
//!
//! The generator itself arrives with #106 onwards. Until something outside
//! the tests uses `graph` and `metrics`, they're test-only.

#[cfg(test)]
pub(crate) mod graph;
#[cfg(test)]
pub(crate) mod metrics;
#[cfg(test)]
mod render;
