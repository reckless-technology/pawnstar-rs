//! The fifty-move clock: what moves it, what restarts it, and what reads it.
//!
//! FIDE Article 9.3 counts fifty moves by each player "without the movement of any pawn and
//! without any capture", and a FEN's halfmove clock is the number of plies since the last capture
//! or pawn advance. Castling is neither, so it advances the clock like any other piece move. It is
//! irreversible, but that is the repetition scan's business, not the clock's: the castling rights
//! it gives up are in the hash, so no position before a castling can recur after it. This suite
//! holds `make_move`, the FEN round trip, the fifty-move draw in the search and the repetition scan
//! to that, and the search to FIDE 5.1.1: a checkmate on the hundredth ply is mate, not a draw.

mod common;

use pawnstar::chess_clock::ClockType;
use pawnstar::constants::{ALPHA, BETA, CHECKMATED_SCORE, DRAW_SCORE};
use pawnstar::game::Game;
use pawnstar::moves::{Move, MoveType};
use pawnstar::position::Position;
use pawnstar::search_state::{evaluate_position, SearchState, Variation};

const NET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/nnue/pawnstar-v20.bin");

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

/// The legal move `uci` (in coordinate notation) from `position`, or `Move::none()`.
fn legal_move(position: &Position, uci: &str) -> Move {
    let moves = position.generate_legal_moves();
    for mv in moves.iter() {
        if mv.to_string() == uci {
            return *mv;
        }
    }
    Move::none()
}

/// Play `uci` from `fen` and return the position after it, checking the move is legal.
fn after(fen: &str, uci: &str) -> Position {
    let position = Position::from_string(fen);
    let mv = legal_move(&position, uci);
    check(mv != Move::none(), &format!("{uci} is legal from {fen}"));
    position.make_move(mv)
}

/// A single-threaded game on `fen` with every move of `moves` played on it, on the shipped net
/// where it is present (so an eval that is not a draw reads as the material it sees).
fn game_after(fen: &str, moves: &[&str]) -> Box<Game> {
    let mut game = Box::new(Game::new());
    game.thread_count = 1;
    game.use_own_book = false;
    // A build of another NNUE shape refuses this net; the clock rules under test do not depend on
    // the evaluation, so the game simply runs without one there.
    if common::is_shipped_net_this_builds_shape() {
        check(
            game.nnue_network.load(NET).is_loaded(),
            "the shipped net loads",
        );
    }
    game.set_position(fen);
    for uci in moves {
        check(
            game.play_move_str(uci) != Move::none(),
            &format!("{uci} plays in the game from {fen}"),
        );
    }
    game
}

/// Each of the four castlings advances the clock by one, and the FEN it leaves reads back to the
/// same position, hash and all.
fn test_castling_advances_the_clock() {
    let white = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 7 20";
    let black = "r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 7 20";
    let cases = [
        (white, "e1g1", "r3k2r/8/8/8/8/8/8/R4RK1 b kq - 8 20"),
        (white, "e1c1", "r3k2r/8/8/8/8/8/8/2KR3R b kq - 8 20"),
        (black, "e8g8", "r4rk1/8/8/8/8/8/8/R3K2R w KQ - 8 21"),
        (black, "e8c8", "2kr3r/8/8/8/8/8/8/R3K2R w KQ - 8 21"),
    ];
    for (fen, uci, want) in cases {
        let position = Position::from_string(fen);
        let mv = legal_move(&position, uci);
        check(
            mv != Move::none() && mv.type_() == MoveType::Castling,
            &format!("{uci} is a castling"),
        );
        let next = position.make_move(mv);
        check(
            next.reversible_move_count == 8,
            &format!(
                "{uci} advances the clock from 7 to 8 (got {})",
                next.reversible_move_count
            ),
        );
        let written = next.to_string();
        check(
            written == want,
            &format!("{uci} writes {want} (got {written})"),
        );
        let read_back = Position::from_string(&written);
        check(
            read_back.to_string() == written
                && read_back.hash == next.hash
                && read_back.reversible_move_count == next.reversible_move_count,
            &format!("the FEN after {uci} reads back to the same position"),
        );
    }

    // From a fresh clock, castling counts from zero like any other piece move.
    let kiwipete = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
    check(
        after(kiwipete, "e1g1").to_string()
            == "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R4RK1 b kq - 1 1",
        "castling from a clock of 0 leaves 1",
    );
}

/// A capture or a pawn move of any kind restarts the clock; every other move advances it.
fn test_what_restarts_the_clock() {
    let cases = [
        (
            "4k3/8/8/8/8/8/4P3/4K3 w - - 30 40",
            "e2e3",
            0,
            "a single pawn push",
        ),
        (
            "4k3/8/8/8/8/8/4P3/4K3 w - - 30 40",
            "e2e4",
            0,
            "a double pawn push",
        ),
        (
            "4k3/8/8/3p4/4P3/8/8/4K3 w - - 30 40",
            "e4d5",
            0,
            "a pawn capture",
        ),
        (
            "4k3/8/8/3pP3/8/8/8/4K3 w - d6 30 40",
            "e5d6",
            0,
            "an en passant capture",
        ),
        (
            "4k3/P7/8/8/8/8/8/4K3 w - - 30 40",
            "a7a8q",
            0,
            "a promotion",
        ),
        (
            "1n2k3/P7/8/8/8/8/8/4K3 w - - 30 40",
            "a7b8n",
            0,
            "a capturing promotion",
        ),
        (
            "4k3/8/8/3n4/8/8/8/3RK3 w - - 30 40",
            "d1d5",
            0,
            "a piece capture",
        ),
        (
            "4k3/8/8/8/8/8/8/3RK3 w - - 30 40",
            "d1d5",
            31,
            "a quiet rook move",
        ),
        (
            "4k3/8/8/8/8/8/8/4K3 w - - 30 40",
            "e1d2",
            31,
            "a quiet king move",
        ),
        ("4k3/8/8/8/8/8/8/4K2R w K - 30 40", "e1g1", 31, "castling"),
    ];
    for (fen, uci, want, what) in cases {
        let clock = after(fen, uci).reversible_move_count;
        check(
            clock == want,
            &format!("{what} leaves the clock at {want} (got {clock})"),
        );
    }
}

/// A castling on the hundredth ply is a fifty-move draw: the search's own test sees it, the eval's
/// short-circuit returns the draw score, and a search whose every move reaches the hundredth ply
/// scores the root a draw, however much material is ahead.
fn test_fifty_move_draw_across_castling() {
    for (fen, uci) in [
        ("4k3/8/8/8/8/8/8/4K2R w K - 99 80", "e1g1"),
        ("r3k3/8/8/8/8/8/8/4K3 b q - 99 80", "e8c8"),
    ] {
        let game = game_after(fen, &[uci]);
        check(
            game.current_position().reversible_move_count == 100,
            &format!("{uci} at a clock of 99 reaches 100"),
        );
        let mut state = SearchState::new(&game);
        check(
            state.is_draw_by_fifty_moves(),
            &format!("{uci} on the hundredth ply is a fifty-move draw"),
        );
        check(
            evaluate_position(&mut state) == DRAW_SCORE,
            &format!("the eval after {uci} on the hundredth ply is the draw score"),
        );
    }

    // A queen and a rook up with no pawn and nothing to capture: every white move is the hundredth
    // ply. Restarting the clock on castling made e1g1 the one move that kept the win.
    let mut game = game_after("k7/8/8/8/8/8/8/3QK2R w K - 99 80", &[]);
    game.time_control.clock_type = ClockType::FixedDepth;
    game.time_control.depth = 4;
    let best = game.search_root_node();
    check(
        best != Move::none() && best.score() == DRAW_SCORE,
        &format!(
            "every root move reaches the hundredth ply, so the root is a draw (best {} at {})",
            best.to_string(),
            best.score()
        ),
    );
}

/// Checkmate takes precedence over the fifty-move draw (FIDE 5.1.1): a mate on the move that
/// completes the fifty stands. At a clock of 99 Ra8 is the hundredth ply and mates, so the root
/// search plays it as mate in one where every other move draws. The reverse at the hundredth ply:
/// a check that leaves a legal move (with the f7 pawn gone, Ra8+ leaves the king f7) and a quiet
/// move (Ra2) both still draw.
fn test_mate_takes_precedence_over_the_fifty_move_draw() {
    let mating = "6k1/5ppp/8/8/8/8/8/R5K1 w - - 99 80";
    let mut game = game_after(mating, &[]);
    game.time_control.clock_type = ClockType::FixedDepth;
    game.time_control.depth = 4;
    let best = game.search_root_node();
    check(
        best.to_string() == "a1a8" && best.score() == -(CHECKMATED_SCORE + 1),
        &format!(
            "a mate on the hundredth ply is mate in one and is played (best {} at {})",
            best.to_string(),
            best.score()
        ),
    );

    let move_score = |fen: &str, uci: &str| -> i32 {
        let mut game = game_after(fen, &[]);
        game.time_control.clock_type = ClockType::FixedDepth;
        let mv = legal_move(&game.current_position(), uci);
        check(mv != Move::none(), &format!("{uci} is legal from {fen}"));
        let mut state = SearchState::new(&game);
        let mut pv = Variation::new();
        state
            .search_single_move(4, 0, ALPHA, BETA, mv, &mut pv, 0)
            .score()
    };
    let score = move_score(mating, "a1a8");
    check(
        score == -(CHECKMATED_SCORE + 1),
        &format!("the mate on the hundredth ply scores mate in one (got {score})"),
    );
    let score = move_score("6k1/6pp/8/8/8/8/8/R5K1 w - - 99 80", "a1a8");
    check(
        score == DRAW_SCORE,
        &format!("a check on the hundredth ply that leaves a legal move draws (got {score})"),
    );
    let score = move_score(mating, "a1a2");
    check(
        score == DRAW_SCORE,
        &format!("a quiet move on the hundredth ply draws (got {score})"),
    );
}

/// The repetition scan stops at a clock of zero, so after a castling it walks on past it - and
/// finds no false match there, since the rights the castling gave up are in the hash. Here the
/// king and rook walk back to e1 and h1 after castling: the same placement as the start, but
/// without the right to castle, so the start is a different position and only the repeats after
/// it count.
fn test_repetition_scan_across_castling() {
    let start = "4k3/8/8/8/8/8/8/4K2R w K - 0 1";
    let back_home = [
        "e1g1", "e8d8", "g1g2", "d8d7", "f1h1", "d7e8", "g2f1", "e8d8", "f1e1", "d8e8",
    ];
    let cycle = ["h1h2", "e8d8", "h2h1", "d8e8"];

    let game = game_after(start, &back_home);
    check(
        game.current_position().reversible_move_count == 10,
        "castling and the walk back leave the clock at 10, unbroken back to the start",
    );
    check(
        game.current_position().hash != Position::from_string(start).hash,
        "the walk back is not the start: the castling right is gone",
    );

    let mut moves: Vec<&str> = back_home.to_vec();
    moves.extend_from_slice(&cycle);
    let game = game_after(start, &moves);
    check(
        !SearchState::new(&game).is_draw_by_repetition(),
        "the second occurrence after castling is not a threefold, and the start does not count",
    );

    moves.extend_from_slice(&cycle);
    let game = game_after(start, &moves);
    check(
        SearchState::new(&game).is_draw_by_repetition(),
        "the third occurrence after castling is a threefold repetition",
    );
}

#[test]
fn fifty_move_clock_suite() {
    test_castling_advances_the_clock();
    test_what_restarts_the_clock();
    test_fifty_move_draw_across_castling();
    test_mate_takes_precedence_over_the_fifty_move_draw();
    test_repetition_scan_across_castling();
    let failures = common::summary("FIFTY-MOVE CLOCK TESTS");
    assert_eq!(failures, 0);
}
