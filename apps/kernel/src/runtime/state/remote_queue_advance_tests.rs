use std::sync::Arc;
use std::time::Duration;

use crate::agent::RemoteAgentBinding;
use crate::app::{KernelPreparedPromptSubmission, KernelSessionService};
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::provider::LaunchProviderRequest;
use crate::runtime::router::CommandRouter;
use crate::session::{
    PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
};

use super::KernelRuntimeState;

struct QueueFixture {
    _worktree: crate::test_support::TestWorktree,
    runtime: KernelRuntimeState,
    remote: bool,
    session_id: String,
    agent_id: String,
    attachment_id: String,
    provider_run_id: Option<String>,
}

async fn queue_fixture(remote: bool, local_provider_run: bool) -> QueueFixture {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .expect("authenticated daemon fixture should bootstrap");
    let worktree = crate::test_support::TestWorktree::new("quiescence-queue-advance");
    let (session, agent) = KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-quiescence-queue-advance",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();

    if remote {
        app.agents()
            .bind_remote_execution(
                &agent_id,
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-queue-advance".to_string(),
                    worker_machine_id: "worker-machine-queue-advance".to_string(),
                    execution_lease_id: "lease-queue-advance".to_string(),
                    leased_agent_id: "leased-agent-queue-advance".to_string(),
                    active_worker_provider_run_id: Some("worker-run-queue-advance".to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
    }

    let provider_run_id = if local_provider_run {
        // The dev-stub adapter creates an in-memory run record and starts no provider process.
        let started = app
            .providers()
            .start_run_provider_only(
                LaunchProviderRequest::new(
                    &session_id,
                    "dev-stub",
                    agent.provider(),
                    "default",
                    "queue-advance-test-model",
                )
                .with_agent_id(&agent_id),
            )
            .expect("test provider run should be created");
        let run = app
            .providers()
            .mark_run_running(started.run().id())
            .expect("test provider run should become running");
        app.sessions_mut()
            .set_active_provider_run(&session_id, Some(run.id().to_string()))
            .expect("session should reference the test provider run");
        Some(run.id().to_string())
    } else {
        None
    };

    let kernel_id = app.config().daemon_id.clone();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let registration = ConfirmedManagedKernelRegistration {
        environment_id: "queue-advance-test-environment".to_string(),
        machine_id: "queue-advance-test-machine".to_string(),
        kernel_id: kernel_id.clone(),
        context_plan: None,
    };
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1)
        .with_managed_kernel_registration(registration);
    let runtime = router.runtime_state();
    runtime
        .ensure_managed_activity_tracking(&kernel_id)
        .expect("managed activity tracking should initialize");
    runtime
        .managed_activity_report_snapshot_with_transition()
        .expect("managed activity fixture should have a durable initial observation");

    QueueFixture {
        _worktree: worktree,
        runtime,
        remote,
        session_id,
        agent_id,
        attachment_id,
        provider_run_id,
    }
}

fn submit_prompt(fixture: &QueueFixture, text: &str) -> PromptQueueItem {
    let prompt_id = format!(
        "queue-advance-{}",
        text.to_ascii_lowercase().replace(' ', "-")
    );
    let prepared = KernelPreparedPromptSubmission {
        session_id: fixture.session_id.clone(),
        prompt: PromptQueueItem::new(
            prompt_id,
            &fixture.attachment_id,
            &fixture.agent_id,
            text,
            PromptStatus::Queued,
        ),
        force_queue: false,
        refresh_projection: true,
    };
    let submission = if fixture.remote {
        fixture
            .runtime
            .owned
            .submit_remote_prepared_prompt(&prepared)
    } else {
        fixture
            .runtime
            .owned
            .submit_local_prepared_prompt(&prepared)
    }
    .expect("prepared prompt should be admitted")
    .expect("runtime should handle the fixture prompt");

    match submission.outcome {
        PromptSubmissionOutcome::Started { prompt }
        | PromptSubmissionOutcome::Queued { prompt } => prompt,
    }
}

fn cancel_active(fixture: &QueueFixture) {
    fixture
        .runtime
        .owned
        .cancel_active_prompt_only(&fixture.session_id, &fixture.agent_id)
        .expect("active prompt should cancel with queued work preserved");
}

fn run_bounded<T: Send + 'static>(
    label: &'static str,
    action: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let _ = result_tx.send(action());
    });
    let result = result_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|error| panic!("{label} did not return within two seconds: {error}"));
    worker
        .join()
        .expect("bounded queue operation thread should join");
    result
}

fn assert_busy_activity_is_durable(runtime: &KernelRuntimeState) {
    let (_, observation, transition_sequence) = runtime
        .managed_activity_report_snapshot_with_transition()
        .expect("managed activity observation should remain durable");
    assert_eq!(observation.running_agent_count, 1);
    assert!(transition_sequence > 0);
}

fn assert_active_prompt(fixture: &QueueFixture, expected_prompt_id: &str) {
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .expect("session should remain available");
    let active = fixture
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &fixture.agent_id)
        .expect("promoted prompt should be active");
    assert_eq!(active.id(), expected_prompt_id);
}

#[tokio::test]
async fn remote_queue_advance_returns_successor_and_records_once() {
    let fixture = queue_fixture(true, false).await;
    submit_prompt(&fixture, "active remote prompt");
    submit_prompt(&fixture, "remote successor prompt");
    cancel_active(&fixture);
    let records_before = fixture
        .runtime
        .managed_activity_record_call_count_for_test();

    let runtime = fixture.runtime.clone();
    let session_id = fixture.session_id.clone();
    let agent_id = fixture.agent_id.clone();
    let (prompt_id, prompt_text, dispatch_prompt_id, dispatch_prompt_text) =
        run_bounded("remote queue advancement", move || {
            runtime
                .owned
                .advance_next_queued_remote_prompt_dispatch(&session_id, &agent_id)
                .map_err(|error| error.to_string())
                .and_then(|submission| {
                    let submission = submission.ok_or_else(|| {
                        "remote queue advancement returned no submission".to_string()
                    })?;
                    let prompt = match submission.outcome {
                        PromptSubmissionOutcome::Started { prompt } => prompt,
                        PromptSubmissionOutcome::Queued { .. } => {
                            return Err("remote successor remained queued".to_string());
                        }
                    };
                    let dispatch = submission
                        .remote_dispatch
                        .ok_or_else(|| "remote successor has no dispatch record".to_string())?;
                    Ok((
                        prompt.id().to_string(),
                        prompt.prompt().to_string(),
                        dispatch.prompt_id,
                        dispatch.prompt,
                    ))
                })
        })
        .expect("remote queue advancement should return its successor");

    assert_eq!(prompt_text, "remote successor prompt");
    assert_eq!(dispatch_prompt_id, prompt_id);
    assert_eq!(dispatch_prompt_text, prompt_text);
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .expect("session should remain available");
    let active = fixture
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &fixture.agent_id)
        .expect("promoted remote successor should be active");
    assert_eq!(active.id(), prompt_id);
    assert_eq!(
        fixture
            .runtime
            .managed_activity_record_call_count_for_test()
            - records_before,
        1,
        "remote queue promotion should record one activity mutation"
    );
    assert_busy_activity_is_durable(&fixture.runtime);
}

#[tokio::test]
async fn local_queue_activation_and_dispatch_transfer_the_activity_guard_once() {
    let fixture = queue_fixture(false, true).await;
    submit_prompt(&fixture, "active local prompt");
    let first_successor = submit_prompt(&fixture, "first local successor");
    submit_prompt(&fixture, "second local successor");
    cancel_active(&fixture);

    let records_before_activation = fixture
        .runtime
        .managed_activity_record_call_count_for_test();
    let runtime = fixture.runtime.clone();
    let session_id = fixture.session_id.clone();
    let agent_id = fixture.agent_id.clone();
    let expected_prompt_id = first_successor.id().to_string();
    let (activated_id, activated_text) = run_bounded("local queue activation", move || {
        runtime
            .owned
            .activate_next_queued_prompt_for_agent(
                &session_id,
                &agent_id,
                Some(&expected_prompt_id),
            )
            .map_err(|error| error.to_string())
            .and_then(|prompt| {
                prompt
                    .map(|prompt| (prompt.id().to_string(), prompt.prompt().to_string()))
                    .ok_or_else(|| "local queue activation returned no successor".to_string())
            })
    })
    .expect("local queue activation should return the first successor");
    assert_eq!(activated_text, "first local successor");
    assert_eq!(
        fixture
            .runtime
            .managed_activity_record_call_count_for_test()
            - records_before_activation,
        1,
        "local queue activation should record one activity mutation"
    );
    assert_active_prompt(&fixture, &activated_id);
    assert_busy_activity_is_durable(&fixture.runtime);

    cancel_active(&fixture);
    let records_before_dispatch = fixture
        .runtime
        .managed_activity_record_call_count_for_test();
    let runtime = fixture.runtime.clone();
    let session_id = fixture.session_id.clone();
    let agent_id = fixture.agent_id.clone();
    let provider_run_id = fixture
        .provider_run_id
        .clone()
        .expect("local dispatch fixture should have a provider run");
    let (dispatched_id, dispatched_text) = run_bounded("local queue dispatch advance", move || {
        runtime
            .owned
            .advance_next_queued_prompt_dispatch(&session_id, &agent_id, &provider_run_id)
            .map_err(|error| error.to_string())
            .and_then(|dispatch| {
                dispatch
                    .map(|dispatch| (dispatch.prompt_id, dispatch.prompt))
                    .ok_or_else(|| "local queue dispatch returned no successor".to_string())
            })
    })
    .expect("local queue dispatch should return the second successor");
    assert_eq!(dispatched_text, "second local successor");
    assert_ne!(activated_id, dispatched_id);
    assert_eq!(
        fixture
            .runtime
            .managed_activity_record_call_count_for_test()
            - records_before_dispatch,
        1,
        "local queue dispatch advance should record one activity mutation"
    );
    assert_active_prompt(&fixture, &dispatched_id);
    assert_busy_activity_is_durable(&fixture.runtime);
}
