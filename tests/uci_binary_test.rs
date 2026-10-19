//! The engine binary's start-up: the banner, where it finds the opening book and the net, the
//! embedded fallbacks when it finds neither, `PAWNSTAR_EVALFILE`, and the end of input.
//!
//! Each case runs the built `pawnstar` with its whole input written up front and stdin then
//! closed, so the engine reads to the end of input and returns from `main` (rather than `quit`'s
//! process exit), and asserts on what it printed.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
const ENGINE: &str = env!("CARGO_BIN_EXE_pawnstar");

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

/// Run `engine` in `cwd` with `envs`, feed it `input`, close stdin and return its stdout and
/// stderr once it exits.
fn run_engine(engine: &Path, cwd: &Path, envs: &[(&str, &str)], input: &str) -> (String, String) {
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
    check(output.status.success(), "the engine exits cleanly");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
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

#[test]
fn uci_binary_suite() {
    test_start_in_repo();
    test_evalfile_override();
    test_embedded_fallbacks();
    test_resources_beside_the_executable();
    test_broken_book_falls_back();
    test_quit();
    assert_eq!(
        common::summary("UCI binary: banner, resource lookup, embedded fallbacks, quit"),
        0,
        "uci binary suite reported failures"
    );
}
