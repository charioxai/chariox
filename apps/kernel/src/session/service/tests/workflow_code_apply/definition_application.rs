use super::*;

#[test]
fn applies_workflow_code_definition_to_session_primitives() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let definition = workflow_code_definition();
    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);
    let owner_user_id = "workflow-code-owner".to_string();
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            owner_user_id.clone(),
            Some("meta-1".to_string()),
        )
        .expect("workflow-code should apply");

    assert_eq!(report.node_ids.len(), 2);
    assert_eq!(report.schema_refs.len(), 3);
    assert_ne!(
        report.schema_refs.get("final").map(String::as_str),
        Some("final")
    );
    assert_eq!(report.edge_ids.len(), 1);
    assert_eq!(report.endpoint_ids.len(), 1);
    assert_eq!(report.queue_ids.len(), 1);
    assert_eq!(report.schedule_ids.len(), 1);
    assert_ne!(
        report.node_ids.get("planner").map(String::as_str),
        Some("planner"),
        "script node handles must not become kernel node ids"
    );
    assert_ne!(
        report.node_ids.get("worker").map(String::as_str),
        Some("worker"),
        "script node handles must not become kernel node ids"
    );
    assert_ne!(
        report.edge_ids.get("planner_to_worker").map(String::as_str),
        Some("planner_to_worker"),
        "script edge handles must not become kernel edge ids"
    );
    assert_ne!(
        report.endpoint_ids.get("entry").map(String::as_str),
        Some("entry"),
        "script endpoint handles must not become kernel endpoint ids"
    );
    assert_ne!(
        report.queue_ids.get("urgent").map(String::as_str),
        Some("urgent"),
        "script queue handles must not become kernel queue ids"
    );
    assert_ne!(
        report
            .schedule_ids
            .get("entry_watchdog")
            .map(String::as_str),
        Some("entry_watchdog"),
        "script watchdog handles must not become kernel watchdog ids"
    );
    assert!(report.canvas_layout_applied);
    assert!(report.warnings.is_empty());

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    assert_eq!(workflow.alias(), Some("coded_flow"));
    assert_eq!(workflow.controlled_by_metaagent_id(), Some("meta-1"));
    assert!(!workflow.flush_agent_context_before_run());
    assert_eq!(workflow.max_concurrent(), 2);
    assert_eq!(workflow.nodes().len(), 2);
    assert_eq!(workflow.edges().len(), 1);
    assert_eq!(workflow.endpoints().len(), 1);
    assert_eq!(workflow.schemas().len(), 3);
    let final_schema_id = report.schema_refs.get("final").expect("final schema id");
    let progress_schema_id = report
        .schema_refs
        .get("progress")
        .expect("progress schema id");
    let handoff_schema_id = report
        .schema_refs
        .get("handoff")
        .expect("handoff schema id");
    assert_eq!(
        workflow.run_output_schema_ref(),
        Some(final_schema_id.as_str())
    );
    assert_eq!(
        workflow
            .schema(final_schema_id)
            .and_then(|schema| schema.alias()),
        Some("Final")
    );

    let planner_id = report.node_ids.get("planner").expect("planner id");
    let planner = workflow.node(planner_id).expect("planner node");
    assert_eq!(planner.agent_id(), "agent-1");
    assert_eq!(planner.public_label(), "Planner");
    assert_eq!(planner.instructions(), Some("Plan the task."));
    assert!(planner.can_emit_intermediate_run_output());
    assert_eq!(
        planner.intermediate_output_schema_ref(),
        Some(progress_schema_id.as_str())
    );
    assert_eq!(planner.max_turns(), Some(4));

    let edge_id = report.edge_ids.get("planner_to_worker").expect("edge id");
    let edge = workflow.edge(edge_id).expect("workflow edge");
    assert_eq!(edge.handoff_schema_ref(), Some(handoff_schema_id.as_str()));

    let endpoint_id = report.endpoint_ids.get("entry").expect("entry id");
    let endpoint = workflow.endpoint(endpoint_id).expect("workflow endpoint");
    assert_eq!(endpoint.owner_user_id(), owner_user_id);

    let urgent_queue_id = report.queue_ids.get("urgent").expect("urgent queue id");
    let urgent_queue = session
        .workflow_prompt_queue(&report.workflow_id, urgent_queue_id)
        .expect("urgent queue should exist");
    assert_eq!(urgent_queue.alias(), "urgent");
    assert_eq!(urgent_queue.priority(), 10);
    assert!(!urgent_queue.enabled());

    let watchdog = session
        .workflow_watchdog(
            report
                .schedule_ids
                .get("entry_watchdog")
                .expect("watchdog id"),
        )
        .expect("watchdog should exist");
    assert_eq!(watchdog.workflow_id(), report.workflow_id);
    assert!(!watchdog.enabled());
    assert_eq!(watchdog.interval_seconds(), 60);
    assert_eq!(watchdog.max_wakeups(), Some(2));
    assert_eq!(watchdog.queue_id(), Some(urgent_queue_id.as_str()));

    let layout = workflow
        .canvas_layout()
        .expect("canvas layout should exist");
    assert!(layout.nodes.contains_key(planner_id));
    assert!(layout.endpoints.contains_key(endpoint_id));
    assert!(layout
        .edges
        .contains_key(report.edge_ids.get("planner_to_worker").expect("edge id")));
}

#[test]
fn workflow_code_definition_alias_base_allocates_template_aliases() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let definition = workflow_code_definition();
    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);

    let first = service
        .apply_workflow_code_definition_with_alias_base(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            "workflow-code-owner".to_string(),
            None,
            Some("Prompt Chaining"),
        )
        .expect("first template workflow should apply");
    let second = service
        .apply_workflow_code_definition_with_alias_base(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            "workflow-code-owner".to_string(),
            None,
            Some("Prompt Chaining"),
        )
        .expect("second template workflow should apply");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(
        session
            .workflow(&first.workflow_id)
            .and_then(|workflow| workflow.alias()),
        Some("prompt-chaining-1")
    );
    assert_eq!(
        session
            .workflow(&second.workflow_id)
            .and_then(|workflow| workflow.alias()),
        Some("prompt-chaining-2")
    );
}

#[test]
fn workflow_code_apply_uses_configured_default_concurrency_when_script_omits_it() {
    let mut config = test_config();
    config.user_config.workflow.code = Some(crate::config::UserWorkflowCodeConfig {
        max_concurrent: Some(7),
        ..Default::default()
    });
    let mut service = SessionService::new(&config);
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.workflow.max_concurrent = None;
    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);

    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &config.workflow_code_limits(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow-code should apply with configured default concurrency");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    assert_eq!(workflow.max_concurrent(), 7);
}

#[test]
fn workflow_code_apply_preserves_node_intermediate_schema_override() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.schemas.push(WorkflowCodeSchemaDefinition {
        handle: "node_progress".to_string(),
        alias: Some("node-progress".to_string()),
        description: Some("Node-specific progress output".to_string()),
        schema: serde_json::json!({
            "type": "object",
            "required": ["node_value"],
            "properties": {
                "node_value": { "type": "number" }
            },
            "additionalProperties": false
        }),
    });
    definition.nodes[0].intermediate_output_schema = Some("node_progress".to_string());

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow-code should apply with node-specific intermediate schema");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    let planner_id = report.node_ids.get("planner").expect("planner id");
    let planner = workflow
        .node(planner_id)
        .expect("planner node should exist");

    assert_eq!(
        planner.intermediate_output_schema_ref(),
        report.schema_refs.get("node_progress").map(String::as_str)
    );
}
