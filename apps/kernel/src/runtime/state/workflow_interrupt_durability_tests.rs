use super::*;
use std::sync::Arc;
use tokio::sync::Mutex;

// MP-08 / MP-09 / MP-10 / MP-11 R2: the scheduler can verify and fire
// a one-shot in one tick before its caller observes the arm receipt.
#[tokio::test]
async fn a03_verification_survives_one_shot_firing_before_observation() {
    use crate::durable_state::agent_lifecycle::{AgentWake, Operation, Outcome};
    let fixture = interrupt_fixture();
    let store = &fixture.runtime.owned.durable_state_store;
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    let prompt = fixture
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &fixture.target_agent_id)
        .unwrap();
    let now = crate::session::unix_epoch_ms();
    let Outcome::Task(task) = store
        .agent_lifecycle(Operation::Begin {
            owner: session.owner_user_id().into(),
            room: fixture.session_id.clone(),
            agent: fixture.target_agent_id.clone(),
            prompt: prompt.id().into(),
            run: Some("interrupt-provider-run".into()),
            now,
        })
        .unwrap()
    else {
        panic!("task");
    };
    for (id, due) in [
        ("already-fired", now + 1_000),
        ("still-armed", now + 60_000),
    ] {
        let wake: AgentWake = serde_json::from_value(serde_json::json!({
            "id":id, "task_id":task.task_id, "room_id":task.room_id, "agent_id":task.agent_id,
            "registration_id":format!("completion-{id}"), "kind":"timer", "label":id,
            "state":"", "created_at_ms":now, "verified_at_ms":null,
            "next_due_ms":due, "interval_ms":null, "command":[], "match_text":null,
            "matched_at_ms":null, "pid":null, "exit_code":null, "fire_count":0,
            "missed_fires":0, "last_fired_at_ms":null, "last_sequence":null,
            "last_delivery":null, "last_delivered_at_ms":null,
            "last_acknowledged_at_ms":null, "alerted_sequence":null
        }))
        .unwrap();
        store
            .agent_lifecycle(Operation::CreateWake {
                task: task.task_id.clone(),
                prompt: prompt.id().into(),
                wake,
            })
            .unwrap();
    }
    store
        .agent_lifecycle(Operation::VerifyWakes { now: now + 1_000 })
        .unwrap();
    store
        .agent_lifecycle(Operation::FireWakes { now: now + 1_000 })
        .unwrap();
    let fired = store
        .agent_wakes(Some(&task.room_id), Some(&task.agent_id))
        .unwrap()
        .into_iter()
        .find(|w| w.id == "already-fired")
        .unwrap();
    assert_eq!(fired.state, "fired");
    assert_eq!(fired.verified_at_ms, Some(now + 1_000));
    assert_eq!(
        fixture.runtime.await_wake_verification("still-armed").await,
        Some(now + 1_000)
    );
    assert_eq!(
        fixture
            .runtime
            .await_wake_verification("already-fired")
            .await,
        Some(now + 1_000),
        "scheduler confirmation must survive firing before the caller observes it"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11 G14: native turn settlement is not
// workflow completion while its kernel task still owns a supervised wait.
#[tokio::test]
async fn a03_workflow_wait_keeps_returned_run_addressable_and_cancel_settles_task() {
    workflow_wait_cancellation(false).await;
}

// MP-08 / MP-09 / MP-10 / MP-11 R1: cancelling the creator owns the
// canonical workflow_run and must release its waiting child's resources.
#[tokio::test]
async fn a03_creator_task_cancels_canonical_waiting_workflow_run() {
    workflow_wait_cancellation(true).await;
}

async fn workflow_wait_cancellation(cancel_creator: bool) {
    use crate::durable_state::agent_lifecycle::{AgentWake, ExecutionState, Operation, Outcome};
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let fixture = interrupt_fixture_with_creator(
        crate::test_support::TestWorktree::new("a03-workflow-wait"),
        config,
        cancel_creator,
    );
    let owned = &fixture.runtime.owned;
    let session = owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    let prompt = owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &fixture.target_agent_id)
        .unwrap();
    owned
        .admit_agent_task(&crate::app::KernelPreparedPromptSubmission {
            session_id: fixture.session_id.clone(),
            prompt: prompt.clone(),
            force_queue: false,
            refresh_projection: false,
        })
        .unwrap();
    let now = crate::session::unix_epoch_ms();
    let Outcome::Task(task) = owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: session.owner_user_id().into(),
            room: fixture.session_id.clone(),
            agent: fixture.target_agent_id.clone(),
            prompt: prompt.id().into(),
            run: Some("interrupt-provider-run".into()),
            now,
        })
        .unwrap()
    else {
        panic!("task")
    };
    assert_eq!(
        owned.agent_workflow_task_context(&task).unwrap(),
        Some((fixture.workflow_run_id.clone(), fixture.node_run_id.clone()))
    );
    assert!(owned
        .durable_state_store
        .agent_lifecycle(Operation::BindWorkflow {
            task: task.task_id.clone(),
            prompt: prompt.id().into(),
            run: "foreign-run".into(),
            node: fixture.node_run_id.clone()
        })
        .is_err());
    let wake = AgentWake {
        id: "workflow-timer".into(),
        task_id: task.task_id.clone(),
        room_id: task.room_id.clone(),
        agent_id: task.agent_id.clone(),
        registration_id: "completion-workflow-timer".into(),
        kind: "timer".into(),
        label: "workflow wait".into(),
        state: String::new(),
        created_at_ms: now,
        verified_at_ms: None,
        next_due_ms: Some(now + 60_000),
        interval_ms: Some(60_000),
        command: vec![],
        match_text: None,
        matched_at_ms: None,
        pid: None,
        exit_code: None,
        fire_count: 0,
        missed_fires: 0,
        last_fired_at_ms: None,
        last_sequence: None,
        last_delivery: None,
        last_delivered_at_ms: None,
        last_acknowledged_at_ms: None,
        alerted_sequence: None,
    };
    owned
        .durable_state_store
        .agent_lifecycle(Operation::CreateWake {
            task: task.task_id.clone(),
            prompt: prompt.id().into(),
            wake,
        })
        .unwrap();
    owned
        .durable_state_store
        .agent_lifecycle(Operation::Yield {
            task: task.task_id.clone(),
            prompt: prompt.id().into(),
            registrations: vec!["completion-workflow-timer".into()],
            cursor: 0,
            deadline: now + 120_000,
            reason: "wait for actual source".into(),
            now,
        })
        .unwrap();
    owned
        .durable_state_store
        .agent_lifecycle(Operation::Settle {
            room: task.room_id.clone(),
            agent: task.agent_id.clone(),
            prompt: prompt.id().into(),
            run: "interrupt-provider-run".into(),
            has_answer: true,
            cancelled: false,
            now,
        })
        .unwrap();
    // Remove the same workflow's duplicate fixture prompts, then reserve the
    // actual native completion, as the provider settlement path does. Only
    // the Waiting task remains to keep this run alive.
    owned
        .prompt_state_owner
        .remove_queued_prompts_by_workflow_run(&session, &fixture.workflow_run_id);
    for agent in owned.agent_store.get_session_agents(&fixture.session_id) {
        let (active, queued) = owned.prompt_state_owner.state_parts(&session, agent.id());
        owned
            .session_store
            .mirror_agent_prompt_state(&fixture.session_id, agent.id(), active, queued)
            .unwrap();
    }
    owned
        .reserve_local_workflow_prompt_completion_if_matches(
            &fixture.session_id,
            &fixture.target_agent_id,
            prompt.id(),
        )
        .unwrap()
        .expect("reserve native completion");
    owned
        .workflow_complete_prompt(&fixture.session_id, &prompt, None)
        .unwrap();
    let run = owned
        .session_store
        .read()
        .resolve_workflow_run_ref(&fixture.session_id, &fixture.workflow_run_id)
        .expect("returned run must remain addressable during a task wait");
    assert_eq!(
        run.status(),
        crate::session::WorkflowRunStatus::Running,
        "native end must not complete a supervised workflow node"
    );
    let targets = owned
        .supervised_workflow_targets(&fixture.session_id)
        .unwrap();
    assert_eq!(
        owned
            .session_store
            .write()
            .reconcile_live_orphaned_workflow_runs_with_tasks(
                &fixture.session_id,
                u64::MAX,
                5_000,
                &targets
            )
            .unwrap(),
        0,
        "a workflow with a Waiting task is not an orphan"
    );
    // The same private task binding survives inbox turn promotion.
    owned
        .durable_state_store
        .agent_lifecycle(Operation::FireWakes { now: now + 60_000 })
        .unwrap();
    let event = owned
        .durable_state_store
        .agent_inbox(&task.room_id, &task.agent_id, 0)
        .unwrap()
        .into_iter()
        .find(|e| e.source_id == "workflow-timer")
        .unwrap();
    owned
        .durable_state_store
        .agent_lifecycle(Operation::Attempt {
            room: task.room_id.clone(),
            agent: task.agent_id.clone(),
            sequence: event.sequence,
            prompt: "workflow-event".into(),
            target: None,
            run: None,
            now: now + 60_001,
        })
        .unwrap();
    let Outcome::Task(promoted) = owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: session.owner_user_id().into(),
            room: task.room_id.clone(),
            agent: task.agent_id.clone(),
            prompt: "workflow-event".into(),
            run: Some("interrupt-provider-run".into()),
            now: now + 60_002,
        })
        .unwrap()
    else {
        panic!("promoted task")
    };
    assert_eq!(promoted.task_id, task.task_id);
    assert_eq!(
        owned.agent_workflow_task_context(&promoted).unwrap(),
        Some((fixture.workflow_run_id.clone(), fixture.node_run_id.clone()))
    );
    // MP-08 / MP-09 / MP-10 / MP-11: the real room command registers
    // workflow_run, rather than the legacy workflow spelling.
    owned
        .durable_state_store
        .agent_lifecycle(Operation::RegisterObligation {
            owner: session.owner_user_id().into(),
            room: task.room_id.clone(),
            agent: fixture.unrelated_agent_id.clone(),
            prompt: "g14-supervisor".into(),
            run: None,
            id: "g14-workflow-obligation".into(),
            kind: "workflow_run".into(),
            resource: Some(run.id().into()),
            now,
        })
        .unwrap();
    owned
        .durable_state_store
        .agent_lifecycle(Operation::DispatchReceipt {
            id: "g14-workflow-obligation".into(),
            accepted: true,
            resource: Some(run.id().into()),
        })
        .unwrap();
    if cancel_creator {
        // Observe the cancelled parent's completion source independently.
        use crate::durable_state::agent_lifecycle::Registration;
        let Outcome::Task(observer) = owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: session.owner_user_id().into(),
                room: task.room_id.clone(),
                agent: fixture.second_target_agent_id.clone(),
                prompt: "completion-observer".into(),
                run: None,
                now,
            })
            .unwrap()
        else {
            panic!("observer task");
        };
        owned
            .durable_state_store
            .agent_lifecycle(Operation::Subscribe {
                task: observer.task_id.clone(),
                prompt: observer.prompt_id.clone(),
                registration: Registration {
                    id: "parent-completion".into(),
                    task_id: observer.task_id,
                    source_id: "g14-supervisor".into(),
                    obligation_id: None,
                    source_cursor: 0,
                    live: true,
                },
            })
            .unwrap();
        let parent = owned
            .durable_state_store
            .agent_tasks(Some(&task.room_id), Some(&fixture.unrelated_agent_id))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == "g14-supervisor")
            .unwrap();
        assert_eq!(run.created_by_agent_id(), Some(parent.agent_id.as_str()));
        let Outcome::Task(cancelled) = owned
            .durable_state_store
            .agent_lifecycle(Operation::CancelTask {
                task: parent.task_id.clone(),
                owner: session.owner_user_id().into(),
                revision: parent.revision,
            })
            .unwrap()
        else {
            panic!("cancel task");
        };
        fixture
            .runtime
            .cancel_agent_task_resources(&cancelled)
            .await
            .unwrap();
    } else {
        let (cancelled, _) = fixture
            .runtime
            .execute_workflow_cancel_run_request(crate::local::CancelWorkflowRunRequest {
                session_id: fixture.session_id.clone(),
                workflow_run_ref: run.id().into(),
            })
            .await;
        let LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } = cancelled.unwrap()
        else {
            panic!("cancel response");
        };
        assert_eq!(workflow_run.id(), run.id());
    }
    // Terminal runs leave the live cache; their durable record is authoritative.
    assert_eq!(
        owned
            .durable_state_store
            .resolve_workflow_run(session.host_daemon_id(), &fixture.session_id, run.id())
            .unwrap()
            .unwrap()
            .status(),
        crate::session::WorkflowRunStatus::Stopped,
        "creator cancellation must reach the waiting child workflow"
    );
    assert_eq!(
        owned
            .durable_state_store
            .agent_tasks(Some(&task.room_id), Some(&task.agent_id))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == task.task_id)
            .unwrap()
            .state,
        ExecutionState::Cancelled
    );
    assert_eq!(
        owned
            .durable_state_store
            .agent_wakes(Some(&task.room_id), Some(&task.agent_id))
            .unwrap()[0]
            .state,
        "cancelled"
    );
    fixture.runtime.sweep_agent_lifecycle().await.unwrap();
    let parent = owned
        .durable_state_store
        .agent_tasks(Some(&task.room_id), Some(&fixture.unrelated_agent_id))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == "g14-supervisor")
        .unwrap();
    assert_ne!(
        parent.obligations[0].status, "open",
        "cancelled workflow must settle its real workflow_run obligation"
    );
    assert!(
        !owned.agent_task_resources_unsettled(&parent).unwrap(),
        "the parent completion source must no longer be held by child resources"
    );
    if cancel_creator {
        // The first sweep settles the workflow obligation; the next observes
        // the parent with no owned resources and releases its completion source.
        fixture.runtime.sweep_agent_lifecycle().await.unwrap();
        assert!(
            owned
                .durable_state_store
                .agent_inbox(&task.room_id, &fixture.second_target_agent_id, 0)
                .unwrap()
                .iter()
                .any(|e| e.source_id == "g14-supervisor" && e.kind == "source_lost"),
            "creator cancellation must release its own completion source after child settlement"
        );
    }
    assert!(owned
        .durable_state_store
        .agent_inbox(&task.room_id, &fixture.unrelated_agent_id, 0)
        .unwrap()
        .iter()
        .any(|e| e.source_id == run.id() && e.kind == "source_lost"));
}

#[tokio::test]
async fn pause_append_failure_preserves_interrupt_state_and_retries_once() {
    run_interrupt_append_failure_regression(true).await;
}

#[tokio::test]
async fn cancel_append_failure_preserves_interrupt_state_and_retries_once() {
    run_interrupt_append_failure_regression(false).await;
}

#[test]
fn paused_interrupt_prompt_removal_survives_restart() {
    assert_interrupt_prompt_removal_survives_restart(true);
}

#[test]
fn cancelled_interrupt_prompt_removal_survives_restart() {
    assert_interrupt_prompt_removal_survives_restart(false);
}

async fn run_interrupt_append_failure_regression(pause: bool) {
    let fixture = interrupt_fixture();
    let baseline_session = fixture
        .runtime
        .owned
        .session_snapshot(&fixture.session_id)
        .expect("running workflow session projection should normalize");
    assert_eq!(
        baseline_session.active_provider_run_id(),
        Some("interrupt-provider-run"),
        "projection normalization should expose the fixture's active provider"
    );
    let baseline_activity = fixture.runtime.managed_activity_snapshot();
    fixture
        .runtime
        .owned
        .session_snapshot(&fixture.session_id)
        .expect("unchanged-work control snapshot should succeed");
    assert_eq!(
        fixture.runtime.managed_activity_snapshot(),
        baseline_activity,
        "an unchanged snapshot after normalization must not advance the activity sequence"
    );
    let baseline_target_prompts = fixture
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&baseline_session, &fixture.target_agent_id);
    let baseline_unrelated_prompts = fixture
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&baseline_session, &fixture.unrelated_agent_id);
    let baseline_second_target_prompts = fixture
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&baseline_session, &fixture.second_target_agent_id);
    assert!(baseline_target_prompts.0.is_some());
    assert_eq!(baseline_target_prompts.1.len(), 2);
    assert!(baseline_second_target_prompts.0.is_some());
    assert_eq!(baseline_second_target_prompts.1.len(), 2);
    assert!(baseline_unrelated_prompts.0.is_some());
    assert!(fixture
        .runtime
        .owned
        .prompt_workspace_claims
        .contains(&fixture.claim_id));

    let reason = if pause {
        "workflow_run_paused"
    } else {
        "workflow_run_cancelled"
    };
    let injected_message = "injected composite prompt-state append failure";
    let state_path = fixture
        .runtime
        .owned
        .durable_state_store
        .path()
        .to_path_buf();
    let connection = rusqlite::Connection::open(state_path)
        .expect("durable database should open for interrupt failure injection");
    connection
        .execute_batch(&format!(
            "CREATE TRIGGER fail_workflow_interrupt_append
             BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'workflow.runtime.updated'
               AND json_extract(NEW.payload_json, '$.reason') = '{reason}'
             BEGIN
               SELECT RAISE(FAIL, '{injected_message}');
             END;"
        ))
        .expect("interrupt append failure trigger should install");
    let workflow_event_count_before = workflow_interrupt_event_count(&fixture, reason);
    let prompt_event_count_before = fixture
        .runtime
        .owned
        .durable_state_store
        .load_events_by_kind(crate::durable_prompt_state::DURABLE_PROMPT_STATE_EVENT_KIND)
        .expect("baseline prompt-state events should load")
        .len();

    let (failed, projected) = if pause {
        fixture
            .runtime
            .execute_workflow_pause_run_request(crate::local::PauseWorkflowRunRequest {
                session_id: fixture.session_id.clone(),
                workflow_run_ref: fixture.workflow_run_id.clone(),
            })
            .await
    } else {
        fixture
            .runtime
            .execute_workflow_cancel_run_request(crate::local::CancelWorkflowRunRequest {
                session_id: fixture.session_id.clone(),
                workflow_run_ref: fixture.workflow_run_id.clone(),
            })
            .await
    };
    assert!(failed
        .expect_err("failed durable append should reject workflow interrupt")
        .to_string()
        .contains(injected_message));
    assert!(
        projected.is_none(),
        "failed interrupt must not publish a fallback session snapshot"
    );
    let rolled_back = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .expect("rolled-back session should remain available");
    assert_running_workflow(&rolled_back, &fixture.workflow_run_id, &fixture.node_run_id);
    assert_eq!(
        fixture
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&rolled_back, &fixture.target_agent_id),
        baseline_target_prompts,
        "failed interrupt must preserve target active and queued prompts"
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&rolled_back, &fixture.unrelated_agent_id),
        baseline_unrelated_prompts,
        "failed interrupt must preserve another agent's prompt state"
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&rolled_back, &fixture.second_target_agent_id),
        baseline_second_target_prompts,
        "failed interrupt must preserve the second affected agent's prompt state"
    );
    assert!(
        fixture
            .runtime
            .owned
            .prompt_workspace_claims
            .contains(&fixture.claim_id),
        "failed interrupt must preserve the workflow workspace claim"
    );
    assert_eq!(
        fixture.runtime.managed_activity_snapshot(),
        baseline_activity,
        "failed interrupt must not advertise an activity transition"
    );

    let unrelated_snapshot = fixture
        .runtime
        .owned
        .session_snapshot(&fixture.session_id)
        .expect("unrelated session snapshot should remain available");
    assert_running_workflow(
        &unrelated_snapshot,
        &fixture.workflow_run_id,
        &fixture.node_run_id,
    );
    assert_eq!(
        unrelated_snapshot
            .active_prompt_for_agent(&fixture.unrelated_agent_id)
            .map(|prompt| prompt.id()),
        baseline_unrelated_prompts
            .0
            .as_ref()
            .map(|prompt| prompt.id())
    );
    assert_eq!(
        fixture.runtime.managed_activity_snapshot(),
        baseline_activity,
        "a post-rejection snapshot of unchanged work must remain sequence-neutral"
    );
    assert_eq!(
        workflow_interrupt_event_count(&fixture, reason),
        workflow_event_count_before,
        "a rejected composite write must roll back its workflow event"
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .durable_state_store
            .load_events_by_kind(crate::durable_prompt_state::DURABLE_PROMPT_STATE_EVENT_KIND)
            .expect("prompt-state events should remain readable")
            .len(),
        prompt_event_count_before,
        "a rejected workflow transaction must not append prompt records"
    );

    connection
        .execute_batch("DROP TRIGGER fail_workflow_interrupt_append;")
        .expect("interrupt append failure trigger should be removed");
    let (retried, projected) = if pause {
        fixture
            .runtime
            .execute_workflow_pause_run_request(crate::local::PauseWorkflowRunRequest {
                session_id: fixture.session_id.clone(),
                workflow_run_ref: fixture.workflow_run_id.clone(),
            })
            .await
    } else {
        fixture
            .runtime
            .execute_workflow_cancel_run_request(crate::local::CancelWorkflowRunRequest {
                session_id: fixture.session_id.clone(),
                workflow_run_ref: fixture.workflow_run_id.clone(),
            })
            .await
    };
    let interrupted = match retried.expect("workflow interrupt retry should succeed") {
        crate::local::LocalDaemonResponse::WorkflowRunPaused { workflow_run, .. }
        | crate::local::LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => {
            workflow_run
        }
        other => panic!("unexpected workflow interrupt response: {other:?}"),
    };
    let expected_status = if pause {
        crate::session::WorkflowRunStatus::Paused
    } else {
        crate::session::WorkflowRunStatus::Stopped
    };
    assert_eq!(interrupted.status(), expected_status);
    assert_eq!(
        interrupted.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Stopped
    );
    let projected = projected.expect("successful interrupt should publish a session snapshot");
    let target_prompts = fixture
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&projected, &fixture.target_agent_id);
    assert_eq!(
        target_prompts.0.as_ref().map(|prompt| prompt.status()),
        Some(crate::session::PromptStatus::Cancelling)
    );
    assert_eq!(target_prompts.1.len(), 1);
    assert!(target_prompts
        .1
        .iter()
        .all(|prompt| prompt.workflow_run_id() != Some(fixture.workflow_run_id.as_str())));
    let second_target_prompts = fixture
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&projected, &fixture.second_target_agent_id);
    assert_eq!(second_target_prompts.0, baseline_second_target_prompts.0);
    assert_eq!(second_target_prompts.1.len(), 1);
    assert!(second_target_prompts
        .1
        .iter()
        .all(|prompt| prompt.workflow_run_id() != Some(fixture.workflow_run_id.as_str())));
    assert_eq!(
        fixture
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&projected, &fixture.unrelated_agent_id),
        baseline_unrelated_prompts,
        "successful interrupt must not disturb another agent's prompts"
    );
    assert!(
        !fixture
            .runtime
            .owned
            .prompt_workspace_claims
            .contains(&fixture.claim_id),
        "successful interrupt should release its workflow workspace claim"
    );

    let owner_id = projected.host_daemon_id().to_string();
    let durable_runs = fixture
        .runtime
        .owned
        .durable_state_store
        .list_workflow_runs_page(
            &owner_id,
            &fixture.session_id,
            Some(&fixture.workflow_id),
            None,
            10,
        )
        .expect("durable interrupted workflow run should load");
    assert_eq!(durable_runs.workflow_runs.len(), 1);
    assert_eq!(durable_runs.workflow_runs[0].status(), expected_status);
    assert_eq!(
        workflow_interrupt_event_count(&fixture, reason),
        workflow_event_count_before + 1,
        "the successful retry must commit the workflow interrupt exactly once"
    );
}

fn workflow_interrupt_event_count(fixture: &InterruptFixture, reason: &str) -> usize {
    fixture
        .runtime
        .owned
        .durable_state_store
        .load_events_by_kind("workflow.runtime.updated")
        .expect("workflow runtime events should load")
        .into_iter()
        .filter(|event| event.payload["reason"] == reason)
        .count()
}

#[test]
fn unrelated_prompt_queues_survive_restart_without_interrupt() {
    let fixture = interrupt_fixture();
    drop(fixture.runtime);
    let restored =
        DaemonApp::bootstrap(fixture.config.clone()).expect("uninterrupted kernel should restore");
    let session = restored
        .sessions()
        .get_session(&fixture.session_id)
        .expect("uninterrupted session should restore");
    for (agent_id, prompt_id) in [
        (
            &fixture.target_agent_id,
            &fixture.target_unrelated_prompt_id,
        ),
        (
            &fixture.second_target_agent_id,
            &fixture.second_target_unrelated_prompt_id,
        ),
        (
            &fixture.unrelated_agent_id,
            &fixture.unrelated_queued_prompt_id,
        ),
    ] {
        let (_, queued) = restored
            .prompt_state_owner()
            .state_parts(&session, agent_id);
        assert!(
            queued.iter().any(|prompt| prompt.id() == prompt_id),
            "baseline restart must preserve {prompt_id}; restored queue: {queued:?}"
        );
    }
}

fn assert_interrupt_prompt_removal_survives_restart(pause: bool) {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("interrupt restart test executor should start");
    let restart = executor.block_on(commit_interrupt_for_restart(pause));
    drop(executor);

    let restored = DaemonApp::bootstrap(restart.config.clone())
        .expect("kernel should restore the committed interrupt");
    let restored_session = restored
        .sessions()
        .get_session(&restart.session_id)
        .expect("interrupted session should restore");
    let prompt_owner = restored.prompt_state_owner();
    for (agent_id, unrelated_prompt_id) in [
        (
            &restart.target_agent_id,
            &restart.target_unrelated_prompt_id,
        ),
        (
            &restart.second_target_agent_id,
            &restart.second_target_unrelated_prompt_id,
        ),
    ] {
        let (_, queued) = prompt_owner.state_parts(&restored_session, agent_id);
        assert!(
            queued
                .iter()
                .all(|prompt| prompt.workflow_run_id() != Some(restart.workflow_run_id.as_str())),
            "restart must not replay a queued prompt from the interrupted workflow"
        );
        assert!(
            queued
                .iter()
                .any(|prompt| prompt.id() == unrelated_prompt_id.as_str()),
            "restart must preserve unrelated queued work for {agent_id}; restored queue: {queued:?}"
        );
    }
    let (_, unrelated_queued) =
        prompt_owner.state_parts(&restored_session, &restart.unrelated_agent_id);
    assert!(
        unrelated_queued
            .iter()
            .any(|prompt| prompt.id() == restart.unrelated_queued_prompt_id.as_str()),
        "restart must preserve an unrelated agent's queued work"
    );

    let expected_status = if pause {
        crate::session::WorkflowRunStatus::Paused
    } else {
        crate::session::WorkflowRunStatus::Stopped
    };
    let durable_runs = restored
        .durable_state_store()
        .list_workflow_runs_page(
            restored_session.host_daemon_id(),
            &restart.session_id,
            Some(&restart.workflow_id),
            None,
            10,
        )
        .expect("interrupted workflow history should restore");
    assert_eq!(durable_runs.workflow_runs.len(), 1);
    assert_eq!(durable_runs.workflow_runs[0].status(), expected_status);
}

async fn commit_interrupt_for_restart(pause: bool) -> InterruptRestartAssertion {
    let fixture = interrupt_fixture();
    let result = if pause {
        fixture
            .runtime
            .execute_workflow_pause_run_request(crate::local::PauseWorkflowRunRequest {
                session_id: fixture.session_id.clone(),
                workflow_run_ref: fixture.workflow_run_id.clone(),
            })
            .await
    } else {
        fixture
            .runtime
            .execute_workflow_cancel_run_request(crate::local::CancelWorkflowRunRequest {
                session_id: fixture.session_id.clone(),
                workflow_run_ref: fixture.workflow_run_id.clone(),
            })
            .await
    };
    result
        .0
        .expect("workflow interrupt should commit before restart");
    let restart = InterruptRestartAssertion {
        config: fixture.config.clone(),
        session_id: fixture.session_id.clone(),
        workflow_id: fixture.workflow_id.clone(),
        workflow_run_id: fixture.workflow_run_id.clone(),
        target_agent_id: fixture.target_agent_id.clone(),
        second_target_agent_id: fixture.second_target_agent_id.clone(),
        unrelated_agent_id: fixture.unrelated_agent_id.clone(),
        target_unrelated_prompt_id: fixture.target_unrelated_prompt_id.clone(),
        second_target_unrelated_prompt_id: fixture.second_target_unrelated_prompt_id.clone(),
        unrelated_queued_prompt_id: fixture.unrelated_queued_prompt_id.clone(),
        _worktree: fixture._worktree,
    };
    drop(fixture.runtime);
    restart
}

struct InterruptRestartAssertion {
    config: crate::config::DaemonConfig,
    session_id: String,
    workflow_id: String,
    workflow_run_id: String,
    target_agent_id: String,
    second_target_agent_id: String,
    unrelated_agent_id: String,
    target_unrelated_prompt_id: String,
    second_target_unrelated_prompt_id: String,
    unrelated_queued_prompt_id: String,
    _worktree: crate::test_support::TestWorktree,
}

fn assert_running_workflow(
    session: &crate::session::RuntimeSession,
    workflow_run_id: &str,
    node_run_id: &str,
) {
    let workflow_run = session
        .workflow_runs()
        .iter()
        .find(|candidate| candidate.id() == workflow_run_id)
        .expect("running workflow should remain available");
    assert_eq!(
        workflow_run.status(),
        crate::session::WorkflowRunStatus::Running
    );
    assert_eq!(
        workflow_run
            .node_runs()
            .iter()
            .find(|candidate| candidate.id() == node_run_id)
            .expect("running workflow node should remain available")
            .status(),
        crate::session::WorkflowNodeRunStatus::Running
    );
}

struct InterruptFixture {
    runtime: KernelRuntimeState,
    config: crate::config::DaemonConfig,
    session_id: String,
    workflow_id: String,
    workflow_run_id: String,
    node_run_id: String,
    target_agent_id: String,
    second_target_agent_id: String,
    unrelated_agent_id: String,
    target_unrelated_prompt_id: String,
    second_target_unrelated_prompt_id: String,
    unrelated_queued_prompt_id: String,
    claim_id: String,
    _worktree: crate::test_support::TestWorktree,
}

fn interrupt_fixture() -> InterruptFixture {
    let worktree = crate::test_support::TestWorktree::new("workflow-interrupt-durability");
    interrupt_fixture_with_config(worktree, crate::config::DaemonConfig::for_tests())
}

fn interrupt_fixture_with_config(
    worktree: crate::test_support::TestWorktree,
    config: crate::config::DaemonConfig,
) -> InterruptFixture {
    interrupt_fixture_with_creator(worktree, config, false)
}

fn interrupt_fixture_with_creator(
    worktree: crate::test_support::TestWorktree,
    config: crate::config::DaemonConfig,
    agent_created: bool,
) -> InterruptFixture {
    let mut app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let target_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("interrupt-target"),
        )
        .expect("target workflow agent should be created");
    let unrelated_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("interrupt-unrelated"),
        )
        .expect("unrelated agent should be created");
    let second_target_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("interrupt-second-target"),
        )
        .expect("second affected workflow agent should be created");
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("interrupt-durability".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), target_agent.id())
        .expect("workflow node should be created");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = if agent_created {
        let queued = app
            .sessions_mut()
            .enqueue_workflow_prompt_by_agent(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("interrupt exactly once".into()),
                None,
                crate::session::WorkflowQueuedPromptSource::Manual,
                None,
                None,
                Some(unrelated_agent.id()),
            )
            .unwrap();
        app.sessions_mut()
            .invoke_queued_workflow_endpoint(session.id(), &queued)
            .unwrap()
    } else {
        app.sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("interrupt exactly once".into()),
            )
            .expect("workflow run should be created")
    };
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "interrupt exactly once".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node should be running");

    let workflow_attachment_id =
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id());
    let active_workflow_prompt = crate::session::PromptQueueItem::new(
        "interrupt-active",
        workflow_attachment_id.clone(),
        target_agent.id(),
        "active workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    let crate::session::PromptSubmissionOutcome::Started { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), active_workflow_prompt, false)
        .expect("workflow prompt should start")
    else {
        panic!("first workflow prompt should start");
    };
    let queued_workflow_prompt = crate::session::PromptQueueItem::new(
        "interrupt-queued",
        workflow_attachment_id,
        target_agent.id(),
        "queued workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_workflow_prompt, true)
        .expect("second workflow prompt should queue")
    else {
        panic!("second workflow prompt should queue");
    };
    let target_unrelated_prompt = crate::session::PromptQueueItem::new(
        "interrupt-target-unrelated-queued",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id("unrelated-target-run"),
        target_agent.id(),
        "unrelated queued prompt for target agent",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), target_unrelated_prompt, true)
        .expect("target agent's unrelated prompt should queue")
    else {
        panic!("target agent's unrelated prompt should remain queued");
    };
    // Queued submissions are re-keyed to their accepted `pending-prompt-*` identity.
    let target_unrelated_prompt_id = prompt.id().to_string();
    let second_target_active = crate::session::PromptQueueItem::new(
        "interrupt-second-target-active",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id("unrelated-second-run"),
        second_target_agent.id(),
        "second target's unrelated active prompt",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), second_target_active, false)
        .expect("second target's unrelated prompt should start")
    else {
        panic!("second target's unrelated prompt should start");
    };
    let second_target_workflow_prompt = crate::session::PromptQueueItem::new(
        "interrupt-second-target-workflow-queued",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        second_target_agent.id(),
        "second target's queued workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), second_target_workflow_prompt, true)
        .expect("second target's workflow prompt should queue")
    else {
        panic!("second target's workflow prompt should remain queued");
    };
    let second_target_unrelated_prompt = crate::session::PromptQueueItem::new(
        "interrupt-second-target-unrelated-queued",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(
            "unrelated-second-queued-run",
        ),
        second_target_agent.id(),
        "second target's unrelated queued prompt",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), second_target_unrelated_prompt, true)
        .expect("second target's unrelated prompt should queue")
    else {
        panic!("second target's unrelated prompt should remain queued");
    };
    let second_target_unrelated_prompt_id = prompt.id().to_string();
    let unrelated_prompt = crate::session::PromptQueueItem::new(
        "interrupt-unrelated-active",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id("unrelated-run"),
        unrelated_agent.id(),
        "unrelated active prompt",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), unrelated_prompt, false)
        .expect("unrelated prompt should start")
    else {
        panic!("unrelated prompt should start");
    };
    let unrelated_queued_prompt = crate::session::PromptQueueItem::new(
        "interrupt-unrelated-queued",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id("unrelated-queued-run"),
        unrelated_agent.id(),
        "unrelated agent's queued prompt",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), unrelated_queued_prompt, true)
        .expect("unrelated agent's second prompt should queue")
    else {
        panic!("unrelated agent's second prompt should remain queued");
    };
    let unrelated_queued_prompt_id = prompt.id().to_string();

    let launch_request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "workflow-interrupt-test",
    )
    .with_agent_id(target_agent.id());
    let mut provider_run = crate::provider::RuntimeProviderRun::new(
        "interrupt-provider-run",
        &launch_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "interrupt-provider-run".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("interrupt-provider-run".to_string()),
        },
    );
    provider_run.mark_running();
    app.providers_mut()
        .insert_run_for_test(provider_run.clone());
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(provider_run.id().to_string()))
        .expect("provider run should become active");
    app.update_provider_run_projection(provider_run);

    let session_id = session.id().to_string();
    let workflow_id = workflow.id().to_string();
    let workflow_run_id = workflow_run.id().to_string();
    let target_agent_id = target_agent.id().to_string();
    let second_target_agent_id = second_target_agent.id().to_string();
    let unrelated_agent_id = unrelated_agent.id().to_string();
    let runtime = runtime_state_from_app(app);
    runtime
        .owned
        .persist_workflow_runtime_session(&session_id, "interrupt_failure_test_baseline")
        .expect("running workflow baseline should persist");
    let claim_id =
        runtime
            .owned
            .workflow_dispatch_claim_id(&session_id, &workflow_run_id, &node_run_id);
    runtime
        .owned
        .acquire_workflow_node_workspace_claim(
            &session_id,
            &claim_id,
            &target_agent_id,
            &workflow_run_id,
            &node_run_id,
        )
        .expect("workflow workspace claim should be acquired");

    InterruptFixture {
        runtime,
        config,
        session_id,
        workflow_id,
        workflow_run_id,
        node_run_id,
        target_agent_id,
        second_target_agent_id,
        unrelated_agent_id,
        target_unrelated_prompt_id,
        second_target_unrelated_prompt_id,
        unrelated_queued_prompt_id,
        claim_id,
        _worktree: worktree,
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

#[tokio::test]
async fn kernel_access_workflow_control_pause_rechecks_provider_lane_wait() {
    revoked_workflow_interrupt(true).await;
}

#[tokio::test]
async fn kernel_access_workflow_control_cancel_rechecks_provider_lane_wait() {
    revoked_workflow_interrupt(false).await;
}

async fn revoked_workflow_interrupt(pause: bool) {
    let fixture = interrupt_fixture();
    let state = &fixture.runtime;
    let grant = state.insert_access_grant_for_test(&fixture.session_id);
    let request = if pause {
        LocalDaemonRequest::PauseWorkflowRun(crate::local::PauseWorkflowRunRequest {
            session_id: fixture.session_id.clone(),
            workflow_run_ref: fixture.workflow_run_id.clone(),
        })
    } else {
        LocalDaemonRequest::CancelWorkflowRun(crate::local::CancelWorkflowRunRequest {
            session_id: fixture.session_id.clone(),
            workflow_run_ref: fixture.workflow_run_id.clone(),
        })
    };
    let runtime = crate::runtime::workflow_actor::WorkflowRuntime::new(
        state.clone(),
        state.owned.session_projection.clone(),
        state.owned.agent_runtime_projection.clone(),
    );
    let before = state.owned.session_snapshot(&fixture.session_id).unwrap();
    let prompts = state
        .owned
        .prompt_state_owner
        .state_parts(&before, &fixture.target_agent_id);
    let permit = state
        .provider_runtime_lanes
        .acquire("interrupt-provider-run")
        .await;
    let probe = state
        .provider_runtime_lanes
        .notify_on_next_acquire_for_tests("interrupt-provider-run");
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "external-workflow-control",
        None,
        None,
        &request,
    );
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.clone();
    let pending = tokio::spawn({
        let runtime = runtime.clone();
        let request = request.clone();
        async move { runtime.dispatch_workflow_command(command, request).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), probe.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(permit);
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    let after = state.owned.session_snapshot(&fixture.session_id).unwrap();
    assert!(
        before == after,
        "revoked workflow control changed run/session"
    );
    assert_eq!(
        state
            .owned
            .prompt_state_owner
            .state_parts(&after, &fixture.target_agent_id),
        prompts
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "terminal-workflow-control",
        None,
        None,
        &request,
    );
    runtime
        .dispatch_workflow_command(command, request)
        .await
        .unwrap();
}

#[tokio::test]
async fn kernel_access_workflow_control_resume_rechecks_settlement_wait() {
    let fixture = interrupt_fixture();
    let state = &fixture.runtime;
    state
        .owned
        .session_store
        .write()
        .pause_workflow_run(&fixture.session_id, &fixture.workflow_run_id)
        .unwrap();
    let session = state
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    state
        .owned
        .prompt_state_owner
        .begin_cancelling_active_prompt(&session, &fixture.target_agent_id)
        .unwrap();
    let grant = state.insert_access_grant_for_test(&fixture.session_id);
    let request = LocalDaemonRequest::ResumeWorkflowRun(crate::local::ResumeWorkflowRunRequest {
        session_id: fixture.session_id.clone(),
        workflow_run_ref: fixture.workflow_run_id.clone(),
    });
    let scoped = state.with_external_command_authority(Some((&grant, &request)));
    let LocalDaemonRequest::ResumeWorkflowRun(inner) = request else {
        unreachable!()
    };
    let pending = scoped.execute_workflow_resume_run_request(inner.clone());
    tokio::pin!(pending);
    // Poll the actual continuation through its cancellation-settlement sleep.
    tokio::select! { biased;
        result = &mut pending => panic!("resume did not wait for cancellation: {}", result.0.is_ok()),
        _ = tokio::task::yield_now() => {}
    }
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    state
        .owned
        .cancel_active_prompt_only(&fixture.session_id, &fixture.target_agent_id)
        .unwrap();
    let before = state.owned.session_snapshot(&fixture.session_id).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), pending)
        .await
        .unwrap();
    let after = state.owned.session_snapshot(&fixture.session_id).unwrap();
    assert!(before == after, "revoked resume changed workflow/prompts");
    assert!(result
        .0
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    state
        .execute_workflow_resume_run_request(inner)
        .await
        .0
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_committed_workflow_control_remote_pause_survives_revocation_at_app_wait() {
    revoked_remote_workflow_interrupt(false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_committed_workflow_control_remote_pause_survives_revocation_at_discovery_wait(
) {
    revoked_remote_workflow_interrupt(true, true).await;
}

// MP-08 / MP-11: both stop dispositions acknowledge their committed settlement.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_committed_workflow_control_remote_stop_survives_revocation_at_app_wait() {
    revoked_remote_workflow_interrupt(false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_committed_workflow_control_remote_stop_survives_revocation_at_discovery_wait(
) {
    revoked_remote_workflow_interrupt(true, false).await;
}

async fn revoked_remote_workflow_interrupt(discovery: bool, pause: bool) {
    use crate::runtime::state::kernel_access::test_support::worker_spy::WorkerSpy;
    use std::sync::atomic::Ordering;
    let worker = WorkerSpy::new(discovery);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("workflow-control-fixture".into());
    let mut fixture = interrupt_fixture_with_config(
        crate::test_support::TestWorktree::new("access-workflow-interrupt"),
        config,
    );
    let probe = Arc::new(tokio::sync::Notify::new());
    fixture
        .runtime
        .observe_app_lock_wait_for_test(probe.clone());
    let state = &fixture.runtime;
    state
        .owned
        .agent_store
        .bind_remote_execution(
            &fixture.target_agent_id,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker.id.clone(),
                worker_machine_id: "fixture-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: Some("worker-run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let before = state
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    let active = state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&before, &fixture.target_agent_id)
        .unwrap();
    state
        .owned
        .mark_active_prompt_delivery(
            &fixture.session_id,
            &fixture.target_agent_id,
            active.id(),
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some("worker-run".into()),
            None,
        )
        .unwrap();
    let grant = state.insert_access_grant_for_test(&fixture.session_id);
    let request = if pause {
        LocalDaemonRequest::PauseWorkflowRun(crate::local::PauseWorkflowRunRequest {
            session_id: fixture.session_id.clone(),
            workflow_run_ref: fixture.workflow_run_id.clone(),
        })
    } else {
        LocalDaemonRequest::CancelWorkflowRun(crate::local::CancelWorkflowRunRequest {
            session_id: fixture.session_id.clone(),
            workflow_run_ref: fixture.workflow_run_id.clone(),
        })
    };
    let runtime = crate::runtime::workflow_actor::WorkflowRuntime::new(
        state.clone(),
        state.owned.session_projection.clone(),
        state.owned.agent_runtime_projection.clone(),
    );
    let guard = if discovery {
        None
    } else {
        Some(state.app.lock().await)
    };
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "external-workflow-control",
        None,
        None,
        &request,
    );
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.clone();
    let pending =
        tokio::spawn(async move { runtime.dispatch_workflow_command(command, request).await });
    let wait = if discovery {
        worker.discovery_started.clone()
    } else {
        probe
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), wait.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(guard);
    if discovery {
        worker.release_discovery.notify_one();
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        1,
        "committed workflow cancellation must reach worker"
    );
    let after = state
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    let remaining = state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&after, &fixture.target_agent_id)
        .unwrap();
    assert_eq!(remaining.id(), active.id());
    assert_eq!(remaining.status(), crate::session::PromptStatus::Cancelling);
    result.unwrap();
}

// MP-08 / MP-09 / MP-10 / MP-11 R1: canonical spelling retains the creator fence.
#[tokio::test]
async fn a03_creator_task_rejects_foreign_canonical_workflow_cancellation() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let fixture = interrupt_fixture_with_creator(
        crate::test_support::TestWorktree::new("a03-foreign-workflow-cancel"),
        config,
        true,
    );
    let store = &fixture.runtime.owned.durable_state_store;
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    let Outcome::Task(task) = store
        .agent_lifecycle(Operation::RegisterObligation {
            owner: session.owner_user_id().into(),
            room: fixture.session_id.clone(),
            agent: fixture.second_target_agent_id.clone(),
            prompt: "foreign-creator".into(),
            run: None,
            id: "foreign-workflow-obligation".into(),
            kind: "workflow_run".into(),
            resource: Some(fixture.workflow_run_id.clone()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("task");
    };
    let Outcome::Task(cancelled) = store
        .agent_lifecycle(Operation::CancelTask {
            task: task.task_id,
            owner: session.owner_user_id().into(),
            revision: task.revision,
        })
        .unwrap()
    else {
        panic!("cancel task");
    };
    let error = fixture
        .runtime
        .cancel_agent_task_resources(&cancelled)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("workflow cancellation creator binding changed"));
    assert_eq!(
        fixture
            .runtime
            .owned
            .session_store
            .read()
            .resolve_workflow_run_ref(&fixture.session_id, &fixture.workflow_run_id)
            .unwrap()
            .status(),
        crate::session::WorkflowRunStatus::Running
    );
    assert!(fixture
        .runtime
        .owned
        .agent_task_resources_unsettled(&cancelled)
        .unwrap());
}
