//! The world built from `/terrain` (plan P3): each piece is a runtime mesh
//! (H4) rebuilt when the terrain changes. So far that's the ground (P3.2)
//! and the water (P3.3); roads, bridges, plots and the tower follow in
//! P3.4–P3.7.

pub mod ground;
pub mod mesh;
pub mod palette;
pub mod water;
