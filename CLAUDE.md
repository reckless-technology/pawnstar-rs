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

**Setting up a machine.** `tools/setup-dev.ps1` (Windows) and `tools/setup-dev.sh` (Linux, macOS) install what
building, testing, debugging and coverage need - rustup with the stable toolchain, rustfmt, clippy and llvm-tools,
cargo-llvm-cov, Visual Studio's C++ build tools on Windows (behind a UAC prompt), a bash >= 4 for the UCI shell suite
(Git Bash on Windows, Homebrew's on macOS) - plus the git hooks and the recommended VS Code extensions, checking each
first, so they are safe to run again; `-Check` / `--check` only reports. In VS Code they are the `setup environment`
and `check environment` tasks.

```bash
cargo build --release                            # engine → target/release/pawnstar
cargo test --release                             # all suites, incl. the uci shell test and the
                                                 #   bench golden (the parity regression lock)
bash test/uci_test.sh target/release/pawnstar    # UCI suite standalone (bash >= 4 for coproc)
cargo llvm-cov --release --ignore-filename-regex 'src/bin/' --summary-only   # line coverage (CI gate: 90 %)
cargo fmt --check && cargo clippy --release      # style gates (fmt is enforced; clippy advisory)
cargo run --release --bin stamp_net -- <in> <out>    # stamp a net into RNNUE1 (see nnue/README.md)
cargo run --release --bin dump_magics
```

Profiles: `dev` is opt-level 3 **with overflow checks** (intentional wraps use `wrapping_*`; a
panic here means a porting bug); `release` matches C++ semantics. Tests inherit dev; run heavy
suites with `--release`. Env knobs (same as C++): `PAWNSTAR_THREADS` (thread-count default; `1`
= deterministic), `PAWNSTAR_EVALFILE` (net path override), `PAWNSTAR_BENCH_DEBUG=1` (bench dumps
per-position best move / node count / DEBUGX counters). Test-only: `PAWNSTAR_PERFT_DEPTH`,
`PAWNSTAR_BK_DEPTH`.

CI runs the same gates on **Linux and Windows** (`.github/workflows/ci.yml`): `cargo build/test
--release`, `cargo fmt --check`, and the bench determinism signature on both. The signature is derived
from `GOLDEN_TOTAL` in `tests/bench_golden_test.rs` — never write a second copy of the number into a
workflow; that drift once kept CI red for three commits in the sibling repo. Every job, in CI and in the
release, runs on a GitHub-hosted image named by its version (`ubuntu-24.04`, `windows-2025`, `macos-15`), free for a
public repository. Each starts empty: `dtolnay/rust-toolchain` brings the stable toolchain `rust-toolchain.toml` names,
and a job installs anything else it needs (coverage builds the pinned cargo-llvm-cov and caches its binary with
`actions/cache`). The organisation's Actions policy allows GitHub's own actions, verified creators' and few others -
`dtolnay/rust-toolchain` among them, `Swatinem/rust-cache` not - and any other action fails the run. Note
`test/uci_test.sh` really executes on Windows (Git Bash is 5.x, so the `coproc` probe passes) rather than skipping.
The Windows binary links the C runtime statically (`.cargo/config.toml`: `+crt-static` for the MSVC target), so it
imports Windows' own DLLs alone - KERNEL32, ntdll, bcryptprimitives and the synchronisation API set - and needs no
Visual C++ redistributable; `tools/windows-dependents.ps1` reads the PE import tables itself and fails CI's Windows
job and the release's Windows build on any other DLL. A RUSTFLAGS in the environment replaces the
config's flags, so no workflow sets one.

**Coverage** (README, "Coverage"): CI's `coverage` job runs every suite under `cargo llvm-cov --release` and fails
under **90 % of lines**. The figure is the engine library and the `pawnstar` binary - `src/` without `src/bin/`:
`dump_magics` and `stamp_net` are developer tools run by hand and are deliberately excluded
(`--ignore-filename-regex 'src/bin/'`). The binary suites (`uci_shell_test`, `uci_robustness_test`,
`uci_binary_test`) count because the binary they spawn is the instrumented one and inherits `LLVM_PROFILE_FILE`; a
spawned engine must therefore end by returning from `main` (end of input or `quit`) or exiting within the suite's
grace period - a killed process writes no profile. New code comes with tests that keep the figure over 90 %.

**Versions and releases** (README, "Versions and releases"): the version is `MAJOR.MINOR.BUILD` from git alone -
the nearest annotated `vMAJOR.MINOR` tag and the commits since it, derived in `build.rs`; nothing in the tree is
edited to release (`Cargo.toml`'s version is a `0.0.0` placeholder). Pushing a mark (`v1.1`) runs
`.github/workflows/release.yml`: built, tested and bench-checked on Linux, Windows and Apple silicon, the Windows
binary eSigner-signed (publishing runs only - each signing spends the allowance), the macOS one Developer ID-signed and
notarised in a zip, published as `v1.1.0`. `gh workflow run release.yml --ref <branch> -f dry_run=true` proves a
change to it without publishing or spending a Windows signing.

## The parity contract (the one hard rule)

The whole value of this port is bit-exactness with `../pawnstar`
(https://github.com/jonny-reckless/pawnstar). `tests/bench_golden_test.rs` locks the signature:
bench depth 13, single-threaded = **6,601,454 nodes** with fixed per-position node counts and best
moves (net v12; `GOLDEN_TOTAL` is the number's one home). The Go port (../pawnstar-go) shares the same signature; a
900-game round-robin at 10+0.1 put all three engines within error bars (C++ vs Rust: an exact
150–150 tie).

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

The two stop flags, as in the C++ and Go engines: `is_stop_requested` ("the GUI said stop") is
written only by the UCI thread and releases the ponder-wait; `is_cancel_pending` (abort; also set
by the main thread at the end of every search to wind the helpers up) is cleared only by the code
that starts a search, before it starts it - `start_thinking` before the spawn, `set_position` for
the direct callers of `search_root_node` - and never by a search thread. When `search_root_node`
cleared it as it began, a `stop` or `quit` that reached `stop_thinking` before the worker got that
far was lost and `go infinite` + `stop` hung for ever.

## Tests

`cargo test` covers everything the C++ `make check` runs: perft (full 131-record EPD to depth 6),
SEE (24 cases + fast==reference cross-check), NNUE (bullet reference ±2cp, int8 bound), NNUE
incremental (bit-identity vs refresh), Bratko-Kopec (hard 24/24 gate at depth 8, single-threaded),
opening book, chess clock, the uci_test.sh shell suite, plus Rust-only gates: table goldens
(`generated_data_test.rs`, and a unit test running the `const` table generators at run time), the bench golden,
every UCI command in process (`uci_commands_test.rs`), the fifty-move clock (`fifty_move_clock_test.rs`: castling
advances it, as FIDE 9.3 and FEN have it, and a mate on the hundredth ply is mate, not the draw, as FIDE 5.1.1 has it -
`search` tests for the mate before it draws, at the point the C++ and Go do), the binary's start-up and fallbacks (`uci_binary_test.rs`),
the shared NNUE container (`nnue_container_test.rs`: CRC-64/XZ against its published check value **first**, the
`RNNUE1` header's layout offset by offset, and every refusal the loader's two checks make - each leaving the net
already loaded playing, and a refused net never replaced by the embedded one)
and the library helpers the search does not call (`library_api_test.rs`). NNUE-dependent suites skip (pass) when
`nnue/pawnstar-v12.bin` is absent, preserving the C++ exit-0 contract. Shipping a new net follows
the C++ checklist (../pawnstar/nnue/README.md §7) plus re-blessing the golden test here.
