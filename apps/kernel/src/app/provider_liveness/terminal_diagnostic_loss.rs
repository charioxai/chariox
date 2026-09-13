use super::*;

use crate::app::KernelSessionService;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};

const TERMINAL_DIAGNOSTIC: &str = "app liveness terminal diagnostic";

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
                format!("printf '%s\\n' '{TERMINAL_DIAGNOSTIC}'; exit 1"),
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
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");

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
    let provider_run = app
        .providers()
        .get_run(run.id())
        .expect("provider run should remain queryable");
    let diagnostic = provider_run
        .terminal_diagnostic()
        .expect("app-level liveness reconciliation should retain the PTY diagnostic");
    assert!(diagnostic.contains(TERMINAL_DIAGNOSTIC), "{diagnostic}");
}
