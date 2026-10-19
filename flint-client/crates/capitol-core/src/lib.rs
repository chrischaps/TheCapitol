//! Networking and state for The Capitol's Flint client: REST and WebSocket
//! clients, reconnect, the world mirror, interpolation, polling and the trade
//! state machine. It has no engine dependency, so its tests stay fast.
//!
//! - [`net`]: the IO thread. The game sends [`net::Command`]s and drains
//!   [`net::NetEvent`]s once per frame; it never blocks on the network.
//! - [`state`]: the world mirror. [`state::GameState::apply`] takes each
//!   `NetEvent` and returns the [`state::StateEvent`]s the game reacts to.
//! - [`motion`]: smooth positions. [`motion::Motion`] draws other players
//!   150 ms behind between snapshots and predicts the local player's walk.
//! - [`harvest`]: clicking a node: extract it, or walk there and extract
//!   on arrival.
//! - [`craft`]: crafting by hand: the recipe the inputs make, what's
//!   missing, the quality range, output with nowhere to go, and one craft from
//!   the ask to the settled inventory.
//! - [`exchange`]: the fiber exchange: a deposit's or withdrawal's
//!   numbers before you confirm, the server's own rule.
//! - [`market`]: the market (E3.3): the books you watch, kept by their
//!   diffs, your orders and the latest trades.
//! - [`hall`]: going to the Exchange hall (E3.7): open the market there, or
//!   walk to its door and open it on arrival; and goods waiting in the box.
//! - [`onboarding`]: the first-session hints: which to show, and what
//!   counts as done (P8.3).
//! - [`poll`]: the polling scheduler. [`poll::Poller`] turns those events, and
//!   a 5 s timer, into the REST refetches the server never pushes.
//! - [`trading`]: trading as a player does it: shift-click someone to
//!   ask, or walk closer and ask, and your offer as you build it.
//! - [`station`]: stations in the world: whether one is in reach.
//! - [`plot`]: what the plot panel offers for a plot: yours, someone
//!   else's, or for sale at its zone's deed price.
//! - [`signin`]: log in, or register, one event at a time.
//! - [`auth`]: the way in for a person: the login form's states, a stored
//!   token tried first, and the server's health while signed out.
//! - [`terrain`]: `/terrain` with a disk cache, revalidated by version.
//! - [`catalog`]: item, station and resource types the server has no endpoint for.
//! - [`bot`]: a headless player built on all of the above, for `capitol-bot`
//!   and the live tests.
//! - [`sim`]: the economy sim (E2.7), a swarm of bots that gathers, crafts and
//!   builds, then reports what the economy produced.

pub mod auth;
pub mod bot;
pub mod catalog;
pub mod craft;
pub mod exchange;
pub mod hall;
pub mod harvest;
pub mod market;
pub mod market_maker;
pub mod motion;
pub mod net;
pub mod onboarding;
pub mod plot;
pub mod poll;
pub mod signin;
pub mod sim;
pub mod state;
pub mod station;
pub mod terrain;
pub mod trading;

/// The wire types, shared with the backend.
pub use capitol_protocol as protocol;
/// The clock and the dates in subscriptions (E4.1), for the client to format.
pub use chrono;
