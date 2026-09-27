use std::sync::Arc;
use std::time::Duration;

use crate::agent::RemoteAgentBinding;
use crate::app::{DaemonApp, KernelSessionService};
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::runtime::router::CommandRouter;
use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus, PromptSubmissionOutcome};

#[tokio::test]
async fn remote_queue_advance_returns_successor_and_records_once() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-queue-advance",
            "worktree-queue-advance",
        ))
        .expect("session should be created");
    let attachment = KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-remote-queue-advance",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should be created");
    app.agents
        .bind_remote_execution(
            agent.id(),
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

    let active = app
        .prompt_owner_submit_prepared_prompt(
            session.id(),
            PromptQueueItem::new(
                "queue-advance-active",
                attachment.id(),
                agent.id(),
                "active remote prompt",
                PromptStatus::Queued,
            ),
            false,
        )
        .expect("active remote prompt should be admitted");
    assert!(matches!(active.outcome, PromptSubmissionOutcome::Started { .. }));
    let successor = app
        .prompt_owner_submit_prepared_prompt(
            session.id(),
            PromptQueueItem::new(
                "queue-advance-successor",
                attachment.id(),
                agent.id(),
                "remote successor prompt",
                PromptStatus::Queued,
            ),
            false,
        )
        .expect("remote successor should be queued");
    assert!(matches!(
        successor.outcome,
        PromptSubmissionOutcome::Queued { .. }
    ));
    app.prompt_owner_cancel_active_prompt_only(session.id(), agent.id())
        .expect("active prompt should cancel while successor stays queued");

    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let runtime = router.runtime_state();
    let records_before = runtime.managed_activity_record_call_count_for_test();
    let advance_runtime = runtime.clone();
    let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
    let advance_thread = std::thread::spawn(move || {
        let result = advance_runtime
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
            });
        result_tx
            .send(result)
            .expect("bounded regression should still be waiting for remote promotion");
    });

    let (prompt_id, prompt_text, dispatch_prompt_id, dispatch_prompt_text) = result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("remote queue promotion should return within two seconds")
        .expect("remote queue promotion should return the queued successor");
    advance_thread
        .join()
        .expect("remote queue promotion thread should join");

    assert_eq!(prompt_text, "remote successor prompt");
    assert_eq!(dispatch_prompt_id, prompt_id);
    assert_eq!(dispatch_prompt_text, prompt_text);
    let session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should remain available");
    let active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, agent.id())
        .expect("promoted successor should be active");
    assert_eq!(active.id(), prompt_id);
    assert_eq!(
        runtime.managed_activity_record_call_count_for_test() - records_before,
        1,
        "one queue promotion should record one activity mutation"
    );
}
