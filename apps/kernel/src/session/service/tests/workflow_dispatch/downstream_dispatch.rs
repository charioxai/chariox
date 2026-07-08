use super::*;

#[test]
fn completing_a_workflow_node_run_creates_structured_downstream_dispatches() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let first = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("first workflow node should be added");
    let second = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("second workflow node should be added");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            first.id(),
            second.id(),
            None,
            None,
        )
        .expect("workflow edge should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            first.id(),
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
    let started = service
        .start_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
        )
        .expect("entry node should start");
    assert_eq!(started.status(), WorkflowRunStatus::Running);
    assert_eq!(
        started.active_node_run_id(),
        Some(workflow_run.node_runs()[0].id())
    );

    let completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
            None,
            None,
        )
        .expect("entry node completion should route downstream work");
    assert_eq!(completion.workflow_run.status(), WorkflowRunStatus::Waiting);
    assert_eq!(completion.dispatches.len(), 1);
    assert_eq!(completion.dispatches[0].node_run.node_id(), second.id());
    assert_eq!(completion.dispatches[0].messages.len(), 1);
    assert_eq!(
        completion.dispatches[0].messages[0].target_node_id(),
        second.id()
    );
    let payload: WorkflowHandoffPayload =
        serde_json::from_str(completion.dispatches[0].messages[0].handoff_payload())
            .expect("handoff payload should deserialize");
    assert_eq!(payload.workflow_run_id(), workflow_run.id());
    assert_eq!(payload.workflow_id(), workflow.id());
    assert_eq!(
        payload.source_node_run_id(),
        workflow_run.node_runs()[0].id()
    );
    assert_eq!(payload.source_node_id(), first.id());
    assert_eq!(payload.source_agent_id(), "agent-1");
    assert_eq!(payload.target_node_id(), second.id());
    assert_eq!(payload.invocation_prompt(), Some("review this diff"));
    assert!(payload.completion().is_none());

    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(resolved.status(), WorkflowRunStatus::Waiting);
    assert_eq!(resolved.node_runs().len(), 2);
    assert_eq!(resolved.messages().len(), 2);
}

#[test]
fn multi_edge_completion_fans_out_without_routed_handoffs() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(
        &mut service,
        session.id(),
        &["router", "worker-a", "worker-b"],
    );
    let workflow = service
        .create_workflow(session.id(), Some("fanout".to_string()))
        .expect("workflow should be created");
    let router = service
        .add_workflow_node(session.id(), workflow.id(), "router")
        .expect("router node should be added");
    let worker_a = service
        .add_workflow_node(session.id(), workflow.id(), "worker-a")
        .expect("worker a node should be added");
    let worker_b = service
        .add_workflow_node(session.id(), workflow.id(), "worker-b")
        .expect("worker b node should be added");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            worker_a.id(),
            None,
            None,
        )
        .expect("router should connect to worker a");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            worker_b.id(),
            None,
            None,
        )
        .expect("router should connect to worker b");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            router.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("go".to_string()),
        )
        .expect("workflow run should be created");
    service
        .start_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
        )
        .expect("router should start");

    let completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
            Some(completion_with_message("plain downstream task")),
            None,
        )
        .expect("router completion should fan out");

    assert_eq!(completion.dispatches.len(), 2);
    let dispatched_targets = completion
        .dispatches
        .iter()
        .map(|dispatch| dispatch.node_run.node_id())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(dispatched_targets.contains(worker_a.id()));
    assert!(dispatched_targets.contains(worker_b.id()));
    for dispatch in &completion.dispatches {
        let payload: WorkflowHandoffPayload =
            serde_json::from_str(dispatch.messages[0].handoff_payload())
                .expect("handoff payload should deserialize");
        let output = payload
            .completion()
            .and_then(|snapshot| snapshot.output())
            .expect("payload should include completion output");
        assert_eq!(output.message(), "plain downstream task");
    }
}

#[test]
fn workflow_max_concurrent_limits_ready_downstream_dispatches() {
    let mut config = test_config();
    config.user_config.workflow.code = Some(crate::config::UserWorkflowCodeConfig {
        max_concurrent: Some(1),
        ..Default::default()
    });
    let mut service = SessionService::new(&config);
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(
        &mut service,
        session.id(),
        &["router", "worker-a", "worker-b"],
    );
    let workflow = service
        .create_workflow(session.id(), Some("fanout-throttled".to_string()))
        .expect("workflow should be created");
    assert_eq!(workflow.max_concurrent(), 1);
    let router = service
        .add_workflow_node(session.id(), workflow.id(), "router")
        .expect("router node should be added");
    let worker_a = service
        .add_workflow_node(session.id(), workflow.id(), "worker-a")
        .expect("worker a node should be added");
    let worker_b = service
        .add_workflow_node(session.id(), workflow.id(), "worker-b")
        .expect("worker b node should be added");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            worker_a.id(),
            None,
            None,
        )
        .expect("router should connect to worker a");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            worker_b.id(),
            None,
            None,
        )
        .expect("router should connect to worker b");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            router.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("go".to_string()),
        )
        .expect("workflow run should be created");
    service
        .start_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
        )
        .expect("router should start");

    let first_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
            Some(completion_with_message("plain downstream task")),
            None,
        )
        .expect("router completion should fan out within the cap");

    assert_eq!(first_completion.dispatches.len(), 1);
    let first_worker_run = first_completion.dispatches[0].node_run.clone();
    let waiting = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(waiting.node_runs().len(), 2);
    assert_eq!(
        waiting
            .messages()
            .iter()
            .filter(|message| message.consumed_by_node_run_id().is_none())
            .count(),
        1
    );

    service
        .start_workflow_node_run(session.id(), workflow_run.id(), first_worker_run.id())
        .expect("first worker should start");
    let second_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            first_worker_run.id(),
            None,
            None,
        )
        .expect("first worker completion should release the next dispatch slot");

    assert_eq!(second_completion.dispatches.len(), 1);
    assert_ne!(
        second_completion.dispatches[0].node_run.node_id(),
        first_worker_run.node_id()
    );
    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(resolved.node_runs().len(), 3);
    assert!(
        resolved
            .messages()
            .iter()
            .filter(|message| message.target_node_id() == worker_a.id()
                || message.target_node_id() == worker_b.id())
            .all(|message| message.consumed_by_node_run_id().is_some())
    );
}
