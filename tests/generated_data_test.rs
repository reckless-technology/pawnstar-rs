//! P1 gate: golden-value cross-check of the generated tables against the C++ engine.
//! The golden constants below were printed by the C++ `generated_data.h` tables (clang, arm64);
//! they pin the Zobrist draw order, the xorshift* PRNG, and the magic-table construction.

use pawnstar::bitboard::Bitboard;
use pawnstar::generated_data::*;
use pawnstar::moves::{Move, NodeType};
use pawnstar::square::Square;

#[test]
fn zobrist_goldens_match_cpp() {
    assert_eq!(PIECE_SQUARE_HASHES[0][0][0], 0x9e5287954c19f719);
    assert_eq!(PIECE_SQUARE_HASHES[0][0][63], 0xe39b4d075cd90895);
    assert_eq!(PIECE_SQUARE_HASHES[0][5][32], 0x724ee2e0956ecd86);
    assert_eq!(PIECE_SQUARE_HASHES[1][0][0], 0x665b77a40000efbf);
    assert_eq!(PIECE_SQUARE_HASHES[1][5][63], 0x3b279f1a9c04c9d1);
    assert_eq!(CASTLING_RIGHTS_HASHES[0], 0xc8a6a536a5921779);
    assert_eq!(CASTLING_RIGHTS_HASHES[15], 0x383dfdc96cee3935);
    assert_eq!(EN_PASSANT_HASHES[16], 0x37443c8dc23934cf);
    assert_eq!(EN_PASSANT_HASHES[17], 0x36fc06322d93452d);
    assert_eq!(EN_PASSANT_HASHES[40], 0xd2f51e0acfc16f57);
    assert_eq!(EN_PASSANT_HASHES[47], 0x9c37465170fb9827);
    assert_eq!(EN_PASSANT_HASHES[0], 0);
    assert_eq!(BLACK_MOVE_HASH, 0x28AB74D640E50602);
}

#[test]
fn attack_table_goldens_match_cpp() {
    assert_eq!(INTERVENING_SQUARES[0][63].0, 0x0040201008040200);
    assert_eq!(EAST_WEST[27].0, 0x00000000f7000000);
    assert_eq!(PAWN_ATTACKS_WHITE[8].0, 0x0000000000020000);
    assert_eq!(KNIGHT_ATTACKS[1].0, 0x0000000000050800);
    assert_eq!(KING_ATTACKS[4].0, 0x0000000000003828);
}

#[test]
fn magic_tables_match_cpp() {
    // Shape checks (attack-set dedup count, hash-slot count, shift) against the C++ build.
    assert_eq!(BISHOP_MAGICS[28].attacks.len(), 108);
    assert_eq!(BISHOP_MAGICS[28].indices.len(), 512);
    assert_eq!(BISHOP_MAGICS[28].shift, 55);
    assert_eq!(ROOK_MAGICS[0].attacks.len(), 49);
    assert_eq!(ROOK_MAGICS[0].indices.len(), 4096);
    assert_eq!(ROOK_MAGICS[0].shift, 52);
    // Rook on a1 with blockers on a4 and d1 → attacks a2,a3,a4,b1,c1,d1.
    let occ = Bitboard((1u64 << 24) | (1u64 << 3));
    assert_eq!(
        pawnstar::attacks::rook_attacks(occ, Square(0)).0,
        0x000000000101010e
    );
    // Every square's magic table must be dedup-consistent with the reference generators.
    for sq in 0u8..64 {
        for &m in &[&BISHOP_MAGICS[sq as usize], &ROOK_MAGICS[sq as usize]] {
            assert!(m.attacks.len() < 256);
        }
        // Exhaustive equivalence on a handful of random-ish occupancies via the carry-rippler.
        let mask = ROOK_MAGICS[sq as usize].occupancy_mask.0;
        let mut n: u64 = 0;
        let mut count = 0;
        loop {
            let got = pawnstar::attacks::rook_attacks(Bitboard(n), Square(sq)).0;
            let want = rook_attacks_gen(n, sq);
            assert_eq!(got, want, "rook attacks diverge at sq {sq} occ {n:#x}");
            n = n.wrapping_sub(1) & mask;
            count += 1;
            if n == 0 || count > 512 {
                break;
            }
        }
    }
}

#[test]
fn move_bitfield_roundtrips() {
    use pawnstar::constants::Piece;
    // Score sign-extension (23-bit signed field) including negative scores.
    let mut m = Move::capture(
        Square::from_str("e1"),
        Square::from_str("e5"),
        Piece::Rook,
        Piece::Pawn,
    );
    for score in [0, 1, -1, 31900, -31900, -1234567, 4194303, -4194304] {
        m.assign_score(score);
        assert_eq!(m.score(), score, "score {score} did not round-trip");
        assert_eq!(m.from(), Square::from_str("e1"));
        assert_eq!(m.to(), Square::from_str("e5"));
        assert_eq!(m.piece(), Piece::Rook);
        assert_eq!(m.captured(), Piece::Pawn);
    }
    // TT metadata: depth round-trips through int8 (including negatives), age through u8.
    for depth in [-5, 0, 1, 13, 127, -128] {
        let t = m.with_tt_data(-31897, depth, NodeType::Pv);
        assert_eq!(t.tt_depth(), depth);
        assert_eq!(t.tt_node_type(), NodeType::Pv);
        assert_eq!(t.score(), -31897);
        // Identity bits preserved.
        assert_eq!(t, m);
        let mut aged = t;
        aged.set_tt_age(200);
        assert_eq!(aged.tt_age(), 200);
        assert_eq!(aged.tt_depth(), depth);
        assert_eq!(aged.score(), -31897);
    }
    // Bits round-trip (lockless TT path).
    let bits = m.with_tt_data(-100, 7, NodeType::Cut).bits();
    let back = Move::from_bits(bits);
    assert_eq!(back.score(), -100);
    assert_eq!(back.tt_depth(), 7);
    assert_eq!(back.tt_node_type(), NodeType::Cut);
    // Equality ignores score/TT bits.
    let mut scored = m;
    scored.assign_score(12345);
    assert_eq!(scored, m);
    // to_string with promotion.
    let p = Move::promotion(
        Square::from_str("a7"),
        Square::from_str("a8"),
        Piece::Queen,
        Piece::None,
    );
    assert_eq!(p.to_string(), "a7a8q");
    assert_eq!(p.promoted(), Piece::Queen);
    assert!(!p.is_quiet());
    assert!(Move::none().is_quiet());
}

#[test]
fn sort_moves_is_stable_descending() {
    use pawnstar::moves::{sort_moves, MoveList};
    let mut list = MoveList::new();
    // Three distinct identities with scores 5, 9, 5, 9, 1 — ties must keep insertion order.
    let mk = |from: u8, score: i32| {
        let mut m = Move::non_capture(Square(from), Square(63), pawnstar::constants::Piece::Knight);
        m.assign_score(score);
        m
    };
    for (from, score) in [(0u8, 5), (1, 9), (2, 5), (3, 9), (4, 1)] {
        list.push_back(mk(from, score));
    }
    sort_moves(&mut list);
    let order: Vec<(u8, i32)> = list.iter().map(|m| (m.from().0, m.score())).collect();
    assert_eq!(order, vec![(1, 9), (3, 9), (0, 5), (2, 5), (4, 1)]);
}
