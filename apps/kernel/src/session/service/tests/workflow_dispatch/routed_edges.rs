use super::*;

#[test]
fn multi_edge_completion_routes_only_matching_edge_id() {
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
        .create_workflow(session.id(), Some("edge-route".to_string()))
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
    let edge_a = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            worker_a.id(),
            None,
            None,
        )
        .expect("router should connect to worker a");
    let _edge_b = service
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
    let routed = serde_json::json!({
        "workflow_handoffs": [{
            "edge_id": edge_a.id(),
            "summary": "send to worker a",
            "output": { "message": { "task": "only a" } }
        }]
    });

    let completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
            Some(completion_with_message(routed.to_string())),
            None,
        )
        .expect("router completion should route to selected edge");

    assert_eq!(completion.dispatches.len(), 1);
    assert_eq!(completion.dispatches[0].node_run.node_id(), worker_a.id());
    let payload: WorkflowHandoffPayload =
        serde_json::from_str(completion.dispatches[0].messages[0].handoff_payload())
            .expect("handoff payload should deserialize");
    let output = payload
        .completion()
        .and_then(|snapshot| snapshot.output())
        .expect("payload should include routed output");
    assert_eq!(output.message(), r#"{"task":"only a"}"#);
}

#[test]
fn multi_edge_completion_routes_by_target_node_and_suppresses_null_message() {
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
        .create_workflow(session.id(), Some("target-route".to_string()))
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
    let edge_b = service
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
    let routed = serde_json::json!({
        "workflow_handoffs": [
            {
                "to_node_id": worker_a.id(),
                "summary": "send to worker a",
                "message": "target node route"
            },
            {
                "edge_id": edge_b.id(),
                "message": null
            }
        ]
    });

    let completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
            Some(completion_with_message(routed.to_string())),
            None,
        )
        .expect("router completion should route by target node and suppress null edge");

    assert_eq!(completion.dispatches.len(), 1);
    assert_eq!(completion.dispatches[0].node_run.node_id(), worker_a.id());
    let payload: WorkflowHandoffPayload =
        serde_json::from_str(completion.dispatches[0].messages[0].handoff_payload())
            .expect("handoff payload should deserialize");
    let output = payload
        .completion()
        .and_then(|snapshot| snapshot.output())
        .expect("payload should include routed output");
    assert_eq!(output.message(), "target node route");
}

#[test]
fn routed_loop_edge_dispatches_back_to_entry_node() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(
        &mut service,
        session.id(),
        &["intake", "worker", "synthesis", "done"],
    );
    let workflow = service
        .create_workflow(session.id(), Some("loop".to_string()))
        .expect("workflow should be created");
    let intake = service
        .add_workflow_node(session.id(), workflow.id(), "intake")
        .expect("intake node should be added");
    let worker = service
        .add_workflow_node(session.id(), workflow.id(), "worker")
        .expect("worker node should be added");
    let synthesis = service
        .add_workflow_node(session.id(), workflow.id(), "synthesis")
        .expect("synthesis node should be added");
    let done = service
        .add_workflow_node(session.id(), workflow.id(), "done")
        .expect("done node should be added");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            intake.id(),
            worker.id(),
            None,
            None,
        )
        .expect("intake should connect to worker");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            worker.id(),
            synthesis.id(),
            None,
            None,
        )
        .expect("worker should connect to synthesis");
    let loop_edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            synthesis.id(),
            intake.id(),
            None,
            None,
        )
        .expect("synthesis should connect back to intake");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            synthesis.id(),
            done.id(),
            None,
            None,
        )
        .expect("synthesis should connect to done");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            intake.id(),
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
        .expect("intake should start");
    let intake_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
            None,
            None,
        )
        .expect("intake should dispatch worker");
    let worker_run = intake_completion.dispatches[0].node_run.clone();
    service
        .start_workflow_node_run(session.id(), workflow_run.id(), worker_run.id())
        .expect("worker should start");
    let worker_completion = service
        .complete_workflow_node_run(session.id(), workflow_run.id(), worker_run.id(), None, None)
        .expect("worker should dispatch synthesis");
    let synthesis_run = worker_completion.dispatches[0].node_run.clone();
    service
        .start_workflow_node_run(session.id(), workflow_run.id(), synthesis_run.id())
        .expect("synthesis should start");
    let routed = serde_json::json!({
        "workflow_handoffs": [{
            "edge_id": loop_edge.id(),
            "summary": "needs revision",
            "message": "revise intake"
        }]
    });

    let synthesis_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            synthesis_run.id(),
            Some(completion_with_message(routed.to_string())),
            None,
        )
        .expect("synthesis should dispatch loop edge only");

    assert_eq!(synthesis_completion.dispatches.len(), 1);
    assert_eq!(
        synthesis_completion.dispatches[0].node_run.node_id(),
        intake.id()
    );
    assert_ne!(
        synthesis_completion.dispatches[0].node_run.node_id(),
        done.id()
    );
    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(
        resolved
            .node_runs()
            .iter()
            .filter(|node_run| node_run.node_id() == intake.id())
            .count(),
        2
    );
}
