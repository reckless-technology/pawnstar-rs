# Pawnstar (Rust)

A mechanical Rust port of the [Pawnstar](../pawnstar) UCI chess engine (C++23): bitboard board
representation, Lazy SMP alpha-beta search, NNUE-only evaluation (1024-wide, 8 king buckets).

**Bit-exact with the C++ original.** At fixed depth, single-threaded, this port produces the exact
same search: `bench` = **6,888,983 nodes** at depth 13, with per-position node counts, best moves,
PVs, scores and every DEBUGX diagnostic counter byte-identical to the C++ reference (verified by
transcript diff at depths 8–13; locked in by `tests/bench_golden_test.rs`).

## Build & run

```bash
cargo build --release          # engine → target/release/pawnstar (net + book embedded)
cargo test --release           # all eight ported suites (perft, SEE, NNUE ×2, BK, book, clock, golden)
bash test/uci_test.sh target/release/pawnstar   # UCI integration suite (needs bash ≥ 4 for coproc)
cargo run --release --bin stamp_net -- <raw.bin> <stamped.bin>   # stamp a raw bullet net
cargo run --release --bin dump_magics                            # regenerate the magic constants
```

The engine loads `nnue/pawnstar-v12.bin` and `pawnstar.book` from the working directory or
relative to the executable, with copies embedded in the binary as fallback (`include_bytes!`, the
C++ `.incbin`). `PAWNSTAR_EVALFILE` overrides the net path; `PAWNSTAR_THREADS` the default thread
count; `PAWNSTAR_BENCH_DEBUG=1` makes `bench` dump per-position best moves, node counts and
diagnostic counters for cross-engine comparison.

Reproduce the parity check against the C++ engine:

```bash
for e in ../pawnstar/build/pawnstar target/release/pawnstar; do
  PAWNSTAR_THREADS=1 PAWNSTAR_BENCH_DEBUG=1 $e <<< $'bench\nquit' |
    sed -E 's/time +[0-9]+ /time X /; s/nps +[0-9]+ /nps X /; s/([0-9]+ nodes) [0-9]+ nps/\1/' |
    awk '/^### pos 01/,0'
done | ... diff the two halves — they are identical.
```

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
- `go`'s numeric parsing is `parse().unwrap_or(0)` (C++ `std::stoi` throws on garbage).
- The int8 eval dot accumulates in `i32` (provably in-range, identical results; the C++ scalar
  uses `int64`, its AVX2 kernel `int32`) so LLVM vectorizes the scalar kernel.

## Performance

Scalar NNUE kernels only (bit-identical to the C++ SIMD kernels by construction — the C++ also
runs scalar on arm64, where this port was verified). Single-threaded bench on an Apple M1 Pro:
~1.02M nps vs the C++'s ~1.11M (within ~8%). Future work: x86-64 AVX2/VNNI output-dot kernels
(`std::arch` + the ported cpuid gate in `platform.rs`), untested here for lack of x86 hardware —
any such change must reproduce the bench golden signature exactly.
