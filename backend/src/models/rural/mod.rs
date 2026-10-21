//! The rural ring, r3000 to r3800 (`the-capitol-rural.md`).
//!
//! Today's layout is still `terrain.rs`'s radial trails. Rural v2 builds the
//! farms here, a work package at a time. So far:
//! - `metrics`: the ring's layout numbers and the plan's §5 targets (#284);
//! - `render`: the plan-view SVG and the `rural_plan_report` test (#284).
//!
//! Both are test-only, as the suburb's are.

#[cfg(test)]
pub(crate) mod metrics;
#[cfg(test)]
mod render;
