use super::*;
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
        app_locked.workspace_coordinator(),
    )
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
async fn provider_completion_with_output_waits_for_quiet_poll_before_settling() {
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

    let draining_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        draining_session
            .active_prompt_for_agent(agent.id())
            .is_some(),
        "final output and completion in the same batch should keep the turn settling"
    );
    let draining_activity = runtime
        .agent_activity_for_session(&draining_session)
        .get(agent.id())
        .cloned()
        .expect("agent activity should be projected");
    assert!(draining_activity.busy);
    assert_eq!(
        draining_activity.prompt_status,
        crate::runtime::projection::AgentPromptRuntimeStatus::Settling
    );

    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch::default(),
        )
        .await
        .expect("quiet poll should settle the completed prompt");
    let settled_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(settled_session
        .active_prompt_for_agent(agent.id())
        .is_none());
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
