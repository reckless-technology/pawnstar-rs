# The opening book's provenance, and why it arrives as bytes

**2026-10-30.** `pawnstar.book` used to be a 385-line book derived from TSCP's `book.txt`, asserted
to be "public domain" in this repository's `README.md`, `THIRD-PARTY-NOTICES.md`, the string the
released engine prints on `license`, and a `uci_binary_test` check that *enforced* the claim.

**The claim was false.** TSCP is Copyright 1997 Tom Kerrigan. The only permission anyone has located
is an informal 2002 email allowing use "as long as you're not making any money off of it" - a
non-commercial permission, which GPLv3 cannot take, and not a dedication of anything. The
`terredeciels/TSCP` repository the documents linked has no LICENSE file at all. So the material went
rather than being re-worded: **no wording could have made that claim true.** (The printed notice even
linked `terredeciles/TSCP`, a transposition of the name - a dead link nobody had followed.)

The replacement is built from the **Lichess open database**, standard rated games, export `2017-01`,
under the **Creative Commons CC0 1.0** public-domain dedication - a dedication the rights holder
actually made, so "this book carries no third party's restriction" is checkable rather than believed.

## The rules that are easy to break here

- **The three ports' books are byte-identical by design, and this port copies the bytes.** The
  generator is `tools/make_book.cpp` in `../pawnstar`, and `tools/filter_book` there screens what it
  produces against that engine's own evaluation; there is **neither here and neither should be
  added**. Take a new book with
  `git -C ../pawnstar show origin/main:pawnstar.book > pawnstar.book`.
- **Never "regenerate" it to check it.** The file's `#` header carries a generation timestamp, so a
  second run of the generator produces a different file and silently breaks the cross-port identity.
  The identity is checked by hashing: the shipped book is sha256
  `5f42b523bb68b011be8989218888c4ddb767c9f89c35100cc68b9115c09e9a19`, 3,328 opening lines,
  162,286 bytes, LF - the screened book taken on 2026-10-31 (`pawnstar` `a7a9198`, here `ae476ee`),
  which is `../pawnstar/pawnstar.book` and `../pawnstar-go/engine/book.txt` blob for blob. It
  replaced the unscreened 3,244-line, 155,503-byte book
  (`154228ffc843dc8068b6d0da2ff7df9daa39442e923d1eb6a82fc93ca448bcae`) of the day before.
- **`.gitattributes` marks `pawnstar.book` `-text`, and that line is load-bearing.** This clone is
  `core.autocrlf=true`; without `-text` a Windows checkout would embed a CRLF book where Linux and
  macOS embed LF, so the same commit would build two different binaries.
- **CC0 waives attribution, so every place that credits Lichess says plainly that nothing requires
  it.** Claiming a duty nobody owes is the mirror image of the defect that was fixed. Do not let a
  later edit turn the courtesy back into an obligation - `uci_binary_test` pins the sentence
  "CC0 waives attribution" for exactly that reason.
- **The book's header is the C++ port's prose, verbatim, because it is bytes.** It names
  `OpeningBook::ParseLineOfPlay`, `tools/make_book.cpp` and `nnue/openings.epd`, none of which exist
  in this repository. That is not a mistake to fix: editing the header changes the bytes.

## What is enforced, and where

- `tests/uci_binary_test.rs` - the `license` notice names the Lichess CC0 source and both URLs, says
  "CC0 waives attribution", and contains **neither** "TSCP" nor "Kerrigan".
- `tests/opening_book_test.rs` - the shipped **file** is loaded and checked for the same three
  things, plus `lines > 385`: a book truncated to its header would load fine and play nothing, and
  only the start-position check would have caught it. Since 2026-10-31 it also pins that the file
  carries `?` marks at all (so regenerating with `make_book` and forgetting `filter_book` is
  caught), that it says which moves are the engine's own and not the database's, and that every
  position a line continues from has a move that is actually selectable - the invariant the 84
  engine-chosen lines exist to guarantee.
- The bench signature is untouched by any of this - `bench` forces `OwnBook` off - so a changed
  bench after a book change means something unrelated broke, not the book. It stayed 5,617,906.
