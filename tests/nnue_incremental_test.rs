//! Correctness gate for the incremental NNUE accumulator.
//! (Port of `test/nnue_incremental_test.cpp`.)
//!
//! Two halves, because a random walk and a chosen line catch different things.
//!
//! **Random search trees, on the shipped net.** The accumulator maintained incrementally by
//! SearchState across play_move/undo_move must yield exactly the same evaluation as a full
//! from-scratch refresh, at every node (covering captures, castling, en passant and promotions as
//! the random games progress, plus make/undo round-trips). Needs a net file; skipped without one.
//!
//! **Chosen boundary crossings, on a synthetic net at this build's shape.** A random game from the
//! start position hardly ever walks a king across the d/e file boundary, and with mirroring on that
//! is exactly the crossing whose refresh rule is silent when it is wrong — the king keeps its
//! bucket while every feature row the perspective reads moves. These lines cross each boundary on
//! purpose: the mirror, the board half, both on one ply, castling, the output-head boundaries, and
//! a promotion and an en passant with no king move. They compare the accumulators lane by lane
//! rather than the evaluations, and they report how many plies actually forced a refresh so a case
//! that quietly stopped crossing anything cannot pass. The net is built in memory at the compiled
//! shape, so this half runs in a checkout with no .bin and is the only way to exercise a shape
//! nothing has been trained for yet.
//!
//! The C++ drives the random walk with a seeded `std::mt19937_64`; the exact sequence is irrelevant
//! (the test asserts internal consistency), so this port substitutes the same xorshift* PRNG used
//! for the Zobrist tables, seeded identically per tree.

mod common;

use pawnstar::game::Game;
use pawnstar::moves::Move;
use pawnstar::nnue;
use pawnstar::position::Position;
use pawnstar::search_state::SearchState;
use std::collections::BTreeSet;

struct Rng {
    state: u64,
}

impl Rng {
    fn next(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

/// Whether two accumulators are identical lane by lane (not merely equal in evaluation).
fn same(a: &nnue::Accumulator, b: &nnue::Accumulator) -> bool {
    a.white == b.white && a.black == b.black
}

/// Recursively play random legal moves, comparing the incremental accumulator against a full
/// refresh at entry and again after undoing all children (to catch undo_move reverse bugs).
fn walk(state: &mut SearchState, rng: &mut Rng, depth: i32, branch: i32, checks: &mut i64) {
    let p = *state.current_position();
    let net = &state.game.nnue_network;
    let inc = net.evaluate(state.current_accumulator(), &p);
    let reference = net.evaluate_position(&p); // full refresh
    *checks += 1;
    if inc != reference {
        common::check(
            false,
            &format!(
                "incremental accumulator {} != full refresh {} fen={}",
                inc,
                reference,
                p.to_string()
            ),
        );
    }
    if depth <= 0 {
        return;
    }
    let moves = p.generate_legal_moves();
    if moves.size() == 0 {
        return;
    }
    for _ in 0..branch {
        let m = moves[(rng.next() % moves.size() as u64) as usize];
        state.play_move(m);
        walk(state, rng, depth - 1, branch, checks);
        state.undo_move();
    }
    // After undoing every child the accumulator must again match this position's full refresh.
    let net = &state.game.nnue_network;
    let inc_after = net.evaluate(state.current_accumulator(), &p);
    *checks += 1;
    if inc_after != reference {
        common::check(
            false,
            &format!(
                "incremental accumulator after undo {} != full refresh {} fen={}",
                inc_after,
                reference,
                p.to_string()
            ),
        );
    }
}

/// The legal move written `name` in `position`, or `Move::none()`.
fn find_move(position: &Position, name: &str) -> Move {
    for m in position.generate_legal_moves().iter() {
        if m.to_string() == name {
            return *m;
        }
    }
    Move::none()
}

/// How a failure names a position: the FEN plus each perspective's indexing and the output head,
/// which is what tells a bank change from a mirror change at a glance.
fn describe(position: &Position, what: &str, stage: &str) -> String {
    let white = nnue::perspective_for(position, false);
    let black = nnue::perspective_for(position, true);
    format!(
        "{} [{}] fen={} white(bank={} flip={}) black(bank={} flip={}) head={}",
        what,
        stage,
        position.to_string(),
        white.bank,
        white.square_flip,
        black.bank,
        black.square_flip,
        nnue::output_bucket(position)
    )
}

/// Play `moves` from `fen`, comparing the incrementally-updated accumulator with a from-scratch
/// rebuild at every ply, then walk the whole line back and compare again at every ply on the way.
///
/// This compares the **accumulators themselves**, lane by lane, not just the evaluations they
/// produce: two different accumulators can evaluate to the same number, so comparing evals would
/// let a real divergence through. It also records how many plies forced a refresh, because a case
/// that was *meant* to cross a boundary but did not would otherwise pass while testing nothing.
fn check_line(net: &nnue::Network, fen: &str, moves: &[&str], what: &str, checks: &mut i64) {
    // Replay the whole line first, so the walk back has every position to hand.
    let mut line = vec![Position::from_string(fen)];
    for name in moves {
        let m = find_move(line.last().unwrap(), name);
        if m == Move::none() {
            common::check(
                false,
                &format!(
                    "{}: '{}' is not legal in {}",
                    what,
                    name,
                    line.last().unwrap().to_string()
                ),
            );
            return;
        }
        line.push(line.last().unwrap().make_move(m));
    }

    let mut incremental = nnue::Accumulator::default();
    net.refresh(&mut incremental, &line[0]);
    let mut forced_refreshes = 0;
    for ply in 1..line.len() {
        let from = line[ply - 1];
        let to = line[ply];
        if nnue::perspective_for(&from, false) != nnue::perspective_for(&to, false)
            || nnue::perspective_for(&from, true) != nnue::perspective_for(&to, true)
        {
            forced_refreshes += 1;
        }
        net.update(&mut incremental, &from, &to);
        let mut rebuilt = nnue::Accumulator::default();
        net.refresh(&mut rebuilt, &to);
        *checks += 1;
        if !same(&incremental, &rebuilt) {
            common::check(
                false,
                &format!(
                    "incremental accumulator != full rebuild — {}",
                    describe(&to, what, &format!("ply {} forward", ply))
                ),
            );
            return;
        }
        *checks += 1;
        if net.evaluate(&incremental, &to) != net.evaluate_position(&to) {
            common::check(
                false,
                &format!(
                    "incremental evaluation != full refresh — {}",
                    describe(&to, what, &format!("ply {} forward", ply))
                ),
            );
            return;
        }
    }
    // And back: update is its own inverse with `from` and `to` swapped, which is what undo_move
    // relies on.
    for ply in (1..line.len()).rev() {
        net.update(&mut incremental, &line[ply], &line[ply - 1]);
        let mut rebuilt = nnue::Accumulator::default();
        net.refresh(&mut rebuilt, &line[ply - 1]);
        *checks += 1;
        if !same(&incremental, &rebuilt) {
            common::check(
                false,
                &format!(
                    "incremental accumulator != full rebuild on the way back — {}",
                    describe(&line[ply - 1], what, &format!("ply {} back", ply - 1))
                ),
            );
            return;
        }
    }
    common::check(
        true,
        &format!(
            "{}: {} plies, {} forcing a refresh, accumulator bit-identical forward and back",
            what,
            moves.len(),
            forced_refreshes
        ),
    );
}

/// The lines that cross the boundaries a random walk almost never reaches.
///
/// A random game from the start position hardly ever walks a king across the d/e file boundary, and
/// the mirror crossing is the one that is **silent** when it is wrong: at the mirrored map a king
/// stepping d1 -> e1 keeps its bucket (file 3 and file 4 both mirror to file 3) while every feature
/// row it reads moves, so a refresh rule that compares bucket indices alone diffs an accumulator
/// whose whole basis just changed. No crash, no assertion — just a wrong evaluation. These lines
/// make each crossing happen on purpose, and report how many plies actually forced a refresh so
/// that a case which quietly stopped crossing anything cannot pass.
fn boundary_crossings(net: &nnue::Network, checks: &mut i64) {
    // The white king stepping across d/e and back, then the black king doing the same. At the
    // mirrored shape each step flips that perspective's mirroring while leaving its bucket alone.
    check_line(
        net,
        "4k3/8/8/8/8/8/8/3K4 w - - 0 1",
        &[
            "d1e1", "e8d8", "e1d1", "d8e8", "d1e1", "e8f8", "e1f1", "f8e8",
        ],
        "kings crossing the d/e mirror boundary, both perspectives",
        checks,
    );
    // Crossing the half boundary (rank 4 -> 5) changes the bucket but not the mirroring: the other
    // half of the refresh rule, on its own. The idle king sits in a corner so the walking one is
    // never blocked.
    check_line(
        net,
        "7k/8/8/8/8/8/8/3K4 w - - 0 1",
        &[
            "d1d2", "h8g8", "d2d3", "g8h8", "d3d4", "h8g8", "d4d5", "g8h8", "d5d4", "h8g8",
        ],
        "the white king crossing the rank-4/5 half boundary",
        checks,
    );
    check_line(
        net,
        "3k4/8/8/8/8/8/8/7K b - - 0 1",
        &[
            "d8d7", "h1g1", "d7d6", "g1h1", "d6d5", "h1g1", "d5d4", "g1h1", "d4d5", "h1g1",
        ],
        "the black king crossing the rank-4/5 half boundary",
        checks,
    );
    // Both at once, diagonally: mirroring and bucket change on the same ply.
    check_line(
        net,
        "7k/8/8/8/3K4/8/8/8 w - - 0 1",
        &["d4e5", "h8g8", "e5d4", "g8h8", "d4e5", "h8g8"],
        "the white king changing mirroring and bucket on the same ply",
        checks,
    );
    // Castling moves a king two files at once, which on the mirrored map can take it from a
    // mirrored file to an unmirrored one in a single ply — and it is the one king move that also
    // moves a rook. The rooks are walked out of the kings' way as the line goes, so that every king
    // step below is legal: a rook left on d8 or a1 covers exactly the squares the kings need to
    // cross back over.
    check_line(
        net,
        "r3k2r/8/8/8/8/8/8/4K2R w Kkq - 0 1",
        &[
            "e1g1", "e8c8", "f1f5", "d8g8", "g1f1", "c8b8", "f1e1", "b8a8", "e1d1", "a8b8",
        ],
        "castling both ways, then the kings walking back across the boundary",
        checks,
    );
    // A promotion changes a piece type without a king moving, and en passant removes a pawn from a
    // square that is neither the from- nor the to-square of the move.
    check_line(
        net,
        "4k3/6P1/8/3pP3/8/8/8/4K3 w - d6 0 1",
        &["e5d6", "e8d8", "g7g8q", "d8d7", "g8d8"],
        "en passant and a promotion, with no king move",
        checks,
    );
}

/// Walk the piece count all the way down from the start position, preferring captures, and check
/// the accumulator at every ply.
///
/// The output head is `(piece_count - 2) / 4`, so its boundaries are at 6, 10, 14, 18, 22, 26 and 30
/// pieces. Hand-writing a legal capture chain that crosses all of them is fiddly and brittle — the
/// C++'s first attempt at one had an illegal move in it — so the line is generated: take a capture
/// whenever one is legal, otherwise a move, from a fixed seed. That walks the count down on its own
/// and reaches every head a position can select, and the test reports which heads it actually
/// visited rather than assuming.
fn capture_walk_through_output_heads(net: &nnue::Network, checks: &mut i64) {
    let mut rng = Rng { state: 20261027 };
    let mut current =
        Position::from_string("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w - - 0 1");
    let mut incremental = nnue::Accumulator::default();
    net.refresh(&mut incremental, &current);
    let mut heads_visited = BTreeSet::new();
    heads_visited.insert(nnue::output_bucket(&current));
    let mut plies = 0;
    while plies < 400 {
        let captures = current.generate_legal_captures();
        let moves = current.generate_legal_moves();
        if moves.size() == 0 {
            break;
        }
        let m = if captures.size() > 0 {
            captures[(rng.next() % captures.size() as u64) as usize]
        } else {
            moves[(rng.next() % moves.size() as u64) as usize]
        };
        let next = current.make_move(m);
        net.update(&mut incremental, &current, &next);
        let mut rebuilt = nnue::Accumulator::default();
        net.refresh(&mut rebuilt, &next);
        plies += 1;
        *checks += 1;
        if !same(&incremental, &rebuilt)
            || net.evaluate(&incremental, &next) != net.evaluate_position(&next)
        {
            common::check(
                false,
                &format!(
                    "capture walk: incremental != full rebuild at ply {} after {}, fen={} head={}",
                    plies,
                    m.to_string(),
                    next.to_string(),
                    nnue::output_bucket(&next)
                ),
            );
            return;
        }
        heads_visited.insert(nnue::output_bucket(&next));
        current = next;
    }
    let visited: Vec<String> = heads_visited.iter().map(|h| h.to_string()).collect();
    common::check(
        heads_visited.len() == nnue::NUM_OUTPUT_BUCKETS,
        &format!(
            "capture walk: {} plies, accumulator bit-identical throughout, output heads visited \
             {{{}}} of {}",
            plies,
            visited.join(","),
            nnue::NUM_OUTPUT_BUCKETS
        ),
    );
}

#[test]
fn nnue_incremental_suite() {
    let mut checks: i64 = 0;

    // The synthetic net first, and unconditionally: it is at this build's shape, so it runs in a
    // checkout with no .bin and it is the only way to exercise a shape no net has been trained for
    // yet.
    {
        let mut net = Box::new(nnue::Network::new());
        let image = nnue::synthetic_net_image();
        if !net.load_from_memory(&image, "synthetic").is_loaded() {
            common::check(
                false,
                "incremental NNUE: this build refused its own synthetic net — the header \
                 expected_net_header writes and the one load_from_memory accepts disagree",
            );
            assert_eq!(common::summary("INCREMENTAL NNUE"), 0);
            return;
        }
        common::check(
            true,
            &format!(
                "synthetic net at this build's shape loads: {} king buckets, {} output head(s), \
                 mirroring {}, {}-byte payload",
                nnue::NUM_KING_BUCKETS,
                nnue::NUM_OUTPUT_BUCKETS,
                if nnue::IS_KING_MIRRORING_ENABLED {
                    "on"
                } else {
                    "off"
                },
                nnue::NET_PAYLOAD_BYTES
            ),
        );
        boundary_crossings(&net, &mut checks);
        capture_walk_through_output_heads(&net, &mut checks);
    }

    let net_path = std::env::var("PAWNSTAR_EVALFILE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/nnue/pawnstar-v12.bin").to_string()
    });
    if !std::path::Path::new(&net_path).exists() {
        let failures = common::summary(&format!(
            "INCREMENTAL NNUE: {} checks bit-identical on the synthetic net (no net file supplied)",
            checks
        ));
        assert_eq!(failures, 0);
        return;
    }
    // Load the net into a Game; SearchState then maintains the accumulator incrementally. At a
    // shape the shipped net was not trained for the loader rightly refuses it, which is the
    // container working: the synthetic half above is then the whole suite.
    let mut game = Box::new(Game::new());
    if !game.nnue_network.load(&net_path).is_loaded() || !game.nnue_network.loaded {
        let failures = common::summary(&format!(
            "INCREMENTAL NNUE: {} checks bit-identical on the synthetic net ('{}' is not this \
             build's shape)",
            checks, net_path
        ));
        assert_eq!(failures, 0);
        return;
    }

    // Several independent random search trees from the start position.
    for seed in 1u64..=12 {
        game.set_position_start();
        let mut state = SearchState::new(&game);
        let mut rng = Rng { state: seed };
        walk(
            &mut state,
            &mut rng,
            /*depth=*/ 10,
            /*branch=*/ 2,
            &mut checks,
        );
    }

    let failures = common::summary(&format!(
        "INCREMENTAL NNUE: {} checks bit-identical to full refresh, over the shipped net's random \
         walks and the boundary-crossing lines on a synthetic net at this build's shape",
        checks
    ));
    assert_eq!(failures, 0);
    assert!(checks > 0);
}
