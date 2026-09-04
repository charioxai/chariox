use super::provider_output_runtime::{
    provider_run_allows_quiet_pty_settlement, provider_run_ids_for_owned_output_pump,
    provider_run_uses_structured_output_pump,
};
use super::*;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
    let (
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    ) = {
        let app_locked = app.lock().await;
        (
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
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
    };
    KernelRuntimeState::new_with_owned_state(
        Arc::clone(app),
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    )
}

fn sync_external_active_prompt_and_queue_chariox_prompt(
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
        "queued from Chariox\n",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
        .prompt_owner_submit_prepared_prompt(session_id, queued_prompt, false)
        .expect("Chariox prompt should queue behind external active prompt")
    else {
        panic!("Chariox prompt must not start while external prompt is active");
    };
    (external_prompt_id, prompt.id().to_string())
}

fn assert_external_active_prompt_and_queued_chariox_prompt(
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
        "Chariox prompt should stay queued behind external active prompt"
    );
}

mod cleanup_liveness;
mod completion_settlement;
mod detached_provider_run;
mod diagnostics_timeouts;
mod external_queue;
mod forwarded_failure;
mod history_projection;
mod prompt_cancellation;
mod publication_settlement;
mod pump_selection;
mod quiet_drain_workflow;
mod structured_output;

#[test]
fn claude_native_runs_never_use_quiet_pty_settlement() {
    let native_request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id("agent-1")
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let native_run = crate::provider::RuntimeProviderRun::new(
        "provider-run-native-claude",
        &native_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-claude-test".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );

    assert!(!provider_run_allows_quiet_pty_settlement(&native_run));
}

#[test]
fn native_client_codex_runs_keep_structured_output_authority() {
    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "codex",
        "codex",
        "default",
        "gpt-5.6-sol",
    )
    .with_agent_id("agent-1")
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-native-codex",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-codex-test".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://127.0.0.1:45000".to_string()),
        },
    );

    assert!(provider_run_uses_structured_output_pump(&run));
    assert!(!provider_run_allows_quiet_pty_settlement(&run));
}
