use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use arroba_daemon::attachment::{AttachRequest, ClientCapabilityLevel};
use arroba_daemon::local::{
    AttachToSessionRequest, EndSessionRequest, LaunchProviderRunRequest, LocalDaemonRequest,
    LocalDaemonResponse, PumpTerminalOutputRequest, SubmitPromptRequest,
    UpdateSessionConfigRequest,
};
use arroba_daemon::provider::{LaunchProviderRequest, ProviderRunState};
use arroba_daemon::session::{
    CreateSessionRequest, PromptStatus, PromptSubmissionOutcome, SessionStatus,
};
use arroba_daemon::{DaemonApp, DaemonConfig};
use serde_json::{json, Value};

static OPENCODE_ENV_LOCK: Mutex<()> = Mutex::new(());

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

    let first_outcome = app
        .submit_prompt(session.id(), first.id(), "first integration prompt\n")
        .expect("first prompt should start");
    let second_outcome = app
        .submit_prompt(session.id(), second.id(), "second integration prompt\n")
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

    let _ = app
        .submit_prompt(session.id(), first.id(), "first integration prompt\n")
        .expect("first prompt should start");
    let _ = app
        .submit_prompt(session.id(), second.id(), "second integration prompt\n")
        .expect("second prompt should queue");

    app.detach(second.id())
        .expect("queued prompt source should detach cleanly");
    let completion = app
        .complete_active_prompt(session.id())
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
    let records = wait_for_terminal_output(&mut app, session.id());
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
        LocalDaemonResponse::SessionCreated { session } => session,
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
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
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
            prompt: "first local prompt\n".to_string(),
        }))
        .expect("first prompt should start");
    let second_prompt = app
        .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: second.id().to_string(),
            prompt: "second local prompt\n".to_string(),
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
    let output = wait_for_local_terminal_output(&mut app, session.id());
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
    assert!(output.contains("first local prompt"));
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

    let _ = app
        .submit_prompt(session.id(), attachment.id(), "first auto prompt\n")
        .expect("first prompt should start");
    let _ = app
        .submit_prompt(session.id(), attachment.id(), "second auto prompt\n")
        .expect("second prompt should queue");

    let combined = collect_terminal_output_until(&mut app, session.id(), |output, session| {
        output.contains("first auto prompt")
            && output.contains("second auto prompt")
            && session.active_prompt().is_none()
            && session.queued_prompts().is_empty()
    });

    assert!(combined.contains("first auto prompt"));
    assert!(combined.contains("second auto prompt"));
}

#[test]
fn unexpected_provider_exit_marks_run_ended_and_clears_active_prompt() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let fixture_path = create_opencode_fixture_script(1);
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

    let _ = app
        .submit_prompt(session.id(), attachment.id(), "first exit prompt\n")
        .expect("first prompt should start");
    let _ = app
        .submit_prompt(session.id(), attachment.id(), "second exit prompt\n")
        .expect("second prompt should queue");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: first exit prompt")
                && app
                    .providers()
                    .get_run(run.id())
                    .expect("run should remain queryable")
                    .state()
                    == ProviderRunState::Ended
        },
    );

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(output.contains("fixture response: first exit prompt"));
    assert_eq!(session_state.active_provider_run_id(), None);
    assert!(session_state.active_prompt().is_none());
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should remain queryable")
            .state(),
        ProviderRunState::Ended
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
    let _ = fs::remove_file(&fixture_path);
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

    let _ = app
        .submit_prompt(session.id(), attachment.id(), "first prompt should fail\n")
        .expect("first prompt should start");
    let _ = app
        .submit_prompt(session.id(), attachment.id(), "second prompt should run\n")
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

    let _ = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            "first prompt should cancel\n",
        )
        .expect("first prompt should start");
    let _ = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            "second prompt after cancel\n",
        )
        .expect("second prompt should queue");

    let cancellation = app
        .cancel_active_prompt(session.id(), attachment.id())
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
        |output, app| {
            output.contains("fixture response: second prompt after cancel")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
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

    let _ = app
        .submit_prompt(session.id(), attachment.id(), "prompt after reconnect\n")
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
fn opencode_launch_requires_explicit_port_override() {
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
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
    let _guard = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
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

    let _ = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            "prompt without session status polling\n",
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
    assert!(app.pty().has_process(run.id()));

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
fn daemon_and_cli_waits_for_delayed_fixture_response_through_opencode_adapter() {
    let socket_path = std::env::temp_dir().join("arroba-tests").join(format!(
        "cli-smoke-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be monotonic enough")
            .as_nanos()
    ));
    let _ = fs::remove_file(&socket_path);
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_secs(1));

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_arroba-daemon"))
        .env("ARROBA_DAEMON_SOCKET", &socket_path)
        .env("ARROBA_OPENCODE_BIN", &fixture_path)
        .env("ARROBA_OPENCODE_PORT", mock_server.port().to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("daemon should launch");

    wait_for_socket(&socket_path);

    let mut cli = Command::new(env!("CARGO_BIN_EXE_arroba-cli"))
        .env("ARROBA_DAEMON_SOCKET", &socket_path)
        .env("ARROBA_OPENCODE_BIN", &fixture_path)
        .env("ARROBA_OPENCODE_PORT", mock_server.port().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cli should launch");

    {
        let stdin = cli.stdin.as_mut().expect("cli stdin should be piped");
        stdin
            .write_all(b"smoke prompt from cli\n")
            .expect("cli stdin should accept prompt");
    }

    let output = cli.wait_with_output().expect("cli should exit");
    let _ = daemon.kill();
    let daemon_output = daemon.wait_with_output().expect("daemon should exit");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&fixture_path);
    mock_server.stop();

    assert!(
        output.status.success(),
        "cli stderr: {}\ndaemon stderr: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&daemon_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("fixture response: smoke prompt from cli"),
        "cli stdout: {stdout}"
    );
}

fn wait_for_terminal_output(
    app: &mut DaemonApp,
    session_id: &str,
) -> Vec<arroba_daemon::terminal::TerminalOutputRecord> {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let records = app
            .pump_terminal_output(session_id)
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

fn wait_for_local_terminal_output(app: &mut DaemonApp, session_id: &str) -> String {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let response = app
            .handle_local_request(LocalDaemonRequest::PumpTerminalOutput(
                PumpTerminalOutputRequest {
                    session_id: session_id.to_string(),
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

fn wait_for_socket(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);

    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for socket {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn collect_terminal_output_until<F>(app: &mut DaemonApp, session_id: &str, done: F) -> String
where
    F: Fn(&str, &arroba_daemon::session::RuntimeSession) -> bool,
{
    let timeout_ms = output_timeout_ms().max(4_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut output = Vec::new();

    loop {
        let records = app
            .pump_terminal_output(session_id)
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
    let timeout_ms = output_timeout_ms().max(4_000);
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

struct MockOpenCodeServer {
    port: u16,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<MockOpenCodeState>>,
    thread: Option<thread::JoinHandle<()>>,
}

struct MockOpenCodeState {
    abort_count: u64,
    disconnect_next_event_stream: bool,
    event_subscribers: Vec<mpsc::Sender<String>>,
    next_prompt_error: Option<String>,
    response_delay: Duration,
    omit_session_status: bool,
    status: String,
    session_id: String,
    messages: Vec<Value>,
    next_message_number: u64,
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
            event_subscribers: Vec::new(),
            next_prompt_error: None,
            response_delay,
            omit_session_status: false,
            status: "idle".to_string(),
            session_id: "mock-session-1".to_string(),
            messages: Vec::new(),
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
            let session_id = state
                .lock()
                .expect("mock state should not be poisoned")
                .session_id
                .clone();
            json!({ "id": session_id })
        }
        ("GET", "/session/status") => {
            let state = state.lock().expect("mock state should not be poisoned");
            if state.omit_session_status {
                json!({})
            } else {
                json!({
                    state.session_id.clone(): {
                        "type": state.status,
                    }
                })
            }
        }
        ("GET", path) if path.starts_with("/session/") && path.ends_with("/message") => {
            let state = state.lock().expect("mock state should not be poisoned");
            Value::Array(state.messages.clone())
        }
        ("POST", path) if path.starts_with("/session/") && path.ends_with("/prompt_async") => {
            let payload: Value =
                serde_json::from_slice(&request.body).expect("prompt body should parse");
            let prompt = payload["parts"][0]["text"]
                .as_str()
                .expect("prompt should include a text part")
                .trim_end_matches('\n')
                .to_string();
            schedule_mock_response(state.clone(), prompt);
            write_http_empty_response(&mut stream, 204);
            return;
        }
        ("POST", path) if path.starts_with("/session/") && path.ends_with("/abort") => {
            let mut state = state.lock().expect("mock state should not be poisoned");
            state.abort_count += 1;
            state.status = "idle".to_string();
            let session_id = state.session_id.clone();
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
    let disconnect_immediately = {
        let mut state = state.lock().expect("mock state should not be poisoned");
        state.event_subscribers.push(tx);
        let disconnect = state.disconnect_next_event_stream;
        state.disconnect_next_event_stream = false;
        disconnect
    };

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

fn schedule_mock_response(state: Arc<Mutex<MockOpenCodeState>>, prompt: String) {
    {
        let mut state = state.lock().expect("mock state should not be poisoned");
        state.status = "busy".to_string();
        let session_id = state.session_id.clone();
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
        let response_delay = state
            .lock()
            .expect("mock state should not be poisoned")
            .response_delay;
        thread::sleep(response_delay);

        let mut state = state.lock().expect("mock state should not be poisoned");
        if let Some(error_message) = state.next_prompt_error.take() {
            state.status = "idle".to_string();
            let session_id = state.session_id.clone();
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
        let session_id = state.session_id.clone();
        let response_text = format!("fixture response: {prompt}\n");
        state.messages.push(json!({
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
        state.status = "idle".to_string();
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

fn output_timeout_ms() -> u64 {
    env::var("ARROBA_HARNESS_OUTPUT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2_000)
}
