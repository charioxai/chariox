use super::*;
use crate::provider::{
    AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest,
    ProviderLaunchResult, ProviderResumeState, ProviderRunState, ProviderRunTokenUsage,
    RuntimeProviderRun,
};

async fn compact_fixture() -> (
    crate::test_support::TestWorktree,
    Arc<Mutex<DaemonApp>>,
    KernelRuntimeState,
    String,
    String,
    RuntimeProviderRun,
) {
    let (root, app, runtime, session, agent, run, _) = compact_fixture_with_lease(false).await;
    (root, app, runtime, session, agent, run)
}

async fn compact_fixture_with_lease(
    leased: bool,
) -> (
    crate::test_support::TestWorktree,
    Arc<Mutex<DaemonApp>>,
    KernelRuntimeState,
    String,
    String,
    RuntimeProviderRun,
    Option<crate::execution_lease::LeasedAgent>,
) {
    let root = crate::test_support::TestWorktree::new("profile-compact");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = leased;
    if !leased {
        crate::test_support::serve_runtime_mcp(&mut config);
    }
    let (app, runtime, mut session, mut agent) = agent_config_runtime_in_worktree(
        config,
        crate::session::DEFAULT_LOCAL_USER_ID,
        root.session_request(),
    )
    .await;
    let lease = if leased {
        let mut app = app.lock().await;
        let mut worker = crate::app::RemoteLeaseRuntime::new(&mut app);
        let execution = worker
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .unwrap();
        let lease = worker
            .create_leased_agent(
                &execution.id,
                "claude",
                "default",
                Some("claude-sonnet-5-5[1m]".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let lease = worker
            .update_leased_agent_profile(
                &lease.id,
                lease.provider.clone(),
                "default".into(),
                lease.model.clone(),
                lease.effort.clone(),
            )
            .unwrap();
        session = lease.backing_session_id.clone();
        agent = lease.backing_agent_id.clone();
        Some(lease)
    } else {
        None
    };
    let account = runtime
        .owned
        .provider_account_profiles
        .get(crate::session::DEFAULT_LOCAL_USER_ID, "claude", "default")
        .unwrap()
        .profile_id;
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent,
            "claude",
            Some("claude-sonnet-5-5[1m]".into()),
            None,
            Some(account.clone()),
            ProviderResumeState::default(),
        )
        .unwrap();
    let request = LaunchProviderRequest::new(
        &session,
        "claude",
        "claude",
        &account,
        "claude-sonnet-5-5[1m]",
    )
    .with_agent_id(&agent);
    let marker = root.path().join("started");
    let release = root.path().join("release");
    let mut run = RuntimeProviderRun::new("compact-run", &request, ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::Managed, process_label: "fixture".into(), pty_target: None, pty_program: Some("/bin/sh".into()),
        pty_args: vec!["-c".into(), format!("while IFS= read -r line; do touch '{}'; n=0; while [ ! -f '{}' ] && [ $n -lt 30 ]; do sleep .1; n=$((n+1)); done; printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\"}}'; done", marker.display(), release.display())],
        pty_env: Default::default(), pty_env_remove: vec![], working_directory: None, structured_endpoint: None,
    });
    run.set_execution_config(AgentExecutionMode::Plan, AgentPermissionLevel::Required);
    run.mark_running();
    run.set_usage(ProviderRunTokenUsage {
        context_tokens: Some(320_000),
        ..Default::default()
    });
    runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run.clone());
    runtime
        .owned
        .provider_store
        .initialize_runtime(&run)
        .unwrap();
    runtime.owned.provider_run_projection.update(run.clone());
    (root, app, runtime, session, agent, run, lease)
}

#[tokio::test]
async fn profile_compaction_reserves_local_agent_until_the_profile_is_committed() {
    let (root, app, runtime, session, agent, run) = compact_fixture().await;
    let state = runtime.clone();
    let s = session.clone();
    let a = agent.clone();
    let update = tokio::spawn(async move {
        state
            .update_agent_profile(
                &s,
                &a,
                crate::session::DEFAULT_LOCAL_USER_ID,
                None,
                Some("default".into()),
                Some("haiku".into()),
                None,
            )
            .await
    });
    let marker = root.path().join("started");
    for _ in 0..100 {
        if marker.exists() || update.is_finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let started = marker.exists();
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Running
    );
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    let outcome = runtime
        .owned
        .prompt_state_owner
        .submit_prepared_prompt(
            &snapshot,
            crate::session::PromptQueueItem::new(
                "during-compact",
                "fixture",
                &agent,
                "request during compact",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .unwrap();
    std::fs::write(root.path().join("release"), "").unwrap();
    let result = update.await.unwrap();
    assert!(
        started,
        "profile update should compact before retiring the source run: {result:?}"
    );
    assert!(
        matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ),
        "prompt must queue during compaction"
    );
    assert_eq!(result.unwrap().model(), Some("haiku"));
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Ended
    );
    drop(app);
}

#[tokio::test]
async fn profile_compaction_validates_busy_and_unknown_account_before_compacting() {
    let (root, app, runtime, session, agent, run) = compact_fixture().await;
    let invalid = runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            Some("missing-account".into()),
            Some("haiku".into()),
            None,
        )
        .await;
    assert!(invalid.is_err());
    assert!(!root.path().join("started").exists());
    sync_active_prompt(&app, &session, &agent).await;
    let busy = runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some("haiku".into()),
            None,
        )
        .await;
    assert!(busy.is_err());
    assert!(!root.path().join("started").exists());
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Running
    );
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
}

#[tokio::test]
async fn profile_compaction_session_command_accepts_empty_output() {
    let (root, _app, runtime, _session, _agent, run) = compact_fixture().await;
    std::fs::write(root.path().join("release"), "").unwrap();
    let output = runtime
        .run_structured_provider_utility_prompt(
            run.clone(),
            "/compact".into(),
            String::new(),
            std::time::Duration::from_secs(2),
            crate::provider::ProviderUtilityExecutionPolicy::SessionCommand,
        )
        .await
        .unwrap();
    assert_eq!(output, "");
    assert!(root.path().join("started").exists());
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(run.session_id(), run.id())
        .unwrap();
}

#[tokio::test]
async fn profile_compaction_does_not_run_on_the_home_for_a_remote_agent() {
    let (root, app, runtime, session, agent, run) = compact_fixture().await;
    app.lock()
        .await
        .agents_mut()
        .bind_remote_execution(
            &agent,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "unavailable-worker".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: Some("worker-run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: None,
            },
        )
        .unwrap();
    let result = runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some("haiku".into()),
            None,
        )
        .await;
    assert!(
        result.is_err(),
        "unavailable worker must leave the home profile unchanged"
    );
    assert!(
        !root.path().join("started").exists(),
        "home must not compact a worker-owned session"
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Running
    );
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
}

#[tokio::test]
async fn leased_worker_compacts_before_retiring_the_source_and_reserves_admission() {
    let (root, app, runtime, session, agent, run, lease) = compact_fixture_with_lease(true).await;
    let lease = lease.unwrap();
    let state = runtime.clone();
    let id = lease.id.clone();
    let update = tokio::spawn(async move {
        state
            .update_relay_leased_agent_profile(
                &id,
                "claude".into(),
                "default".into(),
                Some("haiku".into()),
                None,
            )
            .await
    });
    for _ in 0..100 {
        if root.path().join("started").exists() || update.is_finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let started = root.path().join("started").exists();
    let source_state = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .unwrap()
        .state();
    let app_available = tokio::time::timeout(std::time::Duration::from_millis(100), app.lock())
        .await
        .is_ok();
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    let admission_reserved = runtime
        .owned
        .prompt_state_owner
        .claim_idle_agent_profile_transition(&snapshot, &agent)
        .is_err();
    std::fs::write(root.path().join("release"), "").unwrap();
    let result = update.await.unwrap();
    assert!(
        started,
        "worker must compact before retiring its source run: {result:?}"
    );
    assert_eq!(source_state, ProviderRunState::Running);
    assert!(
        app_available,
        "worker compaction must not hold the app mutex"
    );
    assert!(
        admission_reserved,
        "backing agent admission must be reserved"
    );
    let result = result.unwrap();
    assert_eq!(result.model.as_deref(), Some("haiku"));
    assert_eq!(result.backing_agent_id, lease.backing_agent_id);
    assert_eq!(result.lease_id, lease.lease_id);
    drop(
        runtime
            .owned
            .prompt_state_owner
            .claim_idle_agent_profile_transition(&snapshot, &agent)
            .unwrap(),
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Ended
    );
}

#[tokio::test]
async fn leased_worker_validates_before_compaction_and_confirms_busy_unchanged_profiles() {
    let (root, app, runtime, session, agent, run, lease) = compact_fixture_with_lease(true).await;
    let lease = lease.unwrap();
    assert!(runtime
        .update_relay_leased_agent_profile(
            &lease.id,
            "claude".into(),
            "missing-account".into(),
            Some("haiku".into()),
            None
        )
        .await
        .is_err());
    sync_active_prompt(&app, &session, &agent).await;
    assert!(runtime
        .update_relay_leased_agent_profile(
            &lease.id,
            "claude".into(),
            "default".into(),
            Some("haiku".into()),
            None
        )
        .await
        .is_err());
    let unchanged = runtime
        .update_relay_leased_agent_profile(
            &lease.id,
            lease.provider.clone(),
            lease.account_profile.clone(),
            lease.model.clone(),
            lease.effort.clone(),
        )
        .await
        .unwrap();
    assert_eq!(unchanged.id, lease.id);
    assert!(!root.path().join("started").exists());
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        ProviderRunState::Running
    );
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
}

#[tokio::test]
async fn empty_profile_queue_does_not_prepare_project_or_request_a_vault_unlock() {
    let root = crate::test_support::TestWorktree::new("profile-empty-vault");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.unlock_policy =
        crate::config::CredentialVaultUnlockPolicy::Always;
    config.user_config.credential_vault.path = root
        .path()
        .join("synthetic-vault.json")
        .display()
        .to_string();
    let (app, runtime, session_id, agent_id) = agent_config_runtime_with_config(config).await;
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let evidence = crate::project_environment::ProjectEnvironmentEvidence::default();
    crate::project_environment::ProjectEnvironmentStore::new(
        &runtime
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root(),
    )
    .save(&crate::project_environment::StoredProjectEnvironment {
        source: None,
        manifest: crate::project_environment::ProjectEnvironmentManifest {
            schema_version: 1,
            project_id: session.project_id().into(),
            evidence_digest: evidence.digest(),
            entries: vec![],
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
    for account in [None, Some("missing-account".into())] {
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            runtime.update_agent_profile(
                &session_id,
                &agent_id,
                crate::session::DEFAULT_LOCAL_USER_ID,
                account.as_ref().map(|_| "codex".to_string()),
                account.clone(),
                None,
                None,
            ),
        )
        .await;
        assert!(
            result.is_ok(),
            "an empty queue must not wait on the Always-policy vault interaction"
        );
        assert_eq!(result.unwrap().is_err(), account.is_some());
    }
    drop(app);
}

#[tokio::test]
async fn profile_compaction_does_not_block_session_commands_or_interaction_responses() {
    let (root, app, runtime, session_id, agent_id, run) = compact_fixture().await;
    let actor =
        crate::runtime::session_actor::SessionRuntime::with_queue_limit_and_focus_projection(
            runtime.clone(),
            8,
            crate::runtime::session_actor::FocusedAgentProjection::default(),
            runtime.owned.session_projection.clone(),
            crate::runtime::projection::AgentRuntimeProjectionStore::default(),
            runtime.owned.terminal_stream.clone(),
        );
    let request = crate::local::LocalDaemonRequest::UpdateAgentProfile(
        crate::local::UpdateAgentProfileRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
            provider: None,
            account_profile: None,
            model: Some("haiku".into()),
            effort: None,
            clear_effort: false,
        },
    );
    let update_actor = actor.clone();
    let update = tokio::spawn(async move {
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "profile", None, None, &request,
        );
        update_actor
            .dispatch_session_command(command, request)
            .await
    });
    for _ in 0..100 {
        if root.path().join("started").exists() || update.is_finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let started = root.path().join("started").exists();
    let request = crate::local::LocalDaemonRequest::RespondToInteraction(
        crate::local::RespondToInteractionRequest {
            session_id: session_id.clone(),
            interaction_id: "missing-interaction".into(),
            choice_id: "deny".into(),
            custom_reply: None,
            passkey: None,
            passkey_remember_minutes: None,
        },
    );
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "interaction",
        None,
        None,
        &request,
    );
    let available = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        actor.dispatch_session_command(command, request),
    )
    .await;
    std::fs::write(root.path().join("release"), "").unwrap();
    let updated = update.await.unwrap();
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session_id, run.id())
        .unwrap();
    assert!(started, "fixture must reach /compact: {updated:?}");
    assert!(
        available.is_ok(),
        "another agent's interaction response must reach the session lane during /compact"
    );
    assert!(
        updated.is_ok(),
        "profile must commit after compaction: {updated:?}"
    );
    drop(app);
}

#[tokio::test]
async fn a_prompt_queued_during_compaction_reaches_the_committed_profile() {
    crate::test_support::isolated_env_test!();
    assert_queued_prompt_reaches_committed_profile(false).await;
}

#[tokio::test]
async fn a_leased_prompt_queued_during_compaction_reaches_the_committed_profile() {
    crate::test_support::isolated_env_test!();
    assert_queued_prompt_reaches_committed_profile(true).await;
}

async fn assert_queued_prompt_reaches_committed_profile(leased: bool) {
    let (root, app, runtime, session_id, agent_id, run, lease) =
        compact_fixture_with_lease(leased).await;
    let received = root.path().join("new-profile-input");
    let executable = root.path().join("claude-queue-fixture");
    std::fs::write(&executable, format!("#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> '{}'; printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\"}}'; done\n", received.display())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("CHARIOX_CLAUDE_BIN", &executable);
    let account = runtime
        .owned
        .agent_store
        .get_agent(&agent_id)
        .unwrap()
        .provider_account_profile()
        .to_string();
    crate::test_support::authenticate_provider_account(
        &runtime.owned.provider_account_profiles,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        &account,
    )
    .unwrap();
    // This isolated child uses a fake CLI and never discovers a real login.
    let environment = runtime
        .owned
        .provider_account_profiles
        .resolve_environment(crate::session::DEFAULT_LOCAL_USER_ID, "claude", &account)
        .unwrap();
    let config_dir = environment
        .get("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join(".claude")
        });
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join(".credentials.json"),
        br#"{"claudeAiOauth":{"refreshToken":"synthetic-fixture"}}"#,
    )
    .unwrap();
    let attachment = {
        let mut app = app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "queue-fixture",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap()
    };
    let state = runtime.clone();
    let session = session_id.clone();
    let agent = agent_id.clone();
    let update = tokio::spawn(async move {
        if let Some(lease) = lease {
            state
                .update_relay_leased_agent_profile(
                    &lease.id,
                    "claude".into(),
                    "default".into(),
                    Some("haiku".into()),
                    None,
                )
                .await
                .map(|updated| updated.model)
        } else {
            state
                .update_agent_profile(
                    &session,
                    &agent,
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    None,
                    None,
                    Some("haiku".into()),
                    None,
                )
                .await
                .map(|updated| updated.model().map(str::to_string))
        }
    });
    for _ in 0..100 {
        if root.path().join("started").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let snapshot = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let queued = runtime
        .owned
        .prompt_state_owner
        .submit_prepared_prompt(
            &snapshot,
            crate::session::PromptQueueItem::new(
                "queued-after-compact",
                attachment.id(),
                &agent_id,
                "request after source compact",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .unwrap();
    std::fs::write(root.path().join("release"), "").unwrap();
    let updated = update.await.unwrap().unwrap();
    assert!(matches!(
        queued,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    assert_eq!(updated.as_deref(), Some("haiku"));
    for _ in 0..300 {
        if received.exists() {
            break;
        }
        if let Some(current) = runtime
            .owned
            .provider_store
            .get_run_for_agent(&session_id, &agent_id)
        {
            let _ = runtime
                .pump_owned_provider_output(&session_id, current.id(), vec![], false)
                .await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let current = runtime
        .owned
        .provider_store
        .get_run_for_agent(&session_id, &agent_id);
    let input = std::fs::read_to_string(&received).unwrap_or_default();
    if let Some(current) = &current {
        runtime
            .owned
            .provider_store
            .terminate_run_provider_only(&session_id, current.id())
            .unwrap();
    }
    assert!(
        input.contains("request after source compact"),
        "queued request must reach the new runtime: {input:?}"
    );
    let current = current.unwrap();
    assert_eq!(current.model(), "haiku");
    assert_ne!(current.id(), run.id());
}
