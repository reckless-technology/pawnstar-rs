# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repository is

A **mechanical Rust port** of the C++ Pawnstar UCI chess engine at `../pawnstar`, **bit-exact**
with it: at fixed depth, single-threaded, both engines produce identical node counts, best moves,
PVs and DEBUGX diagnostic counters. The architecture, algorithms and their documentation live in
the C++ repo (`../pawnstar/CLAUDE.md` is the authoritative deep-dive); each `src/*.rs` module here
ports the same-named C++ header 1:1 (snake_case functions, SCREAMING_CASE constants, fields drop
the C++ trailing underscore; `move.h` → `moves.rs` because `move` is a keyword). README.md lists
every deliberate deviation.

## Build & test

```bash
cargo build --release                            # engine → target/release/pawnstar
cargo test --release                             # all suites, incl. the uci shell test and the
                                                 #   bench golden (the parity regression lock)
bash test/uci_test.sh target/release/pawnstar    # UCI suite standalone (bash >= 4 for coproc)
cargo fmt --check && cargo clippy --release      # style gates (fmt is enforced; clippy advisory)
cargo run --release --bin stamp_net -- <raw> <out>   # tools (see ../pawnstar/nnue/README.md)
cargo run --release --bin dump_magics
```

Profiles: `dev` is opt-level 3 **with overflow checks** (intentional wraps use `wrapping_*`; a
panic here means a porting bug); `release` matches C++ semantics. Tests inherit dev; run heavy
suites with `--release`. Env knobs (same as C++): `PAWNSTAR_THREADS` (thread-count default; `1`
= deterministic), `PAWNSTAR_EVALFILE` (net path override), `PAWNSTAR_BENCH_DEBUG=1` (bench dumps
per-position best move / node count / DEBUGX counters). Test-only: `PAWNSTAR_PERFT_DEPTH`,
`PAWNSTAR_BK_DEPTH`.

## The parity contract (the one hard rule)

The whole value of this port is bit-exactness with `../pawnstar`. `tests/bench_golden_test.rs`
locks the signature: bench depth 13, single-threaded = **6,888,983 nodes** with fixed per-position
node counts and best moves (C++ commit 609, net v12).

- **Never** replace `sort_moves` with `sort`/`sort_unstable` — the stable descending insertion
  sort is load-bearing for cross-engine determinism (equal-score moves keep generation order).
- Keep integer semantics exact: wrapping i16 NNUE accumulator, arithmetic-shift sign extension of
  the 23-bit `Move` score field, truncating division, the Zobrist xorshift* draw order in
  `generated_data.rs` (compile-time `const`, golden-tested).
- Any intentional search/eval change must be ported to (or from) the C++ first, then re-verify:
  ```bash
  for e in ../pawnstar/build/pawnstar target/release/pawnstar; do
    PAWNSTAR_THREADS=1 PAWNSTAR_BENCH_DEBUG=1 $e <<< $'bench\nquit'
  done   # strip time/nps fields, diff — must be byte-identical; then re-bless the golden test.
  ```
- Performance changes are welcome but parity-locked: re-run the golden test after each one.
- SIMD status: scalar NNUE kernels only (bit-identical to the C++ SIMD kernels by design; the C++
  is also scalar on arm64, where parity was verified). If adding x86 AVX2/VNNI kernels
  (`std::arch` + the cpuid gate in `platform.rs`), they must reproduce the golden signature.

## Threading model (the one structural deviation)

C++ shares `Game&` mutably across threads by convention; here the convention is type-encoded:
everything a search thread writes is an atomic (TT cells, eval cache, stop/ponder flags,
`ponder_move`, `last_search_node_count`, clock instants as nanos-since-base); the game history is
`Mutex<Vec<Position>>` (never on the per-node hot path — each `SearchState` owns its own stack);
all other `Game` fields are written only via `Engine::game_mut()`, which joins the worker first.
`ponderhit` must never join (it retargets the running search through atomics). Lazy SMP helpers
are scoped threads in `Game::search_root_node`.

## Tests

`cargo test` covers everything the C++ `make check` runs: perft (full 131-record EPD to depth 6),
SEE (24 cases + fast==reference cross-check), NNUE (bullet reference ±2cp, int8 bound), NNUE
incremental (bit-identity vs refresh), Bratko-Kopec (hard 24/24 gate at depth 8, single-threaded),
opening book, chess clock, the uci_test.sh shell suite, plus two Rust-only gates: table goldens
(`generated_data_test.rs`) and the bench golden. NNUE-dependent suites skip (pass) when
`nnue/pawnstar-v12.bin` is absent, preserving the C++ exit-0 contract. Shipping a new net follows
the C++ checklist (../pawnstar/nnue/README.md §7) plus re-blessing the golden test here.
