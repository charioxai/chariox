use std::sync::{mpsc, Arc};
use std::time::Duration;
use tokio::sync::Mutex;

const SETUP_TIMEOUT: Duration = Duration::from_secs(20);
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(2);

#[test]
fn workspace_claim_conflict_completes_without_advancing_or_releasing_the_blocker() {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_workspace_claim_conflict_scenario(ready_tx);
        }))
        .map_err(panic_message);
        let _ = result_tx.send(result);
    });

    match ready_rx.recv_timeout(SETUP_TIMEOUT) {
        Ok(()) => {}
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            panic!(
                "workspace-claim scenario exited during setup: {:?}",
                result_rx.recv_timeout(Duration::from_secs(1))
            );
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("workspace-claim scenario setup exceeded {SETUP_TIMEOUT:?}");
        }
    }
    match result_rx.recv_timeout(COMPLETION_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("workspace-claim scenario failed: {error}"),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("workspace-claim completion exceeded {COMPLETION_TIMEOUT:?}");
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            panic!("workspace-claim scenario exited without reporting its result");
        }
    }
}

fn run_workspace_claim_conflict_scenario(ready_tx: mpsc::SyncSender<()>) {
    let worktree = crate::test_support::TestWorktree::new("prompt-workspace-claim-conflict");
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "workspace-claim-conflict-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("test client should attach");
    let provider_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(provider_run.clone());
    let crate::session::PromptSubmissionOutcome::Started {
        prompt: completed_prompt,
    } = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "complete before a blocked workflow turn",
            Vec::new(),
        )
        .expect("active prompt should start")
    else {
        panic!("first prompt should start immediately");
    };

    let (queued_workflow, queued_node_run_id) = create_single_node_workflow(
        &mut app,
        session.id(),
        agent.id(),
        "queued-conflicting-workflow",
    );
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            queued_workflow.id(),
            &queued_node_run_id,
            format!("workflow-ack:{queued_node_run_id}"),
            "queued workflow turn".to_string(),
            None,
            None,
        )
        .expect("queued workflow turn should be prepared");
    let queued_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(queued_workflow.id()),
        agent.id(),
        "queued workflow turn",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(queued_workflow.id(), &queued_node_run_id);
    let crate::session::PromptSubmissionOutcome::Queued {
        prompt: queued_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("workflow prompt should remain queued behind the active prompt")
    else {
        panic!("workflow prompt should queue behind the active prompt");
    };

    let (blocker_workflow, blocker_node_run_id) =
        create_single_node_workflow(&mut app, session.id(), agent.id(), "blocking-workflow");
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            blocker_workflow.id(),
            &blocker_node_run_id,
            format!("workflow-ack:{blocker_node_run_id}"),
            "workspace claim blocker".to_string(),
            None,
            None,
        )
        .expect("blocking workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), blocker_workflow.id(), &blocker_node_run_id)
        .expect("blocking workflow node should start");
    let blocker_claim_id = format!(
        "workflow-node:{}:{}:{}",
        session.id(),
        blocker_workflow.id(),
        blocker_node_run_id
    );
    app.acquire_workflow_node_workspace_claim(
        session.id(),
        &blocker_claim_id,
        agent.id(),
        blocker_workflow.id(),
        &blocker_node_run_id,
    )
    .expect("other workflow node should hold the shared-worktree claim");

    let app = Arc::new(Mutex::new(app));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let runtime = router.runtime_state();
    runtime.owned.note_prompt_started(provider_run.id());
    assert_eq!(runtime.managed_running_agent_count(), 1);

    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session state should exist");
    let expected_active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session_state, agent.id())
        .expect("active prompt should exist");
    assert_eq!(expected_active.id(), completed_prompt.id());
    let next_queued = runtime
        .owned
        .prompt_state_owner
        .peek_next_queued_prompt(&session_state, agent.id())
        .expect("next queued workflow prompt should exist");
    assert_eq!(next_queued.id(), queued_prompt.id());

    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let provider_run_id = provider_run.id().to_string();
    let expected_prompt_id = completed_prompt.id().to_string();
    let next_queued = next_queued.clone();
    ready_tx
        .send(())
        .expect("outer timeout harness should still be waiting");
    let completion = runtime
        .owned
        .complete_local_prompt_with_queued_advance_if_matches(
            &session_id,
            &agent_id,
            Some(&provider_run_id),
            &next_queued,
            Some(&expected_prompt_id),
        )
        .expect("claim-conflict completion should succeed")
        .expect("completion should match the active prompt");
    assert_eq!(completion.completion.completed.id(), completed_prompt.id());
    assert_eq!(
        completion.completion.completed.status(),
        crate::session::PromptStatus::Completed
    );
    assert!(completion.completion.started_next.is_none());

    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("completed session snapshot should load");
    assert!(snapshot.active_prompt_for_agent(agent.id()).is_none());
    let queued = snapshot
        .queued_prompts_for_agent(agent.id())
        .expect("queued workflow prompt should remain projected");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id(), queued_prompt.id());
    assert_eq!(queued[0].status(), crate::session::PromptStatus::Queued);
    assert!(runtime
        .owned
        .operational_history_store
        .load_prompt_settlement_event(session.id(), agent.id(), completed_prompt.id())
        .expect("completed prompt settlement should load")
        .is_some());

    let next_claim_id = runtime.owned.workflow_dispatch_claim_id(
        session.id(),
        queued_workflow.id(),
        &queued_node_run_id,
    );
    assert!(runtime
        .owned
        .prompt_workspace_claims
        .contains(&blocker_claim_id));
    assert!(!runtime
        .owned
        .prompt_workspace_claims
        .contains(&next_claim_id));
    assert_eq!(runtime.owned.workspace_coordinator.active_claims().len(), 1);
    assert!(runtime.owned.active_turns.get(provider_run.id()).is_none());
    assert_eq!(
        runtime.managed_running_agent_count(),
        1,
        "queued work must continue to count as managed activity after the active turn settles"
    );
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else {
        "scenario panicked with a non-string payload".to_string()
    }
}

fn create_single_node_workflow(
    app: &mut crate::app::DaemonApp,
    session_id: &str,
    agent_id: &str,
    alias: &str,
) -> (crate::session::WorkflowRun, String) {
    let workflow = app
        .sessions_mut()
        .create_workflow(session_id, Some(alias.to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session_id, workflow.id(), agent_id)
        .expect("workflow node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session_id,
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            Some(alias.to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run
        .node_runs()
        .first()
        .expect("workflow run should contain its entry node")
        .id()
        .to_string();
    (workflow_run, node_run_id)
}
