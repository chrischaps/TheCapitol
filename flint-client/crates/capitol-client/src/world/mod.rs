//! The world built from `/terrain` (plan P3): each piece is a runtime mesh
//! (H4) rebuilt when the terrain changes. So far that's the ground (P3.2),
//! the water (P3.3), the roads (P3.4), the bridges (P3.5) and the plots
//! (P3.6); the tower follows in P3.7. A layer with thousands of pieces is split into
//! `sector`s, one mesh each, so each part culls on its own.

pub mod bridges;
pub mod geom;
pub mod ground;
pub mod mesh;
pub mod palette;
pub mod plots;
pub mod roads;
pub mod sector;
pub mod water;
