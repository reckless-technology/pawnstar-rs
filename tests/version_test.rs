//! The version build.rs stamps is MAJOR.MINOR.BUILD, three decimal numbers, as the release workflow and the UCI
//! "id name" line expect.
use pawnstar::version::version_string;

#[test]
fn version_is_three_numbers() {
    let version = version_string();
    let parts: Vec<&str> = version.split('.').collect();
    assert_eq!(
        parts.len(),
        3,
        "version {version:?} is not MAJOR.MINOR.BUILD"
    );
    for part in parts {
        assert!(
            !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()),
            "version {version:?} has a part that is not a number"
        );
    }
}
