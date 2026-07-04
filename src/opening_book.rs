//! Opening book storage and move selection. (Port of `opening_book.h`.)

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use crate::generated_data::ZobristT;
use crate::moves::Move;
use crate::position::Position;

/// PRNG used to randomly select among book moves. The C++ uses a wall-clock-seeded
/// `std::mt19937`; selection randomness affects no test or parity gate (the book is disabled for
/// bench / fixed-depth testing), so this port substitutes a small xorshift* generator with the
/// same wall-clock seeding.
struct BookPrng {
    state: u64,
}

impl BookPrng {
    fn new() -> BookPrng {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        BookPrng {
            state: seed | 1, // never zero
        }
    }

    fn next(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

/// The opening book is stored as a map, indexed by the Zobrist hash of the position, containing a
/// vector of Moves which are available from that position. The same move may be in the moves
/// vector multiple times if it appears in several lines of play, i.e. the frequency of occurrence
/// of a move in the list is proportional to the likelihood that we want to play it.
///
/// A move written with a trailing '?' suffix in the book file (e.g. "f2f4?") is *questionable*:
/// it is a legal move that the engine recognises and replays so that the rest of the line is
/// still parsed, but which the engine will never itself select. Questionable moves are kept in a
/// separate map for display only; get_move never returns them.
pub struct OpeningBook {
    book: BTreeMap<ZobristT, Vec<Move>>, // Map of position hash to playable moves (with repeats).
    questionable: BTreeMap<ZobristT, Vec<Move>>, // Recognised-but-never-played '?' moves (display only).
    prng: Mutex<BookPrng>,                       // PRNG used to randomly select among book moves.
}

impl OpeningBook {
    /// Construct an empty opening book. Seeds the PRNG.
    pub fn new() -> OpeningBook {
        OpeningBook {
            book: BTreeMap::new(),
            questionable: BTreeMap::new(),
            prng: Mutex::new(BookPrng::new()),
        }
    }

    /// Load the opening book from a file.
    pub fn initialize(&mut self, filename: &str) -> bool {
        let Ok(text) = std::fs::read_to_string(filename) else {
            return false;
        };
        self.initialize_from_stream(&text)
    }

    /// Parse the opening book from an in-memory buffer (the copy embedded in the binary, used
    /// when the on-disk book cannot be loaded).
    pub fn initialize_from_memory(&mut self, data: &[u8]) -> bool {
        let text = String::from_utf8_lossy(data);
        self.initialize_from_stream(&text)
    }

    /// Pseudo randomly select from available book moves for `hash`, weighted by frequency.
    /// Returns Move::none() if no move is found.
    pub fn get_move(&self, hash: ZobristT) -> Move {
        if let Some(moves) = self.book.get(&hash) {
            let r = self.prng.lock().unwrap().next();
            return moves[(r % moves.len() as u64) as usize];
        }
        Move::none()
    }

    /// Release the memory held by the book.
    pub fn free(&mut self) {
        self.book.clear();
        self.questionable.clear();
    }

    /// Print available book moves for a position.
    pub fn display_available_moves(&self, position: &Position) {
        let Some(moves) = self.book.get(&position.hash) else {
            return;
        };
        // Count how many times each move appears for this position.
        let mut move_counts: HashMap<Move, i32> = HashMap::new();
        for &mv in moves {
            *move_counts.entry(mv).or_insert(0) += 1;
        }
        println!("MOVE   COUNT");
        for (mv, freq) in &move_counts {
            println!("{:<8} {:3}", mv.to_string(), freq);
        }
        // Questionable moves are recognised but never played; show them flagged with '?'.
        if let Some(q_moves) = self.questionable.get(&position.hash) {
            let mut q_counts: HashMap<Move, i32> = HashMap::new();
            for &mv in q_moves {
                *q_counts.entry(mv).or_insert(0) += 1;
            }
            for (mv, freq) in &q_counts {
                println!(
                    "{:<8} {:3}  (questionable, never played)",
                    mv.to_string() + "?",
                    freq
                );
            }
        }
    }

    /// Parse a single line of play and add it to the book. '#' denotes a comment and the rest of
    /// the line will be ignored. Move numbers are ignored. A move with a trailing '?' (e.g.
    /// "f2f4?") is *questionable*: it is replayed so the rest of the line still parses, but
    /// recorded in `questionable` rather than `book`, so get_move never returns it.
    fn parse_line_of_play(&mut self, line: &str) -> bool {
        let mut position =
            Position::from_string("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        for token in line.split_whitespace() {
            let mut move_string = token;
            if move_string.is_empty() || move_string.as_bytes()[0].is_ascii_digit() {
                continue; // Ignore move numbers or blanks.
            }
            if move_string.as_bytes()[0] == b'#' {
                return true; // Done with this line.
            }
            // A trailing '?' marks a questionable move: recognise and replay it, but never select it.
            let questionable = move_string.ends_with('?');
            if questionable {
                move_string = &move_string[..move_string.len() - 1];
            }
            let hash = position.hash;
            let moves = position.generate_legal_moves();
            let found = moves.iter().find(|m| m.to_string() == move_string).copied();
            if let Some(mv) = found {
                let map = if questionable {
                    &mut self.questionable
                } else {
                    &mut self.book
                };
                map.entry(hash).or_default().push(mv);
                position = position.make_move(mv);
            } else {
                println!("Error found in book line {} move {}.", line, move_string);
                return false;
            }
        }
        true
    }

    /// Parse multiline text containing opening book lines of play and create the opening book
    /// from it. Each line is a sequence of moves in coordinate notation (e.g. "e2e4 e7e5 g1f3").
    fn initialize_from_stream(&mut self, text: &str) -> bool {
        for line_of_play in text.lines() {
            if !self.parse_line_of_play(line_of_play) {
                return false;
            }
        }
        true
    }
}

impl Default for OpeningBook {
    fn default() -> Self {
        Self::new()
    }
}
