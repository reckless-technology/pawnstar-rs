//! Engine version constants and the "major.minor.build" version string. (Port of `version.h`.)

// Engine version, presented to the UCI host as "id name Pawnstar <major>.<minor>.<build>".
//
// The major and minor numbers are edited by hand below (the human-meaningful version). The build
// number is supplied at compile time via the PAWNSTAR_BUILD_NUMBER env (set by build.rs from the
// git commit count, `git rev-list --count HEAD`) — a globally-reproducible number, identical for
// every clone at a given commit; 0 outside a git checkout.
pub const VERSION_MAJOR: i32 = 0;
pub const VERSION_MINOR: i32 = 13;

/// The build number stamped by build.rs.
pub const BUILD_NUMBER: &str = env!("PAWNSTAR_BUILD_NUMBER");

/// The engine version as a "major.minor.build" string (e.g. "0.12.5").
pub fn version_string() -> String {
    format!("{}.{}.{}", VERSION_MAJOR, VERSION_MINOR, BUILD_NUMBER)
}
