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

    assert!(
        assembly
            .visible_user_prompt
            .contains("ENDPOINT_VISIBLE_TOKEN")
    );
    assert!(
        !assembly
            .visible_user_prompt
            .contains("WORKFLOW_HIDDEN_TOKEN")
    );
    assert!(!assembly.visible_user_prompt.contains("NODE_HIDDEN_TOKEN"));
    assert!(
        assembly
            .hidden_system_context
            .contains("WORKFLOW_HIDDEN_TOKEN")
    );
    assert!(assembly.hidden_system_context.contains("NODE_HIDDEN_TOKEN"));
    assert!(
        !assembly
            .hidden_system_context
            .contains("ENDPOINT_VISIBLE_TOKEN")
    );
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

    assert!(
        assembly
            .visible_user_prompt
            .contains("ENDPOINT_INVOCATION_PROMPT")
    );
    assert!(
        !assembly
            .visible_user_prompt
            .contains("WORKFLOW_DEFINITION_PROMPT")
    );
    assert!(
        assembly
            .hidden_system_context
            .contains("WORKFLOW_DEFINITION_PROMPT")
    );
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

    assert!(
        assembly
            .hidden_system_context
            .contains("Outgoing edge routing:")
    );
    assert!(assembly.hidden_system_context.contains("workflow_handoffs"));
    assert!(assembly.hidden_system_context.contains("edge_id"));
    assert!(assembly.hidden_system_context.contains("to_node_id"));
    assert!(
        assembly
            .hidden_system_context
            .contains("the runtime sends the same handoff to every outgoing edge")
    );
    assert!(
        assembly
            .hidden_system_context
            .contains("the runtime sends handoffs only to the matching outgoing edges")
    );
    assert!(
        assembly
            .hidden_system_context
            .contains("validate the routed message for each selected edge")
    );
    assert!(
        assembly
            .hidden_system_context
            .contains("edge edge-1 -> node-2 (Reviewer)")
    );
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
        can_emit_intermediate_output: true,
        wait_for_all_inputs: false,
    });

    let assembly = build_workflow_turn_prompt_assembly(context);
    restore_arroba_home(previous_home);

    assert!(
        assembly
            .hidden_system_context
            .contains("Intermediate outputs are user-visible progress, event, or status updates")
    );
    assert!(
        assembly
            .hidden_system_context
            .contains("They do not send data downstream")
    );
    assert!(
        assembly
            .hidden_system_context
            .contains("multiple times in the same workflow node turn")
    );
    assert!(
        assembly
            .hidden_system_context
            .contains("same node-level intermediate output schema")
    );
    assert!(assembly.hidden_system_context.contains(
        "edge edge-1 -> node-2 (Reviewer), handoff_schema_ref: /tmp/handoff.schema.json"
    ));
    assert!(
        assembly
            .hidden_system_context
            .contains("validate the routed message for each selected edge")
    );
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

    assert!(
        assembly
            .hidden_system_context
            .contains("REGISTRY_WORKFLOW_TEMPLATE workflow-ack:test")
    );
    assert!(
        !assembly
            .visible_user_prompt
            .contains("REGISTRY_WORKFLOW_TEMPLATE")
    );
    assert!(
        assembly
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "workflow/turn")
    );
    assert!(
        assembly
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "workflow/workflow-test/prompt")
    );
    assert!(
        assembly
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "workflow-node/node-test/instructions")
    );
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
        can_emit_intermediate_output: true,
        wait_for_all_inputs: false,
    });
    let assembly = build_workflow_turn_prompt_assembly(context);
    restore_arroba_home(previous_home);

    assert!(
        assembly
            .hidden_system_context
            .contains("REGISTRY_COMPLETION_TOKEN")
    );
    assert!(
        assembly
            .hidden_system_context
            .contains("REGISTRY_INTERMEDIATE_TOKEN")
    );
    assert!(
        !assembly
            .visible_user_prompt
            .contains("REGISTRY_COMPLETION_TOKEN")
    );
    assert!(
        !assembly
            .visible_user_prompt
            .contains("REGISTRY_INTERMEDIATE_TOKEN")
    );
    assert!(
        assembly
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "workflow/run-completion")
    );
    assert!(
        assembly
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "workflow/run-intermediate-output")
    );
}
