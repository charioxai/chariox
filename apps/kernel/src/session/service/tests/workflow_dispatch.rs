use super::*;

fn completion_with_message(message: impl Into<String>) -> WorkflowCompletionSnapshot {
    WorkflowCompletionSnapshot::new(
        "done",
        Some(crate::session::WorkflowOutputPayload::new(
            message.into(),
            Vec::new(),
        )),
    )
}

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

#[test]
fn selected_edge_schema_validation_ignores_unselected_edge_schema() {
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
        .create_workflow(session.id(), Some("selected-schema".to_string()))
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
    let schema_dir = std::env::temp_dir();
    let schema_a = schema_dir.join(format!(
        "arroba-selected-edge-a-{}-{}.json",
        std::process::id(),
        unix_epoch_ms()
    ));
    let schema_b = schema_dir.join(format!(
        "arroba-selected-edge-b-{}-{}.json",
        std::process::id(),
        unix_epoch_ms()
    ));
    std::fs::write(
        &schema_a,
        r#"{"type":"object","required":["kind"],"properties":{"kind":{"const":"a"}},"additionalProperties":false}"#,
    )
    .expect("schema a should write");
    std::fs::write(
        &schema_b,
        r#"{"type":"object","required":["kind"],"properties":{"kind":{"const":"b"}},"additionalProperties":false}"#,
    )
    .expect("schema b should write");
    let edge_a = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            worker_a.id(),
            Some(schema_a.to_string_lossy().to_string()),
            Some(crate::session::WorkflowHandoffValidationPolicy::Halt),
        )
        .expect("router should connect to worker a");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            worker_b.id(),
            Some(schema_b.to_string_lossy().to_string()),
            Some(crate::session::WorkflowHandoffValidationPolicy::Halt),
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
            "summary": "valid only for edge a",
            "output": { "message": { "kind": "a" } }
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
        .expect("selected edge schema should validate without checking unselected edge");

    assert_eq!(completion.validation_warnings.len(), 0);
    assert_eq!(completion.dispatches.len(), 1);
    assert_eq!(completion.dispatches[0].node_run.node_id(), worker_a.id());
    std::fs::remove_file(schema_a).ok();
    std::fs::remove_file(schema_b).ok();
}

#[test]
fn selected_edge_schema_validation_halts_or_warns_by_policy() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(
        &mut service,
        session.id(),
        &["halt-router", "halt-worker", "warn-router", "warn-worker"],
    );
    let schema = std::env::temp_dir().join(format!(
        "arroba-selected-edge-invalid-{}-{}.json",
        std::process::id(),
        unix_epoch_ms()
    ));
    std::fs::write(
        &schema,
        r#"{"type":"object","required":["kind"],"properties":{"kind":{"const":"expected"}},"additionalProperties":false}"#,
    )
    .expect("schema should write");

    let halt_workflow = service
        .create_workflow(session.id(), Some("halt-schema".to_string()))
        .expect("halt workflow should be created");
    let halt_router = service
        .add_workflow_node(session.id(), halt_workflow.id(), "halt-router")
        .expect("halt router should be added");
    let halt_worker = service
        .add_workflow_node(session.id(), halt_workflow.id(), "halt-worker")
        .expect("halt worker should be added");
    let halt_edge = service
        .add_workflow_edge(
            session.id(),
            halt_workflow.id(),
            halt_router.id(),
            halt_worker.id(),
            Some(schema.to_string_lossy().to_string()),
            Some(crate::session::WorkflowHandoffValidationPolicy::Halt),
        )
        .expect("halt edge should be added");
    let halt_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            halt_workflow.id(),
            halt_router.id(),
            Some("entry".to_string()),
        )
        .expect("halt endpoint should be created");
    let halt_run = service
        .invoke_workflow_endpoint(
            session.id(),
            halt_workflow.id(),
            halt_endpoint.id(),
            Some("go".to_string()),
        )
        .expect("halt workflow run should be created");
    service
        .start_workflow_node_run(session.id(), halt_run.id(), halt_run.node_runs()[0].id())
        .expect("halt router should start");
    let invalid = serde_json::json!({
        "workflow_handoffs": [{
            "edge_id": halt_edge.id(),
            "output": { "message": { "kind": "wrong" } }
        }]
    });
    let error = service
        .complete_workflow_node_run(
            session.id(),
            halt_run.id(),
            halt_run.node_runs()[0].id(),
            Some(completion_with_message(invalid.to_string())),
            None,
        )
        .expect_err("halt policy should reject invalid selected payload");
    assert!(matches!(
        error,
        DaemonError::WorkflowHandoffValidationFailed { .. }
    ));

    let warn_workflow = service
        .create_workflow(session.id(), Some("warn-schema".to_string()))
        .expect("warn workflow should be created");
    let warn_router = service
        .add_workflow_node(session.id(), warn_workflow.id(), "warn-router")
        .expect("warn router should be added");
    let warn_worker = service
        .add_workflow_node(session.id(), warn_workflow.id(), "warn-worker")
        .expect("warn worker should be added");
    let warn_edge = service
        .add_workflow_edge(
            session.id(),
            warn_workflow.id(),
            warn_router.id(),
            warn_worker.id(),
            Some(schema.to_string_lossy().to_string()),
            Some(crate::session::WorkflowHandoffValidationPolicy::Warn),
        )
        .expect("warn edge should be added");
    let warn_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            warn_workflow.id(),
            warn_router.id(),
            Some("entry".to_string()),
        )
        .expect("warn endpoint should be created");
    let warn_run = service
        .invoke_workflow_endpoint(
            session.id(),
            warn_workflow.id(),
            warn_endpoint.id(),
            Some("go".to_string()),
        )
        .expect("warn workflow run should be created");
    service
        .start_workflow_node_run(session.id(), warn_run.id(), warn_run.node_runs()[0].id())
        .expect("warn router should start");
    let invalid = serde_json::json!({
        "workflow_handoffs": [{
            "edge_id": warn_edge.id(),
            "output": { "message": { "kind": "wrong" } }
        }]
    });
    let completion = service
        .complete_workflow_node_run(
            session.id(),
            warn_run.id(),
            warn_run.node_runs()[0].id(),
            Some(completion_with_message(invalid.to_string())),
            None,
        )
        .expect("warn policy should record warning and continue");

    assert_eq!(completion.validation_warnings.len(), 1);
    assert_eq!(completion.validation_warnings[0].edge_id, warn_edge.id());
    assert_eq!(completion.dispatches.len(), 1);
    assert_eq!(
        completion.dispatches[0].node_run.node_id(),
        warn_worker.id()
    );
    std::fs::remove_file(schema).ok();
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
