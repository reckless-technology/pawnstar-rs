//! Lockless transposition table (Hyatt XOR trick) memoising search results.
//! (Port of `transposition_table.h`.)

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use crate::constants::MEGABYTE;
use crate::generated_data::ZobristT;
use crate::moves::{Move, NodeType};

/// A transposition table entry.
/// A transposition is the result of a previous search containing pertinent information about what
/// was found when this position was searched earlier. The score, depth, node type and age are
/// packed into the move's spare bits so that an entry is exactly 128 bits (a 64-bit hash and a
/// 64-bit move).
#[derive(Clone, Copy, Debug)]
pub struct Transposition {
    pub hash: ZobristT, // Zobrist hash of this position.
    pub mv: Move,       // Best move, with score/depth/node-type/age packed into its spare bits.
}

impl Transposition {
    /// Construct from a move that already has its TT metadata packed in (used when reconstructing
    /// an entry read back from the table).
    #[inline(always)]
    pub const fn new(hash: ZobristT, mv: Move) -> Transposition {
        Transposition { hash, mv }
    }

    /// Construct an entry packing score/depth/node-type into the move's spare bits.
    #[inline(always)]
    pub const fn with_data(hash: ZobristT, mv: Move, score: i32, depth: i32, type_: NodeType) -> Transposition {
        Transposition {
            hash,
            mv: mv.with_tt_data(score, depth, type_),
        }
    }

    /// Score computed from this position (lower/upper bound or exact, per node_type()).
    #[inline(always)]
    pub const fn score(&self) -> i32 {
        self.mv.score()
    }

    /// Depth to which this position was searched.
    #[inline(always)]
    pub const fn depth(&self) -> i32 {
        self.mv.tt_depth()
    }

    /// Node type of the stored result.
    #[inline(always)]
    pub const fn node_type(&self) -> NodeType {
        self.mv.tt_node_type()
    }

    /// Table generation when stored; stale when != the table's current generation.
    #[inline(always)]
    pub const fn age(&self) -> u8 {
        self.mv.tt_age()
    }
}

/// One lockless table cell. Stores `key = hash ^ data` alongside `data` (the packed move bits).
/// A reader accepts the entry only if `key ^ data` equals the probe hash, so a torn pair (two
/// 64-bit words written by different stores) is detected and treated as a miss (Hyatt's XOR trick).
struct AtomicEntry {
    key: AtomicU64,  // hash ^ data.
    data: AtomicU64, // Packed move bits (move + score/depth/node-type/age).
}

const _: () = assert!(core::mem::size_of::<AtomicEntry>() == 16);

/// The transposition table.
/// The table is lockless: each cell holds key = hash ^ data and data (the packed move). A reader
/// accepts a cell only when key ^ data == probe hash, so a torn pair written by concurrent stores
/// is detected and read as a miss. Individual 64-bit words are atomic, accessed with relaxed
/// ordering; the XOR check supplies the cross-word consistency that relaxed ordering does not.
pub struct TranspositionTable {
    table: Box<[AtomicEntry]>, // Indexed using Zobrist hash; value-initialised to 0.
    size: usize,               // Number of cells in table (a power of two).
    mask: usize,               // size - 1; index = hash & mask (avoids a runtime modulo).
    generation: AtomicU8,      // Current generation; bumped by age(). (AtomicU8 because age()
                               // runs under a shared borrow inside search_root_node; the C++
                               // keeps it plain and calls it single-threaded pre-search.)
}

impl TranspositionTable {
    /// Create the transposition table of approximately `megabytes` MB.
    pub fn new(megabytes: usize) -> TranspositionTable {
        let mut t = TranspositionTable {
            table: Box::new([]),
            size: 0,
            mask: 0,
            generation: AtomicU8::new(0),
        };
        t.resize(megabytes);
        t
    }

    /// Reallocate the table to approximately `megabytes` (clamped to >= 1 MB), clearing every
    /// entry and resetting the generation. Backs the UCI `Hash` option. Not safe to call during a
    /// search (enforced by `&mut self`).
    pub fn resize(&mut self, megabytes: usize) {
        let megabytes = megabytes.max(1);
        // Round the cell count down to a power of two so the index is a single AND (hash & mask)
        // rather than a modulo by a runtime size.
        let cells = (megabytes * MEGABYTE) / core::mem::size_of::<AtomicEntry>();
        let mut size: usize = 1;
        while (size << 1) <= cells {
            size <<= 1;
        }
        self.size = size;
        self.mask = size - 1;
        let mut v = Vec::with_capacity(size);
        v.resize_with(size, || AtomicEntry {
            key: AtomicU64::new(0),
            data: AtomicU64::new(0),
        });
        self.table = v.into_boxed_slice();
        self.generation.store(0, Ordering::Relaxed);
    }

    /// Zero every entry and reset the generation, keeping the current table size (UCI `Clear
    /// Hash`). Not safe to call during a search.
    pub fn clear(&mut self) {
        for e in self.table.iter() {
            e.key.store(0, Ordering::Relaxed);
            e.data.store(0, Ordering::Relaxed);
        }
        self.generation.store(0, Ordering::Relaxed);
    }

    /// Prefetch the cell a future find_transposition(`hash`) will read, hiding the cache-miss
    /// latency of the random table access. Result-neutral.
    #[inline(always)]
    pub fn prefetch(&self, hash: ZobristT) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::x86_64::_mm_prefetch(
                &self.table[(hash as usize) & self.mask] as *const AtomicEntry as *const i8,
                core::arch::x86_64::_MM_HINT_T2,
            );
        }
        #[cfg(target_arch = "aarch64")]
        {
            // No stable prefetch intrinsic on aarch64; a plain relaxed load of the key word warms
            // the line equivalently for this result-neutral hint.
            let _ = self.table[(hash as usize) & self.mask].key.load(Ordering::Relaxed);
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            let _ = hash;
        }
    }

    /// Find an entry in the TT if one exists for this position.
    #[inline(always)]
    pub fn find_transposition(&self, hash: ZobristT) -> Option<Transposition> {
        let e = &self.table[(hash as usize) & self.mask];
        let data = e.data.load(Ordering::Relaxed);
        let key = e.key.load(Ordering::Relaxed);
        if (key ^ data) == hash {
            return Some(Transposition::new(hash, Move::from_bits(data)));
        }
        None
    }

    /// Insert a new entry into the table if it is more valuable than the existing one.
    /// Lockless read-decide-write: a concurrent writer to the same cell results in benign
    /// last-writer-wins, while the XOR-encoded key keeps each stored pair self-consistent.
    pub fn record_transposition(&self, transposition: &Transposition) {
        let e = &self.table[(transposition.hash as usize) & self.mask];
        let cur_key = e.key.load(Ordering::Relaxed);
        let cur_data = e.data.load(Ordering::Relaxed);
        let cur = Move::from_bits(cur_data);
        let generation = self.generation.load(Ordering::Relaxed);
        let is_empty = cur_key == 0 && cur_data == 0;
        let is_old = cur.tt_age() != generation;
        if is_empty || is_old || cur.tt_depth() <= transposition.depth() || transposition.node_type() == NodeType::Pv {
            let mut m = transposition.mv;
            m.set_tt_age(generation);
            let data = m.bits();
            e.key.store(transposition.hash ^ data, Ordering::Relaxed);
            e.data.store(data, Ordering::Relaxed);
        }
    }

    /// Return the number of occupied entries and percentage full of the table.
    pub fn usage_stats(&self) -> (usize, i32) {
        let mut count: usize = 0;
        for e in self.table.iter() {
            if e.data.load(Ordering::Relaxed) != 0 {
                count += 1;
            }
        }
        (count, ((count * 100) / self.size) as i32)
    }

    /// Approximate fill level for UCI `info hashfull`, in per mille. Samples the first
    /// min(size, 1000) cells (positions hash uniformly, so the prefix is a representative
    /// sample) rather than scanning the whole table, so it is cheap to call on every info line.
    pub fn hashfull_permille(&self) -> i32 {
        let sample = self.size.min(1000);
        if sample == 0 {
            return 0;
        }
        let mut count: usize = 0;
        for e in self.table.iter().take(sample) {
            if e.data.load(Ordering::Relaxed) != 0 {
                count += 1;
            }
        }
        ((count * 1000) / sample) as i32
    }

    /// Advance the table generation so that all existing entries become stale (replaceable).
    /// O(1): entries are not touched; an entry is "old" when its age differs from the current
    /// generation. The u8 generation wraps every 256 searches, which is harmless. Called
    /// single-threaded before search starts.
    pub fn age(&self) {
        // fetch_add wraps on u8, matching the C++ ++generation_.
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}
