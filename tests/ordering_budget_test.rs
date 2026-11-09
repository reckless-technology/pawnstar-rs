//! The root's depth-1 ordering pass under a hard limit, seen from the protocol.
//!
//! Under a hard deadline or `go nodes`, `search_root_node` first searches every root move to depth
//! 1 (PV extension off), prints `info depth 1`, and only then runs the depth-3 pass. On the FEN of
//! the 67 games where Pawnstar 2.2.0 answered with no `info` line, a depth-3 pass from a cold table
//! takes 276,020 nodes, far more than any small budget, so before this change `go nodes 4096`,
//! `go nodes 65536` and the clock below each printed no `info` line at all. With no hard limit
//! (`go depth`) nothing changes and the passes print nothing.
//!
//! The search runs in the engine binary, so the lines it prints are read from its stdout. Skipped
//! (passes) when the shipped net is absent or not of this build's shape.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

const ENGINE: &str = env!("CARGO_BIN_EXE_pawnstar");

/// The position of the 67 games: Black to move, b2 one step from promoting.
const FEN: &str = "3k3b/3P1KP1/8/5B2/7p/7P/1p6/8 b - - 0 92";

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

/// A single-threaded engine process with an empty table, the book off and the repro position set.
struct Engine {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Engine {
    fn start() -> Engine {
        let mut child = Command::new(ENGINE)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env_remove("PAWNSTAR_EVALFILE")
            .env_remove("PAWNSTAR_BENCH_DEBUG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn engine");
        let stdin = child.stdin.take().expect("engine stdin");
        let stdout = child.stdout.take().expect("engine stdout");
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut engine = Engine {
            child,
            stdin,
            lines,
        };
        engine.send("setoption name Threads value 1");
        engine.send("setoption name Hash value 16");
        engine.send("setoption name OwnBook value false");
        engine.send("isready");
        engine.read_until("readyok");
        engine
    }

    fn send(&mut self, command: &str) {
        writeln!(self.stdin, "{command}").expect("engine stdin");
        self.stdin.flush().expect("engine stdin");
    }

    /// The lines read up to and including the first that starts with `prefix`.
    fn read_until(&mut self, prefix: &str) -> Vec<String> {
        let mut read = Vec::new();
        while let Ok(line) = self.lines.recv_timeout(Duration::from_secs(120)) {
            let is_the_marker = line.starts_with(prefix);
            read.push(line);
            if is_the_marker {
                break;
            }
        }
        read
    }

    /// Search the repro position from an empty table with `go_command`; the lines up to `bestmove`.
    fn search(&mut self, go_command: &str) -> Vec<String> {
        self.send("ucinewgame");
        self.send(&format!("position fen {FEN}"));
        self.send(go_command);
        self.read_until("bestmove")
    }

    fn finish(mut self) {
        self.send("quit");
        let _ = self.child.wait();
    }
}

fn is_info_depth(line: &str) -> bool {
    line.starts_with("info depth")
}

fn test_limited_searches_print_depth_1() {
    for go_command in [
        "go nodes 4096",
        "go nodes 65536",
        "go wtime 664 btime 1110 winc 80 binc 80",
    ] {
        let mut engine = Engine::start();
        let lines = engine.search(go_command);
        engine.finish();
        let bestmove = lines.last().map(String::as_str).unwrap_or("");
        let first_info = lines.iter().find(|line| is_info_depth(line));
        check(
            bestmove.starts_with("bestmove ") && !bestmove.starts_with("bestmove 0000"),
            &format!("{go_command}: ends in a legal bestmove ({bestmove})"),
        );
        check(
            first_info.is_some_and(|line| line.starts_with("info depth  1 ")),
            &format!("{go_command}: prints an info depth 1 line first ({first_info:?})"),
        );
    }
}

fn test_unlimited_search_prints_no_ordering_info() {
    let mut engine = Engine::start();
    let lines = engine.search("go depth 3");
    engine.finish();
    check(
        lines.iter().any(|line| line.starts_with("bestmove ")),
        "go depth 3 answers",
    );
    check(
        !lines.iter().any(|line| is_info_depth(line)),
        &format!("go depth 3 prints no info line (no hard limit): {lines:?}"),
    );
}

#[test]
fn ordering_budget_suite() {
    if !common::is_shipped_net_this_builds_shape() {
        common::summary("ORDERING BUDGET: skipped (no net of this build's NNUE shape)");
        return;
    }
    test_limited_searches_print_depth_1();
    test_unlimited_search_prints_no_ordering_info();
    let failures = common::summary("ORDERING BUDGET TESTS");
    assert_eq!(failures, 0);
}
