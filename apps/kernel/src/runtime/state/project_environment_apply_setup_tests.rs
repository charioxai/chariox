//! MP-08 / MP-10 / MP-11: supplemental runner regression with a synthetic worker receipt.
//! Real shell commands execute through the production runner; this is not VM acceptance.
use super::*;

fn status(
    phase: ProjectEnvironmentSetupPhase,
    code: Option<&str>,
    retryable: bool,
) -> ProjectEnvironmentSetupStatus {
    ProjectEnvironmentSetupStatus {
        operation_id: "command".into(),
        project_id: "project".into(),
        session_id: String::new(),
        agent_id: String::new(),
        worker_id: "worker".into(),
        platform: actual_worker_platform(),
        phase,
        attempt: 1,
        progress_percent: 0,
        definition_digest: None,
        validation: None,
        message: None,
        failure_code: code.map(str::to_string),
        failure_message: None,
        retryable,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}
#[test]
fn envp05a_setup_progress_proves_only_the_unstarted_suffix() {
    let prefix = vec![
        Some(status(ProjectEnvironmentSetupPhase::Ready, None, false)),
        Some(status(
            ProjectEnvironmentSetupPhase::Failed,
            Some("apply_command_unconfirmed"),
            true,
        )),
        None,
    ];
    assert!(setup_retry_progress_is_settled(Some(2), &prefix));
    assert!(
        !setup_retry_progress_is_settled(None, &prefix),
        "legacy or lost admission progress cannot authorize replay"
    );
    assert!(
        !setup_retry_progress_is_settled(Some(3), &prefix),
        "missing admitted state is uncertain"
    );
    assert!(
        !setup_retry_progress_is_settled(Some(4), &prefix),
        "corrupt progress fails closed"
    );
    let mut unknown = prefix.clone();
    unknown[1] = Some(status(
        ProjectEnvironmentSetupPhase::Failed,
        Some("validation_cleanup_incomplete"),
        false,
    ));
    assert!(!setup_retry_progress_is_settled(Some(2), &unknown));
    unknown[1] = None;
    assert!(!setup_retry_progress_is_settled(Some(2), &unknown));
    unknown = prefix;
    unknown[2] = Some(status(ProjectEnvironmentSetupPhase::Ready, None, false));
    assert!(
        !setup_retry_progress_is_settled(Some(2), &unknown),
        "state beyond the admission frontier is inconsistent"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_retry_completes_suffix_without_replaying_successful_predecessor() {
    setup_retry_regression(false, false, false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_public_dependencies_ignore_locked_unrelated_secret() {
    setup_retry_regression(true, false, false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_preparation_failure_has_proven_unstarted_progress() {
    setup_retry_regression(true, true, false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_rejects_fresh_credential_content_before_command_admission() {
    setup_retry_regression(true, false, true).await;
}

#[cfg(target_os = "linux")]
async fn setup_retry_regression(locked_secret: bool, preparation_failure: bool, changed_public: bool) {
    crate::test_support::isolated_env_test!();
    let _environment = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("envp05-setup-retry");
    let home = root.path().join("home");
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::env::set_var("CHARIOX_HOME", &home);
    let mut config =
        DaemonConfig::for_tests().with_session_history_root(home.join("state/sessions"));
    config.user_config_path = home.join("config.toml");
    config.user_config.state.path = Some(home.join("state/state.db").to_string_lossy().into());
    if locked_secret {
        config.user_config.credential_vault.backend = crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = home.join("fixture-vault.json").to_string_lossy().into();
    }
    config.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
    config.accept_remote_leases = true;
    config.remote_lease_capacity = Some(1);
    config.lease_worker_home_caller = Some(crate::config::LeaseWorkerHomeCaller {
        kernel_id: "fixture-home-kernel".into(),
        realm_id: "realm".into(),
        user_id: "owner".into(),
        relay_public_key: "fixture-home-public".into(),
    });
    config.cloud_relay=Some(serde_json::from_value(serde_json::json!({
        "api_url":"https://fixture.invalid","email":"owner@example.test","account_id":"account","user_id":"owner","account_slug":"account","realm_id":"realm","relay_url":"wss://fixture.invalid","issuer_id":"issuer","machine_id":config.host_machine_id,"machine_credential":format!("mcred_{}","c".repeat(40))
    })).unwrap());
    let receipt = root.path().join("worker-receipt.json");
    std::fs::write(&receipt,serde_json::to_vec(&serde_json::json!({
        "schemaVersion":1,"status":"confirmed","allocationId":"fixture-worker","machineId":config.host_machine_id,"kernelId":config.daemon_id,"relayPublicKey":config.relay_public_key,"runtimeReleaseDigest":format!("sha256:{}","a".repeat(64)),"homeCaller":{"accountId":"account","userId":"owner","realmId":"realm","machineId":"fixture-home-machine","kernelId":"fixture-home-kernel","relayPublicKey":"fixture-home-public"},"confirmedAt":"2026-10-11T00:00:00Z"
    })).unwrap()).unwrap();
    std::env::set_var("CHARIOX_DISPOSABLE_WORKER_RECEIPT", &receipt);
    ensure_worker_validation_boundary(&config).unwrap();
    let app = crate::DaemonApp::bootstrap(config.clone()).unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        1,
    )
    .runtime_state();
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let project = crate::session::RuntimeProject::new(
        "setup-retry-project",
        "owner",
        workspace.to_string_lossy(),
        "Retry",
        crate::session::RuntimeProjectKind::Named,
    );
    let mut env = store.snapshot(&project).unwrap();
    env.reviewed_by = Some("owner".into());
    env.revision = 1;
    let command = |text: &str| EnvironmentCommand {
        executable: "/bin/sh".into(),
        arguments: vec!["-c".into(), text.into()],
        timeout_ms: 5000,
        output_limit_bytes: 4096,
    };
    let mut requirement = Requirement {
        requirement_id: "setup".into(),
        title: "Bounded setup".into(),
        scope: RequirementScope::Project,
        origins: vec![],
        spec: RequirementSpec::SetupChecks {
            commands: vec![
                command("printf first >> successful-count"),
                command("test -f external-fix && printf corrected > corrected-output"),
                command("printf done > suffix-output"),
            ],
            probes: vec![],
            boundary: EnvironmentExecutionBoundary::DisposableWorker,
            source_path: None,
            source_digest: None,
            legacy: None,
        },
        depends_on: vec![],
        platform_variants: vec![],
        required: true,
        legacy_entry: None,
    };
    env.project_requirements = vec![requirement.clone()];
    if locked_secret {
        let mut public = requirement.clone();
        public.requirement_id = "public".into();
        public.spec = RequirementSpec::Variables {
            name: "PUBLIC_BUILD_INPUT".into(),
            locator: ProjectEnvironmentLocator::ConfigFile { path: "public-input".into() },
        };
        std::fs::write(workspace.join("public-input"), "public build input").unwrap();
        env.project_requirements.push(public.clone());
        crate::project_environment::apply_scoped_binding(&store, &config, &env, &public).unwrap();
        let mut stored = store.load(project.id()).unwrap().unwrap();
        stored.manifest.entries.push(ProjectEnvironmentEntry {
            name: "UNRELATED_PRIVATE_INPUT".into(),
            workspace_id: workspace.to_string_lossy().into(),
            kind: ProjectEnvironmentEntryKind::Variable,
            classification: ProjectEnvironmentClassification::Secret,
            excluded: false, uses: vec![],
            locator: ProjectEnvironmentLocator::Vault { service: "fixture-unrelated".into(), key: "private".into() },
            status: ProjectEnvironmentEntryStatus::Found,
        });
        store.save(&stored).unwrap();
        requirement.depends_on.push(public.requirement_id);
        env.project_requirements[0] = requirement.clone();
    }
    let mut operation = EnvironmentOperation {
        operation_id: "retry-operation".into(),
        attempt: 1,
        local_project_id: env.local_project_id.clone(),
        revision_digest: env.content_digest.clone(),
        target: current_check_target(&config),
        kind: EnvironmentOperationKind::Apply,
        phase: EnvironmentOperationPhase::Running,
        selected_items: vec![requirement.requirement_id.clone()],
        per_item_opt_ins: vec![],
        per_item_results: vec![],
        created_at_ms: 1,
        updated_at_ms: 1,
        cancellation: None,
        receipts: vec![],
        recovery_state: EnvironmentRecoveryState::Pending,
    };
    let mut foreign = operation.clone();
    foreign.local_project_id = "other-project".into();
    assert_ne!(
        command_identity(&operation, &requirement, 0),
        command_identity(&foreign, &requirement, 0),
        "command identities are Project-bound"
    );
    let attempt = ApplyAttempt {
        plan_id: "plan".into(),
        request_digest: None,
        operation: operation.clone(),
        folder_binding_digest: folder_bindings_digest(&env),
        binding_scopes: vec![RequirementScope::Project],
        active_requirement: Some("setup".into()),
        uncertain_requirements: vec![],
        setup_admitted_commands: Default::default(),
    };
    store
        .save_applies(
            project.id(),
            &EnvironmentApplies {
                plans: vec![],
                attempts: vec![attempt],
            },
        )
        .unwrap();
    if changed_public {
        // Public at Check/Apply time, credential-shaped when Setup resolves it.
        let replacement = format!("sk-{}", "s".repeat(48));
        std::fs::write(workspace.join("public-input"), &replacement).unwrap();
        let error = apply_setup_requirement(&state, &config, &env, &requirement, &operation, || false).unwrap_err();
        assert!(!workspace.join("successful-count").exists(), "fresh credential content must prevent command admission");
        assert_eq!(store.load_applies(project.id()).unwrap().attempts[0].setup_admitted_commands.get("setup"), Some(&0));
        assert!(!error.to_string().contains(&replacement));
        assert!(setup_retry_is_settled(&state, &env, &requirement, &operation));
        std::fs::write(workspace.join("public-input"), "public build input").unwrap();
    }
    if preparation_failure {
        std::fs::remove_file(workspace.join("public-input")).unwrap();
        assert!(apply_setup_requirement(&state, &config, &env, &requirement, &operation, || false).is_err());
        assert!(!workspace.join("successful-count").exists());
        assert_eq!(store.load_applies(project.id()).unwrap().attempts[0].setup_admitted_commands.get("setup"), Some(&0), "preparation failure must durably prove no command was admitted");
        assert!(setup_retry_is_settled(&state, &env, &requirement, &operation));
        std::fs::write(workspace.join("public-input"), "public build input").unwrap();
    }
    assert!(
        apply_setup_requirement(&state, &config, &env, &requirement, &operation, || false).is_err()
    );
    assert!(workspace.join("successful-count").exists(), "public-only Setup must run despite a locked unrelated Secret");
    assert_eq!(
        std::fs::read_to_string(workspace.join("successful-count")).unwrap(),
        "first"
    );
    assert!(!workspace.join("suffix-output").exists());
    assert_eq!(
        store.load_applies(project.id()).unwrap().attempts[0].setup_admitted_commands["setup"],
        2
    );
    assert!(setup_retry_is_settled(
        &state,
        &env,
        &requirement,
        &operation
    ));
    std::fs::write(workspace.join("external-fix"), "corrected externally").unwrap();
    let mut journal = store.load_applies(project.id()).unwrap();
    operation.attempt = 2;
    let mut retry = journal.attempts[0].clone();
    retry.operation = operation.clone();
    retry.uncertain_requirements = vec!["setup".into()];
    journal.attempts.push(retry);
    store.save_applies(project.id(), &journal).unwrap();
    assert!(
        apply_setup_requirement(&state, &config, &env, &requirement, &operation, || false).is_ok()
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("successful-count")).unwrap(),
        "first",
        "successful predecessor must not repeat"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("suffix-output")).unwrap(),
        "done"
    );
    assert!(setup_commands_completed(
        &state,
        &env,
        &requirement,
        &operation
    ));
}
