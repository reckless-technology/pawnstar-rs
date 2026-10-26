# NNUE net (pawnstar-rs)

`pawnstar-v12.bin` is the shipped evaluation network: a bullet-trained perspective net, 1024-wide
feature transformer with 8 king buckets (file-pair × board-half), trained on ~6B positions of a
public bulletformat dataset, prepended with the 32-byte `PSN1` header (see `src/nnue.rs::NetHeader`).
It is **byte-identical to the net shipped by the C++ engine** — the two engines are cross-loadable
and produce bit-identical evaluations from it.

**This file is a shared asset, not a Rust artifact.** Everything about producing a net — the
bullet trainer setup, data pipeline, quantisation, verification, SPRT testing, and the full
net-shipping checklist — lives in the C++ repository (`nnue/README.md` §6–7 and
`nnue/int8_quant_study.md` in [pawnstar](https://github.com/jonny-reckless/pawnstar), the reference implementation). Nets are
trained once and shipped to both engines.

## Shipping a new net to this repo

After following the C++ checklist (train → `stamp_net` → regenerate references → SPRT):

1. Replace `nnue/pawnstar-v12.bin` (the loader validates the stamped header against the engine
   constants in `src/nnue.rs`; an arch change means updating those constants in lock-step with
   the C++ `nnue.h`).
2. Copy the regenerated `test/nnue_reference.txt` from the C++ repo to `tests/nnue_reference.txt`
   (the bullet cross-check evals are net-specific).
3. Update the Bratko-Kopec accepted-move sets in `tests/bratko_kopec_nnue_test.rs` to match the
   C++ `test/bratko_kopec_nnue_test.cpp` regeneration (must solve 24/24 at depths 8–11).
4. Re-bless `tests/bench_golden_test.rs`: run the parity diff against the C++ engine first
   (README.md, "Reproduce the parity check"), then record the new per-position node counts, best
   moves and total.
5. `cargo build --release` re-embeds the net (`include_bytes!` — no stale-embed pitfall, unlike
   the C++ `.incbin`, but the binary only picks the new net up on rebuild).
