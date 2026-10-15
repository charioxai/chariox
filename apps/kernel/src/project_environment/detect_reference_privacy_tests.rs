// MP-08 / MP-10 / MP-11: review931 ordinary names must survive protected-metadata admission.
use super::*;
#[test]
fn review931_2_ordinary_sk_names_preserve_subtrees_and_dependencies() {
    let root = TestWorktree::new("envp02a-review2");
    write(
        &root,
        "task-runner/package.json",
        r#"{"dependencies":{"ask-sdk":"1","flask-cors":"2"}}"#,
    );
    write(&root, "risk-model/disk-usage.ts", "process.env.API_KEY;\n");
    let detected = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(detected.code_folders.contains("folder"));
    for name in ["ask-sdk", "flask-cors"] {
        assert!(detected.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Software { identity, .. } if identity == name)), "{name}");
    }
    assert!(detected.proposals.iter().any(|p| p.requirement.origins.iter().any(|o| matches!(o, RequirementOrigin::Detected { relative_path, .. } if relative_path == "risk-model/disk-usage.ts"))));
    for value in [
        "sk-proj-private-fixture-never-retain",
        "dir/sk-ant-private-fixture",
        "sk-abcdefghijklmnopqrstuvwxyz0123456789",
    ] {
        assert!(!super::super::detect_index::safe_metadata(value));
    }
}

#[test]
fn review931_2_legacy_key_literals_never_become_evidence_digest() {
    let root = TestWorktree::new("envp02a-review2-key");
    write(
        &root,
        "example.txt",
        "sk-abcdefghijklmnopqrstuvwxyz0123456789",
    );
    let folders = vec![folder(&root)];
    let index = super::super::detect_index::collect(&folders).unwrap();
    let file = index
        .files
        .iter()
        .find(|file| file.path == "example.txt")
        .unwrap();
    assert!(
        file.protected && file.digest.is_none(),
        "legacy credential literals must never become a digest oracle"
    );
    let before = detect_environment(&folders, "environment").unwrap();
    write(
        &root,
        "example.txt",
        "sk-0123456789abcdefghijklmnopqrstuvwxyz",
    );
    assert_eq!(
        detect_environment(&folders, "environment")
            .unwrap()
            .evidence_digest,
        before.evidence_digest
    );
}
