//! Functions for managing history tables. (Port of `history_table.h`.)

use crate::constants::Color;
use crate::moves::Move;

/// Total number of (side, from/to) entries.
pub const TABLE_SIZE: usize = 2 * 4096;

/// Bounds a gravity-managed history entry to +/- HISTORY_MAX. 16384 fits an i16 (continuation
/// history) with margin.
pub const HISTORY_MAX: i32 = 16384;

/// Caps the per-update bonus so a single very deep cutoff cannot dominate; depth^2 only reaches it
/// near depth 20.
pub const MAX_HISTORY_BONUS: i32 = 400;

/// Depth-weighted magnitude credited to a good move (and debited from the quiets that failed before
/// it). Quadratic in depth, clamped to MAX_HISTORY_BONUS.
#[inline(always)]
pub fn history_bonus(depth: i32) -> i32 {
    let depth = depth.max(1);
    (depth * depth).min(MAX_HISTORY_BONUS)
}

/// Nudge `*entry` by `bonus` with a gravity term that keeps it within +/- HISTORY_MAX, so recent
/// evidence overwrites stale evidence. Integer division truncates toward zero (matching Go/C++).
#[inline(always)]
pub fn apply_gravity(entry: &mut i32, bonus: i32) {
    *entry += bonus - *entry * bonus.abs() / HISTORY_MAX;
}

/// Butterfly history table tracking which moves raised alpha or caused a beta cutoff.
/// Moves are indexed by side to move and from/to square, pooling statistics across all plies.
/// The table is per-thread (owned by SearchState) under Lazy SMP — each search thread has its
/// own — so the counts are plain (non-atomic): there is no cross-thread access to synchronise.
///
/// Storage is signed (i32): the main search updates entries with a depth-weighted bonus/malus and a
/// gravity term (apply_bonus), and the malus drives entries negative. Quiescence stays on the simple
/// increment-only record_good_move; the two coexist on the same table (matching the Go engine).
pub struct HistoryTable {
    counts: Box<[i32]>, // Heap-allocated count array (signed: malus can go negative).
}

impl HistoryTable {
    /// Construct the table, allocating and zeroing the count array.
    pub fn new() -> HistoryTable {
        HistoryTable {
            counts: vec![0i32; TABLE_SIZE].into_boxed_slice(),
        }
    }

    /// Reset all counts to zero (call before each root search).
    pub fn reset(&mut self) {
        self.counts.fill(0);
    }

    /// Return the maximum count across all entries.
    pub fn max_count(&self) -> i32 {
        let mut max_val = i32::MIN;
        for &c in self.counts.iter() {
            if c > max_val {
                max_val = c;
            }
        }
        max_val
    }

    /// Increment the count for a move that raised alpha or caused a cutoff. Used only by quiescence,
    /// which carries no meaningful depth and stays on the simple increment; the main search uses
    /// apply_bonus.
    #[inline(always)]
    pub fn record_good_move(&mut self, side: Color, mv: Move) {
        self.counts[side.index() * 4096 + mv.from_to() as usize] += 1;
    }

    /// Credit (bonus > 0) or debit (bonus < 0) the butterfly-history entry for `mv` by `side` with a
    /// gravity-managed, depth-weighted bonus.
    #[inline(always)]
    pub fn apply_bonus(&mut self, side: Color, mv: Move, bonus: i32) {
        apply_gravity(
            &mut self.counts[side.index() * 4096 + mv.from_to() as usize],
            bonus,
        );
    }

    /// Retrieve the (signed) history count for a move by the given side.
    #[inline(always)]
    pub fn get_count(&self, side: Color, mv: Move) -> i32 {
        self.counts[side.index() * 4096 + mv.from_to() as usize]
    }
}

impl Default for HistoryTable {
    fn default() -> Self {
        Self::new()
    }
}
