//! The Capitol client. A stub until P0.4 wraps `flint_player::PlayerApp` and
//! installs the Capitol extension.

// Linked now so the workspace proves it can build the engine from the subtree.
use flint_player as _;

fn main() {
    println!("The Capitol (Flint client) {}: nothing to show yet.", env!("CARGO_PKG_VERSION"));
}
