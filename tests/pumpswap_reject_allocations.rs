use sol_shred_sdk::{accounts::pumpswap::effective_quote_reserves, logs::pump_amm, EventMetadata};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
};

struct CountingAllocator;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn record_allocation() {
    let _ = TRACKING.try_with(|tracking| {
        if tracking.get() {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}

// SAFETY: Every operation delegates unchanged to System; the thread-local
// counters hold only Cells and do not allocate or change allocation ownership.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        System.alloc_zeroed(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct TrackingGuard;

impl Drop for TrackingGuard {
    fn drop(&mut self) {
        TRACKING.with(|tracking| tracking.set(false));
    }
}

fn allocation_count<T>(f: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    let guard = TrackingGuard;
    let value = f();
    drop(guard);
    let count = ALLOCATIONS.with(Cell::get);
    (value, count)
}

fn buy_payload() -> Vec<u8> {
    // Fixed fields + Borsh ix_name + complete boost-era tail.
    let name = b"buy_exact_quote_in";
    let mut data = vec![0; 393];
    data.extend_from_slice(&(name.len() as u32).to_le_bytes());
    data.extend_from_slice(name);
    data.extend_from_slice(&[0; 32]); // cashback and buyback
    data.extend_from_slice(&(-500i128).to_le_bytes());
    data.push(0); // can_boost
    data.extend_from_slice(&0u64.to_le_bytes()); // base_supply
    data
}

#[test]
fn malformed_buy_is_rejected_before_allocating_instruction_name() {
    let valid = buy_payload();
    let tail_start = valid.len() - 57;
    for tail_len in [1, 15, 17, 31, 33, 47, 48, 49, 56, 58, 72] {
        let mut data = valid.clone();
        data.resize(tail_start + tail_len, 0);
        let (event, count) =
            allocation_count(|| pump_amm::parse_buy_from_data(&data, EventMetadata::default()));
        assert!(event.is_none(), "tail_len={tail_len}");
        assert_eq!(count, 0, "tail_len={tail_len}");
    }
    for invalid_bool in [2, 127, 255] {
        let mut data = valid.clone();
        data[tail_start + 48] = invalid_bool;
        let (event, count) =
            allocation_count(|| pump_amm::parse_buy_from_data(&data, EventMetadata::default()));
        assert!(event.is_none());
        assert_eq!(count, 0);

        let mut data = valid.clone();
        data[352] = invalid_bool; // required Borsh track_volume bool
        let (event, count) =
            allocation_count(|| pump_amm::parse_buy_from_data(&data, EventMetadata::default()));
        assert!(event.is_none(), "track_volume={invalid_bool}");
        assert_eq!(count, 0);
    }

    // Successful nonempty Buy still owns its name and needs exactly one allocation.
    let (event, count) =
        allocation_count(|| pump_amm::parse_buy_from_data(&valid, EventMetadata::default()));
    assert!(event.is_some());
    assert_eq!(count, 1);

    let ((), count) = allocation_count(|| {
        for (raw, signed) in [(1_000, -500), (0, i128::MIN), (u64::MAX, i128::MAX)] {
            black_box(effective_quote_reserves(black_box(raw), black_box(signed)));
        }
    });
    assert_eq!(count, 0);
}
