use super::*;

struct TestWorkflowEndpoint {
    workflow: crate::session::WorkflowDefinition,
    endpoint: crate::session::WorkflowEndpointDefinition,
}

fn workflow_with_endpoint(
    service: &mut SessionService,
    session_id: &str,
    alias: &str,
    agent_id: &str,
) -> TestWorkflowEndpoint {
    let workflow = service
        .create_workflow(session_id, Some(alias.to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session_id, workflow.id(), agent_id)
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session_id,
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    TestWorkflowEndpoint { workflow, endpoint }
}

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
fn workflow_run_keeps_publication_invocation_metadata_separate_from_prompt() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "published", "agent-1");
    let publication_invocation = crate::session::WorkflowPublicationInvocationEnvelope {
        publication_id: "publication-1".to_string(),
        hook_id: Some("hook-1".to_string()),
        invocation_id: "req-1".to_string(),
        transport: "human_http".to_string(),
        endpoint_id: endpoint.id().to_string(),
        queue_ref: Some("default".to_string()),
        input: serde_json::json!({ "prompt": "make tea" }),
        artifacts: Vec::new(),
        mode: Some("sync".to_string()),
        caller: serde_json::json!({ "type": "anonymous" }),
    };

    let workflow_run = service
        .invoke_workflow_endpoint_with_publication_invocation(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("make tea".to_string()),
            Some(publication_invocation),
        )
        .expect("workflow run should be created");

    assert_eq!(workflow_run.invocation_prompt(), Some("make tea"));
    let metadata = workflow_run
        .publication_invocation()
        .expect("publication invocation should be stored on workflow run");
    assert_eq!(metadata.invocation_id(), "req-1");
    assert_eq!(metadata.input, serde_json::json!({ "prompt": "make tea" }));
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
fn workflow_prompt_can_be_enqueued_while_a_workflow_run_is_active() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let workflow = service
        .create_workflow(session.id(), Some("queued".to_string()))
        .expect("workflow should be created");
    let first_node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("first node should be added");
    let first_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            first_node.id(),
            Some("first".to_string()),
        )
        .expect("first endpoint should be created");
    let second_node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("second node should be added");
    let second_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            second_node.id(),
            Some("second".to_string()),
        )
        .expect("second endpoint should be created");

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            first_endpoint.id(),
            Some("go".to_string()),
        )
        .expect("first workflow run should be created");
    assert_eq!(workflow_run.status(), WorkflowRunStatus::Created);

    let queued = service
        .enqueue_workflow_prompt(
            session.id(),
            workflow.id(),
            second_endpoint.id(),
            Some("later".to_string()),
            Some("default"),
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("prompt should queue while a run is active");
    assert_eq!(queued.prompt(), Some("later"));
    assert!(service
        .dequeue_next_workflow_prompt(session.id())
        .expect("queue should be readable")
        .is_none());
}

#[test]
fn workflow_prompt_queue_dispatches_by_queue_priority_then_fifo() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let workflow = service
        .create_workflow(session.id(), Some("queued".to_string()))
        .expect("workflow should be created");
    let first_node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("first node should be added");
    let first_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            first_node.id(),
            Some("first".to_string()),
        )
        .expect("first endpoint should be created");
    let second_node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("second node should be added");
    let second_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            second_node.id(),
            Some("second".to_string()),
        )
        .expect("second endpoint should be created");
    let urgent_queue = service
        .create_workflow_prompt_queue(session.id(), workflow.id(), "urgent".to_string(), 10)
        .expect("urgent queue should be created");
    let active = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            first_endpoint.id(),
            Some("go".to_string()),
        )
        .expect("active workflow run should be created");
    assert_eq!(active.status(), WorkflowRunStatus::Created);

    let default_queued = service
        .enqueue_workflow_prompt(
            session.id(),
            workflow.id(),
            second_endpoint.id(),
            Some("second".to_string()),
            Some("default"),
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("default prompt should queue");
    let urgent_queued = service
        .enqueue_workflow_prompt(
            session.id(),
            workflow.id(),
            first_endpoint.id(),
            Some("third".to_string()),
            Some(urgent_queue.id()),
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("urgent prompt should queue");

    let queued = service
        .list_queued_workflow_prompts(session.id())
        .expect("queued prompts should list");
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].id(), default_queued.id());
    assert_eq!(queued[1].id(), urgent_queued.id());
    assert_eq!(queued[0].source(), WorkflowQueuedPromptSource::Manual);
    assert_eq!(queued[1].source(), WorkflowQueuedPromptSource::Manual);

    service
        .cancel_workflow_run(session.id(), active.id())
        .expect("active workflow run should stop");
    let dequeued = service
        .dequeue_next_workflow_prompt(session.id())
        .expect("queued workflow prompt should dequeue")
        .expect("expected queued workflow prompt");
    assert_eq!(dequeued.id(), urgent_queued.id());
}

#[test]
fn workflow_prompt_queues_are_scoped_per_workflow_and_arbitrate_across_workflows() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let first = workflow_with_endpoint(&mut service, session.id(), "first", "agent-1");
    let second = workflow_with_endpoint(&mut service, session.id(), "second", "agent-2");
    let first_fast = service
        .create_workflow_prompt_queue(session.id(), first.workflow.id(), "fast".to_string(), 5)
        .expect("first workflow queue should create");
    let second_fast = service
        .create_workflow_prompt_queue(session.id(), second.workflow.id(), "fast".to_string(), 5)
        .expect("second workflow queue should create with same alias");

    let first_prompt = service
        .enqueue_workflow_prompt(
            session.id(),
            first.workflow.id(),
            first.endpoint.id(),
            Some("first".to_string()),
            Some(first_fast.id()),
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("first prompt should queue");
    let second_prompt = service
        .enqueue_workflow_prompt(
            session.id(),
            second.workflow.id(),
            second.endpoint.id(),
            Some("second".to_string()),
            Some(second_fast.id()),
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("second prompt should queue");

    let first_queues = service
        .list_workflow_prompt_queues(session.id(), Some(first.workflow.id()))
        .expect("first queues should list");
    let second_queues = service
        .list_workflow_prompt_queues(session.id(), Some(second.workflow.id()))
        .expect("second queues should list");
    assert!(first_queues
        .iter()
        .any(|queue| queue.id() == first_fast.id()));
    assert!(!first_queues
        .iter()
        .any(|queue| queue.id() == second_fast.id()));
    assert!(second_queues
        .iter()
        .any(|queue| queue.id() == second_fast.id()));

    let dequeued = service
        .dequeue_next_workflow_prompt(session.id())
        .expect("queued workflow prompt should dequeue")
        .expect("expected queued workflow prompt");
    assert_eq!(dequeued.id(), first_prompt.id());

    let dequeued = service
        .dequeue_next_workflow_prompt(session.id())
        .expect("queued workflow prompt should dequeue")
        .expect("expected queued workflow prompt");
    assert_eq!(dequeued.id(), second_prompt.id());
}

#[test]
fn workflow_prompt_queue_arbitration_prefers_highest_priority_across_workflows() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let low = workflow_with_endpoint(&mut service, session.id(), "low", "agent-1");
    let high = workflow_with_endpoint(&mut service, session.id(), "high", "agent-2");
    let low_queue = service
        .create_workflow_prompt_queue(session.id(), low.workflow.id(), "queue".to_string(), 1)
        .expect("low queue should create");
    let high_queue = service
        .create_workflow_prompt_queue(session.id(), high.workflow.id(), "queue".to_string(), 10)
        .expect("high queue should create");

    let _low_prompt = service
        .enqueue_workflow_prompt(
            session.id(),
            low.workflow.id(),
            low.endpoint.id(),
            Some("queued first".to_string()),
            Some(low_queue.id()),
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("low prompt should queue");
    let high_prompt = service
        .enqueue_workflow_prompt(
            session.id(),
            high.workflow.id(),
            high.endpoint.id(),
            Some("queued second".to_string()),
            Some(high_queue.id()),
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("high prompt should queue");

    let dequeued = service
        .dequeue_next_workflow_prompt(session.id())
        .expect("queued workflow prompt should dequeue")
        .expect("expected queued workflow prompt");
    assert_eq!(dequeued.id(), high_prompt.id());
}

#[test]
fn workflow_prompt_queue_update_rejects_duplicate_alias_within_workflow() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = workflow_with_endpoint(&mut service, session.id(), "queues", "agent-1");
    let first = service
        .create_workflow_prompt_queue(
            session.id(),
            workflow.workflow.id(),
            "review".to_string(),
            1,
        )
        .expect("first queue should create");
    let second = service
        .create_workflow_prompt_queue(
            session.id(),
            workflow.workflow.id(),
            "urgent".to_string(),
            2,
        )
        .expect("second queue should create");

    let error = service
        .update_workflow_prompt_queue(
            session.id(),
            workflow.workflow.id(),
            second.id(),
            Some("review".to_string()),
            None,
            None,
        )
        .expect_err("renaming to an existing alias should fail");

    assert!(matches!(
        error,
        DaemonError::InvalidWorkflowGraphReference { .. }
    ));
    let unchanged = service
        .resolve_workflow_prompt_queue_ref(session.id(), workflow.workflow.id(), second.id())
        .expect("second queue should still resolve by id");
    assert_eq!(unchanged, second.id());
    let first_still_resolves = service
        .resolve_workflow_prompt_queue_ref(session.id(), workflow.workflow.id(), first.alias())
        .expect("first queue alias should still resolve");
    assert_eq!(first_still_resolves, first.id());
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

#[test]
fn publication_runtime_observability_applies_trace_exposure() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("published".to_string()))
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
            Some("prompt".to_string()),
        )
        .expect("workflow run should be created");
    let run_id = workflow_run.id().to_string();
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    {
        let session_mut = service
            .store
            .get_mut(session.id())
            .expect("session should exist");
        let run_mut = session_mut
            .workflow_run_mut(&run_id)
            .expect("run should exist");
        let node_run = run_mut
            .node_run_mut(&node_run_id)
            .expect("node run should exist");
        node_run.set_summary(Some("TRACE_SUMMARY".to_string()));
        node_run.set_completion(Some(WorkflowCompletionSnapshot::new(
            "TRACE_SUMMARY",
            Some(crate::session::WorkflowOutputPayload::new(
                "TRACE_ASSISTANT",
                Vec::new(),
            )),
        )));
        node_run.add_thinking_trace("TRACE_THINKING");
        let mut envelope =
            crate::session::WorkflowTurnEnvelope::new("token-1", "prompt".to_string(), None, None);
        envelope.add_runtime_tool_call(crate::session::WorkflowRuntimeToolCallEvent::new(
            "workflow_console_write",
            "{\"text\":\"TRACE_TOOL\"}",
            Some("{\"ok\":true}".to_string()),
            true,
        ));
        node_run.set_turn_envelope(Some(envelope));
        node_run.set_status(WorkflowNodeRunStatus::Completed);
        run_mut.set_final_output(
            Some(crate::session::WorkflowOutputPayload::new(
                "TRACE_FINAL",
                Vec::new(),
            )),
            Some(true),
            None,
            Some(node_run_id.clone()),
        );
        run_mut.set_status(WorkflowRunStatus::Completed);
    }

    let hidden_publication = service
        .create_workflow_publication(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("default".to_string()),
            Some("hidden".to_string()),
            Some(crate::session::WORKFLOW_PUBLICATION_KIND_INGRESS.to_string()),
            Some("/*".to_string()),
            vec!["GET".to_string()],
            None,
            None,
            None,
            None,
            Some("sync".to_string()),
            None,
            None,
            "local".to_string(),
        )
        .expect("publication should be created");
    let hidden_publication = service
        .mark_workflow_publication_runtime_status(
            session.id(),
            hidden_publication.id(),
            "running",
            None,
            None,
        )
        .expect("runtime status should update");
    let hidden_text = serde_json::to_string(&hidden_publication)
        .expect("publication should serialize");
    assert!(hidden_text.contains("TRACE_FINAL"));
    assert!(!hidden_text.contains("TRACE_SUMMARY"));
    assert!(!hidden_text.contains("TRACE_ASSISTANT"));
    assert!(!hidden_text.contains("TRACE_THINKING"));
    assert!(!hidden_text.contains("TRACE_TOOL"));

    let exposed_publication = service
        .create_workflow_publication(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("default".to_string()),
            Some("exposed".to_string()),
            Some(crate::session::WORKFLOW_PUBLICATION_KIND_INGRESS.to_string()),
            Some("/*".to_string()),
            vec!["GET".to_string()],
            None,
            None,
            None,
            Some(serde_json::json!({
                "nodes": {
                    node.id(): ["output_summary", "assistant_messages", "thinking", "tool_use"]
                }
            })),
            Some("sync".to_string()),
            None,
            None,
            "local".to_string(),
        )
        .expect("publication should be created");
    let exposed_publication = service
        .mark_workflow_publication_runtime_status(
            session.id(),
            exposed_publication.id(),
            "running",
            None,
            None,
        )
        .expect("runtime status should update");
    let exposed_text = serde_json::to_string(&exposed_publication)
        .expect("publication should serialize");
    assert!(exposed_text.contains("TRACE_SUMMARY"));
    assert!(exposed_text.contains("TRACE_ASSISTANT"));
    assert!(exposed_text.contains("TRACE_THINKING"));
    assert!(exposed_text.contains("TRACE_TOOL"));
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
