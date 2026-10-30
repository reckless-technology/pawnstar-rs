//! Regression lock for the cross-engine parity gate.
//!
//! The blessed `bench` signature below belongs to the shipped net **v20** (release 2.0.0) at the
//! shared NNUE shape — king mirroring on, eight material output heads — which is this build's
//! default: 16 positions at depth 13, single-threaded, Hash pinned to 64 MB by `bench` itself,
//! total **5,617,906** nodes. The C++ reference engine and the Go port must produce the identical
//! total and the identical per-position counts and best moves; a mismatch between the three is a
//! defect in one of them, not a rounding difference.
//!
//! It replaced v12's 6,601,454 when v20 shipped. A net change moves these numbers by design — a
//! *code* change that moves them breaks bit-exact parity with the C++ engine and must be flagged.
//!
//! Skipped (passes) when the shipped net is absent — the golden numbers are net-specific.

mod common;

use std::sync::atomic::Ordering;

use pawnstar::chess_clock::ClockType;
use pawnstar::game::Game;
use pawnstar::input_handling::{BENCH_DEPTH, BENCH_FENS};

/// (best move, node count) per bench position — net v20 at depth 13, single-threaded.
const GOLDEN: [(&str, u64); 16] = [
    ("d6d1", 12124),
    ("d4d5", 158702),
    ("f8g8", 356067),
    ("e5e6", 706104),
    ("c3d5", 411424),
    ("g5g6", 83001),
    ("f3g3", 363104),
    ("f4f5", 108017),
    ("f1b5", 455575),
    ("b6c5", 603920),
    ("f2f4", 732278),
    ("d7f5", 398390),
    ("b2b4", 484449),
    ("d1e1", 239053),
    ("g4g7", 160967),
    ("d2e4", 344731),
];
const GOLDEN_TOTAL: u64 = 5_617_906;

#[test]
fn bench_matches_cpp_golden_signature() {
    let net_path = common::shipped_net_path();
    // The signature belongs to this net at this shape. A build at another NNUE shape refuses the
    // net outright, so there is no bench to compare: that shape's gate is the node-count signature
    // in nnue/README.md, measured over the synthetic net.
    if !common::is_shipped_net_this_builds_shape() {
        common::summary("BENCH GOLDEN: skipped (no net of this build's NNUE shape)");
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
