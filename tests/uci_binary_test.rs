//! The engine binary's start-up: the banner, where it finds the opening book and the net, the
//! embedded fallbacks when it finds neither, `PAWNSTAR_EVALFILE`, and the end of input.
//!
//! Most cases run the built `pawnstar` with their whole input written up front and stdin then
//! closed, so the engine reads to the end of input and returns from `main` (rather than `quit`'s
//! process exit), and assert on what it printed.
//!
//! The one case that must watch a *search* survive something — a refused `setoption name EvalFile`
//! leaving the net already loaded playing — drives the engine interactively instead
//! (`DrivenEngine`), reading each answer before sending the next command: input written up front
//! ends while the search runs, and a search cut short proves nothing about the net behind it.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pawnstar::constants::HASHTABLE_MEGABYTES;
use pawnstar::nnue::{NetHeader, HIDDEN_SIZE, NET_HEADER_BYTES};

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
const ENGINE: &str = env!("CARGO_BIN_EXE_pawnstar");

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

/// Run `engine` in `cwd` with `envs`, feed it `input`, close stdin and return its stdout and
/// stderr once it exits, checking that it exited cleanly.
fn run_engine(engine: &Path, cwd: &Path, envs: &[(&str, &str)], input: &str) -> (String, String) {
    let (exited_cleanly, out, err) = run_engine_allowing_failure(engine, cwd, envs, input);
    check(exited_cleanly, "the engine exits cleanly");
    (out, err)
}

/// The same, for the cases that are *meant* to fail: a refused net stops the engine, so its exit
/// status is part of what is asserted rather than a precondition of the run.
fn run_engine_allowing_failure(
    engine: &Path,
    cwd: &Path,
    envs: &[(&str, &str)],
    input: &str,
) -> (bool, String, String) {
    let mut command = Command::new(engine);
    command
        .current_dir(cwd)
        .env_remove("PAWNSTAR_EVALFILE")
        .env_remove("PAWNSTAR_BENCH_DEBUG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("failed to spawn engine");
    {
        let mut stdin = child.stdin.take().expect("engine stdin");
        stdin.write_all(input.as_bytes()).expect("engine stdin");
    } // dropped: the engine sees the end of input
    let output = child.wait_with_output().expect("engine output");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// An engine driven the way a GUI or a match harness drives it: each command goes out only once
/// the engine's answer to the one before has been read, so a search is never cut short by the end
/// of input (`quit` straight after `go` aborts the search, and the `bestmove` that follows has no
/// search behind it). stdout arrives line by line over a channel; stderr is drained by a thread of
/// its own, because a child that fills a pipe nobody reads stops dead, and `finish` hands back the
/// whole of it — stderr is the stream a match harness keeps.
struct DrivenEngine {
    child: Child,
    stdin: ChildStdin,
    stdout_lines: Receiver<String>,
    stderr_reader: JoinHandle<String>,
    stdout_transcript: String,
}

impl DrivenEngine {
    /// Start the engine in `cwd` with `envs`, its three streams piped.
    fn start(engine: &Path, cwd: &Path, envs: &[(&str, &str)]) -> DrivenEngine {
        let mut command = Command::new(engine);
        command
            .current_dir(cwd)
            .env_remove("PAWNSTAR_EVALFILE")
            .env_remove("PAWNSTAR_BENCH_DEBUG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("failed to spawn engine");
        let stdin = child.stdin.take().expect("engine stdin");
        let stdout = child.stdout.take().expect("engine stdout");
        let stderr = child.stderr.take().expect("engine stderr");
        let (sender, stdout_lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break; // the test has finished with this engine
                }
            }
        });
        let stderr_reader = thread::spawn(move || {
            let mut text = String::new();
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                text.push_str(&line);
                text.push('\n');
            }
            text
        });
        DrivenEngine {
            child,
            stdin,
            stdout_lines,
            stderr_reader,
            stdout_transcript: String::new(),
        }
    }

    /// Send one UCI command (and flush, so the engine sees it at once).
    fn send(&mut self, command: &str) {
        writeln!(self.stdin, "{command}").expect("engine stdin");
        self.stdin.flush().expect("engine stdin");
    }

    /// Read stdout until a line starts with `prefix`, and return everything read, that line
    /// included. On a timeout (or an engine that ended first) it returns what did arrive, so the
    /// caller's own check reports the absence rather than the test panicking inside a helper.
    fn read_until(&mut self, prefix: &str, timeout: Duration) -> String {
        let mut read = String::new();
        while let Ok(line) = self.stdout_lines.recv_timeout(timeout) {
            let is_the_marker = line.starts_with(prefix);
            for text in [&mut self.stdout_transcript, &mut read] {
                text.push_str(&line);
                text.push('\n');
            }
            if is_the_marker {
                break;
            }
        }
        read
    }

    /// `quit`, then the engine's exit status, its whole stdout and its whole stderr.
    fn finish(mut self) -> (bool, String, String) {
        self.send("quit");
        let status = self.child.wait().expect("engine exit");
        while let Ok(line) = self.stdout_lines.recv_timeout(Duration::from_secs(5)) {
            self.stdout_transcript.push_str(&line);
            self.stdout_transcript.push('\n');
        }
        let stderr = self.stderr_reader.join().expect("the stderr reader");
        (status.success(), self.stdout_transcript, stderr)
    }
}

/// `ucinewgame` (so the transposition table and the eval cache start empty and two searches of the
/// same position are comparable node for node) then a fixed-depth search from the start position.
/// Returns the best move and the node count of the last iteration that reported one.
fn search_the_start_position(engine: &mut DrivenEngine, depth: u32) -> (String, String) {
    engine.send("ucinewgame");
    engine.send("isready");
    engine.read_until("readyok", Duration::from_secs(30));
    engine.send("position startpos");
    engine.send(&format!("go depth {depth}"));
    let search = engine.read_until("bestmove ", Duration::from_secs(120));
    let best_move = search
        .lines()
        .find(|l| l.starts_with("bestmove "))
        .unwrap_or_default()
        .to_string();
    // `info depth N ... nodes <count> ...`: the count from the deepest iteration reported.
    let nodes = search
        .lines()
        .filter(|l| l.starts_with("info depth"))
        .filter_map(|l| {
            let mut fields = l.split_whitespace();
            while let Some(field) = fields.next() {
                if field == "nodes" {
                    return fields.next().map(str::to_string);
                }
            }
            None
        })
        .next_back()
        .unwrap_or_default();
    (best_move, nodes)
}

/// A fresh, empty scratch directory of this process's own.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pawnstar-binary-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

fn has_line(text: &str, prefix: &str) -> bool {
    text.lines().any(|l| l.starts_with(prefix))
}

/// From the repository root the engine finds both files where it stands, prints its banner,
/// answers the handshake, and evaluates with the shipped net.
fn test_start_in_repo() {
    let (out, err) = run_engine(
        Path::new(ENGINE),
        Path::new(MANIFEST_DIR),
        &[],
        "uci\nisready\nposition startpos\neval\nnnue\nbookmoves\nhelp\n",
    );
    check(
        out.contains("Pawnstar: a UCI compatible chess engine"),
        "the banner",
    );
    check(has_line(&out, "Version "), "the version line");
    check(has_line(&out, "ready"), "ready after loading");
    check(
        !out.contains("embedded"),
        "no fallback from the repository root",
    );
    check(
        err.contains("loaded net 'nnue/pawnstar-v12.bin'"),
        "the net from the working directory",
    );
    check(
        has_line(&out, "uciok") && has_line(&out, "readyok"),
        "the handshake",
    );
    // The Hash default is pinned to the exact number the handshake prints. pawnstar, pawnstar-rs and
    // pawnstar-go are bit-exact with each other at a fixed depth only while their transposition tables
    // are the same size - table size changes the collision pattern and so the node count - and `bench`
    // pins 64 MB in all three, so the one cross-port gate there was could not see a default that had
    // drifted. pawnstar-go's had, to 512, and every UCI `go` the three answered diverged for months
    // (diagnosed 2026-10-27). Changing this number means changing it in all three ports together;
    // claude-skills' reckless-nnue/scripts/hash-default-parity.sh compares the three binaries.
    check(
        has_line(&out, "option name Hash type spin default 64 min 1 max 4096")
            && HASHTABLE_MEGABYTES == 64,
        "the Hash default is 64, the value the C++ and Go ports advertise too",
    );
    check(
        has_line(&out, "evaluation ") && has_line(&out, "nnue "),
        "eval and nnue evaluate",
    );
    check(
        has_line(&out, "MOVE   COUNT"),
        "the book lists the start position's moves",
    );
    check(
        has_line(&out, "Available commands:"),
        "help lists the commands",
    );
}

/// PAWNSTAR_EVALFILE naming a missing file: the engine says so and plays on the embedded net.
fn test_evalfile_override() {
    let (out, err) = run_engine(
        Path::new(ENGINE),
        Path::new(MANIFEST_DIR),
        &[("PAWNSTAR_EVALFILE", "no/such/net.bin")],
        "position startpos\neval\n",
    );
    check(
        err.contains("cannot open net file 'no/such/net.bin'"),
        "the override is the path tried",
    );
    check(
        out.contains("falling back to the embedded net"),
        "a missing override falls back",
    );
    check(has_line(&out, "evaluation "), "the embedded net evaluates");

    // An override naming the shipped net loads it from there.
    let net = Path::new(MANIFEST_DIR)
        .join("nnue")
        .join("pawnstar-v12.bin");
    let net = net.to_string_lossy().into_owned();
    let (out, err) = run_engine(
        Path::new(ENGINE),
        &scratch("override"),
        &[("PAWNSTAR_EVALFILE", &net)],
        "",
    );
    check(
        err.contains(&format!("loaded net '{net}'")),
        "the override's net loads",
    );
    check(
        !out.contains("embedded net"),
        "a good override needs no fallback",
    );
}

/// From a directory holding neither file, with neither beside the executable, the engine falls
/// back to the copies built into it and still plays.
fn test_embedded_fallbacks() {
    let cwd = scratch("empty");
    let (out, _) = run_engine(
        Path::new(ENGINE),
        &cwd,
        &[("PAWNSTAR_THREADS", "1")],
        "setoption name OwnBook value true\nposition startpos\nbookmoves\neval\n",
    );
    check(out.contains("using the embedded book"), "the embedded book");
    check(
        out.contains("falling back to the embedded net"),
        "the embedded net",
    );
    check(
        has_line(&out, "MOVE   COUNT"),
        "the embedded book has the start position's moves",
    );
    check(has_line(&out, "evaluation "), "the embedded net evaluates");
    let _ = std::fs::remove_dir_all(&cwd);
}

/// An engine copied into `<dir>/bin` with the book and the net in `<dir>` finds them through its
/// own location when started anywhere else.
fn test_resources_beside_the_executable() {
    let root = scratch("installed");
    let bin = root.join("bin");
    std::fs::create_dir_all(bin.join("nnue")).expect("bin directory");
    std::fs::create_dir_all(root.join("nnue")).expect("nnue directory");
    let engine = bin.join(Path::new(ENGINE).file_name().expect("engine file name"));
    std::fs::copy(ENGINE, &engine).expect("copy the engine");
    let repo = Path::new(MANIFEST_DIR);
    // The book in the executable's parent, the net in the executable's own directory.
    std::fs::copy(repo.join("pawnstar.book"), root.join("pawnstar.book")).expect("copy the book");
    std::fs::copy(
        repo.join("nnue").join("pawnstar-v12.bin"),
        bin.join("nnue").join("pawnstar-v12.bin"),
    )
    .expect("copy the net");

    let cwd = scratch("elsewhere");
    let (out, err) = run_engine(&engine, &cwd, &[], "position startpos\nbookmoves\n");
    check(
        !out.contains("embedded"),
        "the files beside the executable need no fallback",
    );
    check(
        has_line(&out, "MOVE   COUNT"),
        "the book from the executable's parent",
    );
    check(
        err.contains("loaded net '"),
        "the net from the executable's directory",
    );
    let _ = std::fs::remove_dir_all(&cwd);
    let _ = std::fs::remove_dir_all(&root);
}

/// A book file that fails to parse is refused and the embedded book used in its place.
fn test_broken_book_falls_back() {
    let cwd = scratch("broken-book");
    std::fs::write(cwd.join("pawnstar.book"), "e2e4 e7e5 e1e8\n").expect("broken book");
    let (out, _) = run_engine(Path::new(ENGINE), &cwd, &[], "");
    check(
        out.contains("Error found in book line"),
        "the bad line is reported",
    );
    check(
        out.contains("using the embedded book"),
        "a broken book falls back",
    );
    let _ = std::fs::remove_dir_all(&cwd);
}

/// `quit` stops a running search, which answers, and ends the engine; nothing after it is read.
/// The search is an infinite one and `quit` follows at once, before the worker has begun the root
/// search (which once cleared the cancel flag as it started, losing the quit for ever).
fn test_quit() {
    let (out, _) = run_engine(
        Path::new(ENGINE),
        Path::new(MANIFEST_DIR),
        &[("PAWNSTAR_THREADS", "1")],
        "setoption name OwnBook value false\nposition startpos\ngo infinite\nquit\nisready\n",
    );
    check(
        has_line(&out, "bestmove "),
        "the search answers before the engine quits",
    );
    check(!has_line(&out, "readyok"), "nothing after quit is read");
}

/// A net that is *there* and fails one of the container's checks stops the engine. It must not be
/// quietly replaced by the embedded net: an engine that substituted one would play a whole match
/// on weights nobody asked for, and the match would report a plausible Elo for a net that never
/// played a move.
fn test_refused_net_does_not_fall_back() {
    let net = Path::new(MANIFEST_DIR)
        .join("nnue")
        .join("pawnstar-v12.bin");
    let Ok(mut image) = std::fs::read(&net) else {
        check(
            true,
            "the shipped net is absent; the refusal case is skipped",
        );
        return;
    };
    // One flipped bit deep in the weights: the header still describes this engine's architecture
    // exactly, so only the payload's checksum catches it.
    let at = image.len() / 2;
    image[at] ^= 0x01;
    let dir = scratch("refused");
    let corrupt = dir.join("corrupt-net.bin");
    std::fs::write(&corrupt, &image).expect("write the corrupt net");
    let corrupt = corrupt.to_string_lossy().into_owned();

    let (exited_cleanly, out, err) = run_engine_allowing_failure(
        Path::new(ENGINE),
        Path::new(MANIFEST_DIR),
        &[("PAWNSTAR_EVALFILE", &corrupt)],
        "position startpos\neval\n",
    );
    check(
        err.contains("refused: payload_crc is"),
        "the refusal names payload_crc and both values",
    );
    check(
        !out.contains("falling back to the embedded net"),
        "a refused net is not replaced by the embedded one",
    );
    check(
        out.contains("was refused"),
        "the engine says on stdout that the net was refused",
    );
    check(!exited_cleanly, "the engine stops rather than playing on");
    check(
        !has_line(&out, "evaluation "),
        "nothing is evaluated with a refused net",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `setoption name EvalFile` naming an incompatible net, mid-session: the refusal must be **loud
/// on both streams** and must say which net is playing instead.
///
/// Keeping the net already loaded is right — the alternative is an engine with no evaluator, or
/// one playing weights nobody asked for. Being quiet about it was the defect: the refusal went to
/// stderr alone, the kept-net line to stdout alone as an `info string`, and it named no net at
/// all. A match harness keeps stderr and routinely drops `info` lines, so it set `EvalFile`, saw
/// `readyok`, and ran a thousand games whose result it then attributed to a net that never played
/// a move.
fn test_refused_evalfile_is_loud_and_names_the_kept_net() {
    // An incompatible net built from the shipped one: `hidden_size` one higher than this build's,
    // with `header_crc` re-stamped over it, so it is refused on an architecture field rather than
    // on a checksum — the same shape of mistake as pointing the engine at another engine's net.
    let shipped = Path::new(MANIFEST_DIR)
        .join("nnue")
        .join("pawnstar-v12.bin");
    let Ok(image) = std::fs::read(&shipped) else {
        check(
            true,
            "the shipped net is absent; the refused-EvalFile case is skipped",
        );
        return;
    };
    let file_hidden_size = HIDDEN_SIZE as u16 + 1;
    let mut header = NetHeader::from_bytes(&image);
    header.hidden_size = file_hidden_size;
    let mut incompatible = header.with_header_crc().to_bytes().to_vec();
    incompatible.extend_from_slice(&image[NET_HEADER_BYTES..]);
    let dir = scratch("incompatible-evalfile");
    let incompatible_path = dir.join("incompatible-net.bin");
    std::fs::write(&incompatible_path, &incompatible).expect("write the incompatible net");
    let incompatible_path = incompatible_path.to_string_lossy().into_owned();

    // Started from the repository root, so the net now playing is the shipped one, by that name.
    const KEPT_NET: &str = "nnue/pawnstar-v12.bin";
    let mut engine = DrivenEngine::start(
        Path::new(ENGINE),
        Path::new(MANIFEST_DIR),
        &[("PAWNSTAR_THREADS", "1")],
    );
    check(
        has_line(
            &engine.read_until("ready", Duration::from_secs(60)),
            "ready",
        ),
        "the engine is up on the net it found in the working directory",
    );
    engine.send("uci");
    check(
        has_line(
            &engine.read_until("uciok", Duration::from_secs(30)),
            "uciok",
        ),
        "the handshake before the refusal",
    );
    // Without this, `go` answers from the opening book instantly and no search runs at all.
    engine.send("setoption name OwnBook value false");

    // A real search on the net loaded at start-up, to compare the one after the refusal against.
    let (best_move_before, nodes_before) = search_the_start_position(&mut engine, 12);
    check(
        best_move_before.starts_with("bestmove ") && !nodes_before.is_empty(),
        "the search before the refused EvalFile is a real one (bestmove and a node count)",
    );

    engine.send(&format!(
        "setoption name EvalFile value {incompatible_path}"
    ));
    engine.send("isready");
    let answer = engine.read_until("readyok", Duration::from_secs(30));
    let refusal = format!(
        "NNUE: net '{incompatible_path}' refused: hidden_size is {file_hidden_size} in the file but {HIDDEN_SIZE} in this engine"
    );
    let kept = format!(
        "NNUE: keeping the net already loaded ('{KEPT_NET}'); '{incompatible_path}' was refused and is not in play"
    );
    check(
        answer
            .lines()
            .any(|l| l == format!("info string {refusal}")),
        "stdout carries the refusal as an info string, naming the field and both values",
    );
    check(
        answer.lines().any(|l| l == format!("info string {kept}")),
        "stdout says which net is still playing",
    );
    check(
        has_line(&answer, "readyok"),
        "the engine is ready again after turning the net down",
    );

    // The engine still searches, and searches with the net it kept: from a cleared table, at the
    // same depth, the same position gives the same best move and the same node count. A node count
    // that moved would mean the refused net had reached the evaluation after all.
    let (best_move_after, nodes_after) = search_the_start_position(&mut engine, 12);
    check(
        best_move_after == best_move_before && !best_move_after.is_empty(),
        &format!(
            "the search after the refusal returns the same best move ({best_move_before} / {best_move_after})"
        ),
    );
    check(
        nodes_after == nodes_before && !nodes_after.is_empty(),
        &format!(
            "the search after the refusal is the same search ({nodes_before} nodes / {nodes_after} nodes)"
        ),
    );

    let (exited_cleanly, out, err) = engine.finish();
    check(
        exited_cleanly,
        "the engine quits cleanly: a refused EvalFile mid-session is not fatal",
    );
    check(
        err.lines().any(|l| l == refusal),
        "stderr carries the same refusal, without the info string prefix",
    );
    check(
        err.lines().any(|l| l == kept),
        "stderr says which net is still playing, without the info string prefix",
    );
    check(
        !out.contains("embedded") && !err.contains("embedded"),
        "nothing falls back to the embedded net",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uci_binary_suite() {
    // The engine binary plays on the net it embeds — the shipped v12 asset — and a build at another
    // NNUE shape refuses it and exits, which is the container working rather than a fault: nothing
    // has been trained at that shape yet. These checks are about the binary's start-up, resources
    // and command handling over a net it can read, so they stand down there.
    if !common::is_net_this_builds_shape(common::SHIPPED_NET) {
        common::summary("UCI binary: skipped (the embedded net is not this build's NNUE shape)");
        return;
    }
    test_start_in_repo();
    test_evalfile_override();
    test_refused_net_does_not_fall_back();
    test_refused_evalfile_is_loud_and_names_the_kept_net();
    test_embedded_fallbacks();
    test_resources_beside_the_executable();
    test_broken_book_falls_back();
    test_quit();
    assert_eq!(
        common::summary(
            "UCI binary: banner, resource lookup, embedded fallbacks, a refused EvalFile, quit"
        ),
        0,
        "uci binary suite reported failures"
    );
}
