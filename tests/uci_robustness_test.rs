//! Regression tests for two UCI-protocol bugs: a panic on malformed input, and `bestmove` emitted
//! while pondering.
//!
//! 1. The FEN parser was total-input-hostile. `position fen 8/8` (no kings), `position fen garbage`
//!    (unrecognised letters), `position fen ///////` and `position fen 99999999/99999999 w` (files
//!    walking off the board) all panicked with an index-out-of-bounds — a kingless position makes
//!    `king_location` the lsb of an empty bitboard (64), which indexes one past every 64-entry
//!    attack table. An engine that crashes on a malformed line forfeits the game, so parsing is now
//!    total: anything unrepresentable falls back to the start position.
//!
//! 2. `go ponder` answered with `bestmove` before the GUI resolved the ponder, because the root
//!    search left the shared cancel flag set (it uses it to wind up the Lazy SMP helpers), which
//!    made the ponder-wait guard in `search_thread_entry` dead code. The engine must stay silent
//!    until `ponderhit` or `stop`.
//!
//! 3. A `stop` or `quit` sent straight after `go` was lost when it reached the engine before the
//!    worker had begun the root search, which cleared the cancel flag as it started: the search
//!    never ended. Only the thread that starts a search clears the flag now, before it starts it.
//!
//! 4. A `go` value that is not an integer is ignored with its keyword (it read as 0 before; the C++
//!    engine's std::stoi threw and ended the process).

mod common;

use pawnstar::game::START_FEN;
use pawnstar::position::Position;
use pawnstar::square::Square;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

/// A running engine process whose stdout is drained by a reader thread into `lines`.
struct Engine {
    child: Child,
    stdin: ChildStdin,
    lines: Arc<Mutex<Vec<String>>>,
}

impl Engine {
    fn start() -> Engine {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pawnstar"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to spawn engine");
        let stdin = child.stdin.take().expect("engine stdin");
        let stdout = child.stdout.take().expect("engine stdout");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                sink.lock().unwrap().push(line);
            }
        });
        Engine {
            child,
            stdin,
            lines,
        }
    }

    fn send(&mut self, command: &str) {
        writeln!(self.stdin, "{command}").expect("engine stdin closed");
        self.stdin.flush().expect("engine stdin flush");
    }

    /// How many lines so far start with `prefix`.
    fn count(&self, prefix: &str) -> usize {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with(prefix))
            .count()
    }

    /// Wait up to `timeout_ms` for at least `n` lines starting with `prefix`.
    fn wait_for_count(&self, prefix: &str, n: usize, timeout_ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while Instant::now() < deadline {
            if self.count(prefix) >= n {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    /// Wait up to `timeout_ms` for the process to exit on its own; kill it if it has not.
    fn exits_within(&mut self, timeout_ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        false
    }

    /// Whether any line so far starts with `prefix`.
    fn saw(&self, prefix: &str) -> bool {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with(prefix))
    }

    /// Wait up to `timeout_ms` for a line starting with `prefix`. Returns false on timeout (rather
    /// than blocking forever — a fix that never releases the ponder wait would hang here).
    fn wait_for(&self, prefix: &str, timeout_ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while Instant::now() < deadline {
            if self.saw(prefix) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// Send `quit` and give the engine up to five seconds to exit on its own - long enough for an
    /// instrumented build to write its coverage profile - before killing it.
    fn quit(mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A position the engine mates in one (Ra1-a8#): iterative deepening ends on its own almost
/// immediately, which is exactly the case that used to answer while still pondering.
const MATE_IN_ONE_FEN: &str = "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1";

/// Bug 1: malformed FENs must not panic, and must leave the engine in a playable position.
fn test_malformed_fen_does_not_panic() {
    // Every one of these panicked in the FEN parser before the fix; the empty field is the bare
    // `position fen`, whose board field is missing entirely.
    let malformed = ["8/8", "garbage", "///////", "99999999/99999999 w", ""];
    for fen in malformed {
        let position = Position::from_string(fen); // panicked here before the fix
        check(
            position.has_both_kings(),
            &format!("malformed FEN {fen:?} falls back to a legal position"),
        );
        check(
            position.to_string() == START_FEN,
            &format!("malformed FEN {fen:?} falls back to the start position"),
        );
        check(
            position.generate_legal_moves().size() == 20,
            &format!("malformed FEN {fen:?} leaves a searchable position (20 legal moves)"),
        );
    }

    // The fallback must be a no-op for valid input: every valid FEN still parses exactly as before
    // (this is what preserves the bit-identical search trees across the Go / C++ / Rust ports).
    let valid = [
        START_FEN,
        "r1bqkb1r/4npp1/p1p4p/1p1pP1B1/8/1B6/PPPN1PPP/R2Q1RK1 w kq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3", // en-passant field
    ];
    for fen in valid {
        check(
            Position::from_string(fen).to_string() == fen,
            &format!("valid FEN round-trips unchanged: {fen}"),
        );
    }

    // The en-passant field is untrusted too: a short or off-board name must yield the no-square
    // sentinel instead of panicking / underflowing past the 64-entry en-passant hash table.
    check(
        Square::from_str("f6") == Square::from_file_rank(5, 5),
        "square from_str parses a valid name",
    );
    check(
        Square::from_str("f") == Square(0) && Square::from_str("") == Square(0),
        "square from_str returns the no-square sentinel for a short name",
    );
    check(
        Square::from_str("z9") == Square(0),
        "square from_str returns the no-square sentinel for an off-board name",
    );
}

/// Bug 2: `go ponder` must not emit `bestmove` until the GUI sends `ponderhit` or `stop` — and both
/// of those must release it (a fix that never releases would hang the engine).
fn test_no_bestmove_while_pondering() {
    for resolver in ["ponderhit", "stop"] {
        let mut engine = Engine::start();
        engine.send("uci");
        check(
            engine.wait_for("uciok", 5000),
            &format!("[{resolver}] engine handshakes"),
        );
        engine.send(&format!("position fen {MATE_IN_ONE_FEN}"));
        engine.send("go ponder");
        // The mate-in-one search ends within milliseconds; before the fix `bestmove` appeared here.
        std::thread::sleep(Duration::from_millis(500));
        check(
            !engine.saw("bestmove"),
            &format!(
                "[{resolver}] silent while pondering (no bestmove before the GUI resolves it)"
            ),
        );
        engine.send(resolver);
        check(
            engine.wait_for("bestmove", 5000),
            &format!("[{resolver}] emits bestmove once the ponder is resolved"),
        );
        engine.quit();
    }
}

/// Bug 3: a `stop` or `quit` written straight after `go`, before the engine's worker has begun the
/// root search, must still end the search. The root search used to clear the cancel flag as it
/// began, so such a stop was lost: the engine never answered `go infinite` + `stop`, and never
/// exited on `go infinite` + `quit` (every attempt hung before the fix).
fn test_stop_and_quit_straight_after_go() {
    const SETUP: &str = "setoption name OwnBook value false\nsetoption name Threads value 1";
    let mut engine = Engine::start();
    engine.send(SETUP);
    engine.send("isready");
    check(
        engine.wait_for("readyok", 30000),
        "[stop at once] engine ready",
    );
    const STOPS: usize = 300;
    let mut answered = 0;
    for i in 1..=STOPS {
        engine.send("position startpos\ngo infinite\nstop");
        if !engine.wait_for_count("bestmove", i, 30000) {
            break; // a lost stop: the engine is wedged
        }
        answered += 1;
    }
    check(
        answered == STOPS,
        &format!("go infinite + stop at once: {answered}/{STOPS} answered"),
    );
    if answered == STOPS {
        engine.quit();
    } else {
        let _ = engine.child.kill();
        let _ = engine.child.wait();
    }

    const QUITS: usize = 20;
    let mut exited = 0;
    for _ in 0..QUITS {
        let mut engine = Engine::start();
        engine.send(SETUP);
        engine.send("position startpos\ngo infinite\nquit");
        if !engine.exits_within(30000) {
            break; // one lost quit is the failure; no need to wait out the rest
        }
        exited += 1;
    }
    check(
        exited == QUITS,
        &format!("go infinite + quit at once: {exited}/{QUITS} engines exited"),
    );
}

/// Bug 4: a `go` value that is not an integer, or missing, is ignored with its keyword - the line
/// reads as a plain `go`, which `stop` ends - and the engine carries on.
fn test_malformed_go_values() {
    let mut engine = Engine::start();
    engine.send("setoption name OwnBook value false\nsetoption name Threads value 1");
    let lines = [
        "go depth abc",
        "go movetime",
        "go wtime x btime y",
        "go nodes 12x",
        "go depth +-3",
    ];
    for (i, line) in lines.iter().enumerate() {
        engine.send("position startpos");
        engine.send(line);
        engine.send("stop");
        check(
            engine.wait_for_count("bestmove", i + 1, 30000),
            &format!("{line}: the engine survives and stop answers"),
        );
    }
    engine.send("isready");
    check(
        engine.wait_for("readyok", 30000),
        "still responsive after malformed go values",
    );
    engine.quit();
}

#[test]
fn uci_robustness_suite() {
    // The engine binary plays on the net it embeds — the shipped v20 asset, at the shared shape
    // this build defaults to — and a build at the retired v12 shape refuses it and exits, which is
    // the container working rather than a fault: no net of that shape is in the tree any more.
    // These checks are about the binary's start-up, resources and command handling over a net it
    // can read, so they stand down there.
    if !common::is_net_this_builds_shape(common::SHIPPED_NET) {
        common::summary(
            "UCI robustness: skipped (the embedded net is not this build's NNUE shape)",
        );
        return;
    }
    test_malformed_fen_does_not_panic();
    test_no_bestmove_while_pondering();
    test_stop_and_quit_straight_after_go();
    test_malformed_go_values();
    assert_eq!(
        common::summary("UCI robustness: malformed FEN totality, no bestmove while pondering, stop and quit straight after go, malformed go values"),
        0,
        "uci robustness suite reported failures"
    );
}
