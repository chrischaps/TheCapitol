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
//! - [`bazaar`]: the bazaar (E3.10): its directory, the shops seen at their
//!   stalls and yours, and the walk to a stall.
//! - [`hall`]: going to the Exchange hall (E3.7): open the market there, or
//!   walk to its door and open it on arrival; and goods waiting in the box.
//! - [`onboarding`]: the road in as the client shows it (#441): a view
//!   of the server's tour, and what only the screen can report.
//! - [`poll`]: the polling scheduler. [`poll::Poller`] turns those events, and
//!   a 5 s timer, into the REST refetches the server never pushes.
//! - [`trading`]: trading as a player does it: shift-click someone to
//!   ask, or walk closer and ask, and your offer as you build it.
//! - [`station`]: stations in the world: whether one is in reach.
//! - [`steering`]: walking on a heading (controls T0.4): the movement input
//!   turned into `steer`s, which [`motion::Motion::steer`] predicts.
//! - [`automation`]: a station working on its own (E4.4): where it
//!   stands, a pause in plain words, and the form that sets it up.
//! - [`wages`]: piece-rate wages (Commons S2.7): the Work page's offers,
//!   the shift you're on, the hire form and its calls.
//! - [`plot`]: what the plot panel offers for a plot: yours, someone
//!   else's, or for sale at its zone's deed price.
//! - [`signin`]: log in, or register, one event at a time.
//! - [`auth`]: the way in for a person: the login form's states, a stored
//!   token tried first, and the server's health while signed out.
//! - [`talk`]: the lines heard on each channel, what's unread, and the
//!   bubble over each speaker (Commons S1.2).
//! - [`terrain`]: `/terrain` with a disk cache, revalidated by version.
//! - [`paths`]: walking round the water: a walk graph of the roads and
//!   trails, and A* over it (C0.5, #211).
//! - [`catalog`]: item, station and resource types the server has no endpoint for.
//! - [`bot`]: a headless player built on all of the above, for `capitol-bot`
//!   and the live tests.
//! - [`sim`]: the economy sim (E2.7), a swarm of bots that gathers, crafts and
//!   builds, then reports what the economy produced.
//! - [`citizen`]: AI players who live in the world, run by `capitol-town`
//!   (C1.1).

pub mod admin;
pub mod auth;
pub mod automation;
pub mod bazaar;
pub mod bot;
pub mod card;
pub mod catalog;
pub mod chat;
pub mod commissions;
pub mod citizen;
pub mod craft;
pub mod deeds;
pub mod exchange;
pub mod gazette;
pub mod hall;
pub mod harvest;
pub mod mail;
pub mod market;
pub mod market_maker;
pub mod motion;
pub mod guilds;
pub mod mutes;
pub mod net;
pub mod onboarding;
pub mod paths;
pub mod plot;
pub mod poll;
pub mod signin;
pub mod sim;
pub mod standings;
pub mod state;
pub mod station;
pub mod steering;
pub mod talk;
pub mod terrain;
pub mod trading;
pub mod wages;
pub mod zone_card;

/// The wire types, shared with the backend.
pub use capitol_protocol as protocol;
/// The clock and the dates in subscriptions (E4.1), for the client to format.
pub use chrono;
/// JSON, for the client's offline samples (the Gazette's, E3.9).
pub use serde_json;
