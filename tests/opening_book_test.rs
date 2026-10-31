//! Unit tests for the opening book, focused on the '?' (questionable) move suffix.
//! (Port of `test/opening_book_test.cpp`.)
//!
//! A questionable move (written "f2f4?" in the book) must be recognised and replayed so the rest
//! of the line parses, but must never be returned by get_move. The move that follows it in the
//! line must still be recorded and selectable. These tests build a tiny book in a temp file and
//! assert exactly that.
//!
//! The suite then checks the **shipped** `pawnstar.book` itself: that it loads, and that its `#`
//! header still states the provenance it was replaced for - the Lichess open database under CC0 1.0,
//! credited as the courtesy it is, with no trace of the false TSCP public-domain claim.

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
            common::check(
                false,
                &format!("opening book: test bug - illegal move {}", coord),
            );
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
    common::check(
        !ever_played,
        "questionable move f2f4? must never be returned by GetMove",
    );

    // The line continued past the '?' move: the reply exf4 is recorded and selectable.
    let reply = book.get_move(hash_after(&["e2e4", "e7e5", "f2f4"]));
    common::check(
        reply != Move::none() && reply.to_string() == "e5f4",
        "exf4 should be playable after the questionable f2f4",
    );

    let _ = std::fs::remove_file(&book_path);

    check_shipped_book_provenance();

    let failures = common::summary("OPENING BOOK: '?' (questionable) move handling and provenance");
    assert_eq!(failures, 0);
}

/// The shipped book's own provenance header, which is the reason this book exists at all. The book it
/// replaced was derived from TSCP's `book.txt` and was asserted to be public domain throughout the
/// three ports; TSCP is Copyright 1997 Tom Kerrigan under an informal non-commercial permission, which
/// GPLv3 cannot take, so the claim was false and the material had to go. This book is built from the
/// Lichess open database, whose CC0 1.0 dedication makes the same claim true. Pinned here because a
/// false provenance claim is exactly the kind of defect that survives a rewrite of the prose around
/// it: the file itself has to say where it came from, and must not say the old thing. The bytes are
/// shared with ../pawnstar and ../pawnstar-go, which check the same things.
fn check_shipped_book_provenance() {
    let shipped_path = concat!(env!("CARGO_MANIFEST_DIR"), "/pawnstar.book");
    let shipped = std::fs::read_to_string(shipped_path).expect("read the shipped pawnstar.book");

    // The shipped book loads, which is what the engine does with it at startup.
    let mut shipped_book = OpeningBook::new();
    common::check(
        shipped_book.initialize(shipped_path),
        "the shipped book parses",
    );
    common::check(
        shipped_book.get_move(hash_after(&[])) != Move::none(),
        "the shipped book has moves for the start position",
    );

    common::check(
        shipped.contains("Lichess open database")
            && shipped.contains("https://database.lichess.org/")
            && shipped.contains("CC0 1.0")
            && shipped.contains("https://creativecommons.org/publicdomain/zero/1.0/"),
        "the shipped book names its Lichess source and the CC0 dedication",
    );
    common::check(
        !shipped.contains("TSCP") && !shipped.contains("Kerrigan"),
        "the shipped book makes no claim about TSCP",
    );
    // CC0 waives attribution, so the header must not imply an obligation that does not exist -
    // claiming a duty nobody owes is the mirror image of claiming a permission nobody gave.
    common::check(
        shipped.contains("asks for no attribution"),
        "the shipped book says the Lichess credit is a courtesy, not an obligation",
    );
    // Every line below the header parses, which the initialize above already proves - but the parser
    // stops at the first bad line and reports failure, so count the lines too: a book truncated to its
    // header would load perfectly and play nothing, and only the start-position check above would catch
    // it. This pins the width that was the other half of the point: 385 lines replayed each opening
    // about 13 times in a long match, which inflated every confidence interval the matches produced.
    let shipped_lines = shipped
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .count();
    common::check(
        shipped_lines > 385,
        "the shipped book is wider than the 385-line book it replaced",
    );

    // The book has been through `tools/filter_book`, which lives in ../pawnstar because it needs the
    // engine's own evaluation: a move scoring far enough below its position's best carries a '?', which
    // the parser replays but never selects. Pinned for the same reason as the line count above - an
    // unfiltered book loads perfectly and plays every move the database offered, including the ones this
    // engine evaluates as losing a pawn, and nothing else here would notice. Regenerating means make_book
    // THEN filter_book, and this is what catches forgetting the second step.
    common::check(
        shipped.contains('?'),
        "the shipped book carries questionable-move marks",
    );
    // Where every move a position had is marked, filter_book adds the engine's own choice there, unmarked.
    // Those are the only moves in the file that are not the database's, so the file has to say so: the
    // header's CC0 claim is about the database's lines and a reader must not take it to cover these.
    common::check(
        shipped.contains("NOT THE DATABASE'S"),
        "the shipped book distinguishes the engine's own moves from the database's",
    );

    // And the invariant that addition exists for: no position the book reaches is left with nothing to
    // play. A marked move is not selectable, so a position whose every move is marked would return
    // Move::none and drop the engine out of book - silently, and exactly where an opponent had just
    // played something unusual. Every position a line continues from must offer a selectable move; the
    // last position of a line is a leaf and is allowed to have none.
    let mut dead_ends = 0;
    for line in shipped.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let moves: Vec<&str> = line
            .split('#')
            .next()
            .unwrap_or("")
            .split_whitespace()
            .map(|token| token.trim_end_matches('?'))
            .collect();
        for ply in 1..moves.len() {
            if shipped_book.get_move(hash_after(&moves[..ply])) == Move::none() {
                dead_ends += 1;
            }
        }
    }
    common::check(
        dead_ends == 0,
        "every position the shipped book continues from has a move it may actually play",
    );
}
