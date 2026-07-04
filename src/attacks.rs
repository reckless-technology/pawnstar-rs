//! Sliding piece bitboard attacks. Uses precomputed magic bitboards to look up sliding piece
//! attacks without loops or branches. (Port of `attacks.h`.)

use crate::bitboard::Bitboard;
use crate::constants::Piece;
use crate::generated_data::{BISHOP_MAGICS, KING_ATTACKS, KNIGHT_ATTACKS, ROOK_MAGICS};
use crate::square::Square;

// Sliding attacks use magic bitboards: a masked occupancy times a precomputed magic, shifted down
// to a hash slot (hash = (occupancy & mask) * magic >> shift); the slot's 1-byte `indices` entry
// then selects the de-duplicated attack set in `attacks`. This is a plain 64-bit multiply —
// portable and fast on every CPU.

/// Bishop sliding attacks: set of squares attacked by a bishop on `s` given `occupied_squares`.
#[inline(always)]
pub fn bishop_attacks(occupied_squares: Bitboard, s: Square) -> Bitboard {
    let m = &BISHOP_MAGICS[s.index()];
    m.attacks[m.indices
        [(((occupied_squares.0 & m.occupancy_mask.0).wrapping_mul(m.magic)) >> m.shift) as usize]
        as usize]
}

/// Rook sliding attacks: set of squares attacked by a rook on `s` given `occupied_squares`.
#[inline(always)]
pub fn rook_attacks(occupied_squares: Bitboard, s: Square) -> Bitboard {
    let m = &ROOK_MAGICS[s.index()];
    m.attacks[m.indices
        [(((occupied_squares.0 & m.occupancy_mask.0).wrapping_mul(m.magic)) >> m.shift) as usize]
        as usize]
}

/// Queen sliding attacks: set of squares attacked by a queen on `s` given `occupied_squares`.
#[inline(always)]
pub fn queen_attacks(occupied_squares: Bitboard, s: Square) -> Bitboard {
    bishop_attacks(occupied_squares, s) | rook_attacks(occupied_squares, s)
}

/// An attack-generation function taking occupancy and a square.
pub type AttackFn = fn(Bitboard, Square) -> Bitboard;

fn knight_attack_fn(_occupied: Bitboard, s: Square) -> Bitboard {
    KNIGHT_ATTACKS[s.index()]
}

fn king_attack_fn(_occupied: Bitboard, s: Square) -> Bitboard {
    KING_ATTACKS[s.index()]
}

/// Attack functions for every non-pawn piece type, including the king.
pub const PIECE_ATTACKERS: [(Piece, AttackFn); 5] = [
    (Piece::Knight, knight_attack_fn),
    (Piece::Bishop, bishop_attacks),
    (Piece::Rook, rook_attacks),
    (Piece::Queen, queen_attacks),
    (Piece::King, king_attack_fn),
];

/// Attack functions for every non-pawn piece type, excluding the king.
pub const PIECE_ATTACKERS_EXCEPT_KING: [(Piece, AttackFn); 4] = [
    (Piece::Knight, knight_attack_fn),
    (Piece::Bishop, bishop_attacks),
    (Piece::Rook, rook_attacks),
    (Piece::Queen, queen_attacks),
];
