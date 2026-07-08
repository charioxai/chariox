use super::*;

#[test]
fn metaagent_workflow_code_applies_and_runs_canonical_routing_pattern() {
    run_large_stack_async_test(
        "metaagent-workflow-code-applies-and-runs-canonical-routing-pattern",
        metaagent_workflow_code_applies_and_runs_canonical_routing_pattern_inner,
    );
}

async fn metaagent_workflow_code_applies_and_runs_canonical_routing_pattern_inner() {
    if let Err(error) = crate::workflow_code::discover_workflow_code_node_path() {
        eprintln!(
            "skipping meta workflow-code routing pattern test because Node.js is unavailable: {error}"
        );
        return;
    }

    let env = TestMetaRuntimeEnv::new("workflow-code-routing-pattern");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let routing_example = crate::workflow_code::WORKFLOW_CODE_PATTERN_EXAMPLES
        .iter()
        .find(|example| example.slug == "routing")
        .expect("routing pattern example should be bundled");
    let provider_rebindings = serde_json::json!([
        { "node": "classifier", "provider": "dev-stub", "model": "default" },
        { "node": "code_specialist", "provider": "dev-stub", "model": "default" },
        { "node": "research_specialist", "provider": "dev-stub", "model": "default" }
    ]);

    let created = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_CREATE_TOOL,
            serde_json::json!({
                "name": "routing-pattern",
                "source": routing_example.source
            }),
        )
        .await
        .expect("metaagent should create routing workflow-code artifact");
    assert!(created.ok, "{:?}", created.payload);
    assert_eq!(
        created
            .payload
            .pointer("/WorkflowCodeArtifactCreated/artifact/definition/workflow/alias")
            .and_then(serde_json::Value::as_str),
        Some("pattern-routing")
    );

    let applied = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_APPLY_TOOL,
            serde_json::json!({
                "name": "routing-pattern",
                "provider_rebindings": provider_rebindings
            }),
        )
        .await
        .expect("metaagent should apply routing workflow-code artifact");
    assert!(applied.ok, "{:?}", applied.payload);
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/compile/definition/workflow/alias")
            .and_then(serde_json::Value::as_str),
        Some("pattern-routing")
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/compile/definition/nodes")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(3)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/compile/definition/edges")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/compile/definition/endpoints")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/compile/definition/schemas")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/node_ids")
            .and_then(serde_json::Value::as_object)
            .map(serde_json::Map::len),
        Some(3)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/agent_ids")
            .and_then(serde_json::Value::as_object)
            .map(serde_json::Map::len),
        Some(3)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/edge_ids")
            .and_then(serde_json::Value::as_object)
            .map(serde_json::Map::len),
        Some(2)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/endpoint_ids/entry")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        true
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/schema_refs/route_task")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        true
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/schema_refs/final_output")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        true
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/canvas_layout_applied")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    let workflow_id = applied
        .payload
        .pointer("/WorkflowCodeApplied/result/apply/workflow_id")
        .and_then(serde_json::Value::as_str)
        .expect("apply should return workflow id");
    let generated_agent_ids = applied
        .payload
        .pointer("/WorkflowCodeApplied/result/apply/agent_ids")
        .and_then(serde_json::Value::as_object)
        .expect("apply should return generated agent ids");
    for handle in ["classifier", "code_specialist", "research_specialist"] {
        let agent_id = generated_agent_ids
            .get(handle)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("missing generated agent id for {handle}"));
        assert!(
            applied
                .payload
                .pointer("/WorkflowCodeApplied/session/agents")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|agents| agents.iter().any(|agent| {
                    agent.get("id").and_then(serde_json::Value::as_str) == Some(agent_id)
                        && agent.get("provider").and_then(serde_json::Value::as_str)
                            == Some("dev-stub")
                        && agent.get("model").and_then(serde_json::Value::as_str) == Some("default")
                        && agent
                            .get("controlled_by_metaagent_id")
                            .and_then(serde_json::Value::as_str)
                            == Some(metaagent.id())
                })),
            "generated {handle} agent should be present with rebound provider/model: {:?}",
            applied.payload
        );
    }
    assert!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/session/workflows")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|workflows| workflows.iter().any(|workflow| {
                workflow.get("id").and_then(serde_json::Value::as_str) == Some(workflow_id)
                    && workflow
                        .get("controlled_by_metaagent_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(metaagent.id())
            })),
        "routing workflow should appear in session snapshot as metaagent-controlled"
    );

    let exported = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_EXPORT_TOOL,
            serde_json::json!({ "name": "routing-pattern" }),
        )
        .await
        .expect("metaagent should export routing workflow-code artifact");
    assert!(exported.ok, "{:?}", exported.payload);
    let package = exported
        .payload
        .pointer("/WorkflowCodeArtifactExported/package")
        .cloned()
        .expect("export should return routing package");
    let imported = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_IMPORT_TOOL,
            serde_json::json!({
                "package": package,
                "name": "routing-pattern-imported"
            }),
        )
        .await
        .expect("metaagent should import routing workflow-code package");
    assert!(imported.ok, "{:?}", imported.payload);
    assert_eq!(
        imported
            .payload
            .pointer("/WorkflowCodeArtifactImported/artifact/definition/workflow/alias")
            .and_then(serde_json::Value::as_str),
        Some("pattern-routing")
    );

    let run = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_RUN_TOOL,
            serde_json::json!({
                "name": "routing-pattern-imported",
                "endpoint": "entry",
                "prompt": "Route this request to the code specialist: fix the failing build.",
                "provider_rebindings": [
                    { "node": "classifier", "provider": "dev-stub", "model": "default" },
                    { "node": "code_specialist", "provider": "dev-stub", "model": "default" },
                    { "node": "research_specialist", "provider": "dev-stub", "model": "default" }
                ]
            }),
        )
        .await
        .expect("metaagent should run imported routing workflow-code artifact");
    assert!(run.ok, "{:?}", run.payload);
    assert_eq!(
        run.payload
            .pointer("/WorkflowCodeRun/result/invocation/kind")
            .and_then(serde_json::Value::as_str),
        Some("started")
    );
    assert_eq!(
        run.payload
            .pointer("/WorkflowCodeRun/result/apply/apply/endpoint_ids/entry")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        true
    );
    assert_eq!(
        run.payload
            .pointer("/WorkflowCodeRun/result/invocation/workflow/controlled_by_metaagent_id")
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );
}

#[tokio::test]
async fn metaagent_workflow_code_applies_inline_typescript_source() {
    let node_path = match crate::workflow_code::discover_workflow_code_node_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "skipping meta inline TypeScript workflow-code test because Node.js is unavailable: {error}"
            );
            return;
        }
    };
    if !node_supports_workflow_code_typescript(&node_path) {
        eprintln!(
            "skipping meta inline TypeScript workflow-code test because Node.js cannot strip TypeScript"
        );
        return;
    }

    let env = TestMetaRuntimeEnv::new("workflow-code-inline-typescript");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let source = r#"
type ProviderName = "dev-stub";
const provider: ProviderName = "dev-stub";
workflow.define({ alias: "meta_inline_typescript_flow", maxConcurrent: 2 });
const finalOutput = workflow.schema({
  handle: "final",
  schema: {
    type: "object",
    required: ["answer"],
    properties: { answer: { type: "string" } },
    additionalProperties: false
  }
});
workflow.define({ runOutputSchema: finalOutput });
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "inline-ts-worker", provider, model: "default" }),
  instructions: "Complete with the final schema.",
  canCompleteWorkflowRun: true
});
workflow.endpoint(worker, { handle: "entry", alias: "entry" });
"#;

    let applied = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_APPLY_TOOL,
            serde_json::json!({
                "source": source,
                "language": "typescript"
            }),
        )
        .await
        .expect("metaagent should apply inline TypeScript workflow-code");
    assert!(applied.ok, "{:?}", applied.payload);
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/compile/definition/workflow/alias")
            .and_then(serde_json::Value::as_str),
        Some("meta_inline_typescript_flow")
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/schema_refs/final")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        true
    );
    assert!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/session/workflows")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|workflows| workflows.iter().any(|workflow| {
                workflow.get("alias").and_then(serde_json::Value::as_str)
                    == Some("meta_inline_typescript_flow")
                    && workflow
                        .get("controlled_by_metaagent_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(metaagent.id())
            })),
        "inline TypeScript workflow should appear in the session snapshot"
    );
}

#[tokio::test]
async fn metaagent_workflow_code_applies_canonical_fan_out_pattern() {
    if let Err(error) = crate::workflow_code::discover_workflow_code_node_path() {
        eprintln!(
            "skipping meta workflow-code fan-out pattern test because Node.js is unavailable: {error}"
        );
        return;
    }

    let env = TestMetaRuntimeEnv::new("workflow-code-fan-out-pattern");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let fan_out_example = crate::workflow_code::WORKFLOW_CODE_PATTERN_EXAMPLES
        .iter()
        .find(|example| example.slug == "fan-out-synthesize")
        .expect("fan-out pattern example should be bundled");

    let validated = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_VALIDATE_TOOL,
            serde_json::json!({
                "source": fan_out_example.source,
                "provider_rebindings": [
                    { "node": "planner", "provider": "dev-stub", "model": "default" },
                    { "node": "worker_a", "provider": "dev-stub", "model": "default" },
                    { "node": "worker_b", "provider": "dev-stub", "model": "default" },
                    { "node": "synthesizer", "provider": "dev-stub", "model": "default" }
                ]
            }),
        )
        .await
        .expect("metaagent should validate fan-out workflow-code source");
    assert!(validated.ok, "{:?}", validated.payload);
    assert_eq!(
        validated
            .payload
            .pointer("/WorkflowCodeValidated/result/validation/ok")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let applied = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_APPLY_TOOL,
            serde_json::json!({
                "source": fan_out_example.source,
                "provider_rebindings": [
                    { "node": "planner", "provider": "dev-stub", "model": "default" },
                    { "node": "worker_a", "provider": "dev-stub", "model": "default" },
                    { "node": "worker_b", "provider": "dev-stub", "model": "default" },
                    { "node": "synthesizer", "provider": "dev-stub", "model": "default" }
                ]
            }),
        )
        .await
        .expect("metaagent should apply fan-out workflow-code source");
    assert!(applied.ok, "{:?}", applied.payload);
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/compile/definition/workflow/alias")
            .and_then(serde_json::Value::as_str),
        Some("pattern-fan-out-synthesize")
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/node_ids")
            .and_then(serde_json::Value::as_object)
            .map(serde_json::Map::len),
        Some(4)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/agent_ids")
            .and_then(serde_json::Value::as_object)
            .map(serde_json::Map::len),
        Some(4)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/edge_ids")
            .and_then(serde_json::Value::as_object)
            .map(serde_json::Map::len),
        Some(4)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/schema_refs")
            .and_then(serde_json::Value::as_object)
            .map(serde_json::Map::len),
        Some(3)
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/endpoint_ids/entry")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        true
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/canvas_layout_applied")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    let workflow_id = applied
        .payload
        .pointer("/WorkflowCodeApplied/result/apply/workflow_id")
        .and_then(serde_json::Value::as_str)
        .expect("apply should return workflow id");
    let generated_agent_ids = applied
        .payload
        .pointer("/WorkflowCodeApplied/result/apply/agent_ids")
        .and_then(serde_json::Value::as_object)
        .expect("apply should return generated agent ids");
    for handle in ["planner", "worker_a", "worker_b", "synthesizer"] {
        let agent_id = generated_agent_ids
            .get(handle)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("missing generated agent id for {handle}"));
        assert!(
            applied
                .payload
                .pointer("/WorkflowCodeApplied/session/agents")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|agents| agents.iter().any(|agent| {
                    agent.get("id").and_then(serde_json::Value::as_str) == Some(agent_id)
                        && agent.get("provider").and_then(serde_json::Value::as_str)
                            == Some("dev-stub")
                        && agent.get("model").and_then(serde_json::Value::as_str) == Some("default")
                        && agent
                            .get("controlled_by_metaagent_id")
                            .and_then(serde_json::Value::as_str)
                            == Some(metaagent.id())
                })),
            "generated {handle} agent should be present with rebound provider/model: {:?}",
            applied.payload
        );
    }
    assert!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/session/workflows")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|workflows| workflows.iter().any(|workflow| {
                workflow.get("id").and_then(serde_json::Value::as_str) == Some(workflow_id)
                    && workflow
                        .get("controlled_by_metaagent_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(metaagent.id())
            })),
        "fan-out workflow should appear in session snapshot as metaagent-controlled"
    );
}

#[test]
fn metaagent_workflow_code_validate_rejects_unauthorized_existing_agent_binding() {
    run_large_stack_async_test(
        "metaagent-workflow-code-validate-rejects-unauthorized-existing-agent-binding",
        metaagent_workflow_code_validate_rejects_unauthorized_existing_agent_binding_inner,
    );
}

async fn metaagent_workflow_code_validate_rejects_unauthorized_existing_agent_binding_inner() {
    if let Err(error) = crate::workflow_code::discover_workflow_code_node_path() {
        eprintln!(
            "skipping meta workflow-code existing-agent validation test because Node.js is unavailable: {error}"
        );
        return;
    }

    let env = TestMetaRuntimeEnv::new("workflow-code-existing-agent-auth");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let owned_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("owned-worker")
                .with_owner_user_id(metaagent.owner_user_id()),
        )
        .expect("owned worker should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, owned_worker.id(), metaagent.id());
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("peer-worker")
                .with_owner_user_id("peer-user"),
        )
        .expect("peer worker should spawn");
    let agent_count_before_apply = app.agents().get_session_agents(session.id()).len();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

    let owned_source = workflow_code_existing_agent_source(owned_worker.id());
    let owned_validated = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_VALIDATE_TOOL,
            serde_json::json!({ "source": owned_source }),
        )
        .await
        .expect("metaagent should validate owned existing-agent workflow-code");
    assert!(owned_validated.ok, "{:?}", owned_validated.payload);
    assert_eq!(
        owned_validated
            .payload
            .pointer("/WorkflowCodeValidated/result/validation/ok")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    let owned_applied = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_APPLY_TOOL,
            serde_json::json!({ "source": owned_source }),
        )
        .await
        .expect("metaagent should apply owned existing-agent workflow-code");
    assert!(owned_applied.ok, "{:?}", owned_applied.payload);
    assert_eq!(
        owned_applied
            .payload
            .pointer("/WorkflowCodeApplied/result/apply/agent_ids/worker")
            .and_then(serde_json::Value::as_str),
        Some(owned_worker.id())
    );
    let owned_workflow_id = owned_applied
        .payload
        .pointer("/WorkflowCodeApplied/result/apply/workflow_id")
        .and_then(serde_json::Value::as_str)
        .expect("owned existing-agent apply should return workflow id");
    assert!(
        owned_applied
            .payload
            .pointer("/WorkflowCodeApplied/session/workflows")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|workflows| workflows.iter().any(|workflow| {
                workflow.get("id").and_then(serde_json::Value::as_str) == Some(owned_workflow_id)
                    && workflow
                        .get("controlled_by_metaagent_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(metaagent.id())
                    && workflow
                        .get("nodes")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|nodes| {
                            nodes.iter().any(|node| {
                                node.get("agent_id").and_then(serde_json::Value::as_str)
                                    == Some(owned_worker.id())
                            })
                        })
            })),
        "owned existing-agent workflow should be controlled by the metaagent and use the existing worker: {:?}",
        owned_applied.payload
    );
    {
        let app = router.app.lock().await;
        assert_eq!(
            app.agents().get_session_agents(session.id()).len(),
            agent_count_before_apply,
            "applying an existing-agent workflow-code source should not spawn a new agent"
        );
    }
    let owned_run = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_RUN_TOOL,
            serde_json::json!({
                "source": owned_source,
                "endpoint": "entry",
                "prompt": "Run the existing worker workflow."
            }),
        )
        .await
        .expect("metaagent should run owned existing-agent workflow-code");
    assert!(owned_run.ok, "{:?}", owned_run.payload);
    assert_eq!(
        owned_run
            .payload
            .pointer("/WorkflowCodeRun/result/apply/apply/agent_ids/worker")
            .and_then(serde_json::Value::as_str),
        Some(owned_worker.id())
    );
    assert_eq!(
        owned_run
            .payload
            .pointer("/WorkflowCodeRun/result/invocation/kind")
            .and_then(serde_json::Value::as_str),
        Some("started")
    );
    assert_eq!(
        owned_run
            .payload
            .pointer("/WorkflowCodeRun/result/invocation/workflow/controlled_by_metaagent_id")
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );
    {
        let app = router.app.lock().await;
        assert_eq!(
            app.agents().get_session_agents(session.id()).len(),
            agent_count_before_apply,
            "running an existing-agent workflow-code source should not spawn a generated agent"
        );
        let session_after_run = app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve after existing-agent workflow run");
        assert!(
            session_after_run
                .active_prompt_for_agent(owned_worker.id())
                .is_some(),
            "existing bound worker should receive the workflow run prompt"
        );
    }

    let peer_source = workflow_code_existing_agent_source(peer_worker.id());
    let peer_validated = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_VALIDATE_TOOL,
            serde_json::json!({ "source": peer_source }),
        )
        .await
        .expect("metaagent should receive validation diagnostics for unauthorized existing agent");
    assert!(peer_validated.ok, "{:?}", peer_validated.payload);
    assert_eq!(
        peer_validated
            .payload
            .pointer("/WorkflowCodeValidated/result/validation/ok")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert!(
        peer_validated
            .payload
            .pointer("/WorkflowCodeValidated/result/validation/diagnostics")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| {
                diagnostic.get("code").and_then(serde_json::Value::as_str)
                    == Some("unauthorized_existing_agent_binding")
                    && diagnostic.get("handle").and_then(serde_json::Value::as_str)
                        == Some("worker")
            })),
        "{:?}",
        peer_validated.payload
    );

    let metaagent_source = workflow_code_existing_agent_source(metaagent.id());
    let metaagent_validated = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_VALIDATE_TOOL,
            serde_json::json!({ "source": metaagent_source }),
        )
        .await
        .expect("metaagent should receive validation diagnostics for metaagent node binding");
    assert!(metaagent_validated.ok, "{:?}", metaagent_validated.payload);
    assert_eq!(
        metaagent_validated
            .payload
            .pointer("/WorkflowCodeValidated/result/validation/ok")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert!(
        metaagent_validated
            .payload
            .pointer("/WorkflowCodeValidated/result/validation/diagnostics")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| {
                diagnostic.get("code").and_then(serde_json::Value::as_str)
                    == Some("invalid_existing_agent_binding")
                    && diagnostic.get("handle").and_then(serde_json::Value::as_str)
                        == Some("worker")
            })),
        "{:?}",
        metaagent_validated.payload
    );
}

fn workflow_code_existing_agent_source(agent_id: &str) -> String {
    format!(
        r#"
workflow.define({{ alias: "existing-bind" }})
const worker = workflow.node({{
  handle: "worker",
  agent: workflow.existingAgent("{agent_id}"),
  instructions: "Complete.",
  canCompleteWorkflowRun: true
}})
workflow.endpoint(worker, {{ handle: "entry", alias: "entry" }})
"#
    )
}
