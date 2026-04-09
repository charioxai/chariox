use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use arroba_daemon::agent::CreateAgentRequest;
use arroba_daemon::attachment::{AttachRequest, ClientCapabilityLevel};
use arroba_daemon::local::{
    AttachToSessionRequest, EndSessionRequest, LaunchProviderRunRequest, LocalDaemonRequest,
    LocalDaemonResponse, PumpTerminalOutputRequest, SubmitPromptRequest,
    UpdateSessionConfigRequest,
};
use arroba_daemon::provider::{LaunchProviderRequest, ProviderRunState};
use arroba_daemon::session::{
    CreateSessionRequest, PromptStatus, PromptSubmissionOutcome, SessionStatus,
    WorkflowNodeRunStatus, WorkflowRunStatus,
};
use arroba_daemon::terminal::TerminalOutputKind;
use arroba_daemon::{DaemonApp, DaemonConfig};
use serde_json::{json, Value};

static OPENCODE_ENV_LOCK: Mutex<()> = Mutex::new(());

fn opencode_env_guard() -> std::sync::MutexGuard<'static, ()> {
    OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

#[test]
fn session_lifecycle_round_trip_cleans_runtime_state() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    let ended = app
        .end_session(session.id())
        .expect("session should end cleanly");

    assert_eq!(ended.status(), SessionStatus::Ended);
    assert!(app.attachments().get_attachment(attachment.id()).is_err());
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("terminated run should remain queryable")
            .state(),
        ProviderRunState::Ended
    );
    assert!(!app.pty().has_process(run.id()));
}

#[test]
fn attachments_can_queue_prompts_and_receive_queue_notifications() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let first = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("first attachment should attach");
    let second = app
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("second attachment should attach");

    let _run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    let first_outcome = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        first.id(),
        "first integration prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let second_outcome = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        second.id(),
        "second integration prompt\n",
        Vec::new(),
    )
    .expect("second prompt should queue");

    match first_outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        _ => panic!("expected first prompt to start"),
    }
    match second_outcome {
        PromptSubmissionOutcome::Queued { .. } => {}
        _ => panic!("expected second prompt to queue"),
    }

    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(session.queued_prompts().len(), 1);
    assert_eq!(app.terminal().notice_records().len(), 1);
    assert!(app.terminal().notice_records()[0]
        .recipient_attachment_ids
        .contains(&first.id().to_string()));
}

#[test]
fn detaching_attachment_removes_its_queued_prompts_before_advancement() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let first = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("first attachment should attach");
    let second = app
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("second attachment should attach");

    let _run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        first.id(),
        "first integration prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        second.id(),
        "second integration prompt\n",
        Vec::new(),
    )
    .expect("second prompt should queue");

    app.detach(second.id())
        .expect("queued prompt source should detach cleanly");
    let completion =
        arroba_daemon::transport::TransportService::complete_active_prompt(&mut app, session.id())
            .expect("active prompt should complete");

    assert!(completion.started_next.is_none());
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(session.active_prompt().is_none());
    assert!(session.queued_prompts().is_empty());
}

#[test]
fn completing_a_prompt_without_provider_completion_still_emits_a_terminal_completion_record() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "fallback completion\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let completion =
        arroba_daemon::transport::TransportService::complete_active_prompt(&mut app, session.id())
            .expect("active prompt should complete");

    let mut terminal = app.terminal().clone();
    let records = terminal.drain_completion_records(session.id(), attachment.id());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider_run_id, run.id());
    assert_eq!(
        records[0].message_id,
        format!("prompt-complete:{}", completion.completed.id())
    );
}

#[test]
fn workflow_runs_progress_without_terminal_pumps() {
    let _guard = opencode_env_guard();
    env::set_var("ARROBA_PROMPT_IDLE_MS", "1");

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let _attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let agent = app
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("node-a"))
        .expect("agent should spawn");
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("workflow-pump-test".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be created");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let (run, _workflow, _endpoint) = app
        .invoke_workflow_endpoint_and_schedule(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("kickoff".to_string()),
        )
        .expect("workflow run should start");

    let run_id = run.id().to_string();
    let start = Instant::now();
    loop {
        app.pump_active_prompt_outputs();
        let run = app
            .sessions()
            .resolve_workflow_run_ref(session.id(), &run_id)
            .expect("workflow run should exist");
        if !matches!(
            run.status(),
            WorkflowRunStatus::Running | WorkflowRunStatus::Waiting
        ) {
            break;
        }
        if start.elapsed() > Duration::from_secs(2) {
            panic!("workflow run never completed");
        }
        thread::sleep(Duration::from_millis(5));
    }

    let run = app
        .sessions()
        .resolve_workflow_run_ref(session.id(), &run_id)
        .expect("workflow run should exist");
    assert!(
        matches!(
            run.status(),
            WorkflowRunStatus::Completed | WorkflowRunStatus::Stopped
        ),
        "workflow run should complete or stop"
    );
    assert!(
        run.node_runs().iter().all(|node_run| matches!(
            node_run.status(),
            WorkflowNodeRunStatus::Completed
                | WorkflowNodeRunStatus::Failed
                | WorkflowNodeRunStatus::Stopped
        )),
        "workflow node run should settle"
    );
}

#[test]
fn provider_run_switching_parks_previous_run_and_keeps_terminal_flow_working() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let source = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let first_run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("first provider run should launch");
    let second_run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "opus",
        ))
        .expect("second provider run should launch");

    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(session.active_provider_run_id(), Some(second_run.id()));
    assert_eq!(
        app.providers()
            .get_run(first_run.id())
            .expect("first run should still exist")
            .state(),
        ProviderRunState::Parked
    );
    assert_eq!(
        app.providers()
            .get_run(second_run.id())
            .expect("second run should still exist")
            .state(),
        ProviderRunState::Running
    );

    app.send_terminal_input(session.id(), source.id(), b"switched run\n")
        .expect("attachment input should reach active provider run");
    let records = wait_for_terminal_output(&mut app, session.id(), source.id());
    let combined = records
        .into_iter()
        .flat_map(|record| record.bytes)
        .collect::<Vec<u8>>();
    assert!(String::from_utf8_lossy(&combined).contains("switched run"));
}

#[test]
fn local_request_surface_supports_prompt_queue_and_config_updates() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");

    let session = match app
        .handle_local_request(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-integration", "worktree-integration"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };

    let first = match app
        .handle_local_request(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "first".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let second = match app
        .handle_local_request(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "second".to_string(),
                capability_level: ClientCapabilityLevel::InteractiveStructured,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let _provider_run = match app
        .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: None,
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider launch should succeed")
    {
        LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
        _ => panic!("unexpected local response"),
    };

    let first_prompt = app
        .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: first.id().to_string(),
            target_agent_id: None,
            prompt: "first local prompt\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("first prompt should start");
    let second_prompt = app
        .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: second.id().to_string(),
            target_agent_id: None,
            prompt: "second local prompt\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("second prompt should queue");
    let config = app
        .handle_local_request(LocalDaemonRequest::UpdateSessionConfig(
            UpdateSessionConfigRequest {
                session_id: session.id().to_string(),
                attachment_id: first.id().to_string(),
                values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                requires_idle: false,
            },
        ))
        .expect("config update should succeed");
    let echoed_output = wait_for_local_terminal_output(&mut app, session.id(), second.id());
    let ended = app
        .handle_local_request(LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session.id().to_string(),
        }))
        .expect("end session should succeed");

    match first_prompt {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { .. },
            ..
        } => {}
        _ => panic!("unexpected first prompt response"),
    }
    match second_prompt {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Queued { .. },
            ..
        } => {}
        _ => panic!("unexpected second prompt response"),
    }
    match config {
        LocalDaemonResponse::SessionConfigUpdated { config, .. } => {
            assert_eq!(config.version(), 1)
        }
        _ => panic!("unexpected config response"),
    }
    assert!(echoed_output.contains("first local prompt"));
    match ended {
        LocalDaemonResponse::SessionEnded { session } => {
            assert_eq!(session.status(), SessionStatus::Ended)
        }
        _ => panic!("unexpected end response"),
    }
}

#[test]
fn prompt_queue_advances_after_provider_output_goes_idle() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let _run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first auto prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "second auto prompt\n",
        Vec::new(),
    )
    .expect("second prompt should queue");

    let combined = collect_terminal_output_until(
        &mut app,
        session.id(),
        attachment.id(),
        |output, session| {
            output.contains("first auto prompt")
                && output.contains("second auto prompt")
                && session.active_prompt().is_none()
                && session.queued_prompts().is_empty()
        },
    );

    assert!(combined.contains("first auto prompt"));
    assert!(combined.contains("second auto prompt"));
}

#[test]
fn shared_opencode_endpoint_keeps_prompt_queue_running_without_managed_process() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first exit prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "second exit prompt\n",
        Vec::new(),
    )
    .expect("second prompt should queue");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: first exit prompt")
                && output.contains("fixture response: second exit prompt")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
    );

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(output.contains("fixture response: first exit prompt"));
    assert!(output.contains("fixture response: second exit prompt"));
    assert_eq!(session_state.active_provider_run_id(), Some(run.id()));
    assert!(session_state.active_prompt().is_none());
    assert!(session_state.queued_prompts().is_empty());
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should remain queryable")
            .state(),
        ProviderRunState::Running
    );
    assert!(!app.pty().has_process(run.id()));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

#[test]
fn shared_opencode_idle_status_completes_the_prompt_without_a_settle_window() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(100));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "completes without a settle window\n",
        Vec::new(),
    )
    .expect("prompt should start");

    thread::sleep(Duration::from_millis(120));
    let mut output = Vec::new();
    let completion_deadline = Instant::now() + Duration::from_millis(300);
    loop {
        let recipients = app.attachments().list_session_attachment_ids(session.id());
        output.extend(
            app.pump_provider_output(session.id(), run.id(), recipients)
                .expect("pump after OpenCode idle should succeed"),
        );
        let session_after_pump = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist after completion");
        if session_after_pump.active_prompt().is_none() {
            break;
        }
        assert!(
            Instant::now() < completion_deadline,
            "OpenCode idle should complete the active prompt immediately after the provider reaches idle"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        output
            .iter()
            .any(|record| matches!(record.kind, TerminalOutputKind::ProviderOutput)),
        "the completion pump should include the assistant output"
    );
    let terminal_text = app
        .terminal()
        .output_records()
        .iter()
        .filter(|record| record.session_id == session.id())
        .filter_map(|record| String::from_utf8(record.bytes.clone()).ok())
        .collect::<String>();
    assert!(terminal_text.contains("fixture response: completes without a settle window"));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

#[test]
fn end_session_aborts_active_opencode_session_before_cleanup() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let _attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let ended = app
        .end_session(session.id())
        .expect("session should end cleanly");

    assert_eq!(ended.status(), SessionStatus::Ended);
    assert_eq!(mock_server.abort_count(), 1);
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should remain queryable")
            .state(),
        ProviderRunState::Ended
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn session_error_completes_the_active_prompt_and_advances_the_queue() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.fail_next_prompt("fixture prompt failure");
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first prompt should fail\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "second prompt should run\n",
        Vec::new(),
    )
    .expect("second prompt should queue");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: second prompt should run")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .queued_prompts()
                    .is_empty()
        },
    );

    assert!(output.contains("fixture response: second prompt should run"));
    assert!(app
        .terminal()
        .notice_records()
        .iter()
        .any(|record| record.message.contains("fixture prompt failure")));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn cancelling_active_opencode_prompt_waits_for_provider_confirmation_before_advancing_queue() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first prompt should cancel\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "second prompt after cancel\n",
        Vec::new(),
    )
    .expect("second prompt should queue");

    let cancellation = arroba_daemon::transport::TransportService::cancel_active_prompt(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("active prompt should cancel");
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
    assert!(cancellation.started_next.is_none());
    assert_eq!(mock_server.abort_count(), 1);
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
            .active_prompt()
            .expect("active prompt should still exist")
            .status(),
        PromptStatus::Cancelling
    );

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, _app| output.contains("fixture response: second prompt after cancel"),
    );
    assert!(output.contains("fixture response: second prompt after cancel"));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn cancelling_active_opencode_prompt_without_queue_clears_the_active_prompt() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "cancel just this prompt\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let cancellation = arroba_daemon::transport::TransportService::cancel_active_prompt(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("active prompt should cancel");
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
    assert!(cancellation.started_next.is_none());
    assert_eq!(mock_server.abort_count(), 1);

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let _records = collect_provider_records_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |_records, app| {
            app.sessions()
                .get_session(session.id())
                .expect("session should still exist")
                .active_prompt()
                .is_none()
        },
    );
    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist after cancellation");
    assert!(session_state.active_prompt().is_none());
    assert_eq!(
        session_state.scheduler_state(),
        arroba_daemon::session::SchedulerState::Idle
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn event_stream_disconnect_reconnects_without_restarting_the_provider_run() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.disconnect_next_event_stream();
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "prompt after reconnect\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: prompt after reconnect")
                && app
                    .providers()
                    .get_run(run.id())
                    .expect("run should remain queryable")
                    .state()
                    == ProviderRunState::Running
        },
    );

    assert!(output.contains("fixture response: prompt after reconnect"));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn event_stream_reconnect_retries_temporary_http_failures_without_restarting_the_provider_run() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.disconnect_next_event_stream();
    mock_server.fail_next_event_stream_attempts(2);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "prompt after transient reconnect failures\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: prompt after transient reconnect failures")
                && app
                    .providers()
                    .get_run(run.id())
                    .expect("run should remain queryable")
                    .state()
                    == ProviderRunState::Running
        },
    );

    assert!(output.contains("fixture response: prompt after transient reconnect failures"));
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should remain available")
            .state(),
        ProviderRunState::Running
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn external_opencode_endpoint_accepts_prompts_and_streams_output() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    let previous_endpoint = env::var_os("ARROBA_OPENCODE_ENDPOINT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::remove_var("ARROBA_OPENCODE_PORT");
    env::set_var(
        "ARROBA_OPENCODE_ENDPOINT",
        format!("http://127.0.0.1:{}", mock_server.port()),
    );

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch against external endpoint");

    app.resize_terminal(session.id(), 120, 40)
        .expect("external endpoint resize should be a no-op");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "prompt through external endpoint\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: prompt through external endpoint")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should remain available")
                    .active_prompt()
                    .is_none()
        },
    );

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(output.contains("fixture response: prompt through external endpoint"));
    assert!(session_state.active_prompt().is_none());
    assert_eq!(session_state.active_provider_run_id(), Some(run.id()));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    if let Some(previous_endpoint) = previous_endpoint {
        env::set_var("ARROBA_OPENCODE_ENDPOINT", previous_endpoint);
    } else {
        env::remove_var("ARROBA_OPENCODE_ENDPOINT");
    }
    mock_server.stop();
}

#[test]
fn launch_retries_temporary_event_subscription_failures() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.fail_next_event_stream_attempts(2);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch after retrying event subscribe failures");

    assert_eq!(run.state(), ProviderRunState::Running);

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn focused_agent_prompts_route_to_distinct_opencode_runs_and_history() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = app
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(default_agent.id()),
        )
        .expect("default provider run should launch");
    let reviewer = app
        .spawn_agent(
            arroba_daemon::agent::CreateAgentRequest::new(session.id(), "opencode")
                .with_alias("reviewer"),
        )
        .expect("reviewer agent should spawn");
    let reviewer_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(reviewer.id()),
        )
        .expect("reviewer provider run should launch");

    app.focus_agent(session.id(), default_agent.id())
        .expect("default agent should focus");
    let first_submission = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "default agent prompt\n",
        Vec::new(),
    )
    .expect("default prompt should start");
    match first_submission {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), default_agent.id());
        }
        _ => panic!("expected default prompt to start immediately"),
    }

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let default_records = collect_provider_records_until(
        &mut app,
        session.id(),
        default_run.id(),
        recipients.clone(),
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: default agent prompt")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
    );
    assert!(default_records
        .iter()
        .all(|record| record.agent_id.as_deref() == Some(default_agent.id())));

    app.focus_agent(session.id(), reviewer.id())
        .expect("reviewer agent should focus");
    let second_submission = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "review agent prompt\n",
        Vec::new(),
    )
    .expect("review prompt should start");
    match second_submission {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), reviewer.id());
        }
        _ => panic!("expected review prompt to start immediately"),
    }

    let reviewer_records = collect_provider_records_until(
        &mut app,
        session.id(),
        reviewer_run.id(),
        recipients,
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: review agent prompt")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
    );
    assert!(reviewer_records
        .iter()
        .all(|record| record.agent_id.as_deref() == Some(reviewer.id())));

    let default_history = app
        .session_history_page(
            session.id(),
            Some(default_agent.id()),
            None,
            None,
            None,
            None,
        )
        .expect("default history should load");
    let reviewer_history = app
        .session_history_page(session.id(), Some(reviewer.id()), None, None, None, None)
        .expect("reviewer history should load");

    let default_text = default_history
        .entries
        .iter()
        .map(|entry| entry.entry.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let reviewer_text = reviewer_history
        .entries
        .iter()
        .map(|entry| entry.entry.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(default_text.contains("default agent prompt"));
    assert!(default_text.contains("fixture response: default agent prompt"));
    assert!(!default_text.contains("review agent prompt"));
    assert!(reviewer_text.contains("review agent prompt"));
    assert!(reviewer_text.contains("fixture response: review agent prompt"));
    assert!(!reviewer_text.contains("default agent prompt"));

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(session_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(reviewer_run.id())
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn focusing_another_agent_during_an_opencode_prompt_keeps_the_working_run_active() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(150));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = app
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(default_agent.id()),
        )
        .expect("default provider run should launch");
    let reviewer = app
        .spawn_agent(
            arroba_daemon::agent::CreateAgentRequest::new(session.id(), "opencode")
                .with_alias("reviewer"),
        )
        .expect("reviewer agent should spawn");
    let reviewer_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(reviewer.id()),
        )
        .expect("reviewer provider run should launch");

    app.focus_agent(session.id(), default_agent.id())
        .expect("default agent should focus");
    let started = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "keep streaming while focus changes\n",
        Vec::new(),
    )
    .expect("prompt should start");
    match started {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), default_agent.id());
        }
        _ => panic!("expected prompt to start immediately"),
    }

    app.focus_agent(session.id(), reviewer.id())
        .expect("reviewer agent should focus");

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(session_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(reviewer_run.id())
    );

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let default_records = collect_provider_records_until(
        &mut app,
        session.id(),
        default_run.id(),
        recipients,
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: keep streaming while focus changes")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
    );

    assert!(default_records
        .iter()
        .any(|record| record.agent_id.as_deref() == Some(default_agent.id())));

    let settled_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(settled_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        settled_state.active_provider_run_id(),
        Some(reviewer_run.id())
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn prompt_for_another_agent_starts_on_its_own_run_without_switching_focus_selection() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(150));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = app
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(default_agent.id()),
        )
        .expect("default provider run should launch");
    let reviewer = app
        .spawn_agent(
            arroba_daemon::agent::CreateAgentRequest::new(session.id(), "opencode")
                .with_alias("reviewer"),
        )
        .expect("reviewer agent should spawn");
    let reviewer_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(reviewer.id()),
        )
        .expect("reviewer provider run should launch");

    app.focus_agent(session.id(), default_agent.id())
        .expect("default agent should focus");
    let first_submission = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "default agent prompt stays active\n",
        Vec::new(),
    )
    .expect("default prompt should start");
    match first_submission {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), default_agent.id());
        }
        _ => panic!("expected default prompt to start immediately"),
    }

    app.focus_agent(session.id(), reviewer.id())
        .expect("reviewer agent should focus while default agent is running");
    let second_submission = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "reviewer prompt should queue\n",
        Vec::new(),
    )
    .expect("reviewer prompt should start");
    match second_submission {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), reviewer.id());
        }
        _ => panic!("expected reviewer prompt to start immediately"),
    }

    let queued_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(queued_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        queued_state.active_provider_run_id(),
        Some(reviewer_run.id())
    );
    assert_eq!(
        queued_state
            .active_prompt_for_agent(default_agent.id())
            .expect("default prompt should still be active")
            .target_agent_id(),
        default_agent.id()
    );
    assert_eq!(
        queued_state
            .active_prompt_for_agent(reviewer.id())
            .expect("reviewer prompt should also be active")
            .target_agent_id(),
        reviewer.id()
    );
    assert!(queued_state
        .queued_prompts_for_agent(reviewer.id())
        .is_none_or(|queue| queue.is_empty()));

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let default_records = collect_provider_records_until(
        &mut app,
        session.id(),
        default_run.id(),
        recipients.clone(),
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: default agent prompt stays active")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt_for_agent(default_agent.id())
                    .is_none()
        },
    );
    assert!(default_records
        .iter()
        .all(|record| record.agent_id.as_deref() == Some(default_agent.id())));

    let reviewer_records = collect_provider_records_until(
        &mut app,
        session.id(),
        reviewer_run.id(),
        recipients,
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: reviewer prompt should queue")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt_for_agent(reviewer.id())
                    .is_none()
        },
    );
    assert!(reviewer_records
        .iter()
        .all(|record| record.agent_id.as_deref() == Some(reviewer.id())));

    let settled_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(settled_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        settled_state.active_provider_run_id(),
        Some(reviewer_run.id())
    );
    assert!(settled_state
        .queued_prompts_for_agent(default_agent.id())
        .is_none_or(|queue| queue.is_empty()));
    assert!(settled_state
        .queued_prompts_for_agent(reviewer.id())
        .is_none_or(|queue| queue.is_empty()));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn detaching_the_last_attachment_keeps_an_active_turn_available_on_rejoin() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(150));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let first = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        first.id(),
        "prompt survives detach\n",
        Vec::new(),
    )
    .expect("prompt should start");

    app.detach(first.id())
        .expect("detaching the only attachment should succeed");

    let detached_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(detached_state.attachment_ids().is_empty());
    assert_eq!(
        detached_state.active_prompt().map(|prompt| prompt.status()),
        Some(PromptStatus::Running)
    );
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("provider run should remain queryable")
            .state(),
        ProviderRunState::Running
    );

    thread::sleep(Duration::from_millis(75));

    let second = app
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("reattach should succeed");

    let output =
        collect_terminal_output_until(&mut app, session.id(), second.id(), |output, session| {
            output.contains("fixture response: prompt survives detach")
                && session.active_prompt().is_none()
        });

    assert!(output.contains("fixture response: prompt survives detach"));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn shared_opencode_endpoint_routes_multi_agent_prompts_without_pty_exit() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = app
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(default_agent.id()),
        )
        .expect("default provider run should launch");
    let reviewer = app
        .spawn_agent(
            arroba_daemon::agent::CreateAgentRequest::new(session.id(), "opencode")
                .with_alias("reviewer"),
        )
        .expect("reviewer agent should spawn");
    let reviewer_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(reviewer.id()),
        )
        .expect("reviewer provider run should launch");

    app.focus_agent(session.id(), default_agent.id())
        .expect("default agent should focus");
    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first exit prompt on default\n",
        Vec::new(),
    )
    .expect("first prompt should start");

    app.focus_agent(session.id(), reviewer.id())
        .expect("reviewer agent should focus");
    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "reviewer prompt after default exit\n",
        Vec::new(),
    )
    .expect("reviewer prompt should queue");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let timeout_ms = output_timeout_ms().max(6_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut default_output = Vec::new();
    let mut reviewer_output = Vec::new();
    let mut saw_reviewer_run_become_active = false;

    loop {
        for record in app
            .pump_provider_output(session.id(), default_run.id(), recipients.clone())
            .expect("default provider output should pump")
        {
            default_output.extend(record.bytes);
        }
        for record in app
            .pump_provider_output(session.id(), reviewer_run.id(), recipients.clone())
            .expect("reviewer provider output should pump")
        {
            reviewer_output.extend(record.bytes);
        }

        let default_text = String::from_utf8_lossy(&default_output).into_owned();
        let reviewer_text = String::from_utf8_lossy(&reviewer_output).into_owned();
        let session_state = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        let default_run_state = app
            .providers()
            .get_run(default_run.id())
            .expect("default run should remain queryable")
            .state();
        if session_state.active_provider_run_id() == Some(reviewer_run.id()) {
            saw_reviewer_run_become_active = true;
        }

        if default_text.contains("fixture response: first exit prompt on default")
            && reviewer_text.contains("fixture response: reviewer prompt after default exit")
            && matches!(
                default_run_state,
                ProviderRunState::Running | ProviderRunState::Parked
            )
            && session_state.active_prompt().is_none()
        {
            break;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for multi-agent provider exit recovery after {timeout_ms}ms: default={default_text:?} reviewer={reviewer_text:?} active_run={:?} active_prompt={:?}",
            session_state.active_provider_run_id(),
            session_state.active_prompt().map(|prompt| prompt.id().to_string()),
        );
        thread::sleep(Duration::from_millis(25));
    }

    let default_output = String::from_utf8_lossy(&default_output).into_owned();
    let reviewer_output = String::from_utf8_lossy(&reviewer_output).into_owned();
    assert!(default_output.contains("fixture response: first exit prompt on default"));
    assert!(reviewer_output.contains("fixture response: reviewer prompt after default exit"));

    let settled_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(settled_state.focused_agent_id(), Some(reviewer.id()));
    assert!(saw_reviewer_run_become_active);
    assert!(settled_state.queued_prompts().is_empty());
    assert_eq!(
        app.providers()
            .get_run(default_run.id())
            .expect("default run should remain queryable")
            .state(),
        ProviderRunState::Parked
    );
    assert_eq!(
        app.providers()
            .get_run(reviewer_run.id())
            .expect("reviewer run should remain queryable")
            .state(),
        ProviderRunState::Running
    );
    assert!(!app.pty().has_process(default_run.id()));
    assert!(!app.pty().has_process(reviewer_run.id()));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

#[test]
fn opencode_launch_requires_explicit_port_override() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script(10);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::remove_var("ARROBA_OPENCODE_PORT");

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let error = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect_err("missing OpenCode port override should fail");

    match error {
        arroba_daemon::DaemonError::InvalidConfig { field, message } => {
            assert_eq!(field, "ARROBA_OPENCODE_PORT");
            assert_eq!(
                message,
                "must be set to an explicit OpenCode server TCP port"
            );
        }
        other => panic!("unexpected error: {other}"),
    }

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(session_state.active_provider_run_id().is_none());

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn opencode_event_stream_does_not_depend_on_session_status_polling() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.set_omit_session_status(true);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "prompt without session status polling\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: prompt without session status polling")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
    );

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(output.contains("fixture response: prompt without session status polling"));
    assert_eq!(session_state.active_provider_run_id(), Some(run.id()));
    assert!(session_state.active_prompt().is_none());
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should remain queryable")
            .state(),
        ProviderRunState::Running
    );
    assert!(!app.pty().has_process(run.id()));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

fn wait_for_terminal_output(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Vec<arroba_daemon::terminal::TerminalOutputRecord> {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let records = app
            .pump_terminal_output(session_id, attachment_id)
            .expect("terminal output should fan out");
        if !records.is_empty() {
            return records;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for terminal output after {timeout_ms}ms"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_local_terminal_output(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> String {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let response = app
            .handle_local_request(LocalDaemonRequest::PumpTerminalOutput(
                PumpTerminalOutputRequest {
                    session_id: session_id.to_string(),
                    attachment_id: attachment_id.to_string(),
                },
            ))
            .expect("terminal output polling should succeed");

        let records = match response {
            LocalDaemonResponse::TerminalOutput { records } => records,
            _ => panic!("unexpected local response"),
        };

        if !records.is_empty() {
            let bytes = records
                .into_iter()
                .flat_map(|record| record.bytes)
                .collect::<Vec<u8>>();
            return String::from_utf8_lossy(&bytes).into_owned();
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for local terminal output after {timeout_ms}ms"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn collect_terminal_output_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    done: F,
) -> String
where
    F: Fn(&str, &arroba_daemon::session::RuntimeSession) -> bool,
{
    let timeout_ms = output_timeout_ms().max(8_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut output = Vec::new();

    loop {
        let records = app
            .pump_terminal_output(session_id, attachment_id)
            .expect("terminal output should fan out");
        for record in records {
            output.extend(record.bytes);
        }

        let output_text = String::from_utf8_lossy(&output).into_owned();
        let session = app
            .sessions()
            .get_session(session_id)
            .expect("session should still exist");
        if done(&output_text, &session) {
            return output_text;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for terminal output after {timeout_ms}ms: {output_text}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn collect_provider_output_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    recipient_attachment_ids: Vec<String>,
    done: F,
) -> String
where
    F: Fn(&str, &DaemonApp) -> bool,
{
    let timeout_ms = output_timeout_ms().max(8_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut output = Vec::new();

    loop {
        let records = app
            .pump_provider_output(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
            )
            .expect("provider output should fan out");
        for record in records {
            output.extend(record.bytes);
        }

        let output_text = String::from_utf8_lossy(&output).into_owned();
        if done(&output_text, app) {
            return output_text;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider output after {timeout_ms}ms: {output_text}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn collect_provider_records_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    recipient_attachment_ids: Vec<String>,
    done: F,
) -> Vec<arroba_daemon::terminal::TerminalOutputRecord>
where
    F: Fn(&[arroba_daemon::terminal::TerminalOutputRecord], &DaemonApp) -> bool,
{
    let timeout_ms = output_timeout_ms().max(4_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut records = Vec::new();

    loop {
        let next = app
            .pump_provider_output(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
            )
            .expect("provider output should fan out");
        records.extend(next);

        if done(&records, app) {
            return records;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider records after {timeout_ms}ms: {}",
            render_terminal_output(&records)
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn render_terminal_output(records: &[arroba_daemon::terminal::TerminalOutputRecord]) -> String {
    let mut output = Vec::new();
    for record in records {
        output.extend_from_slice(&record.bytes);
    }
    String::from_utf8_lossy(&output).into_owned()
}

struct MockOpenCodeServer {
    port: u16,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<MockOpenCodeState>>,
    thread: Option<thread::JoinHandle<()>>,
}

struct MockOpenCodeState {
    abort_count: u64,
    disconnect_next_event_stream: bool,
    emit_idle_before_completion: bool,
    emit_tool_call_before_completion: bool,
    fail_next_event_stream_attempts: u64,
    event_subscribers: Vec<mpsc::Sender<String>>,
    next_prompt_error: Option<String>,
    response_delay: Duration,
    omit_session_status: bool,
    sessions: BTreeMap<String, MockOpenCodeSessionState>,
    next_session_number: u64,
    next_message_number: u64,
}

struct MockOpenCodeSessionState {
    status: String,
    messages: Vec<Value>,
}

impl MockOpenCodeServer {
    fn start(response_delay: Duration) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("mock server should bind");
        listener
            .set_nonblocking(true)
            .expect("mock server should become non-blocking");
        let port = listener
            .local_addr()
            .expect("mock server should have local addr")
            .port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let state = Arc::new(Mutex::new(MockOpenCodeState {
            abort_count: 0,
            disconnect_next_event_stream: false,
            emit_idle_before_completion: false,
            emit_tool_call_before_completion: false,
            fail_next_event_stream_attempts: 0,
            event_subscribers: Vec::new(),
            next_prompt_error: None,
            response_delay,
            omit_session_status: false,
            sessions: BTreeMap::new(),
            next_session_number: 0,
            next_message_number: 0,
        }));
        let state_for_thread = state.clone();
        let stop_for_thread_loop = stop_flag.clone();
        let thread = thread::spawn(move || {
            while !stop_flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state_for_connection = state_for_thread.clone();
                        let stop_for_connection = stop_for_thread_loop.clone();
                        thread::spawn(move || {
                            handle_mock_opencode_request(
                                stream,
                                &state_for_connection,
                                &stop_for_connection,
                            );
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            port,
            stop,
            state,
            thread: Some(thread),
        }
    }

    fn port(&self) -> u16 {
        self.port
    }

    fn set_omit_session_status(&self, omit_session_status: bool) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .omit_session_status = omit_session_status;
    }

    fn abort_count(&self) -> u64 {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .abort_count
    }

    fn disconnect_next_event_stream(&self) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .disconnect_next_event_stream = true;
    }

    fn set_emit_tool_call_before_completion(&self, emit_tool_call_before_completion: bool) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .emit_tool_call_before_completion = emit_tool_call_before_completion;
    }

    fn fail_next_event_stream_attempts(&self, count: u64) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .fail_next_event_stream_attempts = count;
    }

    fn fail_next_prompt(&self, message: impl Into<String>) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .next_prompt_error = Some(message.into());
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle_mock_opencode_request(
    mut stream: std::net::TcpStream,
    state: &Arc<Mutex<MockOpenCodeState>>,
    stop: &Arc<AtomicBool>,
) {
    let request = read_http_request(&mut stream);
    let Some(request) = request else {
        return;
    };

    if request.method == "GET" && request.path == "/event" {
        handle_mock_opencode_event_stream(stream, state, stop);
        return;
    }

    let response = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/global/health") => json!({ "healthy": true, "version": "test" }),
        ("POST", "/session") => {
            let session_id = {
                let mut state = state.lock().expect("mock state should not be poisoned");
                state.next_session_number += 1;
                let session_id = format!("mock-session-{}", state.next_session_number);
                state.sessions.insert(
                    session_id.clone(),
                    MockOpenCodeSessionState {
                        status: "idle".to_string(),
                        messages: Vec::new(),
                    },
                );
                session_id
            };
            json!({ "id": session_id })
        }
        ("GET", "/session/status") => {
            let state = state.lock().expect("mock state should not be poisoned");
            if state.omit_session_status {
                json!({})
            } else {
                let status_map = state
                    .sessions
                    .iter()
                    .map(|(session_id, session_state)| {
                        (
                            session_id.clone(),
                            json!({
                                "type": session_state.status,
                            }),
                        )
                    })
                    .collect::<serde_json::Map<String, Value>>();
                Value::Object(status_map)
            }
        }
        ("GET", path) if path.starts_with("/session/") && path.ends_with("/message") => {
            let state = state.lock().expect("mock state should not be poisoned");
            let session_id = path
                .strip_prefix("/session/")
                .and_then(|value| value.strip_suffix("/message"))
                .unwrap_or_default();
            Value::Array(
                state
                    .sessions
                    .get(session_id)
                    .map(|session| session.messages.clone())
                    .unwrap_or_default(),
            )
        }
        ("POST", path) if path.starts_with("/session/") && path.ends_with("/prompt_async") => {
            let payload: Value =
                serde_json::from_slice(&request.body).expect("prompt body should parse");
            let prompt = payload["parts"][0]["text"]
                .as_str()
                .expect("prompt should include a text part")
                .trim_end_matches('\n')
                .to_string();
            let session_id = path
                .strip_prefix("/session/")
                .and_then(|value| value.strip_suffix("/prompt_async"))
                .expect("prompt path should include a session id")
                .to_string();
            schedule_mock_response(state.clone(), session_id, prompt);
            write_http_empty_response(&mut stream, 204);
            return;
        }
        ("POST", path) if path.starts_with("/session/") && path.ends_with("/abort") => {
            let mut state = state.lock().expect("mock state should not be poisoned");
            state.abort_count += 1;
            let session_id = path
                .strip_prefix("/session/")
                .and_then(|value| value.strip_suffix("/abort"))
                .expect("abort path should include a session id")
                .to_string();
            if let Some(session_state) = state.sessions.get_mut(&session_id) {
                session_state.status = "idle".to_string();
                publish_mock_event(
                    &mut state,
                    json!({
                        "type": "session.status",
                        "properties": {
                            "sessionID": session_id,
                            "status": {
                                "type": "idle"
                            }
                        }
                    }),
                );
            }
            json!(true)
        }
        _ => {
            write_http_response(&mut stream, 404, json!({ "error": "not found" }));
            return;
        }
    };

    write_http_response(&mut stream, 200, response);
}

fn handle_mock_opencode_event_stream(
    mut stream: std::net::TcpStream,
    state: &Arc<Mutex<MockOpenCodeState>>,
    stop: &Arc<AtomicBool>,
) {
    let (tx, rx) = mpsc::channel();
    let (disconnect_immediately, fail_with_http_error) = {
        let mut state = state.lock().expect("mock state should not be poisoned");
        if state.fail_next_event_stream_attempts > 0 {
            state.fail_next_event_stream_attempts -= 1;
            (false, true)
        } else {
            state.event_subscribers.push(tx);
            let disconnect = state.disconnect_next_event_stream;
            state.disconnect_next_event_stream = false;
            (disconnect, false)
        }
    };

    if fail_with_http_error {
        let response = "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        return;
    }

    if write_sse_connected_response(&mut stream, disconnect_immediately).is_err() {
        return;
    }

    if disconnect_immediately {
        return;
    }

    while !stop.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(payload) => {
                if write_sse_event(&mut stream, &payload).is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn publish_mock_event(state: &mut MockOpenCodeState, payload: Value) {
    let payload = payload.to_string();
    state
        .event_subscribers
        .retain(|subscriber| subscriber.send(payload.clone()).is_ok());
}

fn write_sse_headers(stream: &mut std::net::TcpStream) {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    stream
        .write_all(response.as_bytes())
        .expect("mock SSE headers should write");
    let _ = stream.flush();
}

fn write_sse_connected_response(
    stream: &mut std::net::TcpStream,
    include_event_with_headers: bool,
) -> std::io::Result<()> {
    let connected = json!({
        "type": "server.connected",
        "properties": {}
    })
    .to_string();
    if include_event_with_headers {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\ndata: {connected}\n\n"
        );
        stream.write_all(response.as_bytes())?;
        return stream.flush();
    }

    write_sse_headers(stream);
    write_sse_event(stream, &connected)
}

fn write_sse_event(stream: &mut std::net::TcpStream, payload: &str) -> std::io::Result<()> {
    stream.write_all(format!("data: {payload}\n\n").as_bytes())?;
    stream.flush()
}

fn schedule_mock_response(
    state: Arc<Mutex<MockOpenCodeState>>,
    session_id: String,
    prompt: String,
) {
    {
        let mut state = state.lock().expect("mock state should not be poisoned");
        if let Some(session_state) = state.sessions.get_mut(&session_id) {
            session_state.status = "busy".to_string();
        }
        publish_mock_event(
            &mut state,
            json!({
                "type": "session.status",
                "properties": {
                    "sessionID": session_id,
                    "status": {
                        "type": "busy"
                    }
                }
            }),
        );
    }

    thread::spawn(move || {
        let (response_delay, emit_idle_before_completion, emit_tool_call_before_completion) = {
            let state = state.lock().expect("mock state should not be poisoned");
            (
                state.response_delay,
                state.emit_idle_before_completion,
                state.emit_tool_call_before_completion,
            )
        };
        thread::sleep(response_delay);

        if emit_tool_call_before_completion {
            let mut state = state.lock().expect("mock state should not be poisoned");
            state.next_message_number += 1;
            let message_id = format!("assistant-tool-message-{}", state.next_message_number);
            let part_id = format!("assistant-tool-part-{}", state.next_message_number);
            if let Some(session_state) = state.sessions.get_mut(&session_id) {
                session_state.messages.push(json!({
                    "info": {
                        "id": message_id.clone(),
                        "sessionID": session_id.clone(),
                        "role": "assistant",
                        "time": {
                            "completed": 1,
                        }
                    },
                    "parts": [
                        {
                            "id": part_id.clone(),
                            "sessionID": session_id.clone(),
                            "messageID": message_id.clone(),
                            "type": "tool",
                            "tool": "read",
                            "state": {
                                "status": "completed",
                                "input": {
                                    "filePath": "./.arroba/mock-instructions.md"
                                },
                                "output": "<content>mock tool output</content>",
                                "title": "mock read"
                            }
                        }
                    ]
                }));
            }
            publish_mock_event(
                &mut state,
                json!({
                    "type": "message.updated",
                    "properties": {
                        "info": {
                            "id": message_id.clone(),
                            "sessionID": session_id.clone(),
                            "role": "assistant",
                            "time": {
                                "completed": 1
                            }
                        }
                    }
                }),
            );
            publish_mock_event(
                &mut state,
                json!({
                    "type": "message.part.updated",
                    "properties": {
                        "part": {
                            "id": part_id,
                            "sessionID": session_id.clone(),
                            "messageID": message_id,
                            "type": "tool",
                            "tool": "read",
                            "state": {
                                "status": "completed",
                                "input": {
                                    "filePath": "./.arroba/mock-instructions.md"
                                },
                                "output": "<content>mock tool output</content>",
                                "title": "mock read"
                            }
                        }
                    }
                }),
            );
            drop(state);
            thread::sleep(response_delay);
        }

        if emit_idle_before_completion {
            let mut state = state.lock().expect("mock state should not be poisoned");
            if let Some(session_state) = state.sessions.get_mut(&session_id) {
                session_state.status = "idle".to_string();
            }
            publish_mock_event(
                &mut state,
                json!({
                    "type": "session.status",
                    "properties": {
                        "sessionID": session_id.clone(),
                        "status": {
                            "type": "idle"
                        }
                    }
                }),
            );
            drop(state);
            thread::sleep(response_delay);
        }

        let mut state = state.lock().expect("mock state should not be poisoned");
        if let Some(error_message) = state.next_prompt_error.take() {
            if let Some(session_state) = state.sessions.get_mut(&session_id) {
                session_state.status = "idle".to_string();
            }
            publish_mock_event(
                &mut state,
                json!({
                    "type": "session.error",
                    "properties": {
                        "sessionID": session_id.clone(),
                        "error": {
                            "message": error_message
                        }
                    }
                }),
            );
            publish_mock_event(
                &mut state,
                json!({
                    "type": "session.status",
                    "properties": {
                        "sessionID": session_id,
                        "status": {
                            "type": "idle"
                        }
                    }
                }),
            );
            return;
        }

        state.next_message_number += 1;
        let message_id = format!("assistant-message-{}", state.next_message_number);
        let part_id = format!("assistant-part-{}", state.next_message_number);
        let response_text = format!("fixture response: {prompt}\n");
        if let Some(session_state) = state.sessions.get_mut(&session_id) {
            session_state.messages.push(json!({
                "info": {
                    "id": message_id.clone(),
                    "sessionID": session_id.clone(),
                    "role": "assistant",
                    "time": {
                        "completed": 1,
                    }
                },
                "parts": [
                    {
                        "id": part_id.clone(),
                        "sessionID": session_id.clone(),
                        "messageID": message_id.clone(),
                        "type": "text",
                        "text": response_text.clone(),
                        "time": {
                            "end": 1
                        }
                    }
                ]
            }));
            session_state.status = "idle".to_string();
        }
        publish_mock_event(
            &mut state,
            json!({
                "type": "message.part.delta",
                "properties": {
                    "sessionID": session_id.clone(),
                    "messageID": message_id.clone(),
                    "partID": part_id.clone(),
                    "field": "text",
                    "delta": response_text.clone(),
                }
            }),
        );
        publish_mock_event(
            &mut state,
            json!({
                "type": "message.updated",
                "properties": {
                    "info": {
                        "id": message_id.clone(),
                        "sessionID": session_id.clone(),
                        "role": "assistant",
                        "time": {
                            "completed": 1
                        }
                    }
                }
            }),
        );
        publish_mock_event(
            &mut state,
            json!({
                "type": "session.status",
                "properties": {
                    "sessionID": session_id,
                    "status": {
                        "type": "idle"
                    }
                }
            }),
        );
    });
}

#[test]
fn shared_opencode_tool_activity_keeps_prompt_alive_until_explicit_idle_after_followup_output() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(150));
    mock_server.set_emit_tool_call_before_completion(true);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_daemon::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "tool activity should keep prompt alive\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let interim = collect_provider_records_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |records, _app| {
            records
                .iter()
                .any(|record| record.kind == TerminalOutputKind::ProviderTool)
        },
    );
    assert!(
        interim
            .iter()
            .any(|record| record.kind == TerminalOutputKind::ProviderTool),
        "mock tool activity should be observed"
    );

    let session_after_tool = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist after tool activity");
    assert!(
        session_after_tool.active_prompt().is_some(),
        "tool activity must keep the active prompt alive"
    );

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let records_after_tool_only_completion = app
        .pump_provider_output(session.id(), run.id(), recipients)
        .expect("pump before followup output should succeed");
    let output_after_tool_only_completion =
        render_terminal_output(&records_after_tool_only_completion);
    let session_after_tool_only_completion = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist after tool-only completion");
    let saw_followup_output = output_after_tool_only_completion
        .contains("fixture response: tool activity should keep prompt alive");
    if !saw_followup_output {
        assert!(
            session_after_tool_only_completion.active_prompt().is_some(),
            "tool-call-only completion must not settle the prompt before OpenCode reports idle"
        );
    }

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = if saw_followup_output {
        collect_provider_output_until(
            &mut app,
            session.id(),
            run.id(),
            recipients,
            |_output, app| {
                app.sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
            },
        )
    } else {
        collect_provider_output_until(
            &mut app,
            session.id(),
            run.id(),
            recipients,
            |output, app| {
                output.contains("fixture response: tool activity should keep prompt alive")
                    && app
                        .sessions()
                        .get_session(session.id())
                        .expect("session should still exist")
                        .active_prompt()
                        .is_none()
            },
        )
    };
    let combined_output = format!("{output_after_tool_only_completion}{output}");
    assert!(combined_output.contains("fixture response: tool activity should keep prompt alive"));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut std::net::TcpStream) -> Option<HttpRequest> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("mock request stream should accept timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end;
    loop {
        let size = stream.read(&mut chunk).ok()?;
        if size == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..size]);
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }

    let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = header_text.lines();
    let request_line = lines.next()?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next()?.to_string();
    let path = request_parts.next()?.to_string();
    let content_length = lines
        .find_map(|line| {
            let mut parts = line.splitn(2, ':');
            let name = parts.next()?.trim();
            let value = parts.next()?.trim();
            (name.eq_ignore_ascii_case("content-length")).then_some(value)
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let size = stream.read(&mut chunk).ok()?;
        if size == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..size]);
    }

    Some(HttpRequest { method, path, body })
}

fn write_http_response(stream: &mut std::net::TcpStream, status: u16, body: Value) {
    let body = serde_json::to_vec(&body).expect("mock response should encode");
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("mock response header should write");
    stream
        .write_all(&body)
        .expect("mock response body should write");
    let _ = stream.flush();
}

fn write_http_empty_response(stream: &mut std::net::TcpStream, status: u16) {
    let status_text = match status {
        204 => "No Content",
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(response.as_bytes())
        .expect("mock empty response should write");
    let _ = stream.flush();
}

fn create_opencode_fixture_script(delay_seconds: u64) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "arroba-opencode-fixture-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be monotonic enough")
            .as_nanos()
    ));
    fs::write(&path, fixture_script_contents(delay_seconds))
        .expect("fixture script should be created");
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&path)
            .expect("fixture script should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("fixture script should be executable");
    }
    path
}

fn fixture_script_contents(delay_seconds: u64) -> String {
    format!("#!/bin/sh\nsleep {delay_seconds}\n")
}

#[test]
fn parked_provider_runs_should_not_produce_unexpected_exit_notices() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let _attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    // Create first agent and launch provider run
    let first_agent = app
        .spawn_agent(arroba_daemon::agent::CreateAgentRequest::new(
            session.id(),
            "dev-stub",
        ))
        .expect("first agent should spawn");
    let first_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect("first provider run should launch");

    // Create second agent and launch provider run (this parks the first run)
    let second_agent = app
        .spawn_agent(
            arroba_daemon::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("second"),
        )
        .expect("second agent should spawn");
    let second_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(second_agent.id()),
        )
        .expect("second provider run should launch");

    // Verify the first run is now parked
    assert_eq!(
        app.providers()
            .get_run(first_run.id())
            .expect("first run should exist")
            .state(),
        ProviderRunState::Parked
    );

    // Verify the second run is active
    assert_eq!(
        app.providers()
            .get_run(second_run.id())
            .expect("second run should exist")
            .state(),
        ProviderRunState::Running
    );

    // Pump output from the parked run - this should NOT produce unexpected exit notices
    let records = app
        .pump_provider_output(
            session.id(),
            first_run.id(),
            app.attachments().list_session_attachment_ids(session.id()),
        )
        .expect("pumping from parked run should succeed");
    assert!(
        records.is_empty(),
        "parked runs should not emit transcript output while inactive"
    );

    // Verify the parked run is still parked (not ended)
    assert_eq!(
        app.providers()
            .get_run(first_run.id())
            .expect("first run should exist")
            .state(),
        ProviderRunState::Parked,
        "parked run should remain parked after pumping output"
    );

    // Check that no unexpected exit notices were recorded
    let notices: Vec<_> = app
        .terminal()
        .notice_records()
        .iter()
        .filter(|record| record.message.contains("ended unexpectedly"))
        .cloned()
        .collect();

    assert!(
        notices.is_empty(),
        "parked runs should not produce 'ended unexpectedly' notices, but got: {:?}",
        notices.iter().map(|n| &n.message).collect::<Vec<_>>()
    );
}

fn output_timeout_ms() -> u64 {
    env::var("ARROBA_HARNESS_OUTPUT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2_000)
}
