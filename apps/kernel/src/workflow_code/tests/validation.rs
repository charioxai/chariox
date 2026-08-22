use super::*;

#[test]
fn workflow_registry_summary_uses_workflow_code_handles() {
    let summary = WorkflowRegistryEntrySummary::from_definition(&multi_endpoint_definition());

    assert_eq!(summary.endpoints, vec!["entry", "review"]);
    assert_eq!(summary.queues, vec!["default", "urgent"]);
    assert_eq!(summary.nodes, vec!["planner"]);
    assert_eq!(summary.default_endpoint.as_deref(), Some("entry"));
}

#[test]
fn validates_minimal_workflow_code_definition() {
    let definition = minimal_definition();
    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

    assert!(report.ok, "{:?}", report.diagnostics);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn canvas_contract_matches_documented_dimensions() {
    let contract = workflow_code_canvas_contract();

    assert_eq!(
        contract
            .pointer("/coordinate_space")
            .and_then(Value::as_str),
        Some(WORKFLOW_CODE_CANVAS_COORDINATE_SPACE)
    );
    assert_eq!(
        contract.pointer("/node/width").and_then(Value::as_i64),
        Some(232)
    );
    assert_eq!(
        contract.pointer("/endpoint/width").and_then(Value::as_i64),
        Some(180)
    );
    assert_eq!(
        contract.pointer("/minimum_gap").and_then(Value::as_i64),
        Some(36)
    );
    assert_eq!(
        contract
            .pointer("/default_endpoint_offset/x")
            .and_then(Value::as_i64),
        Some(-220)
    );
}

#[test]
fn rejects_explicit_canvas_box_collisions() {
    let mut definition = minimal_definition();
    definition.endpoints[0].canvas = Some(WorkflowCodeCanvasPoint { x: -180, y: 0 });

    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

    assert!(!report.ok);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "canvas_overlap"
            && diagnostic.handle.as_deref() == Some("entry")
            && diagnostic
                .message
                .contains(WORKFLOW_CODE_CANVAS_COORDINATE_SPACE)
            && diagnostic.message.contains("36 canvas units")
    }));

    definition.endpoints[0].canvas = Some(WorkflowCodeCanvasPoint { x: -220, y: 0 });
    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

    assert!(report.ok, "{:?}", report.diagnostics);
}

#[test]
fn rejects_endpoint_max_instances_outside_hard_range() {
    for invalid in [0u16, 33] {
        let mut definition = minimal_definition();
        definition.endpoints[0].max_instances = Some(invalid);

        let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

        assert!(!report.ok, "max_instances={invalid} must be rejected");
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "invalid_endpoint_max_instances"
                && diagnostic.handle.as_deref() == Some("entry")
                && diagnostic.message.contains("between 1 and 32")
        }));
    }

    for valid in [1u16, 32] {
        let mut definition = minimal_definition();
        definition.endpoints[0].max_instances = Some(valid);

        let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

        assert!(report.ok, "max_instances={valid}: {:?}", report.diagnostics);
    }
}

#[test]
fn source_export_drops_a_legacy_canvas_layout_that_cannot_be_reused() {
    let mut definition = minimal_definition();
    definition.nodes[0].canvas = Some(WorkflowCodeCanvasPoint { x: 229, y: 121 });
    definition.nodes[0].can_complete_workflow_run = Some(true);
    definition.endpoints[0].canvas = Some(WorkflowCodeCanvasPoint { x: 540, y: 120 });

    let before = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
    assert!(before
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "canvas_overlap"));

    strip_invalid_export_canvas_layout(&mut definition);

    assert!(definition.nodes.iter().all(|node| node.canvas.is_none()));
    assert!(definition.edges.iter().all(|edge| edge.canvas.is_none()));
    assert!(definition
        .endpoints
        .iter()
        .all(|endpoint| endpoint.canvas.is_none()));
    let after = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
    assert!(after.ok, "{:?}", after.diagnostics);
}

#[test]
fn rejects_exit_marker_canvas_collisions() {
    let mut definition = minimal_definition();
    definition.nodes.push(WorkflowCodeNodeDefinition {
        handle: "next".to_string(),
        agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
            alias: Some("next".to_string()),
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            account_profile: None,
        }),
        public_label: None,
        instructions: None,
        can_complete_workflow_run: None,
        can_emit_intermediate_run_output: None,
        wait_for_all_inputs: None,
        intermediate_output_schema: None,
        max_turns: None,
        extensions: Vec::new(),
        canvas: Some(WorkflowCodeCanvasPoint { x: 360, y: 28 }),
    });
    definition.edges.push(WorkflowCodeEdgeDefinition {
        handle: "planner_to_next".to_string(),
        from_node: "planner".to_string(),
        to_node: "next".to_string(),
        source_side: None,
        target_side: None,
        handoff_schema: None,
        validation_policy: None,
        canvas: None,
    });

    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

    assert!(!report.ok);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "canvas_overlap"
            && diagnostic.message.contains("exit_marker `planner`")
            && diagnostic.message.contains("node `next`")
    }));
}

#[test]
fn rejects_invalid_aliases_and_duplicate_queue_aliases() {
    let mut definition = minimal_definition();
    definition.workflow.alias = Some("bad alias".to_string());
    definition.endpoints[0].alias = Some("bad/endpoint".to_string());
    definition.endpoints.push(WorkflowCodeEndpointDefinition {
        handle: "duplicate_endpoint".to_string(),
        entry_node: "planner".to_string(),
        alias: Some("ENTRY".to_string()),
        max_instances: None,
        canvas: None,
    });
    definition.endpoints.push(WorkflowCodeEndpointDefinition {
        handle: "duplicate_endpoint_copy".to_string(),
        entry_node: "planner".to_string(),
        alias: Some("entry".to_string()),
        max_instances: None,
        canvas: None,
    });
    definition.queues = vec![
        WorkflowCodeQueueDefinition {
            handle: "urgent".to_string(),
            alias: "urgent".to_string(),
            priority: 10,
            enabled: true,
        },
        WorkflowCodeQueueDefinition {
            handle: "urgent_copy".to_string(),
            alias: "URGENT".to_string(),
            priority: 5,
            enabled: true,
        },
        WorkflowCodeQueueDefinition {
            handle: "default".to_string(),
            alias: "default".to_string(),
            priority: 0,
            enabled: true,
        },
        WorkflowCodeQueueDefinition {
            handle: "default_copy".to_string(),
            alias: "default".to_string(),
            priority: -1,
            enabled: true,
        },
        WorkflowCodeQueueDefinition {
            handle: "empty".to_string(),
            alias: " ".to_string(),
            priority: 0,
            enabled: true,
        },
    ];

    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
    let invalid_alias_count = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "invalid_alias")
        .count();
    let duplicate_queue_alias_count = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "duplicate_queue_alias")
        .count();
    let duplicate_endpoint_alias_count = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "duplicate_endpoint_alias")
        .count();

    assert!(!report.ok);
    assert_eq!(invalid_alias_count, 3, "{:?}", report.diagnostics);
    assert_eq!(duplicate_queue_alias_count, 2, "{:?}", report.diagnostics);
    assert_eq!(
        duplicate_endpoint_alias_count, 1,
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn reserves_default_queue_handle_for_default_queue() {
    let mut definition = minimal_definition();
    definition.queues = vec![WorkflowCodeQueueDefinition {
        handle: "default".to_string(),
        alias: "urgent".to_string(),
        priority: 10,
        enabled: true,
    }];

    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

    assert!(!report.ok);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "reserved_queue_handle"
            && diagnostic.handle.as_deref() == Some("default")
            && diagnostic.message.contains("kernel default queue")
    }));

    definition.queues[0].alias = " Default ".to_string();
    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

    assert!(report.ok, "{:?}", report.diagnostics);
}

#[test]
fn rejects_unknown_graph_references_and_duplicate_existing_agents() {
    let mut definition = minimal_definition();
    definition.nodes.push(WorkflowCodeNodeDefinition {
        handle: "reviewer".to_string(),
        agent: WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
            agent_ref: "agent-1".to_string(),
        }),
        public_label: None,
        instructions: None,
        can_complete_workflow_run: None,
        can_emit_intermediate_run_output: None,
        wait_for_all_inputs: None,
        intermediate_output_schema: Some("missing-schema".to_string()),
        max_turns: None,
        extensions: Vec::new(),
        canvas: None,
    });
    definition.nodes.push(WorkflowCodeNodeDefinition {
        handle: "duplicate".to_string(),
        agent: WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
            agent_ref: "agent-1".to_string(),
        }),
        public_label: None,
        instructions: None,
        can_complete_workflow_run: None,
        can_emit_intermediate_run_output: None,
        wait_for_all_inputs: None,
        intermediate_output_schema: None,
        max_turns: None,
        extensions: Vec::new(),
        canvas: None,
    });
    definition.edges.push(WorkflowCodeEdgeDefinition {
        handle: "bad-edge".to_string(),
        from_node: "planner".to_string(),
        to_node: "missing-node".to_string(),
        source_side: None,
        target_side: None,
        handoff_schema: Some("missing-schema".to_string()),
        validation_policy: Some(WorkflowHandoffValidationPolicy::Warn),
        canvas: None,
    });

    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
    let codes = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();

    assert!(!report.ok);
    assert!(codes.contains(&"unknown_reference"));
    assert!(codes.contains(&"duplicate_existing_agent"));
}

#[test]
fn rejects_self_edges_and_duplicate_edge_pairs() {
    let mut definition = minimal_definition();
    definition.nodes.push(WorkflowCodeNodeDefinition {
        handle: "reviewer".to_string(),
        agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
            alias: Some("Reviewer".to_string()),
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            account_profile: None,
        }),
        public_label: None,
        instructions: None,
        can_complete_workflow_run: None,
        can_emit_intermediate_run_output: None,
        wait_for_all_inputs: None,
        intermediate_output_schema: None,
        max_turns: None,
        extensions: Vec::new(),
        canvas: None,
    });
    definition.edges.push(WorkflowCodeEdgeDefinition {
        handle: "planner_self".to_string(),
        from_node: "planner".to_string(),
        to_node: "planner".to_string(),
        source_side: None,
        target_side: None,
        handoff_schema: None,
        validation_policy: None,
        canvas: None,
    });
    definition.edges.push(WorkflowCodeEdgeDefinition {
        handle: "plan_to_review".to_string(),
        from_node: "planner".to_string(),
        to_node: "reviewer".to_string(),
        source_side: None,
        target_side: None,
        handoff_schema: None,
        validation_policy: None,
        canvas: None,
    });
    definition.edges.push(WorkflowCodeEdgeDefinition {
        handle: "plan_to_review_again".to_string(),
        from_node: "planner".to_string(),
        to_node: "reviewer".to_string(),
        source_side: None,
        target_side: None,
        handoff_schema: None,
        validation_policy: None,
        canvas: None,
    });

    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
    let codes = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();

    assert!(!report.ok);
    assert!(codes.contains(&"invalid_edge"), "{:?}", report.diagnostics);
    assert!(
        codes.contains(&"duplicate_edge"),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn rejects_nodes_unreachable_from_endpoints() {
    let mut definition = minimal_definition();
    definition.nodes.push(WorkflowCodeNodeDefinition {
        handle: "reviewer".to_string(),
        agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
            alias: Some("reviewer".to_string()),
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            account_profile: None,
        }),
        public_label: None,
        instructions: None,
        can_complete_workflow_run: None,
        can_emit_intermediate_run_output: None,
        wait_for_all_inputs: None,
        intermediate_output_schema: None,
        max_turns: None,
        extensions: Vec::new(),
        canvas: None,
    });
    definition.nodes.push(WorkflowCodeNodeDefinition {
        handle: "orphan".to_string(),
        agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
            alias: Some("orphan".to_string()),
            provider: "dev-stub".to_string(),
            model: Some("default".to_string()),
            effort: None,
            account_profile: None,
        }),
        public_label: None,
        instructions: None,
        can_complete_workflow_run: None,
        can_emit_intermediate_run_output: None,
        wait_for_all_inputs: None,
        intermediate_output_schema: None,
        max_turns: None,
        extensions: Vec::new(),
        canvas: None,
    });
    definition.edges.push(WorkflowCodeEdgeDefinition {
        handle: "plan_to_review".to_string(),
        from_node: "planner".to_string(),
        to_node: "reviewer".to_string(),
        source_side: None,
        target_side: None,
        handoff_schema: None,
        validation_policy: None,
        canvas: None,
    });
    definition.edges.push(WorkflowCodeEdgeDefinition {
        handle: "orphan_to_review".to_string(),
        from_node: "orphan".to_string(),
        to_node: "reviewer".to_string(),
        source_side: None,
        target_side: None,
        handoff_schema: None,
        validation_policy: None,
        canvas: None,
    });

    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

    assert!(!report.ok);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unreachable_node" && diagnostic.handle.as_deref() == Some("orphan")
    }));
    assert!(!report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unreachable_node" && diagnostic.handle.as_deref() == Some("reviewer")
    }));
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unreachable_edge"
            && diagnostic.handle.as_deref() == Some("orphan_to_review")
    }));
    assert!(!report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unreachable_edge"
            && diagnostic.handle.as_deref() == Some("plan_to_review")
    }));
}

#[test]
fn enforces_configured_limits() {
    let mut definition = minimal_definition();
    definition.workflow.max_concurrent = Some(64);
    let limits = WorkflowCodeLimitsConfig {
        max_concurrent: 32,
        max_nodes: 0,
        ..WorkflowCodeLimitsConfig::default()
    };

    let report = definition.validate_with_limits(&limits);

    assert!(!report.ok);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "limit_exceeded"));
}

#[test]
fn enforces_materialized_queue_limit_including_default_queue() {
    let mut definition = minimal_definition();
    definition.queues = vec![WorkflowCodeQueueDefinition {
        handle: "urgent".to_string(),
        alias: "urgent".to_string(),
        priority: 5,
        enabled: true,
    }];
    let limits = WorkflowCodeLimitsConfig {
        max_queues: 1,
        ..WorkflowCodeLimitsConfig::default()
    };

    let report = definition.validate_with_limits(&limits);

    assert!(!report.ok);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "limit_exceeded" && diagnostic.message.contains("queues count 2 exceeds")
    }));
}

#[test]
fn enforces_endpoint_limit() {
    let definition = minimal_definition();
    let limits = WorkflowCodeLimitsConfig {
        max_endpoints: 0,
        ..WorkflowCodeLimitsConfig::default()
    };

    let report = definition.validate_with_limits(&limits);

    assert!(!report.ok);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "limit_exceeded"
            && diagnostic.message.contains("endpoints count 1 exceeds")
    }));
}

#[test]
fn enforces_generated_prompt_byte_limit() {
    let mut definition = minimal_definition();
    definition.nodes[0].instructions = Some("x".repeat(128));
    let limits = WorkflowCodeLimitsConfig {
        max_generated_prompt_bytes: 64,
        ..WorkflowCodeLimitsConfig::default()
    };

    let report = definition.validate_with_limits(&limits);

    assert!(!report.ok);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "limit_exceeded"
            && diagnostic
                .message
                .contains("workflow generated prompt text uses")
    }));
}

#[test]
fn validates_node_extension_grant_shape() {
    let mut definition = minimal_definition();
    definition.nodes[0]
        .extensions
        .push(ExtensionGrant::new(ExtensionKind::Skill, ""));
    definition.nodes[0]
        .extensions
        .push(ExtensionGrant::new(ExtensionKind::Script, "release-script"));
    definition.nodes[0].extensions.push(ExtensionGrant {
        kind: ExtensionKind::Connector,
        name: "deploy-api".to_string(),
        environment: None,
        credential: None,
        max_safety: Some("admin".to_string()),
    });

    let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
    let codes = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();

    assert!(!report.ok);
    assert!(codes.contains(&"invalid_extension_name"));
    assert!(codes.contains(&"invalid_extension_environment"));
    assert!(codes.contains(&"invalid_connector_safety"));
}

#[test]
fn provider_rebindings_normalize_optional_fields() {
    let mut definition = minimal_definition();

    apply_workflow_code_provider_rebindings(
        &mut definition,
        &[WorkflowCodeProviderRebinding {
            node: " planner ".to_string(),
            provider: " dev-stub ".to_string(),
            model: Some(" ".to_string()),
            effort: Some(" low ".to_string()),
            account_profile: Some(" default ".to_string()),
        }],
    )
    .expect("rebinding should apply");

    match &definition.nodes[0].agent {
        WorkflowCodeAgentBinding::Create(agent) => {
            assert_eq!(agent.provider, "dev-stub");
            assert_eq!(agent.model, None);
            assert_eq!(agent.effort.as_deref(), Some("low"));
            assert_eq!(agent.account_profile, None);
        }
        WorkflowCodeAgentBinding::Existing(_) => panic!("planner should use generated agent"),
    }
}

#[test]
fn agent_rebindings_rewrite_generated_agents_to_existing_agents() {
    let mut definition = minimal_definition();

    apply_workflow_code_agent_rebindings(
        &mut definition,
        &[WorkflowCodeAgentRebinding {
            node: " planner ".to_string(),
            agent_ref: " agent-1 ".to_string(),
        }],
    )
    .expect("agent rebinding should apply");

    match &definition.nodes[0].agent {
        WorkflowCodeAgentBinding::Existing(agent) => {
            assert_eq!(agent.agent_ref, "agent-1");
        }
        WorkflowCodeAgentBinding::Create(_) => panic!("planner should use existing agent"),
    }
}

#[test]
fn agent_rebindings_reject_invalid_inputs() {
    assert_agent_rebinding_error(
        &[WorkflowCodeAgentRebinding {
            node: " ".to_string(),
            agent_ref: "agent-1".to_string(),
        }],
        "node handle must not be empty",
    );
    assert_agent_rebinding_error(
        &[WorkflowCodeAgentRebinding {
            node: "planner".to_string(),
            agent_ref: " ".to_string(),
        }],
        "must include agent_ref",
    );
    assert_agent_rebinding_error(
        &[
            WorkflowCodeAgentRebinding {
                node: "planner".to_string(),
                agent_ref: "agent-1".to_string(),
            },
            WorkflowCodeAgentRebinding {
                node: "planner".to_string(),
                agent_ref: "agent-2".to_string(),
            },
        ],
        "duplicate agent rebinding",
    );
    assert_agent_rebinding_error(
        &[WorkflowCodeAgentRebinding {
            node: "missing".to_string(),
            agent_ref: "agent-1".to_string(),
        }],
        "unknown node",
    );

    let mut definition = minimal_definition();
    definition.nodes[0].agent = WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
        agent_ref: "agent-1".to_string(),
    });
    let error = apply_workflow_code_agent_rebindings(
        &mut definition,
        &[WorkflowCodeAgentRebinding {
            node: "planner".to_string(),
            agent_ref: "agent-2".to_string(),
        }],
    )
    .expect_err("rebinding existing agent node should fail");
    assert!(format!("{error}").contains("existing-agent binding"));
}

#[test]
fn rejects_unknown_fields_during_decode() {
    let value = serde_json::json!({
        "workflow": {},
        "nodes": [],
        "endpoints": [],
        "invented": true
    });

    let error = serde_json::from_value::<WorkflowCodeDefinition>(value)
        .expect_err("unknown workflow-code fields should be rejected");

    assert!(error.to_string().contains("unknown field"));
}
