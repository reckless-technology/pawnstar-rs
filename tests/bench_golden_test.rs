//! Regression lock for the cross-engine parity gate.
//!
//! The blessed `bench` signature below was produced by the C++ reference engine
//! (pawnstar @ commit 609, net pawnstar-v12.bin, PAWNSTAR_THREADS=1, depth 13) and verified
//! byte-identical against this port — total node count, per-position node counts, best moves and
//! all DEBUGX counters. Any search/eval/ordering change that alters these numbers breaks
//! bit-exact parity with the C++ engine and must be flagged.
//!
//! Skipped (passes) when the shipped net is absent — the golden numbers are net-specific.

mod common;

use std::sync::atomic::Ordering;

use pawnstar::chess_clock::ClockType;
use pawnstar::game::Game;
use pawnstar::input_handling::{BENCH_DEPTH, BENCH_FENS};

/// (best move, node count) per bench position — C++ reference at depth 13, single-threaded.
const GOLDEN: [(&str, u64); 16] = [
    ("d6d1", 13710),
    ("d4d5", 147492),
    ("f8g8", 1230595),
    ("e5e6", 582804),
    ("c3d5", 410154),
    ("g5g6", 180915),
    ("d1c1", 911931),
    ("f4f5", 170315),
    ("f1b5", 686447),
    ("f6d7", 552912),
    ("f2f4", 514815),
    ("d7f5", 261314),
    ("b2b4", 324298),
    ("d1d2", 271979),
    ("g4g7", 119154),
    ("d2e4", 222619),
];
const GOLDEN_TOTAL: u64 = 6_601_454;

#[test]
fn bench_matches_cpp_golden_signature() {
    let net_path = std::env::var("PAWNSTAR_EVALFILE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/nnue/pawnstar-v12.bin").to_string()
    });
    if !std::path::Path::new(&net_path).exists() {
        common::summary("BENCH GOLDEN: skipped (no net supplied)");
        return;
    }
    let mut game = Box::new(Game::new());
    assert!(
        game.nnue_network.load(&net_path).is_loaded(),
        "failed to load net"
    );
    // The `bench` recipe: single-threaded, book off, TT + eval cache cleared up front.
    game.thread_count = 1;
    game.use_own_book = false;
    game.transposition_table.clear();
    game.eval_cache.clear();

    let mut total_nodes: u64 = 0;
    for (i, fen) in BENCH_FENS.iter().enumerate() {
        game.set_position(fen);
        game.time_control.clock_type = ClockType::FixedDepth;
        game.time_control.depth = BENCH_DEPTH;
        let mv = game.search_root_node();
        let nodes = game.last_search_node_count.load(Ordering::Relaxed);
        total_nodes += nodes;
        let (want_move, want_nodes) = GOLDEN[i];
        common::check(
            mv.to_string() == want_move && nodes == want_nodes,
            &format!(
                "bench pos {:02} bm={} nodes={} (golden bm={} nodes={})",
                i + 1,
                mv.to_string(),
                nodes,
                want_move,
                want_nodes
            ),
        );
    }
    common::check(
        total_nodes == GOLDEN_TOTAL,
        &format!("bench total {} == C++ golden {}", total_nodes, GOLDEN_TOTAL),
    );
    let failures = common::summary(&format!(
        "BENCH GOLDEN: {} nodes at depth {} — bit-exact with the C++ reference",
        total_nodes, BENCH_DEPTH
    ));
    assert_eq!(failures, 0);
}
