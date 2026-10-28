//! UCI command parsing + dispatch. (Port of `input_handling.h`.)
//!
//! Handlers take the `Engine` wrapper rather than the C++ `Game&`: handlers that mutate plain
//! Game fields go through `Engine::game_mut()`, which joins any running search first — the
//! compiler-enforced version of the C++ "UCI thread mutates Game only while no search runs"
//! convention. `stop` joins; `ponderhit` deliberately does not (it retargets the running search
//! through atomics only).

use std::sync::atomic::Ordering;

use crate::chess_clock::ClockType;
use crate::constants::{Color, HASHTABLE_MEGABYTES, MAX_SEARCH_THREADS};
use crate::debug_hashtable::{debug_x_clear, debug_x_write};
use crate::game::Engine;
use crate::nnue::{print_net_message_on_both_streams, NetLoadOutcome};
use crate::search_state::{evaluate_position, SearchState};
use crate::uci_println;
use crate::version::version_string;

/// C++ `std::atoi` semantics: skip leading whitespace, optional sign, then parse leading digits;
/// 0 when no digits ("64abc" -> 64, "abc" -> 0). The C++ reads the PAWNSTAR_THREADS environment
/// variable this way; UCI input is read with parse_integer instead.
pub fn atoi(s: &str) -> i64 {
    let t = s.trim_start();
    let (sign, rest) = match t.as_bytes().first() {
        Some(b'-') => (-1i64, &t[1..]),
        Some(b'+') => (1, &t[1..]),
        _ => (1, t),
    };
    let digits: &str = &rest[..rest.bytes().take_while(u8::is_ascii_digit).count()];
    sign * digits.parse::<i64>().unwrap_or(0)
}

/// Read a numeric UCI value strictly: the whole of `text` must be an optional sign and decimal
/// digits that fit an i64 — the rule of the C++ engine's ParseInteger and the Go engine's
/// strconv.Atoi. Anything else ("abc", "12x", "", "+-3", an overflow) is None, and the caller then
/// treats the value, and the `go` keyword or option it belongs to, as absent.
pub fn parse_integer(text: &str) -> Option<i64> {
    text.parse::<i64>().ok()
}

/// A parsed value narrowed to an i32 field (a depth, a move count), saturating rather than
/// wrapping.
pub fn saturate_to_i32(value: i64) -> i32 {
    value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Pointer to a command handler taking the engine and the command's argument list.
pub type HandlerFn = fn(&mut Engine, &[String]);

/// A handler for an input command.
pub struct Handler {
    pub function: HandlerFn,       // Function to be called
    pub name: &'static str,        // Command name
    pub description: &'static str, // Command description
}

/// Handle the "bookmoves" command: display book moves for the current position.
fn handle_bookmoves(engine: &mut Engine, _args: &[String]) {
    let game = engine.game();
    game.book.display_available_moves(&game.current_position());
}

/// Handle the "freebook" command: release the opening book from memory.
fn handle_freebook(engine: &mut Engine, _args: &[String]) {
    engine.game_mut().book.free();
}

/// Handle the "eval" command: print the static evaluation of the current position.
fn handle_eval(engine: &mut Engine, _args: &[String]) {
    let game = engine.game();
    if !game.nnue_network.loaded {
        uci_println!("info string no NNUE net loaded (use setoption name EvalFile value <path>)");
        return;
    }
    let mut tmp = SearchState::new(game);
    uci_println!("evaluation {} (nnue)", evaluate_position(&mut tmp));
}

/// Handle the "nnue" command: print the raw NNUE evaluation of the current position (diagnostic).
fn handle_nnue(engine: &mut Engine, _args: &[String]) {
    let game = engine.game();
    if !game.nnue_network.loaded {
        uci_println!("info string NNUE: no net loaded (use setoption name EvalFile value <path>)");
        return;
    }
    uci_println!(
        "nnue {}",
        game.nnue_network
            .evaluate_position(&game.current_position())
    );
}

/// Handle the "dbg" command: print diagnostic counters.
fn handle_dbg(_engine: &mut Engine, _args: &[String]) {
    debug_x_write();
}

/// Handle the "dbgclear" command: reset diagnostic counters.
fn handle_dbgclear(_engine: &mut Engine, _args: &[String]) {
    debug_x_clear();
}

/// Handle the "getboard" command: print the FEN of the current position.
fn handle_getboard(engine: &mut Engine, _args: &[String]) {
    let fen = engine.game().current_position().to_string();
    uci_println!("{}", fen);
}

/// Handle the "uci" command: identify the engine and acknowledge UCI mode.
fn handle_uci(engine: &mut Engine, _args: &[String]) {
    let game = engine.game();
    uci_println!("id name Pawnstar {}", version_string());
    uci_println!("id author Jonny Reckless");
    uci_println!(
        "option name Hash type spin default {} min 1 max 4096",
        HASHTABLE_MEGABYTES
    );
    uci_println!(
        "option name Threads type spin default {} min 1 max {}",
        game.thread_count,
        MAX_SEARCH_THREADS
    );
    uci_println!("option name Ponder type check default false");
    uci_println!("option name EvalFile type string default <empty>");
    uci_println!("option name Clear Hash type button");
    uci_println!(
        "option name Move Overhead type spin default {} min 0 max 5000",
        game.move_overhead
    );
    uci_println!(
        "option name OwnBook type check default {}",
        if game.use_own_book { "true" } else { "false" }
    );
    uci_println!("uciok");
}

/// Handle the "setoption" command: "setoption name <Name> value <Value>".
/// Recognises EvalFile (load a different NNUE net file), Hash (transposition table size in MB,
/// 1..4096), Threads (Lazy SMP search threads), Clear Hash (a button that empties the
/// transposition table), Move Overhead (ms reserved from each deadline, 0..5000) and OwnBook
/// (whether to consult the built-in opening book). A numeric option whose value is not an integer
/// (parse_integer) is left as it is.
fn handle_setoption(engine: &mut Engine, args: &[String]) {
    // Parse the UCI "name <words...> value <words...>" grammar.
    let mut name = String::new();
    let mut value = String::new();
    let mut target: Option<&mut String> = None;
    for arg in &args[1..] {
        if arg == "name" {
            target = Some(&mut name);
        } else if arg == "value" {
            target = Some(&mut value);
        } else if let Some(t) = target.as_deref_mut() {
            if !t.is_empty() {
                t.push(' ');
            }
            t.push_str(arg);
        }
    }
    let game = engine.game_mut();
    if name == "EvalFile" {
        // A refused net is not substituted for the one already playing, and does not leave the
        // engine with no evaluator at all: the loader has said which field disagreed, and the net
        // loaded before this command stays loaded. (Without that, a mismatched EvalFile left the
        // network unloaded, the engine still answered `readyok`, and the first `go` had no
        // evaluator.) Only a net that actually loaded invalidates the cached evaluations.
        //
        // Keeping the old net is right; being quiet about it was the defect. **Which** net is now
        // playing goes out on both streams, naming it: a harness that set EvalFile, saw `readyok`
        // and ran a thousand games otherwise attributes the result to a net that never played a
        // move, and `info` lines are exactly what a harness drops.
        let outcome = game.nnue_network.load(&value);
        if outcome.is_loaded() {
            game.eval_cache.clear(); // cached evals belong to the previous net
        } else if game.nnue_network.loaded {
            // A file that could not be read at all is not a refusal — nothing is known about its
            // contents — so it is turned down in its own words, with the same kept-net sentence.
            let what_happened = if outcome == NetLoadOutcome::Refused {
                "was refused"
            } else {
                "could not be read"
            };
            print_net_message_on_both_streams(&format!(
                "NNUE: keeping the net already loaded ('{}'); '{}' {} and is not in play",
                game.nnue_network.loaded_net_origin(),
                value,
                what_happened
            ));
        }
    } else if name == "Hash" {
        // A malformed value (parse_integer) leaves each numeric option as it is.
        if let Some(mb) = parse_integer(&value) {
            game.transposition_table.resize(mb.clamp(1, 4096) as usize);
        }
    } else if name == "Threads" {
        if let Some(threads) = parse_integer(&value) {
            game.thread_count = threads.clamp(1, MAX_SEARCH_THREADS as i64) as i32;
        }
    } else if name == "Clear Hash" {
        // a button: no value, just empty the transposition table
        game.transposition_table.clear();
    } else if name == "Move Overhead" {
        if let Some(overhead) = parse_integer(&value) {
            game.move_overhead = overhead.clamp(0, 5000);
        }
    } else if name == "OwnBook" {
        game.use_own_book = value == "true";
    }
}

/// Handle the "ucinewgame" command: stop searching and reset to a new game.
/// Clears all shared, game-spanning state so the new game starts clean: the transposition table
/// and the NNUE eval cache.
fn handle_ucinewgame(engine: &mut Engine, _args: &[String]) {
    let game = engine.game_mut(); // joins the worker (the C++ StopThinking)
    game.transposition_table.clear();
    game.eval_cache.clear();
    game.set_position_start();
}

/// Fixed, known-legal position set for `bench` (the Bratko-Kopec middlegame/endgame positions).
/// Each is searched to a fixed depth; the total node count is a determinism signature and the nps
/// a speed check.
pub static BENCH_FENS: [&str; 16] = [
    "1k1r4/pp1b1R2/3q2pp/4p3/2B5/4Q3/PPP2B2/2K5 b - - 0 1",
    "3r1k2/4npp1/1ppr3p/p6P/P2PPPP1/1NR5/5K2/2R5 w - - 0 1",
    "2q1rr1k/3bbnnp/p2p1pp1/2pPp3/PpP1P1P1/1P2BNNP/2BQ1PRK/7R b - - 0 1",
    "rnbqkb1r/p3pppp/1p6/2ppP3/3N4/2P5/PPP1QPPP/R1B1KB1R w KQkq - 0 1",
    "r1b2rk1/2q1b1pp/p2ppn2/1p6/3QP3/1BN1B3/PPP3PP/R4RK1 w - - 0 1",
    "2r3k1/pppR1pp1/4p3/4P1P1/5P2/1P4K1/P1P5/8 w - - 0 1",
    "1nk1r1r1/pp2n1pp/4p3/q2pPp1N/b1pP1P2/B1P2R2/2P1B1PP/R2Q2K1 w - - 0 1",
    "4b3/p3kp2/6p1/3pP2p/2pP1P2/4K1P1/P3N2P/8 w - - 0 1",
    "2kr1bnr/pbpq4/2n1pp2/3p3p/3P1P1B/2N2N1Q/PPP3PP/2KR1B1R w - - 0 1",
    "3rr1k1/pp3pp1/1qn2np1/8/3p4/PP1R1P2/2P1NQPP/R1B3K1 b - - 0 1",
    "2r1nrk1/p2q1ppp/bp1p4/n1pPp3/P1P1P3/2PBB1N1/4QPPP/R4RK1 w - - 0 1",
    "r3r1k1/ppqb1ppp/8/4p1NQ/8/2P5/PP3PPP/R3R1K1 b - - 0 1",
    "r2q1rk1/4bppp/p2p4/2pP4/3pP3/3Q4/PP1B1PPP/R3R1K1 w - - 0 1",
    "rnb2r1k/pp2p2p/2pp2p1/q2P1p2/8/1Pb2NP1/PB2PPBP/R2Q1RK1 w - - 0 1",
    "2r3k1/1p2q1pp/2b1pr2/p1pp4/6Q1/1P1PP1R1/P1PN2PP/5RK1 w - - 0 1",
    "r1bqkb1r/4npp1/p1p4p/1p1pP1B1/8/1B6/PPPN1PPP/R2Q1RK1 w kq - 0 1",
];

/// Default `bench` search depth (overridden by `bench <depth>`).
pub const BENCH_DEPTH: i32 = 13;

/// Handle the "bench" command: search each position in BENCH_FENS to a fixed depth (BENCH_DEPTH,
/// or `bench <depth>`) and print `<nodes> nodes <nps> nps`. The node total is a determinism
/// signature (a search change meant to be neutral that alters it shows up immediately) and the
/// nps a quick speed-regression check. Forces a single thread and disables the opening book for
/// reproducibility (restored afterwards); the hash is cleared up front. Leaves the board at the
/// last bench position (run it standalone).
fn handle_bench(engine: &mut Engine, args: &[String]) {
    // `bench <depth>` overrides the default; a missing, malformed or non-positive depth keeps it.
    let depth: i32 = match args.get(1).and_then(|text| parse_integer(text)) {
        Some(requested) if requested > 0 => saturate_to_i32(requested),
        _ => BENCH_DEPTH,
    };
    let game = engine.game_mut();
    let saved_threads = game.thread_count;
    let saved_own_book = game.use_own_book;
    game.thread_count = 1; // deterministic, single-threaded
    game.use_own_book = false; // never short-circuit on a book move
    game.transposition_table.clear();
    game.eval_cache.clear();

    // PAWNSTAR_BENCH_DEBUG dumps each position's bestmove and diagnostic counters after its
    // search, for cross-engine comparison. Off by default, so normal bench output is unchanged.
    let debug = std::env::var_os("PAWNSTAR_BENCH_DEBUG").is_some();
    let mut pos_index = 0;
    let mut total_nodes: u64 = 0;
    let start = std::time::Instant::now();
    for fen in BENCH_FENS {
        game.set_position(fen); // also clears the cancel flag the previous search left set
                                // (search_root_node never does)
        game.time_control.clock_type = ClockType::FixedDepth; // set fixed depth after SetPosition's clock reset
        game.time_control.depth = depth;
        if debug {
            debug_x_clear();
        }
        let mv = game.search_root_node();
        let nodes = game.last_search_node_count.load(Ordering::Relaxed);
        total_nodes += nodes;
        if debug {
            pos_index += 1;
            uci_println!(
                "### pos {:02} bm={} nodes={}",
                pos_index,
                mv.to_string(),
                nodes
            );
            debug_x_write();
        }
    }
    let elapsed_ms = start.elapsed().as_millis() as i64;
    let nps = (total_nodes as i64) * 1000 / elapsed_ms.max(1);

    game.thread_count = saved_threads;
    game.use_own_book = saved_own_book;
    uci_println!("{} nodes {} nps", total_nodes, nps);
}

/// Handle the "go" command: parse time controls and start searching.
/// Parsed as a small state machine alternating between a keyword and its integer argument.
/// Recognised keywords: the flags `infinite`/`ponder`, and the value-taking `wtime`/`btime`/
/// `winc`/`binc`/`movetime`/`depth`/`movestogo`/`nodes`, plus `searchmoves` and its move list.
fn handle_go(engine: &mut Engine, args: &[String]) {
    /// Parser state: whether the next token is a keyword or the pending keyword's integer value.
    enum State {
        AwaitKeyword, // Expecting a keyword (`infinite`, `wtime`, `depth`, …).
        AwaitValue,   // Expecting the integer argument for `pending`.
        CollectMoves, // Collecting the move list that follows `searchmoves`.
    }
    const VALUE_KEYWORDS: [&str; 8] = [
        "wtime",
        "btime",
        "winc",
        "binc",
        "movetime",
        "depth",
        "movestogo",
        "nodes",
    ];

    // The C++ parses into the live Game then StartThinking stops any previous search; game_mut
    // stops first, then parses — a benign, strictly-safer reordering.
    let game = engine.game_mut();

    let mut state = State::AwaitKeyword;
    let mut pending = String::new(); // the value-taking keyword whose argument is expected next

    game.is_pondering.store(false, Ordering::Relaxed); // set true below if this is a `go ponder`
    game.time_control.increment = 0; // cleared up front; set below if winc/binc present
    game.time_control.max_nodes = 0; // cleared up front; set below if `nodes` present
    game.search_moves.clear(); // set below if `searchmoves` present

    let mut i = 1;
    while i < args.len() {
        let token = args[i].as_str();
        match state {
            State::CollectMoves => {
                // Collect coordinate-notation moves until the next recognised keyword.
                if token == "ponder"
                    || token == "infinite"
                    || token == "searchmoves"
                    || VALUE_KEYWORDS.contains(&token)
                {
                    state = State::AwaitKeyword;
                    continue; // reprocess this keyword (the C++ --i)
                }
                game.search_moves.push(token.to_string());
            }
            State::AwaitKeyword => {
                if token == "ponder" {
                    // flag: search on the opponent's time until ponderhit / stop
                    game.is_pondering.store(true, Ordering::Relaxed);
                } else if token == "searchmoves" {
                    // restrict the root search to the move list that follows
                    state = State::CollectMoves;
                } else if token == "infinite" {
                    // a flag, no argument
                    game.time_control.clock_type = ClockType::Infinite; // no time limit
                } else if VALUE_KEYWORDS.contains(&token) {
                    pending = token.to_string(); // a value-taking keyword: its argument is next
                    state = State::AwaitValue;
                }
                // else: ignore unrecognised tokens
            }
            State::AwaitValue => {
                state = State::AwaitKeyword;
                // A value that is not an integer (`go depth abc`) is consumed and ignored, and so
                // is its keyword, exactly as a keyword with no value at the end of the line is:
                // the line reads as though neither were there.
                let Some(value) = parse_integer(token) else {
                    i += 1;
                    continue;
                };
                let color_to_move = game.current_position().color_to_move;
                if (pending == "wtime" && color_to_move == Color::White)
                    || (pending == "btime" && color_to_move == Color::Black)
                {
                    game.time_control.clock_type = ClockType::Standard;
                    game.time_control.remaining_time = value;
                } else if (pending == "winc" && color_to_move == Color::White)
                    || (pending == "binc" && color_to_move == Color::Black)
                {
                    game.time_control.increment = value;
                } else if pending == "movetime" {
                    game.time_control.clock_type = ClockType::FixedTime;
                    game.time_control.remaining_time = value;
                } else if pending == "depth" {
                    game.time_control.clock_type = ClockType::FixedDepth;
                    game.time_control.depth = value as i32;
                } else if pending == "movestogo" {
                    game.time_control.moves_to_go = value as i32;
                } else if pending == "nodes" {
                    // Node-limited search: run with no clock (Infinite) and stop at the node
                    // budget. Useful for reproducible, machine-speed-independent testing.
                    game.time_control.clock_type = ClockType::Infinite;
                    game.time_control.max_nodes = value as u64;
                }
                // (wtime/btime for the side *not* to move falls through and is ignored.)
                state = State::AwaitKeyword;
            }
        }
        i += 1;
    }
    engine.start_thinking();
}

/// Handle the "stop" command: stop searching and report the best move found.
fn handle_stop(engine: &mut Engine, _args: &[String]) {
    engine.stop_thinking();
}

/// Handle the "ponderhit" command: the predicted move was played, so convert the running ponder
/// search into a normally-timed search, keeping all work done so far (warm TT, current iteration
/// depth). on_ponderhit starts the budget from this moment (budget-from-ponderhit); is_pondering
/// is cleared last so the search never sees the new flag with a stale start time. Must NOT join
/// the worker.
fn handle_ponderhit(engine: &mut Engine, _args: &[String]) {
    let game = engine.game();
    game.time_control.on_ponderhit();
    game.is_pondering.store(false, Ordering::Relaxed);
}

/// Handle the "quit" command: stop searching. `process_input` then returns false and `main`
/// returns, so the engine exits 0 as the C++ `exit(0)` does - but by returning from `main`, which
/// runs the C runtime's exit handlers on every platform (Rust's `process::exit` is
/// `ExitProcess` on Windows, which skips them - and with them an instrumented build's coverage
/// profile), and lets a library caller quit without ending its own process.
fn handle_quit(engine: &mut Engine, _args: &[String]) {
    engine.stop_thinking();
}

/// Handle the "isready" command: respond with readyok.
fn handle_isready(_engine: &mut Engine, _args: &[String]) {
    uci_println!("readyok");
}

/// Handle the "position" command: set up the position and apply the listed moves.
/// Parses the UCI grammar `position [startpos | fen <6 fields>] [moves <m1> <m2> ...]` as a small
/// state machine over the tokens.
fn handle_position(engine: &mut Engine, args: &[String]) {
    /// Parser state: what the next token is expected to be.
    enum State {
        AwaitSource, // Expecting `startpos` or `fen` (the position source).
        CollectFen,  // Accumulating the six space-separated FEN fields.
        PlayMoves,   // Source is set; the `moves` keyword and the moves themselves follow.
    }

    let game = engine.game_mut();
    let mut state = State::AwaitSource;
    let mut fen = String::new();
    let mut fen_fields = 0;

    for token in &args[1..] {
        match state {
            State::AwaitSource => {
                if token == "startpos" {
                    game.set_position_start();
                    state = State::PlayMoves;
                } else if token == "fen" {
                    state = State::CollectFen;
                } else if token == "moves" {
                    state = State::PlayMoves; // tolerate a missing source: apply to current position
                }
            }
            State::CollectFen => {
                if token == "moves" {
                    game.set_position(&fen); // FEN ended early (fewer than six fields tolerated)
                    state = State::PlayMoves;
                } else {
                    if !fen.is_empty() {
                        fen.push(' ');
                    }
                    fen.push_str(token);
                    fen_fields += 1;
                    if fen_fields == 6 {
                        // a complete FEN; anything after must be the move list
                        game.set_position(&fen);
                        state = State::PlayMoves;
                    }
                }
            }
            State::PlayMoves => {
                if token != "moves" {
                    // skip the `moves` keyword itself; everything else is a move
                    game.play_move_str(token);
                }
            }
        }
    }
    // A FEN that ran to the end of input without a `moves` keyword or a sixth field is applied.
    if matches!(state, State::CollectFen) && !fen.is_empty() {
        game.set_position(&fen);
    }
}

/// Handle the "help" command: list all available commands.
fn handle_help(_engine: &mut Engine, _args: &[String]) {
    uci_println!("Available commands:");
    for handler in HANDLERS {
        uci_println!("{:<12} {}", handler.name, handler.description);
    }
}

/// Table of all supported input commands and their handlers.
pub static HANDLERS: &[Handler] = &[
    Handler {
        function: handle_bench,
        name: "bench",
        description: "Search a fixed position set to fixed depth; print nodes + nps",
    },
    Handler {
        function: handle_bookmoves,
        name: "bookmoves",
        description: "Display available book moves for current position",
    },
    Handler {
        function: handle_dbg,
        name: "dbg",
        description: "Display diagnostic counts",
    },
    Handler {
        function: handle_dbgclear,
        name: "dbgclear",
        description: "Reset diagnostic counts",
    },
    Handler {
        function: handle_eval,
        name: "eval",
        description: "Display the current static evaluation",
    },
    Handler {
        function: handle_freebook,
        name: "freebook",
        description: "Delete the opening book from memory",
    },
    Handler {
        function: handle_getboard,
        name: "getboard",
        description: "Display the FEN of the current position",
    },
    Handler {
        function: handle_go,
        name: "go",
        description: "Search the current position",
    },
    Handler {
        function: handle_help,
        name: "help",
        description: "Display a summary of commands",
    },
    Handler {
        function: handle_isready,
        name: "isready",
        description: "Respond with readyok",
    },
    Handler {
        function: handle_nnue,
        name: "nnue",
        description: "Display the raw NNUE evaluation of the current position",
    },
    Handler {
        function: handle_ponderhit,
        name: "ponderhit",
        description: "Pondered move was played; convert ponder search to a timed search",
    },
    Handler {
        function: handle_position,
        name: "position",
        description: "Set the position and series of moves",
    },
    Handler {
        function: handle_quit,
        name: "quit",
        description: "Exit the program",
    },
    Handler {
        function: handle_setoption,
        name: "setoption",
        description: "Set a UCI option (EvalFile/Hash/Threads/Clear Hash/Move Overhead/OwnBook)",
    },
    Handler {
        function: handle_stop,
        name: "stop",
        description: "Stop searching and return best move found",
    },
    Handler {
        function: handle_uci,
        name: "uci",
        description: "Enter UCI protocol",
    },
    Handler {
        function: handle_ucinewgame,
        name: "ucinewgame",
        description: "UCI mode start new game",
    },
];

/// Tokenize a line of input and dispatch it to the matching command handler. Returns false once
/// the line was `quit` (the caller stops reading input and exits), true otherwise.
pub fn process_input(engine: &mut Engine, line: &str) -> bool {
    let args: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    if args.is_empty() {
        return true;
    }
    for handler in HANDLERS {
        if args[0] == handler.name {
            (handler.function)(engine, &args);
            break;
        }
    }
    args[0] != "quit"
}
