//! The Capitol's wire protocol, shared by the backend and the clients.
//!
//! The backend serialises these types and the Flint client deserialises them, so
//! drift between the two becomes a build error instead of a silent bug.
//!
//! - [`ws`]: WebSocket messages (`ClientMessage`, `ServerMessage`)
//! - [`rest`]: request and response bodies for the REST API
//! - [`terrain`]: the `/terrain` response
//! - [`rules`]: speeds, ranges, zones, quality grades, the water-crossing test
//!   and moving on the land (`rules::ground`)
//! - [`fmath`]: trig that gives the same bits on every platform
//! - [`land`]: the height of the ground, a pure function of `LandParams`
//! - [`ledger`]: the world's books, `GET /admin/ledger`, and what two of them
//!   say about conservation and load (C1.7)
//! - [`annotation`]: what explains a bend in the console's charts (the
//!   Observatory's D0.4)
//!
//! The `backend` feature adds the server-only derives (utoipa schemas, sqlx rows).
//! Clients leave it off.

pub mod annotation;
pub mod fmath;
pub mod land;
pub mod ledger;
pub mod rest;
pub mod rules;
pub mod terrain;
pub mod ws;
