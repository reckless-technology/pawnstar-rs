---
name: pawnstar-rs-port
description: A bit-exact Rust port of pawnstar lives in ../pawnstar-rs; parity workflow and maintenance rules
metadata: 
  node_type: memory
  type: project
  originSessionId: cf0fdb79-a9de-45aa-8c13-0199c07a5b2c
---

`/Users/jonny/work/pawnstar-rs` (created 2026-07-03, pushed to
git@github.com:jonny-reckless/pawnstar-rs.git main) is a mechanical Rust port of the C++ engine,
**bit-exact** with it: single-threaded fixed-depth search produces identical node counts, best
moves, PVs and DEBUGX counters (bench depth 13 = 6,888,983 nodes, C++ commit 609 / net v12).
Modules map 1:1 to the C++ headers (snake_case names; `move.h` → `moves.rs`); all eight test
suites are ported as cargo integration tests plus a `tests/bench_golden_test.rs` that locks the
per-position bench signature.

**Why:** cross-verification and a second implementation of the same engine.

**How to apply:**
- Any C++ search/eval/ordering change breaks parity until re-ported; the Rust golden test then
  fails by design. Re-bless goldens only after re-porting and re-diffing transcripts.
- Parity check recipe (both repos): `PAWNSTAR_THREADS=1 PAWNSTAR_BENCH_DEBUG=1 <engine> <<< $'bench\nquit'`,
  strip `time`/`nps` fields, diff — must be byte-identical. This is the same workflow the C++
  used against the Go port ([[pawnstar-go-port]] if ever written).
- The Rust port runs scalar NNUE kernels only (verified on arm64 where the C++ is also scalar);
  x86 AVX2/VNNI kernels are unwritten — if added they must reproduce the bench golden exactly.
- Rust is ~9% slower than C++ on M1 in real search (bench ~1.04M vs ~1.15M nps, 0.91×) and 0.58×
  on raw perft (255 vs 436 Mnps — bounds checks / indirect attack fns in gen_moves; irrelevant in
  play since search is eval-bound). Biggest P8 win: i32 (not i64) accumulation in the int8 eval
  dot so LLVM vectorizes it (provably identical, bounded ≤66M).
- 2026-07-03 round-robin vs [[pawnstar-go-port]] at 10+0.1 (300 games/pairing, UHO openings,
  Threads=1/Hash=64/books off): C++ vs Go −5.8±23.9, C++ vs Rust 0.0±25.1 (exact 150/300 tie),
  Go vs Rust +26.7±25.4. Least-squares (C++=0): Go +13, Rust −7 — all three within noise of each
  other, as bit-identical searches should be; only Go>Rust grazed 95% significance.
