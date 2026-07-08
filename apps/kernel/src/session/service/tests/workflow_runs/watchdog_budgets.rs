use super::*;

#[test]
fn workflow_watchdog_defaults_to_bounded_max_wakeups() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("watchdog".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");

    let watchdog = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            None,
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Skip,
            None,
        )
        .expect("watchdog should be created");

    assert_eq!(
        watchdog.max_wakeups(),
        Some(crate::session::DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS),
    );
    assert_eq!(watchdog.wakeups_executed(), 0);
}

#[test]
fn workflow_watchdog_budget_can_be_unbounded_or_auto_disable_when_exhausted() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("watchdog".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");

    let bounded = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            None,
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Skip,
            Some(Some(1)),
        )
        .expect("bounded watchdog should be created");
    let unbounded = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            None,
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Skip,
            Some(None),
        )
        .expect("unbounded watchdog should be created");

    let bounded = service
        .mark_workflow_watchdog_invoked(session.id(), bounded.id(), "workflow-run-1")
        .expect("bounded watchdog should update");
    assert_eq!(bounded.max_wakeups(), Some(1));
    assert_eq!(bounded.wakeups_executed(), 1);
    assert!(!bounded.enabled());
    assert_eq!(bounded.last_status(), Some("completed_budget"));

    let unbounded = service
        .mark_workflow_watchdog_invoked(session.id(), unbounded.id(), "workflow-run-2")
        .expect("unbounded watchdog should update");
    assert_eq!(unbounded.max_wakeups(), None);
    assert_eq!(unbounded.wakeups_executed(), 1);
    assert!(unbounded.enabled());
    assert_eq!(unbounded.last_status(), Some("started"));
}

#[test]
fn workflow_watchdog_queued_start_is_rejected_after_budget_is_exhausted() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("watchdog".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");

    let watchdog = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            None,
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Queue,
            Some(Some(1)),
        )
        .expect("watchdog should be created");
    service
        .mark_workflow_watchdog_queued(session.id(), watchdog.id())
        .expect("watchdog should be queued");
    service
        .mark_workflow_watchdog_invoked(session.id(), watchdog.id(), "workflow-run-1")
        .expect("watchdog should consume budget");

    let allowed = service
        .prepare_workflow_watchdog_queued_start(session.id(), watchdog.id())
        .expect("stale queued start should be evaluated");
    assert!(!allowed);
    let watchdog = service
        .resolve_workflow_watchdog_ref(session.id(), watchdog.id())
        .expect("watchdog should resolve");
    assert_eq!(watchdog.max_wakeups(), Some(1));
    assert_eq!(watchdog.wakeups_executed(), 1);
    assert!(!watchdog.enabled());
    assert!(!watchdog.pending_run());
    assert_eq!(watchdog.last_status(), Some("completed_budget"));
    assert_eq!(watchdog.last_workflow_run_id(), Some("workflow-run-1"));
}
