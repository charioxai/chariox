use std::collections::BTreeMap;
use std::thread;
use std::time::Duration;

use chariox_kernel::agent::CreateAgentRequest;
use chariox_kernel::attachment::{AttachRequest, ClientCapabilityLevel};
use chariox_kernel::local::{
    AttachToSessionRequest, EndSessionRequest, LaunchProviderRunRequest, LocalDaemonClient,
    LocalDaemonRequest, LocalDaemonResponse, SubmitPromptRequest, UpdateSessionConfigRequest,
};
use chariox_kernel::provider::{LaunchProviderRequest, ProviderRunState};
use chariox_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use chariox_kernel::session::{
    CreateSessionRequest, PromptSubmissionOutcome, SessionStatus, WorkflowNodeRunStatus,
    WorkflowRunStatus,
};
use chariox_kernel::{DaemonApp, DaemonConfig};
use tokio::sync::{oneshot, Mutex as TokioMutex};

mod support;
use support::kernel_websocket::{connect_with_retry, reserved_kernel_listener, unused_tcp_port};
use support::runtime_integration::{
    collect_terminal_output_until, wait_for_local_provider_run_ready,
    wait_for_local_terminal_output,
};

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

    let first_outcome = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        first.id(),
        "first integration prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let second_outcome = chariox_kernel::transport::TransportService::schedule_direct_prompt(
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
fn detaching_attachment_preserves_queued_prompts_and_promotes_via_remaining_attachment() {
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

    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        first.id(),
        "first integration prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
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
        chariox_kernel::transport::TransportService::complete_active_prompt(&mut app, session.id())
            .expect("active prompt should complete");

    let started_next = completion
        .started_next
        .expect("queued prompt should promote through the remaining attachment");
    assert_eq!(started_next.prompt(), "second integration prompt\n");
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(
        session.active_prompt().map(|prompt| prompt.id()),
        Some(started_next.id())
    );
    assert!(session.queued_prompts().is_empty());
    assert!(
        app.terminal().input_records().iter().any(|record| {
            record.source_attachment_id == first.id()
                && String::from_utf8_lossy(&record.bytes).contains("second integration prompt")
        }),
        "promoted queued prompt should be echoed through the remaining attachment"
    );
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

    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "fallback completion\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let completion =
        chariox_kernel::transport::TransportService::complete_active_prompt(&mut app, session.id())
            .expect("active prompt should complete");

    let terminal = app.terminal().clone();
    let records = terminal.drain_completion_records(session.id(), attachment.id());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider_run_id, run.id());
    assert_eq!(
        records[0].message_id,
        format!("prompt-complete:{}", completion.completed.id())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn workflow_runs_progress_without_terminal_pumps() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.runtime_mcp_port = unused_tcp_port();
    let mut app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("node-a")
                .with_model("workflow-single-turn-node"),
        )
        .expect("agent should spawn");
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("workflow-pump-test".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be created");
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("workflow node should be allowed to complete");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let app = std::sync::Arc::new(TokioMutex::new(app));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_app = std::sync::Arc::clone(&app);
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener(server_app, kernel_websocket_listener, async {
            let _ = shutdown_rx.await;
        })
        .await
    });

    let readiness_socket = connect_with_retry(&config.kernel_websocket_url()).await;
    drop(readiness_socket);

    let run = {
        let mut app = app.lock().await;
        app.invoke_workflow_endpoint_and_schedule(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("kickoff".to_string()),
        )
        .expect("workflow run should start")
        .0
    };

    let run_id = run.id().to_string();
    let run = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let run = {
                let app = app.lock().await;
                app.sessions()
                    .resolve_workflow_run_ref(session.id(), &run_id)
                    .expect("workflow run should exist")
                    .clone()
            };
            if !matches!(
                run.status(),
                WorkflowRunStatus::Running | WorkflowRunStatus::Waiting
            ) {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    let run = match run {
        Ok(run) => run,
        Err(_) => {
            let app = app.lock().await;
            panic!(
                "workflow run never completed: run={:#?}, pending_terminal_output_records={}",
                app.sessions()
                    .resolve_workflow_run_ref(session.id(), &run_id),
                app.terminal().output_records().len(),
            )
        }
    };
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

    app.lock()
        .await
        .end_session(session.id())
        .expect("session should end cleanly");
    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[test]
fn provider_run_switching_parks_previous_run_and_records_terminal_input() {
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
            "terminal-echo-a",
        ))
        .expect("first provider run should launch");
    let second_run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "terminal-echo-b",
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

    app.send_terminal_input(session.id(), source.id(), None, b"switched run\n")
        .expect("attachment input should reach active provider run");
    let input_records = app.terminal().input_records();
    assert_eq!(input_records.len(), 1);
    assert_eq!(input_records[0].provider_run_id, second_run.id());
    assert_eq!(input_records[0].source_attachment_id, source.id());
    assert_eq!(input_records[0].bytes, b"switched run\n");
}

#[test]
fn local_request_surface_supports_prompt_queue_and_config_updates() {
    thread::Builder::new()
        .name("local-request-surface-supports-prompt-queue-and-config-updates".to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(local_request_surface_supports_prompt_queue_and_config_updates_inner)
        .expect("local request surface large-stack test thread should spawn")
        .join()
        .expect("local request surface large-stack test thread should complete");
}

fn local_request_surface_supports_prompt_queue_and_config_updates_inner() {
    let app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let client = LocalDaemonClient::new(app).expect("local daemon client should start");

    let session = match client
        .send(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-integration", "worktree-integration"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };

    let first = match client
        .send(LocalDaemonRequest::AttachToSession(
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

    let second = match client
        .send(LocalDaemonRequest::AttachToSession(
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

    let provider_run = match client
        .send(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: None,
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        ))
        .expect("provider launch should succeed")
    {
        LocalDaemonResponse::ProviderRunLaunched { provider_run }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => provider_run,
        _ => panic!("unexpected local response"),
    };

    wait_for_local_provider_run_ready(&client, session.id(), provider_run.id());

    let first_prompt = client
        .send(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: first.id().to_string(),
            target_agent_id: None,
            prompt: "first local prompt\n".to_string(),
            attachments: Vec::new(),
            prompt_source: None,
        }))
        .expect("first prompt should start");
    let second_prompt = client
        .send(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: second.id().to_string(),
            target_agent_id: None,
            prompt: "second local prompt\n".to_string(),
            attachments: Vec::new(),
            prompt_source: None,
        }))
        .expect("second prompt should queue");
    let config = client
        .send(LocalDaemonRequest::UpdateSessionConfig(
            UpdateSessionConfigRequest {
                session_id: session.id().to_string(),
                attachment_id: first.id().to_string(),
                values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                requires_idle: false,
            },
        ))
        .expect("config update should succeed");
    let echoed_output = wait_for_local_terminal_output(&client, session.id(), second.id());
    let ended = client
        .send(LocalDaemonRequest::EndSession(EndSessionRequest {
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

    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first auto prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
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
        .spawn_agent(chariox_kernel::agent::CreateAgentRequest::new(
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
            chariox_kernel::agent::CreateAgentRequest::new(session.id(), "dev-stub")
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
    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let records = chariox_kernel::transport::TransportService::pump_provider_output(
        &mut app,
        session.id(),
        first_run.id(),
        recipients,
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
