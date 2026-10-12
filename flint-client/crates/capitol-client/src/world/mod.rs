//! The world built from `/terrain` (plan P3): each piece is a runtime mesh
//! (H4) rebuilt when the terrain changes. So far that's the ground (P3.2);
//! water, roads, bridges, plots and the tower follow in P3.3–P3.7.

pub mod ground;
pub mod mesh;
pub mod palette;
