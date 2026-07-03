//! Types, values and functions for using chess Bitboards. (Port of `bitboard.h`.)

use crate::square::Square;

/// A set-wise interpretation of a 64-bit unsigned integer, with each bit mapping to a square on
/// the chessboard (LERF: bit 0 = a1, bit 7 = h1, bit 56 = a8, bit 63 = h8).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Bitboard(pub u64);

const NOT_FILE_A: u64 = 0xFEFEFEFEFEFEFEFE; // Mask off the a file.
const NOT_FILE_H: u64 = 0x7F7F7F7F7F7F7F7F; // Mask off the h file.

impl Bitboard {
    /// Construct from a square index. (C++ `Bitboard(Square)`.)
    #[inline(always)]
    pub const fn from_square(location: Square) -> Bitboard {
        Bitboard(1u64 << location.0)
    }

    /// Construct from file and rank. (C++ `Bitboard(int x, int y)`.)
    #[inline(always)]
    pub const fn from_file_rank(x: u8, y: u8) -> Bitboard {
        Bitboard(1u64 << (x + 8 * y))
    }

    /// Convert Bitboard to 8 x 8 string for debug purposes.
    pub fn to_string(self) -> String {
        let mut result = String::new();
        for y in (0..8).rev() {
            for x in 0..8 {
                result.push(if self.0 & (1u64 << (x + 8 * y)) != 0 { '1' } else { '0' });
            }
            result.push('\n');
        }
        result
    }

    /// Clear all but the LSB, i.e. make unique square.
    #[inline(always)]
    pub fn isolate_lsb(&mut self) {
        self.0 &= self.0.wrapping_neg();
    }

    /// Square index of the least significant set bit. (64 on an empty board, as in C++.)
    #[inline(always)]
    pub const fn lsb(self) -> Square {
        Square(self.0.trailing_zeros() as u8)
    }

    /// Square index of the most significant set bit. (Wraps to 255 on an empty board, as in C++.)
    #[inline(always)]
    pub const fn msb(self) -> Square {
        Square((63i32 - self.0.leading_zeros() as i32) as u8)
    }

    /// Number of bits set.
    #[inline(always)]
    pub const fn pop_count(self) -> i32 {
        self.0.count_ones() as i32
    }

    /// true if no bits are set.
    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// true if at least 1 bit is set.
    #[inline(always)]
    pub const fn is_not_empty(self) -> bool {
        self.0 != 0
    }

    /// Shift one square to the North.
    #[inline(always)]
    pub const fn shift_north(self) -> Bitboard {
        Bitboard(self.0 << 8)
    }

    /// Shift one square to the Northeast.
    #[inline(always)]
    pub const fn shift_northeast(self) -> Bitboard {
        Bitboard((self.0 & NOT_FILE_H) << 9)
    }

    /// Shift one square to the East.
    #[inline(always)]
    pub const fn shift_east(self) -> Bitboard {
        Bitboard((self.0 & NOT_FILE_H) << 1)
    }

    /// Shift one square to the Southeast.
    #[inline(always)]
    pub const fn shift_southeast(self) -> Bitboard {
        Bitboard((self.0 & NOT_FILE_H) >> 7)
    }

    /// Shift one square to the South.
    #[inline(always)]
    pub const fn shift_south(self) -> Bitboard {
        Bitboard(self.0 >> 8)
    }

    /// Shift one square to the Southwest.
    #[inline(always)]
    pub const fn shift_southwest(self) -> Bitboard {
        Bitboard((self.0 & NOT_FILE_A) >> 9)
    }

    /// Shift one square to the West.
    #[inline(always)]
    pub const fn shift_west(self) -> Bitboard {
        Bitboard((self.0 & NOT_FILE_A) >> 1)
    }

    /// Shift one square to the Northwest.
    #[inline(always)]
    pub const fn shift_northwest(self) -> Bitboard {
        Bitboard((self.0 & NOT_FILE_A) << 7)
    }
}

impl core::ops::BitAnd for Bitboard {
    type Output = Bitboard;
    #[inline(always)]
    fn bitand(self, that: Bitboard) -> Bitboard {
        Bitboard(self.0 & that.0)
    }
}

impl core::ops::BitOr for Bitboard {
    type Output = Bitboard;
    #[inline(always)]
    fn bitor(self, that: Bitboard) -> Bitboard {
        Bitboard(self.0 | that.0)
    }
}

impl core::ops::BitXor for Bitboard {
    type Output = Bitboard;
    #[inline(always)]
    fn bitxor(self, that: Bitboard) -> Bitboard {
        Bitboard(self.0 ^ that.0)
    }
}

impl core::ops::Not for Bitboard {
    type Output = Bitboard;
    #[inline(always)]
    fn not(self) -> Bitboard {
        Bitboard(!self.0)
    }
}

impl core::ops::BitAndAssign for Bitboard {
    #[inline(always)]
    fn bitand_assign(&mut self, that: Bitboard) {
        self.0 &= that.0;
    }
}

impl core::ops::BitOrAssign for Bitboard {
    #[inline(always)]
    fn bitor_assign(&mut self, that: Bitboard) {
        self.0 |= that.0;
    }
}

impl core::ops::BitXorAssign for Bitboard {
    #[inline(always)]
    fn bitxor_assign(&mut self, that: Bitboard) {
        self.0 ^= that.0;
    }
}

/// Iterator over the squares in a Bitboard: yields the LSB square, then clears it —
/// the C++ "for (Square s : b)" semantics.
pub struct BitboardIterator {
    i: u64, // Remaining set bits still to be iterated over.
}

impl Iterator for BitboardIterator {
    type Item = Square;
    #[inline(always)]
    fn next(&mut self) -> Option<Square> {
        if self.i == 0 {
            return None;
        }
        let sq = Square(self.i.trailing_zeros() as u8);
        self.i &= self.i - 1;
        Some(sq)
    }
}

impl IntoIterator for Bitboard {
    type Item = Square;
    type IntoIter = BitboardIterator;
    #[inline(always)]
    fn into_iter(self) -> BitboardIterator {
        BitboardIterator { i: self.0 }
    }
}

// Useful Bitboard constant values.
pub const NO_SQUARES: Bitboard = Bitboard(0); // Empty board (no squares set).
pub const ALL_SQUARES: Bitboard = Bitboard(!0); // Every square set.
pub const RANK_1: Bitboard = Bitboard(0xFF); // All squares on rank 1.
pub const RANK_2: Bitboard = RANK_1.shift_north(); // All squares on rank 2.
pub const RANK_3: Bitboard = RANK_2.shift_north(); // All squares on rank 3.
pub const RANK_4: Bitboard = RANK_3.shift_north(); // All squares on rank 4.
pub const RANK_5: Bitboard = RANK_4.shift_north(); // All squares on rank 5.
pub const RANK_6: Bitboard = RANK_5.shift_north(); // All squares on rank 6.
pub const RANK_7: Bitboard = RANK_6.shift_north(); // All squares on rank 7.
pub const RANK_8: Bitboard = RANK_7.shift_north(); // All squares on rank 8.
pub const FILE_A: Bitboard = Bitboard(0x0101010101010101); // All squares on the a file.
pub const FILE_B: Bitboard = FILE_A.shift_east(); // All squares on the b file.
pub const FILE_C: Bitboard = FILE_B.shift_east(); // All squares on the c file.
pub const FILE_D: Bitboard = FILE_C.shift_east(); // All squares on the d file.
pub const FILE_E: Bitboard = FILE_D.shift_east(); // All squares on the e file.
pub const FILE_F: Bitboard = FILE_E.shift_east(); // All squares on the f file.
pub const FILE_G: Bitboard = FILE_F.shift_east(); // All squares on the g file.
pub const FILE_H: Bitboard = FILE_G.shift_east(); // All squares on the h file.
pub const WHITE_SQUARES: Bitboard = Bitboard(0x55AA55AA55AA55AA); // All light squares.
pub const BLACK_SQUARES: Bitboard = Bitboard(!0x55AA55AA55AA55AAu64); // All dark squares.
