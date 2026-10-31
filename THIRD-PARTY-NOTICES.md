# Third-party notices

pawnstar-rs is free software under the **GNU General Public License, version 3** (see
[LICENSE](LICENSE)). That licence covers the engine's own code. This file records the third-party
material distributed with it, and the notices that material obliges.

The crate has **no third-party code dependencies at all**: `Cargo.toml` declares no dependencies and
`Cargo.lock` holds the single package `pawnstar`, so nothing is vendored, linked or compiled in from
anyone else. Everything below is about **data** shipped in this repository and in its release
binaries.

One thing below carries a notice this engine owes, and one does not. The shipped network was trained
entirely on a database published under the **Open Database License**, which obliges a notice naming
the source wherever a work trained on it is distributed: that is the first section. The shipped
**opening book** was built from the Lichess open database, which is released under a **CC0
public-domain dedication** — and **CC0 waives the attribution requirement**, so that section is not a
notice this engine owes; it is there because saying where the data came from is the right thing to do.

## The NNUE network's training data — Leela Chess Zero (ODbL 1.0 / DbCL 1.0)

The shipped evaluation network [`nnue/pawnstar-v20.bin`](nnue/pawnstar-v20.bin) — the same blob the
C++ [pawnstar](https://github.com/reckless-technology/pawnstar) and
[pawnstar-go](https://github.com/reckless-technology/pawnstar-go) engines ship, and the copy embedded
in this engine's binary — was trained **entirely** on Leela Chess Zero self-play training data.

| | |
|---|---|
| Source | **Leela Chess Zero self-play training data** |
| Source home | <https://storage.lczero.org/files/training_data/> |
| Database licence | **ODbL-1.0** — <http://opendatacommons.org/licenses/odbl/1.0/> |
| Contents licence | **DbCL-1.0** — <http://opendatacommons.org/licenses/dbcl/1.0/> |
| Runs used | `test91` (62.83% of the training pool) and `test60` (37.17%) |
| Pool size | 6,143,698,748 positions |

A trained network is an ODbL **"Produced Work"**, and distributing one obliges a notice naming the
source. This is that notice:

> Position data from the Leela Chess Zero self-play training data
> (https://storage.lczero.org/files/training_data/), run test91, used under the Open Database
> License 1.0 (http://opendatacommons.org/licenses/odbl/1.0/).

> Position data from the Leela Chess Zero self-play training data
> (https://storage.lczero.org/files/training_data/), run test60, used under the Open Database
> License 1.0 (http://opendatacommons.org/licenses/odbl/1.0/).

Those two lines are the attribution the extractor that read the archives writes into every shard's
provenance sidecar (`tools/lc0_extract.cpp` in the C++ repository, which is where the data pipeline
lives), one per run, reproduced here verbatim.

### What the obligation is, and what it is not

- **The obligation is the notice.** ODbL asks that a Produced Work carry an attribution naming the
  source database and its licence. That is what this section is.
- **ODbL does not require the network itself to be ODbL-licensed.** A Produced Work is not a
  derivative database, and ODbL's share-alike applies to derivative *databases*, not to works
  produced from one. `nnue/pawnstar-v20.bin` is distributed under this repository's own licence.
- **The network is not GPL merely because GPLv3 code trained it.** The trainer and the extraction
  tools being GPLv3 says nothing about the licence of their output; a program's licence does not
  reach the data it processes. The network is distributed under the GPLv3 as a deliberate choice of
  this project, not as a consequence of either licence.
- **DbCL-1.0** covers the *contents* of the database — the individual positions and their labels —
  and places them in the public domain as far as the publisher is able. Nothing further is owed for
  them beyond the notice above.

## The opening book — the Lichess open database (CC0 1.0)

[`pawnstar.book`](pawnstar.book), read by `src/opening_book.rs` and embedded in the engine binary, was
**built from the Lichess open database** — specifically the monthly export of standard rated games
named in the book file's own header, which also records the filters used, the counts and the SHA-256
of exactly the bytes read. It is the same file the C++
[pawnstar](https://github.com/reckless-technology/pawnstar) and
[pawnstar-go](https://github.com/reckless-technology/pawnstar-go) engines ship, byte for byte;
`tools/make_book.cpp` in the C++ repository built it, and it is not reproducible by regenerating it
(the header carries a generation timestamp), so it travels between the ports as bytes.

- **Source:** Lichess open database, standard rated games, <https://database.lichess.org/>
- **Dedication:** Creative Commons CC0 1.0 Universal (`CC0-1.0`),
  <https://creativecommons.org/publicdomain/zero/1.0/>

**This section is a courtesy, not an obligation, and it should not be read as one.** CC0 is a
dedication of the work to the public domain so far as the law allows, and it explicitly waives
attribution: whoever uses the data owes Lichess nothing, this project included. Saying where a book's
lines came from is simply how a provenance claim is made checkable, and Lichess made the database
freely available, so the credit is given because it is right. Nobody downstream of this engine
inherits a condition from this section — there is none to inherit, and the book carries no third
party's restriction.

That matters here for a reason worth recording. **The book this replaced did not have a licence
anybody could point to.** It was derived from TSCP's `book.txt` and was described throughout all three
ports as public domain; TSCP is Copyright 1997 Tom Kerrigan, and the only permission ever located for
it was an informal 2002 email allowing use "as long as you're not making any money off of it" — a
non-commercial permission, which is incompatible with the GPLv3 this engine is distributed under, and
not a public-domain dedication at all. The material was removed rather than re-worded, because no
wording could have made the claim true. CC0 was chosen for the replacement precisely because it is a
dedication the rights holder actually made, so "this book carries no restriction" is now a statement
that can be checked rather than believed.

## Withdrawn: the PlentyChess self-play data

Networks up to and including **v12** were trained on self-play data generated by **PlentyChess**, by
**Yoshie2000** — approximately 6 billion positions. **No network in the current release was trained
on any of it, and every network that was has been withdrawn:** `nnue/pawnstar-v12.bin` and every net
before it are removed from the source tree, and every release that shipped them has been deleted, so
no binary distributed from this repository contains an evaluator trained on that data.

The credit is kept anyway — those networks were measurably stronger for having been trained on that
data, and the record should show where they came from. See
[nnue/README.md](nnue/README.md), "Provenance of earlier networks".

## Not owed here

- **bullet** (the NNUE trainer, MIT). The C++ repository ships two files adapted from bullet's
  examples and carries bullet's MIT notice for them. **Nothing in this crate is adapted from
  bullet**: the trainer and the reference evaluator live in the C++ repository, and this port's
  `src/nnue.rs` only *matches* bullet's quantisation, feature indexing and activation by
  specification, as the inference side of a net format must. No bullet code, and so no bullet
  notice, is distributed from here.
- **The test suites' position sets** (the perft EPD, the Bratko-Kopec positions) are standard
  published test positions, not a licensed database, and are not shipped in any release binary.

## Reading this file in the engine

The engine prints a condensed form of these notices on the `license` command, so a release binary
downloaded on its own carries the attribution with it.
