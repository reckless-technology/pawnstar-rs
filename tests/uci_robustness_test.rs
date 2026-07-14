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

    fn quit(mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();
        std::thread::sleep(Duration::from_millis(50));
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

#[test]
fn uci_robustness_suite() {
    test_malformed_fen_does_not_panic();
    test_no_bestmove_while_pondering();
    assert_eq!(
        common::summary("UCI robustness: malformed FEN totality, no bestmove while pondering"),
        0,
        "uci robustness suite reported failures"
    );
}
