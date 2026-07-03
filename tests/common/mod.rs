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
    println!("{} {}", if failures == 0 { "[PASS]" } else { "[FAIL]" }, detail);
    failures
}
