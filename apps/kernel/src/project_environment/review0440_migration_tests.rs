//! MP-08 / MP-10 / MP-11: old public classifications cannot lock unrelated edits.
use super::*;
#[test]
fn review0440_legacy_public_secret_like_source_allows_unrelated_save() {
    let (root, store, _) = environment();
    let project = crate::session::RuntimeProject::new(
        "legacy",
        "owner",
        root.path().to_str().unwrap(),
        "Legacy",
        crate::session::RuntimeProjectKind::Named,
    );
    let evidence = ProjectEnvironmentEvidence::default();
    store
        .save(&StoredProjectEnvironment {
            apply_owned_bindings: Default::default(),
            source: None,
            manifest: ProjectEnvironmentManifest {
                schema_version: 1,
                project_id: project.id().into(),
                evidence_digest: evidence.digest(),
                entries: vec![ProjectEnvironmentEntry {
                    name: "NEXT_PUBLIC_FIREBASE_API_KEY".into(),
                    workspace_id: project.workspace_id().into(),
                    kind: ProjectEnvironmentEntryKind::Variable,
                    classification: ProjectEnvironmentClassification::NonSecret,
                    excluded: false,
                    uses: vec![],
                    locator: ProjectEnvironmentLocator::EnvFile {
                        path: "public-settings".into(),
                        key: "NEXT_PUBLIC_FIREBASE_API_KEY".into(),
                    },
                    status: ProjectEnvironmentEntryStatus::Found,
                }],
                private_files: vec![],
                toolchain_hints: vec![],
                package_hints: vec![],
                service_hints: vec![],
            },
            evidence,
            reported_missing: Default::default(),
            reviewed_manifest: None,
            last_review: None,
        })
        .unwrap();
    let env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    draft.folders[0].requirements[0].title = "Public web configuration".into();
    store
        .save_revision_locked(
            &env,
            &crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                expected_content_digest: env.content_digest.clone(),
                draft,
                accepted_proposal_ids: vec![],
                excluded_proposal_ids: vec![],
            },
            "owner",
        )
        .expect("an unrelated edit must remain possible after migration");
    let current = store.snapshot(&project).unwrap();
    let mut baseline = Default::default();
    let observed = check_requirement(
        &super::review0034::context(root.path()),
        &current,
        &current.folders[0].requirements[0],
        &EnvironmentTargetBinding {
            machine_id: "machine".into(),
            target_instance_generation: "boot".into(),
            slice_ref: None,
        },
        "legacy",
        &mut baseline,
        std::time::Instant::now() + std::time::Duration::from_secs(5),
    );
    assert_eq!(observed.reason_code, "vault_required");
    assert_eq!(observed.measured_facts.present, None);
    assert_eq!(observed.measured_facts.content_digest, None);
    assert!(baseline.is_empty());
}

#[test]
fn review0440_new_secret_like_source_requires_vault_and_names_requirement() {
    let (_root, store, env) = environment();
    let mut draft = environment_draft(&env);
    let mut r = requirement("new-config");
    r.title = "Public web configuration".into();
    r.spec = RequirementSpec::Variables {
        name: "PUBLIC_CONFIG".into(),
        locator: ProjectEnvironmentLocator::WorkspaceEnvironment {
            name: "NEXT_PUBLIC_FIREBASE_API_KEY".into(),
        },
    };
    draft.project_requirements = vec![r];
    let error = store
        .save_revision_locked(
            &env,
            &crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: env.local_project_id.clone(),
                expected_revision: env.revision,
                expected_content_digest: env.content_digest.clone(),
                draft,
                accepted_proposal_ids: vec![],
                excluded_proposal_ids: vec![],
            },
            "owner",
        )
        .unwrap_err();
    assert!(error.to_string().contains("Public web configuration"));
}
