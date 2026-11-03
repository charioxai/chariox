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
    assert_eq!(observed.reason_code, "binding_absent");
    assert_eq!(observed.measured_facts.present, Some(false));
    assert_eq!(observed.measured_facts.content_digest, None);
    assert!(baseline.is_empty());
}

#[test]
fn review0440_new_public_source_can_be_explicitly_reviewed() {
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
    let saved = store
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
        .expect("explicit public classification must authorize application configuration");
    assert_eq!(
        saved.0.project_requirements[0].title,
        "Public web configuration"
    );
}

#[test]
fn r6_environment_reviewed_public_source_resolves_at_provider_launch() {
    let root = crate::test_support::TestWorktree::new("r6-public-launch");
    let config = crate::config::DaemonConfig::for_tests();
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    std::fs::write(
        root.path().join("public-settings"),
        "NEXT_PUBLIC_FIREBASE_API_KEY=browser-public-value\n",
    )
    .unwrap();
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
    let launched = project_launch_environment(
        &config,
        project.id(),
        root.path().to_str().unwrap(),
        root.path(),
    )
    .expect("reviewed public configuration must resolve at launch");
    assert!(launched
        .iter()
        .any(|(name, value)| name == "NEXT_PUBLIC_FIREBASE_API_KEY"
            && value == "browser-public-value"));
    let ctx = EnvironmentCheckContext {
        config,
        owner: "owner".into(),
        profiles: crate::account_profile::ProviderAccountProfileRegistry::open(
            root.path().join("profiles"),
        )
        .unwrap(),
        apps: crate::durable_state::DurableKernelStateStore::open(root.path().join("apps.db"))
            .unwrap(),
    };
    let mut baseline = Default::default();
    let observed = check_requirement(
        &ctx,
        &current,
        &current.folders[0].requirements[0],
        &EnvironmentTargetBinding {
            machine_id: "machine".into(),
            target_instance_generation: "boot".into(),
            slice_ref: None,
        },
        "public-launch",
        &mut baseline,
        std::time::Instant::now() + std::time::Duration::from_secs(5),
    );
    assert_eq!(observed.status, EnvironmentObservationStatus::Ready);
}
