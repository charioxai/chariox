use super::*;

#[test]
fn creates_lists_resolves_and_cancels_workflow_runs() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    assert_eq!(workflow_run.workflow_id(), workflow.id());
    assert_eq!(workflow_run.endpoint_id(), endpoint.id());
    assert_eq!(workflow_run.entry_node_id(), node.id());
    assert_eq!(workflow_run.status(), WorkflowRunStatus::Created);
    assert_eq!(workflow_run.node_runs().len(), 1);
    assert_eq!(
        workflow_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Ready
    );
    assert_eq!(workflow_run.messages().len(), 1);
    assert_eq!(workflow_run.messages()[0].target_node_id(), node.id());

    let listed = service
        .list_workflow_runs(session.id(), Some(workflow.id()))
        .expect("workflow runs should list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id(), workflow_run.id());

    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(resolved.id(), workflow_run.id());

    let cancelled = service
        .cancel_workflow_run(session.id(), workflow_run.id())
        .expect("workflow run should cancel");
    assert_eq!(cancelled.status(), WorkflowRunStatus::Stopped);
    assert_eq!(cancelled.active_node_run_id(), None);
    assert_eq!(
        cancelled.node_runs()[0].status(),
        WorkflowNodeRunStatus::Stopped
    );

    let error = service
        .cancel_workflow_run(session.id(), workflow_run.id())
        .expect_err("terminal workflow run should reject a second cancellation");
    assert!(matches!(error, DaemonError::InvalidWorkflowRunState { .. }));
}

#[test]
fn provider_failure_marks_workflow_and_node_failed() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();

    let failed = service
        .fail_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node should fail");

    assert_eq!(failed.status(), WorkflowRunStatus::Failed);
    assert_eq!(failed.active_node_run_id(), None);
    assert_eq!(
        failed.node_runs()[0].status(),
        WorkflowNodeRunStatus::Failed
    );
}

#[test]
fn node_turn_budget_exhaustion_stops_the_whole_run() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    service
        .set_workflow_node_max_turns(session.id(), workflow.id(), node.id(), Some(1))
        .expect("node max turns should update");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    let node_run = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist");

    let update = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            node_run.id(),
            Some(WorkflowCompletionSnapshot::new("done", None)),
            None,
        )
        .expect("node completion should succeed");

    assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Stopped);
    assert!(update.dispatches.is_empty());
    assert!(update.workflow_run.final_output().is_none());
}

#[test]
fn manual_workflow_launch_rejects_while_any_session_workflow_run_is_active() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let first_workflow = service
        .create_workflow(session.id(), Some("first".to_string()))
        .expect("first workflow should be created");
    let first_node = service
        .add_workflow_node(session.id(), first_workflow.id(), "agent-1")
        .expect("first node should be added");
    let first_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            first_workflow.id(),
            first_node.id(),
            Some("entry".to_string()),
        )
        .expect("first endpoint should be created");

    let second_workflow = service
        .create_workflow(session.id(), Some("second".to_string()))
        .expect("second workflow should be created");
    let second_node = service
        .add_workflow_node(session.id(), second_workflow.id(), "agent-2")
        .expect("second node should be added");
    let second_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            second_workflow.id(),
            second_node.id(),
            Some("entry".to_string()),
        )
        .expect("second endpoint should be created");

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            first_workflow.id(),
            first_endpoint.id(),
            Some("go".to_string()),
        )
        .expect("first workflow run should be created");
    assert_eq!(workflow_run.status(), WorkflowRunStatus::Created);

    let error = service
        .admit_manual_workflow_launch(
            session.id(),
            second_workflow.id(),
            second_endpoint.id(),
            Some("later".to_string()),
        )
        .expect_err("launch should reject while a session workflow run is active");
    assert!(matches!(error, DaemonError::WorkflowLaunchRejected { .. }));
}

#[test]
fn manual_workflow_launch_queue_is_fifo_across_workflows() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let first_workflow = service
        .create_workflow(session.id(), Some("first".to_string()))
        .expect("first workflow should be created");
    let first_node = service
        .add_workflow_node(session.id(), first_workflow.id(), "agent-1")
        .expect("first node should be added");
    let first_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            first_workflow.id(),
            first_node.id(),
            Some("entry".to_string()),
        )
        .expect("first endpoint should be created");

    let second_workflow = service
        .create_workflow(session.id(), Some("second".to_string()))
        .expect("second workflow should be created");
    let second_node = service
        .add_workflow_node(session.id(), second_workflow.id(), "agent-2")
        .expect("second node should be added");
    let second_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            second_workflow.id(),
            second_node.id(),
            Some("entry".to_string()),
        )
        .expect("second endpoint should be created");

    service
        .set_workflow_launch_policy(session.id(), WorkflowLaunchPolicy::Queue)
        .expect("queue policy should be set");
    let active = service
        .invoke_workflow_endpoint(
            session.id(),
            first_workflow.id(),
            first_endpoint.id(),
            Some("go".to_string()),
        )
        .expect("active workflow run should be created");
    assert_eq!(active.status(), WorkflowRunStatus::Created);

    let first_queued = service
        .admit_manual_workflow_launch(
            session.id(),
            second_workflow.id(),
            second_endpoint.id(),
            Some("second".to_string()),
        )
        .expect("second workflow should queue");
    let second_queued = service
        .admit_manual_workflow_launch(
            session.id(),
            first_workflow.id(),
            first_endpoint.id(),
            Some("third".to_string()),
        )
        .expect("third launch should queue");

    let queued = service
        .list_queued_workflow_launches(session.id())
        .expect("queued launches should list");
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].source(), QueuedWorkflowLaunchSource::Manual);
    assert_eq!(queued[1].source(), QueuedWorkflowLaunchSource::Manual);

    match first_queued {
        WorkflowLaunchAdmission::Queued(ref queued_launch) => {
            assert_eq!(queued[0].id(), queued_launch.id())
        }
        WorkflowLaunchAdmission::StartNow => panic!("expected queued launch"),
    }
    match second_queued {
        WorkflowLaunchAdmission::Queued(ref queued_launch) => {
            assert_eq!(queued[1].id(), queued_launch.id())
        }
        WorkflowLaunchAdmission::StartNow => panic!("expected queued launch"),
    }

    service
        .cancel_workflow_run(session.id(), active.id())
        .expect("active workflow run should stop");
    let dequeued = service
        .dequeue_next_workflow_launch(session.id())
        .expect("queued workflow launch should dequeue")
        .expect("expected queued workflow launch");
    assert_eq!(dequeued.id(), queued[0].id());
}

#[test]
fn workflow_console_supports_append_read_and_clear() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");

    let initial = service
        .read_workflow_console(session.id(), workflow.id())
        .expect("console should read");
    assert_eq!(initial.workflow_id(), workflow.id());
    assert!(initial.entries().is_empty());

    let first = service
        .append_workflow_console_entry(
            session.id(),
            workflow.id(),
            Some("node-run-1".to_string()),
            Some("agent-1".to_string()),
            "hello\n",
        )
        .expect("console append should succeed");
    assert_eq!(first.text(), "hello\n");

    let second = service
        .append_workflow_console_entry(
            session.id(),
            workflow.id(),
            Some("node-run-2".to_string()),
            Some("agent-2".to_string()),
            "world\n",
        )
        .expect("console append should succeed");
    assert_eq!(second.text(), "world\n");

    let populated = service
        .read_workflow_console(session.id(), workflow.id())
        .expect("console should read");
    assert_eq!(populated.entries().len(), 2);
    assert_eq!(populated.entries()[0].text(), "hello\n");
    assert_eq!(populated.entries()[1].text(), "world\n");

    let cleared = service
        .clear_workflow_console(session.id(), workflow.id())
        .expect("console clear should succeed");
    assert!(cleared.entries().is_empty());
}

#[test]
fn workflow_watchdog_skip_policy_skips_when_endpoint_run_is_active() {
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
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Skip,
            None,
        )
        .expect("watchdog should be created");
    let run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("manual".to_string()),
        )
        .expect("workflow should invoke");
    let plans = service
        .collect_due_workflow_watchdog_invocations(watchdog.next_run_at_ms())
        .expect("watchdog collection should succeed");
    assert!(plans.is_empty());
    let watchdog = service
        .resolve_workflow_watchdog_ref(session.id(), watchdog.id())
        .expect("watchdog should resolve");
    assert_eq!(watchdog.last_status(), Some("skipped_running"));
    assert!(!watchdog.pending_run());
    assert_eq!(run.status(), WorkflowRunStatus::Created);
}

#[test]
fn workflow_watchdog_queue_policy_queues_one_pending_run() {
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
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Queue,
            None,
        )
        .expect("watchdog should be created");
    let run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("manual".to_string()),
        )
        .expect("workflow should invoke");
    let queued = service
        .collect_due_workflow_watchdog_invocations(watchdog.next_run_at_ms())
        .expect("watchdog collection should succeed");
    assert!(queued.is_empty());
    let watchdog = service
        .resolve_workflow_watchdog_ref(session.id(), watchdog.id())
        .expect("watchdog should resolve");
    assert_eq!(watchdog.last_status(), Some("queued_running"));
    assert!(watchdog.pending_run());

    let session_mut = service
        .store
        .get_mut(session.id())
        .expect("session should exist");
    let active_run = session_mut
        .workflow_run_mut(run.id())
        .expect("workflow run should exist");
    active_run.set_status(WorkflowRunStatus::Completed);

    let plans = service
        .collect_due_workflow_watchdog_invocations(unix_epoch_ms())
        .expect("watchdog collection should succeed");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].endpoint_id, endpoint.id());
    let watchdog = service
        .resolve_workflow_watchdog_ref(session.id(), watchdog.id())
        .expect("watchdog should resolve");
    assert!(!watchdog.pending_run());
    assert_eq!(watchdog.last_status(), Some("invoking_pending"));
}

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
