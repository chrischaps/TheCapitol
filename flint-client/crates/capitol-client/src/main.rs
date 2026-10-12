//! The Capitol client. A stub until P1's `PlayerExtension` lands; the P0.4
//! embedding spike that proved the integration lives on branch `spike/p0.4`.

#[allow(dead_code)] // used once the world is built (P3)
mod coords;

// Linked now so the workspace proves it can build the engine from the subtree.
use flint_player as _;

fn main() {
    println!("The Capitol (Flint client) {}: nothing to show yet.", env!("CARGO_PKG_VERSION"));
}
