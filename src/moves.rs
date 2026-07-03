//! Types and functions for using chess moves. (Port of `move.h`; named `moves.rs` because
//! `move` is a Rust keyword.)

use crate::constants::{Piece, MAX_MOVES_PER_POSITION};
use crate::square::Square;
use crate::stack_list::StackList;

/// Transposition-table node type, packed into a move's spare bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum NodeType {
    /// Beta cutoff.
    Cut = 0,
    /// No move exceeded alpha.
    All,
    /// Principal variation (rare).
    Pv,
}

impl NodeType {
    #[inline(always)]
    pub const fn from_u8(v: u8) -> NodeType {
        debug_assert!(v <= 2);
        // SAFETY: NodeType is repr(u8) with contiguous discriminants 0..=2.
        unsafe { core::mem::transmute::<u8, NodeType>(v) }
    }
}

/// Move types. (C++ `Move::Type`; the promotion variants alias the promoted piece's value.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum MoveType {
    /// Regular non capture move.
    NonCapture = 0,
    /// Capture move.
    Capture = 1,
    /// Promotion to knight. (= Piece::Knight)
    PromotionKnight = 2,
    /// Promotion to bishop. (= Piece::Bishop)
    PromotionBishop = 3,
    /// Promotion to rook. (= Piece::Rook)
    PromotionRook = 4,
    /// Promotion to queen. (= Piece::Queen)
    PromotionQueen = 5,
    /// Pawn double push (affects en passant).
    PawnDoublePush = 6,
    /// En passant capture.
    EpCapture = 7,
    /// Castling.
    Castling = 8,
}

impl MoveType {
    #[inline(always)]
    pub const fn from_u8(v: u8) -> MoveType {
        debug_assert!(v <= 8);
        // SAFETY: MoveType is repr(u8) with contiguous discriminants 0..=8.
        unsafe { core::mem::transmute::<u8, MoveType>(v) }
    }
}

// Packed-field layout (see the struct comment). Each field has a shift (its lowest bit) and,
// where it is read back out, a width mask.
const TO_SHIFT: i32 = 0; // bits 0-5:   to square
const FROM_SHIFT: i32 = 6; // bits 6-11:  from square
const PIECE_SHIFT: i32 = 12; // bits 12-14: moving piece
const CAPTURED_SHIFT: i32 = 15; // bits 15-17: captured piece
const TYPE_SHIFT: i32 = 18; // bits 18-21: move type
const CHECKING_SHIFT: i32 = 22; // bit 22:     gives-check flag
const NODE_TYPE_SHIFT: i32 = 23; // bits 23-24: TT node type
const DEPTH_SHIFT: i32 = 25; // bits 25-32: TT depth searched
const AGE_SHIFT: i32 = 33; // bits 33-40: TT age / generation
const SCORE_SHIFT: i32 = 41; // bits 41-63: score

const SQUARE_MASK: i64 = 0x3F; // 6-bit to / from square field
const PIECE_MASK: i64 = 0x07; // 3-bit piece field (moving / captured)
const TYPE_MASK: i64 = 0x0F; // 4-bit move-type field
const NODE_TYPE_MASK: i64 = 0x03; // 2-bit TT node-type field
const BYTE_MASK: i64 = 0xFF; // 8-bit TT depth / age field
const FROM_TO_MASK: i64 = 0xFFF; // bits 0-11:  from+to (history-table index)
const IDENTITY_MASK: i64 = 0x3FFFFF; // bits 0-21:  to/from/piece/captured/type (move identity)

const IS_CHECKING: i64 = 1i64 << CHECKING_SHIFT; // bit 22 mask
const AGE_MASK: i64 = BYTE_MASK << AGE_SHIFT; // bits 33-40
const SCORE_MASK: i64 = !((1i64 << SCORE_SHIFT) - 1); // bits 41-63
const TT_DATA_MASK: i64 = !((1i64 << NODE_TYPE_SHIFT) - 1); // bits 23-63

/// A chess move, 64 bits in size.
/// Bits are as follows:
/// ```text
///   Bits  | Meaning
/// --------|--------
///  0 -  5 | to square
///  6 - 11 | from square
/// 12 - 14 | moving piece
/// 15 - 17 | captured piece (in the case of capture moves)
/// 18 - 21 | move type
/// 22 - 22 | is checking flag (move gives check)
/// 23 - 24 | transposition node type (cut, all or pv)
/// 25 - 32 | transposition depth searched (int8_t)
/// 33 - 40 | transposition age / generation (uint8_t)
/// 41 - 63 | score for this move (signed, 23-bit); used for move ordering and as the stored TT score
/// ```
/// `Default` yields `Move::none()` (the C++ default constructor deliberately leaves the value
/// uninitialized; the port zero-initializes instead — every C++ code path that reads a
/// default-constructed move first assigns it).
#[derive(Clone, Copy, Default, Debug)]
pub struct Move {
    val: i64,
}

/// Equality compares from, to, piece, captured, type fields (22 identity bits) only,
/// exactly like the C++ `operator==`.
impl PartialEq for Move {
    #[inline(always)]
    fn eq(&self, that: &Move) -> bool {
        (self.val & IDENTITY_MASK) == (that.val & IDENTITY_MASK)
    }
}
impl Eq for Move {}

/// Used for putting moves in hashed containers (the C++ `Move::operator()` hasher hashed the raw
/// value; identity-masked here so Hash stays consistent with the identity-bit `PartialEq`).
impl core::hash::Hash for Move {
    #[inline(always)]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        (self.val & IDENTITY_MASK).hash(state);
    }
}

impl Move {
    /// Destination square.
    #[inline(always)]
    pub const fn to(self) -> Square {
        Square((self.val & SQUARE_MASK) as u8)
    }

    /// Source square.
    #[inline(always)]
    pub const fn from(self) -> Square {
        Square(((self.val >> FROM_SHIFT) & SQUARE_MASK) as u8)
    }

    /// Move type.
    #[inline(always)]
    pub const fn type_(self) -> MoveType {
        MoveType::from_u8(((self.val >> TYPE_SHIFT) & TYPE_MASK) as u8)
    }

    /// Moving piece.
    #[inline(always)]
    pub const fn piece(self) -> Piece {
        Piece::from_u8(((self.val >> PIECE_SHIFT) & PIECE_MASK) as u8)
    }

    /// The captured piece or Piece::None if not a capture move.
    #[inline(always)]
    pub const fn captured(self) -> Piece {
        Piece::from_u8(((self.val >> CAPTURED_SHIFT) & PIECE_MASK) as u8)
    }

    /// In the case of a pawn promotion, the piece to be promoted.
    #[inline(always)]
    pub const fn promoted(self) -> Piece {
        match self.type_() {
            MoveType::NonCapture
            | MoveType::Capture
            | MoveType::PawnDoublePush
            | MoveType::EpCapture
            | MoveType::Castling => Piece::None,
            MoveType::PromotionKnight => Piece::Knight,
            MoveType::PromotionBishop => Piece::Bishop,
            MoveType::PromotionRook => Piece::Rook,
            MoveType::PromotionQueen => Piece::Queen,
        }
    }

    /// The move score (bits 41-63, signed 23-bit). Used for move ordering and as the stored TT
    /// score. The arithmetic right shift sign-extends from the top bit.
    #[inline(always)]
    pub const fn score(self) -> i32 {
        (self.val >> SCORE_SHIFT) as i32
    }

    /// Transposition node type (bits 23-24).
    #[inline(always)]
    pub const fn tt_node_type(self) -> NodeType {
        NodeType::from_u8(((self.val >> NODE_TYPE_SHIFT) & NODE_TYPE_MASK) as u8)
    }

    /// Transposition depth searched (bits 25-32, int8_t).
    #[inline(always)]
    pub const fn tt_depth(self) -> i32 {
        ((self.val >> DEPTH_SHIFT) & BYTE_MASK) as u8 as i8 as i32
    }

    /// Transposition age / generation (bits 33-40, uint8_t).
    #[inline(always)]
    pub const fn tt_age(self) -> u8 {
        ((self.val >> AGE_SHIFT) & BYTE_MASK) as u8
    }

    /// Create a from-to bits value for indexing into history tables (12 bits).
    #[inline(always)]
    pub const fn from_to(self) -> i32 {
        (self.val & FROM_TO_MASK) as i32
    }

    /// Create a 9 bit key from the piece and the destination square: `[piece << 6 | to]`.
    #[inline(always)]
    pub const fn piece_to(self) -> i32 {
        ((self.val & SQUARE_MASK) | ((self.val >> FROM_SHIFT) & (PIECE_MASK << FROM_SHIFT))) as i32
    }

    /// Does this move give check?
    #[inline(always)]
    pub const fn is_checking(self) -> bool {
        (self.val & IS_CHECKING) != 0
    }

    /// Whether this is a quiet move (neither a capture nor a promotion).
    #[inline(always)]
    pub const fn is_quiet(self) -> bool {
        matches!(self.captured(), Piece::None) && matches!(self.promoted(), Piece::None)
    }

    /// Raw 64-bit representation of the move (all packed fields). Used by the lockless TT.
    #[inline(always)]
    pub const fn bits(self) -> u64 {
        self.val as u64
    }

    /// Construct a move from its raw 64-bit representation. Used by the lockless TT.
    #[inline(always)]
    pub const fn from_bits(bits: u64) -> Move {
        Move { val: bits as i64 }
    }

    /// Assign the score to the move (bits 41-63, signed 23-bit). Leaves all other bits untouched.
    #[inline(always)]
    pub fn assign_score(&mut self, score: i32) {
        self.val = (self.val & !SCORE_MASK) | ((score as i64) << SCORE_SHIFT);
    }

    /// Return a copy of this move with transposition metadata packed in (clearing any prior
    /// metadata / sort score in bits 23-63). Age is set to 0; the table stamps it on store.
    #[inline(always)]
    pub const fn with_tt_data(self, score: i32, depth: i32, type_: NodeType) -> Move {
        let mut v = self.val & !TT_DATA_MASK;
        v |= (type_ as u8 as i64) << NODE_TYPE_SHIFT;
        v |= (depth as i8 as u8 as i64) << DEPTH_SHIFT;
        v |= (score as i64) << SCORE_SHIFT;
        Move { val: v }
    }

    /// Stamp the transposition age / generation (bits 33-40), leaving all other bits untouched.
    #[inline(always)]
    pub fn set_tt_age(&mut self, age: u8) {
        self.val = (self.val & !AGE_MASK) | ((age as i64) << AGE_SHIFT);
    }

    /// Set the flag that indicates this move gives check.
    #[inline(always)]
    pub fn gives_check(&mut self) {
        self.val |= IS_CHECKING;
    }

    /// Special sentinel move that indicates not a move.
    #[inline(always)]
    pub const fn none() -> Move {
        Move { val: 0 }
    }

    /// Create a non capture move.
    #[inline(always)]
    pub const fn non_capture(from: Square, to: Square, piece: Piece) -> Move {
        Move::new(from, to, piece)
    }

    /// Create a capture move.
    #[inline(always)]
    pub const fn capture(from: Square, to: Square, piece: Piece, captured: Piece) -> Move {
        Move::new_typed_capture(from, to, piece, MoveType::Capture, captured)
    }

    /// Create a pawn promotion move. Pass `Piece::None` as `captured` for a non-capture
    /// promotion (bit-identical to the C++ 3-argument `Promotion` overload).
    #[inline(always)]
    pub const fn promotion(from: Square, to: Square, promoted: Piece, captured: Piece) -> Move {
        Move::new_typed_capture(from, to, Piece::Pawn, MoveType::from_u8(promoted as u8), captured)
    }

    /// Create a castling move (king from/to squares).
    #[inline(always)]
    pub const fn castling(from: Square, to: Square) -> Move {
        Move::new_typed(from, to, Piece::King, MoveType::Castling)
    }

    /// Create an en passant capture move (capturing pawn from/to squares).
    #[inline(always)]
    pub const fn ep_capture(from: Square, to: Square) -> Move {
        Move::new_typed_capture(from, to, Piece::Pawn, MoveType::EpCapture, Piece::Pawn)
    }

    /// Create a pawn double push move.
    #[inline(always)]
    pub const fn double_push(from: Square, to: Square) -> Move {
        Move::new_typed(from, to, Piece::Pawn, MoveType::PawnDoublePush)
    }

    /// Convert a move to an algebraic notation string, e.g. "e1g1", "a7a8q".
    pub fn to_string(self) -> String {
        let mut result = self.from().to_string() + &self.to().to_string();
        if !matches!(self.promoted(), Piece::None) {
            result.push(b" pnbrqk"[self.promoted() as usize] as char);
        }
        result
    }

    /// Construct a non-capture move of a piece.
    #[inline(always)]
    const fn new(from: Square, to: Square, piece: Piece) -> Move {
        Move {
            val: to.0 as i64 | ((from.0 as i64) << FROM_SHIFT) | ((piece as u8 as i64) << PIECE_SHIFT),
        }
    }

    /// Construct a typed move (no captured piece).
    #[inline(always)]
    const fn new_typed(from: Square, to: Square, piece: Piece, type_: MoveType) -> Move {
        Move {
            val: to.0 as i64
                | ((from.0 as i64) << FROM_SHIFT)
                | ((piece as u8 as i64) << PIECE_SHIFT)
                | ((type_ as u8 as i64) << TYPE_SHIFT),
        }
    }

    /// Construct a typed capture move.
    #[inline(always)]
    const fn new_typed_capture(from: Square, to: Square, piece: Piece, type_: MoveType, captured: Piece) -> Move {
        Move {
            val: to.0 as i64
                | ((from.0 as i64) << FROM_SHIFT)
                | ((piece as u8 as i64) << PIECE_SHIFT)
                | ((captured as u8 as i64) << CAPTURED_SHIFT)
                | ((type_ as u8 as i64) << TYPE_SHIFT),
        }
    }
}

const _: () = assert!(core::mem::size_of::<Move>() == core::mem::size_of::<u64>());

/// A list of moves, typically used to hold the list of legal moves available from a given
/// position. A StackList is much faster than a heap vector, and more convenient than a raw array.
pub type MoveList = StackList<Move, MAX_MOVES_PER_POSITION>;

/// Sort moves best first (descending score) with a stable, in-place insertion sort.
///
/// Stable and fully deterministic: equal-score moves keep their original generation order (the
/// inner loop shifts an element left only past STRICTLY smaller scores, so an equal score is
/// never jumped), so the search tree — and the `bench` node count — is identical across ports.
/// This is the exact insertion sort of the C++ original (which replaced `std::sort` precisely
/// because unstable tie-breaks diverged the bench node count between standard libraries);
/// do NOT replace it with `sort_unstable_by`.
pub fn sort_moves(moves: &mut MoveList) {
    let base = moves.as_mut_slice();
    let element_count = base.len();
    for i in 1..element_count {
        let key = base[i];
        let mut j = i as isize - 1;
        // Shift scores STRICTLY smaller than key's one place right, stopping at the first score
        // >= it, so equal scores keep their generation order (stable). Descending: larger scores
        // sort to the front.
        while j >= 0 && base[j as usize].score() < key.score() {
            base[(j + 1) as usize] = base[j as usize];
            j -= 1;
        }
        base[(j + 1) as usize] = key;
    }
}
