use super::*;

struct TestMetaRuntimeEnv {
    root: std::path::PathBuf,
}

impl TestMetaRuntimeEnv {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "arroba-m23-metaagent-runtime-{label}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("test meta runtime root should be created");
        Self { root }
    }
}

impl Drop for TestMetaRuntimeEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn node_supports_workflow_code_typescript(node: &std::path::Path) -> bool {
    std::process::Command::new(node)
        .arg("--no-warnings")
        .arg("--input-type=module")
        .arg("-e")
        .arg(
            "const mod = await import('node:module'); if (typeof mod.stripTypeScriptTypes !== 'function') process.exit(1)",
        )
        .status()
        .is_ok_and(|status| status.success())
}

fn mark_test_agent_controlled_by_metaagent(
    app: &mut DaemonApp,
    agent_id: &str,
    metaagent_id: &str,
) {
    app.agents_mut()
        .set_controlled_by_metaagent_id(agent_id, Some(metaagent_id.to_string()))
        .expect("test agent should exist");
}

fn run_large_stack_async_test<Fut>(name: &str, test: fn() -> Fut)
where
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("test runtime should build")
                .block_on(test());
        })
        .expect("test thread should spawn")
        .join()
        .expect("test thread should not panic");
}

#[tokio::test]
async fn runtime_mcp_advertises_meta_tools_only_to_metaagent_provider_runs() {
    let env = TestMetaRuntimeEnv::new("tool-visibility");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, standard_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let standard_run = launch_test_provider(
        &mut app,
        session.id(),
        standard_agent.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let standard_auth_token = standard_run
        .runtime_mcp_auth_token()
        .expect("standard run should expose runtime MCP auth token")
        .to_string();
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let standard_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&standard_auth_token);
    assert!(
        standard_specs.iter().all(|spec| {
            spec.name != crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL
        }),
        "standard agents must not see metaagent runtime tools"
    );

    let meta_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&meta_auth_token);
    assert!(
        meta_specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL),
        "metaagents should see the metaagent runtime MCP tools"
    );
    assert!(
        meta_specs.iter().any(
            |spec| spec.name == crate::transport::runtime_tools::META_WORKFLOW_CODE_CREATE_TOOL
        ),
        "metaagents should see workflow-code construction tools"
    );
    assert!(
        meta_specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::READ_ARTIFACT_TOOL),
        "metaagents should see read-only workspace context tools"
    );
    assert!(
        meta_specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::SEARCH_RECALL_TOOL),
        "metaagents should see recall tools"
    );
    assert!(
        meta_specs
            .iter()
            .all(|spec| spec.name.starts_with("arroba.meta.")
                || spec.name == crate::transport::runtime_tools::READ_ARTIFACT_TOOL
                || spec.name == crate::transport::runtime_tools::SEARCH_RECALL_TOOL
                || spec.name == crate::transport::runtime_tools::QUERY_RECALL_TOOL),
        "metaagents should only see meta, read-only workspace, and recall tools: {meta_specs:?}"
    );

    let denied_direct_tool = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL,
            serde_json::json!({ "path": "README.md", "content_text": "nope" }),
        )
        .await
        .expect("metaagent direct mutation tools should return structured denials");
    assert!(
        !denied_direct_tool.ok
            && denied_direct_tool
                .payload
                .get("error")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|message| message.contains("not available to metaagents")),
        "{:?}",
        denied_direct_tool.payload
    );

    let denied = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &standard_auth_token,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect_err("standard agents should not be able to guess-call meta tools");
    assert!(
        denied
            .to_string()
            .contains("exactly one active metaagent provider run"),
        "{denied:?}"
    );
}

#[tokio::test]
async fn metaagent_runtime_tools_create_validate_apply_and_delete_workflow_code() {
    if let Err(error) = crate::workflow_code::discover_workflow_code_node_path() {
        eprintln!("skipping meta workflow-code tool test because Node.js is unavailable: {error}");
        return;
    }

    let env = TestMetaRuntimeEnv::new("workflow-code");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    std::fs::create_dir_all(workspace.join("schemas")).expect("schema directory should be created");
    let skill_dir = workspace
        .join(".arroba")
        .join("skills")
        .join("meta-workflow-code-skill");
    std::fs::create_dir_all(&skill_dir).expect("workflow-code test skill dir should be created");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: meta-workflow-code-skill\ndescription: Meta workflow-code test skill.\n---\nUse this skill only in meta workflow-code tests.\n",
    )
    .expect("workflow-code test skill should be written");
    let schema_path = workspace.join("schemas/final.json");
    std::fs::write(
        &schema_path,
        r#"{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}},"additionalProperties":false}"#,
    )
    .expect("schema file should be written");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let source = r#"
workflow.define({ alias: "meta_scripted_flow", maxConcurrent: 2 })
const final = workflow.schemaFromFile({
  handle: "final",
  path: "schemas/final.json",
  alias: "Final output"
})
workflow.define({ runOutputSchema: final })
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "planner", provider: "codex", model: "gpt-5" }),
  instructions: "Plan and complete.",
  canCompleteWorkflowRun: true,
  extensions: [
    { kind: "skill", name: "meta-workflow-code-skill" }
  ],
  canvas: { x: 40, y: 80 }
})
workflow.endpoint(planner, { handle: "entry", alias: "entry" })
"#;

    let created = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_CREATE_TOOL,
            serde_json::json!({
                "name": "meta-flow",
                "source": source
            }),
        )
        .await
        .expect("metaagent should create workflow-code artifact");
    assert!(created.ok, "{:?}", created.payload);
    assert_eq!(
        created
            .payload
            .pointer("/WorkflowCodeArtifactCreated/artifact/metadata/name")
            .and_then(serde_json::Value::as_str),
        Some("meta-flow")
    );
    assert_eq!(
        created
            .payload
            .pointer(
                "/WorkflowCodeArtifactCreated/artifact/metadata/provenance/created_by/metaagent_id"
            )
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );
    assert_eq!(
        created
            .payload
            .pointer("/WorkflowCodeArtifactCreated/artifact/metadata/history/0/action")
            .and_then(serde_json::Value::as_str),
        Some("created")
    );
    assert_eq!(
        created
            .payload
            .pointer(
                "/WorkflowCodeArtifactCreated/artifact/definition/schemas/0/schema/properties/answer/type"
            )
            .and_then(serde_json::Value::as_str),
        Some("string")
    );
    let listed = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_LIST_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect("metaagent should list workflow-code artifacts");
    assert!(listed.ok, "{:?}", listed.payload);
    assert!(
        listed
            .payload
            .pointer("/WorkflowCodeArtifactsListed/artifacts")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|artifacts| artifacts.iter().any(|artifact| {
                artifact.get("name").and_then(serde_json::Value::as_str) == Some("meta-flow")
            })),
        "{:?}",
        listed.payload
    );

    let updated_source = source.replace("meta_scripted_flow", "meta_scripted_flow_updated");
    let updated = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_UPDATE_TOOL,
            serde_json::json!({
                "name": "meta-flow",
                "source": updated_source
            }),
        )
        .await
        .expect("metaagent should update workflow-code artifact");
    assert!(updated.ok, "{:?}", updated.payload);
    assert_eq!(
        updated
            .payload
            .pointer("/WorkflowCodeArtifactUpdated/artifact/definition/workflow/alias")
            .and_then(serde_json::Value::as_str),
        Some("meta_scripted_flow_updated")
    );
    assert_eq!(
        updated
            .payload
            .pointer("/WorkflowCodeArtifactUpdated/artifact/metadata/history/1/action")
            .and_then(serde_json::Value::as_str),
        Some("updated")
    );
    std::fs::remove_file(&schema_path)
        .expect("schema source file should be removable after artifact update");

    let validated = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_VALIDATE_TOOL,
            serde_json::json!({ "name": "meta-flow" }),
        )
        .await
        .expect("metaagent should validate saved workflow-code artifact");
    assert!(validated.ok, "{:?}", validated.payload);
    assert_eq!(
        validated
            .payload
            .pointer("/WorkflowCodeValidated/result/validation/ok")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        validated
            .payload
            .pointer("/WorkflowCodeValidated/result/definition/workflow/run_output_schema")
            .and_then(serde_json::Value::as_str),
        Some("final")
    );

    let applied = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_APPLY_TOOL,
            serde_json::json!({
                "name": "meta-flow",
                "provider_rebindings": [
                    { "node": "planner", "provider": "dev-stub", "model": "default" }
                ]
            }),
        )
        .await
        .expect("metaagent should apply saved workflow-code artifact");
    assert!(applied.ok, "{:?}", applied.payload);
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/compile/definition/nodes/0/agent/provider")
            .and_then(serde_json::Value::as_str),
        Some("dev-stub")
    );
    assert_eq!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/result/compile/definition/workflow/run_output_schema")
            .and_then(serde_json::Value::as_str),
        Some("final")
    );
    let workflow_id = applied
        .payload
        .pointer("/WorkflowCodeApplied/result/apply/workflow_id")
        .and_then(serde_json::Value::as_str)
        .expect("apply should return workflow id");
    let planner_agent_id = applied
        .payload
        .pointer("/WorkflowCodeApplied/result/apply/agent_ids/planner")
        .and_then(serde_json::Value::as_str)
        .expect("apply should return planner agent id");
    assert!(
        applied
            .payload
            .pointer("/WorkflowCodeApplied/session/agents")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|agents| agents.iter().any(|agent| {
                agent.get("id").and_then(serde_json::Value::as_str) == Some(planner_agent_id)
                    && agent.get("provider").and_then(serde_json::Value::as_str) == Some("dev-stub")
                    && agent.get("model").and_then(serde_json::Value::as_str) == Some("default")
                    && agent
                        .get("extension_grants")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|grants| grants.iter().any(|grant| {
                            grant.get("kind").and_then(serde_json::Value::as_str)
                                == Some("skill")
                                && grant.get("name").and_then(serde_json::Value::as_str)
                                    == Some("meta-workflow-code-skill")
                        }))
            })),
        "applied workflow-code should create the planner with the rebound provider/model and skill grant: {:?}",
        applied.payload
    );
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
        "applied workflow should be controlled by the metaagent"
    );

    let run = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_RUN_TOOL,
            serde_json::json!({
                "name": "meta-flow",
                "endpoint": "entry",
                "prompt": "Plan this tiny scripted workflow run.",
                "provider_rebindings": [
                    { "node": "planner", "provider": "dev-stub", "model": "default" }
                ]
            }),
        )
        .await
        .expect("metaagent should apply and run saved workflow-code artifact");
    assert!(run.ok, "{:?}", run.payload);
    assert_eq!(
        run.payload
            .pointer("/WorkflowCodeRun/result/apply/apply/endpoint_ids/entry")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        true
    );
    assert_eq!(
        run.payload
            .pointer("/WorkflowCodeRun/result/invocation/kind")
            .and_then(serde_json::Value::as_str),
        Some("started")
    );
    assert_eq!(
        run.payload
            .pointer("/WorkflowCodeRun/result/invocation/workflow/controlled_by_metaagent_id")
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );
    assert_eq!(
        run.payload
            .pointer("/WorkflowCodeRun/result/invocation/endpoint/alias")
            .and_then(serde_json::Value::as_str),
        Some("entry")
    );
    let run_workflow_id = run
        .payload
        .pointer("/WorkflowCodeRun/result/apply/apply/workflow_id")
        .and_then(serde_json::Value::as_str)
        .expect("run response should include generated workflow id");
    let run_endpoint_id = run
        .payload
        .pointer("/WorkflowCodeRun/result/invocation/endpoint/id")
        .and_then(serde_json::Value::as_str)
        .expect("run response should include invoked endpoint id");
    let run_id = run
        .payload
        .pointer("/WorkflowCodeRun/result/invocation/workflow_run/id")
        .and_then(serde_json::Value::as_str)
        .expect("run response should include workflow run id");
    let durable_events = {
        let app = router.app.lock().await;
        app.durable_state_store()
            .load_events_after(0)
            .expect("durable state events should load")
    };
    let run_event = durable_events
        .iter()
        .find(|event| {
            event.kind == "workflow_code.run"
                && event.subject_id.as_deref() == Some(run_workflow_id)
        })
        .expect("meta workflow-code run should persist a durable workflow-code run event");
    assert_eq!(run_event.payload["session_id"], session.id());
    assert_eq!(
        run_event.payload["caller_user_id"],
        metaagent.owner_user_id()
    );
    assert_eq!(
        run_event.payload["controlled_by_metaagent_id"],
        metaagent.id()
    );
    assert_eq!(run_event.payload["outcome"], "invoked");
    assert_eq!(run_event.payload["workflow_id"], run_workflow_id);
    assert_eq!(run_event.payload["endpoint_id"], run_endpoint_id);
    assert_eq!(run_event.payload["workflow_run_id"], run_id);

    let read = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_READ_TOOL,
            serde_json::json!({ "name": "meta-flow" }),
        )
        .await
        .expect("metaagent should read workflow-code artifact");
    assert!(read.ok, "{:?}", read.payload);
    assert_eq!(
        read.payload
            .pointer("/WorkflowCodeArtifact/artifact/definition/workflow/alias")
            .and_then(serde_json::Value::as_str),
        Some("meta_scripted_flow_updated")
    );
    assert_eq!(
        read.payload
            .pointer("/WorkflowCodeArtifact/artifact/metadata/provenance/updated_by/metaagent_id")
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );
    assert_eq!(
        read.payload
            .pointer("/WorkflowCodeArtifact/artifact/metadata/history/0/action")
            .and_then(serde_json::Value::as_str),
        Some("created")
    );
    assert_eq!(
        read.payload
            .pointer("/WorkflowCodeArtifact/artifact/metadata/history/1/action")
            .and_then(serde_json::Value::as_str),
        Some("updated")
    );
    assert_eq!(
        read.payload
            .pointer("/WorkflowCodeArtifact/artifact/metadata/history/2/action")
            .and_then(serde_json::Value::as_str),
        Some("applied")
    );
    assert_eq!(
        read.payload
            .pointer("/WorkflowCodeArtifact/artifact/metadata/history/3/action")
            .and_then(serde_json::Value::as_str),
        Some("run")
    );
    assert!(
        read.payload
            .pointer("/WorkflowCodeArtifact/artifact/metadata/history/3/workflow_id")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "{:?}",
        read.payload
    );

    let exported = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_EXPORT_TOOL,
            serde_json::json!({ "name": "meta-flow" }),
        )
        .await
        .expect("metaagent should export workflow-code artifact");
    assert!(exported.ok, "{:?}", exported.payload);
    assert_eq!(
        exported
            .payload
            .pointer("/WorkflowCodeArtifactExported/package/name")
            .and_then(serde_json::Value::as_str),
        Some("meta-flow")
    );
    let package = exported
        .payload
        .pointer("/WorkflowCodeArtifactExported/package")
        .cloned()
        .expect("export should return package");

    let imported = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_IMPORT_TOOL,
            serde_json::json!({
                "package": package,
                "name": "meta-flow-imported"
            }),
        )
        .await
        .expect("metaagent should import workflow-code package");
    assert!(imported.ok, "{:?}", imported.payload);
    assert_eq!(
        imported
            .payload
            .pointer("/WorkflowCodeArtifactImported/artifact/metadata/name")
            .and_then(serde_json::Value::as_str),
        Some("meta-flow-imported")
    );
    assert_eq!(
        imported
            .payload
            .pointer("/WorkflowCodeArtifactImported/artifact/definition/workflow/alias")
            .and_then(serde_json::Value::as_str),
        Some("meta_scripted_flow_updated")
    );

    let deleted = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_DELETE_TOOL,
            serde_json::json!({ "name": "meta-flow" }),
        )
        .await
        .expect("metaagent should delete workflow-code artifact");
    assert!(deleted.ok, "{:?}", deleted.payload);
    assert_eq!(
        deleted
            .payload
            .pointer("/WorkflowCodeArtifactDeleted/name")
            .and_then(serde_json::Value::as_str),
        Some("meta-flow")
    );

    let deleted_import = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_DELETE_TOOL,
            serde_json::json!({ "name": "meta-flow-imported" }),
        )
        .await
        .expect("metaagent should delete imported workflow-code artifact");
    assert!(deleted_import.ok, "{:?}", deleted_import.payload);
    assert_eq!(
        deleted_import
            .payload
            .pointer("/WorkflowCodeArtifactDeleted/name")
            .and_then(serde_json::Value::as_str),
        Some("meta-flow-imported")
    );
}

#[tokio::test]
async fn metaagent_workflow_code_applies_and_runs_canonical_routing_pattern() {
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
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
async fn metaagent_workflow_code_validate_rejects_unauthorized_existing_agent_binding() {
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
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
                        .is_some_and(|nodes| nodes.iter().any(|node| {
                            node.get("agent_id").and_then(serde_json::Value::as_str)
                                == Some(owned_worker.id())
                        }))
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

#[tokio::test]
async fn metaagent_trace_subscription_drains_live_worker_output() {
    let env = TestMetaRuntimeEnv::new("trace-subscription");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, worker) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let subscribed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_TRACE_TOOL,
            serde_json::json!({ "agent_ref": worker.id() }),
        )
        .await
        .expect("subscribe_trace should dispatch");
    assert!(subscribed.ok, "{:?}", subscribed.payload);
    let subscription_id = subscribed
        .payload
        .pointer("/subscription/subscription_id")
        .and_then(serde_json::Value::as_str)
        .expect("subscribe_trace should return subscription id")
        .to_string();

    {
        let mut app = app.lock().await;
        app.fan_out_output(
            session.id(),
            worker_run.id(),
            crate::terminal::TerminalOutputKind::ProviderTool,
            None,
            Vec::new(),
            serde_json::json!({
                "tool": "bash",
                "status": "running",
                "input": {"command": "printf trace-visible"}
            })
            .to_string()
            .as_bytes(),
        );
    }

    let polled = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_POLL_TRACE_TOOL,
            serde_json::json!({ "subscription_id": subscription_id, "limit": 10 }),
        )
        .await
        .expect("poll_trace should dispatch");
    assert!(polled.ok, "{:?}", polled.payload);
    assert_eq!(
        polled
            .payload
            .pointer("/items/0/title")
            .and_then(serde_json::Value::as_str),
        Some("bash · RUNNING"),
        "{:?}",
        polled.payload
    );
    assert!(
        polled
            .payload
            .pointer("/items/0/summary")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|summary| summary.contains("printf trace-visible")),
        "{:?}",
        polled.payload
    );

    {
        let mut app = app.lock().await;
        app.fan_out_output(
            session.id(),
            worker_run.id(),
            crate::terminal::TerminalOutputKind::ProviderTool,
            None,
            Vec::new(),
            serde_json::json!({
                "tool": "bash",
                "status": "running",
                "input": {"command": "printf trace-visible"}
            })
            .to_string()
            .as_bytes(),
        );
    }

    let duplicate = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_POLL_TRACE_TOOL,
            serde_json::json!({ "subscription_id": subscription_id, "limit": 10 }),
        )
        .await
        .expect("duplicate poll_trace should dispatch");
    assert!(duplicate.ok, "{:?}", duplicate.payload);
    assert_eq!(
        duplicate
            .payload
            .get("empty")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "compact trace should suppress repeated identical lifecycle records: {:?}",
        duplicate.payload
    );
    assert_eq!(
        duplicate
            .payload
            .get("suppressed_count")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "{:?}",
        duplicate.payload
    );

    let drained = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_POLL_TRACE_TOOL,
            serde_json::json!({ "agent_ref": worker.id() }),
        )
        .await
        .expect("second poll_trace should dispatch");
    assert!(drained.ok, "{:?}", drained.payload);
    assert_eq!(
        drained
            .payload
            .get("empty")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "{:?}",
        drained.payload
    );

    {
        let mut app = app.lock().await;
        for _ in 0..2 {
            app.fan_out_output(
                session.id(),
                worker_run.id(),
                crate::terminal::TerminalOutputKind::PromptEcho,
                None,
                Vec::new(),
                b"worker prompt echo",
            );
        }
        app.fan_out_output(
            session.id(),
            worker_run.id(),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"worker output trace-visible",
        );
    }

    let waited = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_WAIT_TRACE_TOOL,
            serde_json::json!({
                "subscription_id": subscription_id,
                "until": "worker_output",
                "wait_ms": 1000,
                "limit": 10
            }),
        )
        .await
        .expect("wait_trace should dispatch");
    assert!(waited.ok, "{:?}", waited.payload);
    assert_eq!(
        waited
            .payload
            .get("matched")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "{:?}",
        waited.payload
    );
    let items = waited
        .payload
        .get("items")
        .and_then(serde_json::Value::as_array)
        .expect("wait_trace should return items");
    assert_eq!(
        items
            .iter()
            .filter(
                |item| item.get("kind").and_then(serde_json::Value::as_str) == Some("prompt_echo")
            )
            .count(),
        1,
        "{:?}",
        waited.payload
    );
    assert!(
        items.iter().any(|item| {
            item.get("kind").and_then(serde_json::Value::as_str) == Some("provider_output")
                && item
                    .get("summary")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|summary| summary.contains("worker output trace-visible"))
        }),
        "{:?}",
        waited.payload
    );
}

#[tokio::test]
async fn metaagent_wait_trace_wakes_when_worker_output_arrives_after_wait_starts() {
    let env = TestMetaRuntimeEnv::new("trace-wait-notify");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, worker) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let subscribed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_TRACE_TOOL,
            serde_json::json!({ "agent_ref": worker.id() }),
        )
        .await
        .expect("subscribe_trace should dispatch");
    assert!(subscribed.ok, "{:?}", subscribed.payload);
    let subscription_id = subscribed
        .payload
        .pointer("/subscription/subscription_id")
        .and_then(serde_json::Value::as_str)
        .expect("subscribe_trace should return subscription id")
        .to_string();

    let wait_runtime = router.runtime_state.clone();
    let wait_auth_token = meta_auth_token.clone();
    let wait_subscription_id = subscription_id.clone();
    let wait_task = tokio::spawn(async move {
        wait_runtime
            .dispatch_authenticated_runtime_tool_call(
                &wait_auth_token,
                crate::transport::runtime_tools::META_WAIT_TRACE_TOOL,
                serde_json::json!({
                    "subscription_id": wait_subscription_id,
                    "until": "worker_output",
                    "wait_ms": 100,
                    "limit": 10
                }),
            )
            .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    {
        let mut app = app.lock().await;
        app.fan_out_output(
            session.id(),
            worker_run.id(),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"worker output after wait started",
        );
    }

    let waited = tokio::time::timeout(std::time::Duration::from_millis(150), wait_task)
        .await
        .expect("wait_trace should wake promptly from terminal fanout")
        .expect("wait task should join")
        .expect("wait_trace should dispatch");
    assert!(waited.ok, "{:?}", waited.payload);
    assert_eq!(
        waited
            .payload
            .get("matched")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "{:?}",
        waited.payload
    );
    assert!(
        waited
            .payload
            .get("items")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|items| items.iter().any(|item| {
                item.get("kind").and_then(serde_json::Value::as_str) == Some("provider_output")
                    && item
                        .get("summary")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|summary| summary.contains("after wait started"))
            })),
        "{:?}",
        waited.payload
    );
}

#[tokio::test]
async fn remote_runtime_projection_records_metaagent_turn_completion_event() {
    let env = TestMetaRuntimeEnv::new("remote-projection-completion-event");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, worker) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let submitted = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(worker.id()),
            "remote worker prompt",
            Vec::new(),
        )
        .expect("worker prompt should submit");
    assert!(matches!(
        submitted,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    router
        .runtime_state
        .project_relay_remote_runtime_projection(
            session.id(),
            worker.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            vec![crate::transport::relay_peer::RelayProjectedOutputChunk {
                kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                merge_key: Some("assistant-1".to_string()),
                bytes: b"remote output".to_vec(),
            }],
            Vec::new(),
            vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "assistant-msg-1".to_string(),
                completed_at_ms: 1234,
            }],
        )
        .await
        .expect("runtime projection should succeed");

    let events = app.lock().await.metaagent_event_store().list(
        metaagent.id(),
        Some("agent.turn.completed"),
        None,
        10,
    );
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].source_agent_id.as_deref(), Some(worker.id()));
    assert_eq!(
        events[0]
            .detail
            .get("completed_agent_id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
}

#[tokio::test]
async fn metaagent_runtime_mcp_manages_scoped_task_artifacts() {
    let env = TestMetaRuntimeEnv::new("task-artifacts");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, standard_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let standard_run = launch_test_provider(
        &mut app,
        session.id(),
        standard_agent.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let standard_auth_token = standard_run
        .runtime_mcp_auth_token()
        .expect("standard run should expose runtime MCP auth token")
        .to_string();
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let meta_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&meta_auth_token);
    assert!(
        meta_specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::META_READ_TASK_TOOL),
        "metaagents should see task artifact tools"
    );
    assert!(
        !router
            .runtime_state
            .runtime_tool_specs_for_auth_token(&standard_auth_token)
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::META_READ_TASK_TOOL),
        "standard agents must not see task artifact tools"
    );

    let initial = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_READ_TASK_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect("metaagent should read empty task state");
    assert!(initial.ok, "{:?}", initial.payload);
    assert_eq!(
        initial
            .payload
            .pointer("/status")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );

    let task = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_UPDATE_TASK_TOOL,
            serde_json::json!({ "markdown": "# Task\nPlan the work." }),
        )
        .await
        .expect("metaagent should update task");
    assert!(task.ok, "{:?}", task.payload);
    assert_eq!(
        task.payload
            .pointer("/task/status")
            .and_then(serde_json::Value::as_str),
        Some("active")
    );
    assert_eq!(
        task.payload
            .pointer("/task/task_markdown")
            .and_then(serde_json::Value::as_str),
        Some("# Task\nPlan the work.")
    );

    let plan = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_UPDATE_PLAN_TOOL,
            serde_json::json!({ "markdown": "1. Delegate implementation." }),
        )
        .await
        .expect("metaagent should update plan");
    assert!(plan.ok, "{:?}", plan.payload);
    assert_eq!(
        plan.payload
            .pointer("/plan_markdown")
            .and_then(serde_json::Value::as_str),
        Some("1. Delegate implementation.")
    );
    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session.id().to_string(),
    });
    let state_command =
        KernelCommand::from_local_request("meta-task-projection-state", None, None, &state_request);
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "meta task runtime tool updates should publish a complete session projection"
    );
    drop(app_guard);
    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert!(
                session
                    .agents()
                    .iter()
                    .any(|agent| agent.id() == metaagent.id()),
                "projected session should retain agent membership"
            );
            assert_eq!(
                session
                    .metaagent_task(metaagent.id())
                    .map(|task| task.plan_markdown()),
                Some("1. Delegate implementation."),
                "projected session should retain the metaagent task"
            );
        }
        other => panic!("unexpected state response: {other:?}"),
    }

    let blocked = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_MARK_BLOCKED_TOOL,
            serde_json::json!({ "reason": "worker unavailable" }),
        )
        .await
        .expect("metaagent should mark task blocked");
    assert!(blocked.ok, "{:?}", blocked.payload);
    assert_eq!(
        blocked
            .payload
            .pointer("/task/status")
            .and_then(serde_json::Value::as_str),
        Some("blocked")
    );
    assert_eq!(
        blocked
            .payload
            .pointer("/task/blocked_reason")
            .and_then(serde_json::Value::as_str),
        Some("worker unavailable")
    );

    let completed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_COMPLETE_TASK_TOOL,
            serde_json::json!({ "summary": "done" }),
        )
        .await
        .expect("metaagent should complete task");
    assert!(completed.ok, "{:?}", completed.payload);
    assert_eq!(
        completed
            .payload
            .pointer("/task/status")
            .and_then(serde_json::Value::as_str),
        Some("completed")
    );
    assert_eq!(
        completed
            .payload
            .pointer("/task/completion_summary")
            .and_then(serde_json::Value::as_str),
        Some("done")
    );

    let denied = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &standard_auth_token,
            crate::transport::runtime_tools::META_UPDATE_TASK_TOOL,
            serde_json::json!({ "markdown": "not allowed" }),
        )
        .await
        .expect_err("standard agents should not call meta task tools");
    assert!(
        denied
            .to_string()
            .contains("exactly one active metaagent provider run"),
        "{denied:?}"
    );
}

#[tokio::test]
async fn prompt_to_metaagent_creates_task_without_overwriting_active_task() {
    let env = TestMetaRuntimeEnv::new("prompt-task-create");
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let attach = attach_request(session.id(), "client-prompt-task-create");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach-prompt-task-create", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };

    let first_prompt = "figure out the repo and organize the work";
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: first_prompt.to_string(),
        attachments: Vec::new(),
    });
    let first = router
        .dispatch(
            KernelCommand::from_local_request("submit-meta-task", None, None, &submit),
            submit,
        )
        .await
        .expect("metaagent prompt should submit");
    let LocalDaemonResponse::PromptSubmitted { session, .. } = first else {
        panic!("unexpected submit response: {first:?}");
    };
    assert_eq!(
        session
            .metaagent_task(metaagent.id())
            .map(|task| task.task_markdown()),
        Some(first_prompt)
    );

    let followup = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: "also keep the report short".to_string(),
        attachments: Vec::new(),
    });
    let second = router
        .dispatch(
            KernelCommand::from_local_request("submit-meta-task-followup", None, None, &followup),
            followup,
        )
        .await
        .expect("metaagent follow-up prompt should submit");
    let LocalDaemonResponse::PromptSubmitted { session, .. } = second else {
        panic!("unexpected submit response: {second:?}");
    };
    assert_eq!(
        session
            .metaagent_task(metaagent.id())
            .map(|task| task.task_markdown()),
        Some(first_prompt)
    );
}

#[tokio::test]
async fn local_metaagent_task_update_notifies_metaagent() {
    let env = TestMetaRuntimeEnv::new("local-task-update-notify");
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let update =
        LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            task_markdown: Some("# Updated task".to_string()),
            plan_markdown: Some("1. Re-plan.".to_string()),
        });
    let response = router
        .dispatch(
            KernelCommand::from_local_request("update-meta-task", None, None, &update),
            update,
        )
        .await
        .expect("task update should dispatch");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = response else {
        panic!("unexpected task response: {response:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.task_markdown()),
        Some("# Updated task")
    );
    let active = session
        .active_prompt_for_agent(metaagent.id())
        .expect("task update should notify the metaagent");
    assert!(
        active.prompt().contains("edited your task and plan"),
        "{}",
        active.prompt()
    );
    let task_attachments = app
        .lock()
        .await
        .attachments()
        .list_client_attachments(&format!("metaagent:{}:task", metaagent.id()));
    assert_eq!(task_attachments.len(), 1);
    assert_eq!(task_attachments[0].session_id(), session.id());
}

#[test]
fn local_metaagent_task_pause_and_abort_cancel_active_prompt() {
    run_large_stack_async_test(
        "local-metaagent-task-pause-and-abort-cancel-active-prompt",
        local_metaagent_task_pause_and_abort_cancel_active_prompt_inner,
    );
}

async fn local_metaagent_task_pause_and_abort_cancel_active_prompt_inner() {
    let env = TestMetaRuntimeEnv::new("local-task-cancel");
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let update =
        LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            task_markdown: Some("# Active task".to_string()),
            plan_markdown: Some("1. Keep going.".to_string()),
        });
    router
        .dispatch(
            KernelCommand::from_local_request("update-meta-task-before-pause", None, None, &update),
            update,
        )
        .await
        .expect("task update should start notification prompt");

    let pause = LocalDaemonRequest::PauseMetaagentTask(crate::local::PauseMetaagentTaskRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
    });
    let paused = router
        .dispatch(
            KernelCommand::from_local_request("pause-meta-task", None, None, &pause),
            pause,
        )
        .await
        .expect("pause should dispatch");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = paused else {
        panic!("unexpected pause response: {paused:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Paused)
    );
    assert_eq!(
        session
            .active_prompt_for_agent(metaagent.id())
            .map(|prompt| prompt.status()),
        Some(crate::session::PromptStatus::Cancelling)
    );

    let abort = LocalDaemonRequest::AbortMetaagentTask(crate::local::AbortMetaagentTaskRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        reason: Some("user stopped the task".to_string()),
    });
    let aborted = router
        .dispatch(
            KernelCommand::from_local_request("abort-meta-task", None, None, &abort),
            abort,
        )
        .await
        .expect("abort should dispatch");
    let LocalDaemonResponse::MetaagentTaskUpdated { session, task } = aborted else {
        panic!("unexpected abort response: {aborted:?}");
    };
    assert_eq!(
        task.as_ref().map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Aborted)
    );
    assert_eq!(
        session
            .active_prompt_for_agent(metaagent.id())
            .map(|prompt| prompt.status()),
        Some(crate::session::PromptStatus::Cancelling)
    );
}

#[tokio::test]
async fn runtime_mcp_shared_token_with_metaagent_stays_meta_only() {
    let env = TestMetaRuntimeEnv::new("shared-token-tool-visibility");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, standard_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let shared_auth_token = "shared-meta-runtime-token".to_string();
    let standard_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "worker-model",
            )
            .with_agent_id(standard_agent.id())
            .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                "http://127.0.0.1:1",
                shared_auth_token.clone(),
            )),
        )
        .expect("standard provider run should launch");
    app.update_provider_run_projection(standard_run);
    let meta_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "meta-model",
            )
            .with_agent_id(metaagent.id())
            .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                "http://127.0.0.1:1",
                shared_auth_token.clone(),
            )),
        )
        .expect("meta provider run should launch");
    app.update_provider_run_projection(meta_run);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&shared_auth_token);
    assert!(
        specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL),
        "shared token with a metaagent run should expose metaagent tools"
    );
    assert!(
        specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::READ_ARTIFACT_TOOL),
        "shared token with a metaagent run should expose read-only context tools"
    );
    assert!(
        specs
            .iter()
            .all(|spec| spec.name.starts_with("arroba.meta.")
                || spec.name == crate::transport::runtime_tools::READ_ARTIFACT_TOOL
                || spec.name == crate::transport::runtime_tools::SEARCH_RECALL_TOOL
                || spec.name == crate::transport::runtime_tools::QUERY_RECALL_TOOL),
        "shared token with a metaagent run should expose only meta, read-only workspace, and recall tools: {specs:?}"
    );

    let overview = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &shared_auth_token,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect("shared meta token should dispatch meta tools");
    assert!(overview.ok, "{:?}", overview.payload);
    assert_eq!(
        overview
            .payload
            .get("metaagent")
            .and_then(|value| value.get("id"))
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );

    let denied_direct_tool = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &shared_auth_token,
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL,
            serde_json::json!({ "path": "README.md", "content_text": "nope" }),
        )
        .await
        .expect("shared meta token mutation tools should return structured denials");
    assert!(!denied_direct_tool.ok, "{:?}", denied_direct_tool.payload);
}

#[tokio::test]
async fn forwarded_remote_metaagent_runtime_tools_use_home_scope() {
    let env = TestMetaRuntimeEnv::new("forwarded-remote");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("remote-meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let metaagent = app
        .agents()
        .bind_remote_execution(
            metaagent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("worker-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("metaagent should be remote-backed");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: session.id().to_string(),
        home_agent_id: metaagent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        worker_provider_run_id: "worker-run-1".to_string(),
        worker_worktree_path: workspace.to_string_lossy().to_string(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(
            workspace.to_string_lossy().to_string(),
        ),
    };

    let overview = router
        .dispatch_forwarded_meta_runtime_tool_call(
            context.clone(),
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL.to_string(),
            serde_json::json!({}),
        )
        .await
        .expect("forwarded overview should dispatch home-side");
    assert!(overview.ok, "{overview:?}");
    assert_eq!(
        overview
            .payload
            .pointer("/metaagent/id")
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );

    let command = router
        .dispatch_forwarded_meta_runtime_tool_call(
            context,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL.to_string(),
            serde_json::json!({ "command": "agent list" }),
        )
        .await
        .expect("forwarded run_command should dispatch through the router");
    assert!(command.ok, "{command:?}");
}

#[tokio::test]
async fn forwarded_remote_metaagent_runtime_tools_reject_forged_worker_context() {
    let env = TestMetaRuntimeEnv::new("forwarded-remote-forgery");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("remote-meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let metaagent = app
        .agents()
        .bind_remote_execution(
            metaagent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("worker-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("metaagent should be remote-backed");
    let regular_remote = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("remote-worker"))
        .expect("regular remote agent should spawn");
    let regular_remote = app
        .agents()
        .bind_remote_execution(
            regular_remote.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-2".to_string(),
                leased_agent_id: "leased-agent-2".to_string(),
                active_worker_provider_run_id: Some("worker-run-2".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("regular agent should be remote-backed");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: session.id().to_string(),
        home_agent_id: metaagent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        worker_provider_run_id: "worker-run-1".to_string(),
        worker_worktree_path: workspace.to_string_lossy().to_string(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(
            workspace.to_string_lossy().to_string(),
        ),
    };

    let mut wrong_lease = context.clone();
    wrong_lease.leased_agent_id = "leased-agent-forged".to_string();
    let lease_denied = router
        .dispatch_forwarded_meta_runtime_tool_call(
            wrong_lease,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL.to_string(),
            serde_json::json!({}),
        )
        .await
        .expect_err("mismatched leased agent should be rejected home-side");
    assert!(
        lease_denied
            .to_string()
            .contains("forwarded metaagent context does not match"),
        "{lease_denied:?}"
    );

    let mut wrong_worker = context.clone();
    wrong_worker.worker_kernel_id = "worker-kernel-forged".to_string();
    let worker_denied = router
        .dispatch_forwarded_meta_runtime_tool_call(
            wrong_worker,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL.to_string(),
            serde_json::json!({ "command": "agent list" }),
        )
        .await
        .expect_err("mismatched worker kernel should be rejected home-side");
    assert!(
        worker_denied
            .to_string()
            .contains("forwarded metaagent context does not match"),
        "{worker_denied:?}"
    );

    let mut regular_context = context;
    regular_context.home_agent_id = regular_remote.id().to_string();
    regular_context.leased_agent_id = "leased-agent-2".to_string();
    regular_context.worker_provider_run_id = "worker-run-2".to_string();
    let regular_denied = router
        .dispatch_forwarded_meta_runtime_tool_call(
            regular_context,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL.to_string(),
            serde_json::json!({}),
        )
        .await
        .expect_err("regular remote agents should not get forwarded meta tools");
    assert!(
        regular_denied
            .to_string()
            .contains("only available to session metaagents"),
        "{regular_denied:?}"
    );
}

#[tokio::test]
async fn metaagent_runtime_mcp_returns_session_overview_and_command_docs() {
    let env = TestMetaRuntimeEnv::new("overview");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("peer-worker")
                .with_owner_user_id("user-2"),
        )
        .expect("peer worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let owned_interaction = RuntimeInteraction::new(
        "overview-owned-interaction",
        worker.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow owned command?".to_string()),
        "Allow owned command?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _owned_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), owned_interaction)
        .await
        .expect("owned interaction should register");
    let peer_interaction = RuntimeInteraction::new(
        "overview-peer-interaction",
        peer_worker.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow peer command?".to_string()),
        "Allow peer command?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _peer_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), peer_interaction)
        .await
        .expect("peer interaction should register");

    let overview = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            "arroba_meta_session_overview",
            serde_json::json!({
                "include_workflows": false,
                "include_events": true
            }),
        )
        .await
        .expect("meta session overview should dispatch");
    assert!(overview.ok);
    assert_eq!(
        overview
            .payload
            .pointer("/metaagent/id")
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );
    assert_eq!(
        overview
            .payload
            .pointer("/agents/owned_total")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        overview
            .payload
            .pointer("/agents/total")
            .and_then(serde_json::Value::as_u64),
        Some(4)
    );
    let owned_agents = overview
        .payload
        .pointer("/agents/owned")
        .and_then(serde_json::Value::as_array)
        .expect("owned agents should be included");
    assert!(owned_agents
        .iter()
        .any(|agent| { agent.get("id").and_then(serde_json::Value::as_str) == Some(worker.id()) }));
    assert_eq!(
        overview.payload.get("workflows"),
        Some(&serde_json::Value::Null)
    );
    let pending_interactions = overview
        .payload
        .get("pending_interactions")
        .and_then(serde_json::Value::as_array)
        .expect("pending interactions should be included");
    assert_eq!(pending_interactions.len(), 1);
    assert_eq!(
        pending_interactions
            .first()
            .and_then(|interaction| interaction.get("id"))
            .and_then(serde_json::Value::as_str),
        Some("overview-owned-interaction")
    );

    let search = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SEARCH_COMMANDS_TOOL,
            serde_json::json!({
                "query": "workflow",
                "mutates": true
            }),
        )
        .await
        .expect("meta command search should dispatch");
    assert!(search.ok);
    let commands = search
        .payload
        .get("commands")
        .and_then(serde_json::Value::as_array)
        .expect("commands should be returned");
    assert!(commands.iter().any(|command| {
        command
            .get("name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|name| name.starts_with("workflow "))
    }));

    let listed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_COMMANDS_TOOL,
            serde_json::json!({
                "tag": "agent",
                "scope": "session",
                "policy": "allow",
                "limit": 20
            }),
        )
        .await
        .expect("meta command list should dispatch");
    assert!(listed.ok);
    let listed_commands = listed
        .payload
        .get("commands")
        .and_then(serde_json::Value::as_array)
        .expect("listed commands should be returned");
    assert!(listed_commands.iter().any(|command| {
        command.get("name").and_then(serde_json::Value::as_str) == Some("agent spawn")
    }));
    assert!(listed_commands.iter().all(|command| {
        command
            .get("tags")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tags| tags.iter().any(|tag| tag.as_str() == Some("agent")))
            && command.get("scope").and_then(serde_json::Value::as_str) == Some("session")
            && command
                .get("metaagent_policy")
                .and_then(serde_json::Value::as_str)
                == Some("allow")
    }));

    let docs = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_COMMAND_DOCS_TOOL,
            serde_json::json!({
                "command": "session create"
            }),
        )
        .await
        .expect("meta command docs should dispatch");
    assert!(docs.ok);
    assert_eq!(
        docs.payload
            .get("metaagent_policy")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );

    let guide_search = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SEARCH_GUIDES_TOOL,
            serde_json::json!({
                "query": "create endpoint run workflow",
                "tag": "workflow",
                "limit": 5
            }),
        )
        .await
        .expect("meta guide search should dispatch");
    assert!(guide_search.ok);
    let guides = guide_search
        .payload
        .get("guides")
        .and_then(serde_json::Value::as_array)
        .expect("guide search should return guides");
    assert!(guides.iter().any(|guide| {
        guide.get("id").and_then(serde_json::Value::as_str) == Some("workflows/basic-components")
    }));
    assert!(guides.iter().all(|guide| guide.get("body").is_none()));

    let listed_guides = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_GUIDES_TOOL,
            serde_json::json!({
                "command": "workflow run",
                "limit": 10
            }),
        )
        .await
        .expect("meta guide list should dispatch");
    assert!(listed_guides.ok);
    assert!(listed_guides
        .payload
        .get("guides")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|guides| guides.iter().any(|guide| {
            guide
                .get("commands")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|commands| {
                    commands
                        .iter()
                        .any(|command| command.as_str() == Some("workflow run"))
                })
        })));

    let guide = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_READ_GUIDE_TOOL,
            serde_json::json!({
                "guide": "agent-apps/generate-app"
            }),
        )
        .await
        .expect("meta guide read should dispatch");
    assert!(guide.ok);
    assert!(guide
        .payload
        .get("body")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|body| body.contains("Do not implement directly")));
}

#[test]
fn metaagent_run_command_submits_prompts_through_router_path() {
    run_large_stack_async_test(
        "metaagent-run-command-prompt",
        metaagent_run_command_submits_prompts_through_router_path_inner,
    );
}

async fn metaagent_run_command_submits_prompts_through_router_path_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-prompt");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_controlled_by_metaagent_id(metaagent.id()),
        )
        .expect("worker should spawn");
    let _worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let result = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "prompt worker \"please inspect the failing test\""
            }),
        )
        .await
        .expect("meta run_command should dispatch through the router");

    assert!(result.ok, "{:?}", result.payload);
    assert_eq!(
        result
            .payload
            .get("command")
            .and_then(serde_json::Value::as_str),
        Some("prompt worker \"please inspect the failing test\"")
    );
    assert!(
        result.payload.get("outcome").is_some(),
        "compact prompt outcome should be included"
    );
    assert_eq!(
        result
            .payload
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("submitted")
    );
    let steered = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "prompt worker \"add this to the active investigation\""
            }),
        )
        .await
        .expect("meta run_command should steer active worker prompts");
    assert!(steered.ok, "{:?}", steered.payload);
    assert_eq!(
        steered
            .payload
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("steered")
    );
    let worker_queued_prompts = app
        .lock()
        .await
        .sessions()
        .get_session(session.id())
        .expect("session should load")
        .queued_prompts_for_agent(worker.id())
        .map(|queued| queued.len())
        .unwrap_or_default();
    assert_eq!(
        worker_queued_prompts, 0,
        "metaagent prompt commands should steer active local agents instead of queueing"
    );
    let attachments = app
        .lock()
        .await
        .attachments()
        .list_client_attachments(&format!("metaagent:{}:commands", metaagent.id()));
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].session_id(), session.id());
    let audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    let command_audit = audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.command.executed"
                && event.payload["metaagent_id"] == metaagent.id()
                && event.payload["command"] == "prompt worker \"please inspect the failing test\""
                && event.payload["status"] == "succeeded"
        })
        .expect("metaagent command audit should include durable provenance");
    assert_eq!(command_audit.payload["provider_run_id"], meta_run.id());
    assert_eq!(command_audit.payload["causation_id"], meta_run.id());
    let command_correlation_id = command_audit
        .payload
        .get("correlation_id")
        .and_then(serde_json::Value::as_str)
        .expect("command audit should include a correlation id");
    assert!(
        command_correlation_id.starts_with(&format!("metaagent:{}:command:", metaagent.id())),
        "{command_correlation_id}"
    );
    let prompt_audit = audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.prompt.submitted"
                && event.payload["metaagent_id"] == metaagent.id()
                && event.payload["target_agent_id"] == worker.id()
                && event.payload["status"] == "steered"
        })
        .expect("metaagent prompt audit should include durable provenance");
    let prompt_id = prompt_audit
        .payload
        .get("prompt_id")
        .and_then(serde_json::Value::as_str)
        .expect("prompt audit should include a prompt id");
    assert_eq!(prompt_audit.payload["causation_id"], prompt_id);
    assert_eq!(
        prompt_audit.payload["correlation_id"],
        format!("metaagent:{}:prompt:{prompt_id}", metaagent.id())
    );
}

#[test]
fn multiple_metaagents_in_one_session_are_isolated() {
    run_large_stack_async_test(
        "multiple-metaagents-isolated",
        multiple_metaagents_in_one_session_are_isolated_inner,
    );
}

async fn multiple_metaagents_in_one_session_are_isolated_inner() {
    let env = TestMetaRuntimeEnv::new("multi-metaagent-isolation");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let meta_a = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta-a")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("first metaagent should spawn");
    let meta_b = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta-b")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("second metaagent should spawn");
    let meta_a_run = launch_test_provider(
        &mut app,
        session.id(),
        meta_a.id(),
        "dev-stub",
        "dev-stub",
        "meta-a-model",
    );
    let meta_b_run = launch_test_provider(
        &mut app,
        session.id(),
        meta_b.id(),
        "dev-stub",
        "dev-stub",
        "meta-b-model",
    );
    let meta_a_auth = meta_a_run
        .runtime_mcp_auth_token()
        .expect("meta A run should expose runtime MCP auth token")
        .to_string();
    let meta_b_auth = meta_b_run
        .runtime_mcp_auth_token()
        .expect("meta B run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let spawn_a = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent spawn alpha" }),
        )
        .await
        .expect("meta A spawn command should dispatch");
    assert!(spawn_a.ok, "{:?}", spawn_a.payload);
    let alpha_id = spawn_a
        .payload
        .pointer("/response/agent/id")
        .and_then(serde_json::Value::as_str)
        .expect("spawn A response should include agent id")
        .to_string();

    let spawn_b = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_b_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent spawn beta" }),
        )
        .await
        .expect("meta B spawn command should dispatch");
    assert!(spawn_b.ok, "{:?}", spawn_b.payload);
    let beta_id = spawn_b
        .payload
        .pointer("/response/agent/id")
        .and_then(serde_json::Value::as_str)
        .expect("spawn B response should include agent id")
        .to_string();

    {
        let app = app.lock().await;
        let alpha = app
            .agents()
            .get_agent(&alpha_id)
            .expect("alpha worker should exist");
        let beta = app
            .agents()
            .get_agent(&beta_id)
            .expect("beta worker should exist");
        assert_eq!(alpha.controlled_by_metaagent_id(), Some(meta_a.id()));
        assert_eq!(beta.controlled_by_metaagent_id(), Some(meta_b.id()));
    }

    let list_a = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent list" }),
        )
        .await
        .expect("meta A list command should dispatch");
    assert!(list_a.ok, "{:?}", list_a.payload);
    let listed_agents = list_a
        .payload
        .pointer("/response/agents")
        .and_then(serde_json::Value::as_array)
        .expect("agent list should include agents");
    assert_eq!(listed_agents.len(), 1);
    assert_eq!(
        listed_agents[0]
            .get("alias")
            .and_then(serde_json::Value::as_str),
        Some("alpha")
    );

    let prompt_cross = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "prompt beta \"do not allow this\"" }),
        )
        .await
        .expect("cross prompt command should dispatch as a rejected tool result");
    assert!(!prompt_cross.ok, "{:?}", prompt_cross.payload);
    assert!(
        prompt_cross
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        prompt_cross.payload
    );

    let create_workflow = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow new flow-a" }),
        )
        .await
        .expect("meta A workflow create should dispatch");
    assert!(create_workflow.ok, "{:?}", create_workflow.payload);

    let add_cross_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow node add flow-a beta" }),
        )
        .await
        .expect("cross workflow node command should dispatch as a rejected tool result");
    assert!(!add_cross_node.ok, "{:?}", add_cross_node.payload);

    let resolve_cross_workflow = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_b_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow resolve flow-a" }),
        )
        .await
        .expect("cross workflow resolve should dispatch as a rejected tool result");
    assert!(
        !resolve_cross_workflow.ok,
        "{:?}",
        resolve_cross_workflow.payload
    );
    assert!(
        resolve_cross_workflow
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not controlled by metaagent")),
        "{:?}",
        resolve_cross_workflow.payload
    );
}

#[test]
fn metaagent_prompt_command_does_not_steer_active_workflow_turns() {
    run_large_stack_async_test(
        "metaagent-prompt-active-workflow-guard",
        metaagent_prompt_command_does_not_steer_active_workflow_turns_inner,
    );
}

async fn metaagent_prompt_command_does_not_steer_active_workflow_turns_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-workflow-prompt-guard");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_controlled_by_metaagent_id(metaagent.id()),
        )
        .expect("worker should spawn");
    let _worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("guarded-flow".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), worker.id())
        .expect("workflow node should be created");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("start".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run guarded workflow".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow node prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node run should start");
    let workflow_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        worker.id(),
        "workflow node prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    app.prompt_owner_submit_prepared_prompt(session.id(), workflow_prompt, false)
        .expect("workflow prompt should become active");

    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let result = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "prompt worker \"finish the active workflow turn\""
            }),
        )
        .await
        .expect("meta run_command should return a structured failure");

    assert!(!result.ok, "{:?}", result.payload);
    assert!(
        result
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| {
                message.contains("currently executing workflow run")
                    && message.contains("normal metaagent prompts cannot steer")
            }),
        "{:?}",
        result.payload
    );
    let session_state = app
        .lock()
        .await
        .sessions()
        .get_session(session.id())
        .expect("session should load");
    let active_prompt = session_state
        .active_prompt_for_agent(worker.id())
        .expect("workflow prompt should remain active");
    assert_eq!(active_prompt.workflow_run_id(), Some(workflow_run.id()));
    assert_eq!(
        session_state
            .queued_prompts_for_agent(worker.id())
            .map(|queued| queued.len())
            .unwrap_or_default(),
        0,
        "metaagent workflow steering failures must not queue detached prompts"
    );
}

#[test]
fn metaagent_prompt_command_does_not_queue_over_workflow_turns() {
    run_large_stack_async_test(
        "metaagent-prompt-queued-workflow-guard",
        metaagent_prompt_command_does_not_queue_over_workflow_turns_inner,
    );
}

async fn metaagent_prompt_command_does_not_queue_over_workflow_turns_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-queued-workflow-prompt-guard");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let workflow_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id("workflow-run-queued"),
        worker.id(),
        "workflow node prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context("workflow-run-queued", "workflow-node-run-queued");
    app.prompt_owner_submit_prepared_prompt(session.id(), workflow_prompt, true)
        .expect("workflow prompt should remain queued");

    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let result = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "prompt worker \"start this instead\""
            }),
        )
        .await
        .expect("meta run_command should return a structured failure");

    assert!(!result.ok, "{:?}", result.payload);
    assert!(
        result
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| {
                message.contains("already has queued workflow run")
                    && message.contains("normal metaagent prompts cannot be queued")
            }),
        "{:?}",
        result.payload
    );
    let session_state = app
        .lock()
        .await
        .sessions()
        .get_session(session.id())
        .expect("session should load");
    let queued_prompts = session_state
        .queued_prompts_for_agent(worker.id())
        .expect("workflow prompt should remain queued");
    assert_eq!(queued_prompts.len(), 1);
    assert_eq!(
        queued_prompts[0].workflow_run_id(),
        Some("workflow-run-queued")
    );
}

#[test]
fn metaagent_run_command_routes_core_workflow_commands() {
    run_large_stack_async_test(
        "metaagent-run-command-workflow",
        metaagent_run_command_routes_core_workflow_commands_inner,
    );
}

async fn metaagent_run_command_routes_core_workflow_commands_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-workflow");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("peer-worker")
                .with_owner_user_id("user-2"),
        )
        .expect("peer worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_owner_user_id(metaagent.owner_user_id()),
        )
        .expect("worker should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let created = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow new meta-flow"
            }),
        )
        .await
        .expect("workflow create command should dispatch");
    assert!(created.ok, "{:?}", created.payload);
    assert!(
        serde_json::to_string(&created.payload)
            .expect("payload should serialize")
            .contains("meta-flow"),
        "{:?}",
        created.payload
    );

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow list"
            }),
        )
        .await
        .expect("workflow list command should dispatch");
    assert!(listed.ok, "{:?}", listed.payload);
    assert!(
        serde_json::to_string(&listed.payload)
            .expect("payload should serialize")
            .contains("meta-flow"),
        "{:?}",
        listed.payload
    );

    let help = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow new --help"
            }),
        )
        .await
        .expect("workflow help-like aliases should return a structured usage error");
    assert!(!help.ok);
    assert!(
        help.payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("usage: workflow new [alias]")),
        "{:?}",
        help.payload
    );

    let node_added = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow node add meta-flow worker"
            }),
        )
        .await
        .expect("workflow node add command should dispatch");
    assert!(node_added.ok, "{:?}", node_added.payload);
    assert!(
        serde_json::to_string(&node_added.payload)
            .expect("payload should serialize")
            .contains(worker.id()),
        "{:?}",
        node_added.payload
    );
    let node_id = node_added
        .payload
        .pointer("/response/node/id")
        .and_then(serde_json::Value::as_str)
        .expect("node add response should include the node id")
        .to_string();

    let endpoint_created = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("workflow endpoint new meta-flow {node_id} default")
            }),
        )
        .await
        .expect("workflow endpoint new command should dispatch");
    assert!(endpoint_created.ok, "{:?}", endpoint_created.payload);
    assert!(
        endpoint_created
            .payload
            .pointer("/response/endpoint/alias")
            .and_then(serde_json::Value::as_str)
            == Some("default"),
        "{:?}",
        endpoint_created.payload
    );

    let meta_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow node add meta-flow meta"
            }),
        )
        .await
        .expect("metaagent node add should return structured denial");
    assert!(!meta_node.ok);
    assert!(
        meta_node
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        meta_node.payload
    );

    let peer_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("workflow node add meta-flow {}", peer_worker.agent_ref())
            }),
        )
        .await
        .expect("peer node add should return structured denial");
    assert!(!peer_node.ok);
    assert!(
        peer_node
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        peer_node.payload
    );

    let invalid_run = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow run endpoint-only"
            }),
        )
        .await
        .expect("workflow run usage errors should be structured");
    assert!(!invalid_run.ok);
    assert!(
        invalid_run
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("workflow run <workflow-ref>")),
        "{:?}",
        invalid_run.payload
    );
}

#[test]
fn metaagent_workflow_run_commands_expose_execution_visibility() {
    run_large_stack_async_test(
        "metaagent-workflow-run-visibility",
        metaagent_workflow_run_commands_expose_execution_visibility_inner,
    );
}

async fn metaagent_workflow_run_commands_expose_execution_visibility_inner() {
    let env = TestMetaRuntimeEnv::new("workflow-run-visibility");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_owner_user_id(metaagent.owner_user_id()),
        )
        .expect("worker should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let reviewer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("reviewer")
                .with_owner_user_id(metaagent.owner_user_id()),
        )
        .expect("reviewer should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, reviewer.id(), metaagent.id());
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let created = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow new visible-flow" }),
        )
        .await
        .expect("workflow create command should dispatch");
    assert!(created.ok);

    let worker_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow node add visible-flow worker" }),
        )
        .await
        .expect("worker node add command should dispatch");
    assert!(worker_node.ok);
    let worker_node_id = worker_node
        .payload
        .pointer("/response/node/id")
        .and_then(serde_json::Value::as_str)
        .expect("worker node add response should include node id")
        .to_string();

    let reviewer_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow node add visible-flow reviewer" }),
        )
        .await
        .expect("reviewer node add command should dispatch");
    assert!(reviewer_node.ok);
    let reviewer_node_id = reviewer_node
        .payload
        .pointer("/response/node/id")
        .and_then(serde_json::Value::as_str)
        .expect("reviewer node add response should include node id")
        .to_string();

    let edge_added = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("workflow edge add visible-flow {worker_node_id} {reviewer_node_id}")
            }),
        )
        .await
        .expect("workflow edge add command should dispatch");
    assert!(edge_added.ok);
    assert_eq!(
        edge_added
            .payload
            .pointer("/response/type")
            .and_then(serde_json::Value::as_str),
        Some("WorkflowEdgeAdded")
    );
    assert_eq!(
        edge_added
            .payload
            .pointer("/response/workflow/edges/0/from_node_id")
            .and_then(serde_json::Value::as_str),
        Some(worker_node_id.as_str())
    );

    let endpoint = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("workflow endpoint new visible-flow {worker_node_id} default")
            }),
        )
        .await
        .expect("workflow endpoint new command should dispatch");
    assert!(endpoint.ok);

    let invoked = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow run visible-flow default implement the requested change"
            }),
        )
        .await
        .expect("workflow run command should dispatch");
    assert!(invoked.ok);
    assert_eq!(
        invoked
            .payload
            .pointer("/response/type")
            .and_then(serde_json::Value::as_str),
        Some("WorkflowRunInvoked")
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/node_runs/0/node_id")
            .and_then(serde_json::Value::as_str),
        Some(worker_node_id.as_str())
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/active_node_run/node_id")
            .and_then(serde_json::Value::as_str),
        Some(worker_node_id.as_str())
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/active_node_run/turn/state")
            .and_then(serde_json::Value::as_str),
        Some("dispatched")
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/message_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/unconsumed_message_count")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/final_output_present")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    let run_id = invoked
        .payload
        .pointer("/response/workflow_run/id")
        .and_then(serde_json::Value::as_str)
        .expect("workflow run response should include id")
        .to_string();
    drop(invoked);

    let run_status = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": format!("workflow get-run {run_id}") }),
        )
        .await
        .expect("workflow get-run command should dispatch");
    assert!(run_status.ok);
    assert_eq!(
        run_status
            .payload
            .pointer("/response/type")
            .and_then(serde_json::Value::as_str),
        Some("WorkflowRun")
    );
    assert!(run_status
        .payload
        .pointer("/response/workflow_run/node_run_counts_by_status")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|counts| !counts.is_empty()));
}

#[test]
fn metaagent_run_command_routes_owned_agent_lifecycle_commands() {
    run_large_stack_async_test(
        "metaagent-run-command-agent-lifecycle",
        metaagent_run_command_routes_owned_agent_lifecycle_commands_inner,
    );
}

async fn metaagent_run_command_routes_owned_agent_lifecycle_commands_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-agent-lifecycle");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("peer-worker")
                .with_owner_user_id("user-2"),
        )
        .expect("peer worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let aliased = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "agent alias worker renamed-worker"
            }),
        )
        .await
        .expect("agent alias command should dispatch");
    assert!(aliased.ok, "{:?}", aliased.payload);
    assert!(
        serde_json::to_string(&aliased.payload)
            .expect("payload should serialize")
            .contains("renamed-worker"),
        "{:?}",
        aliased.payload
    );

    let peer_delete = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("agent delete {}", peer_worker.agent_ref())
            }),
        )
        .await
        .expect("peer delete should return structured denial");
    assert!(!peer_delete.ok);
    assert!(
        peer_delete
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        peer_delete.payload
    );

    let deleted = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "agent delete renamed-worker"
            }),
        )
        .await
        .expect("owned agent delete command should dispatch");
    assert!(deleted.ok, "{:?}", deleted.payload);
    let app = app.lock().await;
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should remain");
    assert!(session
        .agents()
        .iter()
        .all(|agent| agent.id() != worker.id()));
}

#[test]
fn metaagent_run_command_allows_agent_slice_placement_but_denies_slice_management_policy() {
    run_large_stack_async_test(
        "metaagent-slice-placement-policy",
        metaagent_run_command_allows_agent_slice_placement_but_denies_slice_management_policy_inner,
    );
}

async fn metaagent_run_command_allows_agent_slice_placement_but_denies_slice_management_policy_inner(
) {
    let env = TestMetaRuntimeEnv::new("run-command-slice-policy");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let daemon_id = app.config().daemon_id.clone();
    let host_machine_id = app.config().host_machine_id.clone();
    app.slices()
        .create(
            &daemon_id,
            &host_machine_id,
            crate::slice::CreateSliceInput {
                name: "linux-dev".to_string(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: crate::slice::SliceDisplayMode::Headless,
                workspace_id: Some(session.workspace_id().to_string()),
                worktree_id: Some(session.worktree_id().to_string()),
                workspace_mount: Some(session.worktree_id().to_string()),
                worker_kernel_ref: Some(daemon_id.clone()),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: crate::session::unix_epoch_ms(),
            },
        )
        .expect("test slice should be seeded");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let slice_placement = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "agent spawn helper --slice linux-dev"
            }),
        )
        .await
        .expect("slice-backed helper spawn should dispatch");
    assert!(slice_placement.ok, "{:?}", slice_placement.payload);

    let slice_list = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "slice list" }),
        )
        .await
        .expect("slice list should return a structured denial");
    assert!(!slice_list.ok, "{:?}", slice_list.payload);
    assert!(
        slice_list
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| {
                message.contains("cannot manage slices") && message.contains("regular agents")
            }),
        "{:?}",
        slice_list.payload
    );

    let reset_state = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "slice reset-state linux-dev" }),
        )
        .await
        .expect("unrouted slice command should return a structured denial");
    assert!(!reset_state.ok);
    assert!(
        reset_state
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("cannot manage slices")),
        "{:?}",
        reset_state.payload
    );
}

#[test]
fn collaborator_metaagents_are_allowed_and_controller_scoped() {
    run_large_stack_async_test(
        "collaborator-metaagent-scope",
        collaborator_metaagents_are_allowed_and_controller_scoped_inner,
    );
}

async fn collaborator_metaagents_are_allowed_and_controller_scoped_inner() {
    let env = TestMetaRuntimeEnv::new("collaborator-metaagent-scope");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_id,
            "invite-metaagent-collaborator".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Private,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("collaborator should join session");
    assert!(app
        .sessions()
        .get_session(&session_id)
        .expect("session should remain")
        .has_member("user-2"));

    let owner_metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("owner-meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("owner metaagent should spawn");
    let peer_metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("peer-meta")
                .with_owner_user_id("user-2")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("peer metaagent should spawn");
    let owner_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("owner-worker")
                .with_controlled_by_metaagent_id(owner_metaagent.id()),
        )
        .expect("owner worker should spawn");
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("peer-worker")
                .with_owner_user_id("user-2")
                .with_controlled_by_metaagent_id(peer_metaagent.id()),
        )
        .expect("peer worker should spawn");

    let owner_second_metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("owner-meta-2")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("owner should be allowed to create a second metaagent");
    let peer_second_metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("peer-meta-2")
                .with_owner_user_id("user-2")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("collaborator should be allowed to create a second metaagent");
    assert!(owner_second_metaagent.is_metaagent());
    assert!(peer_second_metaagent.is_metaagent());

    let owner_meta_run = launch_test_provider(
        &mut app,
        &session_id,
        owner_metaagent.id(),
        "dev-stub",
        "dev-stub",
        "owner-meta-model",
    );
    let peer_meta_run = launch_test_provider(
        &mut app,
        &session_id,
        peer_metaagent.id(),
        "dev-stub",
        "dev-stub",
        "peer-meta-model",
    );
    let owner_meta_auth_token = owner_meta_run
        .runtime_mcp_auth_token()
        .expect("owner meta run should expose runtime MCP auth token")
        .to_string();
    let peer_meta_auth_token = peer_meta_run
        .runtime_mcp_auth_token()
        .expect("peer meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let owner_alias = router
        .dispatch_authenticated_runtime_tool_call(
            &owner_meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent alias owner-worker owner-renamed" }),
        )
        .await
        .expect("owner metaagent should dispatch owned alias command");
    assert!(owner_alias.ok, "{:?}", owner_alias.payload);
    let owner_peer_denial = router
        .dispatch_authenticated_runtime_tool_call(
            &owner_meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent alias peer-worker owner-takeover" }),
        )
        .await
        .expect("owner metaagent peer alias should return structured denial");
    assert!(!owner_peer_denial.ok);
    assert!(
        owner_peer_denial
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        owner_peer_denial.payload
    );

    let peer_alias = router
        .dispatch_authenticated_runtime_tool_call(
            &peer_meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent alias peer-worker peer-renamed" }),
        )
        .await
        .expect("peer metaagent should dispatch owned alias command");
    assert!(peer_alias.ok, "{:?}", peer_alias.payload);
    let peer_owner_denial = router
        .dispatch_authenticated_runtime_tool_call(
            &peer_meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent alias owner-renamed peer-takeover" }),
        )
        .await
        .expect("peer metaagent owner alias should return structured denial");
    assert!(!peer_owner_denial.ok);
    assert!(
        peer_owner_denial
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        peer_owner_denial.payload
    );

    let app = app.lock().await;
    let owner_worker = app
        .agents()
        .get_agent(owner_worker.id())
        .expect("owner worker should remain");
    let peer_worker = app
        .agents()
        .get_agent(peer_worker.id())
        .expect("peer worker should remain");
    assert_eq!(owner_worker.alias(), Some("owner-renamed"));
    assert_eq!(peer_worker.alias(), Some("peer-renamed"));
}

#[test]
fn user_agent_lifecycle_events_notify_metaagent_but_meta_commands_do_not() {
    run_large_stack_async_test(
        "user-agent-lifecycle-events",
        user_agent_lifecycle_events_notify_metaagent_but_meta_commands_do_not_inner,
    );
}

async fn user_agent_lifecycle_events_notify_metaagent_but_meta_commands_do_not_inner() {
    let env = TestMetaRuntimeEnv::new("agent-lifecycle-events");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let human_spawn = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        session_id: session.id().to_string(),
        alias: Some("human-worker".to_string()),
        provider: Some("dev-stub".to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some(workspace.to_string_lossy().to_string()),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let spawned = router
        .dispatch(
            KernelCommand::from_local_request("human-spawn-worker", None, None, &human_spawn),
            human_spawn,
        )
        .await
        .expect("human spawn should dispatch");
    let LocalDaemonResponse::AgentSpawned {
        agent: human_worker,
    } = spawned
    else {
        panic!("unexpected human spawn response");
    };

    let events = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.spawned" }),
        )
        .await
        .expect("metaagent should list lifecycle events");
    assert!(events.ok, "{events:?}");
    assert_eq!(
        events
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        events.payload
    );

    let meta_spawn = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent spawn quiet-worker" }),
        )
        .await
        .expect("metaagent spawn command should dispatch");
    assert!(meta_spawn.ok, "{meta_spawn:?}");
    let events_after_meta_spawn = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.spawned" }),
        )
        .await
        .expect("metaagent should list lifecycle events");
    assert_eq!(
        events_after_meta_spawn
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        events_after_meta_spawn.payload
    );

    let human_delete = LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
        session_id: session.id().to_string(),
        agent_id: human_worker.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("human-delete-worker", None, None, &human_delete),
            human_delete,
        )
        .await
        .expect("human delete should dispatch");
    let deleted_events = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.deleted" }),
        )
        .await
        .expect("metaagent should list delete lifecycle events");
    assert_eq!(
        deleted_events
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        deleted_events.payload
    );
}

#[tokio::test]
async fn forged_metaagent_caller_id_does_not_suppress_lifecycle_events() {
    let env = TestMetaRuntimeEnv::new("forged-metaagent-caller");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let metaagent_id = metaagent.id().to_string();
    let session_id = session.id().to_string();
    let worktree_id = workspace.to_string_lossy().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        session_id,
        alias: Some("forged-worker".to_string()),
        provider: Some("dev-stub".to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some(worktree_id),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let mut command =
        KernelCommand::from_local_request("forged-metaagent-spawn-worker", None, None, &request);
    command.caller.caller_id = format!("metaagent:{metaagent_id}");
    router
        .dispatch(command, request)
        .await
        .expect("forged caller id should dispatch as a normal user command");

    let events = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.spawned" }),
        )
        .await
        .expect("metaagent should list lifecycle events");
    assert_eq!(
        events
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        events.payload
    );
}

#[test]
fn metaagent_run_command_returns_structured_denials_for_forbidden_commands() {
    run_large_stack_async_test(
        "metaagent-run-command-denials",
        metaagent_run_command_returns_structured_denials_for_forbidden_commands_inner,
    );
}

async fn metaagent_run_command_returns_structured_denials_for_forbidden_commands_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-deny");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let prompt_flag_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("prompt-flag-worker")
                .with_owner_user_id(metaagent.owner_user_id()),
        )
        .expect("prompt flag worker should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, prompt_flag_worker.id(), metaagent.id());
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "session new"
            }),
        )
        .await
        .expect("meta run_command denials should be structured tool results");

    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("cannot create")),
        "{:?}",
        denied.payload
    );

    let docs = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_COMMAND_DOCS_TOOL,
            serde_json::json!({
                "command": "mcp list"
            }),
        )
        .await
        .expect("meta command docs should dispatch");
    assert!(docs.ok);
    assert_eq!(
        docs.payload
            .get("metaagent_policy")
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
    assert_eq!(
        docs.payload
            .get("routed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    for command in ["mcp list", "skill list", "credential list"] {
        let routed = router
            .dispatch_authenticated_runtime_tool_call(
                &meta_auth_token,
                crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
                serde_json::json!({
                    "command": command
                }),
            )
            .await
            .expect("safe registered commands should dispatch");
        assert!(routed.ok, "{command}: {:?}", routed.payload);
    }

    let slice_denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "slice list"
            }),
        )
        .await
        .expect("slice commands should return structured denials");
    assert!(!slice_denied.ok);
    assert!(
        slice_denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("cannot manage slices")),
        "{:?}",
        slice_denied.payload
    );

    let not_routed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "mcp install test --command node"
            }),
        )
        .await
        .expect("registry-backed not-routed commands should return structured tool results");
    assert!(!not_routed.ok);
    assert!(
        not_routed
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("mcp install-json")),
        "{:?}",
        not_routed.payload
    );

    let prompt_flag_denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("prompt {} --wait inspect this", prompt_flag_worker.id())
            }),
        )
        .await
        .expect("prompt flag denial should return a structured tool result");
    assert!(!prompt_flag_denied.ok);
    assert!(
        prompt_flag_denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("does not support blocking reply flags")),
        "{:?}",
        prompt_flag_denied.payload
    );

    let audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    let denied_audit = audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.command.executed"
                && event.payload["metaagent_id"] == metaagent.id()
                && event.payload["command"] == "session new"
                && event.payload["status"] == "denied"
        })
        .expect("denied metaagent commands should be audited");
    assert_eq!(denied_audit.payload["provider_run_id"], meta_run.id());
    assert_eq!(denied_audit.payload["causation_id"], meta_run.id());
    let denied_correlation_id = denied_audit
        .payload
        .get("correlation_id")
        .and_then(serde_json::Value::as_str)
        .expect("denied command audit should include a correlation id");
    assert!(
        denied_correlation_id.starts_with(&format!("metaagent:{}:command:", metaagent.id())),
        "{denied_correlation_id}"
    );
}

#[test]
fn regular_agent_turn_completion_injects_metaagent_event_and_inbox_entry() {
    run_large_stack_async_test(
        "regular-agent-turn-completion-meta-event",
        regular_agent_turn_completion_injects_metaagent_event_and_inbox_entry_inner,
    );
}

async fn regular_agent_turn_completion_injects_metaagent_event_and_inbox_entry_inner() {
    let env = TestMetaRuntimeEnv::new("turn-event");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let _worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let attach = attach_request(session.id(), "client-1");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(worker.id().to_string()),
        prompt: "finish this test turn".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit", None, None, &submit),
            submit,
        )
        .await
        .expect("worker prompt should submit");
    let complete = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("complete", None, None, &complete),
            complete,
        )
        .await
        .expect("worker prompt should complete and notify metaagent");

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.turn.completed" }),
        )
        .await
        .expect("meta list_events should dispatch");
    assert!(listed.ok);
    let event = listed
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .and_then(|events| events.first())
        .expect("turn completion event should be listed");
    let event_id = event
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .expect("event should have id")
        .to_string();
    assert_eq!(
        event
            .get("source_agent_id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    assert!(
        event
            .get("injected_prompt_id")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "event should record injected prompt id"
    );
    assert!(
        event
            .get("prompt_delivery_status")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| matches!(status, "submitted" | "delivered")),
        "event should expose visible prompt delivery status: {event:?}"
    );

    let read = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_READ_EVENT_TOOL,
            serde_json::json!({ "event_id": event_id }),
        )
        .await
        .expect("meta read_event should dispatch");
    assert!(read.ok);
    assert_eq!(
        read.payload
            .pointer("/event/read_at_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        true
    );
    assert!(
        read.payload
            .pointer("/event/prompt_delivery_status")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "{:?}",
        read.payload
    );
    let acked = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_ACK_EVENT_TOOL,
            serde_json::json!({ "event_id": event_id }),
        )
        .await
        .expect("meta ack_event should dispatch");
    assert!(acked.ok);
    assert_eq!(
        acked
            .payload
            .get("acked")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn idle_metaagent_turn_with_active_task_injects_orphaned_task_event() {
    run_large_stack_async_test(
        "metaagent-orphaned-task-event",
        idle_metaagent_turn_with_active_task_injects_orphaned_task_event_inner,
    );
}

async fn idle_metaagent_turn_with_active_task_injects_orphaned_task_event_inner() {
    let env = TestMetaRuntimeEnv::new("metaagent-orphaned-task-event");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let attach = attach_request(session.id(), "client-meta-orphaned-task");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: "Start a task, then stop without marking it complete.".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-meta", None, None, &submit),
            submit,
        )
        .await
        .expect("metaagent prompt should submit");
    let complete = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("complete-meta", None, None, &complete),
            complete,
        )
        .await
        .expect("metaagent completion should inject orphan recovery prompt");

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "metaagent.task.orphaned" }),
        )
        .await
        .expect("meta list_events should dispatch");
    assert!(listed.ok);
    let event = listed
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .and_then(|events| events.first())
        .expect("orphaned task event should be listed");
    assert_eq!(
        event.get("kind").and_then(serde_json::Value::as_str),
        Some("metaagent.task.orphaned")
    );
    assert!(
        event
            .get("injected_prompt_id")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "orphaned task event should inject a continuation prompt: {event:?}"
    );
    assert!(
        event
            .get("prompt_delivery_status")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| matches!(status, "submitted" | "delivered")),
        "orphaned task event should be delivered to the metaagent: {event:?}"
    );
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session.id().to_string(),
    });
    let session_after = router
        .dispatch(
            KernelCommand::from_local_request("session-get", None, None, &state_request),
            state_request,
        )
        .await
        .expect("session should load");
    let LocalDaemonResponse::SessionState { session, .. } = session_after else {
        panic!("unexpected session response: {session_after:?}");
    };
    assert_eq!(
        session
            .metaagent_task(metaagent.id())
            .map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Active)
    );
    assert!(
        session.active_prompt_for_agent(metaagent.id()).is_some(),
        "orphan recovery prompt should leave the metaagent active"
    );
}

#[test]
fn metaagent_turn_with_active_worker_does_not_inject_orphaned_task_event() {
    run_large_stack_async_test(
        "metaagent-active-worker-no-orphaned-task-event",
        metaagent_turn_with_active_worker_does_not_inject_orphaned_task_event_inner,
    );
}

async fn metaagent_turn_with_active_worker_does_not_inject_orphaned_task_event_inner() {
    let env = TestMetaRuntimeEnv::new("metaagent-active-worker-no-orphaned");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let _worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let attach = attach_request(session.id(), "client-meta-active-worker");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let submit_worker = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(worker.id().to_string()),
        prompt: "keep working".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-worker", None, None, &submit_worker),
            submit_worker,
        )
        .await
        .expect("worker prompt should submit");
    let submit_meta = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: "Start a task while the worker is still active.".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-meta", None, None, &submit_meta),
            submit_meta,
        )
        .await
        .expect("metaagent prompt should submit");
    let complete_meta = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("complete-meta", None, None, &complete_meta),
            complete_meta,
        )
        .await
        .expect("metaagent completion should not inject orphan recovery while worker is active");

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "metaagent.task.orphaned" }),
        )
        .await
        .expect("meta list_events should dispatch");
    assert!(listed.ok);
    assert_eq!(
        listed
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(0),
        "active worker should suppress orphan recovery: {:?}",
        listed.payload
    );
}

#[test]
fn local_metaagent_command_search_request_enforces_owner_scope() {
    run_large_stack_async_test(
        "local-metaagent-command-search",
        local_metaagent_command_search_request_enforces_owner_scope_impl,
    );
}

async fn local_metaagent_command_search_request_enforces_owner_scope_impl() {
    let env = TestMetaRuntimeEnv::new("local-command-search");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let mut owner_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    owner_caller.user_id = Some(metaagent.owner_user_id().to_string());

    let search_request =
        LocalDaemonRequest::SearchMetaagentCommands(SearchMetaagentCommandsRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            query: Some("agent".to_string()),
            tag: Some("agent".to_string()),
            scope: Some("session".to_string()),
            mutates: None,
            policy: Some("allow".to_string()),
            limit: Some(10),
        });
    let searched = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "search-metaagent-commands",
                KernelCommandSource::LocalCli,
                owner_caller,
                None,
                None,
                &search_request,
            ),
            search_request.clone(),
        )
        .await
        .expect("owner should search metaagent commands");
    let LocalDaemonResponse::MetaagentCommandsSearched { commands } = searched else {
        panic!("unexpected metaagent command search response: {searched:?}");
    };
    assert!(
        commands.iter().any(|command| {
            command
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| name.contains("agent"))
        }),
        "command search should return agent command descriptors: {commands:?}"
    );

    let mut forged_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    forged_caller.user_id = Some("user-2".to_string());
    let denied = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "foreign-search-metaagent-commands",
                KernelCommandSource::LocalCli,
                forged_caller,
                None,
                None,
                &search_request,
            ),
            search_request,
        )
        .await
        .expect_err("another user must not search a metaagent command registry");
    assert!(
        denied
            .to_string()
            .contains("requires an owned session metaagent"),
        "{denied:?}"
    );
}

#[test]
fn local_metaagent_turn_inspection_requests_enforce_owner_scope() {
    run_large_stack_async_test(
        "local-metaagent-turn-inspection",
        local_metaagent_turn_inspection_requests_enforce_owner_scope_impl,
    );
}

async fn local_metaagent_turn_inspection_requests_enforce_owner_scope_impl() {
    let env = TestMetaRuntimeEnv::new("local-turn-inspection");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let prompt_entry = crate::history::SessionHistoryEntry::user_prompt(
        session.id(),
        "attachment-local-turn",
        worker.id(),
        "inspect this turn",
    );
    router
        .operational_history_store
        .append_transcript(
            &prompt_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(worker.id().to_string()),
                provider: Some(worker_run.provider().to_string()),
                model: Some(worker_run.model().to_string()),
                provider_run_id: Some(worker_run.id().to_string()),
                turn_id: Some("local-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("user prompt should append to operational history");
    let tool_entry = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        worker_run.id(),
        Some(worker.id()),
        crate::terminal::TerminalOutputKind::ProviderTool,
        None,
        serde_json::json!({
            "tool": "shell",
            "status": "completed",
            "input": {"command": "cargo test"}
        })
        .to_string(),
    );
    router
        .operational_history_store
        .append_transcript(
            &tool_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(worker.id().to_string()),
                provider: Some(worker_run.provider().to_string()),
                model: Some(worker_run.model().to_string()),
                provider_run_id: Some(worker_run.id().to_string()),
                turn_id: Some("local-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("provider tool output should append to operational history");

    let mut owner_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    owner_caller.user_id = Some(metaagent.owner_user_id().to_string());
    let overview_request =
        LocalDaemonRequest::GetMetaagentTurnOverview(GetMetaagentTurnOverviewRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            agent_ref: Some("worker".to_string()),
            turn_ref: Some("local-turn".to_string()),
            turns_back: None,
            limit: Some(20),
        });
    let overview = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "local-metaagent-turn-overview",
                KernelCommandSource::LocalCli,
                owner_caller.clone(),
                None,
                None,
                &overview_request,
            ),
            overview_request,
        )
        .await
        .expect("owner should inspect metaagent turn overview");
    let LocalDaemonResponse::MetaagentTurnOverview { overview } = overview else {
        panic!("unexpected metaagent turn overview response: {overview:?}");
    };
    assert_eq!(
        overview
            .pointer("/agent/id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    let blob_id = overview
        .pointer("/turns/0/items")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.iter().find_map(|item| item.get("blob_id")))
        .and_then(serde_json::Value::as_str)
        .expect("overview should expose provider tool blob id")
        .to_string();

    let blob_request = LocalDaemonRequest::GetMetaagentTurnBlob(GetMetaagentTurnBlobRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        blob_id: blob_id.clone(),
    });
    let blob = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "local-metaagent-turn-blob",
                KernelCommandSource::LocalCli,
                owner_caller,
                None,
                None,
                &blob_request,
            ),
            blob_request,
        )
        .await
        .expect("owner should inspect metaagent turn blob");
    let LocalDaemonResponse::MetaagentTurnBlob { blob } = blob else {
        panic!("unexpected metaagent turn blob response: {blob:?}");
    };
    assert_eq!(
        blob.get("blob_id").and_then(serde_json::Value::as_str),
        Some(blob_id.as_str())
    );
    assert!(
        blob.pointer("/entries/0/entry/text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| text.contains("cargo test")),
        "{blob:?}"
    );

    let mut forged_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    forged_caller.user_id = Some("user-2".to_string());
    let forged_request =
        LocalDaemonRequest::GetMetaagentTurnOverview(GetMetaagentTurnOverviewRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            agent_ref: Some("worker".to_string()),
            turn_ref: Some("local-turn".to_string()),
            turns_back: None,
            limit: Some(20),
        });
    let denied = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "forged-local-metaagent-turn-overview",
                KernelCommandSource::LocalCli,
                forged_caller,
                None,
                None,
                &forged_request,
            ),
            forged_request,
        )
        .await
        .expect_err("foreign users must not inspect owned metaagent turns");
    assert!(
        denied.to_string().contains("owned session metaagent"),
        "{denied}"
    );
}

#[test]
fn local_metaagent_event_requests_enforce_owner_and_mutate_inbox() {
    run_large_stack_async_test(
        "local-metaagent-event-requests",
        local_metaagent_event_requests_enforce_owner_and_mutate_inbox_impl,
    );
}

async fn local_metaagent_event_requests_enforce_owner_and_mutate_inbox_impl() {
    let env = TestMetaRuntimeEnv::new("local-event-requests");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let event =
        app.metaagent_event_store()
            .record(crate::runtime::metaagent_event::NewMetaagentEvent {
                session_id: session.id().to_string(),
                metaagent_id: metaagent.id().to_string(),
                owner_user_id: metaagent.owner_user_id().to_string(),
                kind: "agent.turn.completed".to_string(),
                source_agent_id: Some("agent-1".to_string()),
                title: "Worker completed".to_string(),
                summary: "Worker completed a turn".to_string(),
                detail: serde_json::json!({ "prompt_id": "prompt-1" }),
                injected_prompt_id: None,
            });
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let mut owner_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    owner_caller.user_id = Some(metaagent.owner_user_id().to_string());

    let list_request = LocalDaemonRequest::ListMetaagentEvents(ListMetaagentEventsRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        limit: Some(10),
        status: None,
        kind: Some("agent.turn.completed".to_string()),
    });
    let listed = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "list-metaagent-events",
                KernelCommandSource::LocalCli,
                owner_caller.clone(),
                None,
                None,
                &list_request,
            ),
            list_request.clone(),
        )
        .await
        .expect("owner should list metaagent events");
    let LocalDaemonResponse::MetaagentEventsListed { events } = listed else {
        panic!("unexpected metaagent event list response: {listed:?}");
    };
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]
            .get("event_id")
            .and_then(serde_json::Value::as_str),
        Some(event.event_id.as_str())
    );

    let mut forged_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    forged_caller.user_id = Some("user-2".to_string());
    let denied = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "foreign-list-metaagent-events",
                KernelCommandSource::LocalCli,
                forged_caller,
                None,
                None,
                &list_request,
            ),
            list_request,
        )
        .await
        .expect_err("another user must not list a metaagent inbox");
    assert!(
        denied
            .to_string()
            .contains("requires an owned session metaagent"),
        "{denied:?}"
    );

    let read_request = LocalDaemonRequest::ReadMetaagentEvent(ReadMetaagentEventRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        event_id: event.event_id.clone(),
    });
    let read = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "read-metaagent-event",
                KernelCommandSource::LocalCli,
                owner_caller.clone(),
                None,
                None,
                &read_request,
            ),
            read_request,
        )
        .await
        .expect("owner should read metaagent event");
    let LocalDaemonResponse::MetaagentEventRead { event: read_event } = read else {
        panic!("unexpected metaagent event read response: {read:?}");
    };
    assert!(
        read_event
            .get("read_at_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "{read_event:?}"
    );

    let ack_request = LocalDaemonRequest::AckMetaagentEvents(AckMetaagentEventsRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        event_id: Some(event.event_id.clone()),
        event_ids: None,
        up_to_sequence: None,
    });
    let acked = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "ack-metaagent-event",
                KernelCommandSource::LocalCli,
                owner_caller,
                None,
                None,
                &ack_request,
            ),
            ack_request,
        )
        .await
        .expect("owner should ack metaagent event");
    let LocalDaemonResponse::MetaagentEventsAcked { acked } = acked else {
        panic!("unexpected metaagent event ack response: {acked:?}");
    };
    assert_eq!(acked.len(), 1);
    assert!(
        acked[0]
            .get("ack_at_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "{acked:?}"
    );
}

#[tokio::test]
async fn metaagent_event_prompts_retry_after_provider_launch() {
    let env = TestMetaRuntimeEnv::new("event-retry-after-launch");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    router
        .runtime_state
        .inject_metaagent_agent_lifecycle_event_for_agent(session.id(), &worker, "agent.spawned")
        .await
        .expect("metaagent event should record even before provider launch");
    let failed_event = app
        .lock()
        .await
        .metaagent_event_store()
        .list(metaagent.id(), Some("agent.spawned"), Some("failed"), 10)
        .into_iter()
        .next()
        .expect("event prompt should fail while no metaagent provider route exists");
    let failed_prompt_id = failed_event.injected_prompt_id.clone();

    let started = app
        .lock()
        .await
        .start_provider_launch(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "meta-model",
            )
            .with_agent_id(metaagent.id()),
        )
        .expect("metaagent provider launch should start");
    router
        .runtime_state
        .finish_provider_launch(&started, None)
        .await;

    let retried_event = app
        .lock()
        .await
        .metaagent_event_store()
        .read(metaagent.id(), &failed_event.event_id)
        .expect("event should still be readable after retry");
    assert_ne!(
        retried_event.prompt_delivery_status,
        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
        "provider launch should retry failed metaagent event prompts: {retried_event:?}"
    );
    assert_ne!(
        retried_event.injected_prompt_id, failed_prompt_id,
        "retry should use a fresh prompt id for the replayed visible event prompt"
    );
    assert!(
        matches!(
            retried_event.prompt_delivery_status.as_str(),
            "submitted" | "queued" | "delivered"
        ),
        "retried event should be re-admitted through normal prompt delivery: {retried_event:?}"
    );
}

#[test]
fn metaagent_turn_overview_and_blob_are_scoped_to_owned_regular_agents() {
    run_large_stack_async_test(
        "metaagent-turn-overview-and-blob-are-scoped",
        metaagent_turn_overview_and_blob_are_scoped_to_owned_regular_agents_inner,
    );
}

async fn metaagent_turn_overview_and_blob_are_scoped_to_owned_regular_agents_inner() {
    let env = TestMetaRuntimeEnv::new("turn-trace");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("peer-worker")
                .with_owner_user_id("user-2"),
        )
        .expect("peer worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let peer_worker_run = launch_test_provider(
        &mut app,
        session.id(),
        peer_worker.id(),
        "dev-stub",
        "dev-stub",
        "peer-worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let attach = attach_request(session.id(), "client-trace");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach-trace", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(worker.id().to_string()),
        prompt: "summarize the trace".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-trace", None, None, &submit),
            submit,
        )
        .await
        .expect("worker prompt should submit");
    let tool_entry = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        worker_run.id(),
        Some(worker.id()),
        crate::terminal::TerminalOutputKind::ProviderTool,
        None,
        serde_json::json!({
            "tool": "shell",
            "status": "completed",
            "input": {"command": "cargo test"}
        })
        .to_string(),
    );
    router
        .operational_history_store
        .append_transcript(
            &tool_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(worker.id().to_string()),
                provider: Some(worker_run.provider().to_string()),
                model: Some(worker_run.model().to_string()),
                provider_run_id: Some(worker_run.id().to_string()),
                turn_id: Some("trace-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("provider tool output should append to operational history");
    let peer_prompt_entry = crate::history::SessionHistoryEntry::user_prompt(
        session.id(),
        "peer-attachment",
        peer_worker.id(),
        "peer private prompt",
    );
    router
        .operational_history_store
        .append_transcript(
            &peer_prompt_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(peer_worker.id().to_string()),
                provider: Some(peer_worker_run.provider().to_string()),
                model: Some(peer_worker_run.model().to_string()),
                provider_run_id: Some(peer_worker_run.id().to_string()),
                turn_id: Some("peer-trace-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("peer prompt should append to operational history");
    let peer_tool_entry = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        peer_worker_run.id(),
        Some(peer_worker.id()),
        crate::terminal::TerminalOutputKind::ProviderTool,
        None,
        serde_json::json!({
            "tool": "shell",
            "status": "completed",
            "input": {"command": "cat secret-peer-file"}
        })
        .to_string(),
    );
    router
        .operational_history_store
        .append_transcript(
            &peer_tool_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(peer_worker.id().to_string()),
                provider: Some(peer_worker_run.provider().to_string()),
                model: Some(peer_worker_run.model().to_string()),
                provider_run_id: Some(peer_worker_run.id().to_string()),
                turn_id: Some("peer-trace-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("peer provider tool output should append to operational history");

    let overview = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL,
            serde_json::json!({ "agent_ref": "worker" }),
        )
        .await
        .expect("turn overview should dispatch");
    assert!(overview.ok, "{:?}", overview.payload);
    let blob_id = overview
        .payload
        .pointer("/turns/0/items")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.iter().find_map(|item| item.get("blob_id")))
        .and_then(serde_json::Value::as_str)
        .expect("overview should include provider tool blob id")
        .to_string();

    let blob = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_BLOB_TOOL,
            serde_json::json!({ "blob_id": blob_id }),
        )
        .await
        .expect("turn blob should dispatch");
    assert!(blob.ok, "{:?}", blob.payload);
    assert_eq!(
        blob.payload
            .pointer("/agent/id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    assert!(
        blob.payload
            .pointer("/entries/0/entry/text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| text.contains("cargo test")),
        "{:?}",
        blob.payload
    );

    let denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL,
            serde_json::json!({ "agent_ref": "meta" }),
        )
        .await
        .expect("turn overview denial should dispatch");
    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        denied.payload
    );

    let peer_overview_denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL,
            serde_json::json!({ "agent_ref": "peer-worker" }),
        )
        .await
        .expect("peer turn overview denial should dispatch");
    assert!(!peer_overview_denied.ok);
    assert!(
        peer_overview_denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        peer_overview_denied.payload
    );

    let peer_history = crate::runtime::history_requests::execute_session_history_outline_request(
        router.operational_history_store.clone(),
        crate::local::GetSessionHistoryOutlineRequest {
            session_id: session.id().to_string(),
            agent_ids: Some(vec![peer_worker.id().to_string()]),
            latest_prompt_count: Some(1),
            cursor: None,
        },
    )
    .await
    .expect("peer history outline should load");
    let crate::local::LocalDaemonResponse::SessionHistoryOutline { agents } = peer_history else {
        panic!("unexpected peer history response");
    };
    let peer_blob_id = agents
        .first()
        .and_then(|agent| agent.turns.first())
        .and_then(|turn| turn.blobs.first())
        .map(|blob| blob.blob_id.clone())
        .expect("peer history should include a blob");
    let peer_blob_denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_BLOB_TOOL,
            serde_json::json!({ "blob_id": peer_blob_id }),
        )
        .await
        .expect("peer blob denial should dispatch");
    assert!(!peer_blob_denied.ok);
    assert!(
        peer_blob_denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("owned regular agent")),
        "{:?}",
        peer_blob_denied.payload
    );
}

#[tokio::test]
async fn metaagent_event_subscriptions_persist_and_can_be_removed() {
    let env = TestMetaRuntimeEnv::new("event-subscriptions");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let subscribed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow.output.final" }),
        )
        .await
        .expect("subscribe should dispatch");
    assert!(subscribed.ok);
    let subscription_id = subscribed
        .payload
        .pointer("/subscription/subscription_id")
        .and_then(serde_json::Value::as_str)
        .expect("subscription id should be returned")
        .to_string();

    let listed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_SUBSCRIPTIONS_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect("list subscriptions should dispatch");
    assert!(listed.ok);
    let subscriptions = listed
        .payload
        .get("subscriptions")
        .and_then(serde_json::Value::as_array)
        .expect("subscriptions should be listed");
    let valid_event_kinds = listed
        .payload
        .get("valid_event_kinds")
        .and_then(serde_json::Value::as_array)
        .expect("valid event kinds should be listed");
    assert!(valid_event_kinds.iter().any(|kind| {
        kind.as_str()
            == Some(crate::transport::runtime_tools::META_EVENT_KIND_WORKFLOW_OUTPUT_FINAL)
    }));
    assert!(subscriptions.iter().any(|subscription| {
        subscription.get("kind").and_then(serde_json::Value::as_str) == Some("agent.turn.completed")
            && subscription
                .get("required")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    }));
    assert!(subscriptions.iter().any(|subscription| {
        subscription
            .get("subscription_id")
            .and_then(serde_json::Value::as_str)
            == Some(subscription_id.as_str())
            && subscription.get("kind").and_then(serde_json::Value::as_str)
                == Some("workflow.output.final")
    }));

    let removed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_UNSUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "subscription_id": subscription_id }),
        )
        .await
        .expect("unsubscribe should dispatch");
    assert!(removed.ok);
    assert_eq!(
        removed
            .payload
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("removed")
    );
}

#[tokio::test]
async fn metaagent_event_subscription_rejects_unknown_kinds_with_suggestions() {
    let env = TestMetaRuntimeEnv::new("event-subscription-validation");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let rejected = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow_output" }),
        )
        .await
        .expect("subscribe should dispatch");

    assert!(!rejected.ok);
    assert_eq!(
        rejected
            .payload
            .get("kind")
            .and_then(serde_json::Value::as_str),
        Some("workflow_output")
    );
    let suggestions = rejected
        .payload
        .get("suggestions")
        .and_then(serde_json::Value::as_array)
        .expect("suggestions should be returned");
    assert!(suggestions.iter().any(|suggestion| {
        suggestion.as_str()
            == Some(crate::transport::runtime_tools::META_EVENT_KIND_WORKFLOW_OUTPUT_FINAL)
    }));
    let valid_event_kinds = rejected
        .payload
        .get("valid_event_kinds")
        .and_then(serde_json::Value::as_array)
        .expect("valid event kinds should be returned");
    assert!(valid_event_kinds.iter().any(|kind| {
        kind.as_str() == Some(crate::transport::runtime_tools::META_EVENT_KIND_AGENT_TURN_COMPLETED)
    }));
}

#[test]
fn subscribed_collaborator_workflow_output_records_and_injects_metaagent_event() {
    run_large_stack_async_test(
        "subscribed-collaborator-workflow-output",
        subscribed_collaborator_workflow_output_records_and_injects_metaagent_event_inner,
    );
}

async fn subscribed_collaborator_workflow_output_records_and_injects_metaagent_event_inner() {
    let env = TestMetaRuntimeEnv::new("workflow-output-event");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_owner_user_id("user-2"),
        )
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let worker_auth_token = worker_run
        .runtime_mcp_auth_token()
        .expect("worker run should expose runtime MCP auth token")
        .to_string();
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow.output.final" }),
        )
        .await
        .expect("metaagent should subscribe to workflow final outputs");

    let create_workflow = LocalDaemonRequest::CreateWorkflow(crate::local::CreateWorkflowRequest {
        session_id: session.id().to_string(),
        alias: Some("review".to_string()),
    });
    let workflow = match router
        .dispatch(
            KernelCommand::from_local_request("create-workflow", None, None, &create_workflow),
            create_workflow,
        )
        .await
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        other => panic!("unexpected workflow create response: {other:?}"),
    };
    let add_node = LocalDaemonRequest::AddWorkflowNode(crate::local::AddWorkflowNodeRequest {
        session_id: session.id().to_string(),
        workflow_ref: workflow.id().to_string(),
        agent_id: worker.id().to_string(),
        expected_workflow_revision: None,
    });
    let node = match router
        .dispatch(
            KernelCommand::from_local_request("add-workflow-node", None, None, &add_node),
            add_node,
        )
        .await
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        other => panic!("unexpected workflow node response: {other:?}"),
    };
    let set_completion = LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(
        crate::local::SetWorkflowNodeCanCompleteRunRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            node_id: node.id().to_string(),
            can_complete_workflow_run: true,
            expected_workflow_revision: None,
        },
    );
    router
        .dispatch(
            KernelCommand::from_local_request("set-workflow-complete", None, None, &set_completion),
            set_completion,
        )
        .await
        .expect("workflow node should be allowed to complete run");
    let create_endpoint =
        LocalDaemonRequest::CreateWorkflowEndpoint(crate::local::CreateWorkflowEndpointRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            entry_node_id: node.id().to_string(),
            alias: Some("entry".to_string()),
            expected_workflow_revision: None,
        });
    let endpoint = match router
        .dispatch(
            KernelCommand::from_local_request(
                "create-workflow-endpoint",
                None,
                None,
                &create_endpoint,
            ),
            create_endpoint,
        )
        .await
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        other => panic!("unexpected workflow endpoint response: {other:?}"),
    };
    let invoke =
        LocalDaemonRequest::InvokeWorkflowEndpoint(crate::local::InvokeWorkflowEndpointRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            endpoint_ref: endpoint.id().to_string(),
            prompt: Some("produce final output".to_string()),
            queue_ref: None,
            publication_invocation: None,
        });
    let workflow_run = match router
        .dispatch(
            KernelCommand::from_local_request("invoke-workflow", None, None, &invoke),
            invoke,
        )
        .await
        .expect("workflow should be invoked")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        other => panic!("unexpected workflow invoke response: {other:?}"),
    };

    let submitted = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &worker_auth_token,
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
            serde_json::json!({
                "workflow_output_json": "{\"summary\":\"done\"}"
            }),
        )
        .await
        .expect("workflow final output tool should dispatch");
    assert!(submitted.ok, "{:?}", submitted.payload);
    assert_eq!(
        submitted
            .payload
            .get("workflow_run_id")
            .and_then(serde_json::Value::as_str),
        Some(workflow_run.id())
    );

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow.output.final" }),
        )
        .await
        .expect("workflow final output event should list");
    assert!(listed.ok);
    let event = listed
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .and_then(|events| events.first())
        .expect("workflow output event should be recorded");
    assert_eq!(
        event
            .get("source_agent_id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    assert!(
        event
            .get("injected_prompt_id")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "workflow output event should record prompt injection id"
    );
}

#[tokio::test]
async fn metaagent_can_resolve_owned_regular_agent_interactions_but_not_its_own() {
    let env = TestMetaRuntimeEnv::new("interaction");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let attach = attach_request(session.id(), "client-meta-busy");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach-meta-busy", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let meta_prompt = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: "stay busy while a worker asks for permission".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-meta-busy", None, None, &meta_prompt),
            meta_prompt,
        )
        .await
        .expect("metaagent prompt should start");
    let worker_interaction = RuntimeInteraction::new(
        "interaction-worker",
        worker.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow command?".to_string()),
        "Allow command?",
        vec![
            RuntimeInteractionChoice::new(
                "allow_once",
                "Allow once",
                "allow",
                Some(RuntimeInteractionChoiceStyle::Primary),
            ),
            RuntimeInteractionChoice::new(
                "deny",
                "Deny",
                "deny",
                Some(RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        None,
        None,
    );
    let worker_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), worker_interaction)
        .await
        .expect("worker interaction should register");
    let meta_queued_prompts = app
        .lock()
        .await
        .sessions()
        .get_session(session.id())
        .expect("session should load")
        .queued_prompts_for_agent(metaagent.id())
        .map(|queued| queued.len())
        .unwrap_or_default();
    assert_eq!(
        meta_queued_prompts, 0,
        "runtime interaction event prompts should steer an active metaagent instead of queueing"
    );
    let listed_events = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "runtime.interaction" }),
        )
        .await
        .expect("required interaction event should be listed");
    assert!(listed_events.ok);
    let events = listed_events
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .expect("interaction events should be returned");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events
            .first()
            .and_then(|event| event.get("source_agent_id"))
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );

    let resolved = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
            serde_json::json!({
                "interaction_id": "interaction-worker",
                "choice_id": "allow_once"
            }),
        )
        .await
        .expect("meta interaction resolution should dispatch");
    assert!(resolved.ok, "{:?}", resolved.payload);
    let resolution = tokio::time::timeout(std::time::Duration::from_secs(1), worker_resolution)
        .await
        .expect("resolution should be delivered")
        .expect("interaction responder should receive resolution");
    assert_eq!(resolution.choice_id.as_deref(), Some("allow_once"));
    assert_eq!(resolution.reply.as_deref(), Some("allow"));
    let audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    let resolution_audit = audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.interaction.resolved"
                && event.payload["session_id"] == session.id()
                && event.payload["metaagent_id"] == metaagent.id()
                && event.payload["target_agent_id"] == worker.id()
                && event.payload["interaction_id"] == "interaction-worker"
                && event.payload["choice_id"] == "allow_once"
                && event.payload["causation_id"] == "interaction-worker"
                && event.payload["correlation_id"]
                    == format!(
                        "metaagent:{}:runtime-interaction:interaction-worker",
                        metaagent.id()
                    )
        })
        .expect("metaagent interaction resolution should include durable provenance");
    assert_eq!(resolution_audit.payload["provider_run_id"], worker_run.id());
    assert!(
        resolution_audit
            .payload
            .get("timestamp_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "{:?}",
        resolution_audit.payload
    );
    assert_eq!(resolution_audit.payload["input"], serde_json::Value::Null);

    let custom_interaction = RuntimeInteraction::new(
        "interaction-custom-worker",
        worker.id(),
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Warning,
        Some("Explain approval".to_string()),
        "Explain approval",
        vec![RuntimeInteractionChoice::new(
            "cancel",
            "Cancel",
            "cancel",
            Some(RuntimeInteractionChoiceStyle::Danger),
        )],
        Some(crate::session::RuntimeInteractionCustomChoice::new(
            "custom_reason",
            "Custom reason",
            Some("Reason".to_string()),
            Some(3),
            Some(256),
        )),
        None,
        None,
    );
    let custom_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), custom_interaction)
        .await
        .expect("custom worker interaction should register");
    let custom_reply = "ship after checking logs";
    let custom_resolved = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
            serde_json::json!({
                "interaction_id": "interaction-custom-worker",
                "choice_id": "custom_reason",
                "input": custom_reply
            }),
        )
        .await
        .expect("custom meta interaction resolution should dispatch");
    assert!(custom_resolved.ok, "{:?}", custom_resolved.payload);
    let custom_runtime_resolution =
        tokio::time::timeout(std::time::Duration::from_secs(1), custom_resolution)
            .await
            .expect("custom resolution should be delivered")
            .expect("custom interaction responder should receive resolution");
    assert_eq!(
        custom_runtime_resolution.choice_id.as_deref(),
        Some("custom_reason")
    );
    assert_eq!(
        custom_runtime_resolution.reply.as_deref(),
        Some(custom_reply)
    );
    let custom_audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    let custom_audit = custom_audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.interaction.resolved"
                && event.payload["interaction_id"] == "interaction-custom-worker"
        })
        .expect("custom interaction resolution should include durable provenance");
    assert_eq!(custom_audit.payload["provider_run_id"], worker_run.id());
    assert_eq!(
        custom_audit.payload.pointer("/input/kind"),
        Some(&serde_json::json!("custom"))
    );
    assert_eq!(
        custom_audit.payload.pointer("/input/char_count"),
        Some(&serde_json::json!(custom_reply.chars().count()))
    );
    assert_eq!(
        custom_audit.payload["input"]["reply"],
        serde_json::Value::Null
    );

    let self_interaction = RuntimeInteraction::new(
        "interaction-meta",
        metaagent.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow self?".to_string()),
        "Allow self?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _self_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), self_interaction)
        .await
        .expect("self interaction should register");
    let denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
            serde_json::json!({
                "interaction_id": "interaction-meta",
                "choice_id": "allow_once"
            }),
        )
        .await
        .expect("self resolution denial should dispatch");
    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("cannot resolve their own")),
        "{:?}",
        denied.payload
    );
}
