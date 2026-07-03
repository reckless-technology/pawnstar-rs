//! Functions for managing history tables. (Port of `history_table.h`.)

use crate::constants::Color;
use crate::moves::Move;

/// Total number of (side, from/to) entries.
pub const TABLE_SIZE: usize = 2 * 4096;

/// Butterfly history table tracking which moves raised alpha or caused a beta cutoff.
/// Moves are indexed by side to move and from/to square, pooling statistics across all plies.
/// The table is per-thread (owned by SearchState) under Lazy SMP — each search thread has its
/// own — so the counts are plain (non-atomic): there is no cross-thread access to synchronise.
pub struct HistoryTable {
    counts: Box<[u32]>, // Heap-allocated count array.
}

impl HistoryTable {
    /// Construct the table, allocating and zeroing the count array.
    pub fn new() -> HistoryTable {
        HistoryTable {
            counts: vec![0u32; TABLE_SIZE].into_boxed_slice(),
        }
    }

    /// Reset all counts to zero (call before each root search).
    pub fn reset(&mut self) {
        self.counts.fill(0);
    }

    /// Return the maximum count across all entries.
    pub fn max_count(&self) -> u32 {
        let mut max_val = 0u32;
        for &c in self.counts.iter() {
            if c > max_val {
                max_val = c;
            }
        }
        max_val
    }

    /// Increment the count for a move that raised alpha or caused a cutoff.
    #[inline(always)]
    pub fn record_good_move(&mut self, side: Color, mv: Move) {
        self.counts[side.index() * 4096 + mv.from_to() as usize] += 1;
    }

    /// Retrieve the history count for a move by the given side.
    #[inline(always)]
    pub fn get_count(&self, side: Color, mv: Move) -> u32 {
        self.counts[side.index() * 4096 + mv.from_to() as usize]
    }
}

impl Default for HistoryTable {
    fn default() -> Self {
        Self::new()
    }
}
