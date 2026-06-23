use super::provider_output_runtime::provider_run_ids_for_owned_output_pump;
use super::*;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
    let app_locked = app.lock().await;
    KernelRuntimeState::new_with_owned_state(
        Arc::clone(app),
        app_locked.config_projection_store(),
        app_locked.session_state_store(),
        app_locked.agents().clone(),
        app_locked.attachments().clone(),
        app_locked.providers().clone(),
        app_locked.provider_process_tracking_store(),
        app_locked.slices(),
        app_locked.session_state_projection_store(),
        app_locked.provider_run_projection_store(),
        app_locked.history_store(),
        app_locked.operational_history_store(),
        app_locked.durable_state_store(),
        app_locked.session_history_projection_store(),
        app_locked.prompt_state_owner(),
        app_locked.active_turn_store(),
        app_locked.prompt_activity_store(),
        app_locked.prompt_workspace_claim_store(),
        app_locked.structured_output_record_store(),
        app_locked.terminal_stream_store(),
        app_locked.workflow_design_event_store(),
        app_locked.metaagent_event_store(),
        app_locked.workspace_coordinator(),
    )
}

fn sync_external_active_prompt_and_queue_arroba_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    agent_id: &str,
) -> (String, String) {
    let external_prompt_id = format!("external:claude:test-session:{agent_id}:user-1");
    let external_prompt = crate::session::PromptQueueItem::new(
        external_prompt_id.clone(),
        "external:claude",
        agent_id,
        "external prompt in progress",
        crate::session::PromptStatus::Running,
    )
    .with_prompt_origin(crate::session::PromptOrigin::External);
    app.prompt_owner_sync_external_active_prompt(session_id, agent_id, Some(external_prompt))
        .expect("external active prompt should sync");

    let queued_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment_id,
        agent_id,
        "queued from Arroba\n",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
        .prompt_owner_submit_prepared_prompt(session_id, queued_prompt, false)
        .expect("Arroba prompt should queue behind external active prompt")
    else {
        panic!("Arroba prompt must not start while external prompt is active");
    };
    (external_prompt_id, prompt.id().to_string())
}

fn assert_external_active_prompt_and_queued_arroba_prompt(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    external_prompt_id: &str,
    queued_prompt_id: &str,
) {
    let session_state = runtime
        .owned
        .session_snapshot(session_id)
        .expect("session snapshot should exist");
    let active_prompt = session_state
        .active_prompt_for_agent(agent_id)
        .expect("external prompt should remain active");
    assert_eq!(active_prompt.id(), external_prompt_id);
    assert_eq!(
        active_prompt.prompt_origin(),
        crate::session::PromptOrigin::External
    );
    let queued_prompts = session_state
        .queued_prompts_for_agent(agent_id)
        .expect("queued prompts should be mirrored");
    assert!(
        queued_prompts
            .iter()
            .any(|prompt| prompt.id() == queued_prompt_id),
        "Arroba prompt should stay queued behind external active prompt"
    );
}

#[tokio::test]
async fn provider_output_pump_ignores_projected_remote_active_run() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    app.sessions
        .set_active_provider_run(
            session.id(),
            Some("remote-projected-provider-run-1".to_string()),
        )
        .expect("active provider run should be recorded");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    let provider_run_ids = provider_run_ids_for_owned_output_pump(&runtime.owned, &session_state);

    assert!(
        provider_run_ids.is_empty(),
        "projected remote provider runs are drained through remote projection, not local PTY output"
    );
}

#[tokio::test]
async fn provider_switch_does_not_park_runs_with_active_prompts() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("second"),
        )
        .expect("second agent should spawn");
    let idle_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("idle"),
        )
        .expect("idle agent should spawn");

    let first_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect("first provider should launch");
    app.update_provider_run_projection(first_run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(first_agent.id()),
        "first prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");

    let second_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(second_agent.id()),
        )
        .expect("second provider should launch");
    app.update_provider_run_projection(second_run.clone());

    assert_eq!(
        app.providers
            .get_run(first_run.id())
            .expect("first run should exist")
            .state(),
        crate::provider::ProviderRunState::Running,
        "launching another provider must not park a run that owns an active prompt",
    );

    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(second_agent.id()),
        "second prompt\n",
        Vec::new(),
    )
    .expect("second prompt should start");
    crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), idle_agent.id())
        .expect("idle agent focus should succeed");

    assert_eq!(
        app.providers
            .get_run(second_run.id())
            .expect("second run should exist")
            .state(),
        crate::provider::ProviderRunState::Running,
        "focusing an idle agent while multiple prompts are active must not park active work",
    );
    assert_eq!(
        app.sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(second_run.id()),
        "ambiguous multi-agent prompt work should keep the active provider pointer stable",
    );
}

#[tokio::test]
async fn provider_output_pump_includes_unfocused_active_prompt_runs() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-output-pump",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("second"),
        )
        .expect("second agent should spawn");
    let idle_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("idle"),
        )
        .expect("idle agent should spawn");

    let first_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect("first provider should launch");
    app.update_provider_run_projection(first_run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(first_agent.id()),
        "first prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");

    let second_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(second_agent.id()),
        )
        .expect("second provider should launch");
    app.update_provider_run_projection(second_run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(second_agent.id()),
        "second prompt\n",
        Vec::new(),
    )
    .expect("second prompt should start");

    let stale_idle_run = app
        .providers
        .start_run_provider_only(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(idle_agent.id()),
        )
        .expect("stale idle provider should start without becoming active")
        .into_run();
    app.update_provider_run_projection(stale_idle_run.clone());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    let provider_run_ids = provider_run_ids_for_owned_output_pump(&runtime.owned, &session_state);

    assert!(
        provider_run_ids.contains(first_run.id()),
        "output pump must keep draining an unfocused agent with active prompt"
    );
    assert!(
        provider_run_ids.contains(second_run.id()),
        "output pump must include the focused active prompt run"
    );
    assert!(
        !provider_run_ids.contains(stale_idle_run.id()),
        "stale Arroba runs without active prompts must not be pumped as background runs"
    );
}

#[tokio::test]
async fn provider_output_pump_includes_runs_with_pending_git_snapshots() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");

    let provider_run = app
        .providers
        .start_run_provider_only(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should start without becoming active")
        .into_run();
    app.update_provider_run_projection(provider_run.clone());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .git_turn_snapshots
        .insert(crate::git_observer::GitTurnSnapshot {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider: "dev-stub".to_string(),
            model: "sonnet".to_string(),
            provider_run_id: provider_run.id().to_string(),
            provider_session_id: None,
            prompt_id: "prompt-1".to_string(),
            turn_id: "prompt-1".to_string(),
            started_at_ms: None,
            machine_id: None,
            prompt_summary: "test prompt".to_string(),
            repo_root: "/tmp/repo".to_string(),
            worktree_path: "/tmp/repo".to_string(),
            branch: Some("main".to_string()),
            head_sha: Some("head".to_string()),
            upstream_ref: None,
            ahead_count: None,
            status_fingerprint: String::new(),
            is_dirty: false,
            workspace_live_sync_tracked: true,
            workspace_live_sync_file_snapshots: Default::default(),
        });

    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    let provider_run_ids = provider_run_ids_for_owned_output_pump(&runtime.owned, &session_state);

    assert!(
        provider_run_ids.contains(provider_run.id()),
        "output pump must drain runs that still have pending git/WLS snapshots"
    );
}

#[tokio::test]
async fn provider_completed_signal_settles_matching_active_prompt() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let first_settlement = runtime
        .settle_owned_provider_prompt(session.id(), run.id(), true, false, false)
        .await
        .expect("provider completion signal should be accepted");
    assert!(first_settlement.had_active_prompt);
    assert!(!first_settlement.started_next_prompt);
    assert!(runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .is_none());
}

#[tokio::test]
async fn provider_completion_signal_preserves_external_active_prompt_and_queue() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-external-settlement",
            "worktree-external-settlement",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-settlement",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_arroba_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let settlement = runtime
        .settle_owned_provider_prompt(session.id(), run.id(), true, false, false)
        .await
        .expect("provider completion signal should be accepted");
    assert!(!settlement.had_active_prompt);
    assert!(!settlement.started_next_prompt);
    assert_external_active_prompt_and_queued_arroba_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );
}

#[tokio::test]
async fn provider_terminal_failure_preserves_external_active_prompt_and_queue() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-external-terminal-failure",
            "worktree-external-terminal-failure",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-terminal-failure",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.3-codex-spark",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-external-terminal-failure",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_arroba_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                terminal_failure: Some("external provider stderr".to_string()),
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("terminal failure batch should be accepted");
    assert_external_active_prompt_and_queued_arroba_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );
}

#[tokio::test]
async fn provider_completion_with_output_settles_after_fanning_out_records() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-completion-drain",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("assistant-final".to_string()),
                    bytes: b"final output".to_vec(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "assistant-final".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("completion batch with output should be accepted");
    assert_eq!(records.len(), 1);

    let settled_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        settled_session
            .active_prompt_for_agent(agent.id())
            .is_none(),
        "final output is fanned out before settlement, so a completed structured prompt must not wait for another poll"
    );
}

#[tokio::test]
async fn provider_quiet_gap_does_not_settle_without_completion_signal() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-quiet-gap",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "long quiet turn\n",
        Vec::new(),
    )
    .expect("prompt should start");

    if let Some(state) = app.prompt_activity.write().get_mut(run.id()) {
        state.saw_response_content = true;
        state.last_output_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(5));
    } else {
        panic!("prompt activity should exist for the active run");
    }

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let settlement = runtime
        .settle_owned_provider_prompt(session.id(), run.id(), false, false, false)
        .await
        .expect("quiet provider poll should be accepted");
    assert!(settlement.had_active_prompt);
    assert!(!settlement.started_next_prompt);

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(session_state.active_prompt_for_agent(agent.id()).is_some());
    let activity = runtime.agent_activity_for_session(&session_state);
    let agent_activity = activity
        .get(agent.id())
        .expect("agent activity should be projected");
    assert_eq!(
        agent_activity.status,
        crate::runtime::projection::AgentRuntimeStatus::Working
    );
    assert_eq!(
        agent_activity.prompt_status,
        crate::runtime::projection::AgentPromptRuntimeStatus::Running
    );
    assert!(agent_activity.busy);
    assert!(agent_activity.active_turn.is_some());
}

#[tokio::test]
async fn workflow_prompt_settles_after_structured_message_completion_drain() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("second"),
        )
        .expect("second agent should spawn");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());

    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("completion-gate".to_string()))
        .expect("workflow should be created");
    let first_node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), first_agent.id())
        .expect("first node should be added");
    let second_node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), second_agent.id())
        .expect("second node should be added");
    app.sessions_mut()
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            first_node.id(),
            second_node.id(),
            None,
            None,
        )
        .expect("edge should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            first_node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run the workflow".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow node prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node run should start");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        first_agent.id(),
        "workflow node prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("workflow prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("assistant-final".to_string()),
                    bytes: br#"```json
{"summary":"sent","output":{"message":"{\"value\":1842}"}}
```"#
                        .to_vec(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "assistant-final".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: false,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured output should be accepted");
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("quiet poll should settle a drained completed workflow prompt");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state
            .active_prompt_for_agent(first_agent.id())
            .is_none(),
        "workflow prompt must settle after assistant completion drains"
    );
    let resolved_run = session_state
        .workflow_run(workflow_run.id())
        .expect("workflow run should exist");
    assert_eq!(resolved_run.node_runs().len(), 2);
    assert_eq!(
        resolved_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Completed
    );
    assert_eq!(
        resolved_run.node_runs()[1].status(),
        crate::session::WorkflowNodeRunStatus::Ready
    );
}

#[tokio::test]
async fn workflow_reasoning_records_thinking_from_prompt_owner_context() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());

    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("thinking-trace".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run the workflow".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow node prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node run should start");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "workflow node prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("workflow prompt should start");
    app.sessions
        .mirror_agent_prompt_state(session.id(), agent.id(), None, VecDeque::new())
        .expect("session mirror should be cleared for regression coverage");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderReasoning,
                    merge_key: Some("thinking-1".to_string()),
                    bytes: b"real provider reasoning".to_vec(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured reasoning should be accepted");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    let node_run = session_state
        .workflow_run(workflow_run.id())
        .and_then(|run| {
            run.node_runs()
                .iter()
                .find(|node_run| node_run.id() == node_run_id)
        })
        .expect("workflow node run should exist");
    assert_eq!(node_run.thinking_traces().len(), 1);
    assert_eq!(
        node_run.thinking_traces()[0].message(),
        "real provider reasoning"
    );
}

#[tokio::test]
async fn codex_tool_output_text_does_not_classify_as_terminal_failure() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-tool-output",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-codex",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "read tool output\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderTool,
                    merge_key: Some("tool-call-1".to_string()),
                    bytes: br#"{"tool":"bash","status":"completed","output":"Check if a CLAUDE.md file exists in the project root. If it does not exist, create it. This text mentions model context but is normal tool output."}"#.to_vec(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured tool output should be accepted");

    assert_eq!(records.len(), 1);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_some(),
        "normal Codex tool output must not settle the active prompt"
    );
    let agent_activity = runtime
        .agent_activity_for_session(&session_state)
        .get(agent.id())
        .cloned()
        .expect("agent activity should be projected");
    assert!(agent_activity.busy);
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .terminal_diagnostic(),
        None
    );
}

#[tokio::test]
async fn structured_terminal_failure_records_single_clean_notice() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-provider-error",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.3-codex-spark",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-codex-error",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "trigger provider error\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let raw_error = r#"{
  "type": "error",
  "error": {
    "type": "invalid_request_error",
    "code": "unsupported_parameter",
    "message": "Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model.",
    "param": "reasoning.summary"
  },
  "status": 400
}"#;
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                notices: vec![raw_error.to_string(), raw_error.to_string()],
                terminal_failure: Some(raw_error.to_string()),
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured terminal failure should be accepted");

    let notices = runtime
        .owned
        .terminal_stream
        .drain_notice_records(session.id(), attachment.id());
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].message,
        "Provider prompt dispatch failed: Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model."
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .terminal_diagnostic(),
        Some("Provider prompt dispatch failed: Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model.")
    );
}

#[tokio::test]
async fn first_output_timeout_records_diagnostic_and_closes_prompt() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-first-output-timeout",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-first-output-timeout",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex-timeout".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-timeout-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start but never answer\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    let prompt_id = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .expect("active prompt should exist")
        .id()
        .to_string();
    let mut timed_out_turn = crate::app::ActiveTurnState::new(
        session.id().to_string(),
        agent.id().to_string(),
        prompt_id,
        run.id().to_string(),
    )
    .with_phase(crate::app::ActiveTurnPhase::AwaitingFirstOutput);
    timed_out_turn.started_at_ms = crate::session::unix_epoch_ms().saturating_sub(11 * 60 * 1000);
    app.active_turn_store().start(timed_out_turn);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap silent timeout");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "silent provider timeout must close the active prompt"
    );
    let run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should still exist");
    assert!(run
        .terminal_diagnostic()
        .expect("timeout diagnostic should be recorded")
        .contains("Provider prompt produced no output"));
    let notices = runtime
        .owned
        .terminal_stream
        .drain_notice_records(session.id(), attachment.id());
    assert!(
        notices.iter().any(|record| record
            .message
            .contains("Provider prompt produced no output")),
        "timeout diagnostic should be visible to attached clients"
    );
}

#[tokio::test]
async fn provider_inactivity_timeout_records_diagnostic_and_closes_prompt() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-inactivity-timeout",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "zen",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-inactivity-timeout",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-timeout".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start, emit a tool, then stall\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    crate::transport::flow_control::note_prompt_response_content(&mut app, run.id());
    app.active_turns.mark_streaming(run.id());
    if let Some(state) = app.prompt_activity.write().get_mut(run.id()) {
        state.last_output_at =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(11 * 60));
        state.saw_response_content = true;
    } else {
        panic!("prompt activity should exist for the active run");
    }

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap inactive provider turn");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "inactive provider timeout must close the active prompt"
    );
    let run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should still exist");
    assert!(run
        .terminal_diagnostic()
        .expect("timeout diagnostic should be recorded")
        .contains("Provider prompt produced no output"));
    let notices = runtime
        .owned
        .terminal_stream
        .drain_notice_records(session.id(), attachment.id());
    assert!(
        notices
            .iter()
            .any(|record| record.message.contains("after its last activity")),
        "inactivity timeout diagnostic should be visible to attached clients"
    );
}

#[tokio::test]
async fn metaagent_receives_required_failed_turn_event_on_provider_timeout() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-meta-failure",
            "worktree-meta-failure",
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-meta-failure",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let worker_request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut worker_run = crate::provider::RuntimeProviderRun::new(
        "provider-run-meta-failure-worker",
        &worker_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-meta-failure-worker".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-meta-failure-worker-runtime".to_string()),
        },
    );
    worker_run.mark_running();
    app.providers_mut().insert_run_for_test(worker_run.clone());
    app.update_provider_run_projection(worker_run.clone());

    let meta_request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(metaagent.id());
    let mut meta_run = crate::provider::RuntimeProviderRun::new(
        "provider-run-meta-failure-meta",
        &meta_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-meta-failure-meta".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-meta-failure-meta-runtime".to_string()),
        },
    );
    meta_run.mark_running();
    app.providers_mut().insert_run_for_test(meta_run.clone());
    app.update_provider_run_projection(meta_run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start but never answer\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, worker_run.id());
    let prompt_id = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .expect("active prompt should exist")
        .id()
        .to_string();
    let mut timed_out_turn = crate::app::ActiveTurnState::new(
        session.id().to_string(),
        agent.id().to_string(),
        prompt_id,
        worker_run.id().to_string(),
    )
    .with_phase(crate::app::ActiveTurnPhase::AwaitingFirstOutput);
    timed_out_turn.started_at_ms = crate::session::unix_epoch_ms().saturating_sub(11 * 60 * 1000);
    app.active_turn_store().start(timed_out_turn);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            worker_run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap silent timeout");

    let events =
        runtime
            .owned
            .metaagent_events
            .list(metaagent.id(), Some("agent.turn.failed"), None, 10);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source_agent_id.as_deref(), Some(agent.id()));
    assert!(events[0]
        .summary
        .contains("Provider prompt produced no output"));
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state
            .active_prompt_for_agent(metaagent.id())
            .is_some(),
        "failed-turn event should start an inline metaagent prompt when the metaagent is idle"
    );
}
