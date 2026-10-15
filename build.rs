// The engine's version, MAJOR.MINOR.BUILD, from git alone - the rule every reckless-technology project follows
// (pawnstar's cmake/version.cmake, pawnstar-go's Makefile, the plugins' and IR Capture's CMake). MAJOR.MINOR is the
// nearest annotated tag vMAJOR.MINOR reachable from HEAD, made by hand; BUILD is the number of commits since it, the
// tagged commit itself being 0. The release workflow's own three-component tags (vMAJOR.MINOR.BUILD) are excluded, so
// nothing in the tree is edited to release: tag main vMAJOR.MINOR and push the tag.
//
// A tree with no such tag reachable - a source tarball, a shallow clone, a fork without the tags - builds as 0.0.0 with
// a warning, which the release workflow refuses. The version reaches the code as PAWNSTAR_VERSION (src/version.rs).
use std::path::{Path, PathBuf};
use std::process::Command;

/// git's answer to `args`, trimmed, or None where git is missing or fails.
fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
}

/// "MAJOR.MINOR.BUILD" from `git describe`'s "vMAJOR.MINOR-BUILD-gHASH", or None for any other answer.
fn version_from_describe(describe: &str) -> Option<String> {
    let mut parts = describe.strip_prefix('v')?.split('-');
    let mark = parts.next()?;
    let build = parts.next()?;
    let hash = parts.next()?;
    let (major, minor) = mark.split_once('.')?;
    let is_number = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if parts.next().is_some()
        || !is_number(major)
        || !is_number(minor)
        || !is_number(build)
        || !hash.starts_with('g')
    {
        return None;
    }
    Some(format!("{major}.{minor}.{build}"))
}

fn main() {
    let describe = git(&[
        "describe",
        "--tags",
        "--long",
        "--match",
        "v[0-9]*.[0-9]*",
        "--exclude",
        "v*.*.*",
    ]);
    let version = describe
        .as_deref()
        .and_then(version_from_describe)
        .unwrap_or_else(|| {
            println!(
                "cargo:warning=No vMAJOR.MINOR tag is reachable from HEAD (git describe said {describe:?}): the \
                 engine reports 0.0.0. Fetch the tags (git fetch --tags; CI checks out with fetch-depth 0)."
            );
            "0.0.0".to_string()
        });
    println!("cargo:rustc-env=PAWNSTAR_VERSION={version}");

    // Build timestamp for the banner (the C++ __DATE__ __TIME__).
    let timestamp = Command::new("date")
        .arg("+%b %e %Y %H:%M:%S")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=PAWNSTAR_BUILD_TIMESTAMP={timestamp}");

    // Derived again when HEAD moves (a commit or a checkout writes logs/HEAD) or a tag arrives (refs/tags, or
    // packed-refs after a fetch). Only files that exist are named: cargo runs a script whose watched file is missing
    // on every build.
    if let (Some(git_dir), Some(common_dir)) = (
        git(&["rev-parse", "--git-dir"]),
        git(&["rev-parse", "--git-common-dir"]),
    ) {
        let git_dir = PathBuf::from(git_dir);
        let common_dir = PathBuf::from(common_dir);
        let watched: [&Path; 4] = [
            &git_dir.join("HEAD"),
            &git_dir.join("logs/HEAD"),
            &common_dir.join("packed-refs"),
            &common_dir.join("refs/tags"),
        ];
        for path in watched.iter().filter(|path| path.exists()) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
