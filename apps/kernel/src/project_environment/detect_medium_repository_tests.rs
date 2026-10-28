// MP-08 / MP-10 / MP-11: medium code trees retain late manifests and merged secret provenance.
use super::*;
#[test]
fn review931_3_medium_repo_preserves_late_manifests_and_deduplicates_references() {
    let root = TestWorktree::new("envp02a-review3");
    write(&root, "package.json", "{}");
    for i in 0..2200 {
        write(
            &root,
            &format!("apps/a/source-{i}.ts"),
            "process.env.API_KEY;\n",
        );
    }
    write(
        &root,
        "packages/z/package.json",
        r#"{"engines":{"node":"22"}}"#,
    );
    let d = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(d.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Software { identity, version_constraint, .. } if identity == "node" && version_constraint.as_deref() == Some("22"))));
    let secrets: Vec<_> = d.proposals.iter().filter(|p| matches!(&p.requirement.spec, RequirementSpec::Secrets { name, .. } if name == "API_KEY")).collect();
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0].requirement.origins.len(), 128);
    assert!(d
        .skips
        .iter()
        .any(|s| s.reason_code == "origin_limit" && s.safe_summary.contains("2072")));
    assert!(d.proposals.len() < 20);
    assert!(!d.skips.iter().any(|s| s.reason_code == "proposal_limit"));
}
