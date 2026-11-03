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
            inventory::satisfies(version, constraint, inventory::VersionSemantics::Npm),
            Some(expected),
            "{version} against {constraint}"
        );
    }
}

// MP-08 / MP-10: identical syntax follows the declared ecosystem in Check/Save.
#[test]
fn review2018_partial_bounds_select_software_semantics() {
    for (identity, source, version, lower, upper) in [
        ("python", Some("mise"), "3.10.12", true, false),
        ("python3", None, "3.10.12", true, false),
        ("node", Some("mise"), "3.10.12", false, true),
        ("public-tool", Some("npm package"), "3.10.12", false, true),
    ] {
        let semantics = inventory::VersionSemantics::software(identity, source);
        assert_eq!(
            inventory::satisfies(version, ">3.10", semantics),
            Some(lower),
            "{identity}"
        );
        assert_eq!(
            inventory::satisfies(version, "<=3.10", semantics),
            Some(upper),
            "{identity}"
        );
    }
    let numeric = inventory::VersionSemantics::Numeric;
    assert_eq!(
        inventory::satisfies("3.10.0", ">3.10", numeric),
        Some(false)
    );
    assert_eq!(
        inventory::satisfies("3.10.0", "<=3.10", numeric),
        Some(true)
    );
}
