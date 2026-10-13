//! The world built from `/terrain` (plan P3): each piece is a runtime mesh
//! (H4) rebuilt when the terrain changes. So far that's the ground (P3.2),
//! the water (P3.3), the roads (P3.4) and the bridges (P3.5); plots and
//! the tower follow in P3.6–P3.7. A layer with thousands of pieces is split into
//! `sector`s, one mesh each, so each part culls on its own.

pub mod bridges;
pub mod ground;
pub mod mesh;
pub mod palette;
pub mod roads;
pub mod sector;
pub mod water;
