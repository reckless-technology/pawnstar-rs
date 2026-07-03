//! Pawnstar — a UCI chess engine using bitboard board representation and parallel alpha-beta
//! search. Mechanical Rust port of the C++ original; each module ports the same-named C++ header
//! (`move.h` → `moves.rs`).

// The port intentionally carries the full C++ API surface, parts of which are only used by the
// tests/tools; keep the mechanical 1:1 mapping rather than pruning to what the engine binary uses.
#![allow(dead_code)]

pub mod attacks;
pub mod bitboard;
pub mod castling_rights;
pub mod constants;
pub mod debug_hashtable;
pub mod eval_cache;
pub mod generated_data;
pub mod history_table;
pub mod moves;
pub mod nnue;
pub mod pins;
pub mod platform;
pub mod position;
pub mod square;
pub mod stack_list;
pub mod static_exchange_evaluation;
pub mod version;
