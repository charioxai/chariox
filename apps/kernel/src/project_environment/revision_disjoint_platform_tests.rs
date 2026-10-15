//! MP-08 / MP-10 / MP-11: scopes conflict only where platform applicability intersects.
use super::*;
#[test]
fn review0212_disjoint_platform_requirements_do_not_conflict() {
    let root = crate::test_support::TestWorktree::new("review0212-platform");
    let project = RuntimeProject::new(
        "platform",
        "owner",
        root.path().to_str().unwrap(),
        "Platform",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let current = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&current);
    let make = |id: &str, os: &str, arch: &str, version: &str| -> Requirement {
        serde_json::from_value(serde_json::json!({"requirement_id":id,"title":"Node","scope":{"kind":"project"},"origins":[],"spec":{"kind":"software","identity":"node","version_constraint":version,"platform":{"os":os,"architecture":arch},"install_scope":"system","install_source":null,"detect_only":true},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap()
    };
    draft.project_requirements = vec![
        make("mac", "macos", "aarch64", "22"),
        make("linux", "linux", "x86_64", "24"),
    ];
    environment_revision_diff(&current, &draft)
        .expect("disjoint OS/architecture declarations are compatible");
    draft.project_requirements[1] = make("linux", "macos", "aarch64", "24");
    assert!(
        environment_revision_diff(&current, &draft).is_err(),
        "same platform conflicting constraints remain rejected"
    );
    if let RequirementSpec::Software { platform, .. } = &mut draft.project_requirements[0].spec {
        *platform = None;
    }
    if let RequirementSpec::Software { platform, .. } = &mut draft.project_requirements[1].spec {
        *platform = None;
    }
    draft.project_requirements[0].platform_variants = vec![EnvironmentPlatform {
        os: "macos".into(),
        architecture: "aarch64".into(),
    }];
    draft.project_requirements[1].platform_variants = vec![EnvironmentPlatform {
        os: "linux".into(),
        architecture: "x86_64".into(),
    }];
    environment_revision_diff(&current, &draft).expect("disjoint platform variants are compatible");
    draft.project_requirements[1].platform_variants.clear();
    assert!(
        environment_revision_diff(&current, &draft).is_err(),
        "unconstrained platform overlaps an applicable variant"
    );
}
