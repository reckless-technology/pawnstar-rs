//! UCI entry point. (Port of `main.cpp`.) Prints the banner, loads the book and net (with the
//! embedded fallbacks — the C++ `.incbin` assembly becomes `include_bytes!`), then runs the UCI
//! input loop.

use std::io::BufRead;
use std::io::Write;

use pawnstar::debug_hashtable::debug_x_clear;
use pawnstar::game::Engine;
use pawnstar::input_handling::process_input;
use pawnstar::version::version_string;

/// The shipped NNUE net embedded into the binary (the C++ src/embedded_net.S), used as a fallback
/// when the on-disk net cannot be loaded. Linked into the engine executable only (not the lib/tests).
static EMBEDDED_NET: &[u8] = include_bytes!("../nnue/pawnstar-v12.bin");

/// The shipped opening book embedded into the binary (the C++ src/embedded_book.S), used as a
/// fallback when the on-disk pawnstar.book cannot be loaded.
static EMBEDDED_BOOK: &[u8] = include_bytes!("../pawnstar.book");

/// Locate a resource file shipped with the engine (the net or the opening book).
/// Tries the path as given (relative to the current working directory) first, then relative to
/// the running executable's directory and that directory's parent (so the engine finds nnue/ and
/// the book when launched from anywhere). Returns the first candidate that exists, or the
/// original path if none do (so the caller's not-found handling — and the embedded fallback —
/// still applies).
fn locate_resource(relpath: &str) -> String {
    if std::path::Path::new(relpath).exists() {
        return relpath.to_string();
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for base in [Some(exe_dir), exe_dir.parent()].into_iter().flatten() {
                let candidate = base.join(relpath);
                if candidate.exists() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }
    }
    relpath.to_string()
}

/// Program entry point. Prints the banner, loads the book, then runs the UCI input loop.
fn main() {
    // (The C++ unbuffers stdin/stdout globally; here every protocol write goes through
    // uci_println!, which flushes.)
    print!(
        "   .::.    \n\
         \x20  _::_    \n\
         \x20_/____\\_ \n\
         \x20\\      / \n\
         \x20 \\____/  \n\
         \x20 (____)   \n\
         \x20  |  |    \n\
         \x20  |__|    \n\
         \x20 /    \\  \n\
         \x20(______)  \n\
         (________) \n\
         /________\\\n\
         Pawnstar: a UCI compatible chess engine\n\
         (C) Jonny Reckless 2009 - 2026\n"
    );
    println!("Version {} Compiled {}", version_string(), env!("PAWNSTAR_BUILD_TIMESTAMP"));
    let _ = std::io::stdout().flush();

    let mut engine = Engine::new();

    // Load the opening book from the working directory; if the file can't be loaded (wrong cwd,
    // missing/renamed file), fall back to the copy embedded in the binary so the engine always
    // has a book. The book is optional, so a failure of both is only a warning, not fatal.
    {
        let game = engine.game_mut();
        let book_path = locate_resource("pawnstar.book");
        if !game.book.initialize(&book_path) {
            if game.book.initialize_from_memory(EMBEDDED_BOOK) {
                println!(
                    "info string book file load failed; using the embedded book ({} bytes)",
                    EMBEDDED_BOOK.len()
                );
            } else {
                println!("info string Unable to open book file (and the embedded fallback failed).");
            }
        }
        // NNUE is the only evaluator, so a net is required. Load the shipped net from the working
        // directory (like the opening book); PAWNSTAR_EVALFILE=<path> overrides the path, and UCI
        // `setoption name EvalFile value <path>` loads a different net at runtime. If the on-disk
        // net cannot be loaded, fall back to the copy embedded in the binary so the engine always
        // has a working evaluator. Only if even that fails — which would mean the build is
        // corrupt — does the engine give up.
        let net_path = std::env::var("PAWNSTAR_EVALFILE").unwrap_or_else(|_| locate_resource("nnue/pawnstar-v12.bin"));
        if !game.nnue_network.load(&net_path) {
            println!(
                "info string NNUE: file load failed; falling back to the embedded net ({} bytes)",
                EMBEDDED_NET.len()
            );
            if !game.nnue_network.load_from_memory(EMBEDDED_NET, "embedded") {
                println!(
                    "info string FATAL: could not load the NNUE net from '{}' or the embedded fallback. The build may be corrupt.",
                    net_path
                );
                std::process::exit(1);
            }
        }
    }
    debug_x_clear();
    println!("ready");
    let _ = std::io::stdout().flush();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        process_input(&mut engine, &line);
    }
}
