// Mirrors cmake/write_build_number.cmake: the build number is the git commit count
// (`git rev-list --count HEAD`) — globally reproducible, 0 outside a git checkout.
fn main() {
    let n = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "0".to_string());
    println!("cargo:rustc-env=PAWNSTAR_BUILD_NUMBER={n}");
    println!("cargo:rerun-if-changed=.git/HEAD");
}
