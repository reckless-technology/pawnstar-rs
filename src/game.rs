//! Game: the shared engine state (transposition table, clock, NNUE net, book, options) and the
//! root search entry point, shared by all Lazy SMP threads. (Port of `game.h`.)
//!
//! Threading model vs the C++: C++ shares `Game&` mutably across the UCI thread, the detached
//! search worker and the Lazy SMP helpers by convention. Rust encodes that convention in types:
//! everything a search thread writes is an atomic (or the `positions` mutex, which is never on
//! the per-node hot path — each SearchState owns its own position stack); every other field is
//! written only through `&mut Game`, which the `Engine` wrapper hands out only after joining the
//! worker (`Engine::game_mut`).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::chess_clock::{ChessClock, ClockType};
use crate::constants::*;
use crate::debug_hashtable::debug_x_clear;
use crate::eval_cache::EvalCache;
use crate::moves::{sort_moves, Move, MoveList};
use crate::nnue;
use crate::opening_book::OpeningBook;
use crate::position::Position;
use crate::search_state::{SearchState, Variation};
use crate::transposition_table::TranspositionTable;
use crate::uci_println;

/// The standard initial position.
pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// A game of chess: the shared engine state and the root search entry point.
pub struct Game {
    // State variables
    pub transposition_table: TranspositionTable, // The transposition table (shared by all Lazy SMP threads).
    pub eval_cache: EvalCache,                   // Lockless NNUE evaluation cache (shared by all threads).
    pub time_control: ChessClock,                // Clock controls for the current game.
    pub book: OpeningBook,                       // The opening book.
    pub is_cancel_pending: AtomicBool,           // Shared stop flag: set on timeout, UCI stop, or completion.
    pub thread_count: i32,                       // Lazy SMP search threads; default from PAWNSTAR_THREADS / hardware.
    pub move_overhead: i64,                      // Reserved from each hard deadline for GUI/network lag (ms).
    pub is_pondering: AtomicBool,                // True while a `go ponder` search runs; suppresses time stops.
    pub ponder_move: AtomicU64,                  // Predicted opponent reply (PV[1]) as Move::bits().
    pub use_own_book: bool,                      // Whether to consult the built-in opening book (UCI `OwnBook`).
    pub last_search_node_count: AtomicU64,       // Node count of the most recent search's main thread (for `bench`).
    pub search_moves: Vec<String>,               // UCI `go searchmoves`: restrict the root search (empty = all).
    pub positions: Mutex<Vec<Position>>,         // Game position history (grows with the game; no fixed cap).
    pub nnue_network: nnue::Network,             // NNUE network instance (loaded via EvalFile; read-only in search).
}

impl Game {
    /// Construct a game, sizing the transposition tables and starting from the initial position.
    pub fn new() -> Game {
        let mut game = Game {
            transposition_table: TranspositionTable::new(HASHTABLE_MEGABYTES as usize),
            eval_cache: EvalCache::new(EVAL_CACHE_MB),
            time_control: ChessClock::new(),
            book: OpeningBook::new(),
            is_cancel_pending: AtomicBool::new(false),
            thread_count: Game::compute_default_threads(),
            move_overhead: 30,
            is_pondering: AtomicBool::new(false),
            ponder_move: AtomicU64::new(Move::none().bits()),
            use_own_book: true,
            last_search_node_count: AtomicU64::new(0),
            search_moves: Vec::new(),
            positions: Mutex::new(Vec::new()),
            nnue_network: nnue::Network::new(),
        };
        game.set_position_start();
        game
    }

    /// Default Lazy SMP thread count: PAWNSTAR_THREADS if set (>0), else hardware concurrency,
    /// clamped to [1, MAX_SEARCH_THREADS]. Read once at construction; the UCI `Threads` option
    /// overrides `thread_count`.
    pub fn compute_default_threads() -> i32 {
        let hw = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(1);
        let mut n = hw.clamp(1, MAX_SEARCH_THREADS);
        if let Ok(env) = std::env::var("PAWNSTAR_THREADS") {
            let requested: i32 = env.trim().parse().unwrap_or(0);
            if requested > 0 {
                n = requested.clamp(1, MAX_SEARCH_THREADS);
            }
        }
        n
    }

    /// Current position (a copy — the C++ returns a reference into `positions_`, which the port's
    /// mutex-guarded history cannot; Position is a 160-byte Copy type and no caller is on the
    /// per-node hot path, which uses SearchState's own stack).
    pub fn current_position(&self) -> Position {
        *self.positions.lock().unwrap().last().expect("position history is never empty")
    }

    /// Set the board to a position from a FEN (Forsyth Edwards) string, resetting per-game
    /// transient state (move stack, clock, eval cache, cancel flag). Called for a new game and by
    /// every `position` command.
    pub fn set_position(&mut self, fen_string: &str) {
        self.time_control.reset();
        self.is_cancel_pending.store(false, Ordering::Relaxed);
        self.eval_cache.clear(); // evals are net-specific; start each game with a clean cache
        let mut positions = self.positions.lock().unwrap();
        positions.clear();
        positions.reserve(1024); // typical games stay well under this
        positions.push(Position::from_string(fen_string));
    }

    /// Set the board to the standard initial position (the C++ no-argument SetPosition overload).
    pub fn set_position_start(&mut self) {
        self.set_position(START_FEN);
    }

    /// Play a move and update state. Move must be legal for the current position.
    pub fn play_move(&self, mv: Move) {
        let mut positions = self.positions.lock().unwrap();
        let cur = *positions.last().unwrap();
        positions.push(cur.make_move(mv));
    }

    /// Make a null move. Only used during null move heuristic.
    pub fn make_null_move(&self) {
        let mut positions = self.positions.lock().unwrap();
        let cur = *positions.last().unwrap();
        positions.push(cur.make_null_move());
    }

    /// Undo the last move played.
    pub fn undo_move(&self) {
        self.positions.lock().unwrap().pop();
    }

    /// Play a move from the given string and update game state accordingly.
    /// Returns the move which was made, or Move::none() in the case the string did not contain a
    /// legal move for this position, and no move was therefore played.
    pub fn play_move_str(&self, move_str: &str) -> Move {
        let move_list = self.current_position().generate_legal_moves();
        for mv in move_list.iter() {
            if mv.to_string() == move_str {
                self.play_move(*mv);
                return *mv;
            }
        }
        Move::none()
    }

    /// Entry point for the search worker thread.
    pub fn search_thread_entry(&self) {
        let mv = self.search_root_node();
        // UCI forbids sending `bestmove` while pondering: if the ponder search ended on its own
        // (reached max depth or a forced mate) before the GUI resolved it, wait here until
        // `ponderhit` clears is_pondering or `stop` sets is_cancel_pending. For a normal
        // (non-ponder) search is_pondering is false, so this is a no-op.
        while self.is_pondering.load(Ordering::Relaxed) && !self.is_cancel_pending.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if mv != Move::none() {
            self.play_move(mv);
            let ponder = Move::from_bits(self.ponder_move.load(Ordering::Relaxed));
            if ponder != Move::none() {
                uci_println!("bestmove {} ponder {}", mv.to_string(), ponder.to_string());
            } else {
                uci_println!("bestmove {}", mv.to_string());
            }
        }
    }

    /// Search the current position and return the best move.
    /// Returns a book move immediately on a book hit; otherwise runs the Lazy SMP
    /// iterative-deepening search: one authoritative main thread plus (thread_count - 1) helpers,
    /// all sharing the transposition table. Returns Move::none() if there are no legal moves.
    pub fn search_root_node(&self) -> Move {
        let game = self; // local alias so the search body reads uniformly through `game`
        game.ponder_move.store(Move::none().bits(), Ordering::Relaxed);
        // Clear the stop flag (set true by start_thinking's stop_thinking) up front — BEFORE the
        // book / single-move early returns. Otherwise a leftover cancel makes
        // search_thread_entry's ponder-wait exit immediately and emit a premature `bestmove` when
        // `go ponder` lands on a book/forced position (which returns without search).
        game.is_cancel_pending.store(false, Ordering::Relaxed);
        game.last_search_node_count.store(0, Ordering::Relaxed); // stays 0 for book/forced returns
        // If there is a book move for this position, do not bother with search (unless OwnBook is
        // disabled). Skip the book when `go searchmoves` restricts the root.
        if game.use_own_book && game.search_moves.is_empty() {
            let book_move = game.book.get_move(game.current_position().hash);
            if book_move != Move::none() {
                return book_move;
            }
        }
        let mut move_list = game.current_position().generate_legal_moves();
        // UCI `go searchmoves`: restrict the root move list to the requested moves (ignored if
        // none of them are legal here, so a malformed list can't wedge the search).
        if !game.search_moves.is_empty() {
            let mut filtered = MoveList::new();
            for mv in move_list.iter() {
                if game.search_moves.iter().any(|s| *s == mv.to_string()) {
                    filtered.push_back(*mv);
                }
            }
            if filtered.size() != 0 {
                move_list = filtered;
            }
        }
        if move_list.size() == 0 {
            return Move::none();
        }
        // With only one legal move there is nothing to search — but when `searchmoves` is set we
        // still search it (the caller wants its evaluation), so only take this shortcut for a
        // genuine forced move.
        if move_list.size() == 1 && game.search_moves.is_empty() {
            return move_list[0];
        }
        // Plan time usage for this search.
        let mut allocated_time: i64 = 0; // soft budget (drives between-iteration stopping), ms
        let mut max_search_time: i64 = 0; // hard budget (deadline), ms; zero means no hard stop
        match game.time_control.clock_type {
            ClockType::Standard => {
                let moves_to_go: i32 = if game.time_control.moves_to_go != 0 {
                    game.time_control.moves_to_go
                } else {
                    (40 - game.current_position().full_move_count as i32).max(5)
                };
                // Soft budget: an even slice of the remaining time. NOTE: folding the per-move
                // increment in here was SPRT-tested and did NOT help; `increment` is parsed
                // (winc/binc) but deliberately unused here.
                allocated_time = game.time_control.remaining_time / moves_to_go as i64;
                // Hard budget: up to 2x the soft budget, but never spend into the last second of
                // our clock, and always allow at least 100 ms.
                max_search_time = 100i64.max((allocated_time * 2).min(game.time_control.remaining_time - 1000));
            }
            ClockType::FixedDepth => {} // no time limit; the depth loop bounds the search
            ClockType::FixedTime => {
                max_search_time = game.time_control.remaining_time; // movetime: a hard budget
            }
            ClockType::Infinite => {} // no time limit; only `stop` ends it
        }
        // Reserve the configured move overhead (GUI/network lag) from the hard deadline so we
        // don't forfeit on time. Only when there is a hard deadline.
        if max_search_time > 0 {
            max_search_time = 1i64.max(max_search_time - game.move_overhead);
        }

        // Arm the clock. A ponder search has no hard deadline until `ponderhit`
        // (budget-from-ponderhit); the budgets are recorded so the conversion can start the real
        // clock from that moment.
        if game.is_pondering.load(Ordering::Relaxed) {
            game.time_control.start_ponder_search(allocated_time, max_search_time);
        } else {
            game.time_control.start_search(allocated_time, max_search_time);
        }
        debug_x_clear();
        game.transposition_table.age();

        // Shallow pass for initial move ordering (on a temporary state shared as the launch ordering).
        let mut state = SearchState::new(game);
        let mut shallow_pv: Variation = Vec::new();
        for move_index in 0..move_list.size() {
            let mv = move_list[move_index];
            let score = state
                .search_single_move(START_DEPTH, 0, ALPHA, BETA, mv, &mut shallow_pv, move_index as i32)
                .score();
            move_list[move_index].assign_score(score);
        }
        sort_moves(&mut move_list);
        let mut best_move = move_list[0];

        // Lazy SMP: each thread runs an independent iterative-deepening search of the same root,
        // sharing only the (lockless) transposition table (history/killers/etc. are per-thread on
        // each SearchState). The main thread is authoritative; helper threads deepen the shared TT
        // to accelerate it. Each thread has its own SearchState and move-list copy.
        let n_threads = game.thread_count.clamp(1, MAX_SEARCH_THREADS);
        std::thread::scope(|s| {
            for i in 1..n_threads {
                // Each helper gets a distinct thread id, which selects its iteration-skip schedule.
                let ml = move_list.clone();
                let bm = best_move;
                s.spawn(move || {
                    let mut helper_state = SearchState::new(game);
                    helper_state.iterative_deepen(ml, bm, i);
                });
            }

            best_move = state.iterative_deepen(move_list.clone(), best_move, 0);
            game.last_search_node_count.store(state.node_count, Ordering::Relaxed); // main-thread total (for `bench`)

            game.is_cancel_pending.store(true, Ordering::Relaxed); // signal helpers to stop
        }); // scope end joins the helpers, as the C++ join loop does
        best_move
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

/// Owner of the shared `Game` and the detached UCI search worker (the C++ `Game::worker_thread_`
/// plus `StartThinking`/`StopThinking`). `game_mut` is the compiler-enforced version of the C++
/// convention "the UCI thread mutates Game only while no search runs": it joins the worker, then
/// `Arc::get_mut` proves exclusivity.
pub struct Engine {
    game: Arc<Game>,
    worker: Option<std::thread::JoinHandle<()>>, // Worker thread for searching moves.
}

impl Engine {
    /// Construct the engine around a fresh heap-allocated Game (the C++ `std::make_unique<Game>()`).
    pub fn new() -> Engine {
        Engine {
            game: Arc::new(Game::new()),
            worker: None,
        }
    }

    /// Shared access to the game (always available; search-facing state is atomic).
    #[inline(always)]
    pub fn game(&self) -> &Game {
        &self.game
    }

    /// Exclusive access to the game for the UCI handlers that mutate plain fields: stops any
    /// running search first, then proves exclusivity via Arc::get_mut.
    pub fn game_mut(&mut self) -> &mut Game {
        self.stop_thinking();
        Arc::get_mut(&mut self.game).expect("worker joined; no other Arc holders remain")
    }

    /// If currently thinking, stop immediately.
    /// Note: this does NOT clear is_pondering — handle_go sets it just before calling
    /// start_thinking (which calls stop_thinking), so clearing it here would clobber the flag for
    /// the search about to start. The ponder-wait in search_thread_entry exits on
    /// is_cancel_pending (set below), so a `stop` during a ponder still works.
    pub fn stop_thinking(&mut self) {
        self.game.is_cancel_pending.store(true, Ordering::Relaxed);
        if let Some(handle) = self.worker.take() {
            handle.join().expect("search worker panicked");
        }
    }

    /// Start thinking on the worker thread.
    pub fn start_thinking(&mut self) {
        self.stop_thinking();
        let game = Arc::clone(&self.game);
        self.worker = Some(std::thread::spawn(move || game.search_thread_entry()));
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop_thinking();
    }
}
