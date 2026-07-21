use super::*;

#[test]
fn metaagent_and_workflow_tasks_share_one_serial_task_lane() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = workflow_with_endpoint(&mut service, session.id(), "after-meta", "agent-1");

    let meta = service
        .enqueue_metaagent_task(
            session.id(),
            "agent-1",
            "attachment-1",
            "inspect the task lane",
            Vec::new(),
        )
        .expect("Meta task should queue");
    let (_, claimed) = service
        .enqueue_workflow_prompt_and_maybe_create_run(
            session.id(),
            workflow.workflow.id(),
            workflow.endpoint.id(),
            Some("run after Meta".to_string()),
            Some("default"),
            WorkflowQueuedPromptSource::Manual,
            None,
            None,
        )
        .expect("workflow task should queue");
    assert!(
        claimed.is_none(),
        "workflow must not overtake the older Meta task"
    );

    let popped = service
        .pop_next_queued_metaagent_task(session.id())
        .expect("Meta queue should be readable")
        .expect("Meta task should be selected first");
    assert_eq!(popped.id(), meta.id());
    service
        .start_or_update_metaagent_task(session.id(), "agent-1", popped.task_markdown())
        .expect("Meta task should become active");
    assert!(service
        .dequeue_next_workflow_prompt(session.id())
        .expect("workflow queue should remain readable")
        .is_none());

    service
        .complete_metaagent_task(session.id(), "agent-1", Some("done".to_string()))
        .expect("Meta task should complete");
    assert!(service
        .dequeue_next_workflow_prompt(session.id())
        .expect("workflow should dequeue after Meta completion")
        .is_some());
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
