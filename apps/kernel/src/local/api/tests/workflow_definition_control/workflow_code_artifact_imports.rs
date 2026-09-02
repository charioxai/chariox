use super::*;

#[test]
fn local_request_api_rejects_workflow_code_artifact_import_with_definition_hash_mismatch() {
    let workspace_root = std::env::temp_dir().join(format!(
        "chariox-workflow-code-hash-mismatch-import-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-mismatch"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let definition = crate::workflow_code::WorkflowCodeDefinition {
        schema_version: crate::workflow_code::WORKFLOW_CODE_SCHEMA_VERSION,
        parameters_schema: None,
        workflow: crate::workflow_code::WorkflowCodeWorkflow {
            alias: Some("mismatch_import".to_string()),
            prompt: None,
            flush_agent_context_before_run: None,
            max_concurrent: None,
            run_output_schema: None,
        },
        schemas: Vec::new(),
        nodes: vec![crate::workflow_code::WorkflowCodeNodeDefinition {
            handle: "worker".to_string(),
            agent: crate::workflow_code::WorkflowCodeAgentBinding::Create(
                crate::workflow_code::WorkflowCodeAgentCreate {
                    alias: Some("worker".to_string()),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                },
            ),
            public_label: None,
            instructions: None,
            can_complete_workflow_run: Some(true),
            can_emit_intermediate_run_output: None,
            wait_for_all_inputs: None,
            intermediate_output_schema: None,
            max_turns: None,
            extensions: Vec::new(),
            canvas: None,
        }],
        edges: Vec::new(),
        endpoints: vec![crate::workflow_code::WorkflowCodeEndpointDefinition {
            handle: "entry".to_string(),
            entry_node: "worker".to_string(),
            alias: Some("entry".to_string()),
            max_instances: None,
            canvas: None,
        }],
        queues: Vec::new(),
        schedules: Vec::new(),
    };
    let source = "workflow.define({ alias: \"mismatch_import\" })";
    let package = crate::workflow_code::WorkflowCodeArtifactPackage {
        package_version: crate::workflow_code::WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION,
        name: "hash-mismatch-import".to_string(),
        language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
        source: source.to_string(),
        source_sha256: workflow_code_test_sha256_hex(source.as_bytes()),
        source_bytes: source.len() as u64,
        definition_sha256: "not-the-definition-hash".to_string(),
        validation: definition
            .validate_with_limits(&crate::config::WorkflowCodeLimitsConfig::default()),
        definition,
        exported_at_ms: crate::session::unix_epoch_ms(),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::ImportWorkflowCodeArtifact(
            crate::local::ImportWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                package,
                name: None,
                overwrite: false,
                node_path: "node".to_string(),
            },
        ))
        .expect_err("definition hash mismatch should not import");

    assert!(format!("{error}").contains("definition sha256 mismatch"));
    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflowCodeArtifacts(
            crate::local::ListWorkflowCodeArtifactsRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("workflow-code artifacts should list");
    match listed {
        LocalDaemonResponse::WorkflowCodeArtifactsListed { artifacts } => {
            assert!(!artifacts
                .iter()
                .any(|artifact| artifact.name == "hash-mismatch-import"));
        }
        _ => panic!("unexpected local response"),
    }

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_creates_typescript_workflow_code_artifact() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code TypeScript artifact test because node is not available");
        return;
    };
    if !node_supports_workflow_code_typescript(&node_path) {
        eprintln!(
            "skipping workflow-code TypeScript artifact test because Node.js cannot strip TypeScript"
        );
        return;
    }

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-workflow-code-ts", "worktree-workflow-code-ts"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let name = format!("ts-flow-{}", crate::session::unix_epoch_ms());
    let source = r#"
type ProviderName = "dev-stub";
const provider: ProviderName = "dev-stub";
const final = workflow.schema({
  handle: "final",
  schema: {
    type: "object",
    required: ["answer"],
    properties: { answer: { type: "string" } },
    additionalProperties: false
  }
})
workflow.define({ alias: "typescript_artifact_flow", runOutputSchema: final })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "ts-worker", provider, model: "default" }),
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;

    let created = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
                language: crate::workflow_code::WorkflowCodeLanguage::TypeScript,
                node_path: node_path.display().to_string(),
                source: source.to_string(),
            },
        ))
        .expect("TypeScript workflow-code artifact should create");

    match created {
        LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact } => {
            assert_eq!(artifact.metadata.name, name);
            assert_eq!(
                artifact.metadata.language,
                crate::workflow_code::WorkflowCodeLanguage::TypeScript
            );
            assert!(artifact.metadata.validation.ok);
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("typescript_artifact_flow")
            );
            assert_eq!(
                artifact.definition.workflow.run_output_schema.as_deref(),
                Some("final")
            );
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_applies_inline_typescript_workflow_code() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping inline workflow-code TypeScript test because node is not available");
        return;
    };
    if !node_supports_workflow_code_typescript(&node_path) {
        eprintln!(
            "skipping inline workflow-code TypeScript test because Node.js cannot strip TypeScript"
        );
        return;
    }

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-inline-workflow-code-ts",
                "worktree-inline-workflow-code-ts",
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source = r#"
type ProviderName = "dev-stub";
const provider: ProviderName = "dev-stub";
workflow.define({ alias: "inline_typescript_flow" });
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "inline-ts-worker", provider, model: "default" }),
  canCompleteWorkflowRun: true
});
workflow.endpoint(worker, { handle: "entry", alias: "entry" });
"#;

    let applied = harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowCode(
            crate::local::ApplyWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: Some(crate::workflow_code::WorkflowCodeLanguage::TypeScript),
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
            },
        ))
        .expect("inline TypeScript workflow-code should apply");

    let LocalDaemonResponse::WorkflowCodeApplied { result, session } = applied else {
        panic!("unexpected local response");
    };
    assert!(result.compile.validation.ok);
    assert_eq!(
        result.compile.definition.workflow.alias.as_deref(),
        Some("inline_typescript_flow")
    );
    assert!(session
        .workflows()
        .iter()
        .any(|workflow| workflow.id() == result.apply.workflow_id));
}
