//! Per-thread search state for the parallel alpha-beta search. (Port of `search_state.h`.)

use crate::chess_clock::ClockType;
use crate::constants::*;
use crate::game::Game;
use crate::generated_data::ZobristT;
use crate::history_table::{history_bonus, HistoryTable};
use crate::moves::{sort_moves, Move, MoveList, MoveType, NodeType};
use crate::nnue;
use crate::position::Position;
use crate::static_exchange_evaluation::evaluate_static_exchange;
use crate::transposition_table::Transposition;
use crate::{increment, increment_if, uci_println};

/// Compact position hash entry for draw-by-repetition detection.
#[derive(Clone, Copy)]
pub struct HashEntry {
    pub hash: ZobristT,            // Zobrist hash of the position.
    pub reversible_move_count: u8, // Fifty-move (halfmove) clock at this position.
}

/// A variation (typically principal variation) is a line of play or series of moves.
pub type Variation = Vec<Move>;

/// When the PV changes we need to copy the new PV up the tree recursively.
pub fn copy_variation(dst: &mut Variation, src: &Variation, best_move: Move) {
    dst.clear();
    dst.push(best_move);
    dst.extend_from_slice(src);
}

/// Killer moves indexed by ply.
pub type KillerArray = [[Move; 2]; MAX_PLY];

/// Per-thread search state for the parallel alpha-beta search.
/// Each Lazy SMP thread owns an independent SearchState with its own position stack, hash
/// history, killers and history heuristic table, while sharing the transposition table and time
/// control through the Game reference. The history table is per-thread to avoid cross-thread
/// cache-line contention on its hot counters.
pub struct SearchState<'a> {
    // State variables
    pub game: &'a Game,           // Shared state: TT, clock, cancellation flag.
    pub history: HistoryTable,    // Per-thread history heuristic counts (no sharing).
    pub killers: KillerArray,     // Killer moves indexed by ply.
    pub node_count: u64,          // Cumulative nodes this thread searched.
    pub seldepth: i32,            // Max ply reached this search (selective depth, for `info`).
    pub positions: Vec<Position>, // Position stack.
    pub acc_stack: Vec<nnue::Accumulator>, // Per-ply NNUE accumulators (copy-make; see current_accumulator).
    pub acc_valid: Vec<bool>,              // acc_stack[i] is up to date iff acc_valid[i].
    pub hash_stack: Vec<HashEntry>,        // Draw by repetition stack.
    pub countermoves: [Move; CONT_KEYS],   // Best quiet countermove to prev move.
    pub continuation_history: Box<[i16]>,  // Continuation history scores.
}

impl<'a> SearchState<'a> {
    /// Construct a search state from the current game.
    /// `hash_stack` is populated with all ancestor positions (positions 0..N-1 where N is the
    /// current position), so that is_draw_by_repetition can walk backwards through the full game
    /// history.
    pub fn new(game: &'a Game) -> SearchState<'a> {
        // Initialise the killer and countermove tables to "no move" (Move::none() is Move{0} — a
        // value no legal move equals). The C++ documents an uninitialised-read bug here that once
        // made bench node counts differ across optimisation levels; Rust zero-init is that fix by
        // construction.
        let mut state = SearchState {
            game,
            history: HistoryTable::new(),
            killers: [[Move::none(); 2]; MAX_PLY],
            node_count: 0,
            seldepth: 0,
            positions: Vec::new(),
            acc_stack: Vec::new(),
            acc_valid: Vec::new(),
            hash_stack: Vec::new(),
            countermoves: [Move::none(); CONT_KEYS],
            continuation_history: vec![0i16; CONT_KEYS * CONT_KEYS].into_boxed_slice(),
        };
        let pos = game.positions.lock().unwrap();
        // Reserve the whole game history plus the deepest the search can descend, so no push
        // during the search reallocates (and an arbitrarily long game cannot overflow).
        state.hash_stack.reserve(pos.len() + MAX_PLY + 4);
        // Push all positions except the last (current) one into the hash history.
        for i in 0..pos.len().saturating_sub(1) {
            state.hash_stack.push(HashEntry {
                hash: pos[i].hash,
                reversible_move_count: pos[i].reversible_move_count,
            });
        }
        // Seed the per-thread position stack with only the current game position.
        // Reserve the deepest the search can descend so no push during search reallocates.
        state.positions.reserve(MAX_PLY + 4);
        state
            .positions
            .push(*pos.last().expect("game position history is never empty"));
        drop(pos);
        // Seed the per-ply NNUE accumulator stack; acc_stack[0] is the root accumulator.
        state.acc_stack.reserve(MAX_PLY + 4);
        state.acc_valid.reserve(MAX_PLY + 4);
        state.acc_stack.push(nnue::Accumulator::default());
        state.acc_valid.push(true);
        let root = state.positions[0];
        game.nnue_network.refresh(&mut state.acc_stack[0], &root);
        state
    }

    /// Current position at the tip of the search tree.
    #[inline(always)]
    pub fn current_position(&self) -> &Position {
        self.positions.last().unwrap()
    }

    /// Record a quiet move that caused a beta cutoff as a killer move for this ply.
    /// Shifts the previous first killer into the second slot; ignores duplicates.
    #[inline]
    pub fn record_killer(&mut self, ply: i32, mv: Move) {
        if self.killers[ply as usize][0] != mv {
            increment!("killer beta cutoffs");
            self.killers[ply as usize][1] = self.killers[ply as usize][0];
            self.killers[ply as usize][0] = mv;
        }
    }

    /// Continuation-history score for playing `mv` after `prev` (0 if prev/move not
    /// quiet-tracked). The table key is Move::piece_to() — a (piece, to-square) index;
    /// Move::none() maps to 0, a harmless slot.
    #[inline(always)]
    pub fn continuation_hist_score(&self, prev: Move, mv: Move) -> i32 {
        self.continuation_history[(prev.piece_to() as usize) * CONT_KEYS + mv.piece_to() as usize]
            as i32
    }

    /// Whether `mv` is the recorded countermove (best quiet refutation) to `prev`.
    #[inline(always)]
    pub fn is_countermove(&self, prev: Move, mv: Move) -> bool {
        self.countermoves[prev.piece_to() as usize] == mv
    }

    /// Credit (bonus > 0) or debit (bonus < 0) the 1-ply continuation-history entry for playing `mv`
    /// after `prev`, with the same gravity as the butterfly table. Non-quiet moves are skipped
    /// (captures order by SEE). The i16 entry is loaded into an i32, gravity-updated, then stored
    /// back — the gravity bound (+/- HISTORY_MAX) keeps it within i16 range.
    #[inline]
    pub fn apply_continuation_history_bonus(&mut self, prev: Move, mv: Move, bonus: i32) {
        if !mv.is_quiet() {
            return;
        }
        let idx = (prev.piece_to() as usize) * CONT_KEYS + mv.piece_to() as usize;
        let mut e = self.continuation_history[idx] as i32;
        crate::history_table::apply_gravity(&mut e, bonus);
        self.continuation_history[idx] = e as i16;
    }

    /// Credit `mv` (which raised alpha, or caused a beta cutoff when `is_cutoff`) into the
    /// move-ordering heuristics with a depth-weighted gravity bonus: the butterfly history and
    /// continuation history for `mv`. On a beta cutoff it additionally (a) debits every other quiet
    /// move that was searched at this node but failed (the malus, over `quiets_tried`) and (b) makes
    /// a quiet `mv` the killer at `ply` and the countermove to `prev`. Ported verbatim from the Go
    /// engine's recordGood.
    pub fn record_good(
        &mut self,
        ply: i32,
        depth: i32,
        prev: Move,
        mv: Move,
        is_cutoff: bool,
        quiets_tried: &[Move],
    ) {
        let c = self.current_position().color_to_move;
        let bonus = history_bonus(depth);
        self.history.apply_bonus(c, mv, bonus);
        self.apply_continuation_history_bonus(prev, mv, bonus);
        if is_cutoff {
            for &quiet in quiets_tried {
                if quiet != mv {
                    // identity compare (Move == masks score bits), so the cutoff move is skipped
                    self.history.apply_bonus(c, quiet, -bonus);
                    self.apply_continuation_history_bonus(prev, quiet, -bonus);
                }
            }
            if mv.is_quiet() {
                self.record_killer(ply, mv);
                self.record_countermove(prev, mv);
            }
        }
    }

    /// Record a quiet `mv` that caused a beta cutoff as the countermove to `prev`.
    #[inline]
    pub fn record_countermove(&mut self, prev: Move, mv: Move) {
        if mv.is_quiet() {
            self.countermoves[prev.piece_to() as usize] = mv;
        }
    }

    /// Push the current position's hash then make the move on a copy of the position.
    /// The NNUE accumulator is NOT advanced here — it is brought current lazily by
    /// current_accumulator the next time an evaluation actually needs it, so a node that cuts off
    /// before evaluating pays no accumulator cost.
    pub fn play_move(&mut self, mv: Move) {
        let cur = *self.current_position();
        self.hash_stack.push(HashEntry {
            hash: cur.hash,
            reversible_move_count: cur.reversible_move_count,
        });
        self.positions.push(cur.make_move(mv));
        let d = self.positions.len() - 1; // this ply's accumulator is now stale
        while self.acc_valid.len() <= d {
            self.acc_stack.push(nnue::Accumulator::default());
            self.acc_valid.push(false);
        }
        self.acc_valid[d] = false;
    }

    /// Pop the position and hash stacks to undo the last move.
    /// Copy-make: undo does no accumulator work — the ancestor accumulators in acc_stack stay
    /// valid, and this ply's slot is re-invalidated by the next play_move into it.
    pub fn undo_move(&mut self) {
        self.positions.pop();
        self.hash_stack.pop();
    }

    /// Push a null move (skip this side's turn) used for null-move pruning.
    /// Piece placement is unchanged by a null move, so the accumulator needs no update.
    pub fn make_null_move(&mut self) {
        let cur = *self.current_position();
        self.hash_stack.push(HashEntry {
            hash: cur.hash,
            reversible_move_count: cur.reversible_move_count,
        });
        self.positions.push(cur.make_null_move());
        let d = self.positions.len() - 1; // this ply's accumulator is stale (null update is a no-op)
        while self.acc_valid.len() <= d {
            self.acc_stack.push(nnue::Accumulator::default());
            self.acc_valid.push(false);
        }
        self.acc_valid[d] = false;
    }

    /// The accumulator for the current tip position, brought current by applying any feature
    /// updates deferred since the last evaluation (advancing one ply at a time through the
    /// position stack; null-move plies apply a no-op update since placements are unchanged).
    /// Bit-identical to eager maintenance — the same deltas in the same order — but nodes that
    /// never evaluate (e.g. TT cutoffs) skip the work entirely.
    /// (The C++ member is const with `mutable` stacks; Rust expresses the same as `&mut self`.)
    pub fn current_accumulator(&mut self) -> &nnue::Accumulator {
        let d = self.positions.len() - 1;
        while self.acc_stack.len() <= d {
            self.acc_stack.push(nnue::Accumulator::default());
            self.acc_valid.push(false);
        }
        let mut k = d; // walk back to the deepest valid ancestor (acc_valid[0] is seeded true)
        while k > 0 && !self.acc_valid[k] {
            k -= 1;
        }
        for j in k + 1..=d {
            // copy the parent accumulator, then apply this ply's feature delta
            let (parents, tail) = self.acc_stack.split_at_mut(j);
            tail[0] = parents[j - 1];
            self.game
                .nnue_network
                .update(&mut tail[0], &self.positions[j - 1], &self.positions[j]);
            self.acc_valid[j] = true;
        }
        &self.acc_stack[d]
    }

    /// Score moves for ordering, then sort descending.
    /// The score field is a signed 23-bit value shared with the stored TT score. Move ordering
    /// uses, from best to worst:
    ///   - winning / equal captures and promotions: WINNING_CAPTURE_BASE + SEE score,
    ///   - the two killer moves for this ply: just below winning captures, above all quiet moves,
    ///   - the countermove: just below the killers,
    ///   - quiet moves: their history count, clamped below the countermove,
    ///   - losing captures (negative SEE): scored by their (negative) SEE, below quiet moves.
    pub fn score_and_sort_moves(&self, moves: &mut MoveList, ply: i32, prev_move: Move) {
        const WINNING_CAPTURE_BASE: i32 = 1 << 20; // 1,048,576: winning captures / promotions
        const KILLER_BASE: i32 = 1 << 19; // 524,288: killers sit just below winning captures
        const COUNTERMOVE_SCORE: i32 = KILLER_BASE - 1; // countermove: just below the two killers
        const MAX_QUIET: i32 = KILLER_BASE - 2; // history scores clamp just below the countermove
        let position = self.current_position();
        let killer0 = self.killers[ply as usize][0];
        let killer1 = self.killers[ply as usize][1];
        for mv in moves.as_mut_slice() {
            let sort: i32;
            if !mv.is_quiet() {
                // capture or promotion: order by static exchange evaluation
                let (see, is_checking) = evaluate_static_exchange(position, *mv);
                if is_checking {
                    mv.gives_check();
                }
                sort = if see >= 0 {
                    WINNING_CAPTURE_BASE + see
                } else {
                    see
                }; // losing captures sort below quiets
            } else if *mv == killer0 {
                // first killer: just below winning captures
                sort = KILLER_BASE + 1;
            } else if *mv == killer1 {
                // second killer
                sort = KILLER_BASE;
            } else if self.is_countermove(prev_move, *mv) {
                // countermove: best quiet refutation to the previous move
                sort = COUNTERMOVE_SCORE;
            } else {
                // quiet move: main history + 1-ply continuation history, clamped below countermove.
                // History is SIGNED (the malus drives entries negative); clamp only the HIGH side so
                // negative scores pass through — an unsigned clamp/wrap here would be catastrophic.
                let h = self.history.get_count(position.color_to_move, *mv)
                    + self.continuation_hist_score(prev_move, *mv);
                sort = h.min(MAX_QUIET);
            }
            mv.assign_score(sort);
        }
        sort_moves(moves);
    }

    /// Check the fifty-move clock: 100 plies or more since the last pawn move or capture.
    /// The clock alone - a checkmate on the hundredth ply still stands, which `search` tests for
    /// before it draws. The static eval's own use of it decides no mate: in the search every
    /// position it scores has passed `search`'s tests first, or is a capture's child with the clock
    /// at 0, and it scores a quiet position not in check.
    #[inline(always)]
    pub fn is_draw_by_fifty_moves(&self) -> bool {
        self.current_position().reversible_move_count >= 100
    }

    /// Check for three-fold repetition using the compact hash history.
    /// Walks backwards through hash_stack two entries at a time (same side to move), starting
    /// four half-moves before the current position, and stops at a fifty-move clock of zero (a
    /// capture or pawn move), before which no position can recur. Castling does not zero the
    /// clock, so the walk carries on past it; the castling rights it gave up are in the hash, so
    /// the positions before it never match.
    pub fn is_draw_by_repetition(&self) -> bool {
        let hash = self.current_position().hash;
        let mut repetitions = 2;
        let mut i = self.hash_stack.len() as i32 - 4;
        while i >= 0 {
            if self.hash_stack[i as usize].hash == hash {
                repetitions -= 1;
                if repetitions == 0 {
                    increment!("draws by repetition");
                    return true;
                }
            }
            if self.hash_stack[i as usize].reversible_move_count == 0 {
                break;
            }
            i -= 2;
        }
        false
    }

    /// Check whether this thread should abort (the shared stop flag is set).
    #[inline(always)]
    pub fn is_cancelled(&self) -> bool {
        self.game
            .is_cancel_pending
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    // ------------------------------------------------------------------------
    // Alpha-beta and quiescence search.
    // ------------------------------------------------------------------------

    /// Search a single move from the current node.
    /// Returns the move carrying its alpha-beta score (score()) and whether it gives check
    /// (is_checking()).
    pub fn search_single_move(
        &mut self,
        depth: i32,
        ply: i32,
        alpha: i32,
        beta: i32,
        mut mv: Move,
        pv: &mut Variation,
        move_index: i32,
    ) -> Move {
        let was_in_check = self.current_position().is_in_check(); // check extensions are added in search()

        let mut child_depth = depth - 1;
        match mv.type_() {
            MoveType::PromotionKnight
            | MoveType::PromotionBishop
            | MoveType::PromotionRook
            | MoveType::PromotionQueen => {
                child_depth += 1;
                increment!("promotion extensions");
            }
            MoveType::EpCapture => {
                child_depth += 1;
                increment!("ep capture extensions");
            }
            _ => {}
        }
        self.play_move(mv);
        // Prefetch the child's TT cell now; the recursive search probes it after its own
        // prologue, by which time the (random-access, cache-missing) line should be resident.
        self.game
            .transposition_table
            .prefetch(self.current_position().hash);
        let is_checking = self.current_position().is_in_check();
        let score: i32;
        if beta > alpha + 1 && move_index > 0 && !was_in_check && !is_checking {
            // Try principal variation search.
            increment!("pvs attempts");
            let mut s = -self.search(child_depth, ply + 1, -alpha - 1, -alpha, pv, mv);
            if s > alpha {
                increment!("pvs fails");
                increment_if!(ply == 0, "pvs fails root ply");
                s = -self.search(child_depth, ply + 1, -beta, -alpha, pv, mv);
            }
            score = s;
        } else {
            score = -self.search(child_depth, ply + 1, -beta, -alpha, pv, mv);
        }
        self.undo_move();
        // Return the move carrying its search score and check status.
        mv.assign_score(score);
        if is_checking {
            mv.gives_check();
        }
        mv
    }

    /// Try null move pruning, reusing the caller's precomputed static eval.
    /// The caller has already established this is a non-PV node that is not in check.
    /// Returns the null-move score, or alpha if null move was not tried / did not cut.
    fn attempt_null_move(
        &mut self,
        depth: i32,
        ply: i32,
        alpha: i32,
        beta: i32,
        eval_score: i32,
    ) -> i32 {
        let position = self.current_position();
        let color = position.color_to_move;
        // Only try null move pruning if all conditions are met.
        let friendly = position.colors[color.index()];
        if !position.is_null_move && // previous move was not a null move
            friendly.pop_count() > 3 && // we have at least 4 friendly pieces
            eval_score >= beta
        // and we are already failing high statically
        {
            increment!("null move attempts");
            let mut dummy: Variation = Vec::new();
            self.make_null_move();
            let score = -self.search(depth - 4, ply + 1, -beta, -alpha, &mut dummy, Move::none());
            self.undo_move();
            if self.is_cancelled() {
                return SEARCH_CANCELLED_SCORE;
            }
            if score >= beta {
                return beta;
            }
            increment!("null move fails");
        }
        alpha
    }

    /// Fail-soft negamax alpha-beta search of the current node.
    /// `depth`: depth to search; `ply`: distance from root; `parent_pv`: principal variation at
    /// the parent node; `prev_move`: the move played at the parent to reach this node
    /// (Move::none() at root / after null).
    pub fn search(
        &mut self,
        mut depth: i32,
        ply: i32,
        mut alpha: i32,
        beta: i32,
        parent_pv: &mut Variation,
        prev_move: Move,
    ) -> i32 {
        increment!("alpha beta calls");
        self.seldepth = self.seldepth.max(ply); // track selective depth for `info seldepth`
                                                // Poll the hard deadline (and the `go nodes` limit) often — every 2048 nodes: between
                                                // polls the search cannot stop, so a coarse interval lets a node-heavy batch overrun the
                                                // deadline. The node limit (max_nodes, 0 = none) is checked per-thread, so it is exact at
                                                // Threads=1 (the reproducible-testing case) and approximate above.
        self.node_count += 1;
        if (self.node_count & 0x7FF) == 0
            && (self.game.time_control.has_reached_hard_deadline()
                || (self.game.time_control.max_nodes != 0
                    && self.node_count >= self.game.time_control.max_nodes))
        {
            self.game
                .is_cancel_pending
                .store(true, std::sync::atomic::Ordering::Relaxed);
            return SEARCH_CANCELLED_SCORE;
        }
        if self.current_position().is_draw_by_material() {
            return DRAW_SCORE;
        }
        if self.is_draw_by_fifty_moves() {
            // Checkmate takes precedence over the fifty-move draw (FIDE 5.1.1; 9.6.2 says so of the
            // 75-move rule): a mate delivered on the move that completes the fifty stands. So in check
            // the clock draws only when the side to move has a legal move; with none it is mated,
            // scored here at this ply as the move loop's own checkmate is. Out of check there can be
            // no mate, and the clock draws whatever the moves. Mirrors the C++ Search.
            if self.current_position().is_in_check()
                && self.current_position().generate_legal_moves().is_empty()
            {
                increment!("checkmates");
                return CHECKMATED_SCORE + ply;
            }
            return DRAW_SCORE;
        }
        if self.is_draw_by_repetition() {
            return DRAW_SCORE;
        }
        if self.current_position().is_in_check() {
            increment!("check extensions");
            depth += 1;
        }
        if ply == MAX_PLY as i32 {
            increment!("max ply reached");
            return evaluate_position(self);
        }
        // Determine if there is an entry in the transposition table for this position.
        let transposition = self
            .game
            .transposition_table
            .find_transposition(self.current_position().hash);
        if let Some(t) = &transposition {
            if t.depth() >= depth {
                match t.node_type() {
                    NodeType::Cut => {
                        increment!("table hit cut node");
                        if t.score() >= beta {
                            increment!("table hit cut node cutoffs");
                            return t.score();
                        }
                    }
                    NodeType::All => {
                        increment!("table hit all node");
                        if t.score() <= alpha {
                            increment!("table hit all node cutoffs");
                            return t.score();
                        }
                    }
                    NodeType::Pv => {
                        increment!("table hit pv node");
                        depth += 1; // extend depth if we think we are on the PV
                    }
                }
            }
        }
        // Internal iterative reduction: with no usable transposition-table move our move ordering
        // is weak. Rather than spend the full depth with poor ordering, reduce by one ply — the
        // shallower search still fills the TT with a best move, so the cost is recouped by better
        // ordering when this node is revisited.
        if depth >= NO_TT_REDUCE_MIN_DEPTH
            && transposition
                .as_ref()
                .map_or(true, |t| t.mv == Move::none())
        {
            increment!("internal iterative reductions");
            depth -= 1;
        }
        // Can we go directly to the quiescence search?
        if depth <= 0 && !self.current_position().is_in_check() {
            return self.search_quiescent(depth, ply, alpha, beta);
        }
        // Static-eval-based pruning at non-PV nodes that are not in check. The static eval is
        // computed once here and reused by reverse-futility pruning and null-move pruning.
        let is_pv = beta > alpha + 1;
        if !is_pv && !self.current_position().is_in_check() {
            let eval_score = evaluate_position(self);
            // Reverse futility pruning (a.k.a. static null move): at shallow depth, if we are far
            // enough above beta that even conceding RFP_MARGIN per ply still leaves us at/above
            // beta, assume a quiet move holds the advantage and cut. Disabled in the mate zone so
            // it cannot mask a forced mate.
            if depth <= RFP_MAX_DEPTH
                && beta.abs() < -CHECKMATED_SCORE - MAX_PLY as i32
                && eval_score - RFP_MARGIN * depth >= beta
            {
                increment!("reverse futility prunes");
                return eval_score;
            }
            // Null move pruning, reusing the static eval.
            let null_move_score = self.attempt_null_move(depth, ply, alpha, beta, eval_score);
            if self.is_cancelled() {
                return SEARCH_CANCELLED_SCORE;
            }
            if null_move_score >= beta {
                return null_move_score;
            }
        }

        // Try the transposition table move first.
        let mut pv: Variation = Vec::new();
        let mut best_move = Move::none();
        let mut best_score = ALPHA;
        let mut has_raised_alpha = false;

        // quiets_tried collects the quiet moves searched at this node (TT move included) so a later
        // beta cutoff can penalise the ones that failed (the history malus in record_good). Backed by
        // a stack array to avoid a per-node heap allocation; capped at 64 (deeper is rare and the tail
        // matters least). The cutoff move is in the list and is skipped inside record_good.
        let mut quiets_buf = [Move::none(); 64];
        let mut quiets_len: usize = 0;

        if let Some(t) = &transposition {
            if t.mv != Move::none() {
                increment!("table move");
                best_move = t.mv;
                pv.clear(); // child writes its line here only if it is a PV node
                let score = self
                    .search_single_move(depth, ply, alpha, beta, t.mv, &mut pv, 0)
                    .score();
                if self.is_cancelled() {
                    return SEARCH_CANCELLED_SCORE;
                }
                if score >= beta {
                    increment!("table move beta cutoffs");
                    self.game
                        .transposition_table
                        .record_transposition(&Transposition::with_data(
                            self.current_position().hash,
                            t.mv,
                            score,
                            depth,
                            NodeType::Cut,
                        ));
                    self.record_good(ply, depth, prev_move, t.mv, true, &quiets_buf[..quiets_len]);
                    return score;
                }
                if t.mv.is_quiet() && quiets_len < 64 {
                    quiets_buf[quiets_len] = t.mv;
                    quiets_len += 1;
                }
                best_score = score;
                if score > alpha {
                    increment!("table move raised alpha");
                    alpha = score;
                    has_raised_alpha = true;
                    copy_variation(parent_pv, &pv, t.mv); // capture the PV now, while pv holds THIS move's line
                    self.record_good(
                        ply,
                        depth,
                        prev_move,
                        t.mv,
                        false,
                        &quiets_buf[..quiets_len],
                    );
                }
            }
        }

        let mut move_list = self.current_position().generate_legal_moves();
        if move_list.size() == 0 {
            if self.current_position().is_in_check() {
                increment!("checkmates");
                return CHECKMATED_SCORE + ply;
            }
            increment!("stalemates");
            return DRAW_SCORE;
        }
        self.score_and_sort_moves(&mut move_list, ply, prev_move);

        // Search the moves one at a time. Parallelism comes from Lazy SMP (independent whole-tree
        // searches sharing the transposition table), not from splitting this node.
        let in_check_seq = self.current_position().is_in_check();

        for move_index in 0..move_list.size() {
            let mv = move_list[move_index];
            if let Some(t) = &transposition {
                if mv == t.mv {
                    continue; // we already searched that one.
                }
            }

            // Late move pruning (move-count pruning): at shallow depth in a non-PV node not in
            // check, once we are deep into the (history/SEE-sorted) move list, skip the remaining
            // quiet, non-checking moves — they almost never raise alpha. Captures, promotions and
            // checking moves are never pruned.
            if beta == alpha + 1
                && !in_check_seq
                && depth <= LMP_MAX_DEPTH
                && mv.is_quiet()
                && !mv.is_checking()
                && move_index as i32 >= LMP_BASE + depth * depth
            {
                increment!("late move prunes");
                continue;
            }

            // Late move reduction: reduce late moves outside a check sequence at null-window nodes.
            let mut lmr_depth = depth;
            if beta == alpha + 1 && move_index > 3 && !in_check_seq && depth > 2 {
                increment!("late move reduction 1");
                lmr_depth = depth - 1;
                if depth > 3 && move_index > 6 {
                    lmr_depth -= 1;
                    increment!("late move reduction 2");
                    if move_index > 12 {
                        lmr_depth -= 1;
                        increment!("late move reduction 3");
                    }
                }
            }

            // Record this quiet move as tried before searching it, so a cutoff by a later sibling
            // penalises it (the eventual cutoff move is excluded inside record_good).
            if mv.is_quiet() && quiets_len < 64 {
                quiets_buf[quiets_len] = mv;
                quiets_len += 1;
            }

            pv.clear(); // see TT-move note: clear so a non-PV-node child leaves no stale tail in pv
            let mut score = self
                .search_single_move(lmr_depth, ply, alpha, beta, mv, &mut pv, move_index as i32)
                .score();
            if score > alpha && lmr_depth < depth {
                increment!("late move reduction fails");
                score = self
                    .search_single_move(depth, ply, alpha, beta, mv, &mut pv, move_index as i32)
                    .score();
            }
            if self.is_cancelled() {
                return SEARCH_CANCELLED_SCORE;
            }
            if score >= beta {
                increment!("beta cutoffs");
                self.game
                    .transposition_table
                    .record_transposition(&Transposition::with_data(
                        self.current_position().hash,
                        mv,
                        score,
                        depth,
                        NodeType::Cut,
                    ));
                self.record_good(ply, depth, prev_move, mv, true, &quiets_buf[..quiets_len]);
                return score;
            }
            if score > best_score {
                increment!("best score changed");
                best_score = score;
                best_move = mv;
                if score > alpha {
                    increment!("pv changed");
                    alpha = score;
                    has_raised_alpha = true;
                    copy_variation(parent_pv, &pv, mv); // capture the PV now, while pv holds THIS move's line
                    self.record_good(ply, depth, prev_move, mv, false, &quiets_buf[..quiets_len]);
                }
            }
        }

        // ── End of main loop ─────────────────────────────────────────────────
        if has_raised_alpha {
            // Raised alpha without a cutoff: PV node.
            increment!("pv nodes");
            self.game
                .transposition_table
                .record_transposition(&Transposition::with_data(
                    self.current_position().hash,
                    best_move,
                    alpha,
                    depth,
                    NodeType::Pv,
                ));
            self.record_good(
                ply,
                depth,
                prev_move,
                best_move,
                false,
                &quiets_buf[..quiets_len],
            );
            // parent_pv was already set, with the correct line, at the alpha-raise above.
        } else {
            // Searched all moves without raising alpha: all-node.
            increment!("all nodes");
            self.game
                .transposition_table
                .record_transposition(&Transposition::with_data(
                    self.current_position().hash,
                    best_move,
                    best_score,
                    depth,
                    NodeType::All,
                ));
        }
        best_score
    }

    /// Quiescence search over captures from the current node.
    pub fn search_quiescent(&mut self, depth: i32, ply: i32, mut alpha: i32, beta: i32) -> i32 {
        increment!("quiescent calls");
        self.seldepth = self.seldepth.max(ply); // track selective depth for `info seldepth`
        if ply == MAX_PLY as i32 {
            increment!("quiescent max ply");
            return evaluate_position(self);
        }
        if self.current_position().is_in_check() {
            // We can't handle checks in quiescence.
            increment!("quiescent checks");
            let mut dummy: Variation = Vec::new();
            return self.search(depth, ply, alpha, beta, &mut dummy, Move::none());
        }
        let mut score = evaluate_position(self);
        if score >= beta {
            increment!("quiescent eval beta cutoffs");
            return score;
        }
        if score > alpha {
            increment!("quiescent eval raises alpha");
            alpha = score;
        }
        let mut best_score = score;
        let mut move_list = self.current_position().generate_legal_captures();
        self.score_and_sort_moves(&mut move_list, ply, Move::none());
        for move_index in 0..move_list.size() {
            let mv = move_list[move_index];
            // SEE pruning: skip captures that lose material.
            if mv.score() < 0 {
                increment!("quiescent see prunes");
                return best_score; // All moves after this are -ve SEE and may be skipped.
            }
            self.play_move(mv);
            score = -self.search_quiescent(depth - 1, ply + 1, -beta, -alpha);
            self.undo_move();
            if self.is_cancelled() {
                return SEARCH_CANCELLED_SCORE;
            }
            if score >= beta {
                increment!("quiescent beta cutoffs");
                let color = self.current_position().color_to_move;
                self.history.record_good_move(color, mv);
                return score;
            }
            if score > best_score {
                best_score = score;
                if score > alpha {
                    alpha = score;
                    increment!("quiescent pv changed");
                    let color = self.current_position().color_to_move;
                    self.history.record_good_move(color, mv);
                }
            }
        }
        increment!("quiescent all nodes");
        best_score
    }

    // ------------------------------------------------------------------------
    // Iterative deepening — one Lazy SMP thread. Game::search_root_node launches one per thread.
    // ------------------------------------------------------------------------

    /// Run iterative deepening on this search state — one Lazy SMP thread.
    /// The main thread (`thread_id` 0) prints info, applies time management and produces the
    /// authoritative move. `move_list` is this thread's copy of the root move list (re-sorted per
    /// iteration); `best_move` is the seed from the shallow ordering pass; `thread_id` drives the
    /// iteration-skip schedule.
    pub fn iterative_deepen(
        &mut self,
        mut move_list: MoveList,
        mut best_move: Move,
        thread_id: i32,
    ) -> Move {
        let is_main = thread_id == 0; // thread 0 is the authoritative main thread
                                      // Per-thread iteration-skip schedule (Stockfish-style): helper threads skip certain
                                      // iteration depths so their searches desynchronize from the main thread and from each
                                      // other, prefilling the shared TT with diverse entries instead of recomputing the same
                                      // tree in lockstep. The main thread never skips.
        const SKIP_SIZE: [i32; 20] = [1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4];
        const SKIP_PHASE: [i32; 20] = [0, 1, 0, 1, 2, 3, 0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, 6, 7];
        const SKIP_ENTRIES: i32 = SKIP_SIZE.len() as i32;

        let mut principal_variation: Variation = Vec::new();
        let mut best_moves: Vec<Move> = Vec::new(); // Best move at each depth (main thread, stability checks).
        best_moves.reserve(MAX_PLY);

        let mut depth = START_DEPTH + 1;
        while depth != MAX_PLY as i32 {
            if self.game.time_control.clock_type == ClockType::FixedDepth
                && depth > self.game.time_control.depth
            {
                break;
            }
            if !is_main {
                let i = ((thread_id - 1) % SKIP_ENTRIES) as usize;
                if ((depth + SKIP_PHASE[i]) / SKIP_SIZE[i]) % 2 != 0 {
                    depth += 1;
                    continue; // this helper thread skips this iteration depth
                }
            }
            sort_moves(&mut move_list); // Sort based on scores from the previous iteration.
            let mut child_pv: Variation = Vec::new();
            let mut alpha = ALPHA;
            if is_main {
                best_moves.push(best_move);
            }
            for move_index in 0..move_list.size() {
                let mv = move_list[move_index];
                // Report the move currently being searched at the root, but only once a search is
                // slow enough that a GUI benefits.
                if is_main && self.game.time_control.elapsed_since_search_start() > 3000 {
                    uci_println!(
                        "info depth {} currmove {} currmovenumber {}",
                        depth,
                        mv.to_string(),
                        move_index + 1
                    );
                }
                child_pv.clear(); // clear per move so a terminal/non-PV-node child leaves no stale tail
                let score = self
                    .search_single_move(depth, 0, alpha, BETA, mv, &mut child_pv, move_index as i32)
                    .score();
                move_list[move_index].assign_score(score);
                if self.is_cancelled() {
                    return best_move;
                }
                if score > alpha {
                    alpha = score;
                    best_move = move_list[move_index];
                    if is_main {
                        // Show thinking output.
                        copy_variation(&mut principal_variation, &child_pv, best_move);
                        // Publish the predicted reply (PV[1]) as it improves, so `bestmove <m>
                        // ponder <reply>` has it on every exit path.
                        let ponder = if principal_variation.len() >= 2 {
                            principal_variation[1]
                        } else {
                            Move::none()
                        };
                        self.game
                            .ponder_move
                            .store(ponder.bits(), std::sync::atomic::Ordering::Relaxed);
                        let mut pv_string = String::new();
                        for m in &principal_variation {
                            pv_string.push_str(&m.to_string());
                            pv_string.push(' ');
                        }
                        pv_string.pop();
                        // Report a forced mate as `score mate <moves>` (positive = we mate,
                        // negative = we get mated) rather than a huge `cp` value. Mate scores are
                        // +/-(MATE_VALUE - plies_to_mate), so plies_to_mate = MATE_VALUE - |score|
                        // and the move count rounds up (+1)/2.
                        const MATE_VALUE: i32 = -CHECKMATED_SCORE; // 31900
                        let score_string = if alpha >= WIN_THRESHOLD {
                            format!("mate {:2}", (MATE_VALUE - alpha + 1) / 2)
                        } else if alpha <= LOSE_THRESHOLD {
                            format!("mate {:2}", -((MATE_VALUE + alpha + 1) / 2))
                        } else {
                            format!("cp {:4}", alpha)
                        };
                        let elapsed_ms = self.game.time_control.elapsed_since_search_start();
                        let nps = (self.node_count as i64) * 1000 / elapsed_ms.max(1);
                        uci_println!(
                            "info depth {:2} seldepth {:2} score {} time {:5} nodes {:8} nps {:7} hashfull {:3} pv {}",
                            depth,
                            self.seldepth,
                            score_string,
                            elapsed_ms,
                            self.node_count,
                            nps,
                            self.game.transposition_table.hashfull_permille(),
                            pv_string
                        );
                    }
                }
            }
            if alpha > WIN_THRESHOLD || alpha < LOSE_THRESHOLD {
                break;
            }
            // Soft time management: never stop while pondering (no real clock is running yet);
            // once `ponderhit` has reset the search-start instant, elapsed is measured from the
            // ponderhit moment.
            if is_main
                && self.game.time_control.clock_type == ClockType::Standard
                && !self
                    .game
                    .is_pondering
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                let allocated = self.game.time_control.allocated_time();
                let elapsed = self.game.time_control.elapsed_since_search_start();
                let is_best_move_consistent = best_moves.iter().all(|m| *m == best_move);
                let is_score_stable = best_moves
                    .iter()
                    .all(|m| (m.score() - best_move.score()).abs() <= SCORE_INSTABILITY_THRESHOLD);
                if (is_best_move_consistent && is_score_stable) && (elapsed * 4) > allocated {
                    uci_println!(
                        "info string Best move consistent AND score stable - search terminated."
                    );
                    break;
                }
                if (is_best_move_consistent || is_score_stable) && (elapsed * 2) > allocated {
                    uci_println!(
                        "info string {} - search terminated.",
                        if is_best_move_consistent {
                            "Best move consistent"
                        } else {
                            "Score stable"
                        }
                    );
                    break;
                }
                if elapsed > allocated {
                    break;
                }
            }
            depth += 1;
        }
        best_move
    }
}

// ---- evaluate_position (the C++ EvaluatePosition, declared in evaluation.h) ----

/// Plies the fifty-move clock may reach before the static eval is discounted. Below this the eval
/// is untouched, so normal play (where the clock is reset often by pawn moves/captures) is
/// unaffected.
pub const RULE50_SCALE_THRESHOLD: i32 = 20;

/// Scale the static eval toward draw as the fifty-move clock climbs, so the search has a standing
/// incentive to make progress (reset the clock) rather than shuffle in a won-but-not-yet-converted
/// position. NNUE inputs are piece placements only, so without this a winning eval is identical at
/// clock 0 and clock 99. Identity for clock <= RULE50_SCALE_THRESHOLD; then a linear ramp to 0 as
/// the clock approaches 100 (the hard draw, handled by the is_draw_by_fifty_moves short-circuit).
pub fn scale_by_rule50(score: i32, rule50: i32) -> i32 {
    if rule50 <= RULE50_SCALE_THRESHOLD {
        return score;
    }
    // num falls 80..1 over rule50 21..99; den is the ramp length. A progress move (clock reset to
    // 0) keeps the full eval, a shuffle (clock +1) keeps less — the difference is the gradient
    // that drives progress.
    let num = 100 - rule50;
    let den = 100 - RULE50_SCALE_THRESHOLD;
    ((score as i64) * (num as i64) / (den as i64)) as i32
}

/// Evaluate the current position, assuming neither king is in check and the position is quiet.
/// NNUE is the only evaluator: after the draw short-circuit this returns the network's score for
/// the (incrementally maintained) accumulator, memoised by Zobrist hash in the net-specific eval
/// cache, then scaled toward draw by the fifty-move clock (see scale_by_rule50).
/// (The C++ takes `const SearchState &` with `mutable` accumulator stacks; Rust expresses the
/// same lazily-updated state as `&mut`.)
pub fn evaluate_position(state: &mut SearchState) -> i32 {
    increment!("eval calls");
    if state.current_position().is_draw_by_material()
        || state.is_draw_by_fifty_moves()
        || state.is_draw_by_repetition()
    {
        return DRAW_SCORE;
    }
    // Memoise the (net-specific) NNUE eval by Zobrist hash: a cache hit skips the forward pass.
    // The accumulator is maintained incrementally by SearchState make/undo; the cache only saves
    // the dot-product. The cache is cleared whenever the net changes (SetPosition / EvalFile load).
    //
    // The cached value is the RAW, clock-independent NNUE eval: the Zobrist hash excludes the
    // fifty-move clock, so the same position can be probed at different clock values. The
    // fifty-move scaling is therefore applied AFTER the cache, on the way out — never stored.
    let (hash, reversible_move_count, color_to_move) = {
        let p = state.current_position();
        (p.hash, p.reversible_move_count, p.color_to_move)
    };
    if let Some(raw) = state.game.eval_cache.probe(hash) {
        increment!("eval hash hits");
        return scale_by_rule50(raw, reversible_move_count as i32);
    }
    let game = state.game; // copy the shared reference out so the accumulator borrow below is independent
    let raw = game
        .nnue_network
        .evaluate(state.current_accumulator(), color_to_move);
    game.eval_cache.store(hash, raw);
    scale_by_rule50(raw, reversible_move_count as i32)
}
