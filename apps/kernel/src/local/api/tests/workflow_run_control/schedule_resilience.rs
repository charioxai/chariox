use super::*;
use crate::local::{CreateWorkflowScheduleRequest, DestroyAgentRequest};

#[test]
fn failed_schedule_tick_does_not_leave_a_queued_prompt() {
    run_schedule_resilience_large_stack_test(
        "failed-schedule-tick-does-not-leave-a-queued-prompt",
        failed_schedule_tick_does_not_leave_a_queued_prompt_inner,
    );
}

fn failed_schedule_tick_does_not_leave_a_queued_prompt_inner() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            harness.fixture_session_request("workspace-schedule-failure"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = harness.spawn_workflow_test_agent(session.id(), "scheduled-agent");
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("scheduled-workflow".to_string()),
        }))
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = harness.add_workflow_test_node(session.id(), workflow.id(), agent.id());
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("scheduled-entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let schedule = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowSchedule(
            CreateWorkflowScheduleRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                queue_ref: Some("default".to_string()),
                trigger: crate::session::WorkflowScheduleTrigger::interval(1),
                invocation_prompt: "scheduled prompt".to_string(),
                overlap_policy: crate::session::WorkflowScheduleOverlapPolicy::Queue,
                max_runs_configured: true,
                max_runs: Some(2),
            },
        ))
        .expect("workflow schedule should be created")
    {
        LocalDaemonResponse::WorkflowScheduleCreated { schedule, .. } => schedule,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        let mut sessions = app.sessions_mut();
        for index in 0..256 {
            sessions
                .enqueue_workflow_prompt(
                    session.id(),
                    workflow.id(),
                    endpoint.id(),
                    Some(format!("stale scheduled prompt {index}")),
                    Some("default"),
                    crate::session::WorkflowQueuedPromptSource::Scheduled,
                    Some(schedule.id().to_string()),
                )
                .expect("stale scheduled prompt should seed");
        }
    });
    match harness
        .dispatch(LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
        }))
        .expect("workflow agent should be destroyed")
    {
        LocalDaemonResponse::AgentDestroyed { .. } => {}
        _ => panic!("unexpected local response"),
    }

    let wait_ms = schedule
        .next_run_at_ms()
        .saturating_sub(crate::session::unix_epoch_ms())
        .saturating_add(20);
    std::thread::sleep(Duration::from_millis(wait_ms));
    harness.pump_transport_runtime();

    let updated = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
    });
    assert!(updated.workflow_queued_prompts().is_empty());
    let updated_schedule = updated
        .workflow_schedules()
        .iter()
        .find(|candidate| candidate.id() == schedule.id())
        .expect("schedule should still exist");
    assert_eq!(updated_schedule.last_status(), Some("invoke_failed"));
    assert!(!updated_schedule.enabled());

    let persisted_state = harness.with_app(|app| {
        app.durable_state_store()
            .load_subject_events(session.id(), 20)
            .expect("durable session events should load")
            .into_iter()
            .find(|event| {
                event.kind == "workflow.runtime.updated"
                    && event.payload["reason"] == "workflow_schedule_tick"
            })
            .expect("schedule tick should persist a bounded workflow transition");
        app.durable_state_store()
            .load_workflow_hot_states(session.host_daemon_id())
            .expect("durable workflow hot state should load")
            .into_iter()
            .find(|(session_id, _)| session_id == session.id())
            .map(|(_, state)| state)
            .expect("persisted schedule state should exist")
    });
    assert!(persisted_state.workflow_queued_prompts.is_empty());
    assert!(!persisted_state
        .workflow_schedules
        .iter()
        .find(|candidate| candidate.id() == schedule.id())
        .expect("persisted schedule should exist")
        .enabled());
}

#[test]
fn disabled_schedule_queue_recovers_without_duplicates_or_busy_polling() {
    run_schedule_resilience_large_stack_test(
        "disabled-schedule-queue-recovers-without-duplicates-or-busy-polling",
        disabled_schedule_queue_recovers_without_duplicates_or_busy_polling_inner,
    );
}

fn disabled_schedule_queue_recovers_without_duplicates_or_busy_polling_inner() {
    schedule_queue_recovers_with_owner(false);
}

#[test]
fn mp11_busy_schedule_queue_records_owner_before_start() {
    run_schedule_resilience_large_stack_test("busy-schedule-queue-owner", || {
        schedule_queue_recovers_with_owner(true)
    });
}

fn schedule_queue_recovers_with_owner(entry_busy: bool) {
    let worktree = crate::test_support::TestWorktree::new("schedule-queue");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = harness.spawn_workflow_test_agent(session.id(), "queued-scheduled-agent");
    harness.launch_workflow_test_provider(session.id(), agent.id());
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("queued-scheduled-workflow".to_string()),
        }))
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = harness.add_workflow_test_node(session.id(), workflow.id(), agent.id());
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("queued-scheduled-entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let schedule = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowSchedule(
            CreateWorkflowScheduleRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                queue_ref: Some("default".to_string()),
                trigger: crate::session::WorkflowScheduleTrigger::interval(1),
                invocation_prompt: "queued scheduled prompt".to_string(),
                overlap_policy: crate::session::WorkflowScheduleOverlapPolicy::Queue,
                max_runs_configured: true,
                max_runs: Some(2),
            },
        ))
        .expect("workflow schedule should be created")
    {
        LocalDaemonResponse::WorkflowScheduleCreated { schedule, .. } => schedule,
        _ => panic!("unexpected local response"),
    };
    let busy_run = entry_busy.then(|| {
        harness.with_app_mut(|app| {
            app.sessions_mut()
                .invoke_workflow_endpoint(
                    session.id(),
                    workflow.id(),
                    endpoint.id(),
                    Some("keep entry busy".into()),
                )
                .unwrap()
        })
    });
    update_default_queue_enabled(&harness, session.id(), workflow.id(), false);

    let wait_ms = schedule
        .next_run_at_ms()
        .saturating_sub(crate::session::unix_epoch_ms())
        .saturating_add(20);
    std::thread::sleep(Duration::from_millis(wait_ms));
    harness.pump_transport_runtime();
    harness.pump_transport_runtime();

    let queued = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
    });
    assert_eq!(queued.workflow_queued_prompts().len(), 1);
    assert_eq!(queued.workflow_runs().len(), usize::from(entry_busy));
    let queued_id = queued.workflow_queued_prompts()[0].id();
    assert_eq!(
        harness.with_app(|app| app
            .durable_state_store()
            .workflow_invocation_owner(session.host_daemon_id(), session.id(), queued_id)
            .unwrap()),
        Some(crate::session::DEFAULT_LOCAL_USER_ID.into()),
        "MP-11: a busy schedule captures its owner when it queues, before launch"
    );
    assert!(queued
        .workflow_schedules()
        .iter()
        .find(|candidate| candidate.id() == schedule.id())
        .expect("schedule should still exist")
        .pending_run());
    assert!(
        harness.transport_runtime_pump_interval_ms(500, 5_000, crate::session::unix_epoch_ms(),)
            > 0
    );

    if let Some(run) = busy_run {
        harness.with_app_mut(|app| {
            app.sessions_mut()
                .stop_workflow_run_with_final_output(session.id(), run.id(), None, None, None, None)
                .unwrap()
        });
    }
    update_default_queue_enabled(&harness, session.id(), workflow.id(), true);
    harness.pump_transport_runtime();

    let resumed = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
    });
    assert!(resumed.workflow_queued_prompts().is_empty());
    assert_eq!(resumed.workflow_runs().len(), 1); // The stopped fixture run is archived.
                                                  // MP-08/MP-10/MP-11 review: inspect the private owner's real router projection,
                                                  // not only the scheduler's unredacted application state.
    let run_id = resumed
        .workflow_runs()
        .iter()
        .find(|run| run.queue_item_id() == Some(queued_id))
        .unwrap()
        .id();
    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflowRuns(
            ListWorkflowRunsRequest {
                session_id: session.id().into(),
                workflow_ref: None,
                cursor: None,
                limit: None,
            },
        ))
        .unwrap();
    let LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } = listed else {
        panic!("unexpected list");
    };
    assert!(
        workflow_runs.iter().any(|run| run.id() == run_id),
        "MP-11: a private owner sees its scheduled run"
    );
    let got = harness
        .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().into(),
            workflow_run_ref: run_id.into(),
        }))
        .unwrap();
    let LocalDaemonResponse::WorkflowRun { workflow_run } = got else {
        panic!("unexpected get")
    };
    assert!(!workflow_run.created_before_ownership_tracking());
    assert!(matches!(
        harness
            .dispatch(LocalDaemonRequest::PauseWorkflowRun(
                crate::local::PauseWorkflowRunRequest {
                    session_id: session.id().into(),
                    workflow_run_ref: run_id.into()
                },
            ))
            .unwrap(),
        LocalDaemonResponse::WorkflowRunPaused { .. }
    ));
    assert!(matches!(
        harness
            .dispatch(LocalDaemonRequest::CancelWorkflowRun(
                crate::local::CancelWorkflowRunRequest {
                    session_id: session.id().into(),
                    workflow_run_ref: run_id.into()
                },
            ))
            .unwrap(),
        LocalDaemonResponse::WorkflowRunCancelled { .. }
    ));

    let resumed_schedule = resumed
        .workflow_schedules()
        .iter()
        .find(|candidate| candidate.id() == schedule.id())
        .expect("schedule should still exist");
    assert_eq!(resumed_schedule.wakeups_executed(), 1);
    assert!(!resumed_schedule.pending_run());
}

fn run_schedule_resilience_large_stack_test(name: &str, test: fn()) {
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(test)
        .expect("schedule resilience test thread should spawn");
    if let Err(payload) = handle.join() {
        std::panic::resume_unwind(payload);
    }
}

fn update_default_queue_enabled(
    harness: &LocalRouterTestHarness,
    session_id: &str,
    workflow_id: &str,
    enabled: bool,
) {
    match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowPromptQueue(
            UpdateWorkflowPromptQueueRequest {
                session_id: session_id.to_string(),
                workflow_ref: Some(workflow_id.to_string()),
                queue_ref: "default".to_string(),
                alias: None,
                priority: None,
                enabled: Some(enabled),
            },
        ))
        .expect("default queue should update")
    {
        LocalDaemonResponse::WorkflowPromptQueueUpdated { .. } => {}
        _ => panic!("unexpected local response"),
    }
}
