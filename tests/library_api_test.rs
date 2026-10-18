//! The library's small public helpers that the search itself never calls - the C++ API surface the
//! port carries one to one (`lib.rs`): bitboard and square conversions, the move list's stack
//! operations, the history table's reset and maximum, the transposition table's usage figures,
//! the net header's serialisation and the loader's refusals, the opening book's in-memory form
//! and its display, the material-draw rules and the FEN reader's fallbacks, and the CPU probe.

mod common;

use std::collections::HashSet;

use pawnstar::bitboard::Bitboard;
use pawnstar::castling_rights::CastlingRights;
use pawnstar::chess_clock::{ChessClock, ClockType};
use pawnstar::constants::{Color, Piece};
use pawnstar::history_table::HistoryTable;
use pawnstar::moves::{Move, MoveList, NodeType};
use pawnstar::nnue::{expected_net_header, NetHeader, Network, NET_HEADER_BYTES};
use pawnstar::opening_book::OpeningBook;
use pawnstar::position::Position;
use pawnstar::square::Square;
use pawnstar::transposition_table::{Transposition, TranspositionTable};

const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

fn sq(name: &str) -> Square {
    Square::from_str(name)
}

fn test_bitboard_and_square() {
    let b = Bitboard::from_square(sq("a1")) | Bitboard::from_square(sq("h8"));
    let text = b.to_string();
    check(
        text.lines().count() == 8
            && text.lines().next() == Some("00000001")
            && text.lines().last() == Some("10000000"),
        "bitboard to_string prints rank 8 first, file a on the left",
    );
    check(b.msb() == sq("h8") && b.lsb() == sq("a1"), "msb and lsb");
    let d4 = Bitboard::from_square(sq("d4"));
    check(
        d4.shift_east() == Bitboard::from_square(sq("e4")),
        "shift east",
    );
    check(
        d4.shift_west() == Bitboard::from_square(sq("c4")),
        "shift west",
    );
    check(
        Bitboard::from_square(sq("h4")).shift_east().is_empty()
            && Bitboard::from_square(sq("a4")).shift_west().is_empty(),
        "shifts off the board's edge vanish",
    );
    check(Square::new(28) == sq("e4"), "Square::new");
    check(
        Square::from(28u8) == sq("e4") && u8::from(sq("e4")) == 28,
        "Square from / into u8",
    );
}

fn test_castling_and_color() {
    check(
        CastlingRights::from_raw(0).to_fen_string() == "-",
        "no castling rights print '-'",
    );
    check(
        CastlingRights::from_raw(0b1111).to_fen_string() == "KQkq",
        "all castling rights",
    );
    check(
        CastlingRights::from_fen("KxQ").to_fen_string() == "KQ",
        "unknown castling letters are ignored",
    );
    check(
        Color::from_u8(0) == Color::White && Color::from_u8(1) == Color::Black,
        "Color::from_u8",
    );
    check(Piece::from_u8(6) == Piece::King, "Piece::from_u8");
}

fn test_move_list() {
    let mut list = MoveList::default();
    check(
        list.is_empty() && list.size() == 0,
        "a default move list is empty",
    );
    let a = Move::non_capture(sq("e2"), sq("e4"), Piece::Pawn);
    let b = Move::non_capture(sq("g1"), sq("f3"), Piece::Knight);
    list.push_back(a);
    list.push_back(b);
    check(
        !list.is_empty() && *list.back() == b,
        "back is the last pushed",
    );
    list.back_mut().assign_score(77);
    check(
        list.back().score() == 77,
        "back_mut writes the last element",
    );
    let collected: Vec<Move> = (&list).into_iter().copied().collect();
    check(collected == vec![a, b], "iterating by reference");
    check(list.pop_back() == b && list.size() == 1, "pop_back");
    list.clear();
    check(list.is_empty(), "clear");

    // A move's hash, like its equality, sees only its identity: a scored copy is the same key.
    let mut scored = a;
    scored.assign_score(-123);
    let set: HashSet<Move> = [a, scored, b].into_iter().collect();
    check(set.len() == 2, "a move's hash ignores its score");
}

fn test_history_table() {
    let mut table = HistoryTable::default();
    check(
        table.max_count() == 0,
        "a fresh history table's maximum is 0",
    );
    let mv = Move::non_capture(sq("e2"), sq("e4"), Piece::Pawn);
    table.apply_bonus(Color::White, mv, 300);
    table.record_good_move(Color::Black, mv);
    check(
        table.max_count() == table.get_count(Color::White, mv),
        "max_count finds the largest entry",
    );
    table.reset();
    check(
        table.max_count() == 0 && table.get_count(Color::White, mv) == 0,
        "reset zeroes the table",
    );
}

fn test_transposition_table() {
    let table = TranspositionTable::new(1);
    check(table.usage_stats() == (0, 0), "an empty table is 0 % used");
    check(
        table.hashfull_permille() == 0,
        "an empty table is 0 per mille full",
    );
    let mv = Move::non_capture(sq("e2"), sq("e4"), Piece::Pawn);
    table.record_transposition(&Transposition::with_data(
        0x1234_5678_9abc_def1,
        mv,
        42,
        7,
        NodeType::Pv,
    ));
    let (count, _) = table.usage_stats();
    check(count == 1, "one entry recorded is one entry used");
    let found = table
        .find_transposition(0x1234_5678_9abc_def1)
        .expect("the entry is found");
    check(
        found.score() == 42 && found.depth() == 7 && found.node_type() == NodeType::Pv,
        "the entry reads back",
    );
    let first_age = found.age();
    table.age();
    table.record_transposition(&Transposition::with_data(
        0x1234_5678_9abc_def1,
        mv,
        1,
        1,
        NodeType::Cut,
    ));
    let again = table
        .find_transposition(0x1234_5678_9abc_def1)
        .expect("the entry is found");
    check(
        again.age() == first_age.wrapping_add(1),
        "an entry carries the table's generation",
    );
}

fn test_net_header_and_loader() {
    let header = expected_net_header();
    let bytes = header.to_bytes();
    check(bytes.len() == NET_HEADER_BYTES, "the header is 32 bytes");
    check(
        NetHeader::from_bytes(&bytes) == header,
        "the header round-trips",
    );
    check(
        bytes[18..].iter().all(|&b| b == 0),
        "the header pads with zeros",
    );

    let mut net = Network::default();
    check(!net.loaded, "a default network is unloaded");
    check(!net.load("no/such/net.bin"), "a missing file is refused");
    check(!net.load("nnue"), "a directory is refused");
    check(
        !net.load_from_memory(b"", "empty"),
        "an empty image is refused",
    );
    check(
        !net.load_from_memory(&[0u8; 64], "raw"),
        "an unstamped image is refused",
    );

    let mut wrong = header;
    wrong.hidden_size = 512;
    check(
        !net.load_from_memory(&wrong.to_bytes(), "wrong"),
        "an image of another architecture is refused",
    );
    let mut truncated = bytes.to_vec();
    truncated.extend_from_slice(&[0u8; 100]);
    check(
        !net.load_from_memory(&truncated, "truncated"),
        "a truncated image is refused",
    );
    check(!net.loaded, "a refused net leaves the network unloaded");
}

fn test_opening_book() {
    let mut book = OpeningBook::default();
    check(
        !book.initialize("no/such/book"),
        "a missing book file is refused",
    );

    // Move numbers and comments are skipped; a '?' move is recorded as questionable.
    let text = b"1. e2e4 e7e5 2. g1f3 # the main line\n\n1. e2e4 c7c5?\nd2d4 d7d5 # comment\n";
    check(
        book.initialize_from_memory(text),
        "an in-memory book parses",
    );
    let start = Position::from_string(START_FEN);
    check(
        book.get_move(start.hash) != Move::none(),
        "the start position has book moves",
    );
    let e4 = start.make_move(
        start
            .generate_legal_moves()
            .iter()
            .copied()
            .find(|m| m.to_string() == "e2e4")
            .unwrap(),
    );
    check(
        book.get_move(e4.hash).to_string() == "e7e5",
        "a questionable reply is never chosen",
    );
    book.display_available_moves(&start);
    book.display_available_moves(&e4); // with its questionable move
    book.display_available_moves(&Position::from_string("4k3/8/8/8/8/8/8/4K3 w - - 0 1"));

    let mut broken = OpeningBook::new();
    check(
        !broken.initialize_from_memory(b"e2e4 e7e5\ne2e4 e2e4\n"),
        "an illegal move in a line refuses the book",
    );
}

fn test_material_draws() {
    let draws = [
        ("4k3/8/8/8/8/8/8/4K3 w - - 0 1", true, "king against king"),
        (
            "4k3/8/8/8/8/8/8/2B1K3 w - - 0 1",
            true,
            "king and bishop against king",
        ),
        (
            "4k3/8/8/8/8/8/8/1N2K3 w - - 0 1",
            true,
            "king and knight against king",
        ),
        (
            "4k3/8/8/8/8/8/8/R3K3 w - - 0 1",
            false,
            "king and rook against king",
        ),
        (
            "2b1k3/8/8/8/8/8/8/2B1K3 w - - 0 1",
            false,
            "bishops on unlike squares",
        ),
        (
            "3bk3/8/8/8/8/8/8/2B1K3 w - - 0 1",
            true,
            "bishops on like squares",
        ),
        (
            "4k3/8/8/8/8/8/8/1NB1K3 w - - 0 1",
            false,
            "two minor pieces of one side",
        ),
        ("4k3/p7/8/8/8/8/P7/4K3 w - - 0 1", false, "pawns"),
    ];
    for (fen, want, what) in draws {
        check(
            Position::from_string(fen).is_draw_by_material() == want,
            &format!("material draw: {what}"),
        );
    }
    // A board of fewer than two pieces cannot arise from a legal FEN; the default position has none.
    check(
        Position::default().is_draw_by_material(),
        "an empty board is a draw by material",
    );
}

fn test_fen_fallbacks() {
    // A board field that runs off the board falls back to the start position.
    check(
        Position::from_string("k7/8/8/8/8/8/8/K7p w - - 0 1").to_string() == START_FEN,
        "a ninth file falls back to the start position",
    );
    check(
        Position::from_string("k7/8/8/8/8/8/8/K7/p w - - 0 1").to_string() == START_FEN,
        "a ninth rank falls back to the start position",
    );
    // A halfmove clock that is not a number reads 0, and the fullmove field is then not read.
    check(
        Position::from_string("4k3/8/8/8/8/8/8/4K3 b - - x 40").to_string()
            == "4k3/8/8/8/8/8/8/4K3 b - - 0 1",
        "a garbled halfmove clock reads 0 and skips the fullmove number",
    );
    check(
        Position::from_string("4k3/8/8/8/8/8/8/4K3 b - - 7").to_string()
            == "4k3/8/8/8/8/8/8/4K3 b - - 7 1",
        "a missing fullmove number keeps its default",
    );
}

fn test_clock_and_platform() {
    let clock = ChessClock::default();
    check(
        clock.clock_type == ClockType::Standard,
        "a default clock is a standard clock",
    );
    let first = pawnstar::platform::has_avx_vnni();
    check(
        first == pawnstar::platform::has_avx_vnni(),
        "the CPU probe is stable",
    );
    if !cfg!(target_arch = "x86_64") {
        check(!first, "no AVX-VNNI off x86-64");
    }
    check(
        !pawnstar::version::version_string().is_empty(),
        "a version string",
    );
}

#[test]
fn library_api_suite() {
    test_bitboard_and_square();
    test_castling_and_color();
    test_move_list();
    test_history_table();
    test_transposition_table();
    test_net_header_and_loader();
    test_opening_book();
    test_material_draws();
    test_fen_fallbacks();
    test_clock_and_platform();
    assert_eq!(
        common::summary("Library API: helpers, loaders and fallbacks the search does not call"),
        0,
        "library api suite reported failures"
    );
}
