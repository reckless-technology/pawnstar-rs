---
name: pawnstar-go-port
description: The Go port of pawnstar at ../pawnstar-go — also bit-exact; binary layout and how to drive it
metadata: 
  node_type: memory
  type: project
  originSessionId: cf0fdb79-a9de-45aa-8c13-0199c07a5b2c
---

`/Users/jonny/work/pawnstar-go` (remote git@github.com:jonny-reckless/pawnstar-go.git) is a Go
port of the C++ engine that predates [[pawnstar-rs-port]]. It is **bit-exact** with the C++ and
Rust engines: same bench signature as the other two, which is **5,617,906** on net v20 (release
2.0.0); the 6,888,983 verified on 2026-07-03 is a historical figure, as is v12's 6,601,454. ~1.09M nps on the M1
(between C++ 1.15M and Rust 1.04M). The C++'s portable determinism engineering (xorshift* Zobrist,
stable insertion sort, `PAWNSTAR_BENCH_DEBUG` transcript diffing) was originally built for this
port; the C++ `generated_data.h` comment references its `engine/tables.go initZobrist`.

**How to apply:**
- Binary: `./pawnstar-go` (a symlink to the version-named build, e.g. `pawnstar-go_0_1_178`);
  rebuild with `make` in that repo. UCI options mirror the C++ (Threads default 10 — pin to 1 for
  determinism; Hash default **64**, brought there from 512 on 2026-10-27 because an unequal table
  size breaks the three ports' bit-exactness at every `go`; EvalFile default `<embedded>`; OwnBook
  default true).
- Any search/eval change in one engine desyncs the other two; the three-way bench-transcript diff
  is the check. Tournament evidence (10+0.1, 300 games/pairing): all three within error bars.
