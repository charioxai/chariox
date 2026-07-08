use super::*;

mod compiler;
mod registry;
mod validation;

fn minimal_definition() -> WorkflowCodeDefinition {
    WorkflowCodeDefinition {
        schema_version: WORKFLOW_CODE_SCHEMA_VERSION,
        parameters_schema: None,
        workflow: WorkflowCodeWorkflow {
            alias: Some("toy".to_string()),
            prompt: Some("Run the toy workflow.".to_string()),
            flush_agent_context_before_run: Some(true),
            max_concurrent: Some(32),
            run_output_schema: Some("final".to_string()),
        },
        schemas: vec![WorkflowCodeSchemaDefinition {
            handle: "final".to_string(),
            alias: Some("Final output".to_string()),
            description: None,
            schema: serde_json::json!({
                "type": "object",
                "properties": {"answer": {"type": "string"}},
                "required": ["answer"],
                "additionalProperties": false
            }),
        }],
        nodes: vec![WorkflowCodeNodeDefinition {
            handle: "planner".to_string(),
            agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                alias: Some("planner".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                account_profile: None,
            }),
            public_label: Some("Planner".to_string()),
            instructions: Some("Plan the task.".to_string()),
            can_complete_workflow_run: Some(true),
            can_emit_intermediate_run_output: None,
            wait_for_all_inputs: None,
            intermediate_output_schema: None,
            max_turns: Some(4),
            extensions: Vec::new(),
            canvas: Some(WorkflowCodeCanvasPoint { x: 0, y: 0 }),
        }],
        edges: Vec::new(),
        endpoints: vec![WorkflowCodeEndpointDefinition {
            handle: "entry".to_string(),
            entry_node: "planner".to_string(),
            alias: Some("entry".to_string()),
            canvas: None,
        }],
        queues: vec![WorkflowCodeQueueDefinition {
            handle: "default".to_string(),
            alias: "default".to_string(),
            priority: 0,
            enabled: true,
        }],
        schedules: Vec::new(),
    }
}

fn multi_endpoint_definition() -> WorkflowCodeDefinition {
    let mut definition = minimal_definition();
    definition.endpoints.push(WorkflowCodeEndpointDefinition {
        handle: "review".to_string(),
        entry_node: "planner".to_string(),
        alias: Some("review".to_string()),
        canvas: None,
    });
    definition.queues.push(WorkflowCodeQueueDefinition {
        handle: "urgent".to_string(),
        alias: "urgent".to_string(),
        priority: 10,
        enabled: true,
    });
    definition
}

fn find_node() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("NODE") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/local/bin/node"),
        PathBuf::from("/usr/bin/node"),
    ]);

    candidates.into_iter().find(|candidate| {
        Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn assert_agent_rebinding_error(rebindings: &[WorkflowCodeAgentRebinding], expected: &str) {
    let mut definition = minimal_definition();
    let error = apply_workflow_code_agent_rebindings(&mut definition, rebindings)
        .expect_err("agent rebinding should fail");
    assert!(format!("{error}").contains(expected), "{error}");
}
