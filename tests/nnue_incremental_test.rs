//! Correctness gate for the incremental NNUE accumulator.
//! (Port of `test/nnue_incremental_test.cpp`.)
//!
//! Verifies that the accumulator maintained incrementally by SearchState across
//! play_move/undo_move yields exactly the same evaluation as a full from-scratch refresh, at
//! every node of a random search tree (covering captures, castling, en passant and promotions as
//! the random games progress, plus make/undo round-trips). Skipped (passes) when the net is
//! absent.
//!
//! The C++ drives the walk with a seeded `std::mt19937_64`; the exact sequence is irrelevant (the
//! test asserts internal consistency), so this port substitutes the same xorshift* PRNG used for
//! the Zobrist tables, seeded identically per tree.

mod common;

use pawnstar::game::Game;
use pawnstar::search_state::SearchState;

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

/// Recursively play random legal moves, comparing the incremental accumulator against a full
/// refresh at entry and again after undoing all children (to catch undo_move reverse bugs).
fn walk(state: &mut SearchState, rng: &mut Rng, depth: i32, branch: i32, checks: &mut i64) {
    let p = *state.current_position();
    let net = &state.game.nnue_network;
    let inc = net.evaluate(state.current_accumulator(), p.color_to_move);
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
    let inc_after = net.evaluate(state.current_accumulator(), p.color_to_move);
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

#[test]
fn nnue_incremental_suite() {
    let net_path = std::env::var("PAWNSTAR_EVALFILE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/nnue/pawnstar-v12.bin").to_string()
    });
    if !std::path::Path::new(&net_path).exists() {
        common::summary("incremental NNUE: skipped (no net supplied)");
        return;
    }
    // Load the net into a Game; SearchState then maintains the accumulator incrementally.
    let mut game = Box::new(Game::new());
    if !game.nnue_network.load(&net_path) || !game.nnue_network.loaded {
        common::check(
            false,
            &format!("incremental NNUE: failed to load net '{}'", net_path),
        );
        assert_eq!(common::summary("INCREMENTAL NNUE"), 0);
        return;
    }

    // Several independent random search trees from the start position.
    let mut checks: i64 = 0;
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
        "INCREMENTAL NNUE: {} checks bit-identical to full refresh",
        checks
    ));
    assert_eq!(failures, 0);
    assert!(checks > 0);
}
