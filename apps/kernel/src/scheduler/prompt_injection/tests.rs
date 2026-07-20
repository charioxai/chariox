use super::*;
use crate::env_lock;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_arroba_home(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let index = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join("arroba-workflow-prompt-tests")
        .join(format!("{}-{}-{index}", name, std::process::id()))
}

fn set_arroba_home(home: &PathBuf) -> Option<std::ffi::OsString> {
    let previous = std::env::var_os("ARROBA_HOME");
    std::env::set_var("ARROBA_HOME", home);
    previous
}

fn restore_arroba_home(previous: Option<std::ffi::OsString>) {
    if let Some(previous) = previous {
        std::env::set_var("ARROBA_HOME", previous);
    } else {
        std::env::remove_var("ARROBA_HOME");
    }
}

fn test_context() -> WorkflowPromptInjectionContext {
    WorkflowPromptInjectionContext {
        workflow_ref: Some("workflow-test".to_string()),
        endpoint_prompt: "ENDPOINT_VISIBLE_TOKEN".to_string(),
        workflow_prompt: "WORKFLOW_HIDDEN_TOKEN".to_string(),
        node_id: Some("node-test".to_string()),
        node_instructions: "NODE_HIDDEN_TOKEN".to_string(),
        instruction_ref: None,
        handoff_payloads_json: None,
        outgoing_edge_contracts: String::new(),
        control_mailbox: None,
        delivery_token: "workflow-ack:test".to_string(),
        node_turn: None,
        base_directory: None,
        hide_in_native_tui: false,
    }
}

#[test]
fn workflow_prompt_assembly_keeps_endpoint_visible_and_layers_hidden() {
    let assembly = build_workflow_turn_prompt_assembly(test_context());

    assert!(assembly
        .visible_user_prompt
        .starts_with("<endpoint-prompt>\n"));
    assert!(assembly
        .visible_user_prompt
        .contains("\n</endpoint-prompt>"));
    assert!(assembly
        .visible_user_prompt
        .contains("ENDPOINT_VISIBLE_TOKEN"));
    assert!(!assembly
        .visible_user_prompt
        .contains("WORKFLOW_HIDDEN_TOKEN"));
    assert!(!assembly.visible_user_prompt.contains("NODE_HIDDEN_TOKEN"));
    assert!(assembly
        .hidden_system_context
        .contains("WORKFLOW_HIDDEN_TOKEN"));
    assert!(assembly.hidden_system_context.contains("NODE_HIDDEN_TOKEN"));
    assert!(assembly
        .hidden_system_context
        .contains("<workflow-level-prompt>\nWORKFLOW_HIDDEN_TOKEN\n</workflow-level-prompt>"));
    assert!(assembly
        .hidden_system_context
        .contains("<node-level-prompt>\nNODE_HIDDEN_TOKEN\n</node-level-prompt>"));
    assert!(assembly
        .hidden_system_context
        .contains("<workflow-runtime-instructions>"));
    assert!(!assembly
        .hidden_system_context
        .contains("ENDPOINT_VISIBLE_TOKEN"));
    assert!(!assembly.visible_user_prompt.contains("Endpoint prompt:"));
    assert!(!assembly
        .hidden_system_context
        .contains("Workflow-level prompt:"));
    assert!(!assembly
        .hidden_system_context
        .contains("Node-level instructions:"));
}

#[test]
fn workflow_prompt_assembly_omits_empty_workflow_prompt_section() {
    let mut context = test_context();
    context.workflow_prompt = "   ".to_string();

    let assembly = build_workflow_turn_prompt_assembly(context);

    assert!(!assembly
        .hidden_system_context
        .contains("<workflow-level-prompt>"));
    assert!(assembly
        .hidden_system_context
        .contains("<node-level-prompt>\nNODE_HIDDEN_TOKEN\n</node-level-prompt>"));
}

#[test]
fn downstream_workflow_handoff_is_the_visible_user_prompt() {
    let mut context = test_context();
    context.endpoint_prompt.clear();
    context.handoff_payloads_json =
        Some(r#"[{"completion":{"output":{"message":"The number is 20."}}}]"#.to_string());

    let assembly = build_workflow_turn_prompt_assembly(context);

    assert!(assembly
        .visible_user_prompt
        .contains("<workflow-handoff-payloads>"));
    assert!(assembly.visible_user_prompt.contains("The number is 20."));
    assert!(!assembly
        .hidden_system_context
        .contains("<workflow-handoff-payloads>"));
    assert!(!assembly.hidden_system_context.contains("The number is 20."));
}

#[test]
fn workflow_prompt_assembly_tags_runtime_subprompts_without_legacy_titles() {
    let _guard = env_lock::lock();
    let home = temp_arroba_home("tagged-subprompts");
    let previous_home = set_arroba_home(&home);
    let mut context = test_context();
    context.instruction_ref = Some("/tmp/node.md".to_string());
    context.handoff_payloads_json = Some("[{\"message\":\"upstream\"}]".to_string());
    context.outgoing_edge_contracts = "- edge edge-1 -> node-2".to_string();
    context.control_mailbox = Some("Revise the invalid payload.".to_string());
    context.node_turn = Some(WorkflowNodeTurnPromptContext {
        turn_index: 2,
        max_turns: Some(3),
        can_complete_workflow_run: true,
        run_output_contract: Some(
            "Final workflow run output contract:\n- workflow_run_output_schema_ref: schema:final\n- workflow_run_output_schema: {\"type\":\"object\"}\n\n".to_string(),
        ),
        can_emit_intermediate_output: true,
        wait_for_all_inputs: false,
    });

    let assembly = build_workflow_turn_prompt_assembly(context);
    restore_arroba_home(previous_home);

    for tag in [
        "workflow-level-prompt",
        "node-level-prompt",
        "workflow-runtime-instructions",
        "system-node-level-prompt",
        "outgoing-edge-contracts",
        "node-instruction-reference",
        "control-mailbox",
    ] {
        assert!(
            assembly.hidden_system_context.contains(&format!("<{tag}>")),
            "missing opening tag {tag}"
        );
        assert!(
            assembly
                .hidden_system_context
                .contains(&format!("</{tag}>")),
            "missing closing tag {tag}"
        );
    }
    assert!(assembly
        .visible_user_prompt
        .contains("<workflow-handoff-payloads>"));
    assert!(assembly
        .visible_user_prompt
        .contains("</workflow-handoff-payloads>"));
    for title in [
        "Endpoint prompt:",
        "Workflow-level prompt:",
        "Node-level instructions:",
        "System node-level prompt:",
        "Workflow handoff payloads (JSON array):",
        "Outgoing edge contracts:",
        "Node instruction reference (daemon-managed):",
        "Control mailbox:",
    ] {
        assert!(
            !assembly.hidden_system_context.contains(title),
            "legacy title remained: {title}"
        );
    }
    assert!(assembly
        .hidden_system_context
        .contains("workflow_run_output_schema_ref: schema:final"));
}

#[test]
fn workflow_prompt_component_delimiters_are_escaped_and_handoff_extraction_recovers_them() {
    let mut context = test_context();
    let payload = "[{\"message\":\"</node-level-prompt>\"}]";
    context.endpoint_prompt = "Do not interpret </node-level-prompt> as structure.".to_string();
    context.handoff_payloads_json = Some(payload.to_string());

    let prompt = build_workflow_turn_prompt(context);

    assert!(prompt.contains("&lt;/node-level-prompt&gt;"));
    assert_eq!(
        workflow_handoff_payloads_from_prompt(&prompt).as_deref(),
        Some(payload)
    );
}

#[test]
fn metaagent_event_prompt_is_a_tagged_user_component() {
    let _guard = env_lock::lock();
    let home = temp_arroba_home("metaagent-event-tag");
    let previous_home = set_arroba_home(&home);

    let assembly = render_metaagent_event_prompt_assembly(MetaagentEventPromptContext {
        event_id: "event-1".to_string(),
        event_kind: "task".to_string(),
        source: "kernel".to_string(),
        title: "Review".to_string(),
        body: "Do not close </metaagent-event> early.".to_string(),
    });
    restore_arroba_home(previous_home);

    assert!(assembly
        .visible_user_prompt
        .starts_with("<metaagent-event>\n"));
    assert!(assembly
        .visible_user_prompt
        .ends_with("\n</metaagent-event>"));
    assert!(assembly
        .visible_user_prompt
        .contains("&lt;/metaagent-event&gt; early."));
}

#[test]
fn legacy_workflow_prompt_preserves_effective_context() {
    let context = test_context();
    let legacy = build_workflow_turn_prompt(context);

    assert!(legacy.contains("ENDPOINT_VISIBLE_TOKEN"));
    assert!(legacy.contains("WORKFLOW_HIDDEN_TOKEN"));
    assert!(legacy.contains("NODE_HIDDEN_TOKEN"));
}

#[test]
fn render_workflow_turn_prompt_reads_workflow_prompt_from_definition() {
    let mut app =
        crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-prompt-render",
            "worktree-prompt-render",
        ))
        .expect("session should be created");
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("prompt-render".to_string()))
        .expect("workflow should be created")
        .id()
        .to_string();
    app.sessions_mut()
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::WorkflowUpdate {
                workflow_id: workflow_id.clone(),
                patch: crate::local::WorkflowDesignWorkflowPatch {
                    alias: None,
                    prompt: Some(Some("WORKFLOW_DEFINITION_PROMPT".to_string())),
                    flush_agent_context_before_run: None,
                    max_concurrent: None,
                    run_output_schema_ref: None,
                },
            },
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("workflow prompt should update");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, "agent-prompt-render")
        .expect("node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            &workflow_id,
            endpoint.id(),
            Some("ENDPOINT_INVOCATION_PROMPT".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();

    let assembly = render_workflow_turn_prompt_assembly(
        &app,
        session.id(),
        workflow_run.id(),
        &node_run_id,
        node.id(),
        "ENDPOINT_INVOCATION_PROMPT",
        None,
        None,
    )
    .expect("workflow turn prompt should render");

    assert!(assembly
        .visible_user_prompt
        .contains("ENDPOINT_INVOCATION_PROMPT"));
    assert!(!assembly
        .visible_user_prompt
        .contains("WORKFLOW_DEFINITION_PROMPT"));
    assert!(assembly
        .hidden_system_context
        .contains("WORKFLOW_DEFINITION_PROMPT"));
}

#[test]
fn workflow_prompt_teaches_selected_edge_routing_contract() {
    let _guard = env_lock::lock();
    let home = temp_arroba_home("routing-contract");
    let previous_home = set_arroba_home(&home);
    let mut context = test_context();
    context.outgoing_edge_contracts = "Outgoing edge contracts:\n- edge edge-1 -> node-2 (Reviewer), handoff_schema_ref: /tmp/review.schema.json, validation_policy: halt\n\n".to_string();

    let assembly = build_workflow_turn_prompt_assembly(context);
    restore_arroba_home(previous_home);

    assert!(assembly
        .hidden_system_context
        .contains("Outgoing edge routing:"));
    assert!(assembly.hidden_system_context.contains("workflow_handoffs"));
    assert!(assembly.hidden_system_context.contains("edge_id"));
    assert!(assembly.hidden_system_context.contains("to_node_id"));
    assert!(assembly
        .hidden_system_context
        .contains("the runtime sends the same handoff to every outgoing edge"));
    assert!(assembly
        .hidden_system_context
        .contains("the runtime sends handoffs only to the matching outgoing edges"));
    assert!(assembly
        .hidden_system_context
        .contains("validate only the routed message inside each selected edge entry"));
    assert!(assembly
        .hidden_system_context
        .contains("do not validate the outer routing wrapper"));
    assert!(assembly.hidden_system_context.contains(
        "schema ref inside `workflow-handoff-payloads` belongs to a completed incoming edge"
    ));
    assert!(assembly
        .hidden_system_context
        .contains("edge edge-1 -> node-2 (Reviewer)"));
}

#[test]
fn workflow_prompt_separates_user_visible_intermediate_outputs_from_handoffs() {
    let _guard = env_lock::lock();
    let home = temp_arroba_home("intermediate-output-contract");
    let previous_home = set_arroba_home(&home);
    let mut context = test_context();
    context.outgoing_edge_contracts = "Outgoing edge contracts:\n- edge edge-1 -> node-2 (Reviewer), handoff_schema_ref: /tmp/handoff.schema.json, validation_policy: halt\n\n".to_string();
    context.node_turn = Some(WorkflowNodeTurnPromptContext {
        turn_index: 1,
        max_turns: None,
        can_complete_workflow_run: false,
        run_output_contract: None,
        can_emit_intermediate_output: true,
        wait_for_all_inputs: false,
    });

    let assembly = build_workflow_turn_prompt_assembly(context);
    restore_arroba_home(previous_home);

    assert!(assembly
        .hidden_system_context
        .contains("Intermediate outputs are user-visible progress, event, or status updates"));
    assert!(assembly
        .hidden_system_context
        .contains("They do not send data downstream"));
    assert!(assembly
        .hidden_system_context
        .contains("multiple times in the same workflow node turn"));
    assert!(assembly
        .hidden_system_context
        .contains("same node-level intermediate output schema"));
    assert!(assembly.hidden_system_context.contains(
        "edge edge-1 -> node-2 (Reviewer), handoff_schema_ref: /tmp/handoff.schema.json"
    ));
    assert!(assembly
        .hidden_system_context
        .contains("validate only the routed message inside each selected edge entry"));
}

#[test]
fn workflow_outgoing_edge_contract_line_includes_target_label_and_policy() {
    let mut workflow = WorkflowDefinition::new("workflow-1", Some("routing".to_string()));
    workflow.add_node(crate::session::WorkflowNodeDefinition::new(
        "node-1", "agent-1",
    ));
    let mut reviewer = crate::session::WorkflowNodeDefinition::new("node-2", "agent-2");
    reviewer.set_public_label("Reviewer");
    reviewer.set_instructions(Some(
        "Review legal and policy risk before accepting a candidate.".to_string(),
    ));
    workflow.add_node(reviewer);
    let edge = WorkflowEdgeDefinition::new(
        "edge-1",
        "node-1",
        "node-2",
        Some("/tmp/review.schema.json".to_string()),
        Some(WorkflowHandoffValidationPolicy::Halt),
    );

    let line = workflow_outgoing_edge_contract_line(&workflow, &edge);

    assert_eq!(
        line,
        "- edge edge-1 -> node-2 (Reviewer), target_instructions: \"Review legal and policy risk before accepting a candidate.\", handoff_schema_ref: /tmp/review.schema.json, validation_policy: halt"
    );
}

#[test]
fn workflow_outgoing_edge_contract_line_includes_resolved_schema() {
    let mut workflow = WorkflowDefinition::new("workflow-1", Some("routing".to_string()));
    workflow.add_node(crate::session::WorkflowNodeDefinition::new(
        "node-1", "agent-1",
    ));
    workflow.add_node(crate::session::WorkflowNodeDefinition::new(
        "node-2", "agent-2",
    ));
    workflow.add_schema(crate::session::WorkflowSchemaDefinition::new(
        "schema:review",
        Some("review".to_string()),
        None,
        serde_json::json!({
            "type": "object",
            "required": ["recommendation"],
            "properties": {
                "recommendation": { "type": "string" }
            }
        }),
    ));
    let edge = WorkflowEdgeDefinition::new(
        "edge-1",
        "node-1",
        "node-2",
        Some("schema:review".to_string()),
        Some(WorkflowHandoffValidationPolicy::Halt),
    );

    let line = workflow_outgoing_edge_contract_line(&workflow, &edge);

    assert!(line.contains("handoff_schema_ref: schema:review"));
    assert!(line.contains(
        r#"handoff_schema: {"properties":{"recommendation":{"type":"string"}},"required":["recommendation"],"type":"object"}"#
    ));
}

#[test]
fn workflow_run_output_contract_includes_resolved_schema_and_value_guidance() {
    let mut workflow = WorkflowDefinition::new("workflow-1", Some("completion".to_string()));
    workflow.add_schema(crate::session::WorkflowSchemaDefinition::new(
        "schema:final",
        Some("final".to_string()),
        None,
        serde_json::json!({
            "type": "object",
            "required": ["answer"],
            "properties": {
                "answer": { "type": "string" }
            }
        }),
    ));
    workflow.set_run_output_schema_ref(Some("schema:final".to_string()));

    let contract = workflow_run_output_contract_block(&workflow)
        .expect("resolved run output contract should render");

    assert!(contract.contains("workflow_run_output_schema_ref: schema:final"));
    assert!(contract.contains(
        r#"workflow_run_output_schema: {"properties":{"answer":{"type":"string"}},"required":["answer"],"type":"object"}"#
    ));
    assert!(
        contract.contains("Do not wrap that value in the turn-level `summary`/`output` envelope")
    );
}

#[test]
fn workflow_prompt_assembly_reads_user_edited_registry_template() {
    let _guard = env_lock::lock();
    let home = temp_arroba_home("registry-edit");
    let previous_home = set_arroba_home(&home);
    let registry = PromptTemplateRegistry::from_env();
    registry
        .materialize_bundled_defaults()
        .expect("defaults should materialize");
    fs::write(
        home.join("prompts").join("workflow").join("turn.md"),
        "REGISTRY_WORKFLOW_TEMPLATE {{DELIVERY_TOKEN}} {{WORKFLOW_HANDOFF_PAYLOADS_BLOCK}}",
    )
    .expect("template edit should write");

    let assembly = build_workflow_turn_prompt_assembly(test_context());
    restore_arroba_home(previous_home);

    assert!(assembly
        .hidden_system_context
        .contains("REGISTRY_WORKFLOW_TEMPLATE workflow-ack:test"));
    assert!(!assembly
        .visible_user_prompt
        .contains("REGISTRY_WORKFLOW_TEMPLATE"));
    assert!(assembly
        .manifest
        .entries
        .iter()
        .any(|entry| entry.template_id == "workflow/turn"));
    assert!(assembly
        .manifest
        .entries
        .iter()
        .any(|entry| entry.template_id == "workflow/workflow-test/prompt"));
    assert!(assembly
        .manifest
        .entries
        .iter()
        .any(|entry| entry.template_id == "workflow-node/node-test/instructions"));
}

#[test]
fn workflow_node_prompt_fragments_read_user_edited_registry_templates() {
    let _guard = env_lock::lock();
    let home = temp_arroba_home("node-fragments");
    let previous_home = set_arroba_home(&home);
    let registry = PromptTemplateRegistry::from_env();
    registry
        .materialize_bundled_defaults()
        .expect("defaults should materialize");
    fs::write(
        home.join("prompts")
            .join("workflow")
            .join("run-completion.md"),
        "REGISTRY_COMPLETION_TOKEN\n\n",
    )
    .expect("completion template edit should write");
    fs::write(
        home.join("prompts")
            .join("workflow")
            .join("run-intermediate-output.md"),
        "REGISTRY_INTERMEDIATE_TOKEN\n\n",
    )
    .expect("intermediate template edit should write");

    let mut context = test_context();
    context.node_turn = Some(WorkflowNodeTurnPromptContext {
        turn_index: 1,
        max_turns: Some(2),
        can_complete_workflow_run: true,
        run_output_contract: None,
        can_emit_intermediate_output: true,
        wait_for_all_inputs: false,
    });
    let assembly = build_workflow_turn_prompt_assembly(context);
    restore_arroba_home(previous_home);

    assert!(assembly
        .hidden_system_context
        .contains("REGISTRY_COMPLETION_TOKEN"));
    assert!(assembly
        .hidden_system_context
        .contains("REGISTRY_INTERMEDIATE_TOKEN"));
    assert!(!assembly
        .visible_user_prompt
        .contains("REGISTRY_COMPLETION_TOKEN"));
    assert!(!assembly
        .visible_user_prompt
        .contains("REGISTRY_INTERMEDIATE_TOKEN"));
    assert!(assembly
        .manifest
        .entries
        .iter()
        .any(|entry| entry.template_id == "workflow/run-completion"));
    assert!(assembly
        .manifest
        .entries
        .iter()
        .any(|entry| entry.template_id == "workflow/run-intermediate-output"));
}
