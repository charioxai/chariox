use super::provider_output_runtime::{
    provider_run_allows_quiet_pty_settlement, provider_run_ids_for_owned_output_pump,
    provider_run_uses_structured_output_pump,
};
use super::*;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

mod inert_pty;
use inert_pty::{spawn_inert_pty_for_run, InertPtyCleanup};

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

mod app_quiet_tool_guard;
mod approval_lifetime;
mod browser_import_execution_gate;
mod cleanup_liveness;
mod completion_settlement;
mod detached_provider_run;
mod diagnostics_timeouts;
mod external_queue;
mod history_projection;
mod large_codex_resume;
mod leased_output;
mod mcp_catalog_reload;
#[cfg(unix)]
mod project_queued_environment;
mod prompt_cancellation;
mod prompt_parking;
mod publication_settlement;
mod pump_selection;
mod quiet_drain_workflow;
mod structured_exit_diagnostic;
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

mod file_pick_revocation;

// MP-08/MP-10: capability continuations survive the requesting client's detach.
#[tokio::test]
async fn mcp_catalog_continuation_uses_kernel_attachment_after_client_detach() {
    let scratch = std::env::temp_dir().join(format!(
        "chariox-extfix-continuation-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            crate::session::CreateSessionRequest::new(
                scratch.to_string_lossy(),
                scratch.to_string_lossy(),
            )
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .unwrap();
    let attachment = app
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "extfix-client",
            crate::attachment::ClientCapabilityLevel::AutomationOnly,
            agent.owner_user_id(),
        ))
        .unwrap();
    let continuation = PendingMcpContinuation {
        session_id: session.id().into(),
        agent_id: agent.id().into(),
        mcp_name: "mid_session_script".into(),
        previous_prompt: "invoke the granted script".into(),
        reload_reason: ProviderReloadReason::RuntimeToolCatalog,
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.owned.detach(attachment.id()).unwrap();
    let source = runtime
        .ensure_mcp_continuation_attachment(&continuation)
        .unwrap();
    assert_ne!(source, attachment.id());
    assert_eq!(
        source,
        runtime
            .ensure_mcp_continuation_attachment(&continuation)
            .unwrap()
    );
    let prepared = crate::app::KernelPreparedPromptSubmission {
        session_id: session.id().into(),
        prompt: crate::session::PromptQueueItem::new(
            "extfix-continuation",
            &source,
            agent.id(),
            &continuation.previous_prompt,
            crate::session::PromptStatus::Queued,
        ),
        force_queue: false,
        refresh_projection: true,
    };
    let submission = runtime.submit_prepared_prompt(prepared).await.unwrap();
    assert!(matches!(
        submission.outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let attribution = runtime
        .owned
        .ensure_attachment_in_session(session.id(), &source)
        .unwrap();
    assert_eq!(attribution.owner_user_id(), agent.owner_user_id());
    runtime.owned.detach(&source).unwrap();
    std::fs::remove_dir_all(scratch).unwrap();
}

// MP-08/MP-10: a persistent grant does not make a newly registered definition live.
#[tokio::test]
async fn mcp_catalog_reregistration_marks_existing_grant_pending_synchronously() {
    let root = std::env::temp_dir().join(format!(
        "chariox-extfix-registration-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            crate::session::CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            )
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("opencode")),
        )
        .unwrap();
    app.launch_provider(
        crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "default",
        )
        .with_agent_id(agent.id()),
    )
    .unwrap();
    // The inert launch selects dev-stub; restore the provider policy under test
    // without launching or authenticating an official provider in this unit test.
    app.agents()
        .update_agent_profile(agent.id(), Some("opencode".into()), None, None)
        .unwrap();
    let agent = app
        .agents()
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::script("extfix_registration", "extfix_python"),
        )
        .unwrap();
    assert_eq!(agent.provider(), "opencode");
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    // MP-08/MP-10: other daemon fixtures reuse agent IDs in the shared
    // continuation store. This catalog-only fixture owns an empty store.
    runtime.owned.pending_mcp_continuations = PendingMcpContinuationStore::default();
    let previous = runtime.runtime_catalog_signature_for_agent(&agent);
    assert_eq!(
        runtime.runtime_catalog_grant_effect(&agent, true),
        ("now", false)
    );
    let source = root.join("fixture.py");
    std::fs::write(&source, "def run() -> str:\n    \"\"\"Return a fixture result.\"\"\"\n    return 'ok'\n\ndef test_run():\n    \"\"\"Validate fixture.\"\"\"\n    assert run() == 'ok'\n").unwrap();
    let registry = crate::script::CharioxScriptRegistry::new(vec![
        crate::script::CharioxScriptRegistry::project_root(&root),
    ]);
    registry
        .install(
            &source,
            Some("extfix_registration"),
            &crate::script::CharioxEnvironmentConfig {
                name: "extfix_python".into(),
                runtime: crate::script::CharioxEnvironmentRuntime::Python {
                    python: "/usr/bin/python3".into(),
                },
            },
        )
        .unwrap();
    runtime.runtime_catalog_registration_changed(&agent, &previous);
    assert_eq!(
        runtime.runtime_catalog_grant_effect(&agent, true),
        ("after_provider_reload", true),
        "MP-08/MP-10 re-registration must report the outstanding reload"
    );
    runtime
        .owned
        .pending_provider_reloads
        .write()
        .remove(agent.id());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
mod credential_copy_recovery;
mod worker_copied_claude;

mod popup_notices;
