//! MP-08 / MP-10 / MP-11: review of delivered Detect constraints and bounded provenance.
use super::*;

#[test]
fn review931_7_manifest_runtime_versions_survive_proposal_merge() {
    let root = TestWorktree::new("envp02a-review7-versions");
    write(
        &root,
        "pyproject.toml",
        "[project]\nrequires-python = \">=3.12\"\n",
    );
    write(
        &root,
        "Cargo.toml",
        "[package]\nname = \"demo\"\nrust-version = \"1.85\"\n",
    );
    let detection = detect_environment(&[folder(&root)], "environment").unwrap();
    for (runtime, expected, path, line) in [
        ("python", ">=3.12", "pyproject.toml", 2),
        ("rust", "1.85", "Cargo.toml", 3),
    ] {
        let proposals: Vec<_> = detection.proposals.iter().filter(|p| matches!(&p.requirement.spec, RequirementSpec::Software { identity, .. } if identity == runtime)).collect();
        assert_eq!(proposals.len(), 1);
        assert!(
            matches!(&proposals[0].requirement.spec, RequirementSpec::Software { version_constraint: Some(v), .. } if v == expected),
            "{runtime} must preserve declared version"
        );
        assert!(proposals[0].requirement.origins.iter().any(|o| matches!(o, RequirementOrigin::Detected { relative_path, line: Some(n), .. } if relative_path == path && *n == line)));
    }
}
