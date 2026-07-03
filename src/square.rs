//! Square indices on the chess board. (Port of `square.h`.)

/// A square index (location) on a chess board, in LERF format.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Square(pub u8);

impl Square {
    /// Construct from a LERF index. (C++ `Square(int x)`.)
    #[inline(always)]
    pub const fn new(x: u8) -> Square {
        Square(x)
    }

    /// Construct from file and rank. (C++ `Square(int x, int y)`.)
    #[inline(always)]
    pub const fn from_file_rank(x: u8, y: u8) -> Square {
        Square(x + 8 * y)
    }

    /// Construct from a square name, e.g. "e4". (C++ `Square(const char *str)`.)
    #[inline(always)]
    pub const fn from_str(s: &str) -> Square {
        let b = s.as_bytes();
        Square((b[0] | 0x20) - b'a' + 8 * (b[1] - b'1'))
    }

    /// File on board (0 thru 7 -> a thru h).
    #[inline(always)]
    pub const fn file(self) -> u8 {
        self.0 & 7
    }

    /// Rank on board (zero indexed 0 thru 7).
    #[inline(always)]
    pub const fn rank(self) -> u8 {
        self.0 >> 3
    }

    /// Array-index convenience (the C++ implicit `operator uint8_t`).
    #[inline(always)]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Square name, e.g. "e4". (C++ `ToString`.)
    pub fn to_string(self) -> String {
        let mut s = String::with_capacity(2);
        s.push((b'a' + self.file()) as char);
        s.push((b'1' + self.rank()) as char);
        s
    }
}

impl From<u8> for Square {
    #[inline(always)]
    fn from(v: u8) -> Square {
        Square(v)
    }
}

impl From<Square> for u8 {
    #[inline(always)]
    fn from(s: Square) -> u8 {
        s.0
    }
}
