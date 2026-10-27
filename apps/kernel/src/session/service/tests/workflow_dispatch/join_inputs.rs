use super::*;

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
        .set_workflow_node_wait_for_all_inputs(session.id(), workflow.id(), join.id(), true)
        .expect("join node should wait for synchronized inputs");
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

#[test]
fn multi_input_nodes_dispatch_per_message_by_default() {
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
        .create_workflow(session.id(), Some("default-join".to_string()))
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
    for (from, to) in [
        (entry.id(), branch_one.id()),
        (entry.id(), branch_two.id()),
        (branch_one.id(), join.id()),
        (branch_two.id(), join.id()),
    ] {
        service
            .add_workflow_edge(session.id(), workflow.id(), from, to, None, None)
            .expect("workflow edge should be added");
    }
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
            Some("run default join behavior".to_string()),
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
    let branch_one_run = entry_completion
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == branch_one.id())
        .expect("branch one dispatch should exist")
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
        .expect("branch one completion should dispatch join immediately");

    assert_eq!(branch_one_completion.dispatches.len(), 1);
    let join_dispatch = &branch_one_completion.dispatches[0];
    assert_eq!(join_dispatch.node_run.node_id(), join.id());
    assert_eq!(join_dispatch.node_run.iteration_index(), 1);
    assert_eq!(join_dispatch.messages.len(), 1);
    assert_eq!(
        join_dispatch.messages[0].source_node_run_id(),
        Some(branch_one_run.id())
    );
}

#[test]
fn wait_for_all_inputs_groups_by_source_iteration() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(
        &mut service,
        session.id(),
        &["agent-1", "agent-2", "agent-3", "agent-4", "agent-5"],
    );
    let workflow = service
        .create_workflow(session.id(), Some("loop-join".to_string()))
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
    let repeater = service
        .add_workflow_node(session.id(), workflow.id(), "agent-5")
        .expect("repeater node should be added");
    service
        .set_workflow_node_wait_for_all_inputs(session.id(), workflow.id(), join.id(), true)
        .expect("join node should wait for synchronized inputs");
    for (from, to) in [
        (entry.id(), branch_one.id()),
        (entry.id(), branch_two.id()),
        (branch_one.id(), join.id()),
        (branch_two.id(), join.id()),
        (branch_one.id(), repeater.id()),
        (repeater.id(), branch_one.id()),
    ] {
        service
            .add_workflow_edge(session.id(), workflow.id(), from, to, None, None)
            .expect("workflow edge should be added");
    }
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
            Some("run loop join behavior".to_string()),
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
    let branch_one_run_1 = entry_completion
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == branch_one.id())
        .expect("branch one dispatch should exist")
        .node_run
        .clone();
    let branch_two_run_1 = entry_completion
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == branch_two.id())
        .expect("branch two dispatch should exist")
        .node_run
        .clone();

    service
        .start_workflow_node_run(session.id(), workflow_run.id(), branch_one_run_1.id())
        .expect("branch one iteration 1 should start");
    let branch_one_completion_1 = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            branch_one_run_1.id(),
            None,
            None,
        )
        .expect("branch one iteration 1 completion should succeed");
    assert!(branch_one_completion_1
        .dispatches
        .iter()
        .all(|dispatch| dispatch.node_run.node_id() != join.id()));
    let repeater_run_1 = branch_one_completion_1
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == repeater.id())
        .expect("repeater dispatch should exist")
        .node_run
        .clone();

    service
        .start_workflow_node_run(session.id(), workflow_run.id(), repeater_run_1.id())
        .expect("repeater should start");
    let repeater_completion_1 = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            repeater_run_1.id(),
            None,
            None,
        )
        .expect("repeater completion should dispatch branch one iteration 2");
    let branch_one_run_2 = repeater_completion_1
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == branch_one.id())
        .expect("branch one iteration 2 dispatch should exist")
        .node_run
        .clone();
    assert_eq!(branch_one_run_2.iteration_index(), 2);

    service
        .start_workflow_node_run(session.id(), workflow_run.id(), branch_one_run_2.id())
        .expect("branch one iteration 2 should start");
    let branch_one_completion_2 = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            branch_one_run_2.id(),
            None,
            None,
        )
        .expect("branch one iteration 2 completion should succeed");
    assert!(branch_one_completion_2
        .dispatches
        .iter()
        .all(|dispatch| dispatch.node_run.node_id() != join.id()));

    service
        .start_workflow_node_run(session.id(), workflow_run.id(), branch_two_run_1.id())
        .expect("branch two iteration 1 should start");
    let branch_two_completion_1 = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            branch_two_run_1.id(),
            None,
            None,
        )
        .expect("branch two iteration 1 completion should dispatch synchronized join");
    let join_dispatch = branch_two_completion_1
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == join.id())
        .expect("join dispatch should exist");
    assert_eq!(join_dispatch.messages.len(), 2);
    let source_run_ids = join_dispatch
        .messages
        .iter()
        .filter_map(|message| message.source_node_run_id())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(source_run_ids.contains(branch_one_run_1.id()));
    assert!(source_run_ids.contains(branch_two_run_1.id()));
    assert!(!source_run_ids.contains(branch_one_run_2.id()));

    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    let branch_one_iteration_2_messages = resolved
        .messages()
        .iter()
        .filter(|message| {
            message.target_node_id() == join.id()
                && message.source_node_run_id() == Some(branch_one_run_2.id())
        })
        .collect::<Vec<_>>();
    assert_eq!(branch_one_iteration_2_messages.len(), 1);
    assert!(branch_one_iteration_2_messages[0]
        .consumed_by_node_run_id()
        .is_none());
}
