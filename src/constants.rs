//! Program configuration constants and core enums. (Port of `constants.h`.)

/// Chess pieces.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[repr(u8)]
pub enum Piece {
    /// No piece / empty square.
    None = 0,
    /// Pawn.
    Pawn,
    /// Knight.
    Knight,
    /// Bishop.
    Bishop,
    /// Rook.
    Rook,
    /// Queen.
    Queen,
    /// King.
    King,
}

impl Piece {
    /// All real pieces, in enum order (used where C++ iterates `kPawn..=kKing`).
    pub const ALL: [Piece; 6] = [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
        Piece::King,
    ];

    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Lossy conversion from the 3-bit field packings (0..=6).
    #[inline(always)]
    pub const fn from_u8(v: u8) -> Piece {
        debug_assert!(v <= 6);
        // SAFETY: Piece is repr(u8) with contiguous discriminants 0..=6.
        unsafe { core::mem::transmute::<u8, Piece>(v) }
    }
}

/// Piece colors.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Color {
    /// White pieces / side.
    White = 0,
    /// Black pieces / side.
    Black,
}

impl Color {
    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[inline(always)]
    pub const fn from_u8(v: u8) -> Color {
        debug_assert!(v <= 1);
        // SAFETY: Color is repr(u8) with discriminants 0..=1.
        unsafe { core::mem::transmute::<u8, Color>(v) }
    }
}

/// Return the enemy of a color.
#[inline(always)]
pub const fn enemy_of(color: Color) -> Color {
    match color {
        Color::White => Color::Black,
        Color::Black => Color::White,
    }
}

pub const HASHTABLE_MEGABYTES: i32 = 64; // default transposition table size in MB
pub const EVAL_CACHE_MB: i32 = 8; // NNUE evaluation cache size in MB (lockless)
pub const MAX_MOVES_PER_POSITION: usize = 256; // maximum possible number of moves for a chess position
                                               // Score band (all comfortably within int16, so eval-cache packing and mate scores never overflow):
                                               //   |eval| <= MAX_EVALUATION (30000) < WIN_THRESHOLD (31000) < |mate| in [31836,31900] < BETA (32000) < 32767.
pub const BETA: i32 = 32000; // greater than any possible evaluation score
pub const ALPHA: i32 = -32000; // smaller than any possible evaluation score
pub const CHECKMATED_SCORE: i32 = -31900; // score for losing the game (mate scores = this + ply)
pub const WIN_THRESHOLD: i32 = 31000; // score threshold for winning at any ply (above any clamped eval)
pub const LOSE_THRESHOLD: i32 = -31000; // score threshold for losing at any ply
pub const MAX_EVALUATION: i32 = 30000; // NNUE eval clamp magnitude; keeps |eval| below WIN_THRESHOLD
pub const DRAW_SCORE: i32 = 0; // score for achieving a draw
pub const MAX_PLY: usize = 64; // terminate the search at this ply no matter what
pub const RANK_FLIP: u8 = 0x38; // used as an XOR mask to flip the board horizontally
pub const SEARCH_CANCELLED_SCORE: i32 = -12345678; // illegal value returned when search is cancelled
pub const SCORE_INSTABILITY_THRESHOLD: i32 = 50; // window of score variability
pub const START_DEPTH: i32 = 3; // depth to do full width search
pub const RFP_MAX_DEPTH: i32 = 7; // reverse-futility pruning: max depth to apply it
pub const RFP_MARGIN: i32 = 80; // reverse-futility pruning: centipawn margin per ply
pub const LMP_MAX_DEPTH: i32 = 8; // late-move (move-count) pruning: max depth to apply it
pub const LMP_BASE: i32 = 3; // quiet skipped once move index >= LMP_BASE + depth*depth
pub const NO_TT_REDUCE_MIN_DEPTH: i32 = 4; // min depth at which a node lacking a TT move is reduced
pub const MEGABYTE: usize = 1 << 20; // An old school megabyte ;-)
pub const MAX_SEARCH_THREADS: i32 = 256; // upper bound on Lazy SMP search threads
pub const CONT_KEYS: usize = 7 * 64; // Index by [piece, to] for continuation scores.
