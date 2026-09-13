use crate::app::KernelSessionService;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};

const TERMINAL_DIAGNOSTIC: &str = "app liveness terminal diagnostic";
const SYNTHETIC_API_KEY: &str = "sk-app-liveness-secret";
const SYNTHETIC_TOKEN: &str = "app-liveness-token";

#[test]
fn app_resize_liveness_reconciliation_preserves_pty_terminal_diagnostic() {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-app-liveness-diagnostic",
            "worktree-app-liveness-diagnostic",
        ))
        .expect("session should be created");
    let attachment = KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-app-liveness-diagnostic",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "test-model",
    )
    .with_agent_id(agent.id());
    let mut run = RuntimeProviderRun::new(
        "provider-run-app-liveness-diagnostic",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "app-liveness-diagnostic".to_string(),
            pty_target: Some("app-liveness-diagnostic".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                format!(
                    "printf '%s\\n' '{TERMINAL_DIAGNOSTIC} api_key={SYNTHETIC_API_KEY} token={SYNTHETIC_TOKEN}'; exit 1"
                ),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "exercise app liveness reconciliation",
        PromptStatus::Queued,
    );
    let prompt_id = match app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
        crate::session::PromptSubmissionOutcome::Queued { prompt } => {
            panic!("diagnostic prompt should start, got queued prompt {}", prompt.id())
        }
    };
    let active_prompt_id = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("active prompt should resolve")
        .expect("diagnostic prompt should remain active before process exit")
        .id()
        .to_string();
    assert_eq!(active_prompt_id, prompt_id);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if matches!(
            app.pty.poll_process_state(run.id()),
            Ok(crate::pty::PtyProcessState::Exited { .. })
        ) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the app-liveness diagnostic process did not exit"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let resize_result = app.resize_terminal(session.id(), 80, 24);
    assert!(
        matches!(
            &resize_result,
            Err(crate::error::DaemonError::InvalidProviderRunState { .. })
        ),
        "liveness reconciliation should settle the exited run before resize: {resize_result:?}"
    );
    let diagnostic = {
        let provider_run = app
            .providers()
            .get_run(run.id())
            .expect("provider run should remain queryable");
        provider_run
            .terminal_diagnostic()
            .expect("app-level liveness reconciliation should retain the PTY diagnostic")
            .to_string()
    };
    assert!(diagnostic.contains(TERMINAL_DIAGNOSTIC), "{diagnostic}");
    assert!(diagnostic.contains("api_key"), "{diagnostic}");
    assert!(diagnostic.contains("token"), "{diagnostic}");
    assert_eq!(diagnostic.matches("[redacted]").count(), 2, "{diagnostic}");
    assert!(!diagnostic.contains(SYNTHETIC_API_KEY), "{diagnostic}");
    assert!(!diagnostic.contains(SYNTHETIC_TOKEN), "{diagnostic}");

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should remain available");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "the app liveness path must settle the prompt exactly once"
    );
    let completed_turn = app
        .completed_git_turn_snapshot_store()
        .latest_projection_for_agent(session.id(), agent.id())
        .expect("app liveness settlement should remain projected");
    assert_eq!(completed_turn.prompt_id, prompt_id);
    assert_eq!(
        app.agents
            .get_agent(agent.id())
            .expect("agent should remain queryable after provider exit")
            .state(),
        crate::agent::AgentState::Error,
        "app-level liveness keeps the legacy Completed settlement while marking the unexpected exit on the agent"
    );
    assert_eq!(
        completed_turn.settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Completed
    );

    let repeated_input_result = app.send_terminal_input(
        session.id(),
        attachment.id(),
        Some(run.id()),
        b"repeated input observation",
    );
    assert!(
        matches!(
            &repeated_input_result,
            Err(crate::error::DaemonError::InvalidProviderRunState { .. })
        ),
        "a repeated public input observation must not revive the ended run: {repeated_input_result:?}"
    );
    let diagnostic_after_input = {
        let provider_run = app
            .providers()
            .get_run(run.id())
            .expect("provider run should remain queryable after repeated observation");
        provider_run
            .terminal_diagnostic()
            .expect("terminal diagnostic should remain after repeated observation")
            .to_string()
    };
    assert_eq!(diagnostic_after_input, diagnostic);
    assert_eq!(
        app.terminal()
            .drain_completion_records(session.id(), attachment.id())
            .len(),
        1,
        "repeated liveness observation must not settle the prompt twice"
    );
}
