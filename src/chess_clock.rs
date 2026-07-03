//! Chess clock and per-search time management. (Port of `chess_clock.h`.)
//!
//! The C++ stores `std::atomic<steady_clock::time_point>`; Rust's `Instant` cannot be atomic, so
//! instants are stored as nanoseconds since a fixed `base` Instant in `AtomicU64`s. Budgets and
//! the remaining clock are integer milliseconds (the UCI granularity). Soft-time management
//! compares the time elapsed since the search started against the allocated budget; the hard
//! deadline is the wall-clock instant at which the search must stop no matter what.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

/// Time control types.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClockType {
    /// N moves in a given time (or sudden death): time is managed per move.
    Standard,
    /// Search to a fixed depth every move.
    FixedDepth,
    /// Search for a fixed amount of time every move.
    FixedTime,
    /// Search until told to stop.
    Infinite,
}

/// Sentinel hard deadline meaning "no hard stop" — `has_reached_hard_deadline` never returns true.
const NO_DEADLINE: u64 = u64::MAX;

/// Clock and time controls for the current game / search.
pub struct ChessClock {
    /// Fixed at construction; all instants are nanoseconds since this base.
    base: Instant,
    pub clock_type: ClockType, // The current time-control type.
    pub remaining_time: i64,   // Time left on our clock this period, ms (UCI wtime/btime).
    pub increment: i64,        // Per-move increment, ms (UCI winc/binc). Parsed but not used by time planning.
    pub moves_to_go: i32,      // Moves until the next time control (0 = unknown / sudden death).
    pub depth: i32,            // Target depth when clock_type == FixedDepth.
    pub max_nodes: u64,        // Node limit for `go nodes` (0 = no limit); checked per-thread in search.

    // search_start_time and hard_deadline are mutated mid-search by `ponderhit` (UCI thread) and
    // read every node by the search worker threads, so they are atomic. The budgets are written
    // by start_search (called under a shared borrow inside search_root_node), so they are atomic
    // too — the C++ keeps them plain because C++ has no shared/exclusive borrow distinction.
    search_start_time: AtomicU64, // nanoseconds since base.
    hard_deadline: AtomicU64,     // nanoseconds since base; NO_DEADLINE = no hard stop.
    allocated_time: AtomicI64,    // Soft time budget for this move, ms (drives between-iteration stops).
    max_search_time: AtomicI64,   // Hard budget remembered for on_ponderhit, ms.
}

impl ChessClock {
    /// Construct with default time control (5-minute sudden death), no search armed.
    /// `reset` restores these same values for a new game.
    pub fn new() -> ChessClock {
        let clock = ChessClock {
            base: Instant::now(),
            clock_type: ClockType::Standard,
            remaining_time: 5 * 60 * 1000,
            increment: 0,
            moves_to_go: 0,
            depth: 10,
            max_nodes: 0,
            search_start_time: AtomicU64::new(0),
            hard_deadline: AtomicU64::new(NO_DEADLINE),
            allocated_time: AtomicI64::new(0),
            max_search_time: AtomicI64::new(0),
        };
        clock.search_start_time.store(clock.now_nanos(), Ordering::Relaxed);
        clock
    }

    /// Nanoseconds elapsed since the base instant.
    #[inline(always)]
    fn now_nanos(&self) -> u64 {
        self.base.elapsed().as_nanos() as u64
    }

    /// Reset to default time control (called when starting a new game).
    pub fn reset(&mut self) {
        self.clock_type = ClockType::Standard;
        self.remaining_time = 5 * 60 * 1000;
        self.increment = 0;
        self.moves_to_go = 0;
        self.depth = 10;
        self.max_nodes = 0;
        self.allocated_time.store(0, Ordering::Relaxed);
        self.max_search_time.store(0, Ordering::Relaxed);
        self.search_start_time.store(self.now_nanos(), Ordering::Relaxed);
        self.hard_deadline.store(NO_DEADLINE, Ordering::Relaxed);
    }

    /// Begin timing a search: record the start instant and the soft/hard budgets, and arm the
    /// hard deadline. A non-positive `maximum` means no hard deadline (fixed-depth / infinite).
    pub fn start_search(&self, allocated: i64, maximum: i64) {
        self.allocated_time.store(allocated, Ordering::Relaxed);
        self.max_search_time.store(maximum, Ordering::Relaxed);
        self.search_start_time.store(self.now_nanos(), Ordering::Relaxed);
        self.arm_hard_deadline(maximum);
    }

    /// Begin a ponder search: timed like start_search but with NO hard deadline — the search runs
    /// until `ponderhit` or `stop`. The budgets are remembered so on_ponderhit can start the real
    /// clock.
    pub fn start_ponder_search(&self, allocated: i64, maximum: i64) {
        self.allocated_time.store(allocated, Ordering::Relaxed);
        self.max_search_time.store(maximum, Ordering::Relaxed);
        self.search_start_time.store(self.now_nanos(), Ordering::Relaxed);
        self.hard_deadline.store(NO_DEADLINE, Ordering::Relaxed);
    }

    /// The opponent played the pondered move: start the real clock now (budget-from-ponderhit),
    /// keeping the work already done. Resets the soft-time origin and arms the hard deadline from
    /// this instant.
    pub fn on_ponderhit(&self) {
        self.search_start_time.store(self.now_nanos(), Ordering::Relaxed);
        self.arm_hard_deadline(self.max_search_time.load(Ordering::Relaxed));
    }

    /// Time elapsed since the most recent search start (or ponderhit), in milliseconds.
    pub fn elapsed_since_search_start(&self) -> i64 {
        ((self.now_nanos() - self.search_start_time.load(Ordering::Relaxed)) / 1_000_000) as i64
    }

    /// Whether the hard deadline has passed. Always false when there is no hard deadline.
    #[inline(always)]
    pub fn has_reached_hard_deadline(&self) -> bool {
        self.now_nanos() >= self.hard_deadline.load(Ordering::Relaxed)
    }

    /// The soft time budget for this move, ms (public field in C++).
    #[inline(always)]
    pub fn allocated_time(&self) -> i64 {
        self.allocated_time.load(Ordering::Relaxed)
    }

    /// Arm the hard deadline at now + `maximum`, or clear it (no hard stop) if `maximum` <= 0.
    fn arm_hard_deadline(&self, maximum: i64) {
        let deadline = if maximum > 0 {
            self.now_nanos() + (maximum as u64) * 1_000_000
        } else {
            NO_DEADLINE
        };
        self.hard_deadline.store(deadline, Ordering::Relaxed);
    }
}

impl Default for ChessClock {
    fn default() -> Self {
        Self::new()
    }
}
