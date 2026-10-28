//! MP-08 / MP-10 / MP-11: opaque credentials cannot become public through aliases.
use super::*;

#[test]
fn envp05a_public_alias_rejects_protected_and_credential_sources() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("envp05-source-alias");
    let config = crate::config::DaemonConfig::for_tests()
        .with_session_history_root(root.path().join("state/sessions"));
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let project = crate::session::RuntimeProject::new(
        "alias",
        "owner",
        root.path().to_string_lossy(),
        "Alias",
        crate::session::RuntimeProjectKind::Named,
    );
    let mut env = store.snapshot(&project).unwrap();
    let ctx = EnvironmentCheckContext {
        config: config.clone(),
        owner: "owner".into(),
        profiles: crate::account_profile::ProviderAccountProfileRegistry::open(
            root.path().join("profiles"),
        )
        .unwrap(),
        apps: crate::durable_state::DurableKernelStateStore::open(root.path().join("apps.db"))
            .unwrap(),
    };
    // No credential signature: the authority of the source name is the fence.
    let opaque = "synthetic-opaque-input";
    assert!(!super::detect_index::credential_content(opaque));
    let mut requirement: Requirement = serde_json::from_value(serde_json::json!({
        "requirement_id":"public", "title":"Public prerequisite", "scope":{"kind":"project"},
        "origins":[], "spec":{"kind":"variables","name":"PUBLIC_BUILD_INPUT","locator":{"kind":"workspace_environment","name":"CHARIOX_RELAY_TOKEN"}},
        "depends_on":[], "platform_variants":[], "required":true,"legacy_entry":null
    })).unwrap();
    for source in [
        "CHARIOX_RELAY_TOKEN",
        "CODEX_HOME",
        "ENV",
        "GITHUB_TOKEN",
        "INHERITED_PASSWORD",
    ] {
        std::env::set_var(source, opaque);
        std::fs::write(
            root.path().join("source-input"),
            format!("{source}={opaque}\n"),
        )
        .unwrap();
        for locator in [
            ProjectEnvironmentLocator::WorkspaceEnvironment {
                name: source.into(),
            },
            ProjectEnvironmentLocator::EnvFile {
                path: "source-input".into(),
                key: source.into(),
            },
        ] {
            requirement.spec = RequirementSpec::Variables {
                name: "PUBLIC_BUILD_INPUT".into(),
                locator: locator.clone(),
            };
            env.project_requirements = vec![requirement.clone()];
            let mut baseline = Default::default();
            let observed = check_requirement(
                &ctx,
                &env,
                &requirement,
                &EnvironmentTargetBinding {
                    machine_id: config.host_machine_id.clone(),
                    target_instance_generation: "generation".into(),
                    slice_ref: None,
                },
                "alias-check",
                &mut baseline,
                std::time::Instant::now() + std::time::Duration::from_secs(2),
            );
            assert_eq!(
                observed.status,
                EnvironmentObservationStatus::NeedsYourInput,
                "Check must reject the source independently of its public alias"
            );
            assert!(
                baseline.is_empty(),
                "no protected value digest may be retained"
            );
            assert!(
                apply_scoped_binding(&store, &config, &env, &requirement).is_err(),
                "Apply must not publish a protected source"
            );

            // A manifest saved by an older kernel cannot bypass launch protection.
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
                            name: "PUBLIC_BUILD_INPUT".into(),
                            workspace_id: root.path().to_string_lossy().into(),
                            kind: ProjectEnvironmentEntryKind::Variable,
                            classification: ProjectEnvironmentClassification::NonSecret,
                            excluded: false,
                            uses: vec![],
                            locator,
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
            let error = project_launch_environment(
                &config,
                project.id(),
                &root.path().to_string_lossy(),
                root.path(),
            )
            .unwrap_err();
            assert!(!error.to_string().contains(opaque));
            assert!(project_public_setup_environment(
                &config,
                project.id(),
                &env.folders[0],
                root.path(),
                &[&requirement]
            )
            .is_err());
        }
        std::env::remove_var(source);
    }
    requirement.spec = RequirementSpec::Variables {
        name: "PUBLIC_BUILD_INPUT".into(),
        locator: ProjectEnvironmentLocator::WorkspaceEnvironment {
            name: "PUBLIC_SOURCE_INPUT".into(),
        },
    };
    std::env::set_var("PUBLIC_SOURCE_INPUT", "public-input");
    env.project_requirements = vec![requirement.clone()];
    let mut stored = store.load(project.id()).unwrap().unwrap();
    stored.manifest.entries.clear();
    store.save(&stored).unwrap();
    apply_scoped_binding(&store, &config, &env, &requirement).unwrap();
    assert!(project_public_setup_environment(
        &config,
        project.id(),
        &env.folders[0],
        root.path(),
        &[&requirement]
    )
    .unwrap()
    .iter()
    .any(|(name, value)| name == "PUBLIC_BUILD_INPUT" && value == "public-input"));
}
