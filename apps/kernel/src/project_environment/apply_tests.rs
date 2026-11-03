//! MP-08 / MP-10 / MP-11: focused recovery and conflict regressions; not live acceptance.
use super::*;
fn fixture() -> (
    crate::test_support::TestWorktree,
    ProjectEnvironmentStore,
    ProjectEnvironment,
) {
    let root = crate::test_support::TestWorktree::new("envp05-apply");
    let project = crate::session::RuntimeProject::new(
        "apply-project",
        "owner",
        root.path().to_string_lossy(),
        "Apply",
        crate::session::RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let env = store.snapshot(&project).unwrap();
    (root, store, env)
}
fn requirement(id: &str) -> Requirement {
    Requirement {
        requirement_id: id.into(),
        title: id.into(),
        scope: RequirementScope::Project,
        origins: vec![],
        spec: RequirementSpec::Software {
            identity: "node".into(),
            version_constraint: None,
            platform: None,
            install_scope: EnvironmentInstallScope::System,
            install_source: None,
            detect_only: true,
        },
        depends_on: vec![],
        platform_variants: vec![],
        required: true,
        legacy_entry: None,
    }
}
fn target(generation: &str) -> EnvironmentTargetBinding {
    EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: generation.into(),
        slice_ref: None,
    }
}
fn observed(
    env: &ProjectEnvironment,
    id: &str,
    status: EnvironmentObservationStatus,
) -> MachineObservation {
    MachineObservation {
        environment_id: env.lineage.environment_id.clone(),
        requirement_id: id.into(),
        target: target("one"),
        revision_digest: env.content_digest.clone(),
        status,
        checked_at_ms: Some(crate::session::unix_epoch_ms()),
        measured_facts: empty_environment_facts(),
        reason_code: "measured".into(),
        safe_summary: "Measured target fact".into(),
        check_operation_id: None,
        stale: false,
    }
}
#[test]
fn envp05a_conflict_blocks_dependents_but_not_unrelated_scoped_work() {
    let (_root, _store, mut env) = fixture();
    let node = requirement("global-node");
    let mut dependent = requirement("build");
    dependent.depends_on = vec!["global-node".into()];
    let mut public = requirement("public");
    public.spec = RequirementSpec::Variables {
        name: "PUBLIC_SITE_URL".into(),
        locator: ProjectEnvironmentLocator::WorkspaceEnvironment {
            name: "PUBLIC_SITE_URL".into(),
        },
    };
    env.project_requirements = vec![dependent, public, node];
    let observations = vec![
        observed(&env, "global-node", EnvironmentObservationStatus::Conflict),
        observed(&env, "public", EnvironmentObservationStatus::Ready),
    ];
    let plan = build_apply_plan(&env, target("one"), &[], &observations).unwrap();
    assert_eq!(plan.selected_items, ["global-node", "build", "public"]);
    assert_eq!(
        plan.conflicts
            .iter()
            .map(|r| r.requirement_id.as_str())
            .collect::<Vec<_>>(),
        ["global-node", "build"]
    );
    assert!(plan.required_opt_ins.is_empty());
}
#[test]
fn envp05a_selection_closes_optional_service_dependencies_and_rejects_cycle() {
    let (_root, _store, mut env) = fixture();
    let mut runtime = requirement("runtime");
    runtime.required = false;
    let mut service = requirement("service");
    service.spec = RequirementSpec::Services {
        identity: "preview".into(),
        target_binding: None,
        probe: None,
        health_expectation: None,
        depends_on: vec!["runtime".into()],
    };
    env.project_requirements = vec![service, runtime];
    assert_eq!(apply_selection(&env, &[]).unwrap(), ["runtime", "service"]);
    env.project_requirements[1].depends_on = vec!["service".into()];
    assert!(apply_selection(&env, &[]).is_err());
    assert!(apply_selection(&env, &["foreign".into()]).is_err());
}
#[test]
fn envp05a_plan_never_trusts_stale_or_foreign_target_readiness() {
    let (_root, _store, mut env) = fixture();
    let mut dependent = requirement("dependent");
    dependent.depends_on = vec!["tool".into()];
    env.project_requirements = vec![requirement("tool"), dependent];
    for foreign in [false, true] {
        let mut observation = observed(&env, "tool", EnvironmentObservationStatus::Ready);
        if foreign {
            observation.target = target("other");
        } else {
            observation.stale = true;
        }
        let plan = build_apply_plan(&env, target("one"), &[], &[observation]).unwrap();
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].requirement_id, "dependent");
    }
}
#[test]
fn envp05a_restart_retains_receipt_and_uncertain_item_without_replay() {
    let (_root, store, env) = fixture();
    let receipt = EnvironmentReceipt {
        receipt_id: "committed".into(),
        requirement_id: Some("public".into()),
        target: target("one"),
        revision_digest: env.content_digest.clone(),
        artifact_digest: "metadata-only".into(),
        recorded_at_ms: 100,
    };
    let operation = EnvironmentOperation {
        operation_id: "apply".into(),
        attempt: 1,
        local_project_id: env.local_project_id.clone(),
        revision_digest: env.content_digest.clone(),
        target: target("one"),
        kind: EnvironmentOperationKind::Apply,
        phase: EnvironmentOperationPhase::Validating,
        selected_items: vec!["public".into(), "build".into()],
        per_item_opt_ins: vec![],
        per_item_results: vec![],
        created_at_ms: 90,
        updated_at_ms: 100,
        cancellation: None,
        receipts: vec![receipt.clone()],
        recovery_state: EnvironmentRecoveryState::Pending,
    };
    let request = crate::local::ApplyProjectEnvironmentRequest {
        project_id: env.local_project_id.clone(),
        operation_id: operation.operation_id.clone(),
        plan_id: "plan".into(),
        expected_revision: env.revision,
        revision_digest: env.content_digest.clone(),
        target: target("one"),
        selected_items: operation.selected_items.clone(),
        per_item_opt_ins: vec![],
    };
    let a = ApplyAttempt {
        binding_scopes: vec![],
        plan_id: "plan".into(),
        request_digest: Some(metadata_digest(&request)),
        operation,
        folder_binding_digest: folder_bindings_digest(&env),
        active_requirement: Some("build".into()),
        uncertain_requirements: vec![],
        setup_admitted_commands: Default::default(),
    };
    let _lock = store.lock(&env.local_project_id).unwrap();
    store
        .save_applies(
            &env.local_project_id,
            &EnvironmentApplies {
                plans: vec![],
                attempts: vec![a],
            },
        )
        .unwrap();
    let recovered = store
        .recover_applies(&env.local_project_id, &target("two"))
        .unwrap();
    let a = &recovered.attempts[0];
    assert_eq!(a.operation.phase, EnvironmentOperationPhase::Failed);
    assert_eq!(a.operation.receipts, vec![receipt]);
    assert_eq!(a.uncertain_requirements, ["build"]);
    assert!(a.active_requirement.is_none());
    assert!(matches!(
        a.operation.recovery_state,
        EnvironmentRecoveryState::Resumable { .. }
    ));
    let again = store
        .recover_applies(&env.local_project_id, &target("two"))
        .unwrap();
    assert_eq!(again.attempts[0].operation, a.operation);
    let mut journal = again.clone();
    let old = journal.attempts[0].operation.clone();
    let mut retry = journal.attempts[0].clone();
    retry.operation.attempt += 1;
    retry.operation.target = target("two");
    let current = retry.operation.clone();
    journal.attempts.push(retry);
    assert!(
        latest_apply_attempt(&mut journal, &old).is_none(),
        "late old completion cannot select a historical attempt"
    );
    assert!(latest_apply_attempt(&mut journal, &current).is_some());
    assert_eq!(
        replayed_apply_operation(&journal, &request).unwrap(),
        Some(current.clone()),
        "original lost ACK returns the latest retry despite a new target generation"
    );
    let mut changed_request = request.clone();
    changed_request.target = target("two");
    assert!(replayed_apply_operation(&journal, &changed_request).is_err());
    for i in 0..64 {
        let mut other = journal.attempts[0].clone();
        other.operation.operation_id = format!("other-{i}");
        journal.attempts.push(other);
    }
    store.save_applies(&env.local_project_id, &journal).unwrap();
    let durable = store.load_applies(&env.local_project_id).unwrap();
    assert_eq!(
        replayed_apply_operation(&durable, &request).unwrap(),
        Some(current),
        "later operations never retire the identity required by an old ACK"
    );
    store.remove(&env.local_project_id).unwrap();
    assert!(store
        .load_applies(&env.local_project_id)
        .unwrap()
        .attempts
        .is_empty());
}

#[test]
fn envp05a_reviewed_manual_binding_needs_no_invented_source_use() {
    let (_root, _store, env) = fixture();
    let evidence = ProjectEnvironmentEvidence::default();
    let manifest = ProjectEnvironmentManifest {
        schema_version: 1,
        project_id: env.local_project_id,
        evidence_digest: evidence.digest(),
        entries: vec![ProjectEnvironmentEntry {
            name: "PROJECT_LICENSE_TEXT".into(),
            workspace_id: "reviewed-folder".into(),
            kind: ProjectEnvironmentEntryKind::Variable,
            classification: ProjectEnvironmentClassification::NonSecret,
            excluded: false,
            uses: vec![],
            locator: ProjectEnvironmentLocator::ConfigFile {
                path: "LICENSE".into(),
            },
            status: ProjectEnvironmentEntryStatus::Found,
        }],
        private_files: vec![],
        toolchain_hints: vec![],
        package_hints: vec![],
        service_hints: vec![],
    };
    assert!(
        manifest.validate().is_ok(),
        "reviewed manual locators have no detected use sites"
    );
    // MP-11: the real public-only provider drill previously stalled on a Vault
    // passphrase interaction. Preserve every resolver read/write authorization.
    assert!(!project_environment_requires_vault(&manifest));
    let mut protected = manifest.clone();
    protected.entries[0].classification = ProjectEnvironmentClassification::Secret;
    assert!(project_environment_requires_vault(&protected));
    protected.entries[0].classification = ProjectEnvironmentClassification::NonSecret;
    protected.entries[0].locator = ProjectEnvironmentLocator::Vault {
        service: "authorized-service".into(),
        key: "authorized-name".into(),
    };
    assert!(project_environment_requires_vault(&protected));
    protected.entries[0].locator = ProjectEnvironmentLocator::Missing;
    assert!(project_environment_requires_vault(&protected));
    protected.entries[0].excluded = true;
    assert!(!project_environment_requires_vault(&protected));
    let input = ProjectEnvironmentDiscoveryInput {
        project_id: manifest.project_id.clone(),
        evidence_digest: manifest.evidence_digest.clone(),
        changed_paths: Default::default(),
        previous_manifest: None,
        references: vec![],
        private_files: vec![],
        revision: None,
    };
    assert!(
        parse_project_environment_discovery_output(
            &serde_json::to_string(&manifest).unwrap(),
            &input
        )
        .is_err(),
        "no-use metadata cannot bypass discovery reference admission"
    );
}

#[test]
fn envp05a_inventory_ready_does_not_prove_interrupted_setup_completed() {
    assert!(!setup_reconciliation_allowed(false, true, false, true));
    assert!(!setup_reconciliation_allowed(false, true, true, false));
    assert!(setup_reconciliation_allowed(false, true, true, true));
    assert!(setup_reconciliation_allowed(true, false, false, true));
    assert!(!setup_reconciliation_allowed(false, false, true, true));
}

#[test]
fn envp05a_removed_apply_binding_is_absent_from_next_provider_launch() {
    removed_binding_after_status_refresh(ProjectEnvironmentEntryStatus::Found);
}

// MP-08 / MP-10 / MP-11: refresh changes measured status, not binding authority.
#[test]
fn envp05a_status_refresh_preserves_retirement_ownership() {
    for status in [
        ProjectEnvironmentEntryStatus::Missing,
        ProjectEnvironmentEntryStatus::Problem,
    ] {
        removed_binding_after_status_refresh(status);
    }
}

fn removed_binding_after_status_refresh(status: ProjectEnvironmentEntryStatus) {
    let root = crate::test_support::TestWorktree::new("envp05-retirement");
    let config = crate::config::DaemonConfig::for_tests();
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let project = crate::session::RuntimeProject::new(
        "retirement",
        "owner",
        root.path().to_string_lossy(),
        "Retirement",
        crate::session::RuntimeProjectKind::Named,
    );
    let mut env = store.snapshot(&project).unwrap();
    std::fs::write(root.path().join("input.txt"), "public input").unwrap();
    let mut removed = requirement("removed");
    removed.spec = RequirementSpec::Variables {
        name: "RETIRED_PUBLIC_INPUT".into(),
        locator: ProjectEnvironmentLocator::ConfigFile {
            path: "input.txt".into(),
        },
    };
    let mut remaining = removed.clone();
    remaining.requirement_id = "remaining".into();
    if let RequirementSpec::Variables { name, .. } = &mut remaining.spec {
        *name = "REMAINING_PUBLIC_INPUT".into();
    }
    env.project_requirements = vec![removed.clone(), remaining.clone()];
    apply_scoped_binding(&store, &config, &env, &removed).unwrap();
    apply_scoped_binding(&store, &config, &env, &remaining).unwrap();
    let mut refreshed = store.load(project.id()).unwrap().unwrap();
    refreshed
        .manifest
        .entries
        .iter_mut()
        .find(|entry| entry.name == "RETIRED_PUBLIC_INPUT")
        .unwrap()
        .status = status;
    store.save(&refreshed).unwrap();
    env.project_requirements.remove(0);
    apply_scoped_binding(&store, &config, &env, &remaining).unwrap();
    let launch = project_launch_environment(
        &config,
        project.id(),
        &root.path().to_string_lossy(),
        root.path(),
    )
    .unwrap();
    assert!(launch
        .iter()
        .any(|(name, _)| name == "REMAINING_PUBLIC_INPUT"));
    assert!(
        !launch
            .iter()
            .any(|(name, _)| name == "RETIRED_PUBLIC_INPUT"),
        "removed Apply input must not enter the provider launch"
    );
}

// MP-08 / MP-10 / MP-11: selective retirement preserves unrelated launch authority.
#[test]
fn envp05a_retirement_preserves_legacy_unselected_and_independently_edited_entries() {
    let (_root, store, env) = fixture();
    let evidence = ProjectEnvironmentEvidence::default();
    let entry = |name: &str| ProjectEnvironmentEntry {
        name: name.into(),
        workspace_id: env.folders[0].local_workspace_binding.clone(),
        kind: ProjectEnvironmentEntryKind::Variable,
        classification: ProjectEnvironmentClassification::Secret,
        excluded: false,
        uses: vec![],
        locator: ProjectEnvironmentLocator::Vault {
            service: "authorized-service".into(),
            key: name.into(),
        },
        status: ProjectEnvironmentEntryStatus::Found,
    };
    let retired = entry("RETIRED_SECRET");
    let unselected = entry("UNSELECTED_SECRET");
    let original = entry("INDEPENDENTLY_EDITED");
    let mut edited = original.clone();
    edited.locator = ProjectEnvironmentLocator::Vault {
        service: "other-authorized-service".into(),
        key: "reviewed".into(),
    };
    store
        .save(&StoredProjectEnvironment {
            source: None,
            evidence: evidence.clone(),
            reported_missing: Default::default(),
            reviewed_manifest: None,
            last_review: None,
            manifest: ProjectEnvironmentManifest {
                schema_version: 1,
                project_id: env.local_project_id.clone(),
                evidence_digest: evidence.digest(),
                entries: vec![
                    retired.clone(),
                    unselected.clone(),
                    edited.clone(),
                    entry("LEGACY_SECRET"),
                ],
                private_files: vec![],
                toolchain_hints: vec![],
                package_hints: vec![],
                service_hints: vec![],
            },
            apply_owned_bindings: vec![
                super::store::ApplyOwnedBinding {
                    requirement_id: "removed".into(),
                    scope: RequirementScope::Project,
                    entry: retired,
                },
                super::store::ApplyOwnedBinding {
                    requirement_id: "other-folder".into(),
                    scope: RequirementScope::Folder {
                        folder_id: "unselected-folder".into(),
                    },
                    entry: unselected,
                },
                super::store::ApplyOwnedBinding {
                    requirement_id: "edited".into(),
                    scope: RequirementScope::Project,
                    entry: original,
                },
            ],
        })
        .unwrap();
    retire_removed_apply_bindings(&store, &env, &[RequirementScope::Project]).unwrap();
    let stored = store.load(&env.local_project_id).unwrap().unwrap();
    assert_eq!(
        stored
            .manifest
            .entries
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["UNSELECTED_SECRET", "INDEPENDENTLY_EDITED", "LEGACY_SECRET"]
    );
    assert_eq!(stored.manifest.entries[1], edited);
    assert_eq!(stored.apply_owned_bindings.len(), 1);
}

// MP-08 / MP-10 / MP-11: a new reviewed locator replaces only its own prior Apply.
#[test]
fn envp05a_reviewed_apply_locator_updates_public_and_named_vault_sources() {
    reviewed_locators_after_status_refresh(ProjectEnvironmentEntryStatus::Found);
}

#[test]
fn envp05a_status_refresh_preserves_reviewed_locator_ownership() {
    for status in [
        ProjectEnvironmentEntryStatus::Missing,
        ProjectEnvironmentEntryStatus::Problem,
    ] {
        reviewed_locators_after_status_refresh(status);
    }
}

fn unlocked_fixture_vault_config(root: &std::path::Path) -> crate::config::DaemonConfig {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path =
        root.join("fixture-vault.json").to_string_lossy().into();
    crate::secret::unlock_chariox_encrypted_vault(
        &config.user_config.credential_vault.path,
        "synthetic-fixture-passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    config
}

fn reviewed_locators_after_status_refresh(status: ProjectEnvironmentEntryStatus) {
    let root = crate::test_support::TestWorktree::new("envp05-locator");
    let config = unlocked_fixture_vault_config(root.path());
    let vault = crate::secret::project_environment_vault(&config).unwrap();
    vault
        .set_secret("fixture-service", "first", "synthetic-first")
        .unwrap();
    vault
        .set_secret("fixture-service", "second", "synthetic-second")
        .unwrap();
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let project = crate::session::RuntimeProject::new(
        "locator",
        "owner",
        root.path().to_string_lossy(),
        "Locator",
        crate::session::RuntimeProjectKind::Named,
    );
    let mut env = store.snapshot(&project).unwrap();
    std::fs::write(root.path().join("first-input"), "PUBLIC_SITE=first-site\n").unwrap();
    std::fs::write(
        root.path().join("second-input"),
        "SECOND_SITE=second-site\n",
    )
    .unwrap();
    let mut public = requirement("public");
    public.spec = RequirementSpec::Variables {
        name: "PUBLIC_SITE".into(),
        locator: ProjectEnvironmentLocator::EnvFile {
            path: "first-input".into(),
            key: "PUBLIC_SITE".into(),
        },
    };
    let mut secret = requirement("secret");
    secret.spec = RequirementSpec::Secrets {
        name: "PROJECT_PRIVATE_INPUT".into(),
        vault: Some(EnvironmentVaultReference {
            service: "fixture-service".into(),
            key: "first".into(),
        }),
    };
    env.project_requirements = vec![public.clone(), secret.clone()];
    apply_scoped_binding(&store, &config, &env, &public).unwrap();
    apply_scoped_binding(&store, &config, &env, &secret).unwrap();
    let mut refreshed = store.load(project.id()).unwrap().unwrap();
    for entry in &mut refreshed.manifest.entries {
        entry.status = status;
    }
    store.save(&refreshed).unwrap();
    public.spec = RequirementSpec::Variables {
        name: "PUBLIC_SITE".into(),
        locator: ProjectEnvironmentLocator::EnvFile {
            path: "second-input".into(),
            key: "SECOND_SITE".into(),
        },
    };
    secret.spec = RequirementSpec::Secrets {
        name: "PROJECT_PRIVATE_INPUT".into(),
        vault: Some(EnvironmentVaultReference {
            service: "fixture-service".into(),
            key: "second".into(),
        }),
    };
    env.project_requirements = vec![public.clone(), secret.clone()];
    assert!(
        apply_scoped_binding(&store, &config, &env, &public).is_ok(),
        "a reviewed public locator update must replace its prior Apply binding"
    );
    assert!(
        apply_scoped_binding(&store, &config, &env, &secret).is_ok(),
        "a reviewed named Vault reference update must replace its prior Apply binding"
    );
    let launch = project_launch_environment(
        &config,
        project.id(),
        &root.path().to_string_lossy(),
        root.path(),
    )
    .unwrap();
    assert!(launch
        .iter()
        .any(|(name, value)| name == "PUBLIC_SITE" && value == "second-site"));
    assert!(launch
        .iter()
        .any(|(name, value)| name == "PROJECT_PRIVATE_INPUT" && value == "synthetic-second"));
    let mut legacy = store.load(project.id()).unwrap().unwrap();
    legacy.apply_owned_bindings.clear();
    store.save(&legacy).unwrap();
    public.spec = RequirementSpec::Variables {
        name: "PUBLIC_SITE".into(),
        locator: ProjectEnvironmentLocator::EnvFile {
            path: "first-input".into(),
            key: "PUBLIC_SITE".into(),
        },
    };
    assert!(
        apply_scoped_binding(&store, &config, &env, &public).is_err(),
        "unrelated legacy conflicts remain protected"
    );
    crate::secret::lock_chariox_encrypted_vault(&config.user_config.credential_vault.path).unwrap();
}

// MP-08 / MP-10 / MP-11: GitHub tokens are ordinary Project credentials when the Vault holds them.
#[test]
fn envp05a_github_vault_binding_applies_and_reaches_provider_launch() {
    let root = crate::test_support::TestWorktree::new("envp05-github-vault");
    let config = unlocked_fixture_vault_config(root.path());
    crate::secret::project_environment_vault(&config)
        .unwrap()
        .set_secret("fixture-service", "github", "synthetic-github-value")
        .unwrap();
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let project = crate::session::RuntimeProject::new(
        "github",
        "owner",
        root.path().to_string_lossy(),
        "GitHub",
        crate::session::RuntimeProjectKind::Named,
    );
    let mut env = store.snapshot(&project).unwrap();
    let mut secret = requirement("github");
    secret.spec = RequirementSpec::Secrets {
        name: "GH_TOKEN".into(),
        vault: Some(EnvironmentVaultReference {
            service: "fixture-service".into(),
            key: "github".into(),
        }),
    };
    env.project_requirements = vec![secret.clone()];
    apply_scoped_binding(&store, &config, &env, &secret)
        .expect("an authorized GitHub Vault binding must apply");
    let launch = project_launch_environment(
        &config,
        project.id(),
        &root.path().to_string_lossy(),
        root.path(),
    )
    .expect("a resolved GitHub Vault entry must reach provider launch");
    assert!(launch
        .iter()
        .any(|(name, value)| name == "GH_TOKEN" && value == "synthetic-github-value"));
    let mut public = requirement("public-github");
    public.spec = RequirementSpec::Variables {
        name: "GITHUB_TOKEN".into(),
        locator: ProjectEnvironmentLocator::EnvFile {
            path: "public-input".into(),
            key: "PUBLIC_INPUT".into(),
        },
    };
    std::fs::write(
        root.path().join("public-input"),
        "PUBLIC_INPUT=public-value\n",
    )
    .unwrap();
    env.project_requirements = vec![public.clone()];
    let refused = apply_scoped_binding(&store, &config, &env, &public).unwrap_err();
    assert!(
        refused.to_string().contains("Protected launch binding"),
        "a public binding cannot carry a GitHub token: {refused}"
    );
    crate::secret::lock_chariox_encrypted_vault(&config.user_config.credential_vault.path).unwrap();
}

// MP-08 / MP-10 / MP-11: Application credentials that share a provider's variable names are
// ordinary Vault inputs; only public (non-secret) destinations refuse them.
#[test]
fn envr10_application_credentials_apply_and_reach_provider_launch() {
    let root = crate::test_support::TestWorktree::new("envr10-aws-vault");
    let config = unlocked_fixture_vault_config(root.path());
    let vault = crate::secret::project_environment_vault(&config).unwrap();
    let names = [
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
    ];
    for name in names {
        vault
            .set_secret("fixture-service", name, &format!("synthetic-{name}"))
            .unwrap();
    }
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let project = crate::session::RuntimeProject::new(
        "aws",
        "owner",
        root.path().to_string_lossy(),
        "Application",
        crate::session::RuntimeProjectKind::Named,
    );
    let mut env = store.snapshot(&project).unwrap();
    let requirements: Vec<_> = names
        .iter()
        .map(|name| {
            let mut secret = requirement(name);
            secret.spec = RequirementSpec::Secrets {
                name: (*name).into(),
                vault: Some(EnvironmentVaultReference {
                    service: "fixture-service".into(),
                    key: (*name).into(),
                }),
            };
            secret
        })
        .collect();
    env.project_requirements = requirements.clone();
    for secret in &requirements {
        apply_scoped_binding(&store, &config, &env, secret)
            .expect("an authorized application credential must apply");
    }
    let launch = project_launch_environment(
        &config,
        project.id(),
        &root.path().to_string_lossy(),
        root.path(),
    )
    .expect("an existing manifest with application credentials must reach provider launch");
    for name in names {
        assert!(
            launch
                .iter()
                .any(|(bound, value)| bound == name && value == format!("synthetic-{name}")),
            "{name} must reach the provider launch"
        );
    }
    for name in ["AWS_SECRET_ACCESS_KEY", "OPENAI_API_KEY"] {
        let mut public = requirement("public-credential");
        public.spec = RequirementSpec::Variables {
            name: name.into(),
            locator: ProjectEnvironmentLocator::EnvFile {
                path: "public-input".into(),
                key: "PUBLIC_INPUT".into(),
            },
        };
        std::fs::write(
            root.path().join("public-input"),
            "PUBLIC_INPUT=public-value\n",
        )
        .unwrap();
        env.project_requirements = vec![public.clone()];
        let refused = apply_scoped_binding(&store, &config, &env, &public).unwrap_err();
        assert!(
            refused.to_string().contains("Protected launch binding"),
            "a public binding cannot carry {name}: {refused}"
        );
    }
    crate::secret::lock_chariox_encrypted_vault(&config.user_config.credential_vault.path).unwrap();
}
