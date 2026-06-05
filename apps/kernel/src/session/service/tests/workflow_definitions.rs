use super::*;
use crate::session::WorkflowHandoffValidationPolicy;

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
            Some("public_review".to_string()),
            Some("/review".to_string()),
            vec!["POST".to_string()],
            Some(serde_json::json!({"kind": "http"})),
            Some(serde_json::json!({"kind": "webhook"})),
            None,
            Some("async".to_string()),
            "local".to_string(),
        )
        .expect("workflow publication should be created");

    assert_eq!(publication.workflow_id(), workflow.id());
    assert_eq!(publication.endpoint_id(), endpoint.id());
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
    let updated_workflow = service
        .set_workflow_intermediate_output_schema_ref(
            session.id(),
            workflow.id(),
            Some("/tmp/workflow-intermediate-output-schema.json".to_string()),
        )
        .expect("workflow intermediate output schema should update");
    assert_eq!(
        updated_workflow.intermediate_output_schema_ref(),
        Some("/tmp/workflow-intermediate-output-schema.json")
    );

    let updated_node = service
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("node completion setting should update");
    assert!(updated_node.can_complete_workflow_run());
    let updated_node = service
        .set_workflow_node_can_emit_intermediate_output(
            session.id(),
            workflow.id(),
            node.id(),
            true,
        )
        .expect("node intermediate output capability should update");
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
