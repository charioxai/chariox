use super::*;
use crate::session::{WorkflowCanvasLayoutPatch, WorkflowHandoffValidationPolicy};

#[test]
fn creates_lists_and_resolves_workflows_by_id_and_alias_prefix() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let first = service
        .create_workflow(session.id(), Some("review_loop".to_string()))
        .expect("workflow should be created");
    let second = service
        .create_workflow(session.id(), Some("deploy".to_string()))
        .expect("second workflow should be created");

    let workflows = service
        .list_workflows(session.id())
        .expect("workflow list should succeed");
    assert_eq!(workflows.len(), 2);
    assert_eq!(workflows[0], first);
    assert_eq!(workflows[1], second);

    let unique_prefix_len = (1..=first.id().len())
        .find(|length| {
            let prefix = &first.id()[..*length];
            workflows
                .iter()
                .filter(|workflow| workflow.id().starts_with(prefix))
                .count()
                == 1
        })
        .expect("workflow id should have a unique prefix");
    let unique_prefix = &first.id()[..unique_prefix_len];

    assert_eq!(
        service
            .resolve_workflow_ref(session.id(), first.id())
            .expect("workflow id should resolve")
            .id(),
        first.id()
    );
    assert_eq!(
        service
            .resolve_workflow_ref(session.id(), unique_prefix)
            .expect("workflow id prefix should resolve")
            .id(),
        first.id()
    );
    assert_eq!(
        service
            .resolve_workflow_ref(session.id(), "review_loop")
            .expect("workflow alias should resolve")
            .id(),
        first.id()
    );
    assert_eq!(
        service
            .resolve_workflow_ref(session.id(), "review")
            .expect("workflow alias prefix should resolve")
            .id(),
        first.id()
    );
    assert!(first.flush_agent_context_before_run());
}

#[test]
fn create_workflow_generates_default_aliases() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let first = service
        .create_workflow(session.id(), None)
        .expect("workflow should be created");
    let second = service
        .create_workflow(session.id(), Some(" ".to_string()))
        .expect("blank alias should allocate a default workflow alias");

    assert_eq!(first.alias(), Some("workflow-1"));
    assert_eq!(second.alias(), Some("workflow-2"));
}

#[test]
fn workflow_design_create_generates_default_alias() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let created = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::WorkflowCreate {
                workflow: crate::local::WorkflowDesignWorkflow {
                    id: "workflow-design-1".to_string(),
                    alias: None,
                    prompt: None,
                    flush_agent_context_before_run: None,
                    max_concurrent: None,
                    run_output_schema_ref: None,
                    schemas: Vec::new(),
                },
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("workflow design create should apply");

    assert_eq!(created.id(), "workflow-design-1");
    assert_eq!(created.alias(), Some("workflow-1"));
}

#[test]
fn workflow_design_create_and_update_persist_workflow_prompt() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let created = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::WorkflowCreate {
                workflow: crate::local::WorkflowDesignWorkflow {
                    id: "workflow-design-prompt".to_string(),
                    alias: Some("prompted".to_string()),
                    prompt: Some("  Shared context for all nodes  ".to_string()),
                    flush_agent_context_before_run: None,
                    max_concurrent: None,
                    run_output_schema_ref: None,
                    schemas: Vec::new(),
                },
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("workflow design create should apply");

    assert_eq!(created.prompt(), Some("Shared context for all nodes"));

    let updated = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::WorkflowUpdate {
                workflow_id: created.id().to_string(),
                patch: crate::local::WorkflowDesignWorkflowPatch {
                    alias: None,
                    prompt: Some(None),
                    flush_agent_context_before_run: None,
                    max_concurrent: None,
                    run_output_schema_ref: None,
                },
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("workflow design update should apply");

    assert_eq!(updated.prompt(), None);
}

#[test]
fn creates_lists_resolves_and_disables_workflow_publications() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
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
            Some("main".to_string()),
        )
        .expect("workflow endpoint should be created");

    let publication = service
        .create_workflow_publication(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("default".to_string()),
            Some("public_review".to_string()),
            Some("ingress".to_string()),
            Some("/review".to_string()),
            vec!["POST".to_string()],
            Some(serde_json::json!({"kind": "human_http"})),
            Some(serde_json::json!({"kind": "path_template", "template": "/review/:prompt"})),
            None,
            None,
            Some("async".to_string()),
            None,
            None,
            "local".to_string(),
        )
        .expect("workflow publication should be created");

    assert_eq!(publication.workflow_id(), workflow.id());
    assert_eq!(publication.endpoint_id(), endpoint.id());
    assert_eq!(publication.queue_ref(), Some("default"));
    assert_eq!(publication.alias(), Some("public_review"));
    assert!(publication.enabled());

    let publications = service
        .list_workflow_publications(session.id())
        .expect("publication list should succeed");
    assert_eq!(publications, vec![publication.clone()]);
    assert_eq!(
        service
            .resolve_workflow_publication_ref(session.id(), "public")
            .expect("publication alias prefix should resolve")
            .id(),
        publication.id()
    );

    let served = service
        .register_workflow_publication_endpoint(
            session.id(),
            publication.id(),
            "running",
            "https://relay.example.test/display/publication-1/",
            serde_json::json!({
                "kind": "tunnel",
                "url": "https://relay.example.test/display/publication-1/",
                "local_url": "http://127.0.0.1:3000/"
            }),
        )
        .expect("publication endpoint should register");
    assert_eq!(served.status(), Some("running"));
    assert_eq!(
        served.open_url(),
        Some("https://relay.example.test/display/publication-1/")
    );
    assert_eq!(
        served.viewer_url(),
        Some("https://relay.example.test/display/publication-1/")
    );
    assert!(served.runtime_last_heartbeat_at_ms().is_some());
    assert_eq!(served.runtime_last_error(), None);
    assert_eq!(served.runtime_logs().len(), 1);
    assert_eq!(
        served.runtime_logs()[0].message,
        "publication endpoint running"
    );
    assert_eq!(
        served
            .deployment()
            .and_then(|deployment| deployment.pointer("/kind"))
            .and_then(serde_json::Value::as_str),
        Some("tunnel")
    );

    let failed = service
        .mark_workflow_publication_runtime_error(
            session.id(),
            publication.id(),
            "gateway exited immediately",
        )
        .expect("publication runtime error should update");
    assert_eq!(failed.status(), Some("error"));
    assert_eq!(
        failed.runtime_last_error(),
        Some("gateway exited immediately")
    );
    assert_eq!(failed.runtime_logs().len(), 2);
    assert_eq!(failed.runtime_logs()[1].level, "error");

    let stopped = service
        .mark_workflow_publication_runtime_status(
            session.id(),
            publication.id(),
            "stopped",
            Some(None),
            Some(serde_json::json!({
                "kind": "local_runtime",
                "status": "stopped"
            })),
        )
        .expect("publication runtime status should update");
    assert_eq!(stopped.status(), Some("stopped"));
    assert_eq!(stopped.open_url(), None);
    assert_eq!(stopped.viewer_url(), None);
    assert_eq!(
        stopped
            .deployment()
            .and_then(|deployment| deployment.pointer("/status"))
            .and_then(serde_json::Value::as_str),
        Some("stopped")
    );

    let disabled = service
        .disable_workflow_publication(session.id(), publication.id())
        .expect("publication should be disabled");
    assert!(!disabled.enabled());
}

#[test]
fn workflow_flush_context_defaults_true_and_can_be_updated() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    assert!(workflow.flush_agent_context_before_run());

    let updated = service
        .set_workflow_flush_agent_context_before_run(session.id(), workflow.id(), false)
        .expect("workflow flush setting should update");
    assert!(!updated.flush_agent_context_before_run());
}

#[test]
fn workflow_run_output_and_node_completion_settings_can_be_updated() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");

    let updated_workflow = service
        .set_workflow_run_output_schema_ref(
            session.id(),
            workflow.id(),
            Some("/tmp/workflow-run-output-schema.json".to_string()),
        )
        .expect("workflow run output schema should update");
    assert_eq!(
        updated_workflow.run_output_schema_ref(),
        Some("/tmp/workflow-run-output-schema.json")
    );
    let updated_node = service
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("node completion setting should update");
    assert!(updated_node.can_complete_workflow_run());
    let layout_with_exit = service
        .update_workflow_canvas_layout(
            session.id(),
            workflow.id(),
            vec![WorkflowCanvasLayoutPatch::ExitPosition {
                node_id: node.id().to_string(),
                x: 10,
                y: 20,
            }],
        )
        .expect("exit position should update");
    assert!(layout_with_exit.exits.get(node.id()).is_some());
    let updated_node = service
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), false)
        .expect("node completion setting should update");
    assert!(!updated_node.can_complete_workflow_run());
    let updated_workflow = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should exist");
    assert!(updated_workflow
        .canvas_layout()
        .is_none_or(|layout| !layout.exits.contains_key(node.id())));
    let updated_node = service
        .set_workflow_node_can_emit_intermediate_output(
            session.id(),
            workflow.id(),
            node.id(),
            true,
        )
        .expect("node intermediate output capability should update");
    assert!(updated_node.can_emit_intermediate_run_output());
    assert!(!updated_node.can_complete_workflow_run());
    let updated_node = service
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("node completion setting should update");
    assert!(updated_node.can_complete_workflow_run());
    assert!(updated_node.can_emit_intermediate_run_output());
    let updated_node = service
        .set_workflow_node_intermediate_output_schema_ref(
            session.id(),
            workflow.id(),
            node.id(),
            Some("/tmp/node-intermediate-output-schema.json".to_string()),
        )
        .expect("node intermediate output schema should update");
    assert_eq!(
        updated_node.intermediate_output_schema_ref(),
        Some("/tmp/node-intermediate-output-schema.json")
    );

    let updated_node = service
        .set_workflow_node_max_turns(session.id(), workflow.id(), node.id(), Some(3))
        .expect("node max turns should update");
    assert_eq!(updated_node.max_turns(), Some(3));
}

#[test]
fn workflow_design_edge_update_applies_handoff_schema_patch() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let planner = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("planner node should be added");
    let reviewer = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("reviewer node should be added");
    let edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            planner.id(),
            reviewer.id(),
            None,
            None,
        )
        .expect("edge should be added");

    let updated = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::EdgeUpdate {
                workflow_id: workflow.id().to_string(),
                edge_id: edge.id().to_string(),
                patch: crate::local::WorkflowDesignEdgePatch {
                    handoff_schema_ref: Some(Some("/tmp/handoff-schema.json".to_string())),
                    validation_policy: Some(Some(WorkflowHandoffValidationPolicy::Warn)),
                },
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("edge update design op should apply");

    let updated_edge = updated
        .edge(edge.id())
        .expect("updated workflow should keep the edge");
    assert_eq!(
        updated_edge.handoff_schema_ref(),
        Some("/tmp/handoff-schema.json")
    );
    assert_eq!(
        updated_edge.validation_policy(),
        Some(WorkflowHandoffValidationPolicy::Warn)
    );

    let clear_op: crate::local::WorkflowDesignOp = serde_json::from_value(serde_json::json!({
        "kind": "edge_update",
        "workflow_id": workflow.id(),
        "edge_id": edge.id(),
        "patch": {
            "handoff_schema_ref": null,
            "validation_policy": null
        }
    }))
    .expect("explicit null edge patch should deserialize");
    let cleared = service
        .apply_workflow_design_op(session.id(), clear_op, DEFAULT_LOCAL_USER_ID.to_string())
        .expect("edge clear design op should apply");

    let cleared_edge = cleared
        .edge(edge.id())
        .expect("cleared workflow should keep the edge");
    assert_eq!(cleared_edge.handoff_schema_ref(), None);
    assert_eq!(cleared_edge.validation_policy(), None);
}

#[test]
fn workflow_design_schema_ops_add_update_and_remove_embedded_schema() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");

    let added = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::SchemaAdd {
                workflow_id: workflow.id().to_string(),
                schema: crate::session::WorkflowSchemaDefinition::new(
                    "schema-1",
                    Some("Draft".to_string()),
                    Some("Draft payload".to_string()),
                    serde_json::json!({
                        "type": "object",
                        "required": ["summary"],
                        "properties": {
                            "summary": { "type": "string" }
                        }
                    }),
                ),
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("valid schema should be added");
    assert_eq!(added.schemas().len(), 1);
    assert_eq!(
        added.schema("schema-1").and_then(|schema| schema.alias()),
        Some("Draft")
    );

    let updated = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::SchemaUpdate {
                workflow_id: workflow.id().to_string(),
                schema_id: "schema-1".to_string(),
                patch: crate::local::WorkflowDesignSchemaPatch {
                    alias: Some(Some("Review".to_string())),
                    description: Some(None),
                    schema: Some(serde_json::json!({
                        "type": "object",
                        "required": ["verdict"],
                        "properties": {
                            "verdict": { "enum": ["approve", "reject"] }
                        }
                    })),
                },
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("valid schema update should apply");
    let schema = updated.schema("schema-1").expect("schema should remain");
    assert_eq!(schema.alias(), Some("Review"));
    assert_eq!(schema.description(), None);
    assert_eq!(
        schema.schema().pointer("/properties/verdict/enum/0"),
        Some(&serde_json::json!("approve"))
    );

    let removed = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::SchemaRemove {
                workflow_id: workflow.id().to_string(),
                schema_id: "schema-1".to_string(),
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("unreferenced schema should be removed");
    assert!(removed.schema("schema-1").is_none());
}

#[test]
fn workflow_design_schema_ops_reject_invalid_schema() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");

    let error = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::SchemaAdd {
                workflow_id: workflow.id().to_string(),
                schema: crate::session::WorkflowSchemaDefinition::new(
                    "schema-1",
                    None,
                    None,
                    serde_json::json!({ "type": 42 }),
                ),
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect_err("invalid schema should be rejected");

    assert!(matches!(error, DaemonError::LocalTransport { .. }));
}

#[test]
fn workflow_design_schema_remove_rejects_referenced_schema() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let planner = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("planner node should be added");
    let reviewer = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("reviewer node should be added");
    service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::SchemaAdd {
                workflow_id: workflow.id().to_string(),
                schema: crate::session::WorkflowSchemaDefinition::new(
                    "schema-1",
                    None,
                    None,
                    serde_json::json!({ "type": "object" }),
                ),
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("schema should be added");
    let edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            planner.id(),
            reviewer.id(),
            Some("schema-1".to_string()),
            Some(WorkflowHandoffValidationPolicy::Halt),
        )
        .expect("edge should be added");

    let error = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::SchemaRemove {
                workflow_id: workflow.id().to_string(),
                schema_id: "schema-1".to_string(),
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect_err("referenced schema should not be removed");

    match error {
        DaemonError::LocalTransport { message, .. } => {
            assert!(message.contains(&format!("edge.{}.handoff_schema_ref", edge.id())));
        }
        other => panic!("expected LocalTransport, got {other:?}"),
    }
}

#[test]
fn manages_workflow_nodes_edges_and_endpoints() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");

    let planner = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("planner node should be added");
    let duplicate_node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect_err("duplicate workflow node should be rejected");
    assert!(matches!(
        duplicate_node,
        DaemonError::WorkflowNodeConflict { .. }
    ));
    let reviewer = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("reviewer node should be added");

    let edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            planner.id(),
            reviewer.id(),
            None,
            None,
        )
        .expect("edge should be added");
    assert_eq!(edge.from_node_id(), planner.id());
    assert_eq!(edge.to_node_id(), reviewer.id());

    let duplicate_edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            planner.id(),
            reviewer.id(),
            None,
            None,
        )
        .expect_err("duplicate edge should be rejected");
    assert!(matches!(
        duplicate_edge,
        DaemonError::WorkflowEdgeConflict { .. }
    ));

    let self_edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            planner.id(),
            planner.id(),
            None,
            None,
        )
        .expect_err("self edge should be rejected");
    assert!(matches!(
        self_edge,
        DaemonError::InvalidWorkflowGraphReference { .. }
    ));

    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            planner.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    assert_eq!(endpoint.entry_node_id(), planner.id());

    assert_eq!(
        service
            .resolve_workflow_endpoint_ref(session.id(), workflow.id(), "entry")
            .expect("endpoint alias should resolve")
            .id(),
        endpoint.id()
    );

    let rebound = service
        .bind_workflow_endpoint(session.id(), workflow.id(), endpoint.id(), reviewer.id())
        .expect("endpoint should be rebound");
    assert_eq!(rebound.entry_node_id(), reviewer.id());

    let aliased = service
        .assign_workflow_endpoint_alias(
            session.id(),
            workflow.id(),
            endpoint.id(),
            "review-entry".to_string(),
        )
        .expect("endpoint alias should be updated");
    assert_eq!(aliased.alias(), Some("review-entry"));

    let removed_edge = service
        .remove_workflow_edge(session.id(), workflow.id(), edge.id())
        .expect("edge should be removed");
    assert_eq!(removed_edge.id(), edge.id());

    let removed_node = service
        .remove_workflow_node(session.id(), workflow.id(), planner.id())
        .expect("node should be removed");
    assert_eq!(removed_node.id(), planner.id());
}
