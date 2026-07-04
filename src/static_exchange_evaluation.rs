//! Static exchange evaluation (SEE) for capture move ordering and pruning.
//! (Port of `static_exchange_evaluation.h`.)

use crate::attacks::{bishop_attacks, rook_attacks};
use crate::bitboard::{Bitboard, NO_SQUARES, RANK_1, RANK_8};
use crate::constants::{enemy_of, Color, Piece};
use crate::generated_data::{KING_ATTACKS, KNIGHT_ATTACKS, PAWN_ATTACKS_BLACK, PAWN_ATTACKS_WHITE};
use crate::moves::{Move, MoveType};
use crate::position::Position;
use crate::square::Square;

/// Use nominal 1-3-3-5-9 material values for SEE.
pub const PIECE_VALUES: [i32; 7] = [0, 100, 300, 300, 500, 900, 10000];

/// Bitboards of pieces on the board used for static exchange evaluation.
pub struct SeeBoard {
    pub pieces: [Bitboard; 7], // Per-piece-type bitboards indexed by Piece (index 0 / None = occupancy).
    pub colors: [Bitboard; 2], // Per-color bitboards indexed by Color.
}

impl SeeBoard {
    /// Construct the working board from a position.
    #[inline(always)]
    pub fn new(p: &Position) -> SeeBoard {
        SeeBoard {
            pieces: p.pieces,
            colors: p.colors,
        }
    }
}

/// Determine the swap off value for a capture on a square.
/// `bb`: bitboards of piece locations; `location`: target square; `color`: color to move;
/// `piece_on_square`: piece currently standing on the target square.
pub fn evaluate_swap_off(
    bb: &mut SeeBoard,
    location: Square,
    mut color: Color,
    mut piece_on_square: Piece,
) -> i32 {
    // Squares a pawn of each color must stand on to attack location. A white pawn attacks
    // location from the squares a black pawn on location would attack, and vice versa. The
    // relevant mask depends on whose turn it is, so it must be reselected each ply as color
    // alternates (otherwise a recapturing pawn of the other color is missed).
    let white_pawn_attacks = PAWN_ATTACKS_BLACK[location.index()];
    let black_pawn_attacks = PAWN_ATTACKS_WHITE[location.index()];
    let knight_attacks = KNIGHT_ATTACKS[location.index()];
    let king_attacks = KING_ATTACKS[location.index()];
    let square = Bitboard::from_square(location);
    let mut occupied = bb.pieces[Piece::None.index()]; // pieces[None] holds the occupied-squares bitboard
    let mut scores = [0i32; 32];
    let mut ply: i32 = 0;
    // First pass: perform all the captures onto the square least valuable piece first.
    while ply != 32 {
        // Find the least valuable piece of color which directly attacks location.
        // (The C++ uses `goto FoundAttacker`; the labeled block is its structured equivalent.)
        let pawn_attacks = if color == Color::White {
            white_pawn_attacks
        } else {
            black_pawn_attacks
        };
        let attacking_pieces = bb.colors[color.index()];
        let (capturing_piece, mut attackers) = 'found: {
            let a = pawn_attacks & attacking_pieces & bb.pieces[Piece::Pawn.index()];
            if a.is_not_empty() {
                break 'found (Piece::Pawn, a);
            }
            let a = knight_attacks & attacking_pieces & bb.pieces[Piece::Knight.index()];
            if a.is_not_empty() {
                break 'found (Piece::Knight, a);
            }
            let bishop_att = bishop_attacks(occupied, location);
            let a = bishop_att & attacking_pieces & bb.pieces[Piece::Bishop.index()];
            if a.is_not_empty() {
                break 'found (Piece::Bishop, a);
            }
            let rook_att = rook_attacks(occupied, location);
            let a = rook_att & attacking_pieces & bb.pieces[Piece::Rook.index()];
            if a.is_not_empty() {
                break 'found (Piece::Rook, a);
            }
            let a = (bishop_att | rook_att) & attacking_pieces & bb.pieces[Piece::Queen.index()];
            if a.is_not_empty() {
                break 'found (Piece::Queen, a);
            }
            let a = king_attacks & attacking_pieces & bb.pieces[Piece::King.index()];
            if a.is_not_empty() {
                break 'found (Piece::King, a);
            }
            (Piece::None, NO_SQUARES)
        };
        if capturing_piece == Piece::None {
            // No more attackers - finished first pass.
            scores[ply as usize] = 0;
            break;
        }
        attackers.isolate_lsb();
        bb.pieces[piece_on_square.index()] ^= square;
        bb.colors[enemy_of(color).index()] ^= square;
        bb.pieces[capturing_piece.index()] ^= attackers | square;
        bb.colors[color.index()] ^= attackers | square;
        occupied ^= attackers;
        scores[ply as usize] = PIECE_VALUES[piece_on_square.index()];
        piece_on_square = capturing_piece;
        color = enemy_of(color);
        // Handle special case of pawn capture promotion
        if piece_on_square == Piece::Pawn && (square & (RANK_1 | RANK_8)).is_not_empty() {
            piece_on_square = Piece::Queen;
            bb.pieces[Piece::Pawn.index()] ^= square;
            bb.pieces[Piece::Queen.index()] ^= square;
        }
        ply += 1;
    }
    // Second pass: unwind the capture stack and propagate values back to the top for material
    // winning sequences.
    ply -= 1;
    while ply >= 0 {
        let p = ply as usize;
        scores[p] -= scores[p + 1];
        if scores[p] < 0 {
            scores[p] = 0; // Player would not initiate a losing capture sequence.
        }
        ply -= 1;
    }
    scores[0]
}

/// SeeBoard mirror of `Position::attacks_to`: the set of `color` pieces attacking `location`,
/// using the working board's bitboards and its occupancy. Kept byte-for-byte equivalent to
/// `Position::attacks_to` so the SEE check test matches `Position::checkers` exactly.
pub fn see_attackers_to(bb: &SeeBoard, location: Square, color: Color) -> Bitboard {
    let occupied = bb.pieces[Piece::None.index()];
    let mut result = (if color == Color::White {
        PAWN_ATTACKS_BLACK[location.index()]
    } else {
        PAWN_ATTACKS_WHITE[location.index()]
    }) & bb.pieces[Piece::Pawn.index()];
    result |= (KNIGHT_ATTACKS[location.index()] & bb.pieces[Piece::Knight.index()])
        | (KING_ATTACKS[location.index()] & bb.pieces[Piece::King.index()]);
    result |= rook_attacks(occupied, location)
        & (bb.pieces[Piece::Rook.index()] | bb.pieces[Piece::Queen.index()]);
    result |= bishop_attacks(occupied, location)
        & (bb.pieces[Piece::Bishop.index()] | bb.pieces[Piece::Queen.index()]);
    result &= bb.colors[color.index()];
    result
}

/// MakeMove-based SEE, kept only as the reference the cross-check test asserts the fast path
/// against. Returns {static exchange evaluation score, whether the move gives check}.
pub fn evaluate_static_exchange_reference(src_position: &Position, mv: Move) -> (i32, bool) {
    let dst_position = src_position.make_move(mv);
    let is_checking = dst_position.is_in_check();
    let mut bb = SeeBoard::new(&dst_position);
    let piece = src_position.squares[mv.from().index()];
    let captured = if mv.type_() == MoveType::EpCapture {
        Piece::Pawn
    } else {
        src_position.squares[mv.to().index()]
    };
    let promoted = mv.promoted();
    if promoted != Piece::None {
        return (
            PIECE_VALUES[captured.index()] + PIECE_VALUES[promoted.index()]
                - PIECE_VALUES[Piece::Pawn.index()]
                - evaluate_swap_off(&mut bb, mv.to(), dst_position.color_to_move, mv.promoted()),
            is_checking,
        );
    }
    (
        PIECE_VALUES[captured.index()]
            - evaluate_swap_off(&mut bb, mv.to(), dst_position.color_to_move, piece),
        is_checking,
    )
}

/// Compute the static exchange evaluation of a capture, resolving the full capture sequence.
///
/// SEE only needs the post-move piece/colour bitboards and occupancy, so rather than a full
/// Position make_move this applies just the capturing move to a lightweight SeeBoard. The check
/// flag is derived directly: a move gives check iff the side that just moved now attacks the
/// opposing king, which covers direct, discovered, promotion and en-passant-discovered checks
/// uniformly — the same definition Position uses for `checkers`. Bit-identical to
/// `evaluate_static_exchange_reference` for every capture/promotion (asserted by the SEE test's
/// cross-check); SEE is never called on castling.
/// Returns {static exchange evaluation score for this move, whether the move gives check}.
pub fn evaluate_static_exchange(src_position: &Position, mv: Move) -> (i32, bool) {
    let color = src_position.color_to_move;
    let enemy = enemy_of(color);
    let from = mv.from();
    let to = mv.to();
    let mover = src_position.squares[from.index()];
    let promoted = mv.promoted();
    let is_ep = mv.type_() == MoveType::EpCapture;
    // En-passant captures the pawn on `to`'s file at `from`'s rank; otherwise the captured piece
    // sits on `to` (None for a non-capturing promotion).
    let captured_square = if is_ep {
        Square::from_file_rank(to.file(), from.rank())
    } else {
        to
    };
    let captured = if is_ep {
        Piece::Pawn
    } else {
        src_position.squares[to.index()]
    };
    let piece_on_to = if promoted != Piece::None {
        promoted
    } else {
        mover
    };

    // Apply only the capturing move to a working board (no Zobrist/castling/squares recompute).
    let mut bb = SeeBoard::new(src_position);
    let from_bb = Bitboard::from_square(from);
    let to_bb = Bitboard::from_square(to);
    if captured != Piece::None {
        // a non-capturing promotion leaves `to` empty
        let captured_bb = Bitboard::from_square(captured_square);
        bb.pieces[captured.index()] ^= captured_bb;
        bb.colors[enemy.index()] ^= captured_bb;
    }
    bb.pieces[mover.index()] ^= from_bb; // lift the mover off `from`...
    bb.colors[color.index()] ^= from_bb;
    bb.pieces[piece_on_to.index()] ^= to_bb; // ...and place it (promoted type if promoting) on `to`
    bb.colors[color.index()] ^= to_bb;
    bb.pieces[Piece::None.index()] =
        bb.colors[Color::White.index()] | bb.colors[Color::Black.index()]; // refreshed occupancy

    // Check = the side that just moved now attacks the opposing king (matches Position::checkers).
    let enemy_king = (bb.pieces[Piece::King.index()] & bb.colors[enemy.index()]).lsb();
    let is_checking = see_attackers_to(&bb, enemy_king, color).is_not_empty();

    if promoted != Piece::None {
        return (
            PIECE_VALUES[captured.index()] + PIECE_VALUES[promoted.index()]
                - PIECE_VALUES[Piece::Pawn.index()]
                - evaluate_swap_off(&mut bb, to, enemy, promoted),
            is_checking,
        );
    }
    (
        PIECE_VALUES[captured.index()] - evaluate_swap_off(&mut bb, to, enemy, mover),
        is_checking,
    )
}
