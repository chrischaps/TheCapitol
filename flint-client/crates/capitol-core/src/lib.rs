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
//! - [`poll`]: the polling scheduler. [`poll::Poller`] turns those events, and
//!   a 5 s timer, into the REST refetches the server never pushes.
//! - [`catalog`]: item, station and resource types the server has no endpoint for.

pub mod catalog;
pub mod motion;
pub mod net;
pub mod poll;
pub mod state;

/// The wire types, shared with the backend.
pub use capitol_protocol as protocol;
