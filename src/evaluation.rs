//! Static evaluation of a chess position. (Port of `evaluation.h`.)
//!
//! The C++ header exists only to declare `EvaluatePosition` ahead of the game/search include
//! cycle, with the definition living in the `search_state.h` hub; Rust has no include cycle, so
//! this module simply re-exports the definitions from `search_state`.

pub use crate::search_state::{evaluate_position, scale_by_rule50, RULE50_SCALE_THRESHOLD};
