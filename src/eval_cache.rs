//! Small lockless cache memoising NNUE evaluations by Zobrist hash. (Port of `eval_cache.h`.)

use std::sync::atomic::{AtomicU64, Ordering};

use crate::generated_data::ZobristT;

/// Lockless cache of NNUE evaluation results, keyed by Zobrist hash.
///
/// An NNUE evaluation is a pure function of the position, so memoising it across the search tree
/// (and across transpositions) lets a cache hit skip the per-node forward pass. Each entry is a
/// single 64-bit atomic packing a 48-bit key check (the top 48 bits of the hash) and a 16-bit
/// score, so loads/stores are atomic with no torn values and no locks — shared freely by all
/// Lazy SMP threads. A foreign or evicted entry simply fails the key check and reads as a miss;
/// writers race benignly (last writer wins). Because the stored score depends on the active
/// network, the cache MUST be cleared whenever the loaded net changes.
///
/// Static evaluations are clamped to ±MAX_EVALUATION (30000) in dequant, well within int16 range
/// (and only static evals — never mate scores — are cached), so 16 bits suffice for the score and
/// the other 48 bits go to the key. A zeroed slot has key 0, so a position whose top 48 hash bits
/// are exactly 0 (probability ~2^-48) reads a spurious score of 0; this is a heuristic cache, so
/// that vanishingly rare event only perturbs one leaf evaluation and never affects correctness.
pub struct EvalCache {
    table: Vec<AtomicU64>, // Power-of-two table; each entry packs key|score.
    mask: usize,           // Index mask (table size - 1).
}

impl EvalCache {
    /// Construct a cache of about `megabytes` MB (entry count rounded down to a power of two).
    pub fn new(megabytes: i32) -> EvalCache {
        let want = (megabytes as usize * 1024 * 1024) / core::mem::size_of::<AtomicU64>();
        let mut pow2: usize = 1;
        while pow2 * 2 <= want {
            pow2 *= 2;
        }
        let mut table = Vec::with_capacity(pow2);
        table.resize_with(pow2, || AtomicU64::new(0));
        EvalCache {
            table,
            mask: pow2 - 1,
        }
    }

    /// Probe the cache for a position. Returns the cached evaluation (centipawns, side-to-move
    /// relative) on a hit.
    #[inline(always)]
    pub fn probe(&self, hash: ZobristT) -> Option<i32> {
        let entry = self.table[(hash as usize) & self.mask].load(Ordering::Relaxed);
        if (entry >> 16) != (hash >> 16) {
            return None;
        }
        Some(entry as u16 as i16 as i32)
    }

    /// Store an evaluation for a position.
    #[inline(always)]
    pub fn store(&self, hash: ZobristT, score: i32) {
        let entry = (hash & 0xFFFFFFFFFFFF0000) | (score as i16 as u16 as u64);
        self.table[(hash as usize) & self.mask].store(entry, Ordering::Relaxed);
    }

    /// Reset every entry. Call when the active network changes (and at the start of a game).
    pub fn clear(&self) {
        for entry in &self.table {
            entry.store(0, Ordering::Relaxed);
        }
    }
}
