use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};

// MP-08 / MP-10 / MP-11 (amfix item 1): a Room's turn can end after the lane
// that began it was lost (restart, re-attach). Its end must still be assessed.
struct StartedTurn {
    app: Arc<Mutex<DaemonApp>>,
    session_id: String,
    agent_id: String,
    attachment_id: String,
    run_id: String,
    prompt: crate::session::PromptQueueItem,
    _worktree: crate::test_support::TestWorktree,
}

fn started_turn(label: &str) -> StartedTurn {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "amfix-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let run = app
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
        .unwrap();
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "work",
            Vec::new(),
        )
        .unwrap()
    else {
        panic!("prompt should start")
    };
    StartedTurn {
        session_id: session.id().into(),
        agent_id: agent.id().into(),
        attachment_id: attachment.id().into(),
        run_id: run.id().into(),
        prompt,
        app: Arc::new(Mutex::new(app)),
        _worktree: worktree,
    }
}

fn append_answer(runtime: &KernelRuntimeState, turn: &StartedTurn) {
    let answer = crate::history::SessionHistoryEntry::provider_output(
        &turn.session_id,
        &turn.run_id,
        Some(&turn.agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        "The requested work is finished.",
    );
    runtime
        .owned
        .operational_history_store
        .append_transcript(&answer, Default::default())
        .unwrap();
}

fn begin(runtime: &KernelRuntimeState, turn: &StartedTurn) {
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt.id().into(),
            run: Some(turn.run_id.clone()),
            now: 1,
        })
        .unwrap();
}

fn task_of(
    runtime: &KernelRuntimeState,
    turn: &StartedTurn,
) -> crate::durable_state::agent_lifecycle::AgentTaskExecution {
    runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&turn.session_id), Some(&turn.agent_id))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == turn.prompt.id())
        .unwrap()
}

// Whatever lost a turn end, an answered turn whose prompt already settled is
// assessed by the sweep, not blocked.
#[tokio::test]
async fn sweep_assesses_an_answered_turn_whose_end_never_reached_the_ledger() {
    let turn = started_turn("amfix-sweep-answered-turn");
    let runtime = owned_runtime_state(&turn.app).await;
    begin(&runtime, &turn);
    append_answer(&runtime, &turn);
    // The prompt settles without the ledger ever seeing the turn end.
    runtime
        .owned
        .complete_local_prompt_without_advance(&turn.session_id, &turn.agent_id, Some(&turn.run_id))
        .unwrap();
    runtime.sweep_agent_lifecycle().await.unwrap();
    let task = task_of(&runtime, &turn);
    assert!(
        !task.reason.contains("No live provider turn"),
        "an answered turn must be assessed, not blocked: {:?} {}",
        task.state,
        task.reason
    );
    assert_ne!(task.state, ExecutionState::Working);
}

// After a restart the client that sent the turn is gone; the kernel-owned
// assessment of its answer must go out through a client that is attached now.
#[tokio::test]
async fn assessment_of_a_restored_turn_survives_its_source_client_being_gone() {
    assessment_after_source_client_left(true).await;
}

// The user closed the CLI while the agent worked: nobody is attached at all.
#[tokio::test]
async fn assessment_of_a_turn_ending_with_no_client_attached_is_not_blocked() {
    assessment_after_source_client_left(false).await;
}

async fn assessment_after_source_client_left(reattach: bool) {
    let turn = started_turn("amfix-restored-assessment");
    let runtime = owned_runtime_state(&turn.app).await;
    let store = &runtime.owned.durable_state_store;
    begin(&runtime, &turn);
    append_answer(&runtime, &turn);
    store
        .agent_lifecycle(Operation::StartGoal {
            task: turn.prompt.id().into(),
            owner: "local".into(),
            objective: "Answer the request".into(),
            minutes: 1,
            now: 1,
        })
        .unwrap();
    let Outcome::Settled { task, correction } = store
        .agent_lifecycle(Operation::Settle {
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt.id().into(),
            run: turn.run_id.clone(),
            has_answer: true,
            cancelled: false,
            now: 2,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    assert!(correction, "the completion check must be dispatched");
    runtime
        .owned
        .complete_local_prompt_without_advance(&turn.session_id, &turn.agent_id, Some(&turn.run_id))
        .unwrap();
    // Restart: the sending client detaches and a new one attaches.
    runtime.owned.detach(&turn.attachment_id).unwrap();
    if reattach {
        runtime
            .owned
            .attach(crate::attachment::AttachRequest::new(
                &turn.session_id,
                "amfix-client-after-restart",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
    }
    runtime
        .finish_agent_task_settlement(Some((task, correction)), &turn.prompt)
        .await
        .unwrap();
    let task = task_of(&runtime, &turn);
    assert!(
        !task.reason.contains("Correction delivery failed"),
        "the assessment must not die with the old client: {:?} {}",
        task.state,
        task.reason
    );
    assert_ne!(task.state, ExecutionState::Blocked);
}

// A restart can land after an assessment persisted Continue but before its
// continuation was queued. The sweep must queue that exact continuation, not
// re-read the settled assessment forever.
#[tokio::test]
async fn sweep_queues_a_continuation_persisted_before_restart() {
    let turn = started_turn("amfix-sweep-pending-continuation");
    let runtime = owned_runtime_state(&turn.app).await;
    let store = &runtime.owned.durable_state_store;
    begin(&runtime, &turn);
    append_answer(&runtime, &turn);
    store
        .agent_lifecycle(Operation::StartGoal {
            task: turn.prompt.id().into(),
            owner: "local".into(),
            objective: "Answer the request".into(),
            minutes: 1,
            now: 1,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt.id().into(),
            run: turn.run_id.clone(),
            has_answer: true,
            cancelled: false,
            now: 2,
        })
        .unwrap();
    let check = task_of(&runtime, &turn).pending_prompt_id.unwrap();
    let check_run = "check-run";
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: check.clone(),
            run: Some(check_run.into()),
            now: 3,
        })
        .unwrap();
    let mut asked = crate::history::SessionHistoryEntry::user_prompt(
        &turn.session_id,
        &turn.attachment_id,
        &turn.agent_id,
        "check",
    );
    asked.merge_key = Some(format!("prompt:{check}"));
    let answered = crate::history::SessionHistoryEntry::provider_output(
        &turn.session_id,
        check_run,
        Some(&turn.agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        "{}",
    );
    for entry in [&asked, &answered] {
        runtime
            .owned
            .operational_history_store
            .append_transcript(entry, Default::default())
            .unwrap();
    }
    store
        .agent_lifecycle(Operation::EvaluateGoal {
            task: turn.prompt.id().into(),
            prompt: check,
            result: r#"{"status":"continue","reason":"Answer the request"}"#.into(),
            now: 4,
        })
        .unwrap();
    let before = task_of(&runtime, &turn);
    let continuation = before
        .pending_prompt_id
        .clone()
        .expect("continuation intent");
    runtime
        .owned
        .complete_local_prompt_without_advance(&turn.session_id, &turn.agent_id, Some(&turn.run_id))
        .unwrap();
    runtime.sweep_agent_lifecycle().await.unwrap();
    let task = task_of(&runtime, &turn);
    assert_eq!(task.state, ExecutionState::Working, "{}", task.reason);
    assert_eq!(
        task.prompt_id, continuation,
        "the persisted continuation must be admitted, not left pending"
    );
    // A second sweep finds the continuation in flight and changes nothing.
    runtime.sweep_agent_lifecycle().await.unwrap();
    assert_eq!(task_of(&runtime, &turn), task);
}

// A leased task binds the projected `leased:<lease>:<worker-run>` id while its
// public history carries the worker's own run id.
#[tokio::test]
async fn sweep_assesses_an_answered_leased_turn_by_its_worker_history_run() {
    let turn = started_turn("amfix-sweep-leased-turn");
    let runtime = owned_runtime_state(&turn.app).await;
    let worker_run = "worker-run-1";
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt.id().into(),
            run: Some(crate::provider::projected_leased_provider_run_id(
                "lease-1", worker_run,
            )),
            now: 1,
        })
        .unwrap();
    let answer = crate::history::SessionHistoryEntry::provider_output(
        &turn.session_id,
        worker_run,
        Some(&turn.agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        "The requested work is finished.",
    );
    runtime
        .owned
        .operational_history_store
        .append_transcript(&answer, Default::default())
        .unwrap();
    runtime
        .owned
        .complete_local_prompt_without_advance(&turn.session_id, &turn.agent_id, Some(&turn.run_id))
        .unwrap();
    runtime.sweep_agent_lifecycle().await.unwrap();
    let task = task_of(&runtime, &turn);
    assert!(
        !task.reason.contains("No live provider turn"),
        "an answered leased turn must be assessed, not blocked: {:?} {}",
        task.state,
        task.reason
    );
    assert_ne!(task.state, ExecutionState::Working);
}

// accam A02-S03: a structured provider (Codex) acknowledges the owner's Stop
// with an abort receipt and no terminal event. That receipt ends the task as
// Cancelled; it must not stay Working until the supervisor blocks it.
#[tokio::test]
async fn acknowledged_stop_cancels_the_task_with_its_open_work() {
    let turn = started_turn("a02-s03-acknowledged-stop");
    {
        let mut app = turn.app.lock().await;
        app.prompt_owner_begin_cancelling_active_prompt(&turn.session_id, &turn.agent_id)
            .unwrap();
        app.providers_mut().inject_finished_abort_for_test(
            crate::provider::FinishedProviderPromptAbortJob {
                session_id: turn.session_id.clone(),
                provider_run_id: turn.run_id.clone(),
                prompt_id: turn.prompt.id().into(),
                result: Ok(()),
            },
        );
    }
    let runtime = owned_runtime_state(&turn.app).await;
    begin(&runtime, &turn);
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::RegisterObligation {
            owner: "local".into(),
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt.id().into(),
            run: Some(turn.run_id.clone()),
            id: "build".into(),
            kind: "process".into(),
            resource: None,
            now: 1,
        })
        .unwrap();
    runtime.reap_structured_prompt_jobs_and_dispatch();
    let task = task_of(&runtime, &turn);
    assert_eq!(task.state, ExecutionState::Cancelled, "{}", task.reason);
    assert!(task.wait.is_none());
}
