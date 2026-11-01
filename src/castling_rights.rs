//! Castling-rights flags and their effect on hashing and move making. (Port of `castling_rights.h`.)

use crate::generated_data::{ZobristT, CASTLING_RIGHTS_HASHES};
use crate::moves::Move;

// Castling-rights bit flags, and the AND masks applied when a key square is vacated/entered.
const WHITE_KINGSIDE: u8 = 0x01; // White may castle kingside.
const WHITE_QUEENSIDE: u8 = 0x02; // White may castle queenside.
const BLACK_KINGSIDE: u8 = 0x04; // Black may castle kingside.
const BLACK_QUEENSIDE: u8 = 0x08; // Black may castle queenside.
const OK: u8 = !0; // Mask that preserves all rights.
const A1: u8 = !WHITE_QUEENSIDE; // Mask applied for the a1 square.
const E1: u8 = !(WHITE_KINGSIDE | WHITE_QUEENSIDE); // Mask applied for the e1 square.
const H1: u8 = !WHITE_KINGSIDE; // Mask applied for the h1 square.
const A8: u8 = !BLACK_QUEENSIDE; // Mask applied for the a8 square.
const E8: u8 = !(BLACK_KINGSIDE | BLACK_QUEENSIDE); // Mask applied for the e8 square.
const H8: u8 = !BLACK_KINGSIDE; // Mask applied for the h8 square.

/// Mask values ANDed with move from and to squares to determine new castling rights after a move.
/// Only squares A1, E1, H1, A8, E8, H8 affect castling rights.
#[rustfmt::skip]
const MOVE_MASKS: [u8; 64] = [
    A1, OK, OK, OK, E1, OK, OK, H1,
    OK, OK, OK, OK, OK, OK, OK, OK,
    OK, OK, OK, OK, OK, OK, OK, OK,
    OK, OK, OK, OK, OK, OK, OK, OK,
    OK, OK, OK, OK, OK, OK, OK, OK,
    OK, OK, OK, OK, OK, OK, OK, OK,
    OK, OK, OK, OK, OK, OK, OK, OK,
    A8, OK, OK, OK, E8, OK, OK, H8,
];

/// Encapsulates the four castling-rights flags (white/black × kingside/queenside).
/// The value is kept as a 4-bit field in a u8 so it can be used directly as an index
/// into the Zobrist castling-rights hash table (entries 0–15).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CastlingRights {
    value: u8, // The actual flag value.
}

impl CastlingRights {
    /// Default constructor; no castling rights.
    #[inline(always)]
    pub const fn new() -> CastlingRights {
        CastlingRights { value: 0 }
    }

    /// Construct from a raw 4-bit flag value.
    #[inline(always)]
    pub const fn from_raw(rights: u8) -> CastlingRights {
        CastlingRights { value: rights }
    }

    /// Whether white may still castle kingside.
    #[inline(always)]
    pub const fn may_white_castle_kingside(self) -> bool {
        self.value & WHITE_KINGSIDE != 0
    }

    /// Whether white may still castle queenside.
    #[inline(always)]
    pub const fn may_white_castle_queenside(self) -> bool {
        self.value & WHITE_QUEENSIDE != 0
    }

    /// Whether black may still castle kingside.
    #[inline(always)]
    pub const fn may_black_castle_kingside(self) -> bool {
        self.value & BLACK_KINGSIDE != 0
    }

    /// Whether black may still castle queenside.
    #[inline(always)]
    pub const fn may_black_castle_queenside(self) -> bool {
        self.value & BLACK_QUEENSIDE != 0
    }

    /// Zobrist hash associated with these castling rights.
    #[inline(always)]
    pub fn hash(self) -> ZobristT {
        CASTLING_RIGHTS_HASHES[self.value as usize]
    }

    /// Update to the castling rights that result after making a move.
    /// (The C++ returns `*this`; Rust callers just mutate in place.)
    #[inline(always)]
    pub fn after_move(&mut self, mv: Move) -> &mut CastlingRights {
        self.value &= MOVE_MASKS[mv.from().index()] & MOVE_MASKS[mv.to().index()];
        self
    }

    /// Drop every right whose king and rook are not on the squares castling with them needs.
    /// Within a game the rights carry that themselves - `after_move` revokes a right the moment its
    /// king's or rook's home square is vacated or entered - so this is for a FEN, the one input that
    /// can assert a right the board cannot support. `Position::from_fen_fields` applies it; see the
    /// castling block of `Position::gen_moves` for what the generator relies on (issue #30).
    #[inline(always)]
    pub fn keep_supported(
        &mut self,
        is_white_kingside_supported: bool,
        is_white_queenside_supported: bool,
        is_black_kingside_supported: bool,
        is_black_queenside_supported: bool,
    ) -> &mut CastlingRights {
        let mut supported: u8 = 0;
        if is_white_kingside_supported {
            supported |= WHITE_KINGSIDE;
        }
        if is_white_queenside_supported {
            supported |= WHITE_QUEENSIDE;
        }
        if is_black_kingside_supported {
            supported |= BLACK_KINGSIDE;
        }
        if is_black_queenside_supported {
            supported |= BLACK_QUEENSIDE;
        }
        self.value &= supported;
        self
    }

    /// Parse the castling field from a FEN string (e.g. "KQkq" or "-").
    pub fn from_fen(fen: &str) -> CastlingRights {
        let mut result: u8 = 0;
        if fen == "-" {
            return CastlingRights { value: result };
        }
        for c in fen.chars() {
            match c {
                'K' => result |= WHITE_KINGSIDE,
                'Q' => result |= WHITE_QUEENSIDE,
                'k' => result |= BLACK_KINGSIDE,
                'q' => result |= BLACK_QUEENSIDE,
                _ => {}
            }
        }
        CastlingRights { value: result }
    }

    /// Serialize castling rights to FEN format (e.g. "KQkq" or "-").
    pub fn to_fen_string(self) -> String {
        if self.value == 0 {
            return "-".to_string();
        }
        let mut s = String::new();
        if self.value & WHITE_KINGSIDE != 0 {
            s.push('K');
        }
        if self.value & WHITE_QUEENSIDE != 0 {
            s.push('Q');
        }
        if self.value & BLACK_KINGSIDE != 0 {
            s.push('k');
        }
        if self.value & BLACK_QUEENSIDE != 0 {
            s.push('q');
        }
        s
    }
}
