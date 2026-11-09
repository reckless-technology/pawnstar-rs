//! A cancel that lands inside the root's shallow ordering pass.
//!
//! Before iterative deepening, `search_root_node` gives every root move a depth-3 score. That loop
//! once never looked at the cancel flag: when the hard deadline, `stop` or a node limit landed in
//! it, the move being searched came back with the cancel sentinel, which negated and truncated to
//! the 23-bit score field reads as about +3957070. That move sorted first and was played, with no
//! `info` line. In 4,000 games at 8+0.08 it happened 67 times, and 10 of those moves walked into
//! mate - as on the FEN below, where 2.2.0 answered `go wtime 664 btime 1110 winc 80 binc 80` with
//! b2b1r in about 80 ms (White mates in 5) where `go depth 4` prefers h8g7.
//!
//! The move whose search was interrupted and every move after it are now unsearched and rank below
//! every searched move. The table's move for the root is played if it is a root move and its entry
//! is deeper than the pass (the pass can re-enter and store the root through a repetition, but
//! such entries are shallower than the pass, so the test never prefers them; one can overwrite a
//! deeper entry, and the rule then falls back to the best finished move); otherwise the best
//! finished move; with none finished, the table's move at any depth (if it is a root move) or else
//! the first generated move. This suite replays the ordering pass itself, move by move on a fresh
//! game, to learn which moves finished under each node limit and what they scored, then holds
//! `search_root_node` to the move that rule picks. A node limit is the deterministic
//! cancel: the search polls it every 2048 nodes, so at Threads 1 it lands at the same node every
//! time.
//!
//! Skipped (passes) when the shipped net is absent or not of this build's shape: which moves finish
//! under a node limit depends on the evaluation.

mod common;

use std::sync::atomic::Ordering;

use pawnstar::chess_clock::ClockType;
use pawnstar::constants::{ALPHA, BETA, LIMITED_ORDERING_DEPTH, START_DEPTH};
use pawnstar::game::{Game, START_FEN};
use pawnstar::moves::{sort_moves, Move, NodeType};
use pawnstar::position::Position;
use pawnstar::search_state::{SearchState, Variation};
use pawnstar::transposition_table::Transposition;

/// The position of the 67 games: Black to move, b2 one step from promoting.
const FEN: &str = "3k3b/3P1KP1/8/5B2/7p/7P/1p6/8 b - - 0 92";

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

/// A fresh single-threaded game on FEN, book off, the shipped net loaded, searching under a node
/// limit and no clock - `go nodes <max_nodes>` as `handle_go` sets it.
fn fresh_game(max_nodes: u64, thread_count: i32) -> Box<Game> {
    let mut game = Box::new(Game::new());
    check(
        game.nnue_network
            .load(&common::shipped_net_path())
            .is_loaded(),
        "the shipped net loads",
    );
    game.thread_count = thread_count;
    game.use_own_book = false;
    game.set_position(FEN);
    game.time_control.clock_type = ClockType::Infinite;
    game.time_control.max_nodes = max_nodes;
    game
}

/// The ordering passes replayed on a fresh game, as `search_root_node` runs them.
struct ReplayedPass {
    /// The pass that decides the move if a cancel lands in it: the root moves in generation order
    /// with their scores, None for a move whose search the cancel interrupted and every move after
    /// it. Under a node limit this is the depth-1 pass while that is cut; once it completes it is
    /// the depth-1 pass's scores (the limited ordering) and `is_cut` says whether the depth-3 pass
    /// that followed it, run over the stably sorted list, was cut.
    moves: Vec<(Move, Option<i32>)>,
    /// Whether a cancel cut the last pass run, so that no completed search decides the move.
    is_cut: bool,
}

/// One ordering pass of `depth` over `moves` in list order, as `search_root_node` runs it.
fn replay_one_pass(
    state: &mut SearchState,
    pv: &mut Variation,
    moves: &[Move],
    depth: i32,
) -> Vec<(Move, Option<i32>)> {
    let mut results = Vec::new();
    let mut is_interrupted = false;
    for (move_index, mv) in moves.iter().enumerate() {
        if is_interrupted {
            results.push((*mv, None));
            continue;
        }
        let score = state
            .search_single_move(depth, 0, ALPHA, BETA, *mv, pv, move_index as i32)
            .score();
        is_interrupted = state.is_cancelled();
        results.push((*mv, if is_interrupted { None } else { Some(score) }));
    }
    results
}

/// The ordering passes replayed on a fresh game. Under a node limit the depth-1 pass runs first
/// with the PV extension off; if it completes, its moves are stably sorted by score and the depth-3
/// pass runs over them in that order. With no node limit only the depth-3 pass runs.
fn replay_ordering_pass(max_nodes: u64, is_cancelled_at_start: bool) -> ReplayedPass {
    let game = fresh_game(max_nodes, 1);
    game.is_cancel_pending
        .store(is_cancelled_at_start, Ordering::Relaxed);
    let mut move_list = game.current_position().generate_legal_moves();
    let generated: Vec<Move> = (0..move_list.size())
        .map(|index| move_list[index])
        .collect();
    game.time_control.start_search(0, 0);
    game.transposition_table.age();
    let mut state = SearchState::new(&game);
    let mut pv: Variation = Vec::new();
    if max_nodes == 0 {
        let moves = replay_one_pass(&mut state, &mut pv, &generated, START_DEPTH);
        let is_cut = moves.iter().any(|(_, score)| score.is_none());
        return ReplayedPass { moves, is_cut };
    }
    state.is_pv_extension_suppressed = true;
    let limited = replay_one_pass(&mut state, &mut pv, &generated, LIMITED_ORDERING_DEPTH);
    state.is_pv_extension_suppressed = false;
    if limited.iter().any(|(_, score)| score.is_none()) {
        return ReplayedPass {
            moves: limited,
            is_cut: true,
        };
    }
    // Stable sort by the depth-1 scores, best first, ties in generation order.
    for (index, (_, score)) in limited.iter().enumerate() {
        move_list[index].assign_score(score.unwrap());
    }
    sort_moves(&mut move_list);
    let order: Vec<Move> = (0..move_list.size())
        .map(|index| move_list[index])
        .collect();
    let deep = replay_one_pass(&mut state, &mut pv, &order, START_DEPTH);
    let is_cut = deep.iter().any(|(_, score)| score.is_none());
    ReplayedPass {
        moves: if is_cut { limited } else { deep },
        is_cut,
    }
}

/// The best of the moves that finished: the highest score, the earliest generated on a tie (the
/// stable sort keeps generation order). None when no move finished.
fn best_finished(pass: &[(Move, Option<i32>)]) -> Option<Move> {
    let mut best: Option<(Move, i32)> = None;
    for (mv, score) in pass {
        if let Some(score) = score {
            if best.is_none_or(|(_, best_score)| *score > best_score) {
                best = Some((*mv, *score));
            }
        }
    }
    best.map(|(mv, _)| mv)
}

fn describe(pass: &[(Move, Option<i32>)]) -> String {
    pass.iter()
        .map(|(mv, score)| match score {
            Some(score) => format!("{}={}", mv.to_string(), score),
            None => format!("{}=unsearched", mv.to_string()),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Under each node limit the move played is the best move of the depth-1 pass when that completed
/// and the depth-3 pass was cut (a cut pass is set aside whole), else the best finished move of the
/// depth-1 pass that was cut; and never the move whose search was interrupted. On this FEN the
/// depth-1 pass takes 1,982 nodes, so the first poll (node 2048) falls inside the depth-3 pass at
/// every limit from 64 up, and inside the depth-1 pass only when the table is warm.
fn test_node_limits_inside_the_pass() {
    let mut cancelled_passes = 0;
    for max_nodes in [64, 2048, 4096, 16384, 65536, 131072, 262144] {
        let replayed = replay_ordering_pass(max_nodes, false);
        if !replayed.is_cut {
            continue; // the limit fell after the passes; nothing of this fix is exercised
        }
        let pass = replayed.moves;
        let finished = pass.iter().filter(|(_, score)| score.is_some()).count();
        cancelled_passes += 1;
        // A cut depth-1 pass leaves an interrupted move; a cut depth-3 pass is set aside whole,
        // and the depth-1 ordering (all its moves finished) decides.
        let interrupted = pass.get(finished).map(|(mv, _)| *mv);
        let Some(expected) = best_finished(&pass) else {
            check(
                false,
                &format!(
                    "go nodes {max_nodes} finishes a root move ({})",
                    describe(&pass)
                ),
            );
            continue;
        };
        let played = fresh_game(max_nodes, 1).search_root_node();
        check(
            played == expected,
            &format!(
                "go nodes {max_nodes}: {finished} of {} root moves finished, plays {} (expected {}; {})",
                pass.len(),
                played.to_string(),
                expected.to_string(),
                describe(&pass)
            ),
        );
        if let Some(interrupted) = interrupted {
            check(
                played != interrupted,
                &format!(
                    "go nodes {max_nodes}: does not play {}, the move whose search was interrupted",
                    interrupted.to_string()
                ),
            );
        }
        // Helpers launch after the pass, so with three threads the cancelled pass picks the same
        // move, and the helpers wind up and join.
        let played_with_helpers = fresh_game(max_nodes, 3).search_root_node();
        check(
            played_with_helpers == expected,
            &format!(
                "go nodes {max_nodes} at Threads 3: plays {} and joins its helpers",
                played_with_helpers.to_string()
            ),
        );
    }
    check(
        cancelled_passes > 0,
        &format!("some limit lands inside the pass ({cancelled_passes})"),
    );
}

/// A cancel already pending when the search begins - a `stop` that reached stop_thinking before
/// the worker started - interrupts the first move, so no root move finishes. The first generated
/// move is played then, or, when the table holds a root move for the root, that move; with three
/// threads too, since the helpers launch after the pass.
fn test_cancel_before_any_move_finished() {
    let pass = replay_ordering_pass(0, true).moves;
    check(
        pass.iter().all(|(_, score)| score.is_none()),
        &format!(
            "a cancel pending at the start finishes no root move ({})",
            describe(&pass)
        ),
    );
    let first_move = pass[0].0;
    let table_move = pass.last().unwrap().0; // the last generated, so never the first-move fallback
    for thread_count in [1, 3] {
        let game = fresh_game(0, thread_count);
        game.is_cancel_pending.store(true, Ordering::Relaxed);
        let played = game.search_root_node();
        check(
            played == first_move,
            &format!(
                "Threads {thread_count}, no move finished and nothing in the table: plays the first generated move {} (played {})",
                first_move.to_string(),
                played.to_string()
            ),
        );

        let game = fresh_game(0, thread_count);
        game.transposition_table
            .record_transposition(&Transposition::with_data(
                game.current_position().hash,
                table_move,
                0,
                1,
                NodeType::Pv,
            ));
        game.is_cancel_pending.store(true, Ordering::Relaxed);
        let played = game.search_root_node();
        check(
            played == table_move,
            &format!(
                "Threads {thread_count}, no move finished: plays the table's root move {} (played {})",
                table_move.to_string(),
                played.to_string()
            ),
        );

        // A table move that is not a root move here (the start position's first move) is ignored.
        let foreign_move = Position::from_string(START_FEN).generate_legal_moves()[0];
        let game = fresh_game(0, thread_count);
        game.transposition_table
            .record_transposition(&Transposition::with_data(
                game.current_position().hash,
                foreign_move,
                0,
                1,
                NodeType::Pv,
            ));
        game.is_cancel_pending.store(true, Ordering::Relaxed);
        let played = game.search_root_node();
        check(
            played == first_move,
            &format!(
                "Threads {thread_count}, no move finished and the table's move {} is not a root move: plays the first generated move {} (played {})",
                foreign_move.to_string(),
                first_move.to_string(),
                played.to_string()
            ),
        );
    }
}

/// Cancelled part-way, with a table entry for the root deeper than the pass, as an earlier search
/// of the game leaves one: its move is played, even one the pass never reached, ahead of the best
/// finished move. An entry at the pass's own depth, or one whose move is not a root move here, is
/// passed over for the best finished move. The first node limit, doubling from 2048, at which the
/// pass is cancelled after a move has finished is the one checked.
fn test_deep_table_move_when_cancelled_part_way() {
    // A move that is not a root move here (the start position's first move).
    let foreign_move = Position::from_string(START_FEN).generate_legal_moves()[0];
    let mut max_nodes: u64 = 2048;
    while max_nodes <= 1 << 24 {
        let replayed = replay_ordering_pass(max_nodes, false);
        if !replayed.is_cut {
            check(
                false,
                "a node limit cancels an ordering pass after a move has finished, for the table-move cases",
            );
            break;
        }
        let pass = replayed.moves;
        let finished = pass.iter().filter(|(_, score)| score.is_some()).count();
        let Some(expected) = best_finished(&pass) else {
            max_nodes *= 2;
            continue;
        };
        // A move the pass did not finish, so that playing it can only come from the table.
        let unreached_move = pass[pass.len() - 1].0;
        check(
            unreached_move != expected,
            &format!(
                "go nodes {max_nodes}: the last generated move {} is not the move the passes pick ({finished} of {} finished)",
                unreached_move.to_string(),
                pass.len()
            ),
        );
        let search_with_root_entry = |table_move: Move, depth: i32| {
            let game = fresh_game(max_nodes, 1);
            game.transposition_table
                .record_transposition(&Transposition::with_data(
                    game.current_position().hash,
                    table_move,
                    0,
                    depth,
                    NodeType::Pv,
                ));
            game.search_root_node()
        };
        let played = search_with_root_entry(unreached_move, START_DEPTH + 1);
        check(
            played == unreached_move,
            &format!(
                "go nodes {max_nodes}: cancelled after {finished} of {} moves with a depth-{} table move {}: plays it ({}), not the best finished move {}",
                pass.len(),
                START_DEPTH + 1,
                unreached_move.to_string(),
                played.to_string(),
                expected.to_string()
            ),
        );
        let played = search_with_root_entry(unreached_move, START_DEPTH);
        check(
            played == expected,
            &format!(
                "go nodes {max_nodes}: cancelled part-way with a table move only as deep as the pass: plays the best finished move {} ({})",
                expected.to_string(),
                played.to_string()
            ),
        );
        let played = search_with_root_entry(foreign_move, START_DEPTH + 1);
        check(
            played == expected,
            &format!(
                "go nodes {max_nodes}: cancelled part-way with a deep table move {} that is not a root move: plays the best finished move {} ({})",
                foreign_move.to_string(),
                expected.to_string(),
                played.to_string()
            ),
        );
        break;
    }
}

#[test]
fn root_ordering_cancel_suite() {
    if !common::is_shipped_net_this_builds_shape() {
        common::summary("ROOT ORDERING CANCEL: skipped (no net of this build's NNUE shape)");
        return;
    }
    test_node_limits_inside_the_pass();
    test_cancel_before_any_move_finished();
    test_deep_table_move_when_cancelled_part_way();
    let failures = common::summary("ROOT ORDERING CANCEL TESTS");
    assert_eq!(failures, 0);
}
