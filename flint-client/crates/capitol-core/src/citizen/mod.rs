//! Citizens: AI players who live in the world (`the-capitol-citizens.md`).
//! They play by the protocol any client uses, through [`Bot`](crate::bot::Bot),
//! on accounts an admin made (C0.1), and earn everything they have (D2).
//!
//! - [`town`]: `capitol-town`'s roster, its supervisor and each citizen's
//!   health (C1.1).
//! - [`persona`]: who a citizen is: a name, a temperament, a home and a
//!   market (C1.2).
//! - [`schedule`]: its day on the server's clock, from waking to bed (C1.2).
//! - [`decide`]: how it makes up its mind (C1.3): the situation, the menu,
//!   and a cascade of layers that picks from it.
//! - [`meet`]: how it answers the trades people ask it to (C1.5).
//! - [`idle`]: where it spends the time between its tasks (C2.1).
//! - [`report`]: what the town did, and in a soak whether the server's books
//!   balanced and how hard it worked (C1.7).

pub mod decide;
pub mod idle;
pub mod meet;
pub mod persona;
pub mod report;
pub mod schedule;
pub mod town;
