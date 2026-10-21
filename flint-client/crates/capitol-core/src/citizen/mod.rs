//! Citizens: AI players who live in the world (`the-capitol-citizens.md`).
//! They play by the protocol any client uses, through [`Bot`](crate::bot::Bot),
//! on accounts an admin made (C0.1), and earn everything they have (D2).
//!
//! - [`town`]: `capitol-town`'s roster, its supervisor and each citizen's
//!   health (C1.1).
//! - [`persona`]: who a citizen is. For now, a name to mint it under.
//! - [`decide`]: how it makes up its mind (C1.3): the situation, the menu,
//!   and a cascade of layers that picks from it.
//! - [`meet`]: how it answers the trades people ask it to (C1.5).

pub mod decide;
pub mod meet;
pub mod persona;
pub mod town;
