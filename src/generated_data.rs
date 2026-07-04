//! Precomputed lookup tables (attack/ray bitboards, magic sliding-attack tables,
//! intervening-squares masks and Zobrist hashes) used by pawnstar for hashing, move generation
//! and attack detection. (Port of `generated_data.h`.)
//!
//! The C++ computes these once at program startup (dynamic initialisation of `inline const`
//! globals). Here the pure-array tables (rays, attacks, Zobrist) are `const` — evaluated at
//! compile time by the exact same algorithms — and the heap-backed magic tables are `LazyLock`
//! statics computed once at first use.

use std::sync::LazyLock;

use crate::bitboard::Bitboard;

/// Zobrist hash value type.
pub type ZobristT = u64;

/// Zobrist hash for black to move.
pub const BLACK_MOVE_HASH: ZobristT = 0x28AB74D640E50602;

/// A magic-bitboard entry in the sliding-piece move generator array, for one square of one piece.
/// Maps a masked occupancy to an attack set via a multiply-and-shift hash
/// (`hash = (occupancy * magic) >> shift`). Magic bitboards are portable (plain 64-bit multiply).
/// The `magic` values are precomputed constants (see BISHOP_MAGIC_NUMBERS / ROOK_MAGIC_NUMBERS);
/// the tables are filled once at first use from them.
///
/// The hash does not index the attack sets directly: it indexes `indices` (one byte per hash
/// slot) which in turn indexes `attacks` (the de-duplicated 8-byte attack sets). Many distinct
/// occupancies share an attack set, so the 1-byte indirection shrinks the tables ~5×. Fewer than
/// 256 distinct attack sets per square, so a u8 index suffices.
pub struct MagicBitboard {
    pub occupancy_mask: Bitboard, // Relevant-occupancy mask (excludes board edges / final square).
    pub magic: u64,               // Multiplier hashing a masked occupancy to a hash slot.
    pub shift: u32,               // Right-shift = 64 - popcount(mask); hash range is 1 << popcount.
    pub attacks: Vec<Bitboard>,   // De-duplicated attack sets (8 bytes each).
    pub indices: Vec<u8>,         // Per-hash-slot index into attacks (1 byte each).
}

// ---- Table-building helpers (the C++ `gendata_detail` namespace) ----

/// Compass-rose directions (north = forward for white).
#[derive(Clone, Copy)]
enum Direction {
    North,
    Northeast,
    East,
    Southeast,
    South,
    Southwest,
    West,
    Northwest,
}

const GD_FILE_A: u64 = 0x0101010101010101; // The a-file (LERF mapping).
const GD_FILE_H: u64 = GD_FILE_A << 7; // The h-file.

const fn file_of(sq: u8) -> u8 {
    sq & 7
}
const fn rank_of(sq: u8) -> u8 {
    sq >> 3
}
const fn sq_bb(sq: u8) -> u64 {
    1u64 << sq
}
const fn sq_bb_xy(x: i32, y: i32) -> u64 {
    1u64 << (x + 8 * y)
}
const fn is_in_board(x: i32, y: i32) -> bool {
    x == (x & 7) && y == (y & 7)
}
const fn shift_north(b: u64) -> u64 {
    b << 8
}
const fn shift_northeast(b: u64) -> u64 {
    (b & !GD_FILE_H) << 9
}
const fn shift_east(b: u64) -> u64 {
    (b & !GD_FILE_H) << 1
}
const fn shift_southeast(b: u64) -> u64 {
    (b & !GD_FILE_H) >> 7
}
const fn shift_south(b: u64) -> u64 {
    b >> 8
}
const fn shift_southwest(b: u64) -> u64 {
    (b & !GD_FILE_A) >> 9
}
const fn shift_west(b: u64) -> u64 {
    (b & !GD_FILE_A) >> 1
}
const fn shift_northwest(b: u64) -> u64 {
    (b & !GD_FILE_A) << 7
}

/// Apply the shift for a direction. (The C++ indexes a function-pointer table; const fns can't
/// call through pointers, so this is the equivalent direct dispatch.)
const fn shift(b: u64, direction: Direction) -> u64 {
    match direction {
        Direction::North => shift_north(b),
        Direction::Northeast => shift_northeast(b),
        Direction::East => shift_east(b),
        Direction::Southeast => shift_southeast(b),
        Direction::South => shift_south(b),
        Direction::Southwest => shift_southwest(b),
        Direction::West => shift_west(b),
        Direction::Northwest => shift_northwest(b),
    }
}

/// Ray from a square in one direction (excludes the source square).
const fn ray_from(sq: u8, direction: Direction) -> u64 {
    let mut result: u64 = 0;
    let mut b = shift(sq_bb(sq), direction);
    while b != 0 {
        result |= b;
        b = shift(b, direction);
    }
    result
}

const fn knight_attacks(sq: u8) -> u64 {
    const KNIGHT_VECTORS: [(i32, i32); 8] = [
        (-2, -1),
        (-2, 1),
        (-1, -2),
        (-1, 2),
        (1, -2),
        (1, 2),
        (2, -1),
        (2, 1),
    ];
    let mut result: u64 = 0;
    let mut i = 0;
    while i < 8 {
        let (dx, dy) = KNIGHT_VECTORS[i];
        let x = file_of(sq) as i32 + dx;
        let y = rank_of(sq) as i32 + dy;
        if is_in_board(x, y) {
            result |= sq_bb_xy(x, y);
        }
        i += 1;
    }
    result
}

const fn bishop_attacks_on_empty_board(sq: u8) -> u64 {
    ray_from(sq, Direction::Northeast)
        | ray_from(sq, Direction::Northwest)
        | ray_from(sq, Direction::Southeast)
        | ray_from(sq, Direction::Southwest)
}

const fn rook_attacks_on_empty_board(sq: u8) -> u64 {
    ray_from(sq, Direction::North)
        | ray_from(sq, Direction::South)
        | ray_from(sq, Direction::East)
        | ray_from(sq, Direction::West)
}

const fn king_attacks(sq: u8) -> u64 {
    let b = sq_bb(sq);
    shift_north(b)
        | shift_northeast(b)
        | shift_east(b)
        | shift_southeast(b)
        | shift_south(b)
        | shift_southwest(b)
        | shift_west(b)
        | shift_northwest(b)
}

/// Squares strictly between two colinear squares (0 if not colinear).
const fn intervening_squares(from: u8, to: u8) -> u64 {
    const DIRECTIONS: [Direction; 8] = [
        Direction::North,
        Direction::Northeast,
        Direction::East,
        Direction::Southeast,
        Direction::South,
        Direction::Southwest,
        Direction::West,
        Direction::Northwest,
    ];
    let mut d = 0;
    while d < 8 {
        let dir = DIRECTIONS[d];
        let ray = ray_from(from, dir);
        if ray & sq_bb(to) != 0 {
            return ray ^ ray_from(to, dir) ^ sq_bb(to);
        }
        d += 1;
    }
    0
}

/// Occupancy mask for one direction (excludes the final edge square, which never blocks).
const fn ray_occupancy_mask(sq: u8, direction: Direction) -> u64 {
    let mut result: u64 = 0;
    let mut last_square: u64 = 0;
    let mut b = shift(sq_bb(sq), direction);
    while b != 0 {
        result |= b;
        last_square = b;
        b = shift(b, direction);
    }
    result ^ last_square
}

pub fn bishop_occupancy_mask(sq: u8) -> u64 {
    ray_occupancy_mask(sq, Direction::Northeast)
        | ray_occupancy_mask(sq, Direction::Northwest)
        | ray_occupancy_mask(sq, Direction::Southeast)
        | ray_occupancy_mask(sq, Direction::Southwest)
}

pub fn rook_occupancy_mask(sq: u8) -> u64 {
    ray_occupancy_mask(sq, Direction::North)
        | ray_occupancy_mask(sq, Direction::South)
        | ray_occupancy_mask(sq, Direction::East)
        | ray_occupancy_mask(sq, Direction::West)
}

/// Sliding move targets in one direction, stopping at (and including) the first occupied square.
const fn ray_attacks(occupied_squares: u64, sq: u8, direction: Direction) -> u64 {
    let mut result: u64 = 0;
    let mut b = shift(sq_bb(sq), direction);
    while b != 0 {
        result |= b;
        if b & occupied_squares != 0 {
            break;
        }
        b = shift(b, direction);
    }
    result
}

/// Full bishop attack set for an occupancy (the reference generator behind the magic tables).
pub fn bishop_attacks_gen(occupied_squares: u64, sq: u8) -> u64 {
    ray_attacks(occupied_squares, sq, Direction::Northeast)
        | ray_attacks(occupied_squares, sq, Direction::Southeast)
        | ray_attacks(occupied_squares, sq, Direction::Southwest)
        | ray_attacks(occupied_squares, sq, Direction::Northwest)
}

/// Full rook attack set for an occupancy (the reference generator behind the magic tables).
pub fn rook_attacks_gen(occupied_squares: u64, sq: u8) -> u64 {
    ray_attacks(occupied_squares, sq, Direction::North)
        | ray_attacks(occupied_squares, sq, Direction::East)
        | ray_attacks(occupied_squares, sq, Direction::South)
        | ray_attacks(occupied_squares, sq, Direction::West)
}

/// Enumerate every subset of a bit mask (Hacker's Delight carry-rippler).
pub fn enumerate_mask_combinations(mask: u64) -> Vec<u64> {
    let mut result = Vec::new();
    let mut n: u64 = 0;
    loop {
        result.push(n);
        n = n.wrapping_sub(1) & mask;
        if n == 0 {
            break;
        }
    }
    result
}

/// Build a 64-entry Bitboard table from a per-square generator. (The C++ `MakeBitboards` takes a
/// function pointer; const fns can't call through pointers, so this expands the builder inline.)
macro_rules! make_bitboards_const {
    ($f:ident) => {{
        let mut table = [Bitboard(0); 64];
        let mut sq: u8 = 0;
        while sq != 64 {
            table[sq as usize] = Bitboard($f(sq));
            sq += 1;
        }
        table
    }};
}

/// Build one square's magic entry from a precomputed magic multiplier (no search — the magic is
/// a known-good constant). For each relevant-occupancy subset the magic hash
/// (`(occupancy * magic) >> shift`) selects a hash slot; the slot's attack set is de-duplicated
/// into `attacks` and the slot's 1-byte index into `indices`. The assert guards against a
/// destructive collision should the mask/attack generators ever change — regenerate the magics
/// with dump_magics if it fires.
pub fn build_magic_table(
    sq: u8,
    mask_fn: fn(u8) -> u64,
    attack_fn: fn(u64, u8) -> u64,
    magic: u64,
) -> MagicBitboard {
    let mask = mask_fn(sq);
    let bits = mask.count_ones();
    let shift = 64 - bits;

    let size = 1usize << bits; // number of hash slots = number of occupancy subsets
    let mut slot_attacks = vec![0u64; size]; // attack set per hash slot (0 = slot never produced)
    let mut filled = vec![0u8; size];
    for occupancy in enumerate_mask_combinations(mask) {
        let idx = (occupancy.wrapping_mul(magic) >> shift) as usize;
        let att = attack_fn(occupancy, sq);
        assert!(
            filled[idx] == 0 || slot_attacks[idx] == att,
            "precomputed magic collides — regenerate"
        );
        slot_attacks[idx] = att;
        filled[idx] = 1;
    }

    // De-duplicate the per-slot attack sets into attacks, recording each slot's 1-byte index.
    // Unfilled slots are never read at runtime, so they index attacks[0] arbitrarily.
    let mut attacks: Vec<Bitboard> = Vec::new();
    let mut indices = vec![0u8; size];
    let mut unique_index: std::collections::HashMap<u64, u8> = std::collections::HashMap::new();
    for idx in 0..size {
        if filled[idx] == 0 {
            continue;
        }
        let entry = *unique_index.entry(slot_attacks[idx]).or_insert_with(|| {
            assert!(
                attacks.len() < 256,
                "more than 256 distinct attack sets — u8 index too small"
            );
            attacks.push(Bitboard(slot_attacks[idx]));
            (attacks.len() - 1) as u8
        });
        indices[idx] = entry;
    }
    MagicBitboard {
        occupancy_mask: Bitboard(mask),
        magic,
        shift,
        attacks,
        indices,
    }
}

/// Build the 64-square magic table for a sliding piece from its precomputed magic constants.
fn make_magics(
    mask_fn: fn(u8) -> u64,
    attack_fn: fn(u64, u8) -> u64,
    magics: &[u64; 64],
) -> Vec<MagicBitboard> {
    (0u8..64)
        .map(|sq| build_magic_table(sq, mask_fn, attack_fn, magics[sq as usize]))
        .collect()
}

const fn make_intervening_squares() -> [[Bitboard; 64]; 64] {
    let mut table = [[Bitboard(0); 64]; 64];
    let mut from: u8 = 0;
    while from != 64 {
        let mut to: u8 = 0;
        while to != 64 {
            table[from as usize][to as usize] = Bitboard(intervening_squares(from, to));
            to += 1;
        }
        from += 1;
    }
    table
}

/// The Zobrist tables are filled from this single fixed-seed PRNG, drawn in the historical order
/// (piece-square, then castling rights, then en passant). A 64-bit xorshift* generator (rather
/// than a Mersenne Twister) so ports can reproduce the exact same sequence — and hence
/// bit-identical Zobrist hashes — from a trivially portable algorithm.
struct Xorshift64Star {
    state: u64,
}

impl Xorshift64Star {
    const fn next(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

/// Compute all three Zobrist tables from the shared fixed-seed PRNG, in the mandatory draw order.
/// (The C++ relies on dynamic-initialisation order of three globals; computing them in one const
/// function makes the order explicit and compile-time.)
const fn make_zobrist_tables() -> ([[[ZobristT; 64]; 6]; 2], [ZobristT; 16], [ZobristT; 64]) {
    let mut prng = Xorshift64Star {
        state: 0xAA55AA55AA55AA55,
    };

    // 1. Piece-square hashes: color 0..2, piece 0..6 (pawn..king), square 0..64 — one draw each.
    let mut piece_square = [[[0u64; 64]; 6]; 2];
    let mut color = 0;
    while color != 2 {
        let mut piece = 0;
        while piece != 6 {
            let mut sq = 0;
            while sq != 64 {
                piece_square[color][piece][sq] = prng.next();
                sq += 1;
            }
            piece += 1;
        }
        color += 1;
    }

    // 2. Castling-rights hashes: 16 draws.
    let mut castling = [0u64; 16];
    let mut i = 0;
    while i != 16 {
        castling[i] = prng.next();
        i += 1;
    }

    // 3. En-passant hashes: only ranks 2 and 5 draw from the PRNG; all other squares are 0.
    let mut en_passant = [0u64; 64];
    let mut sq: u8 = 0;
    while sq != 64 {
        let rank = rank_of(sq);
        en_passant[sq as usize] = if rank == 2 || rank == 5 {
            prng.next()
        } else {
            0
        };
        sq += 1;
    }

    (piece_square, castling, en_passant)
}

// ── Precomputed magic multipliers ────────────────────────────────────────────────────────────
// Known-good collision-free magics for the occupancy masks above, found once by an offline
// randomised search (see dump_magics for the generator) and baked in here so startup just fills
// the attack tables — no per-launch search.
#[rustfmt::skip]
pub const BISHOP_MAGIC_NUMBERS: [u64; 64] = [
    0x1850104108008810, 0x1060512c09024090, 0x4021010112000010, 0x0084440280112000,
    0x2004042000000840, 0xc00088200a238104, 0x0004040202109004, 0x9102002c06081404,
    0x4540101042080040, 0x1000900401104604, 0x022211480281000d, 0x0180310502008040,
    0x8400011040000812, 0x0200010c2a400a80, 0x00840b80b0282020, 0x0001009084112020,
    0x5106012008100104, 0x4002498802040420, 0xe810000800882408, 0x0041001804110010,
    0x4011001811400800, 0x2202002022100200, 0x0302100482100201, 0x0800402611008800,
    0x0402401090040881, 0x0482020048080810, 0x1042900818440020, 0x1a40104024004080,
    0x0685010004104000, 0x400400210500a011, 0x001410a0340c8400, 0x40040040008a1080,
    0x109004601050028a, 0x029208202442024c, 0x2122082400120800, 0x0010400a00002200,
    0x2240010200030048, 0x0021101080030814, 0x0008280480044200, 0x0002408a01850440,
    0x0008111090004880, 0x6100540420180420, 0x0210202028005000, 0x5800084200810801,
    0x000c404081201a00, 0x8020200041840140, 0x0092041804a00202, 0x806440a20a020340,
    0x0444110402222280, 0x0000862101200010, 0x1000004218040312, 0xa000080484040000,
    0x2141111002088000, 0x5000400508548100, 0x1020097000808001, 0x4005010802028860,
    0x00a0208808080281, 0x012000a09c0c2082, 0x1001020044040400, 0x4002100300208808,
    0x4040000010204840, 0x18001008b0018200, 0x0902425002008104, 0x5004204202082900,
];
#[rustfmt::skip]
pub const ROOK_MAGIC_NUMBERS: [u64; 64] = [
    0x0080081080204000, 0x8040002000100042, 0x008010008020000d, 0x0480043000804800,
    0x0480040002810800, 0x0200080102000410, 0x020001220000c804, 0x8200008204004133,
    0x0400800040008020, 0x8408804000200080, 0x1041806000803000, 0x101c800800900080,
    0x0002802400080080, 0x2805004300040008, 0x0002000801040200, 0x001a802080004100,
    0x8180014000600040, 0x0019010020804004, 0x8188420020108200, 0x2429210010030900,
    0x0198008004008008, 0x4089818004000200, 0x5000040008100102, 0x04000200005108a4,
    0x0144208280104000, 0x0080400040201000, 0x2001084100200010, 0x0000120200084220,
    0x0000040080080081, 0xa001000300080400, 0x28108a8c00100108, 0xc304010200008044,
    0x0000824001800020, 0x0830402000401008, 0x0320001041002100, 0x4050008010800800,
    0x8000800400800801, 0x0002000400808002, 0x1000c80104000210, 0x01c0804402000081,
    0x8002c00021828000, 0x0010002000414008, 0x0c01002000410010, 0x0002000840120020,
    0x4000040008008080, 0x0002000810020004, 0x2001020108240070, 0xa801000080410002,
    0x0500410c80220200, 0x30ca200482400080, 0x0d00820040182200, 0x0401000820100100,
    0x1480040080080080, 0x8002800400020080, 0x4040084210014400, 0x8019241100488a00,
    0x26034300201a8001, 0x0240001045082081, 0x0000200100400811, 0x0030002008041101,
    0x1021006800100417, 0x2006000144100822, 0x8200180081102a04, 0x00200a84012c4502,
];

// ── The precomputed tables. Pure-array tables are compile-time consts; the heap-backed magic
//    tables are computed once at first use. ─────────────────────────────────────────────────────

const fn make_east_west(sq: u8) -> u64 {
    ray_from(sq, Direction::East) | ray_from(sq, Direction::West)
}
const fn make_pawn_attacks_white(sq: u8) -> u64 {
    shift_northwest(sq_bb(sq)) | shift_northeast(sq_bb(sq))
}
const fn make_pawn_attacks_black(sq: u8) -> u64 {
    shift_southwest(sq_bb(sq)) | shift_southeast(sq_bb(sq))
}

pub const EAST_WEST: [Bitboard; 64] = make_bitboards_const!(make_east_west);
pub const PAWN_ATTACKS_WHITE: [Bitboard; 64] = make_bitboards_const!(make_pawn_attacks_white);
pub const PAWN_ATTACKS_BLACK: [Bitboard; 64] = make_bitboards_const!(make_pawn_attacks_black);
pub const KNIGHT_ATTACKS: [Bitboard; 64] = make_bitboards_const!(knight_attacks);
pub const BISHOP_ATTACKS: [Bitboard; 64] = make_bitboards_const!(bishop_attacks_on_empty_board);
pub const ROOK_ATTACKS: [Bitboard; 64] = make_bitboards_const!(rook_attacks_on_empty_board);
pub const KING_ATTACKS: [Bitboard; 64] = make_bitboards_const!(king_attacks);

pub static BISHOP_MAGICS: LazyLock<Vec<MagicBitboard>> = LazyLock::new(|| {
    make_magics(
        bishop_occupancy_mask,
        bishop_attacks_gen,
        &BISHOP_MAGIC_NUMBERS,
    )
});
pub static ROOK_MAGICS: LazyLock<Vec<MagicBitboard>> =
    LazyLock::new(|| make_magics(rook_occupancy_mask, rook_attacks_gen, &ROOK_MAGIC_NUMBERS));

pub const INTERVENING_SQUARES: [[Bitboard; 64]; 64] = make_intervening_squares();

// The Zobrist tables share one PRNG; make_zobrist_tables draws them in the mandatory order
// (piece-square, castling rights, en passant). Reordering would change every drawn key.
const ZOBRIST_TABLES: ([[[ZobristT; 64]; 6]; 2], [ZobristT; 16], [ZobristT; 64]) =
    make_zobrist_tables();
pub const PIECE_SQUARE_HASHES: [[[ZobristT; 64]; 6]; 2] = ZOBRIST_TABLES.0;
pub const CASTLING_RIGHTS_HASHES: [ZobristT; 16] = ZOBRIST_TABLES.1;
pub const EN_PASSANT_HASHES: [ZobristT; 64] = ZOBRIST_TABLES.2;
