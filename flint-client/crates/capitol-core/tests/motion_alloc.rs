//! `Motion::sample` runs every frame, so it mustn't allocate once warmed up.
//! A counting allocator checks it. It's its own test binary because the
//! allocator is global.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

use capitol_core::motion::Motion;
use capitol_core::net::{ConnectionState, NetEvent};
use capitol_core::protocol::ws::{PlayerPosition, ServerMessage};
use capitol_core::state::GameState;
use uuid::Uuid;

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations() -> usize {
    ALLOCATIONS.with(|n| n.get())
}

#[test]
fn sampling_doesnt_allocate() {
    let me = Uuid::from_u128(1);
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let mut state = GameState::new();
    let mut motion = Motion::new();
    let feed = |state: &mut GameState, motion: &mut Motion, ms: u64, event: NetEvent| {
        let changes = state.apply(&event);
        motion.observe(at(ms), &changes, state);
    };
    feed(
        &mut state,
        &mut motion,
        0,
        NetEvent::Connection(ConnectionState::Connected { player_id: me }),
    );

    // Fifty players walking east; the local player walking south.
    let snapshot = |k: u64| {
        let players = (1..=50u128)
            .map(|n| PlayerPosition {
                id: Uuid::from_u128(n),
                name: format!("player {n}"),
                x: 4000.0 + 5.0 * k as f64,
                y: 4000.0 + 20.0 * n as f64,
                citizen: false,
                tag: None,
            })
            .collect();
        NetEvent::Ws(ServerMessage::Positions { players })
    };
    feed(&mut state, &mut motion, 0, snapshot(0));
    motion.move_to(at(0), 4000.0, 6000.0);

    let mut checked = 0;
    for ms in (0..3000).step_by(5) {
        if ms % 100 == 0 {
            feed(&mut state, &mut motion, ms, snapshot(ms / 100));
        }
        let before = allocations();
        let poses = motion.sample(at(ms));
        let count = poses.len();
        let after = allocations();
        // The first sample grows the pose buffer, which shows the counter
        // works. None after it may allocate.
        if ms == 0 {
            assert!(after > before, "the counter saw the first sample grow");
        } else {
            assert_eq!(after, before, "sample at {ms} ms allocated");
            checked += 1;
        }
        assert_eq!(count, 50);
    }
    assert!(checked > 500);
}
