//! Every UCI command and option, driven in process through `process_input` on a library `Engine`.
//!
//! The shell suite and the robustness suite drive the built binary, which is the faithful test of
//! the protocol as a GUI sees it; this suite walks the parser's every branch - each `position`
//! form, each `go` keyword, each `setoption` name, the diagnostic commands and the malformed lines
//! a GUI or a person may send - and asserts on the engine state each one leaves behind. Its
//! searches are short (a few plies, a few hundred milliseconds), single-threaded where the outcome
//! is checked, and never change what the engine computes: the bench golden still holds the search.

mod common;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pawnstar::chess_clock::ClockType;
use pawnstar::constants::{HASHTABLE_MEGABYTES, MAX_SEARCH_THREADS};
use pawnstar::game::{Engine, START_FEN};
use pawnstar::input_handling::{atoi, process_input, BENCH_DEPTH, BENCH_FENS, HANDLERS};
use pawnstar::moves::Move;

const NET: &str = "nnue/pawnstar-v12.bin";
const BOOK: &str = "pawnstar.book";

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

/// Send one line. After a `go`, give the worker 100 ms to start before anything else is sent:
/// the root search clears the cancel flag as it begins, so a `stop` (or `quit`, or another
/// command that stops the search) landing before the worker reaches that point is lost, and an
/// infinite search would then never end. A GUI never sends `stop` that fast; a test would.
fn run(engine: &mut Engine, line: &str) {
    process_input(engine, line);
    if line.split_whitespace().next() == Some("go") {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn fen(engine: &Engine) -> String {
    engine.game().current_position().to_string()
}

fn history_len(engine: &Engine) -> usize {
    engine.game().positions.lock().unwrap().len()
}

/// Wait until the search worker has played its best move onto the history (it does so just before
/// printing `bestmove`), or `timeout_ms` passes. Returns whether it did.
fn wait_for_move(engine: &Engine, before: usize, timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if history_len(engine) > before {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

/// A single-threaded engine with the shipped net loaded and the book off.
fn quiet_engine() -> Engine {
    let mut engine = Engine::new();
    run(&mut engine, "setoption name Threads value 1");
    run(&mut engine, "setoption name OwnBook value false");
    run(&mut engine, &format!("setoption name EvalFile value {NET}"));
    engine
}

fn net_present() -> bool {
    std::path::Path::new(NET).exists()
}

/// `atoi`: the C++ semantics every numeric option and `go` value is read with.
fn test_atoi() {
    check(atoi("64") == 64, "atoi reads digits");
    check(
        atoi("  -12") == -12,
        "atoi skips whitespace and reads a minus sign",
    );
    check(
        atoi("+7x") == 7,
        "atoi reads a plus sign and stops at the first non-digit",
    );
    check(atoi("64abc") == 64, "atoi reads leading digits only");
    check(
        atoi("abc") == 0 && atoi("") == 0 && atoi("-") == 0,
        "atoi is 0 with no digits",
    );
}

/// Blank lines, unknown commands and stray whitespace change nothing.
fn test_dispatch() {
    let mut engine = Engine::new();
    let before = fen(&engine);
    for line in [
        "",
        "   ",
        "\t",
        "frobnicate",
        "UCI",
        "positionstartpos",
        "go2",
    ] {
        run(&mut engine, line);
    }
    check(
        fen(&engine) == before,
        "blank and unknown lines leave the position alone",
    );
    check(
        HANDLERS.iter().any(|h| h.name == "quit") && HANDLERS.len() == 18,
        "the handler table lists all eighteen commands",
    );
    run(&mut engine, "help");
    run(&mut engine, "uci");
    run(&mut engine, "isready");
    run(&mut engine, "getboard");
    check(
        fen(&engine) == START_FEN,
        "help / uci / isready / getboard are read-only",
    );
}

/// Every `position` form: startpos, a full FEN, a short FEN ended by `moves` or by the line, a
/// missing source, illegal moves and unknown tokens.
fn test_position() {
    let mut engine = Engine::new();

    run(&mut engine, "position startpos moves e2e4 e7e5 g1f3");
    check(
        fen(&engine) == "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2",
        "position startpos moves plays the moves",
    );
    check(
        history_len(&engine) == 4,
        "the history holds the start and three moves",
    );

    run(&mut engine, "position startpos");
    check(
        fen(&engine) == START_FEN && history_len(&engine) == 1,
        "position startpos resets",
    );

    let kiwipete = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
    run(&mut engine, &format!("position fen {kiwipete}"));
    check(fen(&engine) == kiwipete, "position fen with six fields");

    run(&mut engine, &format!("position fen {kiwipete} moves e1g1"));
    check(
        // (The engine's make_move restarts the fifty-move count on castling.)
        fen(&engine) == "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R4RK1 b kq - 0 1",
        "position fen ... moves castles",
    );

    // A FEN of fewer than six fields, ended by `moves`: the parser applies what it has.
    run(
        &mut engine,
        "position fen 4k3/8/8/8/8/8/8/4K2R w K moves e1g1",
    );
    check(
        fen(&engine).starts_with("4k3/8/8/8/8/8/8/5RK1 b"),
        "a short FEN ended by moves is applied, then the moves",
    );

    // A short FEN that runs to the end of the line is applied too.
    run(&mut engine, "position fen 4k3/8/8/8/8/8/8/4K3 b");
    check(
        fen(&engine).starts_with("4k3/8/8/8/8/8/8/4K3 b"),
        "a short FEN at the end of the line",
    );

    // `moves` with no source applies the moves to the current position.
    run(&mut engine, "position startpos");
    run(&mut engine, "position moves d2d4");
    check(
        fen(&engine).starts_with("rnbqkbnr/pppppppp/8/8/3P4/8/PPP1PPPP/RNBQKBNR b"),
        "position moves with no source plays on from the current position",
    );

    // An illegal move is skipped; the rest still play.
    run(&mut engine, "position startpos moves e2e5 e2e4 zz e7e5");
    check(
        history_len(&engine) == 3,
        "illegal and garbled moves are skipped",
    );

    // Unknown leading tokens are ignored until a source arrives; `position` alone does nothing.
    run(&mut engine, "position startpos");
    run(&mut engine, "position banana startpos moves a2a3");
    check(
        history_len(&engine) == 2,
        "unknown tokens before the source are ignored",
    );
    run(&mut engine, "position");
    check(history_len(&engine) == 2, "a bare position changes nothing");

    // A malformed FEN falls back to the start position rather than panicking.
    run(&mut engine, "position fen garbage");
    check(
        fen(&engine) == START_FEN,
        "a malformed FEN falls back to the start position",
    );

    // `position fen` followed straight by `moves`: an empty FEN is the start position.
    run(&mut engine, "position fen moves e2e4");
    check(
        history_len(&engine) == 2,
        "position fen moves: the start position, then the move",
    );
}

/// Every `setoption` name, each with good and out-of-range values, and the grammar's edge cases.
fn test_setoption() {
    let mut engine = Engine::new();

    run(&mut engine, "setoption name Threads value 3");
    check(
        engine.game().thread_count == 3,
        "Threads sets the thread count",
    );
    run(&mut engine, "setoption name Threads value 0");
    check(engine.game().thread_count == 1, "Threads clamps to 1");
    run(&mut engine, "setoption name Threads value 100000");
    check(
        engine.game().thread_count == MAX_SEARCH_THREADS,
        "Threads clamps to the maximum",
    );

    run(&mut engine, "setoption name Hash value 1");
    run(&mut engine, "setoption name Hash value 0");
    run(&mut engine, "setoption name Hash value banana");
    run(
        &mut engine,
        &format!("setoption name Hash value {HASHTABLE_MEGABYTES}"),
    );
    run(&mut engine, "setoption name Clear Hash");

    run(&mut engine, "setoption name Move Overhead value 120");
    check(
        engine.game().move_overhead == 120,
        "Move Overhead sets the overhead",
    );
    run(&mut engine, "setoption name Move Overhead value -5");
    check(
        engine.game().move_overhead == 0,
        "Move Overhead clamps to 0",
    );
    run(&mut engine, "setoption name Move Overhead value 99999");
    check(
        engine.game().move_overhead == 5000,
        "Move Overhead clamps to 5000",
    );

    run(&mut engine, "setoption name OwnBook value false");
    check(!engine.game().use_own_book, "OwnBook false");
    run(&mut engine, "setoption name OwnBook value true");
    check(engine.game().use_own_book, "OwnBook true");
    run(&mut engine, "setoption name OwnBook value maybe");
    check(
        !engine.game().use_own_book,
        "OwnBook is true only for 'true'",
    );

    run(&mut engine, "setoption name EvalFile value no/such/net.bin");
    check(
        !engine.game().nnue_network.loaded,
        "EvalFile with a missing file leaves no net",
    );
    if net_present() {
        run(&mut engine, &format!("setoption name EvalFile value {NET}"));
        check(
            engine.game().nnue_network.loaded,
            "EvalFile loads the shipped net",
        );
    }

    // Grammar: no name, no value, an unknown name, words before `name`, and a value of two words.
    run(&mut engine, "setoption");
    run(&mut engine, "setoption value 4");
    run(&mut engine, "setoption name Threads");
    check(
        engine.game().thread_count == 1,
        "Threads with no value reads 0 and clamps to 1",
    );
    run(&mut engine, "setoption junk name Threads value 2");
    check(
        engine.game().thread_count == 2,
        "words before name are ignored",
    );
    run(&mut engine, "setoption name Ponder value true");
    run(&mut engine, "setoption name No Such Option value 1 2 3");
    check(
        engine.game().thread_count == 2,
        "unknown options change nothing",
    );
}

/// `eval`, `nnue`, `dbg`, `dbgclear`, `getboard`, `bookmoves`, `freebook` and `ucinewgame`.
fn test_diagnostics() {
    let mut engine = Engine::new();
    // With no net loaded, eval and nnue say so instead of evaluating garbage.
    check(
        !engine.game().nnue_network.loaded,
        "a fresh library engine has no net",
    );
    run(&mut engine, "eval");
    run(&mut engine, "nnue");

    if net_present() {
        run(&mut engine, &format!("setoption name EvalFile value {NET}"));
        run(&mut engine, "position startpos moves e2e4");
        run(&mut engine, "eval");
        run(&mut engine, "nnue");
    }
    run(&mut engine, "dbg");
    run(&mut engine, "dbgclear");
    run(&mut engine, "dbg");
    run(&mut engine, "getboard");

    // The book: nothing to show without one, then the start position's moves, then freed.
    run(&mut engine, "position startpos");
    run(&mut engine, "bookmoves");
    check(
        engine.game_mut().book.initialize(BOOK),
        "the shipped book loads",
    );
    let start_hash = engine.game().current_position().hash;
    check(
        engine.game().book.get_move(start_hash) != Move::none(),
        "the book has a move for the start position",
    );
    run(&mut engine, "bookmoves");
    run(&mut engine, "position startpos moves e2e4");
    run(&mut engine, "bookmoves");
    run(&mut engine, "freebook");
    check(
        engine.game().book.get_move(start_hash) == Move::none(),
        "freebook empties the book",
    );

    run(&mut engine, "position startpos moves e2e4 e7e5");
    run(&mut engine, "ucinewgame");
    check(
        fen(&engine) == START_FEN && history_len(&engine) == 1,
        "ucinewgame resets to the start position",
    );
}

/// The `go` parser: each keyword's effect on the clock, read without searching to the end.
fn test_go_parsing() {
    if !net_present() {
        return;
    }
    let mut engine = quiet_engine();

    // White to move: wtime / winc are ours, btime / binc ignored.
    run(&mut engine, "position startpos");
    run(
        &mut engine,
        "go wtime 60000 btime 1 winc 500 binc 9 movestogo 20 bogus",
    );
    {
        let tc = &engine.game().time_control;
        check(
            tc.clock_type == ClockType::Standard,
            "go wtime: a standard clock",
        );
        check(
            tc.remaining_time == 60000,
            "go wtime: white's time when white is to move",
        );
        check(
            tc.increment == 500,
            "go winc: white's increment when white is to move",
        );
        check(tc.moves_to_go == 20, "go movestogo");
    }
    engine.stop_thinking();

    // Black to move: btime / binc are ours.
    run(&mut engine, "position startpos moves e2e4");
    run(&mut engine, "go wtime 1 btime 45000 winc 9 binc 250");
    {
        let tc = &engine.game().time_control;
        check(
            tc.remaining_time == 45000,
            "go btime: black's time when black is to move",
        );
        check(
            tc.increment == 250,
            "go binc: black's increment when black is to move",
        );
    }
    engine.stop_thinking();

    run(&mut engine, "go movetime 5000");
    check(
        engine.game().time_control.clock_type == ClockType::FixedTime
            && engine.game().time_control.remaining_time == 5000,
        "go movetime: a fixed-time clock",
    );
    engine.stop_thinking();

    run(&mut engine, "go nodes 123456789");
    check(
        engine.game().time_control.clock_type == ClockType::Infinite
            && engine.game().time_control.max_nodes == 123456789,
        "go nodes: no clock and a node budget",
    );
    engine.stop_thinking();

    run(&mut engine, "go infinite");
    check(
        engine.game().time_control.clock_type == ClockType::Infinite,
        "go infinite",
    );
    run(&mut engine, "stop");

    // searchmoves collects until the next keyword, which is then read as a keyword.
    run(&mut engine, "position startpos");
    run(&mut engine, "go searchmoves a2a3 h2h4 depth 1");
    check(
        engine.game().search_moves == vec!["a2a3".to_string(), "h2h4".to_string()],
        "go searchmoves collects the move list",
    );
    check(
        engine.game().time_control.clock_type == ClockType::FixedDepth
            && engine.game().time_control.depth == 1,
        "the keyword after the move list is read",
    );
    engine.stop_thinking();
    for keyword in ["ponder", "infinite", "searchmoves", "wtime 1000"] {
        run(&mut engine, &format!("go searchmoves e2e4 {keyword}"));
        check(
            engine.game().search_moves == vec!["e2e4".to_string()],
            &format!("go searchmoves ends at '{keyword}'"),
        );
        engine.stop_thinking();
    }

    // A value keyword with a garbled value reads 0; a dangling keyword reads nothing.
    run(&mut engine, "go depth x");
    engine.stop_thinking();
    run(&mut engine, "go depth");
    engine.stop_thinking();
    // A fresh go clears the previous node budget, increment and searchmoves.
    run(&mut engine, "go depth 1");
    {
        let game = engine.game();
        check(
            game.time_control.max_nodes == 0
                && game.time_control.increment == 0
                && game.search_moves.is_empty(),
            "each go clears nodes, increment and searchmoves first",
        );
    }
    engine.stop_thinking();
}

/// Searches that run to their end, each playing its best move onto the history.
fn test_go_searches() {
    if !net_present() {
        return;
    }
    let mut engine = quiet_engine();

    // Fixed depth.
    run(&mut engine, "position startpos");
    run(&mut engine, "go depth 4");
    check(wait_for_move(&engine, 1, 30000), "go depth 4 answers");

    // A standard clock with a few seconds left: the hard budget is the 100 ms floor.
    run(&mut engine, "position startpos");
    run(&mut engine, "go wtime 1100 btime 1100");
    check(
        wait_for_move(&engine, 1, 30000),
        "go wtime with a short clock answers",
    );

    // A standard clock with a known number of moves to go, and the move overhead.
    run(&mut engine, "setoption name Move Overhead value 50");
    run(&mut engine, "position startpos moves e2e4");
    run(&mut engine, "go wtime 5000 btime 3000 movestogo 30");
    check(
        wait_for_move(&engine, 2, 30000),
        "go btime movestogo answers",
    );

    // Fixed time.
    run(&mut engine, "position startpos");
    run(&mut engine, "go movetime 200");
    check(wait_for_move(&engine, 1, 30000), "go movetime answers");

    // A node budget.
    run(&mut engine, "position startpos");
    run(&mut engine, "go nodes 20000");
    check(
        wait_for_move(&engine, 1, 30000),
        "go nodes answers within its budget",
    );

    // Infinite until stopped: stop joins the worker, which plays its move.
    run(&mut engine, "position startpos");
    run(&mut engine, "go infinite");
    std::thread::sleep(Duration::from_millis(300));
    check(
        history_len(&engine) == 1,
        "go infinite does not answer on its own",
    );
    run(&mut engine, "stop");
    check(
        history_len(&engine) == 2,
        "stop ends go infinite with a move",
    );

    // Pondering, then ponderhit: the search becomes a timed one and answers.
    run(&mut engine, "position startpos");
    run(&mut engine, "go ponder wtime 2000 btime 2000");
    check(
        engine.game().is_pondering.load(Ordering::Relaxed),
        "go ponder sets the ponder flag",
    );
    std::thread::sleep(Duration::from_millis(100));
    run(&mut engine, "ponderhit");
    check(
        !engine.game().is_pondering.load(Ordering::Relaxed),
        "ponderhit clears the ponder flag",
    );
    check(
        wait_for_move(&engine, 1, 30000),
        "a ponderhit search answers",
    );

    // A forced move answers without a search; a mated side answers nothing.
    run(&mut engine, "position fen 7k/8/8/8/8/8/1q6/K7 w - - 0 1");
    run(&mut engine, "go depth 5");
    check(
        wait_for_move(&engine, 1, 30000),
        "a single legal move answers at once",
    );
    check(
        fen(&engine).starts_with("7k/8/8/8/8/8/1K6/8 b"),
        "the forced move is the one played",
    );
    run(&mut engine, "position fen 7k/8/8/8/8/8/1q6/K7 w - - 0 1");
    run(&mut engine, "go searchmoves a1b2 depth 2");
    check(
        wait_for_move(&engine, 1, 30000),
        "searchmoves searches even a forced move",
    );
    run(&mut engine, "position fen k7/1Q6/1K6/8/8/8/8/8 b - - 0 1");
    run(&mut engine, "go depth 3");
    engine.stop_thinking();
    check(history_len(&engine) == 1, "a checkmated side plays no move");

    // searchmoves naming no legal move searches every move.
    run(&mut engine, "position startpos");
    run(&mut engine, "go searchmoves e2e5 a1a8 depth 2");
    check(
        wait_for_move(&engine, 1, 30000),
        "an all-illegal searchmoves list is ignored",
    );

    // The book: a book move answers at once, unless searchmoves restricts the root.
    check(
        engine.game_mut().book.initialize(BOOK),
        "the shipped book loads",
    );
    run(&mut engine, "setoption name OwnBook value true");
    run(&mut engine, "position startpos");
    run(&mut engine, "go depth 30");
    check(
        wait_for_move(&engine, 1, 5000),
        "a book move answers at once",
    );
    run(&mut engine, "position startpos");
    run(&mut engine, "go searchmoves a2a3 depth 2");
    check(
        wait_for_move(&engine, 1, 30000),
        "searchmoves skips the book",
    );
    check(
        fen(&engine).starts_with("rnbqkbnr/pppppppp/8/8/8/P7/"),
        "searchmoves a2a3 plays a2a3",
    );

    // Two threads: a Lazy SMP helper runs beside the main thread.
    run(&mut engine, "setoption name OwnBook value false");
    run(&mut engine, "setoption name Threads value 2");
    run(&mut engine, "position startpos");
    run(&mut engine, "go depth 5");
    check(
        wait_for_move(&engine, 1, 30000),
        "a two-thread search answers",
    );

    // A new go while one runs stops it first.
    run(&mut engine, "position startpos");
    run(&mut engine, "go infinite");
    run(&mut engine, "go depth 2");
    check(
        wait_for_move(&engine, 1, 30000),
        "go during a search restarts it",
    );
    run(&mut engine, "stop");
    run(&mut engine, "stop");
}

/// `bench` at a shallow depth, with and without the per-position debug dump, restores the
/// options it overrides.
fn test_bench() {
    if !net_present() {
        return;
    }
    let mut engine = quiet_engine();
    run(&mut engine, "setoption name Threads value 3");
    run(&mut engine, "setoption name OwnBook value true");
    run(&mut engine, "bench 2");
    check(
        engine.game().thread_count == 3 && engine.game().use_own_book,
        "bench restores Threads and OwnBook",
    );
    check(
        fen(&engine) == BENCH_FENS[BENCH_FENS.len() - 1],
        "bench leaves the board at its last position",
    );
    check(BENCH_DEPTH == 13, "the default bench depth is 13");
    std::env::set_var("PAWNSTAR_BENCH_DEBUG", "1");
    run(&mut engine, "bench 1");
    std::env::remove_var("PAWNSTAR_BENCH_DEBUG");
    check(
        engine.game().last_search_node_count.load(Ordering::Relaxed) > 0,
        "bench with the debug dump searched",
    );
}

/// PAWNSTAR_THREADS sets the default thread count, read with atoi and clamped.
fn test_threads_env() {
    let cases: [(&str, Option<i32>); 5] = [
        ("1", Some(1)),
        ("3", Some(3)),
        ("100000", Some(MAX_SEARCH_THREADS)),
        ("0", None),
        ("banana", None),
    ];
    let hardware = {
        std::env::remove_var("PAWNSTAR_THREADS");
        pawnstar::game::Game::compute_default_threads()
    };
    check(
        (1..=MAX_SEARCH_THREADS).contains(&hardware),
        "with no PAWNSTAR_THREADS the default is the hardware's, clamped",
    );
    for (value, want) in cases {
        std::env::set_var("PAWNSTAR_THREADS", value);
        let got = pawnstar::game::Game::compute_default_threads();
        check(
            got == want.unwrap_or(hardware),
            &format!("PAWNSTAR_THREADS={value} gives {got}"),
        );
    }
    std::env::remove_var("PAWNSTAR_THREADS");
}

/// `quit` stops a running search (which answers) and tells the caller to stop reading; every
/// other line, blank or unknown, keeps it reading.
fn test_quit() {
    let mut engine = Engine::new();
    for line in ["", "isready", "frobnicate", "quitter"] {
        check(
            process_input(&mut engine, line),
            &format!("{line:?} keeps reading"),
        );
    }
    if net_present() {
        engine = quiet_engine();
        run(&mut engine, "position startpos");
        run(&mut engine, "go infinite");
        std::thread::sleep(Duration::from_millis(100));
        check(!process_input(&mut engine, "quit"), "quit stops reading");
        check(
            history_len(&engine) == 2,
            "quit stops the search, which answers",
        );
    }
    check(
        !process_input(&mut engine, "quit now"),
        "quit with words after it still quits",
    );
}

#[test]
fn uci_commands_suite() {
    test_atoi();
    test_dispatch();
    test_position();
    test_setoption();
    test_diagnostics();
    test_go_parsing();
    test_go_searches();
    test_bench();
    test_threads_env();
    test_quit();
    assert_eq!(
        common::summary("UCI commands: position, go, setoption, diagnostics, bench"),
        0,
        "uci commands suite reported failures"
    );
}
