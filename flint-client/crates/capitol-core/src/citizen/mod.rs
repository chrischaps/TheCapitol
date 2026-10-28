//! Citizens: AI players who live in the world (`the-capitol-citizens.md`).
//! They play by the protocol any client uses, through [`Bot`](crate::bot::Bot),
//! on accounts an admin made (C0.1), and earn everything they have (D2).
//!
//! - [`town`]: `capitol-town`'s roster, its supervisor and each citizen's
//!   health (C1.1).
//! - [`persona`]: who a citizen is: a name, a temperament, a home and a
//!   market (C1.2).
//! - [`schedule`]: its day on the server's clock, from waking to the night's
//!   work (C1.2).
//! - [`habit`]: when it plays, on the real clock (#276).
//! - [`calling`]: what it would rather be doing, and how much (Callings
//!   K0.2): a favourite, its focus, its leanings and its wander.
//! - [`decide`]: how it makes up its mind (C1.3): the situation, the menu,
//!   and a cascade of layers that picks from it.
//! - [`meet`]: how it answers the trades people ask it to (C1.5).
//! - [`phrases`]: what it says, and how often (Commons S1.6).
//! - [`idle`]: where it spends the time between its tasks (C2.1).
//! - [`detour`]: the unrelated things it does in work hours (Callings
//!   K1.5): an untried recipe, a lake, the lane, the paper, a neighbour.
//! - [`report`]: what the town did, and in a soak whether the server's books
//!   balanced and how hard it worked (C1.7).
//! - [`balance`]: a soak under a preset of tunables, and two of them held up
//!   against each other (D6.3).
//! - [`measures`]: §10's callings measures, for a soak's report and a
//!   balance run's comparison (#553).

pub mod balance;
pub mod calling;
pub mod decide;
pub mod detour;
pub mod habit;
pub mod idle;
pub mod measures;
pub mod meet;
pub mod persona;
pub mod phrases;
pub mod report;
pub mod schedule;
pub mod town;
pub mod voice;
