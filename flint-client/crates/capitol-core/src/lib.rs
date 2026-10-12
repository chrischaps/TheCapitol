//! Networking and state for The Capitol's Flint client: REST and WebSocket
//! clients, reconnect, the world mirror, interpolation, polling and the trade
//! state machine. It has no engine dependency, so its tests stay fast.
//!
//! - [`net`]: the IO thread. The game sends [`net::Command`]s and drains
//!   [`net::NetEvent`]s once per frame; it never blocks on the network.

pub mod net;

/// The wire types, shared with the backend.
pub use capitol_protocol as protocol;
