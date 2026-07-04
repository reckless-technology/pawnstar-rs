//! Runs the shell-driven UCI integration suite (test/uci_test.sh, copied verbatim from the C++
//! repo) against the freshly built engine binary — the cargo equivalent of the C++ ctest `uci`
//! registration. Cargo builds the `pawnstar` bin for integration tests and exposes its path via
//! CARGO_BIN_EXE_pawnstar.
//!
//! The script needs bash >= 4 (`coproc`); like the CMake build, prefer a Homebrew bash on macOS
//! (the system /bin/bash is 3.2) and skip cleanly when none is available.

mod common;

#[test]
fn uci_shell_suite() {
    let candidates = ["/opt/homebrew/bin/bash", "/usr/local/bin/bash", "bash"];
    let bash = candidates.iter().find(|b| {
        std::process::Command::new(b)
            .args(["-c", "[[ ${BASH_VERSINFO[0]} -ge 4 ]]"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    });
    let Some(bash) = bash else {
        common::summary("UCI: skipped (no bash >= 4 with coproc found)");
        return;
    };
    let engine = env!("CARGO_BIN_EXE_pawnstar");
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/test/uci_test.sh");
    // Run from the repo root (like the C++ ctest WORKING_DIRECTORY) so the engine's cwd-relative
    // net/book lookups resolve.
    let status = std::process::Command::new(bash)
        .arg(script)
        .arg(engine)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("failed to run uci_test.sh");
    assert!(status.success(), "uci_test.sh reported failures");
}
