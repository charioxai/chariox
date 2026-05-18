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
fn join_nodes_wait_for_all_inputs_before_dispatching_once() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(
        &mut service,
        session.id(),
        &["agent-1", "agent-2", "agent-3", "agent-4"],
    );
    let workflow = service
        .create_workflow(session.id(), Some("join".to_string()))
        .expect("workflow should be created");
    let entry = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("entry node should be added");
    let branch_one = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("branch one node should be added");
    let branch_two = service
        .add_workflow_node(session.id(), workflow.id(), "agent-3")
        .expect("branch two node should be added");
    let join = service
        .add_workflow_node(session.id(), workflow.id(), "agent-4")
        .expect("join node should be added");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            entry.id(),
            branch_one.id(),
            None,
            None,
        )
        .expect("entry should connect to branch one");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            entry.id(),
            branch_two.id(),
            None,
            None,
        )
        .expect("entry should connect to branch two");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            branch_one.id(),
            join.id(),
            None,
            None,
        )
        .expect("branch one should connect to join");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            branch_two.id(),
            join.id(),
            None,
            None,
        )
        .expect("branch two should connect to join");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            entry.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run the join drill".to_string()),
        )
        .expect("workflow run should be created");
    let started = service
        .start_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
        )
        .expect("entry node should start");
    let entry_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            started.node_runs()[0].id(),
            None,
            None,
        )
        .expect("entry node should dispatch both branches");
    assert_eq!(entry_completion.dispatches.len(), 2);

    let branch_one_run = entry_completion
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == branch_one.id())
        .expect("branch one dispatch should exist")
        .node_run
        .clone();
    let branch_two_run = entry_completion
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == branch_two.id())
        .expect("branch two dispatch should exist")
        .node_run
        .clone();
    service
        .start_workflow_node_run(session.id(), workflow_run.id(), branch_one_run.id())
        .expect("branch one should start");
    let branch_one_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            branch_one_run.id(),
            None,
            None,
        )
        .expect("branch one completion should succeed");
    assert!(branch_one_completion.dispatches.is_empty());
    let waiting = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve after first branch");
    assert_eq!(waiting.node_runs().len(), 3);
    assert_eq!(
        waiting
            .messages()
            .iter()
            .filter(|message| message.target_node_id() == join.id())
            .count(),
        1
    );
    assert!(waiting
        .messages()
        .iter()
        .filter(|message| message.target_node_id() == join.id())
        .all(|message| message.consumed_by_node_run_id().is_none()));

    service
        .start_workflow_node_run(session.id(), workflow_run.id(), branch_two_run.id())
        .expect("branch two should start");
    let branch_two_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            branch_two_run.id(),
            None,
            None,
        )
        .expect("branch two completion should succeed");
    assert_eq!(branch_two_completion.dispatches.len(), 1);
    let join_dispatch = &branch_two_completion.dispatches[0];
    assert_eq!(join_dispatch.node_run.node_id(), join.id());
    assert_eq!(join_dispatch.messages.len(), 2);
    assert_eq!(
        join_dispatch
            .messages
            .iter()
            .map(|message| message.target_node_id())
            .collect::<Vec<_>>(),
        vec![join.id(), join.id()]
    );
    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(resolved.node_runs().len(), 4);
    assert_eq!(
        resolved
            .messages()
            .iter()
            .filter(|message| message.target_node_id() == join.id())
            .count(),
        2
    );
    assert!(resolved
        .messages()
        .iter()
        .filter(|message| message.target_node_id() == join.id())
        .all(|message| message.consumed_by_node_run_id() == Some(join_dispatch.node_run.id())));
}
