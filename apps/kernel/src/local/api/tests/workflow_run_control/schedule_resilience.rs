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
            CreateSessionRequest::new("workspace-schedule-failure", "worktree-schedule-failure"),
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

    let persisted = harness.with_app(|app| {
        app.durable_state_store()
            .load_subject_events(session.id(), 20)
            .expect("durable session events should load")
            .into_iter()
            .find(|event| {
                event.kind == "session.updated"
                    && event.payload["reason"] == "workflow_schedule_tick"
            })
            .expect("schedule tick should persist the updated session")
    });
    let persisted_session: crate::session::RuntimeSession =
        serde_json::from_value(persisted.payload["session"].clone())
            .expect("persisted schedule session should decode");
    assert!(persisted_session.workflow_queued_prompts().is_empty());
    assert!(!persisted_session
        .workflow_schedules()
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
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-schedule-queue", "worktree-schedule-queue"),
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
    assert!(queued.workflow_runs().is_empty());
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

    update_default_queue_enabled(&harness, session.id(), workflow.id(), true);
    harness.pump_transport_runtime();

    let resumed = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
    });
    assert!(resumed.workflow_queued_prompts().is_empty());
    assert_eq!(resumed.workflow_runs().len(), 1);
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
