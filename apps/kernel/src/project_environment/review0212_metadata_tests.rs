//! MP-08 / MP-10 / MP-11: assignment-shaped paths cannot enter revision metadata.
use super::*;
#[test]
fn review0212_credential_shaped_filename_is_rejected_before_projection() {
    let root = crate::test_support::TestWorktree::new("review0212-path");
    let project = RuntimeProject::new(
        "path",
        "owner",
        root.path().to_str().unwrap(),
        "Path",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let current = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&current);
    let folder = draft.folders[0].folder_id.clone();
    std::fs::write(
        root.path().join("api_key=synthetic-value.txt"),
        "public documentation",
    )
    .unwrap();
    draft.folders[0].requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"file","title":"Public document","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"api_key=synthetic-value.txt","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"include","reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
    assert!(
        environment_revision_diff(&current, &draft).is_err(),
        "Preview must not retain the raw credential-shaped filename"
    );
    let request = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: current.revision,
        expected_content_digest: current.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    assert!(store
        .save_revision_locked(&current, &request, "owner")
        .is_err());
    assert_eq!(store.snapshot(&project).unwrap().revision, current.revision);
}
