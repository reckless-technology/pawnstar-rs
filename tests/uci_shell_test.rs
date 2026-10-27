//! Runs the shell-driven UCI integration suite (test/uci_test.sh, copied verbatim from the C++
//! repo) against the freshly built engine binary — the cargo equivalent of the C++ ctest `uci`
//! registration. Cargo builds the `pawnstar` bin for integration tests and exposes its path via
//! CARGO_BIN_EXE_pawnstar.
//!
//! The script needs bash >= 4 (`coproc`); like the CMake build, prefer a Homebrew bash on macOS
//! (the system /bin/bash is 3.2) and skip cleanly when none is available. On Windows the bash must
//! be Git for Windows' (or MSYS2's), which reads C:\... paths, and never the bare name: Windows
//! resolves `bash` in System32 before the PATH, where it is WSL's launcher, and that bash runs in
//! Linux and mangles the Windows path of the script.

mod common;

use std::path::PathBuf;
use std::process::Command;

/// The bashes to try, in order: the C++ build's `find_program` hints, then (on Windows) the bash
/// of the Git that `git --exec-path` names, which covers a per-user install.
fn bash_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        let mut candidates = Vec::new();
        for root in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(dir) = std::env::var_os(root) {
                let git = PathBuf::from(dir).join("Git");
                candidates.push(git.join("bin").join("bash.exe"));
                candidates.push(git.join("usr").join("bin").join("bash.exe"));
            }
        }
        // <git>/mingw64/libexec/git-core -> <git>/bin/bash.exe
        if let Ok(out) = Command::new("git").arg("--exec-path").output() {
            let exec_path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if let Some(git) = exec_path.ancestors().nth(3) {
                candidates.push(git.join("bin").join("bash.exe"));
            }
        }
        candidates.push(PathBuf::from(r"C:\msys64\usr\bin\bash.exe"));
        candidates
    } else {
        ["/opt/homebrew/bin/bash", "/usr/local/bin/bash", "bash"]
            .iter()
            .map(PathBuf::from)
            .collect()
    }
}

#[test]
fn uci_shell_suite() {
    // The engine binary plays on the net it embeds — the shipped v12 asset — and a build at another
    // NNUE shape refuses it and exits, which is the container working rather than a fault: nothing
    // has been trained at that shape yet. These checks are about the binary's start-up, resources
    // and command handling over a net it can read, so they stand down there.
    if !common::is_net_this_builds_shape(common::SHIPPED_NET) {
        common::summary("UCI: skipped (the embedded net is not this build's NNUE shape)");
        return;
    }
    let bash = bash_candidates().into_iter().find(|b| {
        Command::new(b)
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
    let status = Command::new(&bash)
        .arg(script)
        .arg(engine)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("failed to run uci_test.sh");
    assert!(status.success(), "uci_test.sh reported failures");
}
