//! Absolute-pin detection and the resulting legal move masks. (Port of `pins.h`.)

use crate::bitboard::{Bitboard, ALL_SQUARES, NO_SQUARES};
use crate::constants::Piece;
use crate::generated_data::{BISHOP_ATTACKS, INTERVENING_SQUARES, ROOK_ATTACKS};
use crate::position::Position;
use crate::square::Square;

/// Pinned pieces on the board and the squares to which they are allowed to move.
pub struct Pins {
    pinned_pieces: Bitboard,        // Set of squares which are pinned.
    allowed_squares: [Bitboard; 64], // Allowed squares per source.
}

impl Pins {
    /// Analyze a position for absolute pins.
    pub fn new(position: &Position) -> Pins {
        let mut pins = Pins {
            pinned_pieces: NO_SQUARES,
            allowed_squares: [NO_SQUARES; 64],
        };
        let color = position.color_to_move;
        let occupied_squares = position.occupied_squares();
        let friendly_pieces = position.colors[color.index()];
        let king_location = position.king_location[color.index()];
        let intervening = &INTERVENING_SQUARES[king_location.index()];
        let enemy_sliding_pieces = ((BISHOP_ATTACKS[king_location.index()]
            & (position.pieces[Piece::Bishop.index()] | position.pieces[Piece::Queen.index()]))
            | (ROOK_ATTACKS[king_location.index()]
                & (position.pieces[Piece::Rook.index()] | position.pieces[Piece::Queen.index()])))
            & !friendly_pieces;
        for s in enemy_sliding_pieces {
            let intervening_squares = intervening[s.index()];
            let intervening_pieces = intervening_squares & occupied_squares;
            let intervening_friendly_piece = intervening_pieces & friendly_pieces;
            if intervening_friendly_piece.is_not_empty() && intervening_pieces.pop_count() == 1 {
                // There is only a single piece between the sliding enemy attacker and the king so
                // this piece is pinned along the attack ray. The pinned piece is also allowed to
                // capture the pinning piece.
                pins.pinned_pieces |= intervening_friendly_piece;
                pins.allowed_squares[intervening_friendly_piece.lsb().index()] =
                    intervening_squares | Bitboard::from_square(s);
            }
        }
        pins
    }

    /// Return the allowable squares which a piece may move to considering pins.
    #[inline(always)]
    pub fn allowed_squares(&self, s: Square) -> Bitboard {
        if (Bitboard::from_square(s) & self.pinned_pieces).is_not_empty() {
            self.allowed_squares[s.index()]
        } else {
            ALL_SQUARES
        }
    }

    /// Determine if a from-to square pair is allowed based on whether the piece is pinned.
    #[inline(always)]
    pub fn is_allowed(&self, from: Square, to: Square) -> bool {
        (self.allowed_squares(from) & Bitboard::from_square(to)).is_not_empty()
    }
}
