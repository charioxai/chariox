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
    setup_retry_regression(SetupScenario::Retry).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_public_dependencies_ignore_locked_unrelated_secret() {
    setup_retry_regression(SetupScenario::LockedSecret).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_preparation_failure_has_proven_unstarted_progress() {
    setup_retry_regression(SetupScenario::MissingPublic).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_rejects_fresh_credential_content_before_command_admission() {
    setup_retry_regression(SetupScenario::ChangedPublic).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_foreign_dependency_cannot_borrow_same_name_local_binding() {
    setup_retry_regression(SetupScenario::ForeignDependency).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_platform_gap_prevents_runner_command_admission() {
    setup_retry_regression(SetupScenario::PlatformGap).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_folder_gap_prevents_runner_command_admission() {
    setup_retry_regression(SetupScenario::FolderGap).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_failed_readiness_probe_remains_eligible_for_repair() {
    setup_retry_regression(SetupScenario::FailedReadiness).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_protected_source_alias_admits_no_command() {
    setup_retry_regression(SetupScenario::ProtectedAlias).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_revision_save_stops_active_command_and_unstarted_suffix() {
    setup_retry_regression(SetupScenario::RevisionChanged).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn envp05a_setup_admission_rechecks_project_owner_and_folder_bindings() {
    setup_retry_regression(SetupScenario::AuthorityChanged).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn review1356_setup_public_file_read_is_cancellable_before_admission() {
    setup_retry_regression(SetupScenario::PublicReadCancel).await;
}
#[cfg(target_os = "linux")]
#[tokio::test]
async fn review1356_setup_source_read_is_cancellable_before_admission() {
    setup_retry_regression(SetupScenario::SourceReadCancel).await;
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum SetupScenario {
    PublicReadCancel,
    SourceReadCancel,
    SlowPlan,
    CancelDuringPlan,
    Retry,
    LockedSecret,
    MissingPublic,
    ChangedPublic,
    ForeignDependency,
    PlatformGap,
    FolderGap,
    FailedReadiness,
    ProtectedAlias,
    RevisionChanged,
    AuthorityChanged,
}

#[cfg(target_os = "linux")]
async fn setup_retry_regression(scenario: SetupScenario) {
    let locked_secret = !matches!(
        scenario,
        SetupScenario::SourceReadCancel
            | SetupScenario::Retry
            | SetupScenario::PlatformGap
            | SetupScenario::FolderGap
            | SetupScenario::FailedReadiness
            | SetupScenario::RevisionChanged
            | SetupScenario::AuthorityChanged
            | SetupScenario::SlowPlan
            | SetupScenario::CancelDuringPlan
    );
    let preparation_failure = matches!(scenario, SetupScenario::MissingPublic);
    let changed_public = matches!(scenario, SetupScenario::ChangedPublic);

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
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path =
            home.join("fixture-vault.json").to_string_lossy().into();
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
    let mut project = crate::session::RuntimeProject::new(
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
    if matches!(scenario, SetupScenario::SourceReadCancel) {
        std::fs::write(workspace.join("public-setup.sh"), "reviewed source").unwrap();
        if let RequirementSpec::SetupChecks {
            source_path,
            source_digest,
            ..
        } = &mut requirement.spec
        {
            *source_path = Some("public-setup.sh".into());
            *source_digest = Some(format!("{:x}", Sha256::digest(b"reviewed source")));
        }
    }
    env.project_requirements = vec![requirement.clone()];
    if locked_secret {
        let mut public = requirement.clone();
        public.requirement_id = "public".into();
        public.spec = RequirementSpec::Variables {
            name: "PUBLIC_BUILD_INPUT".into(),
            locator: ProjectEnvironmentLocator::ConfigFile {
                path: "public-input".into(),
            },
        };
        if matches!(scenario, SetupScenario::ForeignDependency) {
            public.scope = RequirementScope::Folder {
                folder_id: env.folders[0].folder_id.clone(),
            };
        }
        std::fs::write(workspace.join("public-input"), "public build input").unwrap();
        env.project_requirements.push(public.clone());
        crate::project_environment::apply_scoped_binding(&store, &config, &env, &public).unwrap();
        let mut stored = store.load(project.id()).unwrap().unwrap();
        stored.manifest.entries.push(ProjectEnvironmentEntry {
            name: "UNRELATED_PRIVATE_INPUT".into(),
            workspace_id: workspace.to_string_lossy().into(),
            kind: ProjectEnvironmentEntryKind::Variable,
            classification: ProjectEnvironmentClassification::Secret,
            excluded: false,
            uses: vec![],
            locator: ProjectEnvironmentLocator::Vault {
                service: "fixture-unrelated".into(),
                key: "private".into(),
            },
            status: ProjectEnvironmentEntryStatus::Found,
        });
        store.save(&stored).unwrap();
        requirement.depends_on.push(public.requirement_id);
        env.project_requirements[0] = requirement.clone();
    }
    if matches!(scenario, SetupScenario::ForeignDependency) {
        let first_id = env.folders[0].folder_id.clone();
        requirement.scope = RequirementScope::Folder {
            folder_id: first_id.clone(),
        };
        let mut public = env.project_requirements[1].clone();
        public.scope = requirement.scope.clone();
        env.project_requirements.clear();
        env.folders[0].requirements = vec![requirement.clone(), public.clone()];
        crate::project_environment::apply_scoped_binding(&store, &config, &env, &public).unwrap();
        let second_workspace = root.path().join("second-workspace");
        std::fs::create_dir_all(&second_workspace).unwrap();
        std::fs::write(
            second_workspace.join("public-input"),
            "distinct second-folder input",
        )
        .unwrap();
        state
            .owned
            .session_store
            .restore_projects(vec![project.clone()]);
        project = state
            .owned
            .session_store
            .update_project_workspaces(
                project.id(),
                vec![
                    workspace.to_string_lossy().into(),
                    second_workspace.to_string_lossy().into(),
                ],
                "owner",
            )
            .unwrap();
        let mut second = store
            .snapshot(&project)
            .unwrap()
            .folders
            .into_iter()
            .find(|f| f.local_workspace_binding == second_workspace.to_string_lossy())
            .unwrap();
        let mut foreign = public.clone();
        foreign.requirement_id = "foreign-public".into();
        foreign.scope = RequirementScope::Folder {
            folder_id: second.folder_id.clone(),
        };
        second.requirements = vec![foreign.clone()];
        env.folders.push(second);
        crate::project_environment::apply_scoped_binding(&store, &config, &env, &foreign).unwrap();
        requirement.depends_on = vec![foreign.requirement_id.clone()];
        env.folders[0].requirements[0] = requirement.clone();
    }
    if matches!(
        scenario,
        SetupScenario::PlatformGap | SetupScenario::FolderGap | SetupScenario::FailedReadiness
    ) {
        state
            .owned
            .session_store
            .restore_projects(vec![project.clone()]);
        if matches!(scenario, SetupScenario::PlatformGap) {
            requirement.platform_variants = vec![EnvironmentPlatform {
                os: "macos".into(),
                architecture: "any".into(),
            }];
        } else if matches!(scenario, SetupScenario::FolderGap) {
            requirement.scope = RequirementScope::Folder {
                folder_id: env.folders[0].folder_id.clone(),
            };
            env.folders[0].requirements = vec![requirement.clone()];
            env.project_requirements.clear();
        }
        let RequirementSpec::SetupChecks {
            commands, probes, ..
        } = &mut requirement.spec
        else {
            panic!("Setup fixture")
        };
        *commands = vec![command("printf forbidden > platform-command-marker")];
        *probes = vec![EnvironmentProbe::Tcp {
            host: "127.0.0.1".into(),
            port: 9,
            timeout_ms: 20,
        }];
        if matches!(scenario, SetupScenario::FolderGap) {
            env.folders[0].requirements = vec![requirement.clone()];
        } else {
            env.project_requirements = vec![requirement.clone()];
        }
        let current = store.snapshot(&project).unwrap();
        let (saved, _) = store
            .save_revision_locked(
                &current,
                &crate::local::SaveProjectEnvironmentRevisionRequest {
                    project_id: project.id().into(),
                    expected_revision: current.revision,
                    expected_content_digest: current.content_digest.clone(),
                    draft: environment_draft(&env),
                    accepted_proposal_ids: vec![],
                    excluded_proposal_ids: vec![],
                },
                "owner",
            )
            .unwrap();
        if matches!(scenario, SetupScenario::FolderGap) {
            std::fs::remove_dir(&workspace).unwrap();
        }
        let LocalDaemonResponse::ProjectEnvironmentPlan { plan } = state
            .plan_project_environment(
                crate::local::PlanProjectEnvironmentRequest {
                    project_id: project.id().into(),
                    expected_revision: saved.revision,
                    revision_digest: saved.content_digest.clone(),
                    target: current_check_target(&config),
                    selected_items: vec!["setup".into()],
                },
                "owner",
            )
            .await
            .unwrap()
        else {
            panic!("Plan response")
        };
        state
            .apply_project_environment(
                crate::local::ApplyProjectEnvironmentRequest {
                    project_id: project.id().into(),
                    operation_id: "platform-apply".into(),
                    plan_id: plan.plan_id.clone(),
                    expected_revision: saved.revision,
                    revision_digest: saved.content_digest.clone(),
                    target: plan.target.clone(),
                    selected_items: plan.selected_items.clone(),
                    per_item_opt_ins: plan
                        .required_opt_ins
                        .iter()
                        .map(|id| EnvironmentItemOptIn {
                            requirement_id: id.clone(),
                            decision: EnvironmentOptInDecision::Accepted,
                            recorded_at_ms: crate::session::unix_epoch_ms(),
                            user_id: "owner".into(),
                        })
                        .collect(),
                },
                "owner",
            )
            .await
            .unwrap();
        let end = Instant::now() + Duration::from_secs(15);
        let attempt = loop {
            let journal = store.load_applies(project.id()).unwrap();
            if let Some(a) = journal
                .attempts
                .first()
                .filter(|a| !apply_active(&a.operation))
            {
                break a.clone();
            }
            assert!(Instant::now() < end, "Apply runner must settle");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        if matches!(scenario, SetupScenario::FailedReadiness) {
            assert!(
                workspace.join("platform-command-marker").exists(),
                "an ordinary failed readiness probe permits opted-in repair"
            );
            assert_eq!(attempt.setup_admitted_commands.get("setup"), Some(&1));
            assert_eq!(
                attempt.operation.per_item_results[0].reason_code, "setup_unconfirmed",
                "unchanged failed target facts cannot become Ready"
            );
            return;
        }
        assert!(
            !workspace.join("platform-command-marker").exists(),
            "inapplicable Setup must not run declared commands"
        );
        assert!(
            !attempt.setup_admitted_commands.contains_key("setup"),
            "applicability failure must precede Setup admission"
        );
        assert_eq!(
            attempt.operation.per_item_results[0].reason_code,
            if matches!(scenario, SetupScenario::PlatformGap) {
                "platform_gap"
            } else {
                "folder_unavailable"
            }
        );
        return;
    }
    state
        .owned
        .session_store
        .restore_projects(vec![project.clone()]);
    if matches!(
        scenario,
        SetupScenario::RevisionChanged | SetupScenario::SlowPlan | SetupScenario::CancelDuringPlan
    ) {
        let RequirementSpec::SetupChecks { commands, .. } = &mut requirement.spec else {
            panic!("Setup fixture")
        };
        *commands = vec![
            command("printf started > command-started; while [ ! -f release-first ]; do sleep .02; done; printf finished > first-finished"),
            command("printf forbidden > second-command-marker"),
        ];
        env.project_requirements = vec![requirement.clone()];
    }
    let _slow_plan_server = if matches!(
        scenario,
        SetupScenario::SlowPlan | SetupScenario::CancelDuringPlan
    ) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let mut service = requirement.clone();
        service.requirement_id = "slow-service".into();
        service.spec = RequirementSpec::Services {
            identity: "Slow service".into(),
            target_binding: None,
            probe: Some(EnvironmentProbe::Http {
                url,
                expected_status: 200,
                timeout_ms: 10000,
            }),
            health_expectation: None,
            depends_on: vec![],
        };
        env.project_requirements.push(service);
        let RequirementSpec::SetupChecks { commands, .. } = &mut requirement.spec else {
            panic!("setup expected")
        };
        *commands=vec![command("printf started > command-started; while [ ! -f release-first ]; do sleep .02; done; printf finished > first-finished"),command("printf complete > second-command-marker")];
        for c in commands {
            c.timeout_ms = 15000;
        }
        env.project_requirements[0] = requirement.clone();
        let marker = workspace.join("plan-probe-started");
        Some(std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let _ = stream.read(&mut [0; 4096]);
            std::fs::write(marker, "entered").unwrap();
            std::thread::sleep(Duration::from_secs(4));
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }))
    } else {
        None
    };
    let current = store.snapshot(&project).unwrap();
    env = store
        .save_revision_locked(
            &current,
            &crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: project.id().into(),
                expected_revision: current.revision,
                expected_content_digest: current.content_digest.clone(),
                draft: environment_draft(&env),
                accepted_proposal_ids: vec![],
                excluded_proposal_ids: vec![],
            },
            "owner",
        )
        .unwrap()
        .0;
    // MP-08 / MP-10 / MP-11: simulate a pre-validation stored alias only
    // after valid Save; current Save rejection is covered separately. Keep
    // testing the runner defense before ambient-source reads or commands.
    if matches!(scenario, SetupScenario::ProtectedAlias) {
        std::env::set_var("ANTHROPIC_API_KEY", "synthetic-opaque-input");
        let locator = ProjectEnvironmentLocator::WorkspaceEnvironment {
            name: "ANTHROPIC_API_KEY".into(),
        };
        let RequirementSpec::Variables {
            locator: source, ..
        } = &mut env.project_requirements[1].spec
        else {
            panic!("public fixture")
        };
        *source = locator.clone();
        let mut legacy = store.load(project.id()).unwrap().unwrap();
        legacy
            .manifest
            .entries
            .iter_mut()
            .find(|e| e.name == "PUBLIC_BUILD_INPUT")
            .unwrap()
            .locator = locator;
        store.save(&legacy).unwrap();
    }
    requirement = environment_requirements(&env)
        .find(|r| r.requirement_id == requirement.requirement_id)
        .unwrap()
        .clone();
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
    if matches!(
        scenario,
        SetupScenario::PublicReadCancel | SetupScenario::SourceReadCancel
    ) {
        let key = if matches!(scenario, SetupScenario::PublicReadCancel) {
            "CHARIOX_SETUP_PUBLIC_STALL_MARKER"
        } else {
            "CHARIOX_SETUP_SOURCE_STALL_MARKER"
        };
        let marker = root.path().join("setup-io-stall");
        std::env::set_var(key, &marker);
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = cancelled.clone();
        let (run_state, run_config, run_env, run_requirement, run_operation) = (
            state.clone(),
            config.clone(),
            env.clone(),
            requirement.clone(),
            operation.clone(),
        );
        let running = tokio::task::spawn_blocking(move || {
            apply_setup_requirement(
                &run_state,
                &run_config,
                &run_env,
                &run_requirement,
                &run_operation,
                || cancel.load(std::sync::atomic::Ordering::SeqCst),
            )
        });
        let start = Instant::now();
        while !marker.exists() {
            assert!(!running.is_finished(), "fresh setup read must be reached");
            assert!(start.elapsed() < Duration::from_secs(10));
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let began = Instant::now();
        cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        let result = running.await.unwrap();
        std::env::remove_var(key);
        assert!(result.is_err());
        assert!(
            began.elapsed() < Duration::from_millis(900),
            "fresh Setup I/O escaped owned cancellation"
        );
        assert!(
            !workspace.join("successful-count").exists(),
            "cancelled preparation admits no command"
        );
        assert_eq!(
            store.load_applies(project.id()).unwrap().attempts[0]
                .setup_admitted_commands
                .get("setup")
                .copied(),
            Some(0)
        );
        return;
    }
    if matches!(scenario, SetupScenario::AuthorityChanged) {
        for changed in [
            crate::session::RuntimeProject::new(
                project.id(),
                "different-owner",
                workspace.to_string_lossy(),
                "Changed owner",
                crate::session::RuntimeProjectKind::Named,
            ),
            crate::session::RuntimeProject::new(
                project.id(),
                "owner",
                root.path().to_string_lossy(),
                "Changed folder",
                crate::session::RuntimeProjectKind::Named,
            ),
        ] {
            state.owned.session_store.restore_projects(vec![changed]);
            assert!(
                record_setup_admission(&state, &config, &env, &requirement, &operation, 1).is_err(),
                "changed Project authority must reject admission"
            );
            assert!(store.load_applies(project.id()).unwrap().attempts[0]
                .setup_admitted_commands
                .is_empty());
        }
        return;
    }
    if matches!(
        scenario,
        SetupScenario::RevisionChanged | SetupScenario::SlowPlan | SetupScenario::CancelDuringPlan
    ) {
        let run_state = state.clone();
        let run_config = config.clone();
        let run_env = env.clone();
        let run_requirement = requirement.clone();
        let run_operation = operation.clone();
        let running = tokio::task::spawn_blocking(move || {
            apply_setup_requirement(
                &run_state,
                &run_config,
                &run_env,
                &run_requirement,
                &run_operation,
                || false,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        while !workspace.join("command-started").exists() {
            assert!(!running.is_finished(), "first command must actually start");
            assert!(Instant::now() < deadline, "paused command start timeout");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if matches!(
            scenario,
            SetupScenario::SlowPlan | SetupScenario::CancelDuringPlan
        ) {
            let planning_state = state.clone();
            let request = crate::local::PlanProjectEnvironmentRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                revision_digest: env.content_digest.clone(),
                target: current_check_target(&config),
                selected_items: vec!["slow-service".into()],
            };
            let planning = tokio::spawn(async move {
                planning_state
                    .plan_project_environment(request, "owner")
                    .await
            });
            let mut cancellation = None;
            if matches!(scenario, SetupScenario::CancelDuringPlan) {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !workspace.join("plan-probe-started").exists() {
                    assert!(Instant::now() < deadline, "Plan probe must start");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let began = Instant::now();
                let result = state
                    .cancel_environment_operation(
                        crate::local::EnvironmentOperationRequest {
                            project_id: project.id().into(),
                            operation_id: operation.operation_id.clone(),
                        },
                        "owner",
                    )
                    .await;
                cancellation = Some((result, began.elapsed()));
            }
            let planned = planning.await.unwrap();
            assert!(
                matches!(
                    planned,
                    Ok(LocalDaemonResponse::ProjectEnvironmentPlan { .. })
                ),
                "Plan must settle: {planned:?}"
            );
            std::fs::write(workspace.join("release-first"), "release").unwrap();
            let result = running.await.unwrap();
            if let Some((cancelled, elapsed)) = cancellation {
                assert!(cancelled.is_ok(), "Cancel must record cancellation during the concurrent Plan probe: {cancelled:?}");
                assert!(
                    elapsed < Duration::from_secs(3),
                    "Plan held the Project lock for {elapsed:?}"
                );
                assert!(store.load_applies(project.id()).unwrap().attempts[0]
                    .operation
                    .cancellation
                    .is_some());
                assert!(
                    result.is_err(),
                    "cancelled Setup cannot admit another command"
                );
                assert!(!workspace.join("second-command-marker").exists());
                return;
            }
            assert!(
                result.is_ok(),
                "unchanged Setup must survive concurrent slow Plan"
            );
            assert!(workspace.join("first-finished").exists());
            assert!(workspace.join("second-command-marker").exists());
            return;
        }
        let mut draft = environment_draft(&env);
        draft
            .project_requirements
            .retain(|r| r.requirement_id != "setup");
        state
            .save_project_environment_revision(
                crate::local::SaveProjectEnvironmentRevisionRequest {
                    project_id: project.id().into(),
                    expected_revision: env.revision,
                    expected_content_digest: env.content_digest.clone(),
                    draft,
                    accepted_proposal_ids: vec![],
                    excluded_proposal_ids: vec![],
                },
                "owner",
            )
            .await
            .unwrap();
        // The live callback must stop command 1 even before the gate is released.
        let stop_deadline = Instant::now() + Duration::from_secs(2);
        while !running.is_finished() && Instant::now() < stop_deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let stopped_before_release = running.is_finished();
        std::fs::write(workspace.join("release-first"), "release").unwrap();
        let result = running.await.unwrap();
        assert!(
            !workspace.join("second-command-marker").exists(),
            "a newer saved revision must fence command 2"
        );
        assert!(
            stopped_before_release,
            "running callback must revoke command 1 when saved authority changes"
        );
        assert!(result.is_err());
        assert_eq!(
            store.load_applies(project.id()).unwrap().attempts[0]
                .setup_admitted_commands
                .get("setup"),
            Some(&1)
        );
        assert!(
            record_setup_admission(&state, &config, &env, &requirement, &operation, 2).is_err(),
            "locked checkpoint must also fence stale revision"
        );
        return;
    }
    if matches!(scenario, SetupScenario::ForeignDependency) {
        // B is the selected prerequisite; A has an independently applied equal name.
        assert!(
            apply_setup_requirement(&state, &config, &env, &requirement, &operation, || false)
                .is_err()
        );
        assert!(
            !workspace.join("successful-count").exists(),
            "cross-folder prerequisite must not substitute A's unselected value"
        );
        assert_eq!(
            store.load_applies(project.id()).unwrap().attempts[0]
                .setup_admitted_commands
                .get("setup"),
            Some(&0)
        );
        // Explicitly selecting the local prerequisite restores eligible Setup.
        requirement.depends_on = vec!["public".into()];
        env.folders[0].requirements[0] = requirement.clone();
        let current = store.snapshot(&project).unwrap();
        env = store
            .save_revision_locked(
                &current,
                &crate::local::SaveProjectEnvironmentRevisionRequest {
                    project_id: project.id().into(),
                    expected_revision: current.revision,
                    expected_content_digest: current.content_digest.clone(),
                    draft: environment_draft(&env),
                    accepted_proposal_ids: vec![],
                    excluded_proposal_ids: vec![],
                },
                "owner",
            )
            .unwrap()
            .0;
        operation.revision_digest = env.content_digest.clone();
        let mut journal = store.load_applies(project.id()).unwrap();
        journal.attempts[0].operation = operation.clone();
        journal.attempts[0].folder_binding_digest = folder_bindings_digest(&env);
        store.save_applies(project.id(), &journal).unwrap();
    }
    if matches!(scenario, SetupScenario::ProtectedAlias) {
        let error =
            apply_setup_requirement(&state, &config, &env, &requirement, &operation, || false)
                .unwrap_err();
        assert!(
            !workspace.join("successful-count").exists(),
            "protected aliases must prevent every command"
        );
        assert_eq!(
            store.load_applies(project.id()).unwrap().attempts[0]
                .setup_admitted_commands
                .get("setup"),
            Some(&0)
        );
        assert!(!error.to_string().contains("synthetic-opaque-input"));
        return;
    }
    if changed_public {
        // Public at Check/Apply time, credential-shaped when Setup resolves it.
        let replacement = format!("sk-{}", "s".repeat(48));
        std::fs::write(workspace.join("public-input"), &replacement).unwrap();
        let error =
            apply_setup_requirement(&state, &config, &env, &requirement, &operation, || false)
                .unwrap_err();
        assert!(
            !workspace.join("successful-count").exists(),
            "fresh credential content must prevent command admission"
        );
        assert_eq!(
            store.load_applies(project.id()).unwrap().attempts[0]
                .setup_admitted_commands
                .get("setup"),
            Some(&0)
        );
        assert!(!error.to_string().contains(&replacement));
        assert!(setup_retry_is_settled(
            &state,
            &env,
            &requirement,
            &operation
        ));
        std::fs::write(workspace.join("public-input"), "public build input").unwrap();
    }
    if preparation_failure {
        std::fs::remove_file(workspace.join("public-input")).unwrap();
        assert!(
            apply_setup_requirement(&state, &config, &env, &requirement, &operation, || false)
                .is_err()
        );
        assert!(!workspace.join("successful-count").exists());
        assert_eq!(
            store.load_applies(project.id()).unwrap().attempts[0]
                .setup_admitted_commands
                .get("setup"),
            Some(&0),
            "preparation failure must durably prove no command was admitted"
        );
        assert!(setup_retry_is_settled(
            &state,
            &env,
            &requirement,
            &operation
        ));
        std::fs::write(workspace.join("public-input"), "public build input").unwrap();
    }
    assert!(
        apply_setup_requirement(&state, &config, &env, &requirement, &operation, || false).is_err()
    );
    assert!(
        workspace.join("successful-count").exists(),
        "public-only Setup must run despite a locked unrelated Secret"
    );
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

#[cfg(target_os = "linux")]
#[tokio::test]
async fn review0034_unchanged_setup_survives_concurrent_slow_plan() {
    setup_retry_regression(SetupScenario::SlowPlan).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn r6_environment_cancel_remains_available_during_concurrent_plan_probe() {
    setup_retry_regression(SetupScenario::CancelDuringPlan).await;
}
