//! MP-08 / MP-10 / MP-11: excluded metadata cannot enter revision metadata.
use super::*;
use crate::session::{RuntimeProject, RuntimeProjectKind};
#[test]
fn review1118_excluded_reason_is_rejected_before_preview_and_save() {
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
    draft.folders[0].requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"file","title":"Public document","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"public.md","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"exclude","reason":"api_key=synthetic-value","secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
    assert!(
        environment_revision_diff(&current, &draft).is_err(),
        "Preview must not retain the credential-shaped excluded reason"
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

#[test]
fn review1118_excluded_measurements_remain_kernel_owned() {
    let root = crate::test_support::TestWorktree::new("review1118-measurements");
    let project = RuntimeProject::new(
        "files",
        "owner",
        root.path().to_str().unwrap(),
        "Files",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let mut current = store.snapshot(&project).unwrap();
    for measured in [false, true] {
        let mut draft = environment_draft(&current);
        let folder = draft.folders[0].folder_id.clone();
        let inclusion = if measured { "include" } else { "exclude" };
        draft.folders[0].requirements = vec![serde_json::from_value(serde_json::json!({"requirement_id":"file","title":"Public document","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"public.md","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":inclusion,"reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap()];
        if measured {
            std::fs::write(root.path().join("public.md"), "public content").unwrap();
            verify_manual_files(&current, &mut draft).unwrap();
            store
                .save_revision_locked(
                    &current,
                    &crate::local::SaveProjectEnvironmentRevisionRequest {
                        project_id: project.id().into(),
                        expected_revision: current.revision,
                        expected_content_digest: current.content_digest.clone(),
                        draft: draft.clone(),
                        accepted_proposal_ids: vec![],
                        excluded_proposal_ids: vec![],
                    },
                    "owner",
                )
                .unwrap();
            current = store.snapshot(&project).unwrap();
            draft = environment_draft(&current);
            std::fs::remove_file(root.path().join("public.md")).unwrap();
        }
        let expected = match &draft.folders[0].requirements[0].spec {
            RequirementSpec::Files { entries, .. } => entries[0].clone(),
            _ => unreachable!(),
        };
        if let RequirementSpec::Files { entries, .. } = &mut draft.folders[0].requirements[0].spec {
            let entry = &mut entries[0];
            entry.transfer_inclusion = EnvironmentTransferInclusion::Exclude;
            entry.content_digest = Some("a".repeat(64));
            entry.byte_count = Some(999);
            entry.git_ignored = Some(true);
            entry.secret_looking = true;
        }
        verify_manual_files(&current, &mut draft).unwrap();
        if let RequirementSpec::Files { entries, .. } = &draft.folders[0].requirements[0].spec {
            assert_eq!(entries[0].content_digest, expected.content_digest);
            assert_eq!(entries[0].byte_count, expected.byte_count);
            assert_eq!(entries[0].git_ignored, expected.git_ignored);
            assert_eq!(entries[0].secret_looking, expected.secret_looking);
            assert_eq!(
                entries[0].credential_filter_verdict,
                expected.credential_filter_verdict
            );
        }
    }
}
