//! MP-08 / MP-10 / MP-11: public revision review regressions.
use super::*;
// MP-08 / MP-10 / MP-11: Setup references are public metadata, too.
#[test]
fn review0821_credential_shaped_setup_source_is_rejected_before_projection() {
    let root = crate::test_support::TestWorktree::new("review0821-setup");
    let project = RuntimeProject::new(
        "setup",
        "owner",
        root.path().to_str().unwrap(),
        "Setup",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let current = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&current);
    let make = |path: &str| {
        serde_json::from_value(serde_json::json!({"requirement_id":"setup","title":"Public setup","scope":{"kind":"project"},"origins":[],"spec":{"kind":"setup_checks","boundary":"disposable_worker","commands":[],"probes":[],"source_path":path,"source_digest":null,"legacy":null},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap()
    };
    draft.project_requirements = vec![make("scripts/setup.sh")];
    environment_revision_diff(&current, &draft).expect("ordinary public reference remains valid");
    draft.project_requirements = vec![make("api_key=synthetic-value.sh")];
    assert!(
        environment_revision_diff(&current, &draft).is_err(),
        "Preview must reject credential-shaped source metadata"
    );
    let save = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: current.revision,
        expected_content_digest: current.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    assert!(store
        .save_revision_locked(&current, &save, "owner")
        .is_err());
    assert_eq!(store.snapshot(&project).unwrap().revision, current.revision);
}
