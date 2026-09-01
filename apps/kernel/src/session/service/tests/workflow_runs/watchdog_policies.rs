use super::*;

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
            None,
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
    let slow_queue = service
        .create_workflow_prompt_queue(session.id(), workflow.id(), "slow".to_string(), -10)
        .expect("slow queue should be created");
    let watchdog = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("slow"),
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
    let queued_prompt = service
        .get_session(session.id())
        .expect("session should exist")
        .workflow_queued_prompts()
        .iter()
        .find(|prompt| prompt.watchdog_id() == Some(watchdog.id()))
        .expect("watchdog prompt should be queued")
        .clone();
    assert_eq!(queued_prompt.queue_id(), slow_queue.id());
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
fn workflow_watchdog_runs_when_only_another_agent_is_busy() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);
    let active = workflow_with_endpoint(&mut service, session.id(), "active", "agent-1");
    let scheduled = workflow_with_endpoint(&mut service, session.id(), "scheduled", "agent-2");
    service
        .invoke_workflow_endpoint(
            session.id(),
            active.workflow.id(),
            active.endpoint.id(),
            Some("keep agent one busy".to_string()),
        )
        .expect("first workflow should invoke");
    let watchdog = service
        .create_workflow_watchdog(
            session.id(),
            scheduled.workflow.id(),
            scheduled.endpoint.id(),
            None,
            1,
            "run independently".to_string(),
            WorkflowWatchdogPolicy::Queue,
            None,
        )
        .expect("watchdog should create");

    let plans = service
        .collect_due_workflow_watchdog_invocations(watchdog.next_run_at_ms())
        .expect("watchdog collection should succeed");

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].workflow_id, scheduled.workflow.id());
    assert_eq!(plans[0].endpoint_id, scheduled.endpoint.id());
}

#[test]
fn publication_runtime_watchdogs_are_collected_from_hidden_materialized_session() {
    let mut service = SessionService::new(&test_config());
    let source_session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("source session should be created");
    seed_agents(&mut service, source_session.id(), &["agent-1"]);
    let source = workflow_with_endpoint(&mut service, source_session.id(), "published", "agent-1");
    let workflow = service
        .resolve_workflow_ref(source_session.id(), source.workflow.id())
        .expect("workflow should resolve with its endpoint");
    let mut watchdog = crate::session::WorkflowWatchdogDefinition::new(
        "published-watchdog",
        workflow.id(),
        source.endpoint.id(),
        60,
        "scheduled publication prompt",
        WorkflowWatchdogPolicy::Queue,
        Some(2),
    );
    watchdog.set_next_run_at_ms(0);

    let runtime_session = service
        .create_session(
            CreateSessionRequest::new("publication-workspace", "publication-worktree")
                .with_hidden(true),
        )
        .expect("publication runtime session should be created");
    let materialized = service
        .replace_publication_runtime_workflows(
            runtime_session.id(),
            vec![workflow.clone()],
            vec![crate::session::WorkflowPromptQueueDefinition::default_queue(workflow.id())],
            vec![watchdog.clone()],
        )
        .expect("publication runtime workflows should materialize");
    assert!(materialized.is_hidden());
    assert_eq!(materialized.workflow_watchdogs(), &[watchdog.clone()]);

    let warmup_plans = service
        .collect_due_workflow_watchdog_invocations(0)
        .expect("publication watchdog should defer during warm-up");
    assert!(warmup_plans.is_empty());
    let warming = service
        .resolve_workflow_watchdog_ref(runtime_session.id(), watchdog.id())
        .expect("materialized watchdog should resolve after warm-up deferral");
    assert_eq!(warming.last_status(), Some("warming_up"));
    assert!(warming.next_run_at_ms() >= materialized.created_at_ms());

    let plans = service
        .collect_due_workflow_watchdog_invocations(warming.next_run_at_ms())
        .expect("publication watchdog should collect");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].watchdog_id, watchdog.id());
    assert_eq!(plans[0].session_id, runtime_session.id());
    assert_eq!(plans[0].workflow_id, workflow.id());
    assert_eq!(plans[0].endpoint_id, source.endpoint.id());
    assert_eq!(plans[0].invocation_prompt, "scheduled publication prompt");

    let updated = service
        .resolve_workflow_watchdog_ref(runtime_session.id(), watchdog.id())
        .expect("materialized watchdog should resolve");
    assert_eq!(updated.last_status(), Some("invoking"));
    assert_eq!(updated.next_run_at_ms(), warming.next_run_at_ms() + 60_000);
}
