//! Unit tests for the ChessClock timing logic. (Port of `test/chess_clock_test.cpp`.)
//!
//! Exercises the parts the fixed-depth suites don't: arming/checking the hard deadline, elapsed
//! time since search start, the ponderhit budget conversion, the "no hard deadline" sentinel, and
//! reset(). Uses real (short) sleeps on the monotonic clock. Bounds are deliberately generous on
//! the "must have elapsed" side so the test stays reliable under CPU load.

mod common;

use pawnstar::chess_clock::{ChessClock, ClockType};

fn check(condition: bool, message: &str) {
    common::check(condition, message);
}

fn sleep(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

#[test]
fn chess_clock_suite() {
    // 1. start_search arms a hard deadline that fires once the maximum budget elapses (not before).
    {
        let clock = ChessClock::new();
        clock.start_search(/*allocated=*/ 100, /*maximum=*/ 500);
        check(!clock.has_reached_hard_deadline(), "deadline not reached immediately after StartSearch");
        check(clock.allocated_time() == 100, "AllocatedTime returns the soft budget");
        sleep(100);
        check(!clock.has_reached_hard_deadline(), "deadline not reached before the maximum elapses");
        sleep(600); // total ~700ms > 500ms maximum
        check(clock.has_reached_hard_deadline(), "deadline reached after the maximum elapses");
    }

    // 2. A non-positive maximum means "no hard deadline" (fixed-depth / infinite): never reached.
    {
        let clock = ChessClock::new();
        clock.start_search(/*allocated=*/ 0, /*maximum=*/ 0);
        sleep(60);
        check(
            !clock.has_reached_hard_deadline(),
            "zero maximum => no hard deadline (fixed-depth / infinite)",
        );
    }

    // 3. Elapsed time grows from the search start (lower bound only — robust under load).
    {
        let clock = ChessClock::new();
        clock.start_search(/*allocated=*/ 1000, /*maximum=*/ 1000);
        sleep(80);
        check(clock.elapsed_since_search_start() >= 70, "elapsed >= time slept since StartSearch");
    }

    // 4. A ponder search has NO hard deadline until ponderhit; ponderhit then arms it from that
    //    moment and resets the elapsed origin (budget-from-ponderhit).
    {
        let clock = ChessClock::new();
        clock.start_ponder_search(/*allocated=*/ 100, /*maximum=*/ 200);
        sleep(260); // longer than the maximum: a real deadline would have fired
        check(!clock.has_reached_hard_deadline(), "ponder search has no deadline before ponderhit");
        check(clock.elapsed_since_search_start() >= 250, "ponder elapsed accumulates before ponderhit");

        clock.on_ponderhit();
        check(!clock.has_reached_hard_deadline(), "no deadline immediately after ponderhit");
        check(clock.elapsed_since_search_start() < 100, "ponderhit resets the elapsed origin");
        sleep(280); // > 200ms maximum, measured from ponderhit
        check(
            clock.has_reached_hard_deadline(),
            "deadline reached after the post-ponderhit budget elapses",
        );
    }

    // 5. Reset clears any armed deadline and restores the default time control.
    {
        let mut clock = ChessClock::new();
        clock.start_search(/*allocated=*/ 10, /*maximum=*/ 10);
        sleep(40);
        check(clock.has_reached_hard_deadline(), "deadline reached before Reset");
        clock.reset();
        check(!clock.has_reached_hard_deadline(), "Reset clears the hard deadline");
        check(clock.clock_type == ClockType::Standard, "Reset restores kStandard clock type");
        check(clock.remaining_time == 5 * 60 * 1000, "Reset restores the default remaining time");
        check(
            clock.moves_to_go == 0 && clock.depth == 10,
            "Reset restores default moves_to_go / depth",
        );
    }

    let failures = common::summary("CHESS CLOCK: deadline / elapsed / ponderhit / reset timing");
    assert_eq!(failures, 0);
}
