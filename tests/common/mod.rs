//! Shared, consistent result reporting for the test suites. (Port of `test/test_report.h`.)
//!
//! Every test result line is printed as "[PASS] <summary>" or "[FAIL] <summary>" so that all
//! suites report uniformly. Each suite reports its individual checks with `check(...)` and ends
//! with one `summary(...)` line for the whole suite; the cargo test then asserts zero failures.

use std::sync::atomic::{AtomicI32, Ordering};

/// Number of failed check() calls so far.
pub static FAILURES: AtomicI32 = AtomicI32::new(0);

/// Report one result line: "[PASS] <summary>" or "[FAIL] <summary>"; tally failures.
/// Returns `passed` (so the call can be used in a condition).
pub fn check(passed: bool, summary: &str) -> bool {
    println!("{} {}", if passed { "[PASS]" } else { "[FAIL]" }, summary);
    if !passed {
        FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    passed
}

/// Print the final whole-suite line — "[PASS] <detail>" if every check() passed, else
/// "[FAIL] <detail>" — without itself counting as a check, and return the failure count
/// (the C++ returns the process exit code; the cargo test asserts this is 0).
pub fn summary(detail: &str) -> i32 {
    let failures = FAILURES.load(Ordering::Relaxed);
    println!(
        "{} {}",
        if failures == 0 { "[PASS]" } else { "[FAIL]" },
        detail
    );
    failures
}

/// The shipped net asset, which the engine also embeds.
pub const SHIPPED_NET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/nnue/pawnstar-v20.bin");

/// The net a suite should read: `PAWNSTAR_EVALFILE` where it is set, otherwise the shipped asset.
pub fn shipped_net_path() -> String {
    std::env::var("PAWNSTAR_EVALFILE").unwrap_or_else(|_| SHIPPED_NET.to_string())
}

/// Whether the net at `path` is present **and of this build's NNUE shape**.
///
/// A build at the retired v12 shape (`--no-default-features --features debugx`) rightly refuses
/// the shipped v20 net, naming `format_version` — the container working rather than a fault, since
/// no net of that shape is in the tree any more. A suite that needs a *trained* net therefore skips
/// there, exactly as it skips when the file is absent, which is the C++ exit-0 contract extended to
/// the shape knobs. The shape-independent suites run either way, and a shape with no net of its own
/// is gated by `nnue_incremental_test`'s synthetic net and the node-count signature in
/// `nnue/README.md`.
///
/// Note this reads `Network::load`, which verifies the payload checksums too, so a net *of this
/// shape* that is corrupt would skip here rather than fail. `nnue_container_test` has its own
/// header-only probe for exactly that reason, and it is the suite whose job is to catch corruption.
pub fn is_net_this_builds_shape(path: &str) -> bool {
    std::path::Path::new(path).exists() && pawnstar::nnue::Network::default().load(path).is_loaded()
}

/// Whether the net this suite would read — `PAWNSTAR_EVALFILE` or the shipped asset — is of this
/// build's NNUE shape.
pub fn is_shipped_net_this_builds_shape() -> bool {
    is_net_this_builds_shape(&shipped_net_path())
}
