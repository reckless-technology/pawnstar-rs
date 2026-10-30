//! NNUE inference cross-check test. (Port of `test/nnue_test.cpp`.)
//!
//! Verifies that the in-engine NNUE forward pass reproduces the trainer's evaluation. The
//! trainer-side reference evals in `tests/nnue_reference.txt` are the **shipped net's own** — they
//! are regenerated for every net that ships (nnue/README.md, "Shipping a new net to this repo") —
//! and agreement to within a couple of centipawns (quantisation rounding) confirms our feature
//! indexing, king bucketing, horizontal mirroring, output-head selection, SCReLU activation,
//! dequantisation and file format all match the trainer's. v20's 250 agree to **0 cp**.
//!
//! The C++ takes `<net.bin> <reference.txt>` on the command line and is skipped without them;
//! here the paths default to the in-repo assets (PAWNSTAR_EVALFILE overrides the net) and the
//! test passes as "skipped" when the net is absent, preserving the exit-0 contract.

mod common;

use pawnstar::nnue::Network;
use pawnstar::position::Position;

/// Allowed engine-vs-reference deviation in centipawns (integer-quantisation rounding only).
const TOLERANCE: i32 = 2;

fn net_path() -> String {
    std::env::var("PAWNSTAR_EVALFILE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/nnue/pawnstar-v20.bin").to_string()
    })
}

#[test]
fn nnue_suite() {
    let net_file = net_path();
    let reference_file = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/nnue_reference.txt");
    // The reference evals are this net's, so a build of another NNUE shape — which refuses the net
    // anyway — has nothing to cross-check against.
    if !common::is_shipped_net_this_builds_shape() {
        common::summary("NNUE: skipped (no net of this build's NNUE shape, or no reference)");
        return;
    }
    let mut net = Box::new(Network::new());
    if !net.load(&net_file).is_loaded() {
        common::check(false, &format!("NNUE: failed to load net '{}'", net_file));
        assert_eq!(common::summary("NNUE"), 0);
        return;
    }
    let reference = match std::fs::read_to_string(reference_file) {
        Ok(r) => r,
        Err(_) => {
            common::check(
                false,
                &format!("NNUE: cannot open reference '{}'", reference_file),
            );
            assert_eq!(common::summary("NNUE"), 0);
            return;
        }
    };

    // Two checks per position:
    //  (1) the exact int16 reference (evaluate_exact) matches bullet within ±TOLERANCE — verifies
    //      feature indexing, SCReLU and dequantisation; and
    //  (2) the shipped int8 evaluate stays within INT8_BOUND cp of that reference — guards the
    //      int8 kernel (which deliberately differs from bullet by the SCReLU-into-8-bits requant).
    //
    // INT8_BOUND is 40, which is EXACTLY what this set measures: **40 cp over v20's 250 reference
    // positions, 2026-10-30**, and the same 40 in all three ports (C++, Rust and Go each report it).
    // It was 26-28 cp with v12, and the rise is not drift: ONE int8 scale is shared across all eight
    // output heads, so the head carrying the largest weight sets the scale every other head is then
    // requantised against.
    //
    // A bound sitting exactly on the measured maximum is deliberate here, because this is integer
    // arithmetic: the deviation is exactly 40 on every platform, deterministically, so there is no
    // marginal pass to tip over — this is not a tolerance with the headroom spent. The three ports
    // being bit-exact on node counts (bench 5,617,906 at depth 13) is the corroborating evidence that
    // they compute the same integers. So 40 is a tight and TRUE gate, and widening it — this port
    // carried 64 for a day, reading a measured-40-against-40 as a latent red CI — weakens a
    // deterministic passing check for no gain.
    //
    // THE REAL FIX for a future net whose heads spread wider is a PER-HEAD int8 scale, not a looser
    // bound: one scale across eight heads is what couples them, and only a per-head scale decouples
    // them. Re-measure this number whenever a net ships, and record beside it what it was measured on
    // and when.
    const INT8_BOUND: i32 = 40;
    let mut checked = 0;
    let mut max_abs_diff = 0;
    let mut max_int8_dev = 0;
    for line in reference.lines() {
        let Some(bar) = line.rfind('|') else { continue };
        let fen = &line[..bar];
        let expected: i32 = line[bar + 1..].trim().parse().unwrap_or(0);

        let position = Position::from_string(fen);
        let got = net.evaluate_exact_position(&position);
        let diff = (got - expected).abs();
        max_abs_diff = max_abs_diff.max(diff);
        checked += 1;
        if diff > TOLERANCE {
            common::check(
                false,
                &format!(
                    "NNUE ref16 {}  ref16={} bullet={} diff={}",
                    fen, got, expected, diff
                ),
            );
        }
        let int8_dev = (net.evaluate_position(&position) - got).abs();
        max_int8_dev = max_int8_dev.max(int8_dev);
        if int8_dev > INT8_BOUND {
            common::check(
                false,
                &format!(
                    "NNUE int8 {}  int8 deviates {} cp from ref16 (> {})",
                    fen, int8_dev, INT8_BOUND
                ),
            );
        }
    }

    let failures = common::summary(&format!(
        "NNUE: {} positions, max |ref16-bullet|={}cp (tol {}), max |int8-ref16|={}cp (bound {})",
        checked, max_abs_diff, TOLERANCE, max_int8_dev, INT8_BOUND
    ));
    assert_eq!(failures, 0);
    assert!(checked > 0);
}
