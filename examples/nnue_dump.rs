//! Ad-hoc parity tool: print the int8 NNUE eval for each FEN on stdin (cross-checked against the
//! C++ engine's `nnue` command).
use pawnstar::{nnue::Network, position::Position};
fn main() {
    let mut net = Box::new(Network::new());
    assert!(net.load(concat!(env!("CARGO_MANIFEST_DIR"), "/nnue/pawnstar-v12.bin")));
    let stdin = std::io::read_to_string(std::io::stdin()).unwrap();
    for fen in stdin.lines() {
        println!("nnue {}", net.evaluate_position(&Position::from_string(fen)));
    }
}
