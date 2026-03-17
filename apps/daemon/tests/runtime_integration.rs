use std::collections::BTreeMap;
use std::env;
use std::thread;
use std::time::{Duration, Instant};

use arroba_daemon::attachment::{AttachRequest, ClientCapabilityLevel};
use arroba_daemon::local::{
    AttachToSessionRequest, EndSessionRequest, LaunchProviderRunRequest, LocalDaemonRequest,
    LocalDaemonResponse, PumpTerminalOutputRequest, SubmitPromptRequest,
    UpdateSessionConfigRequest,
};
use arroba_daemon::provider::{LaunchProviderRequest, ProviderRunState};
use arroba_daemon::session::{CreateSessionRequest, PromptSubmissionOutcome, SessionStatus};
use arroba_daemon::{DaemonApp, DaemonConfig};

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

fn output_timeout_ms() -> u64 {
    env::var("ARROBA_HARNESS_OUTPUT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2_000)
}
