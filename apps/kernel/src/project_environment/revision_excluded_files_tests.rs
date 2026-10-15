//! MP-08 / MP-10 / MP-11: excluded metadata survives absence; synthetic regression only.
use super::*;
#[test]
fn revision_excluded_files_absent_files_survive_preview_and_save() {
    for deleted_selection in [false, true] {
        let root = crate::test_support::TestWorktree::new("review0212-excluded");
        let project = RuntimeProject::new(
            "excluded",
            "owner",
            root.path().to_str().unwrap(),
            "Excluded",
            RuntimeProjectKind::Named,
        );
        let store = ProjectEnvironmentStore::new(root.path());
        let mut current = store.snapshot(&project).unwrap();
        let mut draft = environment_draft(&current);
        let folder = draft.folders[0].folder_id.clone();
        let inclusion = if deleted_selection {
            "include"
        } else {
            "exclude"
        };
        draft.folders[0].requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"file","title":"Public document","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"public.md","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":inclusion,"reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
        if deleted_selection {
            std::fs::write(root.path().join("public.md"), "public documentation").unwrap();
            let request = crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: project.id().into(),
                expected_revision: current.revision,
                expected_content_digest: current.content_digest.clone(),
                draft,
                accepted_proposal_ids: vec![],
                excluded_proposal_ids: vec![],
            };
            store
                .save_revision_locked(&current, &request, "owner")
                .unwrap();
            current = store.snapshot(&project).unwrap();
            draft = environment_draft(&current);
            std::fs::remove_file(root.path().join("public.md")).unwrap();
            if let RequirementSpec::Files { entries, .. } =
                &mut draft.folders[0].requirements[0].spec
            {
                entries[0].transfer_inclusion = EnvironmentTransferInclusion::Exclude;
            }
        }
        environment_revision_diff(&current, &draft).unwrap();
        verify_manual_files(&current, &mut draft)
            .expect("excluded absence must not require a filesystem read during Preview");
        let request = crate::local::SaveProjectEnvironmentRevisionRequest {
            project_id: project.id().into(),
            expected_revision: current.revision,
            expected_content_digest: current.content_digest.clone(),
            draft,
            accepted_proposal_ids: vec![],
            excluded_proposal_ids: vec![],
        };
        store
            .save_revision_locked(&current, &request, "owner")
            .expect("excluded absence must remain saveable");
        let saved = store.snapshot(&project).unwrap();
        if let RequirementSpec::Files { entries, .. } = &saved.folders[0].requirements[0].spec {
            assert_eq!(
                entries[0].transfer_inclusion,
                EnvironmentTransferInclusion::Exclude
            );
        } else {
            panic!("Files lost");
        }
    }
}
