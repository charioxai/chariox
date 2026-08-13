use super::*;

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
        "chariox-selected-edge-a-{}-{}.json",
        std::process::id(),
        unix_epoch_ms()
    ));
    let schema_b = schema_dir.join(format!(
        "chariox-selected-edge-b-{}-{}.json",
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
fn routed_edge_accepts_inline_schema_payload_fields() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["router", "worker"]);
    let workflow = service
        .create_workflow(session.id(), Some("inline-routed-schema".to_string()))
        .expect("workflow should be created");
    let router = service
        .add_workflow_node(session.id(), workflow.id(), "router")
        .expect("router node should be added");
    let worker = service
        .add_workflow_node(session.id(), workflow.id(), "worker")
        .expect("worker node should be added");
    let schema = std::env::temp_dir().join(format!(
        "chariox-inline-routed-schema-{}-{}.json",
        std::process::id(),
        unix_epoch_ms()
    ));
    std::fs::write(
        &schema,
        r#"{"type":"object","required":["question","angle"],"properties":{"question":{"type":"string"},"angle":{"type":"string"}},"additionalProperties":false}"#,
    )
    .expect("schema should write");
    let edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            worker.id(),
            Some(schema.to_string_lossy().to_string()),
            Some(crate::session::WorkflowHandoffValidationPolicy::Halt),
        )
        .expect("router should connect to worker");
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
            "edge_id": edge.id(),
            "angle": "release-validation path",
            "question": "Inspect the release path and return concise findings."
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
        .expect("inline routed payload should route and validate");

    assert_eq!(completion.validation_warnings.len(), 0);
    assert_eq!(completion.dispatches.len(), 1);
    assert_eq!(completion.dispatches[0].node_run.node_id(), worker.id());
    let payload: WorkflowHandoffPayload =
        serde_json::from_str(completion.dispatches[0].messages[0].handoff_payload())
            .expect("handoff payload should deserialize");
    let output = payload
        .completion()
        .and_then(|snapshot| snapshot.output())
        .expect("payload should include routed output");
    let routed_output: serde_json::Value =
        serde_json::from_str(output.message()).expect("inline payload should stay JSON");
    assert_eq!(routed_output["angle"], "release-validation path");
    assert_eq!(
        routed_output["question"],
        "Inspect the release path and return concise findings."
    );
    std::fs::remove_file(schema).ok();
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
        "chariox-selected-edge-invalid-{}-{}.json",
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
    let correction = service
        .complete_workflow_node_run(
            session.id(),
            halt_run.id(),
            halt_run.node_runs()[0].id(),
            Some(completion_with_message(invalid.to_string())),
            None,
        )
        .expect("halt policy should schedule bounded correction for invalid selected payload");
    let failure = correction
        .handoff_validation_failure
        .as_ref()
        .expect("handoff validation failure should be reported");
    assert_eq!(failure.edge_id, halt_edge.id());
    assert_eq!(failure.attempt, 1);
    assert_eq!(failure.max_attempts, 3);
    assert!(failure.retry_scheduled);
    assert_eq!(correction.workflow_run.status(), WorkflowRunStatus::Waiting);
    assert_eq!(correction.dispatches.len(), 1);
    assert_eq!(
        correction.dispatches[0].node_run.node_id(),
        halt_router.id()
    );
    assert!(correction.dispatches[0].messages.is_empty());

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
fn repeated_invalid_handoffs_fail_after_bounded_classifier_corrections() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["classifier", "specialist"]);
    let workflow = service
        .create_workflow(session.id(), Some("bounded-handoff-correction".to_string()))
        .expect("workflow should be created");
    let classifier = service
        .add_workflow_node(session.id(), workflow.id(), "classifier")
        .expect("classifier should be added");
    service
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), classifier.id(), true)
        .expect("classifier should be allowed to complete the run");
    let specialist = service
        .add_workflow_node(session.id(), workflow.id(), "specialist")
        .expect("specialist should be added");
    let schema = std::env::temp_dir().join(format!(
        "chariox-bounded-handoff-correction-{}-{}.json",
        std::process::id(),
        unix_epoch_ms()
    ));
    std::fs::write(
        &schema,
        r#"{"type":"object","required":["task"],"properties":{"task":{"type":"string"}},"additionalProperties":false}"#,
    )
    .expect("schema should write");
    let edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            classifier.id(),
            specialist.id(),
            Some(schema.to_string_lossy().to_string()),
            Some(crate::session::WorkflowHandoffValidationPolicy::Halt),
        )
        .expect("classifier should connect to specialist");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            classifier.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("classify bounded task".to_string()),
        )
        .expect("workflow should invoke");
    let mut node_run_id = run.node_runs()[0].id().to_string();
    service
        .start_workflow_node_run(session.id(), run.id(), &node_run_id)
        .expect("classifier should start");

    for expected_attempt in 1..=3 {
        let invalid = serde_json::json!({
            "workflow_handoffs": [{
                "edge_id": edge.id(),
                "output": { "message": { "wrong": true } }
            }]
        });
        let update = service
            .complete_workflow_node_run(
                session.id(),
                run.id(),
                &node_run_id,
                Some(completion_with_message(invalid.to_string())),
                None,
            )
            .expect("invalid handoff should produce a bounded correction update");
        let failure = update
            .handoff_validation_failure
            .as_ref()
            .expect("handoff validation failure should be reported");
        assert_eq!(failure.attempt, expected_attempt);
        assert_eq!(failure.max_attempts, 3);
        assert_eq!(failure.retry_scheduled, expected_attempt < 3);
        assert_eq!(
            update
                .workflow_run
                .messages()
                .iter()
                .filter(|message| message.message_type() == "handoff")
                .count(),
            0
        );
        assert_eq!(
            update
                .workflow_run
                .node_runs()
                .iter()
                .filter(|node_run| node_run.node_id() == specialist.id())
                .count(),
            0
        );
        service
            .record_workflow_failure_event(
                session.id(),
                run.id(),
                crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::OutputValidationFailed,
                    node_run_id.clone(),
                    vec![edge.id().to_string()],
                    failure.message.clone(),
                ),
            )
            .expect("handoff validation failure should be recorded");

        if expected_attempt < 3 {
            assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Waiting);
            assert_eq!(update.dispatches.len(), 1);
            assert_eq!(update.dispatches[0].node_run.node_id(), classifier.id());
            assert!(update.dispatches[0].messages.is_empty());
            let correction_prompt = update.dispatches[0]
                .endpoint_prompt
                .as_deref()
                .expect("correction should include a prompt");
            assert_eq!(
                correction_prompt.matches("classify bounded task").count(),
                1
            );
            assert!(correction_prompt.contains(
                "If the work is accepted and the workflow should finish, do not emit an outgoing handoff."
            ));
            assert!(correction_prompt.contains("validate_and_submit_workflow_run_output"));
            node_run_id = update.dispatches[0].node_run.id().to_string();
            service
                .start_workflow_node_run(session.id(), run.id(), &node_run_id)
                .expect("classifier correction should start");
        } else {
            assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Failed);
            assert!(update.dispatches.is_empty());
        }
    }

    std::fs::remove_file(schema).ok();
}
