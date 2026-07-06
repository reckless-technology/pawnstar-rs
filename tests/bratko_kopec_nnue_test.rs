//! Bratko-Kopec search-quality suite using the NNUE evaluation.
//! (Port of `test/bratko_kopec_nnue_test.cpp`.)
//!
//! Searches the 24 positions in the CASES array with the NNUE evaluator and checks the *move*
//! (the classic Bratko-Kopec metric — did the engine find a good move). A position is "solved"
//! when the move found is among that position's accepted moves. The search is single-threaded by
//! default (PAWNSTAR_THREADS honoured) so it is deterministic, and the suite is a hard gate: it
//! must solve all 24.
//!
//! The C++ takes [net.bin] [depth] on the command line; here the net defaults to the in-repo
//! asset (PAWNSTAR_EVALFILE overrides) and depth to 8 (PAWNSTAR_BK_DEPTH overrides). Skipped
//! (passes) when the net is absent.

mod common;

use pawnstar::chess_clock::ClockType;
use pawnstar::debug_hashtable::{debug_x_clear, debug_x_write};
use pawnstar::game::Game;
use pawnstar::moves::Move;

/// Depth used when none is given. The accepted-move sets were recorded over depths 8–11.
const DEFAULT_DEPTH: i32 = 8;

/// A Bratko-Kopec position and the moves accepted as correct for it (up to 5; empty ignored).
struct BkCase {
    fen: &'static str,
    moves: [&'static str; 5],
}

/// The 24 Bratko-Kopec positions and their accepted moves (engine's best moves over depths 8–11).
#[rustfmt::skip]
const CASES: [BkCase; 24] = [
    BkCase { fen: "1k1r4/pp1b1R2/3q2pp/4p3/2B5/4Q3/PPP2B2/2K5 b - -", moves: ["d6d1", "", "", "", ""] },
    BkCase { fen: "3r1k2/4npp1/1ppr3p/p6P/P2PPPP1/1NR5/5K2/2R5 w - -", moves: ["d4d5", "", "", "", ""] },
    BkCase { fen: "2q1rr1k/3bbnnp/p2p1pp1/2pPp3/PpP1P1P1/1P2BNNP/2BQ1PRK/7R b - -", moves: ["a6a5", "c8c7", "c8d8", "f8g8", "e7d8"] },
    BkCase { fen: "rnbqkb1r/p3pppp/1p6/2ppP3/3N4/2P5/PPP1QPPP/R1B1KB1R w KQkq -", moves: ["e5e6", "", "", "", ""] },
    BkCase { fen: "r1b2rk1/2q1b1pp/p2ppn2/1p6/3QP3/1BN1B3/PPP3PP/R4RK1 w - -", moves: ["c3d5", "", "", "", ""] },
    BkCase { fen: "2r3k1/pppR1pp1/4p3/4P1P1/5P2/1P4K1/P1P5/8 w - -", moves: ["g5g6", "c2c4", "a2a3", "", ""] },
    BkCase { fen: "1nk1r1r1/pp2n1pp/4p3/q2pPp1N/b1pP1P2/B1P2R2/2P1B1PP/R2Q2K1 w - -", moves: ["a3d6", "f3g3", "d1b1", "", ""] },
    BkCase { fen: "4b3/p3kp2/6p1/3pP2p/2pP1P2/4K1P1/P3N2P/8 w - -", moves: ["e2c3", "f4f5", "", "", ""] },
    BkCase { fen: "2kr1bnr/pbpq4/2n1pp2/3p3p/3P1P1B/2N2N1Q/PPP3PP/2KR1B1R w - -", moves: ["d1e1", "f1d3", "f1b5", "f4f5", ""] },
    BkCase { fen: "3rr1k1/pp3pp1/1qn2np1/8/3p4/PP1R1P2/2P1NQPP/R1B3K1 b - -", moves: ["b6c5", "f6d7", "", "", ""] },
    BkCase { fen: "2r1nrk1/p2q1ppp/bp1p4/n1pPp3/P1P1P3/2PBB1N1/4QPPP/R4RK1 w - -", moves: ["f2f4", "a1a3", "h2h4", "", ""] },
    BkCase { fen: "r3r1k1/ppqb1ppp/8/4p1NQ/8/2P5/PP3PPP/R3R1K1 b - -", moves: ["d7f5", "", "", "", ""] },
    BkCase { fen: "r2q1rk1/4bppp/p2p4/2pP4/3pP3/3Q4/PP1B1PPP/R3R1K1 w - -", moves: ["b2b4", "", "", "", ""] },
    BkCase { fen: "rnb2r1k/pp2p2p/2pp2p1/q2P1p2/8/1Pb2NP1/PB2PPBP/R2Q1RK1 w - -", moves: ["d1d2", "d1e1", "", "", ""] },
    BkCase { fen: "2r3k1/1p2q1pp/2b1pr2/p1pp4/6Q1/1P1PP1R1/P1PN2PP/5RK1 w - -", moves: ["g4g7", "", "", "", ""] },
    BkCase { fen: "r1bqkb1r/4npp1/p1p4p/1p1pP1B1/8/1B6/PPPN1PPP/R2Q1RK1 w kq -", moves: ["d2e4", "", "", "", ""] },
    BkCase { fen: "r2q1rk1/1ppnbppp/p2p1nb1/3Pp3/2P1P1P1/2N2N1P/PPB1QP2/R1B2RK1 b - -", moves: ["a8c8", "c7c6", "h7h5", "", ""] },
    BkCase { fen: "r1bq1rk1/pp2ppbp/2np2p1/2n5/P3PP2/N1P2N2/1PB3PP/R1B1QRK1 b - -", moves: ["f7f5", "", "", "", ""] },
    BkCase { fen: "3rr3/2pq2pk/p2p1pnp/8/2QBPP2/1P6/P5PP/4RRK1 b - -", moves: ["d6d5", "e8e7", "", "", ""] },
    BkCase { fen: "r4k2/pb2bp1r/1p1qp2p/3pNp2/3P1P2/2N3P1/PPP1Q2P/2KRR3 w - -", moves: ["g3g4", "", "", "", ""] },
    BkCase { fen: "3rn2k/ppb2rpp/2ppqp2/5N2/2P1P3/1P5Q/PB3PPP/3RR1K1 w - -", moves: ["f5h6", "", "", "", ""] },
    BkCase { fen: "2r2rk1/1bqnbpp1/1p1ppn1p/pP6/N1P1P3/P2B1N1P/1B2QPP1/R2R2K1 b - -", moves: ["f6h5", "", "", "", ""] },
    BkCase { fen: "r1bqk2r/pp2bppp/2p5/3pP3/P2Q1P2/2N1B3/1PP3PP/R4RK1 b kq -", moves: ["f7f6", "c8f5", "", "", ""] },
    BkCase { fen: "r2qnrnk/p2b2b1/1p1p2pp/2pPpp2/1PP1P3/PRNBB3/3QNPPP/5RK1 w - -", moves: ["f2f4", "", "", "", ""] },
];

/// Format a case's non-empty accepted moves as a space-separated string.
fn accepted_moves_string(tc: &BkCase) -> String {
    tc.moves
        .iter()
        .filter(|m| !m.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn bratko_kopec_nnue_suite() {
    let net_path = std::env::var("PAWNSTAR_EVALFILE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/nnue/pawnstar-v12.bin").to_string()
    });
    if !std::path::Path::new(&net_path).exists() {
        common::summary("Bratko-Kopec NNUE: skipped (no net supplied)");
        return;
    }
    let depth: i32 = std::env::var("PAWNSTAR_BK_DEPTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_DEPTH);

    let mut game = Box::new(Game::new());
    // Default to a deterministic single-threaded search so the suite is reproducible (Lazy SMP
    // makes even a fixed-depth search non-deterministic). The C++ setenv("PAWNSTAR_THREADS","1",
    // overwrite=0); here the constructor already honoured any caller-set PAWNSTAR_THREADS, so
    // only apply the single-thread default when the env is absent.
    if std::env::var("PAWNSTAR_THREADS").is_err() {
        game.thread_count = 1;
    }
    if !game.nnue_network.load(&net_path) || !game.nnue_network.loaded {
        common::check(
            false,
            &format!("Bratko-Kopec NNUE: failed to load net '{}'", net_path),
        );
        assert_eq!(common::summary("BRATKO-KOPEC NNUE"), 0);
        return;
    }

    let t_overall = std::time::Instant::now();

    for (i, tc) in CASES.iter().enumerate() {
        game.set_position(tc.fen);
        game.book.free();
        game.time_control.clock_type = ClockType::FixedDepth;
        game.time_control.depth = depth;
        debug_x_clear();
        let t_start = std::time::Instant::now();
        let m = game.search_root_node();
        let elapsed_ms = t_start.elapsed().as_millis();
        debug_x_write();

        let got_move = m.to_string();
        let found = m != Move::none();
        let matched = found && tc.moves.iter().any(|mv| *mv == got_move);

        // A position is solved when a legal move comes back and it is among the accepted moves.
        common::check(
            matched,
            &format!(
                "BK pos{:02} got={}/{:<6} accepted=[{}] {}ms {}",
                i + 1,
                got_move,
                m.score(),
                accepted_moves_string(tc),
                elapsed_ms,
                tc.fen
            ),
        );
    }

    let total_ms = t_overall.elapsed().as_millis();
    let total = CASES.len() as i32;
    let failures = common::summary(&format!(
        "BRATKO-KOPEC NNUE: solved {}/{} at depth {} ({}ms, net {})",
        total - common::FAILURES.load(std::sync::atomic::Ordering::Relaxed),
        total,
        depth,
        total_ms,
        net_path
    ));
    assert_eq!(failures, 0);
}
