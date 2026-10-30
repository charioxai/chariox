//! MP-08 / MP-10 / MP-11: fail-first review regression.
use super::*;
// MP-08 / MP-10 / MP-11: exclusion preserves metadata, not measured readiness.
#[test]
fn review0821_absent_excluded_files_and_dependents_are_not_ready() {
    let root = crate::test_support::TestWorktree::new("review0821-excluded");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = project(root.path(), "excluded");
    let current = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&current);
    let folder = draft.folders[0].folder_id.clone();
    let file:Requirement=serde_json::from_value(serde_json::json!({"requirement_id":"file","title":"Excluded document","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"absent.md","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"exclude","reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap();
    let mut dependent = requirement("dependent");
    dependent.depends_on = vec!["file".into()];
    draft.project_requirements = vec![dependent];
    draft.folders[0].requirements = vec![file];
    let save = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: current.revision,
        expected_content_digest: current.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    store
        .save_revision_locked(&current, &save, "owner")
        .unwrap();
    let env = store.snapshot(&project).unwrap();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "generation".into(),
        slice_ref: None,
    };
    let op = operation(&env, &target, vec![]);
    store
        .run_check(
            context(root.path()),
            env.clone(),
            op,
            || Ok(project.clone()),
        )
        .unwrap();
    let checks = store.load_checks(project.id()).unwrap();
    for id in ["file", "dependent"] {
        let o = checks
            .observations
            .iter()
            .find(|o| o.requirement_id == id)
            .unwrap();
        assert_ne!(
            o.status,
            EnvironmentObservationStatus::Ready,
            "excluded unmeasured Files cannot prove {id}"
        );
        if id == "file" {
            assert_ne!(o.measured_facts.present, Some(true));
        }
    }
}
