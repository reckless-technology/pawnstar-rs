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
    ///
    /// Total on any input: the en-passant field of `Position::from_string` is untrusted GUI text, so
    /// a short or malformed name must not panic (`b[1]` on a one-character string) nor produce an
    /// off-board index (a garbage name used to underflow into a square >= 64, which then indexed one
    /// past the 64-entry en-passant hash table). Anything that is not a well-formed square yields
    /// `Square(0)` — the no-square sentinel, i.e. "no en-passant target", the same as a "-" field.
    #[inline(always)]
    pub const fn from_str(s: &str) -> Square {
        let b = s.as_bytes();
        if b.len() < 2 {
            return Square(0);
        }
        let file = (b[0] | 0x20).wrapping_sub(b'a');
        let rank = b[1].wrapping_sub(b'1');
        if file > 7 || rank > 7 {
            return Square(0);
        }
        Square(file + 8 * rank)
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
