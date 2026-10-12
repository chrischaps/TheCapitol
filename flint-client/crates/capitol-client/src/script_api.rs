//! The `cap_*` functions scripts call into Rust (plan §4.2). Registered by
//! `CapitolExtension::register_script_api`.
//!
//! Rhai runs with its `sync` feature, so every closure here and the state
//! it captures must be `Send + Sync`.

use std::sync::{Arc, Mutex};

/// State the Rust side writes and scripts read.
#[derive(Clone, Default)]
pub struct Shared {
    /// The inscription over the Capitol (`--note`).
    pub note: Arc<Mutex<String>>,
}

pub fn register(engine: &mut rhai::Engine, shared: &Shared) {
    // `cap_spike_note() -> String`: the text written over the Capitol.
    let note = shared.note.clone();
    engine.register_fn("cap_spike_note", move || -> String {
        note.lock().map(|s| s.clone()).unwrap_or_default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_read_the_note() {
        let shared = Shared::default();
        *shared.note.lock().unwrap() = "Ad astra".into();
        let mut engine = rhai::Engine::new();
        register(&mut engine, &shared);
        let note: String = engine.eval("cap_spike_note()").unwrap();
        assert_eq!(note, "Ad astra");
    }
}
