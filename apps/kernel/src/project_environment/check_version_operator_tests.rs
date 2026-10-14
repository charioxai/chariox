//! MP-08 / MP-10 / MP-11: fail-first review regression.
use super::*;
#[test]
fn check_version_operator_constraints_preserve_operators() {
    for (version, constraint, expected) in [
        ("22.3.1", "<22.x", false),
        ("21.9.0", "<22.x", true),
        ("22.9.0", ">22.x", false),
        ("23.0.0", ">22.x", true),
        ("22.9.0", "<=22.x", true),
        ("23.0.0", "<=22.x", false),
        ("22.0.0", ">=22.x", true),
        ("1.3.0", "^1.2.x", true),
        ("2.0.0", "^1.2.x", false),
        ("0.3.0", "^0.2.x", false),
        ("0.0.9", "^0.0.x", true),
        ("1.3.0", "~1.2.x", false),
        ("1.2.9", "~1.2.x", true),
    ] {
        assert_eq!(
            inventory::satisfies(version, constraint),
            Some(expected),
            "{version} against {constraint}"
        );
    }
}
