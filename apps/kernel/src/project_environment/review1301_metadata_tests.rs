//! MP-08 / MP-10 / MP-11: retained Services references follow the shared contract.
use super::*;
#[test]
fn review1301_service_references_validate_before_preview_and_save() {
    let root = crate::test_support::TestWorktree::new("review1301-services");
    let project = crate::session::RuntimeProject::new(
        "service",
        "owner",
        root.path().to_str().unwrap(),
        "Service",
        crate::session::RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let current = store.snapshot(&project).unwrap();
    for (dependency, slice, valid) in [
        ("base", None, true),
        ("api_key=synthetic-value", None, false),
        ("base", Some("api_key=synthetic-value"), false),
        ("missing", None, false),
        ("service", None, false),
        ("base", Some("slice:worker"), true),
    ] {
        let mut draft = environment_draft(&current);
        let requirement = |id: &str, spec: serde_json::Value| {
            serde_json::from_value(serde_json::json!({"requirement_id":id,"title":id,"scope":{"kind":"project"},"origins":[],"spec":spec,"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap()
        };
        draft.project_requirements = vec![
            requirement(
                "base",
                serde_json::json!({"kind":"software","identity":"node","version_constraint":null,"platform":null,"install_scope":"system","install_source":null,"detect_only":true}),
            ),
            requirement(
                "service",
                serde_json::json!({"kind":"services","identity":"public-service","depends_on":[dependency],"probe":null,"health_expectation":null,"target_binding":{"machine_id":"machine","target_instance_generation":"boot","slice_ref":slice}}),
            ),
        ];
        assert_eq!(
            environment_revision_diff(&current, &draft).is_ok(),
            valid,
            "Preview must validate retained service references"
        );
        let request = crate::local::SaveProjectEnvironmentRevisionRequest {
            project_id: project.id().into(),
            expected_revision: current.revision,
            expected_content_digest: current.content_digest.clone(),
            draft,
            accepted_proposal_ids: vec![],
            excluded_proposal_ids: vec![],
        };
        // Validate the same draft without changing the concurrency baseline for the next case.
        if !valid {
            assert!(store
                .save_revision_locked(&current, &request, "owner")
                .is_err());
            assert_eq!(store.snapshot(&project).unwrap().revision, current.revision);
        }
    }
}

#[test]
fn review1301_valid_service_references_persist_without_execution() {
    let root = crate::test_support::TestWorktree::new("review1301-valid-service");
    let project = crate::session::RuntimeProject::new(
        "valid-service",
        "owner",
        root.path().to_str().unwrap(),
        "Service",
        crate::session::RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let current = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&current);
    let requirement = |id: &str, spec: serde_json::Value| {
        serde_json::from_value(serde_json::json!({"requirement_id":id,"title":id,"scope":{"kind":"project"},"origins":[],"spec":spec,"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap()
    };
    draft.project_requirements = vec![
        requirement(
            "base",
            serde_json::json!({"kind":"software","identity":"node","version_constraint":null,"platform":null,"install_scope":"system","install_source":null,"detect_only":true}),
        ),
        requirement(
            "service",
            serde_json::json!({"kind":"services","identity":"public-service","depends_on":["base"],"probe":null,"health_expectation":null,"target_binding":{"machine_id":"machine","target_instance_generation":"boot","slice_ref":"slice:worker"}}),
        ),
    ];
    environment_revision_diff(&current, &draft).unwrap();
    let expected = draft.project_requirements[1].spec.clone();
    store
        .save_revision_locked(
            &current,
            &crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: project.id().into(),
                expected_revision: current.revision,
                expected_content_digest: current.content_digest.clone(),
                draft,
                accepted_proposal_ids: vec![],
                excluded_proposal_ids: vec![],
            },
            "owner",
        )
        .unwrap();
    let saved = store.snapshot(&project).unwrap();
    assert_eq!(saved.project_requirements[1].spec, expected);
    assert!(saved.observations.is_empty());
}
