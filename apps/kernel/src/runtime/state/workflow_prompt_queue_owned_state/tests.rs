use super::*;
use std::sync::{Arc, Barrier};
use tokio::sync::Mutex;

#[test]
fn concurrent_owned_workflow_launches_preserve_single_run_admission() {
    const INVOCATION_COUNT: usize = 32;

    let (runtime, session_id, workflow_id, endpoint_id, _test_root) = runtime_with_idle_workflow();
    let barrier = Arc::new(Barrier::new(INVOCATION_COUNT));
    let mut handles = Vec::with_capacity(INVOCATION_COUNT);
    for index in 0..INVOCATION_COUNT {
        let runtime = runtime.clone();
        let barrier = Arc::clone(&barrier);
        let session_id = session_id.clone();
        let workflow_id = workflow_id.clone();
        let endpoint_id = endpoint_id.clone();
        handles.push(
            std::thread::Builder::new()
                .name(format!("owned-workflow-launch-{index}"))
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    barrier.wait();
                    runtime.owned.workflow_enqueue_prompt_and_maybe_start(
                        &session_id,
                        &workflow_id,
                        &endpoint_id,
                        Some(format!("concurrent owned invocation {index}")),
                        None,
                        None,
                    )
                })
                .expect("owned workflow launch thread should spawn"),
        );
    }

    let outcomes = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
        })
        .collect::<Vec<_>>();
    let errors = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().err().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "concurrent invokes failed: {errors:?}");
    let started = outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome,
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { .. },
                    _
                ))
            )
        })
        .count();
    let enqueued = outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome,
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. },
                    _
                ))
            )
        })
        .count();
    assert_eq!(started, 1, "exactly one concurrent invoke should start");
    assert_eq!(enqueued, INVOCATION_COUNT - 1);

    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("session should remain available");
    let active_runs = session
        .workflow_runs()
        .iter()
        .filter(|workflow_run| {
            matches!(
                workflow_run.status(),
                crate::session::WorkflowRunStatus::Created
                    | crate::session::WorkflowRunStatus::Running
                    | crate::session::WorkflowRunStatus::Waiting
            )
        })
        .count();
    assert_eq!(active_runs, 1, "concurrent admission created extra runs");
    assert_eq!(
        session.workflow_queued_prompts().len(),
        INVOCATION_COUNT - 1
    );
}

#[test]
fn owned_launch_response_tracks_requested_prompt_when_older_prompt_starts() {
    let (runtime, session_id, workflow_id, endpoint_id, _test_root) = runtime_with_idle_workflow();
    let older_prompt = runtime
        .owned
        .session_store
        .write()
        .enqueue_workflow_prompt(
            &session_id,
            &workflow_id,
            &endpoint_id,
            Some("older queued invocation".to_string()),
            None,
            crate::session::WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("older prompt should be queued");

    let (outcome, _dispatches) = runtime
        .owned
        .workflow_enqueue_prompt_and_maybe_start(
            &session_id,
            &workflow_id,
            &endpoint_id,
            Some("new requested invocation".to_string()),
            None,
            None,
        )
        .expect("new prompt should advance the older queued work");
    let requested_prompt = match outcome {
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { queued_prompt, .. } => {
            queued_prompt
        }
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { .. } => {
            panic!("new invocation must not report the older prompt's run as its own")
        }
    };
    assert_ne!(requested_prompt.id(), older_prompt.id());
    assert_eq!(requested_prompt.prompt(), Some("new requested invocation"));
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("session should remain available");
    assert_eq!(session.workflow_runs().len(), 1);
    assert_eq!(
        session.workflow_runs()[0].invocation_prompt(),
        Some("older queued invocation")
    );
    assert_eq!(
        session
            .workflow_queued_prompts()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![requested_prompt]
    );
}

fn runtime_with_idle_workflow() -> (KernelRuntimeState, String, String, String, TestRoot) {
    let test_root = TestRoot(std::env::temp_dir().join(format!(
        "arroba-owned-workflow-admission-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    )));
    std::fs::create_dir_all(&test_root.0).expect("workflow test root should be created");
    let test_root_path = test_root.0.to_string_lossy().to_string();
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            &test_root_path,
            &test_root_path,
        ))
        .expect("session should be created");
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("owned-workflow-agent"),
        )
        .expect("workflow agent should be created");
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("owned-workflow".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node_owned(
            session.id(),
            workflow.id(),
            agent.id(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            "Owned workflow node".to_string(),
        )
        .expect("workflow node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "workflow-test-idle",
    )
    .with_agent_id(agent.id());
    let mut provider_run = crate::provider::RuntimeProviderRun::new(
        "owned-workflow-provider",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "owned-workflow-provider".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("owned-workflow-provider".to_string()),
        },
    );
    provider_run.mark_running();
    app.providers_mut()
        .insert_run_for_test(provider_run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(provider_run.id().to_string()))
        .expect("provider run should become active");
    app.update_provider_run_projection(provider_run);

    let session_id = session.id().to_string();
    let workflow_id = workflow.id().to_string();
    let endpoint_id = endpoint.id().to_string();
    (
        runtime_state_from_app(app),
        session_id,
        workflow_id,
        endpoint_id,
        test_root,
    )
}

struct TestRoot(std::path::PathBuf);

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn runtime_state_from_app(app: DaemonApp) -> KernelRuntimeState {
    let config_projection = app.config_projection_store();
    let session_store = app.session_state_store();
    let agent_store = app.agents().clone();
    let attachment_store = app.attachments().clone();
    let provider_store = app.providers().clone();
    let provider_process_tracking = app.provider_process_tracking_store();
    let slice_store = app.slices();
    let session_projection = app.session_state_projection_store();
    let provider_run_projection = app.provider_run_projection_store();
    let operational_history_store = app.operational_history_store();
    let durable_state_store = app.durable_state_store();
    let prompt_state_owner = app.prompt_state_owner();
    let active_turns = app.active_turn_store();
    let prompt_activity = app.prompt_activity_store();
    let prompt_workspace_claims = app.prompt_workspace_claim_store();
    let structured_output_records = app.structured_output_record_store();
    let terminal_stream = app.terminal_stream_store();
    let workflow_design_events = app.workflow_design_event_store();
    let metaagent_events = app.metaagent_event_store();
    let workspace_coordinator = app.workspace_coordinator();
    KernelRuntimeState::new_with_owned_state(
        Arc::new(Mutex::new(app)),
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
