//! Chess position representation and legal move generation. (Port of `position.h`.)

use crate::attacks::{bishop_attacks, rook_attacks, PIECE_ATTACKERS, PIECE_ATTACKERS_EXCEPT_KING};
use crate::bitboard::{Bitboard, NO_SQUARES, RANK_1, RANK_4, RANK_5, RANK_8, WHITE_SQUARES};
use crate::castling_rights::CastlingRights;
use crate::constants::{enemy_of, Color, Piece};
use crate::generated_data::{
    ZobristT, BLACK_MOVE_HASH, EAST_WEST, EN_PASSANT_HASHES, INTERVENING_SQUARES, KING_ATTACKS,
    KNIGHT_ATTACKS, PAWN_ATTACKS_BLACK, PAWN_ATTACKS_WHITE, PIECE_SQUARE_HASHES,
};
use crate::increment;
use crate::moves::{Move, MoveList, MoveType};
use crate::pins::Pins;
use crate::square::Square;

/// A chess position. This is the primary struct that holds the state of the board and represents
/// the rules of chess. Immutable after construction: `make_move` / `make_null_move` return a new
/// Position by value.
#[derive(Clone, Copy)]
pub struct Position {
    // State variables
    pub pieces: [Bitboard; 7], // Bitboards indexed by Piece (index 0 = occupied squares)
    pub colors: [Bitboard; 2], // Per-color bitboards indexed by Color.
    pub squares: [Piece; 64],  // Squares array for fast piece lookup.
    pub checkers: Bitboard,    // Set of squares which attack the king.
    pub hash: ZobristT,        // Zobrist hash of this position, maintained incrementally.
    pub king_location: [Square; 2], // Square index of white and black kings.
    pub en_passant_square: Square, // En passant capture availability square.
    pub reversible_move_count: u8, // Number of consecutive reversible half-moves (plies).
    pub full_move_count: u8,   // Number of full moves (zero indexed).
    pub castling_rights: CastlingRights, // Castling rights flags.
    pub is_null_move: bool,    // Whether the last move was a null (pass) move.
    pub color_to_move: Color,  // Side to move.
}

impl Default for Position {
    /// The C++ `Position{}` value-initialization: everything zero / empty.
    fn default() -> Position {
        Position {
            pieces: [NO_SQUARES; 7],
            colors: [NO_SQUARES; 2],
            squares: [Piece::None; 64],
            checkers: NO_SQUARES,
            hash: 0,
            king_location: [Square(0); 2],
            en_passant_square: Square(0),
            reversible_move_count: 0,
            full_move_count: 0,
            castling_rights: CastlingRights::new(),
            is_null_move: false,
            color_to_move: Color::White,
        }
    }
}

impl Position {
    /// Generate all legal moves.
    #[inline]
    pub fn generate_legal_moves(&self) -> MoveList {
        if self.color_to_move == Color::White {
            self.gen_moves::<true, true>()
        } else {
            self.gen_moves::<false, true>()
        }
    }

    /// Generate legal captures and promotions only.
    #[inline]
    pub fn generate_legal_captures(&self) -> MoveList {
        if self.color_to_move == Color::White {
            self.gen_moves::<true, false>()
        } else {
            self.gen_moves::<false, false>()
        }
    }

    /// Whether a square is attacked by a colour.
    #[inline(always)]
    pub fn is_attacked(&self, s: Square, c: Color) -> bool {
        self.attacks_to(s, c).is_not_empty()
    }

    /// All occupied squares.
    #[inline(always)]
    pub fn occupied_squares(&self) -> Bitboard {
        self.pieces[Piece::None.index()]
    }

    /// Whether the side to move is in check.
    #[inline(always)]
    pub fn is_in_check(&self) -> bool {
        self.checkers.is_not_empty()
    }

    /// Determine attacks to a square by a color: the set of squares of `color` containing a piece
    /// attacking `location`.
    pub fn attacks_to(&self, location: Square, color: Color) -> Bitboard {
        let occupied_squares = self.occupied_squares();
        let mut result = if color == Color::White {
            PAWN_ATTACKS_BLACK[location.index()] & self.pieces[Piece::Pawn.index()]
        } else {
            PAWN_ATTACKS_WHITE[location.index()] & self.pieces[Piece::Pawn.index()]
        };
        result |= (KNIGHT_ATTACKS[location.index()] & self.pieces[Piece::Knight.index()])
            | (KING_ATTACKS[location.index()] & self.pieces[Piece::King.index()]);
        result |= rook_attacks(occupied_squares, location)
            & (self.pieces[Piece::Rook.index()] | self.pieces[Piece::Queen.index()]);
        result |= bishop_attacks(occupied_squares, location)
            & (self.pieces[Piece::Bishop.index()] | self.pieces[Piece::Queen.index()]);
        result &= self.colors[color.index()];
        result
    }

    /// Generate legal moves for this position.
    /// `IS_WHITE`: color to generate moves for; `DO_ALL_MOVES`: if true generate all moves,
    /// otherwise captures and promotions only. (The C++ `GenMoves<Color, bool>` template.)
    fn gen_moves<const IS_WHITE: bool, const DO_ALL_MOVES: bool>(&self) -> MoveList {
        let color = if IS_WHITE { Color::White } else { Color::Black };
        let occupied_squares = self.occupied_squares();
        let friendly_pieces = self.colors[color.index()];
        let enemy_pieces = occupied_squares ^ friendly_pieces;
        let enemy_pawns = self.pieces[Piece::Pawn.index()] & enemy_pieces;
        let king_locn = self.king_location[color.index()];

        let mut moves = MoveList::new();

        // Determine the squares which our king cannot move to, i.e. any square which is attacked
        // or X-ray attacked by any enemy piece. Start with pawns.
        let mut forbidden_king_squares = if IS_WHITE {
            enemy_pawns.shift_southwest() | enemy_pawns.shift_southeast()
        } else {
            enemy_pawns.shift_northwest() | enemy_pawns.shift_northeast()
        };

        // Temporarily remove our king to detect X-ray attacks from enemy sliding pieces.
        let occupied_except_king = occupied_squares ^ Bitboard::from_square(king_locn);

        for (piece, attack_fn) in PIECE_ATTACKERS {
            let b = self.pieces[piece.index()] & enemy_pieces;
            for s in b {
                forbidden_king_squares |= attack_fn(occupied_except_king, s);
            }
        }

        // The king may only safely move to squares which are not forbidden.
        let king_move_targets = KING_ATTACKS[king_locn.index()] & !forbidden_king_squares;

        // Generate King moves.
        let king_captures = king_move_targets & enemy_pieces;
        for to in king_captures {
            moves.push_back(Move::capture(
                king_locn,
                to,
                Piece::King,
                self.squares[to.index()],
            ));
        }
        if DO_ALL_MOVES {
            let king_non_captures = king_move_targets & !occupied_squares;
            for to in king_non_captures {
                moves.push_back(Move::non_capture(king_locn, to, Piece::King));
            }
        }

        let mut allowed_captures = enemy_pieces; // The set of pieces we may capture.
        let mut allowed_non_captures = !occupied_squares; // The set of empty squares we may move to.

        // If we are in check then the set of possible captures and empty squares that are legal is
        // much smaller, since we must immediately get out of check.
        if self.is_in_check() {
            if self.checkers.pop_count() > 1 {
                // Multiple check: only moving the king is possible so we are done.
                return moves;
            }
            // We are in check, the options are:
            // a) Capture the checking piece, OR
            // b) If checked by a sliding piece, interpose a piece between the checker and our king.
            let checker_locn = self.checkers.lsb();
            allowed_captures = self.checkers;
            allowed_non_captures = INTERVENING_SQUARES[king_locn.index()][checker_locn.index()];
        }

        let pins = Pins::new(self); // Determine pinned pieces and their allowed destination squares.

        // Generate knight, bishop, rook and queen moves.
        for (piece, attack_fn) in PIECE_ATTACKERS_EXCEPT_KING {
            let b = self.pieces[piece.index()] & friendly_pieces;
            for from in b {
                let attacks = attack_fn(occupied_squares, from) & pins.allowed_squares(from);
                let capture_targets = attacks & allowed_captures;
                for to in capture_targets {
                    moves.push_back(Move::capture(from, to, piece, self.squares[to.index()]));
                }
                if DO_ALL_MOVES {
                    let non_capture_targets = attacks & allowed_non_captures;
                    for to in non_capture_targets {
                        moves.push_back(Move::non_capture(from, to, piece));
                    }
                }
            }
        }

        // Generate castling moves. The conditions for castling are:
        // 1) King must not have moved
        // 2) Rook must not have moved
        // 3) King is not in check
        // 4) Squares between King and Rook are vacant
        // 5) Square King passes through is not attacked by the enemy.
        if DO_ALL_MOVES && !self.is_in_check() {
            if IS_WHITE {
                if self.castling_rights.may_white_castle_kingside()
                    && (occupied_squares & (sq_bb("F1") | sq_bb("G1"))).is_empty()
                    && !self.is_attacked(Square::from_str("F1"), Color::Black)
                    && !self.is_attacked(Square::from_str("G1"), Color::Black)
                {
                    moves.push_back(Move::castling(
                        Square::from_str("E1"),
                        Square::from_str("G1"),
                    ));
                }
                if self.castling_rights.may_white_castle_queenside()
                    && (occupied_squares & (sq_bb("B1") | sq_bb("C1") | sq_bb("D1"))).is_empty()
                    && !self.is_attacked(Square::from_str("D1"), Color::Black)
                    && !self.is_attacked(Square::from_str("C1"), Color::Black)
                {
                    moves.push_back(Move::castling(
                        Square::from_str("E1"),
                        Square::from_str("C1"),
                    ));
                }
            } else {
                if self.castling_rights.may_black_castle_kingside()
                    && (occupied_squares & (sq_bb("F8") | sq_bb("G8"))).is_empty()
                    && !self.is_attacked(Square::from_str("F8"), Color::White)
                    && !self.is_attacked(Square::from_str("G8"), Color::White)
                {
                    moves.push_back(Move::castling(
                        Square::from_str("E8"),
                        Square::from_str("G8"),
                    ));
                }
                if self.castling_rights.may_black_castle_queenside()
                    && (occupied_squares & (sq_bb("B8") | sq_bb("C8") | sq_bb("D8"))).is_empty()
                    && !self.is_attacked(Square::from_str("D8"), Color::White)
                    && !self.is_attacked(Square::from_str("C8"), Color::White)
                {
                    moves.push_back(Move::castling(
                        Square::from_str("E8"),
                        Square::from_str("C8"),
                    ));
                }
            }
        }

        // Generate pawn moves.
        let pawns; // friendly pawns
        let mut single_pushes; // single push targets
        let double_pushes; // double push targets
        let mut captures_west; // capture west targets
        let mut captures_east; // capture east targets
        let en_passant_sources; // pawns which can capture ep
        let promotions; // push promotion targets
        let promotions_west; // promotion capture west targets
        let promotions_east; // promotion capture east targets
        let push_delta: i8; // square index delta for push forward
        let west_delta: i8; // square index delta for capture west
        let east_delta: i8; // square index delta for capture east

        if IS_WHITE {
            pawns = self.pieces[Piece::Pawn.index()] & self.colors[Color::White.index()];
            single_pushes = pawns.shift_north() & !occupied_squares;
            double_pushes = single_pushes.shift_north() & !occupied_squares & RANK_4;
            captures_west = pawns.shift_northwest() & self.colors[Color::Black.index()];
            captures_east = pawns.shift_northeast() & self.colors[Color::Black.index()];
            en_passant_sources = if self.en_passant_square.0 != 0 {
                PAWN_ATTACKS_BLACK[self.en_passant_square.index()] & pawns
            } else {
                NO_SQUARES
            };
            promotions = single_pushes & RANK_8;
            promotions_west = captures_west & RANK_8;
            promotions_east = captures_east & RANK_8;
            push_delta = 8;
            west_delta = 7;
            east_delta = 9;
        } else {
            pawns = self.pieces[Piece::Pawn.index()] & self.colors[Color::Black.index()];
            single_pushes = pawns.shift_south() & !occupied_squares;
            double_pushes = single_pushes.shift_south() & !occupied_squares & RANK_5;
            captures_west = pawns.shift_southwest() & self.colors[Color::White.index()];
            captures_east = pawns.shift_southeast() & self.colors[Color::White.index()];
            en_passant_sources = if self.en_passant_square.0 != 0 {
                PAWN_ATTACKS_WHITE[self.en_passant_square.index()] & pawns
            } else {
                NO_SQUARES
            };
            promotions = single_pushes & RANK_1;
            promotions_west = captures_west & RANK_1;
            promotions_east = captures_east & RANK_1;
            push_delta = -8;
            west_delta = -9;
            east_delta = -7;
        }
        captures_west ^= promotions_west;
        captures_east ^= promotions_east;
        single_pushes ^= promotions;

        let capture_promotions: [(Bitboard, i8); 2] =
            [(promotions_west, west_delta), (promotions_east, east_delta)];
        for (bb, delta) in capture_promotions {
            let b = bb & allowed_captures;
            for to in b {
                let from = Square((to.0 as i32 - delta as i32) as u8);
                if pins.is_allowed(from, to) {
                    moves.push_back(Move::promotion(
                        from,
                        to,
                        Piece::Queen,
                        self.squares[to.index()],
                    ));
                    moves.push_back(Move::promotion(
                        from,
                        to,
                        Piece::Rook,
                        self.squares[to.index()],
                    ));
                    moves.push_back(Move::promotion(
                        from,
                        to,
                        Piece::Bishop,
                        self.squares[to.index()],
                    ));
                    moves.push_back(Move::promotion(
                        from,
                        to,
                        Piece::Knight,
                        self.squares[to.index()],
                    ));
                }
            }
        }
        let b = promotions & allowed_non_captures;
        for to in b {
            let from = Square((to.0 as i32 - push_delta as i32) as u8);
            if pins.is_allowed(from, to) {
                moves.push_back(Move::promotion(from, to, Piece::Queen, Piece::None));
                moves.push_back(Move::promotion(from, to, Piece::Rook, Piece::None));
                moves.push_back(Move::promotion(from, to, Piece::Bishop, Piece::None));
                moves.push_back(Move::promotion(from, to, Piece::Knight, Piece::None));
            }
        }
        let captures: [(Bitboard, i8); 2] =
            [(captures_west, west_delta), (captures_east, east_delta)];
        for (bb, delta) in captures {
            let b = bb & allowed_captures;
            for to in b {
                let from = Square((to.0 as i32 - delta as i32) as u8);
                if pins.is_allowed(from, to) {
                    moves.push_back(Move::capture(
                        from,
                        to,
                        Piece::Pawn,
                        self.squares[to.index()],
                    ));
                }
            }
        }
        if DO_ALL_MOVES {
            let b = single_pushes & allowed_non_captures;
            for to in b {
                let from = Square((to.0 as i32 - push_delta as i32) as u8);
                if pins.is_allowed(from, to) {
                    moves.push_back(Move::non_capture(from, to, Piece::Pawn));
                }
            }
            let b = double_pushes & allowed_non_captures;
            for to in b {
                let from = Square((to.0 as i32 - push_delta as i32 * 2) as u8);
                if pins.is_allowed(from, to) {
                    moves.push_back(Move::double_push(from, to));
                }
            }
        }
        // Generate en passant captures. En passant captures are only legal if the captured pawn
        // location (capture target) is in the capture mask. We also have to test for the special
        // case of discovered horizontal check when removing both pawns from the king's rank.
        for from in en_passant_sources {
            let to = self.en_passant_square;
            // The captured pawn is on the source rank, destination file.
            let captured_pawn_locn = Square((from.0 & 0x38) | (to.0 & 0x07));
            if pins.is_allowed(from, to)
                && (allowed_captures & Bitboard::from_square(captured_pawn_locn)).is_not_empty()
            {
                // Test for the weird check that occurs when there is an enemy rook or queen on the
                // same rank as our king which is only discovered after removing both pawns from the
                // rank during an en passant capture.
                if king_locn.rank() == from.rank() {
                    // Remove the capturing and ep captured pawns from the occupied squares set and
                    // test for horizontal ray attacks.
                    let pseudo_occupied_squares = occupied_squares
                        ^ Bitboard::from_square(captured_pawn_locn)
                        ^ Bitboard::from_square(from);
                    let horizontal_attacks = rook_attacks(pseudo_occupied_squares, king_locn)
                        & EAST_WEST[king_locn.index()];
                    if (horizontal_attacks
                        & enemy_pieces
                        & (self.pieces[Piece::Rook.index()] | self.pieces[Piece::Queen.index()]))
                    .is_empty()
                    {
                        // We can make this move since it's not a discovered check.
                        moves.push_back(Move::ep_capture(from, to));
                    }
                } else {
                    moves.push_back(Move::ep_capture(from, to));
                }
            }
        }
        moves
    }

    /// Make a null (pass) move and return the new position.
    pub fn make_null_move(&self) -> Position {
        let mut position = *self;
        position.is_null_move = true;
        position.color_to_move = enemy_of(position.color_to_move);
        position.hash ^= EN_PASSANT_HASHES[position.en_passant_square.index()];
        position.hash ^= BLACK_MOVE_HASH;
        position.en_passant_square = Square(0);
        position
    }

    /// Add a piece to the board.
    #[inline(always)]
    fn add_piece(&mut self, color: Color, piece: Piece, to: Square) {
        let to_bb = Bitboard::from_square(to);
        self.pieces[piece.index()] ^= to_bb;
        self.colors[color.index()] ^= to_bb;
        self.pieces[Piece::None.index()] ^= to_bb; // pieces[None] holds the occupied-squares bitboard
        self.hash ^= PIECE_SQUARE_HASHES[color.index()][piece.index() - 1][to.index()];
        self.squares[to.index()] = piece;
    }

    /// Remove a piece from the board.
    #[inline(always)]
    fn remove_piece(&mut self, color: Color, piece: Piece, from: Square) {
        let from_bb = Bitboard::from_square(from);
        self.pieces[piece.index()] ^= from_bb;
        self.colors[color.index()] ^= from_bb;
        self.pieces[Piece::None.index()] ^= from_bb; // pieces[None] holds the occupied-squares bitboard
        self.hash ^= PIECE_SQUARE_HASHES[color.index()][piece.index() - 1][from.index()];
        self.squares[from.index()] = Piece::None;
    }

    /// Move a piece on the board.
    #[inline(always)]
    fn move_piece(&mut self, color: Color, piece: Piece, from: Square, to: Square) {
        let from_to_bb = Bitboard::from_square(from) | Bitboard::from_square(to);
        let hash = &PIECE_SQUARE_HASHES[color.index()][piece.index() - 1];
        self.pieces[piece.index()] ^= from_to_bb;
        self.colors[color.index()] ^= from_to_bb;
        self.pieces[Piece::None.index()] ^= from_to_bb; // pieces[None] holds the occupied-squares bitboard
        self.hash ^= hash[to.index()] ^ hash[from.index()];
        self.squares[from.index()] = Piece::None;
        self.squares[to.index()] = piece;
    }

    /// Make a move and return the new position.
    pub fn make_move(&self, mv: Move) -> Position {
        const G1: Square = Square::from_str("G1"); // White kingside castling king destination.
        const C1: Square = Square::from_str("C1"); // White queenside castling king destination.
        const G8: Square = Square::from_str("G8"); // Black kingside castling king destination.
        const C8: Square = Square::from_str("C8"); // Black queenside castling king destination.

        let color = self.color_to_move;
        let from = mv.from();
        let to = mv.to();
        let piece = mv.piece();

        let mut position = *self;
        position.is_null_move = false;
        position.castling_rights.after_move(mv);
        position.hash ^= position.castling_rights.hash() ^ self.castling_rights.hash();
        position.hash ^= EN_PASSANT_HASHES[position.en_passant_square.index()];
        position.en_passant_square = Square(0);

        match mv.type_() {
            MoveType::NonCapture => {
                // A single pawn push is encoded as NonCapture (no dedicated type); like any pawn
                // move it resets the fifty-move / reversible-move clock. Every other NonCapture is
                // a reversible piece move and increments.
                if piece == Piece::Pawn {
                    position.reversible_move_count = 0;
                } else {
                    position.reversible_move_count += 1;
                }
                position.move_piece(color, piece, from, to);
            }
            MoveType::Capture => {
                position.reversible_move_count = 0;
                position.remove_piece(enemy_of(color), mv.captured(), to);
                position.move_piece(color, piece, from, to);
            }
            MoveType::PromotionKnight
            | MoveType::PromotionBishop
            | MoveType::PromotionRook
            | MoveType::PromotionQueen => {
                position.reversible_move_count = 0;
                if mv.captured() != Piece::None {
                    position.remove_piece(enemy_of(color), mv.captured(), to);
                }
                position.remove_piece(color, Piece::Pawn, from);
                position.add_piece(color, mv.promoted(), to);
            }
            MoveType::PawnDoublePush => {
                position.reversible_move_count = 0;
                position.move_piece(color, Piece::Pawn, from, to);
                position.en_passant_square = Square((from.0 + to.0) >> 1);
                position.hash ^= EN_PASSANT_HASHES[position.en_passant_square.index()];
            }
            MoveType::EpCapture => {
                position.reversible_move_count = 0;
                // En passant capture: capture location is source rank, destination file.
                position.remove_piece(
                    enemy_of(color),
                    Piece::Pawn,
                    Square((from.0 & 0x38) | (to.0 & 0x07)),
                );
                position.move_piece(color, Piece::Pawn, from, to);
            }
            MoveType::Castling => {
                position.reversible_move_count = 0;
                if to == G1 {
                    position.move_piece(
                        Color::White,
                        Piece::King,
                        Square::from_str("E1"),
                        Square::from_str("G1"),
                    );
                    position.move_piece(
                        Color::White,
                        Piece::Rook,
                        Square::from_str("H1"),
                        Square::from_str("F1"),
                    );
                } else if to == C1 {
                    position.move_piece(
                        Color::White,
                        Piece::King,
                        Square::from_str("E1"),
                        Square::from_str("C1"),
                    );
                    position.move_piece(
                        Color::White,
                        Piece::Rook,
                        Square::from_str("A1"),
                        Square::from_str("D1"),
                    );
                } else if to == G8 {
                    position.move_piece(
                        Color::Black,
                        Piece::King,
                        Square::from_str("E8"),
                        Square::from_str("G8"),
                    );
                    position.move_piece(
                        Color::Black,
                        Piece::Rook,
                        Square::from_str("H8"),
                        Square::from_str("F8"),
                    );
                } else if to == C8 {
                    position.move_piece(
                        Color::Black,
                        Piece::King,
                        Square::from_str("E8"),
                        Square::from_str("C8"),
                    );
                    position.move_piece(
                        Color::Black,
                        Piece::Rook,
                        Square::from_str("A8"),
                        Square::from_str("D8"),
                    );
                }
            }
        }
        position.color_to_move = enemy_of(color);
        position.hash ^= BLACK_MOVE_HASH;
        position.full_move_count = position.full_move_count.wrapping_add(color as u8); // Increments after black's move.
        position.king_location[color.index()] =
            (position.pieces[Piece::King.index()] & position.colors[color.index()]).lsb();
        position.checkers =
            position.attacks_to(position.king_location[enemy_of(color).index()], color);
        position
    }

    /// Construct a position from a Forsyth-Edwards (FEN) string.
    pub fn from_string(fen_string: &str) -> Position {
        let mut position = Position::default();
        const WHITE_PIECE_NAMES: &str = "PNBRQK";
        const BLACK_PIECE_NAMES: &str = "pnbrqk";
        let mut fields = fen_string.split_whitespace();
        // Pieces on the board
        let pieces = fields.next().unwrap_or("");
        let mut x: i32 = 0;
        let mut y: i32 = 7;
        for c in pieces.chars() {
            if c == '/' {
                x = 0;
                y -= 1;
                continue;
            }
            if ('1'..='8').contains(&c) {
                x += c as i32 - '0' as i32;
                continue;
            }
            if let Some(a) = WHITE_PIECE_NAMES.find(c) {
                let piece = Piece::from_u8(a as u8 + 1);
                position.add_piece(
                    Color::White,
                    piece,
                    Square::from_file_rank(x as u8, y as u8),
                );
                x += 1;
                continue;
            }
            if let Some(a) = BLACK_PIECE_NAMES.find(c) {
                let piece = Piece::from_u8(a as u8 + 1);
                position.add_piece(
                    Color::Black,
                    piece,
                    Square::from_file_rank(x as u8, y as u8),
                );
                x += 1;
                continue;
            }
        }
        // Side to move
        let color_to_move = fields.next().unwrap_or("w");
        if color_to_move == "b" {
            position.color_to_move = Color::Black;
        }
        position.king_location[Color::White.index()] =
            (position.pieces[Piece::King.index()] & position.colors[Color::White.index()]).lsb();
        position.king_location[Color::Black.index()] =
            (position.pieces[Piece::King.index()] & position.colors[Color::Black.index()]).lsb();
        // Castling rights
        let castling_rights = fields.next().unwrap_or("-");
        position.castling_rights = CastlingRights::from_fen(castling_rights);
        // En passant capture square
        let ep_square = fields.next().unwrap_or("-");
        if ep_square == "-" {
            position.en_passant_square = Square(0);
        } else {
            position.en_passant_square = Square::from_str(ep_square);
        }
        // Half move clock - optional
        if let Some(hmc) = fields.next().and_then(|s| s.parse::<i32>().ok()) {
            position.reversible_move_count = hmc as u8;
        }
        // Full move number - optional
        if let Some(fmn) = fields.next().and_then(|s| s.parse::<i32>().ok()) {
            position.full_move_count = (fmn as u8).wrapping_sub(1);
        }
        position.hash = position.compute_hash();
        // Is this position check?
        let color = position.color_to_move;
        position.checkers =
            position.attacks_to(position.king_location[color.index()], enemy_of(color));
        position
    }

    /// Serialize the position to a FEN string.
    pub fn to_string(&self) -> String {
        let mut ss = String::new();
        // Pieces on the board
        let occupied_squares = self.occupied_squares();
        for y in (0..8).rev() {
            let mut num_empty_squares = 0;
            for x in 0..8 {
                if (occupied_squares & Bitboard::from_file_rank(x, y)).is_empty() {
                    num_empty_squares += 1;
                } else {
                    if num_empty_squares != 0 {
                        ss.push_str(&num_empty_squares.to_string());
                        num_empty_squares = 0;
                    }
                    let sq = (x + 8 * y) as usize;
                    let piece = if (self.colors[Color::White.index()]
                        & Bitboard::from_file_rank(x, y))
                    .is_not_empty()
                    {
                        b" PNBRQK"[self.squares[sq].index()]
                    } else {
                        b" pnbrqk"[self.squares[sq].index()]
                    };
                    ss.push(piece as char);
                }
            }
            if num_empty_squares != 0 {
                ss.push_str(&num_empty_squares.to_string());
            }
            if y != 0 {
                ss.push('/');
            }
        }
        // Side to move
        ss.push(' ');
        ss.push(if self.color_to_move == Color::Black {
            'b'
        } else {
            'w'
        });
        ss.push(' ');
        // Castling rights
        ss.push_str(&self.castling_rights.to_fen_string());
        ss.push(' ');
        // En passant capture availability
        if self.en_passant_square.0 == 0 {
            ss.push('-');
        } else {
            ss.push_str(&self.en_passant_square.to_string());
        }
        ss.push(' ');
        // Half move clock and full move number
        ss.push_str(&format!(
            "{} {}",
            self.reversible_move_count,
            self.full_move_count as i32 + 1
        ));
        ss
    }

    /// Determine if the position is draw by insufficient material.
    pub fn is_draw_by_material(&self) -> bool {
        let occupied_squares = self.occupied_squares();
        match occupied_squares.pop_count() {
            0 | 1 => {
                println!("ERROR: too few pieces for play");
                true
            }
            2 => {
                // king vs king
                increment!("draws by material 2");
                true
            }
            3 => {
                // king and bishop vs king or king and knight vs king
                if (self.pieces[Piece::Bishop.index()] | self.pieces[Piece::Knight.index()])
                    .is_not_empty()
                {
                    increment!("draws by material 3");
                    return true;
                }
                false
            }
            4 => {
                // king and bishop vs king and bishop with bishops on same color square
                let white_bishops =
                    self.pieces[Piece::Bishop.index()] & self.colors[Color::White.index()];
                let black_bishops =
                    self.pieces[Piece::Bishop.index()] & self.colors[Color::Black.index()];
                let is_white_bishop_on_white_square =
                    (white_bishops & WHITE_SQUARES).is_not_empty();
                let is_black_bishop_on_white_square =
                    (black_bishops & WHITE_SQUARES).is_not_empty();
                if white_bishops.is_not_empty()
                    && black_bishops.is_not_empty()
                    && is_white_bishop_on_white_square == is_black_bishop_on_white_square
                {
                    increment!("draws by material 4");
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Compute the Zobrist hash for a chess position from scratch.
    pub fn compute_hash(&self) -> ZobristT {
        let mut hash: ZobristT = if self.color_to_move == Color::Black {
            BLACK_MOVE_HASH
        } else {
            0
        };
        hash ^= self.castling_rights.hash();
        hash ^= EN_PASSANT_HASHES[self.en_passant_square.index()];
        for piece in Piece::ALL {
            let b = self.pieces[piece.index()] & self.colors[Color::White.index()];
            for s in b {
                hash ^= PIECE_SQUARE_HASHES[Color::White.index()][piece.index() - 1][s.index()];
            }
            let b = self.pieces[piece.index()] & self.colors[Color::Black.index()];
            for s in b {
                hash ^= PIECE_SQUARE_HASHES[Color::Black.index()][piece.index() - 1][s.index()];
            }
        }
        hash
    }
}

/// Single-square bitboard from a square name (the C++ `Bitboard("F1")` shorthand).
#[inline(always)]
const fn sq_bb(name: &str) -> Bitboard {
    Bitboard::from_square(Square::from_str(name))
}
