//! Static exchange evaluation tests. (Port of `test/see_test.cpp`.)

mod common;

use pawnstar::constants::Piece;
use pawnstar::moves::{Move, MoveList};
use pawnstar::position::Position;
use pawnstar::square::Square;
use pawnstar::static_exchange_evaluation::{
    evaluate_static_exchange, evaluate_static_exchange_reference,
};

/// A single static-exchange-evaluation test case.
struct SeeTest {
    name: &'static str,  // Human-readable case name.
    fen: &'static str,   // Position FEN.
    mv: Move,            // Move to evaluate.
    want_score: i32,     // Expected SEE score.
    want_checking: bool, // Expected give-check flag.
}

fn sq(name: &str) -> Square {
    Square::from_str(name)
}

/// The full set of SEE test cases.
fn tests() -> Vec<SeeTest> {
    vec![
        // ── C reference cases ─────────────────────────────────────────────────
        SeeTest {
            name: "rook takes pawn, no defenders",
            fen: "1k1r4/1pp4p/p7/4p3/8/P5P1/1PP4P/2K1R3 w - - 0 1",
            mv: Move::capture(sq("E1"), sq("E5"), Piece::Rook, Piece::Pawn),
            want_score: 100,
            want_checking: false,
        },
        SeeTest {
            name: "knight takes pawn, loses to defenders",
            fen: "1k1r3q/1ppn3p/p4b2/4p3/8/P2N2P1/1PP1R1BP/2K1Q3 w - - 0 1",
            mv: Move::capture(sq("D3"), sq("E5"), Piece::Knight, Piece::Pawn),
            want_score: -200,
            want_checking: false,
        },
        // ── Undefended captures ───────────────────────────────────────────────
        SeeTest {
            name: "bishop takes undefended knight",
            fen: "k7/8/8/8/3n4/2B5/8/K7 w - - 0 1",
            mv: Move::capture(sq("C3"), sq("D4"), Piece::Bishop, Piece::Knight),
            want_score: 300,
            want_checking: false,
        },
        // ── Equal and winning exchanges ───────────────────────────────────────
        SeeTest {
            name: "equal exchange: pawn takes pawn defended by pawn",
            fen: "7k/8/4p3/3p4/4P3/8/8/K7 w - - 0 1",
            mv: Move::capture(sq("E4"), sq("D5"), Piece::Pawn, Piece::Pawn),
            want_score: 0,
            want_checking: false,
        },
        SeeTest {
            name: "pawn takes knight defended only by pawn",
            fen: "7k/8/4p3/3n4/4P3/8/8/K7 w - - 0 1",
            mv: Move::capture(sq("E4"), sq("D5"), Piece::Pawn, Piece::Knight),
            want_score: 200,
            want_checking: false,
        },
        SeeTest {
            name: "bishop takes rook defended only by bishop",
            fen: "7k/8/4b3/3r4/8/1B6/8/K7 w - - 0 1",
            mv: Move::capture(sq("B3"), sq("D5"), Piece::Bishop, Piece::Rook),
            want_score: 200,
            want_checking: false,
        },
        // ── Losing captures ───────────────────────────────────────────────────
        SeeTest {
            name: "rook takes pawn defended by rook",
            fen: "2rk4/8/8/2p5/8/8/8/2RK4 w - - 0 1",
            mv: Move::capture(sq("C1"), sq("C5"), Piece::Rook, Piece::Pawn),
            want_score: -400,
            want_checking: false,
        },
        SeeTest {
            name: "knight takes pawn defended by king",
            fen: "8/8/8/4kp2/8/4N3/8/K7 w - - 0 1",
            mv: Move::capture(sq("E3"), sq("F5"), Piece::Knight, Piece::Pawn),
            want_score: -200,
            want_checking: false,
        },
        SeeTest {
            name: "queen takes pawn defended by queen",
            fen: "7k/8/8/3p3q/8/8/Q7/K7 w - - 0 1",
            mv: Move::capture(sq("A2"), sq("D5"), Piece::Queen, Piece::Pawn),
            want_score: -800,
            want_checking: false,
        },
        // ── X-ray attacker ────────────────────────────────────────────────────
        SeeTest {
            name: "x-ray: pawn takes queen, bishop revealed behind",
            fen: "7k/8/4p3/3q4/2P5/1B6/8/K7 w - - 0 1",
            mv: Move::capture(sq("C4"), sq("D5"), Piece::Pawn, Piece::Queen),
            want_score: 900,
            want_checking: false,
        },
        // ── Multi-piece exchange ──────────────────────────────────────────────
        SeeTest {
            name: "rook+bishop exchange over a pawn",
            fen: "4r1k1/8/8/4p3/3B4/8/8/4R1K1 w - - 0 1",
            mv: Move::capture(sq("E1"), sq("E5"), Piece::Rook, Piece::Pawn),
            want_score: 100,
            want_checking: false,
        },
        // ── Promotion captures ────────────────────────────────────────────────
        SeeTest {
            name: "promotion: pawn captures rook, promotes to queen, undefended",
            fen: "1r6/P7/8/5k2/8/8/8/K7 w - - 0 1",
            mv: Move::promotion(sq("A7"), sq("B8"), Piece::Queen, Piece::Rook),
            want_score: 1300,
            want_checking: false,
        },
        SeeTest {
            name: "promotion: pawn captures rook, promotes to queen, gives check",
            fen: "7r/6P1/8/4k3/8/8/8/K7 w - - 0 1",
            mv: Move::promotion(sq("G7"), sq("H8"), Piece::Queen, Piece::Rook),
            want_score: 1300,
            want_checking: true,
        },
        // ── Check detection ───────────────────────────────────────────────────
        SeeTest {
            name: "rook takes pawn, checks king on same rank",
            fen: "8/p6k/8/8/8/8/8/RK6 w - - 0 1",
            mv: Move::capture(sq("A1"), sq("A7"), Piece::Rook, Piece::Pawn),
            want_score: 100,
            want_checking: true,
        },
        // ── Additional promotion cases ────────────────────────────────────────
        SeeTest {
            name: "promotion with recapture chain",
            fen: "r6r/6P1/8/3k4/8/8/8/K6R w - - 0 1",
            mv: Move::promotion(sq("G7"), sq("H8"), Piece::Queen, Piece::Rook),
            want_score: 900,
            want_checking: false,
        },
        SeeTest {
            name: "underpromotion: pawn captures bishop, promotes to knight",
            fen: "k6b/6P1/8/8/8/8/8/K7 w - - 0 1",
            mv: Move::promotion(sq("G7"), sq("H8"), Piece::Knight, Piece::Bishop),
            want_score: 500,
            want_checking: false,
        },
        // ── Long recapture sequences ──────────────────────────────────────────
        SeeTest {
            name: "3-rook chain: white has extra rook, gains the pawn",
            fen: "4r2k/4r3/8/4p3/8/4R3/4R3/K3R3 w - - 0 1",
            mv: Move::capture(sq("E1"), sq("E5"), Piece::Rook, Piece::Pawn),
            want_score: 100,
            want_checking: false,
        },
        SeeTest {
            name: "5-piece exchange with stand-pat at two levels",
            fen: "7k/8/3p1b2/4r3/2NP1B2/3n4/8/K7 w - - 0 1",
            mv: Move::capture(sq("D4"), sq("E5"), Piece::Pawn, Piece::Rook),
            want_score: 400,
            want_checking: false,
        },
        // ── En passant captures ───────────────────────────────────────────────
        SeeTest {
            name: "en passant: wins an undefended pawn",
            fen: "7k/8/8/3Pp3/8/8/8/K7 w - e6 0 1",
            mv: Move::ep_capture(sq("D5"), sq("E6")),
            want_score: 100,
            want_checking: false,
        },
        SeeTest {
            name: "en passant: equal, recaptured by an adjacent pawn",
            fen: "7k/5p2/8/3Pp3/8/8/8/K7 w - e6 0 1",
            mv: Move::ep_capture(sq("D5"), sq("E6")),
            want_score: 0,
            want_checking: false,
        },
        // ── Battery / revealed attacker ───────────────────────────────────────
        SeeTest {
            name: "battery: rook in front of queen wins the pawn after the trade",
            fen: "4r2k/8/8/4p3/8/8/4R3/K3Q3 w - - 0 1",
            mv: Move::capture(sq("E2"), sq("E5"), Piece::Rook, Piece::Pawn),
            want_score: 100,
            want_checking: false,
        },
        // ── Losing capture that gives check ───────────────────────────────────
        SeeTest {
            name: "queen grabs a king-defended pawn, loses the queen but gives check",
            fen: "k7/p7/8/8/8/8/8/Q6K w - - 0 1",
            mv: Move::capture(sq("A1"), sq("A7"), Piece::Queen, Piece::Pawn),
            want_score: -800,
            want_checking: true,
        },
        // ── Battery vs a pawn-defended minor: rook behind bishop wins a pawn ───
        SeeTest {
            name: "bishop+rook battery beats a pawn-defended knight",
            fen: "7k/8/4p3/3n4/8/8/6B1/K2R4 w - - 0 1",
            mv: Move::capture(sq("G2"), sq("D5"), Piece::Bishop, Piece::Knight),
            want_score: 100,
            want_checking: false,
        },
        // ── Cross-color pawn recapture: two attackers vs one defender ─────────
        // White answers black's e6xd5 recapture with c4xd5 — the recapturing pawns
        // are of different colors, which the swap-off must follow across plies.
        SeeTest {
            name: "two pawn attackers vs one defender wins a pawn",
            fen: "7k/8/4p3/3p4/2P1P3/8/8/K7 w - - 0 1",
            mv: Move::capture(sq("E4"), sq("D5"), Piece::Pawn, Piece::Pawn),
            want_score: 100,
            want_checking: false,
        },
    ]
}

// Cross-check: the fast (MakeMove-free) evaluate_static_exchange must be bit-identical — score
// AND give-check flag — to the MakeMove reference for every capture/promotion. Walks a recursive
// move tree from capture/ep/promotion-rich roots so it exercises en passant, promotion captures,
// and discovered / promotion / en-passant-discovered checks.
fn cross_check_see(pos: &Position, depth: i32, see_checks: &mut i64, see_mismatches: &mut i64) {
    let moves: MoveList = pos.generate_legal_moves();
    for m in moves.iter() {
        if !m.is_quiet() {
            *see_checks += 1;
            let fast = evaluate_static_exchange(pos, *m);
            let reference = evaluate_static_exchange_reference(pos, *m);
            if fast != reference {
                *see_mismatches += 1;
                if *see_mismatches <= 8 {
                    println!(
                        "       SEE MISMATCH on {}: fast {{{},{}}} ref {{{},{}}}",
                        m.to_string(),
                        fast.0,
                        fast.1,
                        reference.0,
                        reference.1
                    );
                }
            }
        }
        if depth > 0 {
            cross_check_see(&pos.make_move(*m), depth - 1, see_checks, see_mismatches);
        }
    }
}

/// Run all SEE test cases.
#[test]
fn see_suite() {
    // Capture/ep/promotion-rich standard positions (startpos, kiwipete, perft pos3 ep, perft
    // pos4, a promotion pos).
    const CROSS_CHECK_FENS: [&str; 5] = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        "n1n5/PPPk4/8/8/8/8/4Kppp/5N1N b - - 0 1",
    ];
    let mut see_checks: i64 = 0;
    let mut see_mismatches: i64 = 0;
    for fen in CROSS_CHECK_FENS {
        cross_check_see(
            &Position::from_string(fen),
            3,
            &mut see_checks,
            &mut see_mismatches,
        );
    }
    common::check(
        see_mismatches == 0,
        &format!(
            "fast SEE == MakeMove reference over {} captures ({} mismatches)",
            see_checks, see_mismatches
        ),
    );

    let tests = tests();
    for tc in &tests {
        let pos = Position::from_string(tc.fen);
        let (score, is_checking) = evaluate_static_exchange(&pos, tc.mv);
        let ok = score == tc.want_score && is_checking == tc.want_checking;
        if !common::check(ok, tc.name) {
            if score != tc.want_score {
                println!("       score: got {} want {}", score, tc.want_score);
            }
            if is_checking != tc.want_checking {
                println!(
                    "       givesCheck: got {} want {}",
                    is_checking, tc.want_checking
                );
            }
        }
    }
    let total = tests.len() as i32;
    let failures = common::summary(&format!(
        "SEE TESTS: {}/{} cases passed",
        total - common::FAILURES.load(std::sync::atomic::Ordering::Relaxed),
        total
    ));
    assert_eq!(failures, 0);
}
