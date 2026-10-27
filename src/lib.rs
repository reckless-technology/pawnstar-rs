//! Pawnstar — a UCI chess engine using bitboard board representation and parallel alpha-beta
//! search. Mechanical Rust port of the C++ original; each module ports the same-named C++ header
//! (`move.h` → `moves.rs`).

// The port intentionally carries the full C++ API surface, parts of which are only used by the
// tests/tools; keep the mechanical 1:1 mapping rather than pruning to what the engine binary uses.
#![allow(dead_code)]

pub mod attacks;
pub mod bitboard;
pub mod castling_rights;
pub mod chess_clock;
pub mod constants;
pub mod crc64;
pub mod debug_hashtable;
pub mod eval_cache;
pub mod evaluation;
pub mod game;
pub mod generated_data;
pub mod history_table;
pub mod input_handling;
pub mod moves;
pub mod nnue;
pub mod opening_book;
pub mod pins;
pub mod platform;
pub mod position;
pub mod search_state;
pub mod square;
pub mod stack_list;
pub mod static_exchange_evaluation;
pub mod transposition_table;
pub mod version;

/// Print a UCI-protocol line and flush stdout. The C++ sets `std::unitbuf` (every insertion
/// flushes); Rust stdout is block-buffered when piped, so every protocol response flushes
/// explicitly to keep GUIs and the shell test driver responsive.
#[macro_export]
macro_rules! uci_println {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, $($arg)*);
        let _ = out.flush();
    }};
}
