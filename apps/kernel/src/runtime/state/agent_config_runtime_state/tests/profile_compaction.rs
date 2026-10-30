use super::*;
#[path = "profile_compaction/queue_head_review.rs"]
mod queue_head_review;
#[path = "profile_compaction/unknown_context_review.rs"]
mod unknown_context_review;
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
        pty_args: vec!["-c".into(), format!("while IFS= read -r line; do case \"$line\" in *'surviving queued request'*) touch '{}';; *'/compact'*) printf '%s' \"$line\" > '{}';; esac; n=0; while [ ! -f '{}' ] && [ $n -lt 30 ]; do sleep .1; n=$((n+1)); done; if [ -f '{}' ]; then printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"error_during_execution\",\"is_error\":true,\"errors\":[\"native compaction failed\"]}}'; else printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\"}}'; fi; done", root.path().join("surviving-submitted").display(), marker.display(), release.display(), root.path().join("fail-compact").display())],
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn profile_queue_claims_reject_downshift_during_preparation() {
    crate::test_support::isolated_env_test!();
    assert_profile_queue_claims_keep_app_available(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn profile_queue_claims_defer_reserved_queue_without_holding_app_lock() {
    crate::test_support::isolated_env_test!();
    assert_profile_queue_claims_keep_app_available(false).await;
}

async fn assert_profile_queue_claims_keep_app_available(preparation_first: bool) {
    let (root, app, runtime, session, agent, run) = compact_fixture().await;
    // Only the fixture's isolated account is marked authenticated. Otherwise
    // provider admission would stop before reaching the queue activation seam.
    let account = runtime
        .owned
        .agent_store
        .get_agent(&agent)
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
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    runtime
        .owned
        .prompt_state_owner
        .submit_prepared_prompt(
            &snapshot,
            crate::session::PromptQueueItem::new(
                "older",
                "fixture",
                &agent,
                "older queued request",
                crate::session::PromptStatus::Queued,
            ),
            true,
        )
        .unwrap();
    // Hold the actual preparation claim across a gate, just as Project/Vault
    // preparation does, then use normal app queue activation after release.
    let preparation = preparation_first.then(|| {
        runtime
            .owned
            .prompt_state_owner
            .try_claim_idle_queue_promotion(&snapshot, &agent)
            .unwrap()
    });
    let (release_preparation, preparation_gate) = tokio::sync::oneshot::channel();
    let (entered_app, app_entry) = tokio::sync::oneshot::channel();
    let promoter_state = runtime.clone();
    let promoter_session = session.clone();
    let promoter_agent = agent.clone();
    let promotion = tokio::spawn(async move {
        let _preparation = preparation;
        preparation_gate.await.unwrap();
        promoter_state
            .with_app_side_effect(|app| {
                let _ = entered_app.send(());
                app.advance_next_queued_prompt(&promoter_session, &promoter_agent)
            })
            .await
    });
    let update_state = runtime.clone();
    let update_session = session.clone();
    let update_agent = agent.clone();
    let update = tokio::spawn(async move {
        update_state
            .update_agent_profile(
                &update_session,
                &update_agent,
                crate::session::DEFAULT_LOCAL_USER_ID,
                None,
                None,
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
    let compaction_started = root.path().join("started").exists();
    release_preparation.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), app_entry)
        .await
        .unwrap()
        .unwrap();
    // The queue closure has the app mutex before this unrelated read begins.
    // On RED it spins on the profile-reserved head while /compact remains gated.
    // A held mutex never releases while /compact is gated, so a generous bound
    // separates RED from a slow, loaded host.
    let available = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        runtime.with_app_side_effect(|app| {
            crate::app::KernelSessionReadService::new(app).session_snapshot(&session)
        }),
    )
    .await;
    let compaction_still_gated = !root.path().join("release").exists();
    // Always unblock the fixture and drop the profile claim before asserting,
    // including RED: cancellation lets the synchronous queue loop finish.
    std::fs::write(root.path().join("release"), "").unwrap();
    update.abort();
    let updated = update.await;
    let promoted = tokio::time::timeout(std::time::Duration::from_secs(5), promotion)
        .await
        .unwrap()
        .unwrap();
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
    assert!(compaction_still_gated);
    assert!(
        available.is_ok(),
        "profile-reserved queue held the app mutex during gated compaction"
    );
    assert!(available.unwrap().is_ok());
    if preparation_first {
        assert!(
            !compaction_started,
            "profile downshift must reject the already-held preparation claim"
        );
        assert!(updated.unwrap().is_err());
    } else {
        assert!(
            compaction_started,
            "fixture must reach /compact before queue activation"
        );
        assert!(
            promoted.unwrap().is_none(),
            "profile-reserved work must defer without consuming its head"
        );
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .peek_next_queued_prompt(&snapshot, &agent)
                .unwrap()
                .prompt(),
            "older queued request"
        );
    }
    drop(app);
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
    let command = std::fs::read_to_string(&marker).unwrap();
    assert!(
        command.contains("/compact Keep the latest explicit user goal"),
        "compaction should focus on the current records: {command}"
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
            None,
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
        apply_owned_bindings: Default::default(),
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
async fn a_leased_backlog_uses_the_next_home_launch_credential_and_receipt() {
    crate::test_support::isolated_env_test!();
    assert_queued_prompt_reaches_committed_profile(true).await;
}

async fn assert_queued_prompt_reaches_committed_profile(leased: bool) {
    let (root, app, runtime, session_id, agent_id, run, lease) =
        compact_fixture_with_lease(leased).await;
    // MP-08 / MP-10 / MP-11: queued work after /compact must pass through
    // normal handoff assembly, even when synchronous activation starts a new run.
    let earlier_prompt = crate::history::SessionHistoryEntry::user_prompt(
        &session_id,
        "earlier-source",
        &agent_id,
        "Review continuity and keep the proposal distinct from accepted decisions.",
    );
    runtime
        .owned
        .operational_history_store
        .append_transcript(
            &earlier_prompt,
            crate::history::HistoryEventTurnContext {
                prompt_id: Some("earlier-audit".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let proposal = crate::history::SessionHistoryEntry::provider_output(
        &session_id,
        run.id(),
        Some(&agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        "Assistant proposal: Continuity Ledger. Keep it distinct from accepted decisions.",
    );
    runtime
        .owned
        .operational_history_store
        .append_transcript(
            &proposal,
            crate::history::HistoryEventTurnContext {
                prompt_id: Some("earlier-audit".into()),
                provider: Some(run.provider().into()),
                model: Some(run.model().into()),
                provider_run_id: Some(run.id().into()),
                ..Default::default()
            },
        )
        .unwrap();
    let received = root.path().join("new-profile-input");
    let credential_matches = root.path().join("home-credential-matches");
    let executable = root.path().join("claude-queue-fixture");
    std::fs::write(&executable, format!("#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> '{}'; printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\"}}'; done\n", received.display())).unwrap();
    if leased {
        // Only a boolean is recorded; synthetic launch secrets never enter logs.
        std::fs::write(&executable, format!("#!/bin/sh\nif [ \"$CLAUDE_CODE_OAUTH_TOKEN\" = 'synthetic-home-token' ]; then touch '{}'; fi\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> '{}'; printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\"}}'; done\n", credential_matches.display(), received.display())).unwrap();
    }
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
    // Deliberately give the worker a different synthetic login: queue promotion
    // must still use the home credential, never this local account login.
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
    let update_lease = lease.clone();
    let update = tokio::spawn(async move {
        if let Some(lease) = update_lease {
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
    if let Some(lease) = &lease {
        // Profile acknowledgements have no launch credential. A local worker
        // account must not cause a cold launch before the home authorizes it.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let premature = runtime
            .owned
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|candidate| {
                candidate.agent_instance_id() == Some(agent_id.as_str())
                    && candidate.model() == "haiku"
            })
            .collect::<Vec<_>>();
        for candidate in &premature {
            let _ = runtime
                .owned
                .provider_store
                .terminate_run_provider_only(&session_id, candidate.id());
        }
        assert!(
            premature.is_empty(),
            "worker backlog must wait for a credential-bearing home launch"
        );
        let (launched, _) = runtime
            .submit_relay_leased_prompt(
                &lease.id,
                crate::transport::relay_peer::RelayAgentExecutionProfile {
                    provider: lease.provider.clone(),
                    account_profile: lease.account_profile.clone(),
                    model: Some("haiku".into()),
                    effort: None,
                },
                None,
                "next home request",
                "",
                Vec::new(),
                None,
                Some(crate::transport::relay_peer::RemoteGitTurnContext {
                    home_session_id: "home-session".into(),
                    home_agent_id: "home-agent".into(),
                    home_prompt_id: "next-home-prompt".into(),
                    home_turn_id: "next-home-prompt".into(),
                    source_attachment_id: None,
                    workspace_live_sync_mode: None,
                    prompt_origin: Some(crate::session::PromptOrigin::Chariox),
                    external_provider: None,
                    external_provider_session_id: None,
                    external_provider_turn_id: None,
                    prompt_summary: "next home request".into(),
                }),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
                Some(
                    crate::transport::relay_peer::RemoteProviderLaunchCredential {
                        provider: lease.provider.clone(),
                        account_profile: lease.account_profile.clone(),
                        secret_input:
                            crate::transport::relay_peer::RemoteCredentialSecretInput::new(
                                "synthetic-home-token".into(),
                            ),
                    },
                ),
            )
            .await
            .unwrap();
        assert!(runtime
            .owned
            .provider_run_projection
            .is_leased_provider_run(&launched));
        let receipt = runtime
            .query_relay_leased_prompt_receipt(&lease.id, "next-home-prompt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.worker_provider_run_id, launched);
        assert_eq!(
            receipt.phase,
            crate::transport::relay_peer::LeasedPromptReceiptPhase::Active
        );
    }
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
    assert!(
        input.contains("Continuity Ledger"),
        "the actual queued submission must carry protected pre-compaction history: {input:?}"
    );
    let current = current.unwrap();
    assert_eq!(current.model(), "haiku");
    assert_ne!(current.id(), run.id());
    if leased {
        assert!(
            credential_matches.exists(),
            "cold launch must receive the home's credential"
        );
        assert!(runtime
            .owned
            .provider_run_projection
            .is_leased_provider_run(current.id()));
        assert_eq!(input.matches("request after source compact").count(), 1);
    }
}

// MP-08 / MP-10 / MP-11: a native summary can omit an early named proposal.
// The next turn must also receive protected kernel history, even with the same SID.
#[tokio::test]
async fn profile_compaction_preserves_history_for_the_next_native_session_turn() {
    let (root, _, runtime, session, agent, run) = compact_fixture().await;
    let prompt = crate::history::SessionHistoryEntry::user_prompt(
        &session,
        "attachment",
        &agent,
        "Review continuity without changing the accepted decision.",
    );
    let proposal = crate::history::SessionHistoryEntry::provider_output(
        &session,
        run.id(),
        Some(&agent),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        "Assistant proposal: Continuity Ledger. This is not an accepted user decision.",
    );
    for entry in [&prompt, &proposal] {
        runtime
            .owned
            .operational_history_store
            .append_transcript(
                entry,
                crate::history::HistoryEventTurnContext {
                    prompt_id: Some("audit-prompt".into()),
                    turn_id: Some("audit-turn".into()),
                    provider: Some(run.provider().into()),
                    model: Some(run.model().into()),
                    provider_run_id: Some(run.id().into()),
                    provider_session_id: Some("same-native-session".into()),
                    ..Default::default()
                },
            )
            .unwrap();
    }
    std::fs::write(root.path().join("release"), "").unwrap();
    let agent_snapshot = runtime.owned.agent_store.get_agent(&agent).unwrap();
    runtime
        .compact_before_window_downshift(&session, &agent_snapshot, None, None, Some("haiku"))
        .await
        .unwrap();
    let mut target = run.clone();
    target.set_model("haiku");
    target.set_provider_session_id(Some("same-native-session".into()));
    let handoff = runtime
        .owned
        .pending_agent_context_handoffs
        .peek_matching(&session, &agent, &target)
        .expect(
            "successful native compaction must preserve authoritative history for the next turn",
        );
    let packet = handoff.render_hidden(24_000);
    assert!(packet.contains("Continuity Ledger"));
    assert!(packet.contains("not an accepted user decision"));
    // A user may select another model before the first post-compaction turn.
    // Native compaction already removed records regardless of that selection.
    target.set_model("sonnet");
    let next_selection = runtime
        .owned
        .pending_agent_context_handoffs
        .peek_matching(&session, &agent, &target)
        .expect("an intervening model selection must retain compacted history");
    assert!(next_selection
        .render_hidden(24_000)
        .contains("Continuity Ledger"));
    let dispatch = runtime
        .owned
        .context_handoff_for_dispatch(&session, &agent, &target, "next-turn", false)
        .unwrap()
        .expect("the actual next run must receive the compacted records");
    assert_eq!(dispatch.target_model.as_deref(), Some("sonnet"));
    assert!(dispatch.render_hidden(24_000).contains("Continuity Ledger"));
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
}

#[path = "profile_compaction/remote_profile_notice.rs"]
mod remote_profile_notice;

// MP-08 / MP-10 / MP-11: failed source compaction must not give the smaller
// model the same oversized native transcript, on local or ordinary worker paths.
#[tokio::test]
async fn failed_source_compaction_clears_only_matching_native_resume_before_local_commit() {
    assert_failed_source_compaction_starts_fresh(false).await;
}

#[tokio::test]
async fn failed_source_compaction_clears_only_matching_native_resume_before_worker_commit() {
    assert_failed_source_compaction_starts_fresh(true).await;
}

async fn assert_failed_source_compaction_starts_fresh(leased: bool) {
    let (root, _app, runtime, session, agent, mut run, lease) =
        compact_fixture_with_lease(leased).await;
    let snapshot = runtime.owned.agent_store.get_agent(&agent).unwrap();
    crate::test_support::authenticate_provider_account(
        &runtime.owned.provider_account_profiles,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        snapshot.provider_account_profile(),
    )
    .unwrap();
    runtime
        .owned
        .agent_store
        .write()
        .set_agent_provider_resume_state(
            &agent,
            ProviderResumeState::from_external_provider_session(
                "claude",
                "oversized-native-session",
            ),
        )
        .unwrap();
    run.set_provider_session_id(Some("oversized-native-session".into()));
    runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run.clone());
    std::fs::write(root.path().join("fail-compact"), "").unwrap();
    std::fs::write(root.path().join("release"), "").unwrap();
    if let Some(lease) = lease {
        runtime
            .apply_leased_profile_transition(
                &lease.id,
                crate::transport::relay_peer::RelayAgentExecutionProfile {
                    provider: "claude".into(),
                    account_profile: "default".into(),
                    model: Some("haiku".into()),
                    effort: None,
                },
            )
            .await
            .unwrap();
    } else {
        runtime
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
            .unwrap();
    }
    let target = runtime.owned.agent_store.get_agent(&agent).unwrap();
    assert_eq!(target.model(), Some("haiku"));
    assert_eq!(
        target.provider_resume_state().provider_session_id("claude"),
        None,
        "a failed source compact must not resume the oversized transcript on Haiku"
    );
    assert!(
        root.path().join("started").exists(),
        "exercise the actual native utility failure"
    );
}

// MP-08 / MP-10 / MP-11: real launches keep selection default, not the resolved id.
#[tokio::test]
async fn r5_review_implicit_default_downshift_preserves_the_compacted_native_session() {
    let (root, _app, runtime, session, agent, mut run) = compact_fixture().await;
    run.set_provider_session_id(Some("implicit-default-native".into()));
    runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run.clone());
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent,
            "claude",
            Some(run.model().into()),
            None,
            Some("default".into()),
            ProviderResumeState::from_claude_session_id("implicit-default-native"),
        )
        .unwrap();
    std::fs::write(root.path().join("release"), "").unwrap();
    let updated = runtime
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
        .unwrap();
    assert!(
        root.path().join("started").exists(),
        "default-selected source must compact"
    );
    assert_eq!(
        updated
            .provider_resume_state()
            .provider_session_id("claude"),
        Some("implicit-default-native")
    );
}

#[tokio::test]
async fn r5_review_implicit_default_cold_downshift_records_the_recovery_boundary() {
    let (_root, _app, runtime, session, agent, run) = compact_fixture().await;
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent,
            "claude",
            Some(run.model().into()),
            None,
            Some("default".into()),
            ProviderResumeState::from_claude_session_id("implicit-default-native"),
        )
        .unwrap();
    let snapshot = runtime.owned.agent_store.get_agent(&agent).unwrap();
    runtime
        .compact_before_window_downshift(
            &session,
            &snapshot,
            Some("claude"),
            Some(run.account_profile()),
            Some("haiku"),
        )
        .await
        .unwrap();
    let recovered = runtime.owned.agent_store.get_agent(&agent).unwrap();
    assert!(
        recovered
            .provider_resume_state()
            .provider_session_id("claude")
            .is_none(),
        "cold default source must use protected recovery"
    );
    assert!(runtime
        .owned
        .operational_history_store
        .load_agent_compaction_boundary(
            &session,
            &agent,
            "claude",
            run.account_profile(),
            Some("implicit-default-native"),
        )
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn r5_review_cold_default_without_a_source_binding_does_not_assume_resume_ownership() {
    let (_root, _app, runtime, session, agent, run) = compact_fixture().await;
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&session, run.id())
        .unwrap();
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent,
            "claude",
            Some("haiku".into()),
            None,
            Some("default".into()),
            ProviderResumeState::from_claude_session_id("unbound-cold-native"),
        )
        .unwrap();
    let updated = runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some("sonnet".into()),
            None,
        )
        .await
        .unwrap();
    assert!(
        updated
            .provider_resume_state()
            .provider_session_id("claude")
            .is_none(),
        "the current default is not evidence of an unbound native session's source account"
    );
}

#[tokio::test]
async fn explicit_account_pin_keeps_the_authority_bound_run_when_default_moves() {
    let (_root, _app, runtime, session, agent, mut run) = compact_fixture().await;
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let account = run.account_profile().to_string();
    let mut request =
        LaunchProviderRequest::new(&session, "claude", "claude", &account, run.model())
            .with_agent_id(&agent)
            .with_owner_user_id(owner);
    crate::account_profile::bind_provider_account_authority(
        &mut request,
        owner.into(),
        "default".into(),
    );
    run.resolved_provider_account = request.resolved_provider_account;
    run.set_provider_session_id(None);
    assert!(
        run.resume_state().provider_session_id("claude").is_none(),
        "exercise bound selection without matching native-session metadata"
    );
    runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run.clone());
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent,
            "claude",
            Some(run.model().into()),
            None,
            Some("default".into()),
            ProviderResumeState::from_claude_session_id("bound-default-pin-native"),
        )
        .unwrap();
    let unchanged = runtime
        .update_agent_profile(&session, &agent, owner, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        unchanged.provider_account_profile(),
        "default",
        "an omitted account must not pin a true no-op"
    );
    let updated = runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            None,
            Some(account.clone()),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        updated.provider_account_profile(),
        account,
        "explicit account pin was treated as a no-op"
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run_for_agent(&session, &agent)
            .unwrap()
            .id(),
        run.id(),
        "a selection-only pin ended the same-account provider run"
    );
    assert_eq!(
        updated
            .provider_resume_state()
            .provider_session_id("claude"),
        Some("bound-default-pin-native")
    );
    let other = runtime
        .owned
        .provider_account_profiles
        .create_managed(owner, "claude", "new-default")
        .unwrap();
    runtime
        .owned
        .provider_account_profiles
        .set_default(owner, "claude", &other.profile_id)
        .unwrap();
    let persisted = runtime.owned.agent_store.get_agent(&agent).unwrap();
    assert_eq!(
        runtime
            .owned
            .provider_account_profiles
            .get(owner, "claude", persisted.provider_account_profile())
            .unwrap()
            .profile_id,
        account
    );
}
