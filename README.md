# Pawnstar (Rust)

A mechanical Rust port of the [Pawnstar](https://github.com/jonny-reckless/pawnstar) UCI chess
engine (C++23): bitboard board representation, Lazy SMP alpha-beta search, NNUE-only evaluation
(1024-wide, 8 king buckets). A sibling
[Go port](https://github.com/jonny-reckless/pawnstar-go) exists too; all three engines are
bit-exact with one another.

**Bit-exact with the C++ original.** At fixed depth, single-threaded, this port produces the exact
same search: `bench` = **6,888,983 nodes** at depth 13, with per-position node counts, best moves,
PVs, scores and every DEBUGX diagnostic counter byte-identical to the C++ reference (verified by
transcript diff at depths 8–13; locked in by `tests/bench_golden_test.rs`).

## Build & run

```bash
cargo build --release          # engine → target/release/pawnstar (net + book embedded)
cargo test --release           # all ten suites: the eight C++ suites (incl. uci_test.sh) + two Rust-only gates
bash test/uci_test.sh target/release/pawnstar   # UCI integration suite (needs bash ≥ 4 for coproc)
cargo run --release --bin stamp_net -- <raw.bin> <stamped.bin>   # stamp a raw bullet net
cargo run --release --bin dump_magics                            # regenerate the magic constants
```

The engine loads `nnue/pawnstar-v12.bin` and `pawnstar.book` from the working directory or
relative to the executable, with copies embedded in the binary as fallback (`include_bytes!`, the
C++ `.incbin`). Individual suites run with e.g. `cargo test --release --test perft_test`
(`PAWNSTAR_PERFT_DEPTH` caps the EPD depth for quick iteration; `PAWNSTAR_BK_DEPTH` overrides the
Bratko-Kopec depth); `cargo doc --open` builds the API docs (the C++ `make doc`).

Environment variables (same as the C++ engine): `PAWNSTAR_THREADS` (default Lazy SMP thread
count; `1` = deterministic single-threaded), `PAWNSTAR_EVALFILE` (net path override),
`PAWNSTAR_BENCH_DEBUG=1` (`bench` dumps per-position best moves, node counts and diagnostic
counters for cross-engine comparison).

Beyond UCI, the engine answers the same diagnostic commands as the C++: `bench [depth]`, `dbg` /
`dbgclear` (DEBUGX counters; compiled in by default, disabled with
`--no-default-features` — the C++ `RELEASE=1`), `eval` (the search's full evaluation: draw
short-circuit + cache + fifty-move scaling), `nnue` (raw network output), `getboard`, `bookmoves`,
`freebook`, `help`. `examples/nnue_dump.rs` prints raw evals for FENs on stdin (the cross-engine
eval-parity tool).

Reproduce the parity check against the C++ engine:

```bash
for e in ../pawnstar/build/pawnstar target/release/pawnstar; do
  PAWNSTAR_THREADS=1 PAWNSTAR_BENCH_DEBUG=1 $e <<< $'bench\nquit' |
    sed -E 's/time +[0-9]+ /time X /; s/nps +[0-9]+ /nps X /; s/([0-9]+ nodes) [0-9]+ nps/\1/' |
    awk '/^### pos 01/,0'
done | ... diff the two halves — they are identical.
```

## Continuous integration

`.github/workflows/ci.yml` gates every push and pull request to `main` with three jobs:

- **build-test** and **format** — `cargo build/test --release` plus the bench determinism signature,
  and `cargo fmt --check`.
- **build + test (windows x86_64)** — the same build and suites on the organisation's self-hosted
  Windows runners (`[self-hosted, Windows, X64]`), added 2026-09-26. Asserting the bench signature on a
  second platform is the point: it is a cross-platform bit-parity guarantee, and until then it was only
  ever checked on Linux.

The Windows job needs no `dtolnay/rust-toolchain` step — those runners carry rustup and
`stable-x86_64-pc-windows-msvc` machine-wide (under `C:\Rust`, so the runner's service account can read
them) and `rust-toolchain.toml` already pins the channel. Its shell is Git Bash 5.x, which satisfies the
"bash >= 4 with coproc" probe in `tests/uci_shell_test.rs`, so `test/uci_test.sh` genuinely runs there
rather than skipping as it does where no such bash exists.

**Every job runs on the organisation's own machines** — `[self-hosted, Linux, X64]` for the first two,
`[self-hosted, Windows, X64]` for the third. No job in this repository uses a GitHub-hosted image. The
Linux legs keep their `dtolnay/rust-toolchain` step even so: unlike the Windows runners those machines
carry no rustup, and that action's "install rustup if needed" path bootstraps it from rustup-init.

## Structure

Each `src/*.rs` module ports the same-named C++ header 1:1 (structs, fields, functions —
snake_case names, otherwise mechanical):

| C++ | Rust | | C++ | Rust |
|---|---|---|---|---|
| `move.h` | `moves.rs` (keyword) | | `search_state.h` | `search_state.rs` |
| `main.cpp` | `main.rs` | | `game.h` | `game.rs` |
| `embedded_*.S` | `include_bytes!` in `main.rs` | | `evaluation.h` | `evaluation.rs` (re-export stub) |
| `test/test_report.h` | `tests/common/mod.rs` | | everything else | same basename |

Why the search is bit-reproducible across the port (engineering inherited from the C++):
`sort_moves` is a stable descending insertion sort (never `sort_unstable`); Zobrist keys come from
a fixed-seed xorshift* with a defined draw order (compile-time `const` here); there is no floating
point on the search/eval path; the NNUE accumulator uses wrapping i16; `Move` packs everything in
an `i64` whose arithmetic shift sign-extends the 23-bit score field.

C++ overloads become distinct names: `SetPosition()` → `set_position_start()`,
`PlayMove(string)` → `play_move_str()`, `Network::Evaluate(Position)` → `evaluate_position()`,
`EvaluateExact(Position)` → `evaluate_exact_position()`, and the 3-argument `Move::Promotion`
folds into `Move::promotion(from, to, promoted, Piece::None)` (bit-identical encoding). The two
offline analysis tools `filter_book` and `nnue_quant_study` are intentionally not ported (working
C++ versions live in `../pawnstar/tools/`); `Move::operator<` is intentionally unported (unused,
and inconsistent with the identity-masked equality under Rust's trait contracts).

## Deviations from the C++ (all behavior-preserving)

- **Threading is type-encoded.** C++ shares `Game&` mutably across threads by convention. Here:
  everything a search thread writes is an atomic (TT cells, eval cache, stop/ponder flags, ponder
  move, node count, clock instants as nanos-since-base `AtomicU64`); the game history is a
  `Mutex<Vec<Position>>` (never on the per-node hot path); every other field is written only via
  `Engine::game_mut()`, which joins the worker first. Lazy SMP helpers are scoped threads.
- `SearchState`'s C++ `mutable` lazy-accumulator stacks become `&mut self` methods
  (`current_accumulator`, `evaluate_position`).
- The book-move PRNG is a wall-clock-seeded xorshift* instead of `std::mt19937` (selection
  randomness affects no test or gate; the book is off in bench/BK). `dump_magics` by contrast
  ports `std::mt19937_64` exactly, so it reproduces the baked magic constants bit-for-bit.
- Numeric option/env/bench values parse with C++ `atoi` semantics (leading digits; 0 on none).
  `go` values differ from the C++ only in that its `std::stoi` *terminates* on a fully non-numeric
  token where this engine uses 0. On malformed FEN the Rust engine is defined-and-safe where the
  C++ has istream/indexing quirks (missing fields default sanely; stream-poisoning on a bad
  halfmove field is reproduced).
- The int8 eval dot accumulates in `i32` (provably in-range, identical results; the C++ scalar
  uses `int64`, its AVX2 kernel `int32`) so LLVM vectorizes the scalar kernel.

## Performance

Scalar NNUE kernels only (bit-identical to the C++ SIMD kernels by construction — the C++ also
runs scalar on arm64, where this port was verified). Measured single-threaded on an Apple M1 Pro
(medians over alternating runs):

| Metric | C++ | Rust | ratio |
|---|---|---|---|
| `bench` depth 13 (search nps) | ~1.15M | ~1.04M | 0.91× |
| Bratko-Kopec depth 12 (suite time) | ~5.2s | ~5.6s | 0.92× |
| perft suite (raw movegen Mnps) | 436 | 255 | 0.58× |

Real search speed is within ~10% of the C++ (the search is eval-bound and the eval kernels
vectorize). Head-to-head at 10+0.1 (300 games, UHO openings, single-threaded): **an exact 150–150
tie with the C++** (81 wins each, 138 draws; 0.0 ± 25.1 Elo) — a three-way round-robin with the Go
port put all three engines within the error bars of one another (least-squares, C++ = 0: Go +13,
Rust −7). The perft gap is confined to raw move generation (`gen_moves`: per-element bounds
checks and indirect attack-fn calls that clang folds harder); movegen is only ~1–2% of a search
node's cost, so it barely registers in play. Future work: x86-64 AVX2/VNNI output-dot kernels
(`std::arch` + the ported cpuid gate in `platform.rs`), untested here for lack of x86 hardware —
any such change must reproduce the bench golden signature exactly.
