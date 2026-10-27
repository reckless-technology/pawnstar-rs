# Pawnstar (Rust)

A UCI chess engine written in Rust: a mechanical port of the C++
[Pawnstar](https://github.com/jonny-reckless/pawnstar) engine. It has a bitboard board representation with magic
bitboards, fully legal move generation, a Lazy SMP alpha-beta search, and an NNUE evaluation (1024 wide, 8 king
buckets). A sibling [Go port](https://github.com/jonny-reckless/pawnstar-go) exists too, and all three engines are
bit-exact with one another.

**Bit-exact with the C++ original.** At a fixed depth, single-threaded, this port runs exactly the same search as
the C++ engine: `bench` = **6,601,454 nodes** at depth 13. The per-position node counts, best moves, PVs, scores and
every DEBUGX diagnostic counter are byte-identical to the C++ reference. This was checked by diffing transcripts at
depths 8 to 13, and `tests/bench_golden_test.rs` locks it in.

## Build and run

**Setting up a machine.** `tools/setup-dev.ps1` (Windows) and `tools/setup-dev.sh` (Linux, macOS) install what
building, testing, debugging and coverage need - rustup with the stable toolchain, rustfmt, clippy and llvm-tools,
cargo-llvm-cov, Visual Studio's C++ build tools on Windows (behind a UAC prompt), a bash >= 4 for the UCI shell suite
(Git Bash on Windows, Homebrew's on macOS) - plus the git hooks and the recommended VS Code extensions, checking each
first, so they are safe to run again; `-Check` / `--check` only reports. In VS Code they are the `setup environment`
and `check environment` tasks.

```bash
cargo build --release          # engine → target/release/pawnstar (net + book embedded)
./target/release/pawnstar      # interactive UCI session
```

Point any UCI GUI (Arena, Cute Chess, etc.) at the binary; it needs no configuration. The engine loads
`nnue/pawnstar-v12.bin` and `pawnstar.book` from the working directory, or from the executable's directory or its
parent. If neither file can be read, it falls back to the copies embedded in the binary (`include_bytes!`, the C++
`.incbin`), so the executable is self-contained.

The `dev` profile builds at opt-level 3 **with overflow checks on**. Every intended wrap is spelled `wrapping_*`, so
an overflow panic in a dev build means a porting bug. The `release` profile matches the C++ semantics. Diagnostic
counters (DEBUGX) are compiled in by default; `--no-default-features` leaves them out (the C++ `RELEASE=1`).

```bash
cargo run --release --bin stamp_net -- <in.bin> <stamped.bin>    # stamp a trainer net into the RNNUE1 container
cargo run --release --bin dump_magics                            # regenerate the magic multipliers
cargo run --release --example nnue_dump < fens.txt               # raw evals for FENs (cross-engine eval parity)
cargo doc --open                                                 # API documentation (the C++ `make doc`)
```

Environment variables, the same as the C++ engine's:

| Variable | Effect |
|---|---|
| `PAWNSTAR_THREADS` | The default Lazy SMP thread count. `1` gives a deterministic single-threaded search. |
| `PAWNSTAR_EVALFILE` | Overrides the path of the net. |
| `PAWNSTAR_BENCH_DEBUG=1` | Makes `bench` print each position's best move, node count and DEBUGX counters, for cross-engine comparison. |
| `PAWNSTAR_PERFT_DEPTH` | Test only: caps the perft depth for quick iteration. |
| `PAWNSTAR_BK_DEPTH` | Test only: overrides the Bratko-Kopec search depth. |

## Testing

```bash
cargo test --release                              # every suite below
cargo test --release --test perft_test            # one suite
bash test/uci_test.sh target/release/pawnstar     # the UCI integration suite on its own (bash >= 4, for coproc)
```

`cargo test` covers everything the C++ `make check` runs, plus two gates of its own:

| Suite | What it holds |
|---|---|
| `perft_test` | Move generation: the 131-record standard EPD to depth 6, every count exact. |
| `see_test` | 24 static exchange cases. Also checks the fast, make-move-free SEE against the make-move reference over a recursive move tree. |
| `nnue_test` | The exact int16 forward pass matches the trainer's 250 reference evals within ±2 cp. The int8 output layer stays within 40 cp of it. |
| `nnue_incremental_test` | The lazily updated accumulator is bit-identical to a full refresh over random move sequences, including king-bucket crossings. |
| `bratko_kopec_nnue_test` | A hard gate: all 24 Bratko-Kopec positions solved at depth 8, single-threaded. |
| `opening_book_test` | The book's parsing, lookup and `?` (questionable) moves. |
| `chess_clock_test` | Deadlines, elapsed time, `ponderhit` and reset. |
| `uci_shell_test` | Runs `test/uci_test.sh` against the built engine: handshake, `seldepth`, `searchmoves`, `currmove`, `stop` and `quit` written straight after `go infinite`, malformed `go` values and `bench`. |
| `uci_robustness_test` | Malformed FENs never panic, `go ponder` never answers before `ponderhit` or `stop`, a `stop` or `quit` written straight after `go infinite` always ends the search, and malformed `go` values are ignored. |
| `uci_commands_test` | Rust only: every UCI command and option in process, through `process_input` - each `position` form, each `go` keyword and its clock, each `setoption` name and range, the diagnostic commands, `bench`, `quit`, malformed lines and values, `PAWNSTAR_THREADS`, and every command that stops a search sent the instant after `go`, hundreds of times each under a watchdog. |
| `uci_binary_test` | Rust only: the binary's start-up - the banner, the book and the net found in the working directory, beside the executable, through `PAWNSTAR_EVALFILE` or not at all (the embedded fallbacks) - and `quit`. |
| `library_api_test` | Rust only: the public helpers the search never calls, the net loader's refusals, the book's in-memory form and display, the material-draw rules and the FEN reader's fallbacks. |
| `fifty_move_clock_test` | Rust only: castling advances the fifty-move clock and only captures and pawn moves restart it, the FEN after each castling reads back, a castling on the hundredth ply is a draw to the search and the eval, and the repetition scan walks past a castling without a false match. |
| `generated_data_test` | Rust only: golden values for the Zobrist keys and magic tables, printed by the C++. A unit test in `src/generated_data.rs` runs the tables' `const` generators at run time and holds them to the tables the compiler baked. |
| `bench_golden_test` | Rust only: the 6,601,454-node `bench` signature, and each position's node count and best move. |

The NNUE-dependent suites skip (and pass) when `nnue/pawnstar-v12.bin` is absent, as the C++ suites exit 0 without
it. Shipping a new net follows the C++ checklist (`../pawnstar/nnue/README.md` §7), plus re-blessing the golden test
here.

### Coverage

```bash
cargo llvm-cov --release --ignore-filename-regex 'src/bin/' --summary-only   # per-file line coverage
cargo llvm-cov --release --ignore-filename-regex 'src/bin/' --html --open    # line by line
```

CI fails a pull request under **90 % of lines** (the `coverage` job). The figure covers the engine library and the
`pawnstar` binary - everything under `src/` but `src/bin/`, whose `dump_magics` and `stamp_net` are developer tools
run by hand and are left out. Every suite runs instrumented, the binary suites included: the `pawnstar` they spawn is
the instrumented build, and it inherits cargo-llvm-cov's `LLVM_PROFILE_FILE`, so `main.rs` and the UCI handlers
count. An instrumented search is many times slower than a release one, so the whole run takes a while (the bench
golden alone some minutes).

To reproduce the parity check against the C++ engine, run both and compare their transcripts:

```bash
for e in ../pawnstar/build/pawnstar target/release/pawnstar; do
  PAWNSTAR_THREADS=1 PAWNSTAR_BENCH_DEBUG=1 $e <<< $'bench\nquit' |
    sed -E 's/time +[0-9]+ /time X /; s/nps +[0-9]+ /nps X /; s/([0-9]+ nodes) [0-9]+ nps/\1/' |
    awk '/^### pos 01/,0'
done   # diff the two halves: they are identical
```

## Architecture

Each `src/*.rs` module ports the C++ header of the same name, one to one (see [Structure](#structure)). The C++
`../pawnstar/CLAUDE.md` is the authoritative deep-dive on the design and on the experiments behind it. This section
describes the engine as this port implements it.

### Board representation

`Position` ([src/position.rs](src/position.rs)) is a 160-byte `Copy` value. It is `#[repr(C)]` in the C++ field
order, with a `const` assertion that its size is 160, as the C++ `static_assert`s. It is immutable once built:
`make_move` and `make_null_move` return a new `Position`, and the search keeps a stack of them (copy-make), so undoing
a move is a pop and there is no unmake logic.

| Field | Type | Holds |
|---|---|---|
| `pieces` | `[Bitboard; 7]` | One bitboard per piece type, indexed by `Piece` (`Pawn` = 1 … `King` = 6). Index 0 is the occupied squares, kept up to date as pieces move, so occupancy is a plain load. |
| `colors` | `[Bitboard; 2]` | One bitboard per colour. |
| `squares` | `[Piece; 64]` | The piece on each square, for O(1) lookup. |
| `checkers` | `Bitboard` | The pieces giving check to the side to move, recomputed by `make_move`. |
| `hash` | `u64` | The Zobrist hash, updated incrementally (computed from scratch only when a FEN is read). |
| `king_location` | `[Square; 2]` | Each king's square. |
| `en_passant_square` | `Square` | The en passant target, or 0 for none. |
| `reversible_move_count` | `u8` | The fifty-move clock, in plies. |
| `full_move_count` | `u8` | The full-move number, zero-based. |
| `castling_rights` | `CastlingRights` | Four bits. A 64-entry mask table clears the right a move gives up, so moving the king or a rook, or capturing a rook on its square, is one AND. |
| `is_null_move` | `bool` | Whether the last move was a pass, so that null moves never follow one another. |
| `color_to_move` | `Color` | The side to move. It is the last field, so it sits in what would otherwise be padding. |

`Bitboard` ([src/bitboard.rs](src/bitboard.rs)) wraps a `u64`. It has the set operators, `pop_count`, `lsb`, `msb`,
shifts in all eight compass directions (masking the files that would wrap), and an iterator over its set squares that
`for s in bitboard` loops use. The rank, file and square-colour masks are `const`.

### Move encoding

`Move` ([src/moves.rs](src/moves.rs)) is one `i64`:

```
bits  0– 5   to square
bits  6–11   from square
bits 12–14   moving piece
bits 15–17   captured piece (None for a non-capture)
bits 18–21   move type (NonCapture, Capture, Promotion{Knight,Bishop,Rook,Queen}, PawnDoublePush, EpCapture, Castling)
bit     22   gives-check flag
bits 23–24   TT node type (Cut, All, Pv)
bits 25–32   TT depth searched (i8)
bits 33–40   TT age / generation (u8)
bits 41–63   score (signed 23-bit)
```

Equality and hashing look only at the identity, bits 0–21, so the score and TT metadata never make two moves differ.
`Move::none()` is zero. The score is read with an arithmetic right shift, which sign-extends the 23-bit field as the
C++ does.

Bits 23–63 do two jobs. While a move is being ordered they hold its sort score. While it is stored in the
transposition table they hold the search score, depth, node type and age. A move is never both at once, so the bits
are shared, and a table cell is just two 64-bit words.

A `MoveList` is a `StackList<Move, 256>` ([src/stack_list.rs](src/stack_list.rs)): a fixed array on the stack and a
length. Its storage is deliberately left uninitialised (`MaybeUninit`), so making one does not clear 2 KB at every
node, and only the live prefix is ever read. Move generation allocates nothing.

`sort_moves` sorts a list by descending score with a **stable insertion sort**: a move shifts left only past strictly
smaller scores, so moves with equal scores keep their generation order. The C++ replaced `std::sort` with this sort
because unstable tie-breaks made the bench node count differ between standard libraries. Replacing it with
`sort_unstable` would break parity.

### Attack generation

Sliding attacks use **magic bitboards** ([src/attacks.rs](src/attacks.rs),
[src/generated_data.rs](src/generated_data.rs)). For each square, the relevant occupancy is masked, multiplied by a
precomputed magic number and shifted down to a slot: `slot = ((occupied & mask) * magic) >> shift`. That is one
64-bit multiply, portable to every CPU, with no reliance on BMI2's `pext`.

The slot does not index the attack sets directly. It indexes a one-byte `indices` table, which in turn indexes a
de-duplicated `attacks` table. Many occupancies share an attack set: a rook square has 4096 occupancies but only
about 100 distinct attack sets, and every square has fewer than 256, so a `u8` index is enough. The indirection
shrinks the tables about five-fold, to about 155 KB.

The 128 magic multipliers are baked-in constants, found offline by a fixed-seed randomised search (`dump_magics`,
which ports the C++ `std::mt19937_64` exactly and reproduces them bit for bit). The `attacks` and `indices` tables are
filled from them on first use (`LazyLock`). The builder asserts that no two occupancies with different attack sets
share a slot, so a changed mask generator cannot pass silently.

Everything else is a compile-time `const`, computed by `const fn`s that run the C++ startup algorithms:

- the knight, king and pawn attack tables;
- the empty-board bishop and rook rays, which pin detection uses;
- the rank masks, which the en passant discovered-check test uses;
- `INTERVENING_SQUARES[from][to]`, the squares strictly between two squares on a shared line;
- the Zobrist keys.

The Zobrist keys come from a fixed-seed xorshift* generator (seed `0xAA55AA55AA55AA55`, the C++ engine's), drawn in a
fixed order: the piece-square keys, then the 16 castling-rights keys, then en passant keys for ranks 3 and 6 only.
Reordering the draws would change every key and every node count. The `generated_data_test` suite holds them to the
C++ values.

### Move generation

`Position::gen_moves::<IS_WHITE, DO_ALL_MOVES>` ([src/position.rs](src/position.rs)) generates **fully legal** moves.
It is the C++ `GenMoves<Color, bool>` template as const generics. `generate_legal_moves` calls it with
`DO_ALL_MOVES = true`, and `generate_legal_captures` (for quiescence) with captures and promotions only.

**Pins.** `Pins::new` ([src/pins.rs](src/pins.rs)) finds the absolutely pinned pieces before any piece moves are
made:

1. Find the candidate pinners: enemy bishops, rooks and queens on an empty-board diagonal or line from the king.
2. For each one, take the intervening squares between it and the king.
3. If exactly one piece stands there and it is ours, it is pinned. Its allowed destinations are those squares plus
   the pinner's own square, so it may move along the pin or capture the pinner.

`allowed_squares(from)` gives a pinned piece its ray and every other piece all squares.

**Generation order:**

1. **The king's forbidden squares.** These are every square an enemy piece attacks, computed with our king removed
   from the occupancy, so a slider's attack runs on through the king's square (the king cannot step back along the
   check). Pawn attacks are two bulk shifts; every other piece is looked up.
2. **King moves** to the squares that are not forbidden: captures, then quiet moves.
3. **Double check** returns at once, since only the king can move.
4. **Single check** narrows the targets. Captures must take the checker, and quiet moves must land on the squares
   between the king and the checker.
5. **Knight, bishop, rook and queen moves.** Each piece's attack set is masked by its pin ray and then split into
   captures and quiet moves.
6. **Castling**, only when not in check, with the squares between king and rook empty and the squares the king
   crosses not attacked.
7. **Pawn moves**, all by bulk shifts of the pawn bitboard:
   - single and double pushes;
   - captures to the west and east;
   - promotions, both by push and by capture, each to queen, rook, bishop and knight.

   Each target is mapped back to its source by a fixed square delta and checked against that pawn's pin.
8. **En passant.** The capturing pawns are found with a reverse pawn-attack lookup from the target square. A capture
   is legal only if the captured pawn is a permitted capture, which also handles en passant out of check.

   There is one rare extra test. When the king shares the rank, both pawns leave that rank at once, and that can
   uncover a rook or queen. The test removes both pawns from the occupancy and looks for a horizontal rook ray from
   the king to an enemy rook or queen.

Perft confirms the generator against the 131-record standard EPD at depth 6. Perft runs at about 255 Mnps on an M1
Pro; see [Performance](#performance).

### Search

`Game` ([src/game.rs](src/game.rs)) holds what every search thread shares:

- the lockless transposition table and eval cache;
- the NNUE network, read-only during a search;
- the opening book;
- the chess clock;
- the options;
- the cancel, stop and ponder flags;
- the game history.

Everything a search thread writes is an atomic. Every other field is written only through `Engine::game_mut()`, which
joins the search worker first (see [Deviations](#deviations-from-the-c)).

Everything else is **per thread**. It lives on each thread's `SearchState` ([src/search_state.rs](src/search_state.rs)),
so the threads never contend:

```
SearchState
├── game                  &Game                    shared: TT, eval cache, net, clock, flags
├── positions             Vec<Position>            copy-make stack, seeded with the current position, reserved MAX_PLY + 4
├── hash_stack            Vec<HashEntry>           (hash, fifty-move clock) of every earlier position, for repetitions
├── acc_stack, acc_valid  Vec<Accumulator>, Vec<bool>   one NNUE accumulator per ply, brought up to date lazily
├── history               HistoryTable             butterfly history [side][from·to], i32, depth-weighted with gravity
├── killers               [[Move; 2]; MAX_PLY]     two quiet killer moves per ply
├── countermoves          [Move; 7·64]             best quiet reply to the previous move's (piece, to)
├── continuation_history  Box<[i16]>               (prev piece, to) × (piece, to): quiet follow-up scores
├── node_count            u64                      nodes searched by this thread
└── seldepth              i32                      deepest ply reached, for `info seldepth`
```

The stacks are reserved when the state is built, so no push during the search reallocates, and the per-node path
allocates nothing. `hash_stack` holds the whole game's history, so repetitions are found across the game as well as
the search.

#### Lazy SMP

`Game::search_root_node` runs `thread_count` threads (1 to 256). Each runs a complete, independent
iterative-deepening search of the root, and they share only the transposition table and eval cache: one thread's
entries prefill another's probes. There is no splitting of nodes and no work queue. The helpers are scoped threads
(`std::thread::scope`), joined when the main thread finishes and sets the cancel flag.

Thread 0 is authoritative: it prints `info`, manages time and returns the move. The helpers follow an iteration-skip
schedule (`SKIP_SIZE` / `SKIP_PHASE`), skipping selected depths so they don't recompute the main thread's tree in
lockstep.

The default thread count is `PAWNSTAR_THREADS` if set, else the machine's available parallelism. The UCI `Threads`
option changes it. With more than one thread the result is not deterministic; with one it is exactly the C++
engine's.

#### Iterative deepening and time

`search_root_node`:

1. Returns a book move at once if `OwnBook` is on and the book knows the position.
2. Applies `go searchmoves`, then returns a forced move (the only legal one) without searching.
3. Plans the time (below), ages the transposition table, and searches every root move at `START_DEPTH` (3) to order
   them.
4. Launches the threads. Each one's `iterative_deepen` runs from depth 4 upwards. At each depth it re-sorts the root
   moves by the previous iteration's scores and searches them all in one full-window pass. Alpha rises as moves
   improve, so every move after the first is a null-window scout.

There is no aspiration window: the C++ tested one and found no gain. The search stops early on a forced mate.

With a standard clock:

- The soft budget is `remaining / moves_to_go`, where `moves_to_go` is `movestogo` or `max(40 − move, 5)`.
- The hard budget is `max(100 ms, min(2 × soft, remaining − 1 s))`, less `Move Overhead`.
- Between iterations the main thread stops:
  - once the soft budget is spent;
  - after half of it, if the best move has been the same at every depth, or the score has stayed within 50 cp;
  - after a quarter of it, if both are true.
- The hard deadline, and any `go nodes` limit, is polled every 2048 nodes.

`movetime` is a hard budget. `depth`, `infinite` and pondering have none. The increment is parsed but deliberately
not added to the budget: the C++ tested adding it and found it neutral to slightly negative.

#### Alpha-beta

`SearchState::search` is a fail-soft negamax. At each node, in order:

1. Poll the deadline and node limit.
2. Return a draw for insufficient material, the fifty-move rule or a threefold repetition.
3. Extend by a ply when in check.
4. Probe the transposition table.
5. Apply internal iterative reduction.
6. Drop into quiescence at depth 0, unless in check.
7. At a non-PV node not in check, compute the static eval **once** and use it for both reverse futility pruning and
   the null-move gate.
8. Search the TT move, then generate, score and sort the rest.

Whether a node is null-window (`beta == alpha + 1`) is checked **again for each move** against the current alpha. Once
an earlier move raises alpha at a PV node, the remaining moves are null-window too, and LMP and LMR must apply to them;
parity depends on it.

| Technique | Rule |
|---|---|
| Check extension | A node in check searches one ply deeper. Quiescence hands a position in check back to `search`. |
| Promotion and en passant extensions | The child of a promotion or an en passant capture searches one ply deeper. |
| Transposition table | A Cut entry at sufficient depth returns its score if it is ≥ beta. An All entry returns its score if it is ≤ alpha. A PV entry extends the depth by one. |
| Internal iterative reduction | At depth ≥ 4 with no TT move, search one ply shallower. |
| Reverse futility pruning | At a non-PV node not in check, at depth ≤ 7 and outside the mate band, return the eval if `eval − 80·depth ≥ beta`. |
| Null-move pruning | At a non-PV node not in check, after a real move, with more than three pieces of our own and `eval ≥ beta`: pass and search at `depth − 4`. A fail-high returns beta. |
| Late move pruning | At a null-window node not in check, at depth ≤ 8, skip quiet non-checking moves once `move_index ≥ 3 + depth²`. Captures, promotions and checks are never pruned. |
| Principal variation search | The first move gets the full window and later moves a null window, re-searched with the full window on a fail-high. There is no scout when the node is in check or the move gives check. |
| Late move reduction | At a null-window node not in check, at depth > 2: −1 ply from the 5th move, −2 from the 8th (at depth > 3), −3 from the 14th. A fail-high is re-searched at full depth. |
| Mate scores | A mated node returns `−31900 + ply`, so a shorter mate scores higher. `info` reports `score mate N` beyond ±31000. |

The fifty-move clock counts plies since the last capture or pawn move (FIDE 9.3, and FEN's halfmove clock): every
other move advances it, castling included, and the position is a draw once it reaches 100. Draws by repetition look
back through `hash_stack` two plies at a time, starting four plies back, and stop at a clock of zero. A castling does
not zero the clock, so the walk carries on past it, but the castling rights it gives up are in the hash, so no
position before it matches. Two earlier matches make a threefold repetition. Insufficient material covers K v K,
K+minor v K, and K+B v K+B with the bishops on the same colour.

#### Quiescence

`search_quiescent` searches captures and promotions only. It stands pat on the static eval, and it neither probes nor
stores the transposition table. When in check it calls `search` instead, which searches every evasion.

The captures are sorted by SEE (below), so the loop **stops at the first losing capture**. Every move after it loses
material too, so none is searched and no SEE is computed again. In the C++, dropping the quiescence table for this
sorted early exit was worth +38.65 ± 13.78 Elo at 8+0.08.

#### Move ordering

The TT move is searched before any list is generated, so a cutoff on it saves generating and scoring the rest.
`score_and_sort_moves` then stores a 23-bit score in each move and sorts the list by it, in disjoint bands:

| Band | Score |
|---|---|
| Winning or equal captures and promotions | `2^20 + SEE` |
| First killer | `2^19 + 1` |
| Second killer | `2^19` |
| Countermove to the previous move | `2^19 − 1` |
| Other quiet moves | butterfly history + continuation history, clamped below the countermove |
| Losing captures | their (negative) SEE, below every quiet move |

Quiescence keys on the sign of that last band. A capture's SEE also says whether it gives check, which is recorded in
the move's check flag.

**History** ([src/history_table.rs](src/history_table.rs)) is a butterfly table, `[side to move][from·to]`, pooling
statistics across every ply. When a move raises alpha or causes a cutoff, it is credited with a depth-weighted bonus,
`min(depth², 400)`. On a cutoff, every quiet move searched before it at that node is debited by the same amount (the
malus).

Each update uses a gravity term, `entry += bonus − entry·|bonus| / 16384`. That keeps an entry within ±16384, where
recent evidence outweighs stale. The 1-ply continuation history (keyed by the previous move's (piece, to) and this
move's (piece, to), stored as `i16`) gets the same bonus and malus.

A quiet cutoff also becomes this ply's first killer (the old first killer moves to second), and the countermove to the
previous move. Quiescence keeps a plain increment on the same table. The history is signed, so only its high side is
clamped: an unsigned clamp would turn the malus into a top score.

### Transposition table

[src/transposition_table.rs](src/transposition_table.rs) is one table, 64 MB by default, resized and cleared by the
UCI `Hash` option. The number of cells is a power of two, so a cell is found by `hash & mask`.

The table is **lockless** (Hyatt's XOR trick). Each cell is two `AtomicU64`s, `key = hash ^ data` and `data` (the
packed move with its score, depth, node type and age). A reader loads both with relaxed ordering and accepts the cell
only if `key ^ data == hash`. That one check does two jobs:

- **It detects a torn pair.** If two threads write a cell at once and a reader sees one thread's `key` and the
  other's `data`, the XOR matches no real hash, and the read is a miss: never a wrong entry. Racing writers resolve to
  a harmless last-writer-wins.
- **It guards against collisions.** The full 64-bit hash is reconstructed, so an entry for another position that
  indexes the same cell is rejected.

**Replacement:** a cell is overwritten when it is empty, from an older search, no deeper than the new entry, or when
the new entry is a PV node.

**Aging** is O(1). An 8-bit generation is incremented before each search and stamped on every entry written, and an
entry whose age differs is stale. Nothing is cleared between searches; `ucinewgame`, `Clear Hash` and `bench` clear the table.

`search_single_move` prefetches the child's cell straight after making the move, so the cache line is resident by the
time the child probes it. `info hashfull` samples the first 1000 cells.

### Evaluation

`evaluate_position` ([src/search_state.rs](src/search_state.rs)) is the search's evaluation, in centipawns for the
side to move:

1. **Draws** (insufficient material, the fifty-move rule, repetition) return 0.
2. **The eval cache** ([src/eval_cache.rs](src/eval_cache.rs)) is checked next. It is a lockless 8 MB table of single
   `AtomicU64`s, each the top 48 bits of the hash beside a 16-bit score, so it needs no lock and cannot tear. A hit
   skips the network.
3. **The network** runs on a miss, on the accumulator brought up to date for this ply, and the result is stored in
   the cache.
4. **Fifty-move scaling** is applied on the way out, never stored. Above 20 plies on the fifty-move clock, the eval
   ramps linearly towards zero as the clock nears 100. NNUE sees only piece placement, so without the ramp a winning
   eval would be the same at clock 0 and 99, and the search would have no reason to prefer progress over shuffling.

The raw eval is clamped to ±30000, which keeps it below the mate band (±31000) and within the cache's 16 bits.

**Static exchange evaluation** ([src/static_exchange_evaluation.rs](src/static_exchange_evaluation.rs)) resolves the
sequence of captures on a square, least valuable attacker first: P = 100, N = B = 300, R = 500, Q = 900. It plays out
the captures, then unwinds them, letting either side stop rather than make a losing recapture. A recapturing pawn
reaching the last rank becomes a queen.

`evaluate_static_exchange` makes no move. It applies the capture to a lightweight `SeeBoard` of bitboards and works
out whether the move gives check from the attackers to the enemy king. That covers direct, discovered, promotion and
en passant checks alike. The `see_test` suite holds it to the make-move reference, `evaluate_static_exchange_reference`.

### NNUE

[src/nnue.rs](src/nnue.rs) is the engine's only evaluator, a port of the C++ network. It is a perspective net with
bullet's `ChessBuckets` features:

```
768 inputs per (perspective, king bucket)  →  1024 feature transformer, shared by both perspectives
SCReLU,  concat[side to move | other side] = 2048  →  1 output (centipawns, side to move)
QA = 255, QB = 64, SCALE = 400
```

**King buckets.** Each perspective picks one of **8** weight banks by its own king's square (`KING_BUCKET_MAP`). The
bank is the king's file pair (a–b, c–d, e–f, g–h), plus 4 when the king has advanced into the far half of the board.
The file pairs follow king safety and the castled king's pawn shelter; the rank split separates a sheltered king from
an active one.

**Feature index.** For the white perspective a piece's row is `bucket·768 + colour·384 + (type − 1)·64 + square`. The
black perspective swaps the colour and flips the square (`^ 0x38`). The feature-transformer weights are 6144 × 1024
`i16`, 12.6 MB, held in one 64-byte-aligned heap allocation.

**The accumulator** is two 1024-wide `i16` arrays, one per perspective, maintained **incrementally and lazily**.
`play_move` and `make_null_move` only mark the new ply stale, and `undo_move` does nothing. `current_accumulator`
catches up only when an evaluation actually needs it, which is on an eval-cache miss: it walks back to the nearest
up-to-date ancestor, then copies and updates one ply at a time. A node that cuts off before evaluating (a TT cutoff,
RFP, null move or LMP) pays nothing.

`Network::update` finds the changed squares straight from the parent's and child's colour bitboards. A move changes
two to four of them, and captures, en passant, castling and promotion all fall out the same way. Only those columns
are added or subtracted, in both perspectives. A king move that crosses into another bucket changes that
perspective's whole bank, so that side is rebuilt from scratch and the other is diffed. The `i16` arithmetic wraps
(`wrapping_add` / `wrapping_sub`), as the C++ does.

**The output layer** runs in **int8** on the search's path (`evaluate`):

- each SCReLU activation, `clamp(x, 0, 255)²`, is shifted right by 9 into `u8` (0–127);
- it is multiplied by the output weights, requantised to `i8` at load;
- the products are summed in `i32`, which provably cannot overflow and lets LLVM vectorise the loop.

The result is bit-identical to the C++ scalar, AVX2 and AVX-VNNI kernels, and within a bounded 40 cp (measured 26) of
the exact int16 forward pass, `evaluate_exact`. The exact pass is kept as the reference that `nnue_test` holds to the
trainer's evals.

**The net file** is in the shared Reckless NNUE container, `RNNUE1`: the 64-byte self-describing header every
reckless-technology chess engine reads, then the weight payload. The header records the architecture - the features
per bucket, the king bucket count, the hidden size, the output bucket count, the architecture flags, the activation,
QA, QB and the eval scale - plus a CRC-64/XZ of the king-bucket map's 64 entries, the exact payload size, a CRC-64/XZ
over the payload and one over the header's own first 56 bytes.

The loader makes the container's two checks, in order: the header (the magic, its own checksum, every architecture
field against this engine's compile-time constants, then the map's checksum against the checksum of this engine's own
`KING_BUCKET_MAP`), then the payload (its declared size against what remains of the file, then its checksum). A net
that fails any of them is **refused**: the engine names the field and both values on stderr and does not load it.
Nothing is written into the network until every check has passed, so a refused net leaves the net already playing
untouched rather than half-overwritten or unloaded. And a refused net is never papered over with the embedded one -
an engine that substituted a net would play a whole match on weights nobody asked for, and the match would report a
plausible Elo for a net that never played a move. Only a net file that is *missing* falls back to the embedded copy.

The map's contents are part of the architecture, which is what the map checksum closes: the same eight king buckets
meaning other squares would load cleanly and read the wrong weight bank. `stamp_net` writes the header from its own
build's constants, so a net stamped by a tool built at another architecture carries that architecture's header and is
then refused - the header, never the file name, says what a net is.

The shipped net, v12, was trained on about 6 billion public positions; its lineage and the experiments behind it are
in the C++ repository.

### Opening book

[src/opening_book.rs](src/opening_book.rs) reads `pawnstar.book`: 385 lines in coordinate notation, derived from the
public-domain TSCP `book.txt`. Each line is replayed from the start position, and the book maps each position's
Zobrist hash to the moves played from it, repeats included, so a uniform random pick is weighted by frequency.

A move written with a trailing `?` is *questionable*. It is replayed so the rest of its line still parses, but it is
never chosen. `bookmoves` lists a position's moves and their counts. The book is consulted only when `OwnBook` is on
and `go searchmoves` is not restricting the root.

### UCI protocol

[src/input_handling.rs](src/input_handling.rs) reads stdin line by line. It dispatches each command to its handler,
and runs searches on a worker thread so `stop` and `ponderhit` are heard during a search.

| Command | Notes |
|---|---|
| `uci`, `isready` | The handshake: `id`, the options, `uciok`; `readyok`. |
| `ucinewgame` | Stops any search, clears the transposition table and eval cache, and resets the position. |
| `position startpos \| fen <fen> [moves …]` | A malformed FEN never panics: missing fields take sane defaults, and anything unplayable falls back to the start position. |
| `go wtime/btime [winc/binc] [movestogo]` | A standard clock (see [time](#iterative-deepening-and-time)). |
| `go movetime <ms>`, `depth <n>`, `nodes <n>`, `infinite` | A fixed time, depth or node count, or until `stop`. The node limit is checked per thread, so it is exact at `Threads=1`. |
| `go searchmoves <m…>` | Restricts the root search to these moves. |
| `go … ponder`, `ponderhit` | Searches on the opponent's time. No `bestmove` is sent until `ponderhit` (which starts the clock from that moment) or `stop`. |
| `stop`, `quit` | Ends the search and sends `bestmove`, with `ponder <reply>` when the PV has a reply; `quit` also exits. |
| `bench [depth]` | The 16 Bratko-Kopec bench positions at depth 13, single-threaded, book off, with the table and eval cache cleared, printing nodes and nps. The node total is the parity signature. |
| `eval` | The search's full evaluation: draw short-circuit, cache, fifty-move scaling. |
| `nnue` | The raw network output, unscaled. |
| `getboard`, `bookmoves`, `freebook`, `dbg`, `dbgclear`, `help` | Diagnostics: the FEN, the book's moves, freeing the book, the DEBUGX counters and their reset, and the command list. |

| Option | Type | Default | Effect |
|---|---|---|---|
| `Hash` | spin 1–4096 | 64 | Transposition table size in MB (reallocated and cleared). |
| `Threads` | spin 1–256 | cores | Lazy SMP thread count. |
| `Move Overhead` | spin 0–5000 | 30 | Milliseconds held back from every hard deadline for GUI and network lag. |
| `OwnBook` | check | true | Consult the opening book. |
| `EvalFile` | string | empty: the shipped net | Load another `RNNUE1` net; clears the eval cache. A refused net is not loaded and the net already playing stays in use. |
| `Clear Hash` | button | | Empty the transposition table. |
| `Ponder` | check | false | Advertised for GUIs; pondering itself follows `go ponder`. |

`info` lines carry `depth`, `seldepth`, `score cp` or `score mate N`, `time`, `nodes`, `nps`, `hashfull` and `pv`.
Once a search has run for 3 s, they also carry `currmove` and `currmovenumber`.

### Search diagnostics

[src/debug_hashtable.rs](src/debug_hashtable.rs) keeps named DEBUGX counters. The `increment!` and `increment_if!`
macros count events through the search:

- beta cutoffs, and TT hits and cutoffs by node type;
- null-move, PVS and LMR attempts and failures;
- reverse futility prunes, late move prunes and quiescence SEE prunes;
- check, promotion and en passant extensions, and internal iterative reductions;
- draws, checkmates and stalemates, and eval-cache hits.

Each call site caches its counter's slot in a `OnceLock`. The counters are relaxed atomics, which single-threaded (the
mode the parity gate compares) are exactly the C++ plain integers. `dbg` prints them, and `bench` with
`PAWNSTAR_BENCH_DEBUG=1` prints them per position. The `debugx` feature compiles them in; without it the macros are
empty.

## Continuous integration

`.github/workflows/ci.yml` gates every push and pull request to `main` with four jobs:

- **build-test** runs `cargo build/test --release` and checks the bench determinism signature.
- **coverage** runs every suite under `cargo llvm-cov --release` and fails under 90 % of lines, `src/bin/` left out
  (see [Coverage](#coverage)). It builds the pinned cargo-llvm-cov from crates.io and keeps its binary in the Actions
  cache under that version, so it is built again only when the pin moves.
- **format** runs `cargo fmt --check`.
- **build + test (windows x86_64)** runs the same build and suites on Windows (MSVC). Checking the bench signature on
  a second platform makes it a cross-platform parity guarantee rather than a Linux-only one.

The signature the workflow checks is derived from `GOLDEN_TOTAL` in `tests/bench_golden_test.rs`, so the number has
one home.

**Every job runs on a GitHub-hosted image**, named by its version rather than `-latest`: `ubuntu-24.04` for the first
three and `windows-2025` for the Windows one. The images are free for a public repository, and each starts empty, so
every job installs what it needs: `dtolnay/rust-toolchain` brings the stable toolchain `rust-toolchain.toml` names
(with `llvm-tools` for coverage and `rustfmt` for the format check), and the Windows image carries Visual Studio's C++
build tools for the MSVC linker. The Windows job's shell is Git Bash 5.x, which satisfies the "bash >= 4 with coproc"
probe in `tests/uci_shell_test.rs`, so `test/uci_test.sh` really runs there rather than skipping.

## Versions and releases

The version is `MAJOR.MINOR.BUILD`, from git alone, the rule all of reckless-technology's projects follow:

- `MAJOR.MINOR` is the nearest annotated tag `vMAJOR.MINOR` reachable from the commit built (a *version mark*, made by
  hand on `main`);
- `BUILD` is the number of commits since that tag, the tagged commit itself being 0.

`build.rs` works it out (`git describe --tags --match 'v*.*' --exclude 'v*.*.*'`) and the engine reports it:
`id name Pawnstar 1.0.0`, and in the banner. Nothing in the tree holds a version to edit (`Cargo.toml`'s is a fixed
`0.0.0` placeholder). A tree with no mark reachable - a source tarball, a shallow clone, a fork without the tags -
builds as `0.0.0`, with a warning.

`.github/workflows/release.yml` makes a release:

- **Pushing a mark** (`git tag -a v1.1 -m "Pawnstar-RS 1.1" && git push origin v1.1`) releases `1.1.0`.
- **Running it by hand** on `main` (`gh workflow run release.yml --ref main`) releases `main`'s head as
  `MAJOR.MINOR.BUILD`. `-f dry_run=true`, from any branch, builds, tests and signs the macOS binary but publishes
  nothing.

It refuses a mark that isn't `vMAJOR.MINOR`, a commit not on `main`, and a version already released. Each platform is
built and tested from nothing on GitHub-hosted images (`ubuntu-24.04`, `windows-2025`, `macos-15`), with the bench
signature checked, and each shipped binary must report the version. The macOS job installs Homebrew's bash first, since
the image's own is 3.2 and the UCI shell suite would skip without a bash >= 4. The workflow creates the release
`vMAJOR.MINOR.BUILD` and its tag itself. It never runs for a pull request.

| File | What it is |
|---|---|
| `pawnstar-rs-<version>-windows-x86_64.exe` | the engine, Authenticode-signed through SSL.com eSigner (Jonathan Reckless) |
| `pawnstar-rs-<version>-macos-arm64.zip` | the engine for Apple silicon, signed with a Developer ID (hardened runtime) and notarised by Apple |
| `pawnstar-rs-<version>-linux-x86_64` | the engine for Linux |
| `pawnstar-rs-<version>-source.tar.gz`, `-docs-html.zip` | the source at the commit, and the API documentation |

The macOS engine ships in a zip because a bare binary can't carry a stapled notarisation ticket: Gatekeeper looks its
ticket up online, and the zip keeps the executable bit through a browser's download. The signing runs from
`tools/sign-macos.sh` and `tools/sign-windows.ps1`, with the organisation's signing secrets (the same ones as its other
apps). The Windows binary is signed only in a run that publishes, since every signing spends the eSigner account's
allowance. A publishing run that would ship an unsigned binary fails.

## Structure

Each `src/*.rs` module ports the same-named C++ header one to one: the same structs, fields and functions, with
snake_case names and otherwise mechanical. The exceptions are in this table.

| C++ | Rust | | C++ | Rust |
|---|---|---|---|---|
| `move.h` | `moves.rs` (`move` is a keyword) | | `search_state.h` | `search_state.rs` |
| `main.cpp` | `main.rs` | | `game.h` | `game.rs` |
| `embedded_*.S` | `include_bytes!` in `main.rs` | | `evaluation.h` | `evaluation.rs` (re-export stub) |
| `test/test_report.h` | `tests/common/mod.rs` | | everything else | same basename |

The search is reproducible across the port because of engineering inherited from the C++:

- `sort_moves` is a stable descending insertion sort, never `sort_unstable`.
- The Zobrist keys come from a fixed-seed xorshift* with a defined draw order; here they are a compile-time `const`.
- There is no floating point on the search or evaluation path.
- The NNUE accumulator uses wrapping `i16`.
- `Move` packs everything in an `i64`, whose arithmetic shift sign-extends the 23-bit score field.

C++ overloads become distinct names:

| C++ | Rust |
|---|---|
| `SetPosition()` | `set_position_start()` |
| `PlayMove(string)` | `play_move_str()` |
| `Network::Evaluate(Position)` | `evaluate_position()` |
| `EvaluateExact(Position)` | `evaluate_exact_position()` |
| the 3-argument `Move::Promotion` | `Move::promotion(from, to, promoted, Piece::None)`, with an identical encoding |

Some C++ is intentionally not ported:

- The two offline analysis tools, `filter_book` and `nnue_quant_study`. Working C++ versions live in
  `../pawnstar/tools/`.
- `Move::operator<`. It is unused, and it is inconsistent with the identity-masked equality under Rust's trait
  contracts.

## Deviations from the C++

Every deviation preserves the C++'s behaviour. The fifty-move rules are the C++'s and Go's too: castling advances
the clock as any other piece move does (FIDE 9.3 and FEN's halfmove clock restart it on a capture or a pawn move
alone), and a checkmate on the hundredth ply is mate, not the fifty-move draw (FIDE 5.1.1); `fifty_move_clock_test`
holds both.

- **Threading is type-encoded.** The C++ shares `Game&` mutably across threads by convention. Here:
  - everything a search thread writes is an atomic: the TT cells, the eval cache, the stop and ponder flags, the
    ponder move, the node count, and the clock instants as nanoseconds since a base, in `AtomicU64`s;
  - the game history is a `Mutex<Vec<Position>>`, which is never on the per-node hot path;
  - every other field is written only through `Engine::game_mut()`, which joins the worker first;
  - Lazy SMP helpers are scoped threads.
- `SearchState`'s C++ `mutable` lazy-accumulator stacks become `&mut self` methods (`current_accumulator`,
  `evaluate_position`).
- The book's random pick uses a wall-clock-seeded xorshift* instead of `std::mt19937`. The randomness affects no test
  or gate, since the book is off in `bench` and Bratko-Kopec. `dump_magics`, by contrast, ports `std::mt19937_64`
  exactly, so it reproduces the baked magic constants bit for bit.
- Numeric UCI values - `go` values, `setoption` numbers and `bench <depth>` - must be whole integers
  (`parse_integer`, the rule the C++ `ParseInteger` and Go's `strconv.Atoi` share). A `go` keyword whose value is missing
  or malformed is ignored with it (`go depth abc` is a plain `go`), an option is left as it was, and `bench` keeps its
  default depth. The `PAWNSTAR_THREADS` environment variable is still read with C++ `atoi` semantics (the leading
  digits, or 0 if there are none), as the C++ reads it.
- On a malformed FEN this engine is defined and safe, where the C++ has istream and indexing quirks. Missing fields
  take sane defaults, and the stream-poisoning on a bad halfmove field is reproduced.
- The int8 eval dot accumulates in `i32`, which is provably in range and gives identical results, so LLVM vectorises
  the scalar kernel. The C++ scalar kernel uses `int64` and its AVX2 kernel `int32`.
- `quit` stops the search and then returns from `main` (`process_input` returns false), where the C++ calls
  `exit(0)`. The exit code and the output are the same; returning runs the C runtime's exit handlers on every platform,
  where Rust's `process::exit` is `ExitProcess` on Windows and skips them - and with them an instrumented build's
  coverage profile.

## Performance

The NNUE kernels are scalar only. They are bit-identical to the C++ SIMD kernels by construction, and the C++ also
runs scalar on arm64, where this port was verified. Measured single-threaded on an Apple M1 Pro (medians over
alternating runs, with the history of the time):

| Metric | C++ | Rust | ratio |
|---|---|---|---|
| `bench` depth 13 (search nps) | ~1.15M | ~1.04M | 0.91× |
| Bratko-Kopec depth 12 (suite time) | ~5.2s | ~5.6s | 0.92× |
| perft suite (raw movegen Mnps) | 436 | 255 | 0.58× |

Real search speed is within about 10 % of the C++, since the search is bound by the evaluation and the eval kernels
vectorise.

Head to head at 10+0.1, the Rust port tied the C++ exactly at 150–150 over 300 games (UHO openings,
single-threaded): 81 wins each and 138 draws, 0.0 ± 25.1 Elo. A three-way round robin with the Go port put all three
engines within the error bars of one another (least squares, with C++ = 0: Go +13, Rust −7).

The perft gap is confined to raw move generation (`gen_moves`): per-element bounds checks and indirect attack-function
calls, which clang folds harder. Move generation is only about 1–2 % of a search node's cost, so it barely registers
in play.

Future work is x86-64 AVX2/VNNI output-dot kernels, using `std::arch` and the ported cpuid gate in `platform.rs`. They
are untested here for lack of x86 hardware, and any such change must reproduce the bench golden signature exactly.

## License

pawnstar-rs is free software, licensed under the **GNU General Public License, version 3** (GPLv3). You may
redistribute and/or modify it under the terms of that license; it comes with **no warranty**. See the
[LICENSE](LICENSE) file for the full text, or <https://www.gnu.org/licenses/gpl-3.0.html>.

The NNUE network ([nnue/pawnstar-v12.bin](nnue/pawnstar-v12.bin)) and the opening book
([pawnstar.book](pawnstar.book)) are the ones [Pawnstar](https://github.com/reckless-technology/pawnstar)
ships, under the same license; the book is derived from the public-domain
[TSCP](https://github.com/terredeciels/TSCP) `book.txt`.
