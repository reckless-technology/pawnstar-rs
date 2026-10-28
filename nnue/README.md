# NNUE net (pawnstar-rs)

`pawnstar-v12.bin` is the shipped evaluation network: a bullet-trained perspective net, 1024-wide
feature transformer with 8 king buckets (file-pair × board-half), trained on ~6B positions of a
public bulletformat dataset, in the shared Reckless NNUE container `RNNUE1` — a 64-byte
self-describing header (see `src/nnue.rs::NetHeader`) and a 12,589,060-byte payload, with a
CRC-64/XZ over each. It is **byte-identical to the net shipped by the C++ and Go engines** — all
three are cross-loadable and produce bit-identical evaluations from it.

The container is the one every reckless-technology chess engine reads; the `reckless-nnue` skill is
its specification. **By default** this engine is compiled for v12's shape — king mirroring off
(`flags = 0`) and one output head (`output_buckets = 1`) — and a net that asks for either addition
is refused. The shared architecture is built from the same tree behind two Cargo features; see
below.

## The two network shapes, and the features that pick one

Five engines are being unified onto one NNUE evaluator (skill `reckless-nnue`), and the shared
architecture adds two things to the shape the shipped v12 net was trained for. Both were measured in
zenith on this hardware, SPRT-gated, and both only paid past ~1.4B training positions — at 190M they
were neutral or negative, which is why v12 has neither:

| | v12's shape | the shared shape | measured |
|---|---|---|---|
| horizontal king mirroring | off | **on** — a perspective whose own king is on files e–h is mirrored (`square ^ 7`) before indexing | **+14.3 Elo** fixed depth |
| material output heads | 1 | **8**, selected by `(piece_count - 2) / 4` | **+14.7 Elo** fixed depth |
| king bucket map | `half * 4 + file / 2` (file-*pairs*) | `half * 4 + min(file, 7 - file)` (per-*file*) | |
| `king_bucket_map_crc` | `0xCC6DDB5DAF0AB18D` | `0x0A20060D4E9586E0` | |
| `format_version` | 1 | **2** | |
| payload bytes | 12,589,060 | 12,617,760 | |

Mirroring is worth more than its Elo suggests: halving the king-placement space gives **twice the
effective data per bucket at the same file size**, so it is a data multiplier as much as an Elo add.

**Either shape builds from this one tree**, so the three bit-exact ports can be brought across one at
a time. The knobs are the Cargo features `nnue-king-mirroring` and `nnue-output-buckets-8`, together
`nnue-shared-shape`, and they default off. Give the new shape a **target directory of its own** — not
only to keep the two builds apart, but because MSVC's linker has been seen to fail with `LNK1201` on
a PDB the other feature set wrote:

```bash
CARGO_TARGET_DIR=target-newshape cargo test --release --features nnue-shared-shape
```

Expect that build to refuse `nnue/pawnstar-v12.bin` — `net '...' refused: format_version is 1 in the
file but 2 in this engine`. That is the container working, not a fault: nothing has been trained at
the new shape yet. The suites that need a *trained* net skip there and say so, exactly as they skip
when the net file is absent; `nnue_incremental_test` instead builds **its own synthetic net** at
whatever shape the build was configured for, which is how the new shape is tested before a net
exists.

**A shape change is an evaluation change.** It moves the bench signature and breaks parity with
`pawnstar` and `pawnstar-go` until all three are ported together, so the defaults are not to be
flipped for one engine alone. The mirrored map also changes what a bucket *means* rather than only
how many there are, which by the container's own rule makes it a format change — hence
`format_version` 2, **derived from the mirroring switch and never hardcoded**, in every port.

### Why mirroring needs its own refresh rule, and why getting it wrong is silent

A perspective can be updated incrementally only while its weight bank, its square flip and its colour
flip all stay put; if any changes, every feature row it reads moves. Before mirroring there was one
way for that to happen, a king crossing a bucket boundary, and the code compared bucket indices.
Mirroring adds a second, and the two are not the same: on the mirrored map a king stepping
**d1 → e1 keeps bucket 3** (file 3 and file 4 both mirror to file 3) while its square flip gains the
file flip. A comparison of buckets alone would diff an accumulator whose whole basis had just changed
— no crash, no assertion, just a wrong evaluation. So the three live together in one
`nnue::Perspective`, compared whole, and `nnue_incremental_test` crosses each boundary on purpose and
reports how many plies forced a refresh, so a case that quietly stopped crossing anything cannot
pass. Narrowing the comparison back to the bank alone makes that suite fail on the first ply of its
first line, naming `white(bank=2304 flip=7)` — same bank, different flip — while the half-boundary
lines still pass, which is how the test was checked.

### The new shape's cross-port node-count signature

At the new shape the engine refuses the shipped net, so `bench` cannot run, and over random weights a
depth-13 search barely prunes and does not finish in any useful time. The parity signature is instead
six fixed-depth searches over the **synthetic net**, whose bytes are identical on every platform and in
every port (SHA-256 `4abfdb7bf3f6753fb53d8fed3b524750f7c06031cd5464445f18075f8ad4871b`, 12,617,824
bytes — the same file the C++ engine's `test_nnue_incremental --write-synthetic-net` writes).

[`tools/new-shape-signature.sh`](../tools/new-shape-signature.sh) is that gate, and it is **committed
byte-identical in `pawnstar`, `pawnstar-rs` and `pawnstar-go`**: the six expected results live in it and
nowhere else, so `diff` between the three repositories is the whole audit and there is no second copy of
the numbers to drift — this file used to hold one, and a second copy of this repository's *bench* golden
once sat in CI and silently went stale for three commits. The `new-shape` CI job runs it on every pull
request.

```bash
CARGO_TARGET_DIR=target-newshape cargo build --release --bins --features nnue-shared-shape
target-newshape/release/write_synthetic_net synthetic-net.bin
tools/new-shape-signature.sh --hash 64 --net synthetic-net.bin target-newshape/release/pawnstar
```

It asserts the net's size, SHA-256 and RNNUE1 header — including `format_version` 2, `payload_bytes`
12,617,760 and `king_bucket_map_crc` `0x0A20060D4E9586E0` — **before** searching it, so a drifted
generator fails as a drifted generator instead of reading as a search regression. `--hash` is mandatory
and deliberately not defaulted, and its value is printed beside every node count: table size changes the
collision pattern and so the node count at a fixed depth, which is how `pawnstar-go`'s 512 MB default
stayed invisible for five months against this port's 64 MB. The numbers were reproduced by all three
ports at `Hash 64` — C++ (clang 22 on Windows, clang 18 on `reckless-ci-lin-2`), Rust and Go.

The last two of the six positions put the kings on d1/d8 and e1/e8, either side of the mirror boundary,
so the search spends its time in positions whose perspectives keep crossing it — the case that is silent
when the refresh rule is wrong.

Two traps the script handles, and that anybody reading these numbers out of the engine by hand must
handle too: `quit` straight after `go` cancels the search but still prints a `bestmove`, so wait for the
`bestmove` before sending `quit`; and **two line types begin `info depth N`** — only the completed
iteration carries `nodes`, so a `currmove` report reads as a missing node count on a search that
actually finished.

**This file is a shared asset, not a Rust artifact.** Everything about producing a net — the
bullet trainer setup, data pipeline, quantisation, verification, SPRT testing, and the full
net-shipping checklist — lives in the C++ repository (`nnue/README.md` §6–7 and
`nnue/int8_quant_study.md` in [pawnstar](https://github.com/jonny-reckless/pawnstar), the reference implementation). Nets are
trained once and shipped to both engines.

## Shipping a new net to this repo

After following the C++ checklist (train → `stamp_net` → regenerate references → SPRT):

1. Replace `nnue/pawnstar-v12.bin` (the loader validates the stamped header against the engine
   constants in `src/nnue.rs`, and the header's king-bucket map checksum against the checksum of
   this engine's own `KING_BUCKET_MAP`; an arch change means updating those constants in lock-step
   with the C++ `nnue.h`, the Go engine, and the container's `format_version` if a bucket's
   *meaning* changes).
2. Copy the regenerated `test/nnue_reference.txt` from the C++ repo to `tests/nnue_reference.txt`
   (the bullet cross-check evals are net-specific).
3. Update the Bratko-Kopec accepted-move sets in `tests/bratko_kopec_nnue_test.rs` to match the
   C++ `test/bratko_kopec_nnue_test.cpp` regeneration (must solve 24/24 at depths 8–11).
4. Re-bless `tests/bench_golden_test.rs`: run the parity diff against the C++ engine first
   (README.md, "Reproduce the parity check"), then record the new per-position node counts, best
   moves and total.
5. `cargo build --release` re-embeds the net (`include_bytes!` — no stale-embed pitfall, unlike
   the C++ `.incbin`, but the binary only picks the new net up on rebuild).
