//! Citizens: AI players who live in the world (`the-capitol-citizens.md`).
//! They play by the protocol any client uses, through [`Bot`](crate::bot::Bot),
//! on accounts an admin made (C0.1), and earn everything they have (D2).
//!
//! - [`town`]: `capitol-town`'s roster, its supervisor and each citizen's
//!   health (C1.1).
//! - [`persona`]: who a citizen is. For now, a name to mint it under.

pub mod persona;
pub mod town;
