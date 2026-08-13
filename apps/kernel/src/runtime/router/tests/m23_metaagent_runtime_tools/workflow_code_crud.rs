use super::*;

#[test]
fn metaagent_runtime_tools_create_validate_apply_and_delete_workflow_code() {
    run_large_stack_async_test(
        "metaagent-runtime-tools-create-validate-apply-and-delete-workflow-code",
        metaagent_runtime_tools_create_validate_apply_and_delete_workflow_code_inner,
    );
}

async fn metaagent_runtime_tools_create_validate_apply_and_delete_workflow_code_inner() {
    if let Err(error) = crate::workflow_code::discover_workflow_code_node_path() {
        eprintln!("skipping meta workflow-code tool test because Node.js is unavailable: {error}");
        return;
    }

    let env = TestMetaRuntimeEnv::new("workflow-code");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    std::fs::create_dir_all(workspace.join("schemas")).expect("schema directory should be created");
    let skill_dir = workspace
        .join(".chariox")
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
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let guide_search = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_SEARCH_GUIDES_TOOL,
            serde_json::json!({
                "query": "workflow code javascript builder",
                "tag": "workflow-code",
                "command": "chariox.meta.workflow_code.create",
                "limit": 5
            }),
        )
        .await
        .expect("metaagent should discover workflow-code authoring guides");
    assert!(guide_search.ok, "{:?}", guide_search.payload);
    assert!(
        guide_search
            .payload
            .get("guides")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|guides| guides.iter().any(|guide| {
                guide.get("id").and_then(serde_json::Value::as_str)
                    == Some("workflows/workflow-code-authoring")
                    && guide.get("body").is_none()
            })),
        "workflow-code guide search should surface authoring guide summaries without bodies: {:?}",
        guide_search.payload
    );
    let authoring_guide = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_READ_GUIDE_TOOL,
            serde_json::json!({
                "guide": "workflows/workflow-code-authoring"
            }),
        )
        .await
        .expect("metaagent should read workflow-code authoring guide");
    assert!(authoring_guide.ok, "{:?}", authoring_guide.payload);
    let authoring_body = authoring_guide
        .payload
        .get("body")
        .and_then(serde_json::Value::as_str)
        .expect("authoring guide should include body");
    for expected in [
        "workflow.define(options)",
        "workflow.newAgent(options)",
        "workflow.schemaFromFile(options)",
        "chariox.meta.workflow_code.validate",
        "chariox.meta.workflow_code.apply",
        "chariox.meta.workflow_code.run",
    ] {
        assert!(
            authoring_body.contains(expected),
            "authoring guide should teach `{expected}`"
        );
    }
    let pattern_guide_search = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_SEARCH_GUIDES_TOOL,
            serde_json::json!({
                "query": "workflow-code tournament adversarial evaluator optimizer",
                "tag": "workflow-code",
                "command": "chariox.meta.workflow_code.validate",
                "limit": 5
            }),
        )
        .await
        .expect("metaagent should discover workflow-code pattern guide");
    assert!(
        pattern_guide_search.ok,
        "{:?}",
        pattern_guide_search.payload
    );
    assert!(
        pattern_guide_search
            .payload
            .get("guides")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|guides| guides.iter().any(|guide| {
                guide.get("id").and_then(serde_json::Value::as_str)
                    == Some("workflows/workflow-code-patterns")
                    && guide.get("body").is_none()
            })),
        "workflow-code guide search should surface pattern guide summaries without bodies: {:?}",
        pattern_guide_search.payload
    );
    let pattern_guide = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_READ_GUIDE_TOOL,
            serde_json::json!({
                "guide": "workflows/workflow-code-patterns"
            }),
        )
        .await
        .expect("metaagent should read workflow-code pattern guide");
    assert!(pattern_guide.ok, "{:?}", pattern_guide.payload);
    let pattern_body = pattern_guide
        .payload
        .get("body")
        .and_then(serde_json::Value::as_str)
        .expect("pattern guide should include body");
    for example in crate::workflow_code::WORKFLOW_CODE_PATTERN_EXAMPLES {
        assert!(
            pattern_body.contains(example.path),
            "pattern guide should include `{}`",
            example.path
        );
        assert!(
            pattern_body.contains(example.source.trim()),
            "pattern guide should embed `{}` source",
            example.slug
        );
    }
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
                        .is_some_and(|grants| {
                            grants.iter().any(|grant| {
                                grant.get("kind").and_then(serde_json::Value::as_str)
                                    == Some("skill")
                                    && grant.get("name").and_then(serde_json::Value::as_str)
                                        == Some("meta-workflow-code-skill")
                            })
                        })
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

    let package_exported = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL,
            serde_json::json!({ "name": "meta-flow-imported" }),
        )
        .await
        .expect("metaagent should export explicit workflow-code package");
    assert!(package_exported.ok, "{:?}", package_exported.payload);
    assert_eq!(
        package_exported
            .payload
            .pointer("/WorkflowCodePackageExported/package/name")
            .and_then(serde_json::Value::as_str),
        Some("meta-flow-imported")
    );
    let package = package_exported
        .payload
        .pointer("/WorkflowCodePackageExported/package")
        .cloned()
        .expect("explicit package export should return package");
    let package_imported = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL,
            serde_json::json!({
                "package": package,
                "name": "meta-flow-package-imported"
            }),
        )
        .await
        .expect("metaagent should import explicit workflow-code package");
    assert!(package_imported.ok, "{:?}", package_imported.payload);
    assert_eq!(
        package_imported
            .payload
            .pointer("/WorkflowCodePackageImported/artifact/metadata/name")
            .and_then(serde_json::Value::as_str),
        Some("meta-flow-package-imported")
    );
    let source_exported = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL,
            serde_json::json!({ "name": "meta-flow-imported" }),
        )
        .await
        .expect("metaagent should export workflow-code source directory");
    assert!(source_exported.ok, "{:?}", source_exported.payload);
    assert_eq!(
        source_exported
            .payload
            .pointer("/WorkflowCodeSourceExported/export/source_path")
            .and_then(serde_json::Value::as_str),
        Some("workflow.js")
    );
    assert!(
        source_exported
            .payload
            .pointer("/WorkflowCodeSourceExported/export/files")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|files| files.iter().any(|file| {
                file.get("path").and_then(serde_json::Value::as_str) == Some("manifest.json")
            })),
        "source_export_directory should return a manifest file"
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

    let deleted_package_import = router
        .runtime_state
        .dispatch_meta_runtime_tool_call_for_agent(
            session.id(),
            metaagent.id(),
            crate::transport::runtime_tools::META_WORKFLOW_CODE_DELETE_TOOL,
            serde_json::json!({ "name": "meta-flow-package-imported" }),
        )
        .await
        .expect("metaagent should delete package-imported workflow-code artifact");
    assert!(
        deleted_package_import.ok,
        "{:?}",
        deleted_package_import.payload
    );
}
