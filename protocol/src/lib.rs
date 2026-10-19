//! The Capitol's wire protocol, shared by the backend and the clients.
//!
//! The backend serialises these types and the Flint client deserialises them, so
//! drift between the two becomes a build error instead of a silent bug.
//!
//! - [`ws`]: WebSocket messages (`ClientMessage`, `ServerMessage`)
//! - [`rest`]: request and response bodies for the REST API
//! - [`terrain`]: the `/terrain` response
//! - [`rules`]: speeds, ranges, zones, quality grades and the water-crossing test
//! - [`fmath`]: trig that gives the same bits on every platform
//! - [`land`]: the height of the ground, a pure function of `LandParams`
//!
//! The `backend` feature adds the server-only derives (utoipa schemas, sqlx rows).
//! Clients leave it off.

pub mod fmath;
pub mod land;
pub mod rest;
pub mod rules;
pub mod terrain;
pub mod ws;
