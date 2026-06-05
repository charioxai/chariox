use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::opencode_binding::OpenCodeRunSelection;
use crate::provider::{
    AgentEndpointMode, ProviderClientInterface, ProviderLaunchResult, ProviderPromptSignalBatch,
    ProviderResumeState, RuntimeProviderRun,
};
use crate::session::{CreateSessionRequest, SessionService, SessionStatus};

use super::{
    LaunchProviderRequest, ProviderProcessService, ProviderRunLivenessReconciliation,
    ProviderRunState,
};

fn sessions() -> SessionService {
    SessionService::new(&DaemonConfig::for_tests())
}

fn launch_request(session_id: &str, model: &str) -> LaunchProviderRequest {
    LaunchProviderRequest::new(session_id, "dev-stub", "claude-code", "default", model)
}

fn launch_running_provider_run(
    providers: &mut ProviderProcessService,
    sessions: &mut SessionService,
    request: LaunchProviderRequest,
) -> RuntimeProviderRun {
    let outcome = providers
        .start_run_provider_only(request)
        .expect("provider-only start should succeed");
    let run = providers
        .mark_run_running(outcome.run().id())
        .expect("provider run should mark running");
    sessions
        .set_active_provider_run(run.session_id(), Some(run.id().to_string()))
        .expect("session active run should be set");
    run
}

#[test]
fn launches_the_first_provider_run() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();

    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );
    let session = sessions
        .get_session(session.id())
        .expect("session should exist");

    assert_eq!(run.id(), "provider-run-1");
    assert_eq!(run.state(), ProviderRunState::Running);
    assert_eq!(run.adapter_key(), "dev-stub");
    assert_eq!(session.active_provider_run_id(), Some(run.id()));
    assert_eq!(session.status(), SessionStatus::Active);
}

#[test]
fn rejects_workspace_live_sync_when_adapter_cannot_enforce_writes() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();

    let error = providers
        .start_run_provider_only(
            launch_request(session.id(), "sonnet").with_workspace_live_sync_managed(),
        )
        .expect_err("dev-stub cannot enforce workspace live sync writes");

    match error {
        DaemonError::ProviderWorkspaceLiveSyncUnsupported { adapter_key, .. } => {
            assert_eq!(adapter_key, "dev-stub");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn tracked_workspace_live_sync_does_not_require_managed_write_enforcement() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();

    let outcome = providers
        .start_run_provider_only(
            launch_request(session.id(), "sonnet")
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
        )
        .expect("tracked mode should not need managed write fencing");

    assert!(!outcome.run().requires_workspace_live_sync());
    assert!(outcome.run().tracks_workspace_live_sync());
}

#[test]
fn active_provider_run_lookup_prefers_deterministic_latest_highest_state_run() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let agent_id = "agent-1".to_string();
    let mut providers = ProviderProcessService::new();

    let first = providers
        .start_run_provider_only(launch_request(session.id(), "sonnet").with_agent_id(&agent_id))
        .expect("first provider run should start");
    let second = providers
        .start_run_provider_only(launch_request(session.id(), "opus").with_agent_id(&agent_id))
        .expect("second provider run should start");
    let first = providers
        .mark_run_running(first.run().id())
        .expect("first run should mark running");
    let second = providers
        .mark_run_running(second.run().id())
        .expect("second run should mark running");

    assert_eq!(
        providers
            .get_run_for_agent(session.id(), &agent_id)
            .map(|run| run.id().to_string()),
        Some(second.id().to_string())
    );
    assert_eq!(
        providers
            .get_session_run_for_provider(session.id(), "claude-code")
            .map(|run| run.id().to_string()),
        Some(second.id().to_string())
    );
    assert!(second.active_selection_cmp(&first).is_gt());
}

#[test]
fn claude_native_tui_runs_use_pty_output_pumping() {
    let providers = ProviderProcessService::new();
    let request = LaunchProviderRequest::new("session-1", "claude", "default", "sonnet", "low")
        .with_client_interface(ProviderClientInterface::NativeTui);
    let run = RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "claude:native-tui".to_string(),
            pty_target: None,
            pty_program: Some("claude".to_string()),
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );

    assert!(!providers.run_uses_structured_prompt_io(&run));
}

#[test]
fn provider_only_start_run_returns_outcome_without_session_mutation() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();

    let outcome = providers
        .start_run_provider_only(launch_request(session.id(), "sonnet"))
        .expect("provider-only start should succeed");

    assert_eq!(outcome.run().session_id(), session.id());
    assert_eq!(outcome.run().state(), ProviderRunState::Starting);
    assert_eq!(
        sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        None
    );
}

#[test]
fn liveness_reconciliation_without_process_observation_does_not_end_run() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );

    let reconciliation = providers
        .reconcile_run_liveness_provider_only(session.id(), run.id(), None)
        .expect("liveness reconciliation should succeed");

    assert!(matches!(
        reconciliation,
        ProviderRunLivenessReconciliation::StillRunning(_)
    ));
    assert_eq!(
        sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(run.id())
    );
    assert_eq!(
        providers
            .get_run(run.id())
            .expect("run should still exist")
            .state(),
        ProviderRunState::Running
    );
}

#[test]
fn provider_only_liveness_does_not_end_starting_run_before_launch_settles() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let outcome = providers
        .start_run_provider_only(launch_request(session.id(), "sonnet"))
        .expect("provider-only start should succeed");

    let reconciliation = providers
        .reconcile_run_liveness_provider_only(session.id(), outcome.run().id(), Some(false))
        .expect("liveness reconciliation should succeed");

    assert!(matches!(
        reconciliation,
        ProviderRunLivenessReconciliation::StillRunning(_)
    ));
    assert_eq!(
        providers
            .get_run(outcome.run().id())
            .expect("run should still exist")
            .state(),
        ProviderRunState::Starting
    );
}

#[test]
fn provider_only_liveness_reconciliation_with_exited_process_marks_run_ended() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );

    let reconciliation = providers
        .reconcile_run_liveness_provider_only(session.id(), run.id(), Some(false))
        .expect("liveness reconciliation should succeed");

    assert!(matches!(
        reconciliation,
        ProviderRunLivenessReconciliation::NewlyEnded(_)
    ));
    assert_eq!(
        sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(run.id())
    );
    assert_eq!(
        providers
            .get_run(run.id())
            .expect("run should still exist")
            .state(),
        ProviderRunState::Ended
    );
}

#[test]
fn provider_only_liveness_reconciliation_handles_already_ended_without_session_mutation() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );

    providers
        .reconcile_run_liveness_provider_only(session.id(), run.id(), Some(false))
        .expect("initial provider-only reconciliation should succeed");

    let reconciliation = providers
        .reconcile_run_liveness_provider_only(session.id(), run.id(), None)
        .expect("provider-only reconciliation should succeed");

    assert!(matches!(
        reconciliation,
        ProviderRunLivenessReconciliation::AlreadyEnded(_)
    ));
    assert_eq!(
        sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(run.id())
    );
    assert_eq!(
        providers
            .get_run(run.id())
            .expect("run should still exist")
            .state(),
        ProviderRunState::Ended
    );
}

#[test]
fn provider_only_mark_run_ended_returns_outcome_without_session_mutation() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );

    let outcome = providers
        .mark_run_ended_provider_only(session.id(), run.id())
        .expect("provider-only ending should succeed");

    assert!(!outcome.already_ended);
    assert_eq!(outcome.run().id(), run.id());
    assert_eq!(outcome.run().state(), ProviderRunState::Ended);
    assert_eq!(
        sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(run.id())
    );

    let outcome = providers
        .mark_run_ended_provider_only(session.id(), run.id())
        .expect("already-ended provider-only ending should succeed");
    assert!(outcome.already_ended);
    assert_eq!(outcome.run().id(), run.id());
}

#[test]
fn provider_only_terminate_run_returns_outcome_without_session_mutation() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );

    let outcome = providers
        .terminate_run_provider_only(session.id(), run.id())
        .expect("provider-only termination should succeed");

    assert!(!outcome.already_ended);
    assert_eq!(outcome.run().id(), run.id());
    assert_eq!(outcome.run().state(), ProviderRunState::Ended);
    assert_eq!(
        sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(run.id())
    );
}

#[test]
fn provider_only_terminate_session_runs_returns_outcomes_without_session_mutation() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let first = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );
    let second = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "opus"),
    );

    let outcome = providers
        .terminate_session_runs_provider_only(session.id())
        .expect("provider-only session termination should succeed");

    let terminated_run_ids = outcome
        .runs()
        .iter()
        .map(|outcome| outcome.run().id().to_string())
        .collect::<Vec<_>>();
    assert_eq!(terminated_run_ids, vec![first.id(), second.id()]);
    assert_eq!(
        providers
            .get_run(first.id())
            .expect("first run should remain recorded")
            .state(),
        ProviderRunState::Ended
    );
    assert_eq!(
        providers
            .get_run(second.id())
            .expect("second run should remain recorded")
            .state(),
        ProviderRunState::Ended
    );
    assert_eq!(
        sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(second.id())
    );
}

#[test]
fn provider_only_park_run_returns_outcome_without_session_mutation() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );

    let outcome = providers
        .park_run_provider_only(session.id(), run.id())
        .expect("provider-only park should succeed");

    assert_eq!(outcome.run().id(), run.id());
    assert_eq!(outcome.run().state(), ProviderRunState::Parked);
    assert_eq!(
        sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(run.id())
    );
}

#[test]
fn provider_only_resume_run_returns_outcome_without_session_mutation() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );
    providers
        .park_run_provider_only(session.id(), run.id())
        .expect("provider run should park");

    let outcome = providers
        .resume_run_provider_only(session.id(), run.id())
        .expect("provider-only resume should succeed");

    assert_eq!(outcome.run().id(), run.id());
    assert_eq!(outcome.run().state(), ProviderRunState::Running);
    assert_eq!(
        sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(run.id())
    );
}

#[test]
fn parks_existing_run_when_new_run_becomes_active() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();

    let first = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );
    let outcome = providers
        .park_run_provider_only(session.id(), first.id())
        .expect("first run should park");
    sessions
        .set_active_provider_run(session.id(), None)
        .expect("session active run should clear");
    assert_eq!(outcome.run().state(), ProviderRunState::Parked);

    let second = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "opus"),
    );

    let first = providers
        .get_run(first.id())
        .expect("first run should still exist");
    let session = sessions
        .get_session(session.id())
        .expect("session should exist");

    assert_eq!(first.state(), ProviderRunState::Parked);
    assert_eq!(second.state(), ProviderRunState::Running);
    assert_eq!(session.active_provider_run_id(), Some(second.id()));
}

#[test]
fn provider_only_start_allows_new_run_after_ended_run() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();

    let first = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );
    providers
        .get_run_mut(first.id())
        .expect("first run should exist")
        .mark_ended();

    let second = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "opus"),
    );
    let session = sessions
        .get_session(session.id())
        .expect("session should exist");
    let first = providers
        .get_run(first.id())
        .expect("first run should still exist");

    assert_eq!(first.state(), ProviderRunState::Ended);
    assert_eq!(second.state(), ProviderRunState::Running);
    assert_eq!(session.active_provider_run_id(), Some(second.id()));
}

#[test]
fn launch_run_preserves_resume_state_from_the_request() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();

    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet")
            .with_resume_state(ProviderResumeState::from_codex_thread_id("thread-1")),
    );

    assert_eq!(run.resume_state().codex_thread_id(), Some("thread-1"));
}

#[test]
fn structured_output_metadata_records_terminal_diagnostic() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        launch_request(session.id(), "sonnet"),
    );

    providers
        .apply_structured_output_metadata(
            run.id(),
            &ProviderPromptSignalBatch {
                terminal_failure: Some("provider no credits".to_string()),
                ..ProviderPromptSignalBatch::default()
            },
        )
        .expect("terminal diagnostic should record");

    let run = providers.get_run(run.id()).expect("run should exist");
    assert_eq!(run.terminal_diagnostic(), Some("provider no credits"));
}

#[test]
fn structured_output_metadata_records_completed_codex_resume_state() {
    let mut sessions = sessions();
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let mut providers = ProviderProcessService::new();
    let run = launch_running_provider_run(
        &mut providers,
        &mut sessions,
        LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "gpt-5.5"),
    );

    providers
        .apply_structured_output_metadata(
            run.id(),
            &ProviderPromptSignalBatch {
                resolved_resume_state: Some(ProviderResumeState::from_codex_thread_id("thread-1")),
                ..ProviderPromptSignalBatch::default()
            },
        )
        .expect("completed Codex resume state should record");

    let run = providers.get_run(run.id()).expect("run should exist");
    assert_eq!(run.resume_state().codex_thread_id(), Some("thread-1"));
    assert_eq!(run.provider_session_id(), Some("thread-1"));
}

#[test]
fn merge_opencode_run_selection_keeps_the_existing_run_when_sync_has_no_metadata() {
    let request = LaunchProviderRequest::new(
        "session-1",
        "opencode",
        "opencode",
        "default",
        "anthropic/claude-sonnet-4",
    );
    let run = RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "opencode:endpoint".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("http://127.0.0.1:43112".to_string()),
        },
    );

    let merged =
        ProviderProcessService::merge_opencode_run_selection(&run, OpenCodeRunSelection::default());

    assert_eq!(merged.model.as_deref(), Some("anthropic/claude-sonnet-4"));
    assert_eq!(merged.variant.as_deref(), None);
}
