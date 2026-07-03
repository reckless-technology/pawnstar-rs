//! Unit tests for the opening book, focused on the '?' (questionable) move suffix.
//! (Port of `test/opening_book_test.cpp`.)
//!
//! A questionable move (written "f2f4?" in the book) must be recognised and replayed so the rest
//! of the line parses, but must never be returned by get_move. The move that follows it in the
//! line must still be recorded and selectable. These tests build a tiny book in a temp file and
//! assert exactly that.

mod common;

use pawnstar::generated_data::ZobristT;
use pawnstar::moves::Move;
use pawnstar::opening_book::OpeningBook;
use pawnstar::position::Position;

const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Advance a position by a coordinate move (e.g. "e2e4"), matching the parser's lookup.
fn play(position: &Position, coord: &str) -> Position {
    let moves = position.generate_legal_moves();
    match moves.iter().find(|m| m.to_string() == coord) {
        Some(m) => position.make_move(*m),
        None => {
            common::check(false, &format!("opening book: test bug - illegal move {}", coord));
            *position
        }
    }
}

/// Hash of the position reached by replaying a sequence of coordinate moves from the start.
fn hash_after(moves: &[&str]) -> ZobristT {
    let mut p = Position::from_string(START_FEN);
    for m in moves {
        p = play(&p, m);
    }
    p.hash
}

#[test]
fn opening_book_suite() {
    // Line: 1.e4 e5 2.f4? (King's Gambit, marked questionable) exf4 3.Nf3
    let book_text = "e2e4 e7e5 f2f4? e5f4 g1f3\n";
    let book_path = std::env::temp_dir().join("pawnstar_book_test.book");
    std::fs::write(&book_path, book_text).expect("write temp book");

    let mut book = OpeningBook::new();
    common::check(
        book.initialize(book_path.to_str().unwrap()),
        "book with a '?' move should parse successfully",
    );

    // Playable moves along the line are selectable.
    common::check(
        book.get_move(hash_after(&[])) != Move::none(),
        "e2e4 should be a playable book move from the start",
    );
    common::check(
        book.get_move(hash_after(&["e2e4"])) != Move::none(),
        "e7e5 should be playable after e2e4",
    );

    // The questionable move f2f4 must never be returned, even over many draws.
    let after_e4_e5 = hash_after(&["e2e4", "e7e5"]);
    let mut ever_played = false;
    for _ in 0..1000 {
        if book.get_move(after_e4_e5) != Move::none() {
            ever_played = true;
            break;
        }
    }
    common::check(!ever_played, "questionable move f2f4? must never be returned by GetMove");

    // The line continued past the '?' move: the reply exf4 is recorded and selectable.
    let reply = book.get_move(hash_after(&["e2e4", "e7e5", "f2f4"]));
    common::check(
        reply != Move::none() && reply.to_string() == "e5f4",
        "exf4 should be playable after the questionable f2f4",
    );

    let _ = std::fs::remove_file(&book_path);

    let failures = common::summary("OPENING BOOK: '?' (questionable) move handling");
    assert_eq!(failures, 0);
}
