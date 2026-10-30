---
name: nnue-shape-knobs
description: How the two NNUE shapes are built in pawnstar-rs, and the Rust-specific decisions behind it
metadata:
  node_type: memory
  type: project
---

2026-10-27. The shared NNUE architecture (`reckless-nnue` skill) — horizontal king mirroring and 8
material output heads — landed here behind Cargo features, alongside v12's shape. What follows is
only what is specific to this port; the specification is the skill and the reference implementation
is `../pawnstar` (its PR #38, and its own `.claude/memory/project_shared_nnue_shape.md`).

**2026-10-30: the shared shape is now the DEFAULT, and v12's is the retired one.** Release 2.0.0
ships `nnue/pawnstar-v20.bin` at the shared shape (trained entirely on Leela Chess Zero self-play
data; +9.99 Elo at 8+0.08 over 8,000 games and +1.13 at 12+0.12 over 12,000 against v12), and
`nnue/pawnstar-v12.bin` was deleted from the tree. Read every "defaults off" below as historical.
Three things changed shape beyond the feature list:

- **`default = ["debugx", "nnue-king-mirroring", "nnue-output-buckets-8"]`**, so
  `--no-default-features` now drops the *shape* as well as DEBUGX. The retired shape is
  `--no-default-features --features debugx`; the DEBUGX-free build of the *shipped* shape is
  `--no-default-features --features nnue-shared-shape`. That overload is the one trap this flip
  introduced, and it is written down in `Cargo.toml`, `README.md` and `nnue/README.md` because the
  short form reads like it only means "no DEBUGX".
- **The bench golden is 5,617,906** at depth 13 single-threaded, replacing 6,601,454. It must be
  identical in `../pawnstar` and `../pawnstar-go`; the three moved together.
- **A shape probe must not name a shape.** `nnue_container_test` gated its shipped-net checks on a
  const `IS_V12_SHAPE`, which inverted the moment the shipped net changed shape: at the retired
  shape it ran v12's checks against v20's net and failed four of them. It now decodes the header's
  architecture fields and map digest from the image and compares them with
  `expected_net_header()` — and deliberately **not** the payload checksums, or a net of this shape
  that is corrupt would skip quietly instead of failing loudly, which is the one thing that suite
  exists to catch.

## The knobs are Cargo features, not an env var

`nnue-king-mirroring` and `nnue-output-buckets-8`, aggregated as `nnue-shared-shape`, standing in
for the C++'s `PAWNSTAR_NNUE_KING_MIRRORING` and `PAWNSTAR_NNUE_OUTPUT_BUCKETS`. `cfg!` makes them
`const`, so the shape fields, the weight array sizes and `NET_FORMAT_VERSION` are all compile-time
and the payload is sized by the shape rather than by a runtime value. A `build.rs` env var would
have allowed any head count, as the C++ knob does, but it would not have been visible to
`cargo test --features` and the two values anyone will build are 1 and 8.

**`format_version` is derived (`if IS_KING_MIRRORING_ENABLED { 2 } else { 1 }`), never written down.**
Version 2 means mirroring on *and* eight heads, together; a build that hardcoded 2 with mirroring but
one head would stamp a net another engine's version-2 build reads with the wrong number of heads.

**`NUM_KING_BUCKETS` is now derived from `KING_BUCKET_MAP`** (`max_king_bucket_index() + 1`), as the
C++ does, so the map and the count cannot disagree. And the mirrored 64-entry map is *generated* from
the 32 half-board numbers with bullet's own expansion arithmetic — the trainer lives in the C++ repo
and reads the same 32.

## Give the new shape its own target directory

`CARGO_TARGET_DIR=target-newshape`. Not only tidiness: switching feature sets in one target directory
produced repeated `LNK1201: error writing to program database …` from MSVC's linker across most test
binaries, cleared only by deleting `target/release/deps/*.pdb`. Two target directories avoid it and
avoid the full rebuild each switch costs.

## What the gate is at the new shape, and what it is not

`bench` cannot run: the engine rightly refuses v12's net there, and over the synthetic net's random
weights a depth-13 search barely prunes and does not finish in useful time. The gate is instead
**six fixed-depth-8 searches over the synthetic net** — `nnue/README.md`, "The new shape's
node-count signature" — and the synthetic net's bytes are identical to the C++'s
(SHA-256 `4abfdb7b…8ad4871b`, 12,617,824 bytes), which is itself half the proof. `6,601,454` remains
the gate for the **default** shape only.

`6,601,454` is history; `5,617,906` is the **default**-shape gate, and the synthetic-net signature
remains the gate for a build with no trained net of its shape — which, since v12's net was deleted, is
what the retired shape now always is.

The synthetic-net builder lives in the library (`nnue::synthetic_net_image`) rather than in the test,
because a Rust integration test cannot share code with a binary: `nnue_incremental_test` and
`src/bin/write_synthetic_net.rs` both need the same bytes. Its payload is written out by hand, which
is also a second opinion on the layout `load_from_memory` reads.

## Two Rust-specific details worth remembering

**`current_accumulator` takes `&mut self`**, so the evaluator's new `Position` argument could not
simply be borrowed alongside it — and copying the `Position` per evaluation would have put a memcpy
on the eval hot path. `SearchState::current_accumulator_and_position` hands both back from one
`&mut self` borrow instead, which also keeps the side to move and the output head read from one
place.

**The net-dependent suites skip when the shipped net is not this build's shape**, through
`tests/common::is_net_this_builds_shape`, which is the existing "skip when the net is absent"
contract (the C++'s exit-0 contract) extended to the shape knobs. The three suites that spawn the
engine binary skip wholesale, because the binary plays on the *embedded* net — which since 2.0.0 is
v20, so those three now run at the **default** shape and skip at the retired one, the exact reverse
of how they were written.
