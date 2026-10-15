//! The engine's version string. (Port of `version.h`.)

/// The engine's version, MAJOR.MINOR.BUILD, presented to the UCI host as "id name Pawnstar <version>". build.rs derives
/// it from git: MAJOR.MINOR from the nearest tag vMAJOR.MINOR, BUILD the number of commits since it (0.0.0 where no such
/// tag is reachable). Nothing here is edited to release: tag main and push the tag.
pub const VERSION: &str = env!("PAWNSTAR_VERSION");

/// The engine version as a "major.minor.build" string (e.g. "1.0.5").
pub fn version_string() -> String {
    VERSION.to_string()
}
