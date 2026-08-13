use super::*;

#[test]
fn local_request_api_persists_workflow_code_artifacts() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code artifact local API test because node is not available");
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "chariox-workflow-code-artifact-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    std::fs::create_dir_all(workspace_root.join("schemas"))
        .expect("temporary schema directory should be created");
    let worktree_root = workspace_root.join("worktree");
    std::fs::create_dir_all(&worktree_root).expect("temporary worktree should be created");
    let schema_path = workspace_root.join("schemas/final.json");
    std::fs::write(
        &schema_path,
        r#"{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}},"additionalProperties":false}"#,
    )
    .expect("temporary schema file should be written");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                workspace_root.display().to_string(),
                worktree_root.display().to_string(),
            ),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let name = format!("toy-{}", crate::session::unix_epoch_ms());
    let source = r#"
workflow.define({ alias: "artifact_flow" })
const final = workflow.schemaFromFile({
  handle: "final",
  path: "schemas/final.json",
  alias: "Final output"
})
workflow.define({ runOutputSchema: final })
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "planner", provider: "codex", model: "gpt-5" }),
  instructions: "Plan."
})
workflow.queue({
  handle: "fast_lane",
  alias: "urgent",
  priority: 9,
  enabled: true
})
workflow.endpoint(planner, { handle: "entry", alias: "entry" })
"#;
    let updated_source = source.replace("artifact_flow", "artifact_flow_updated");

    let created = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: source.to_string(),
            },
        ))
        .expect("workflow-code artifact should create");
    match created {
        LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact } => {
            assert_eq!(artifact.metadata.name, name);
            assert!(artifact.metadata.validation.ok);
            assert!(artifact.metadata.path.starts_with(&workspace_root));
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("artifact_flow")
            );
            assert_eq!(
                artifact.definition.workflow.run_output_schema.as_deref(),
                Some("final")
            );
            assert_eq!(
                artifact.definition.schemas[0].schema["properties"]["answer"]["type"],
                "string"
            );
        }
        _ => panic!("unexpected local response"),
    }

    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflowCodeArtifacts(
            crate::local::ListWorkflowCodeArtifactsRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("workflow-code artifacts should list");
    match listed {
        LocalDaemonResponse::WorkflowCodeArtifactsListed { artifacts } => {
            assert!(artifacts.iter().any(|artifact| artifact.name == name));
        }
        _ => panic!("unexpected local response"),
    }

    let loaded = harness
        .dispatch(LocalDaemonRequest::GetWorkflowCodeArtifact(
            crate::local::GetWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
            },
        ))
        .expect("workflow-code artifact should load");
    match loaded {
        LocalDaemonResponse::WorkflowCodeArtifact { artifact } => {
            assert_eq!(artifact.source, source);
        }
        _ => panic!("unexpected local response"),
    }

    let updated = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCodeArtifact(
            crate::local::UpdateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: updated_source.clone(),
            },
        ))
        .expect("workflow-code artifact should update");
    match updated {
        LocalDaemonResponse::WorkflowCodeArtifactUpdated { artifact } => {
            assert_eq!(artifact.source, updated_source);
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
        }
        _ => panic!("unexpected local response"),
    }

    let package = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowCodeArtifact(
            crate::local::ExportWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
            },
        ))
        .expect("workflow-code artifact should export")
    {
        LocalDaemonResponse::WorkflowCodeArtifactExported { package } => {
            assert_eq!(package.name, name);
            assert_eq!(package.source, updated_source);
            assert_eq!(
                package.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
            assert_eq!(
                package.definition.schemas[0].schema["properties"]["answer"]["type"],
                "string"
            );
            package
        }
        _ => panic!("unexpected local response"),
    };

    let package_alias = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowCodePackage(
            crate::local::ExportWorkflowCodePackageRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
                target: None,
                agent_mode:
                    crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
            },
        ))
        .expect("workflow-code package alias should export")
    {
        LocalDaemonResponse::WorkflowCodePackageExported { package } => {
            assert_eq!(package.name, name);
            assert_eq!(package.source, updated_source);
            package
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(package_alias.source_sha256, package.source_sha256);
    assert_eq!(package_alias.definition_sha256, package.definition_sha256);

    let inline_source = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowCodeSource(
            crate::local::ExportWorkflowCodeSourceRequest {
                session_id: session.id().to_string(),
                target: crate::local::WorkflowCodeSourceExportTarget::Artifact {
                    name: name.clone(),
                },
                format: crate::workflow_code::WorkflowCodeSourceExportFormat::Inline,
                agent_mode:
                    crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
            },
        ))
        .expect("workflow-code inline source should export")
    {
        LocalDaemonResponse::WorkflowCodeSourceExported { export } => export,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(inline_source.name, name);
    assert_eq!(inline_source.source, updated_source);
    assert!(inline_source.files.is_empty());

    let directory_source = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowCodeSource(
            crate::local::ExportWorkflowCodeSourceRequest {
                session_id: session.id().to_string(),
                target: crate::local::WorkflowCodeSourceExportTarget::Artifact {
                    name: name.clone(),
                },
                format: crate::workflow_code::WorkflowCodeSourceExportFormat::Directory,
                agent_mode:
                    crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
            },
        ))
        .expect("workflow-code directory source should export")
    {
        LocalDaemonResponse::WorkflowCodeSourceExported { export } => export,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(directory_source.source_path, "workflow.js");
    assert!(directory_source
        .files
        .iter()
        .any(|file| file.path == "workflow.js"));
    assert!(directory_source
        .files
        .iter()
        .any(|file| file.path == "manifest.json"));
    assert!(directory_source
        .files
        .iter()
        .any(|file| file.path == "schemas/final-output.json"));
    let export_root = workspace_root.join("workflow-code-source-export");
    for file in &directory_source.files {
        let path = export_root.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("source export parent should create");
        }
        std::fs::write(path, &file.contents).expect("source export file should write");
    }
    let recompiled = crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
        &node_path,
        &directory_source.source,
        directory_source.language,
        &crate::config::WorkflowCodeLimitsConfig::default(),
        Some(&export_root),
    )
    .expect("directory workflow-code source export should recompile");
    assert!(recompiled.validation.ok);
    assert_eq!(
        recompiled.definition.workflow.alias.as_deref(),
        Some("artifact_flow_updated")
    );
    assert_eq!(
        recompiled.definition.workflow.run_output_schema.as_deref(),
        Some("final")
    );
    std::fs::remove_file(&schema_path)
        .expect("source schema file should be removable before portable import");

    let deleted = harness
        .dispatch(LocalDaemonRequest::DeleteWorkflowCodeArtifact(
            crate::local::DeleteWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.clone(),
            },
        ))
        .expect("workflow-code artifact should delete");
    match deleted {
        LocalDaemonResponse::WorkflowCodeArtifactDeleted {
            name: deleted,
            path,
        } => {
            assert_eq!(deleted, name);
            assert!(!path.exists());
        }
        _ => panic!("unexpected local response"),
    }

    let imported_name = format!("{name}-imported");
    let imported = harness
        .dispatch(LocalDaemonRequest::ImportWorkflowCodeArtifact(
            crate::local::ImportWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                package,
                name: Some(imported_name.clone()),
                overwrite: false,
                node_path: node_path.display().to_string(),
            },
        ))
        .expect("workflow-code artifact package should import");
    match imported {
        LocalDaemonResponse::WorkflowCodeArtifactImported { artifact } => {
            assert_eq!(artifact.metadata.name, imported_name);
            assert_eq!(artifact.source, updated_source);
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
            assert_eq!(
                artifact.definition.schemas[0].schema["properties"]["answer"]["type"],
                "string"
            );
            assert!(artifact.metadata.validation.ok);
        }
        _ => panic!("unexpected local response"),
    }

    let package_imported_name = format!("{name}-package-imported");
    let package_imported = harness
        .dispatch(LocalDaemonRequest::ImportWorkflowCodePackage(
            crate::local::ImportWorkflowCodePackageRequest {
                session_id: session.id().to_string(),
                package: package_alias,
                name: Some(package_imported_name.clone()),
                overwrite: false,
                node_path: node_path.display().to_string(),
            },
        ))
        .expect("workflow-code package alias should import");
    match package_imported {
        LocalDaemonResponse::WorkflowCodePackageImported { artifact } => {
            assert_eq!(artifact.metadata.name, package_imported_name);
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
            assert!(artifact.metadata.validation.ok);
        }
        _ => panic!("unexpected local response"),
    }

    let provider_rebindings = vec![crate::workflow_code::WorkflowCodeProviderRebinding {
        node: "planner".to_string(),
        provider: "dev-stub".to_string(),
        model: Some("default".to_string()),
        effort: None,
        account_profile: None,
    }];
    let applied = harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowCodeArtifact(
            crate::local::ApplyWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: imported_name.clone(),
                provider_rebindings: provider_rebindings.clone(),
                agent_rebindings: Vec::new(),
            },
        ))
        .expect("imported workflow-code artifact should apply with provider rebinding");
    match applied {
        LocalDaemonResponse::WorkflowCodeApplied {
            result,
            session: applied_session,
        } => {
            assert_eq!(
                result.apply.schema_refs.get("final").map(String::as_str),
                applied_session
                    .workflows()
                    .iter()
                    .find(|workflow| workflow.id() == result.apply.workflow_id)
                    .and_then(|workflow| workflow.run_output_schema_ref())
            );
            let planner_agent_id = result
                .apply
                .agent_ids
                .get("planner")
                .expect("planner agent id should be reported");
            assert!(applied_session.agents().iter().any(|agent| {
                agent.id() == planner_agent_id
                    && agent.provider() == "dev-stub"
                    && agent.model() == Some("default")
            }));
            let live_source = match harness
                .dispatch(LocalDaemonRequest::ExportWorkflowCodeSource(
                    crate::local::ExportWorkflowCodeSourceRequest {
                        session_id: session.id().to_string(),
                        target: crate::local::WorkflowCodeSourceExportTarget::Workflow {
                            workflow_ref: result.apply.workflow_id.clone(),
                        },
                        format: crate::workflow_code::WorkflowCodeSourceExportFormat::Inline,
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
                    },
                ))
                .expect("live workflow source should export")
            {
                LocalDaemonResponse::WorkflowCodeSourceExported { export } => export,
                _ => panic!("unexpected local response"),
            };
            assert_eq!(live_source.source_path, "workflow.js");
            assert!(live_source.source.contains("workflow.newAgent"));
            let live_recompiled =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    &node_path,
                    &live_source.source,
                    live_source.language,
                    &crate::config::WorkflowCodeLimitsConfig::default(),
                    None,
                )
                .expect("live workflow source export should recompile");
            assert!(live_recompiled.validation.ok);
            assert_eq!(
                live_recompiled.definition.workflow.alias.as_deref(),
                Some("artifact_flow_updated")
            );
            assert!(live_recompiled
                .definition
                .nodes
                .iter()
                .any(|node| node.handle == "planner"));
            assert!(live_recompiled
                .definition
                .endpoints
                .iter()
                .any(|endpoint| endpoint.handle == "entry" && endpoint.entry_node == "planner"));
            assert!(matches!(
                &live_recompiled.definition.nodes[0].agent,
                crate::workflow_code::WorkflowCodeAgentBinding::Create(agent)
                    if agent.provider == "dev-stub"
            ));
            let workflow_package = match harness
                .dispatch(LocalDaemonRequest::ExportWorkflowCodePackage(
                    crate::local::ExportWorkflowCodePackageRequest {
                        session_id: session.id().to_string(),
                        name: "workflow-package".to_string(),
                        target: Some(crate::local::WorkflowCodePackageExportTarget::Workflow {
                            workflow_ref: result.apply.workflow_id.clone(),
                        }),
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
                    },
                ))
                .expect("existing workflow should export as workflow-code package")
            {
                LocalDaemonResponse::WorkflowCodePackageExported { package } => package,
                _ => panic!("unexpected local response"),
            };
            assert_eq!(workflow_package.name, "workflow-package");
            assert!(workflow_package.source.contains("defineWorkflow"));
            workflow_package
                .validate_integrity()
                .expect("workflow package integrity should validate");
            let workflow_package_compile =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    &node_path,
                    &workflow_package.source,
                    workflow_package.language,
                    &crate::config::WorkflowCodeLimitsConfig::default(),
                    None,
                )
                .expect("workflow package source should recompile");
            assert!(workflow_package_compile.validation.ok);
            assert_eq!(
                workflow_package_compile
                    .definition
                    .workflow
                    .alias
                    .as_deref(),
                Some("artifact_flow_updated")
            );
            let live_existing_agent_source = match harness
                .dispatch(LocalDaemonRequest::ExportWorkflowCodeSource(
                    crate::local::ExportWorkflowCodeSourceRequest {
                        session_id: session.id().to_string(),
                        target: crate::local::WorkflowCodeSourceExportTarget::Workflow {
                            workflow_ref: result.apply.workflow_id.clone(),
                        },
                        format: crate::workflow_code::WorkflowCodeSourceExportFormat::Inline,
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::ExistingAgents,
                    },
                ))
                .expect("live workflow source should export with existing agents")
            {
                LocalDaemonResponse::WorkflowCodeSourceExported { export } => export,
                _ => panic!("unexpected local response"),
            };
            assert!(live_existing_agent_source
                .source
                .contains("workflow.existingAgent"));
            let live_existing_agent_recompiled =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    &node_path,
                    &live_existing_agent_source.source,
                    live_existing_agent_source.language,
                    &crate::config::WorkflowCodeLimitsConfig::default(),
                    None,
                )
                .expect("live workflow existing-agent source export should recompile");
            assert!(matches!(
                &live_existing_agent_recompiled.definition.nodes[0].agent,
                crate::workflow_code::WorkflowCodeAgentBinding::Existing(existing)
                    if existing.agent_ref == *planner_agent_id
            ));
        }
        _ => panic!("unexpected local response"),
    }

    let run = harness
        .dispatch(LocalDaemonRequest::RunWorkflowCodeArtifact(
            crate::local::RunWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: imported_name.clone(),
                provider_rebindings,
                agent_rebindings: Vec::new(),
                endpoint: Some("entry".to_string()),
                queue_ref: Some("fast_lane".to_string()),
                prompt: "Run the portable imported artifact.".to_string(),
            },
        ))
        .expect("imported workflow-code artifact should run with provider rebinding");
    let run_result = match run {
        LocalDaemonResponse::WorkflowCodeRun { result, session } => {
            let planner_agent_id = result
                .apply
                .apply
                .agent_ids
                .get("planner")
                .expect("planner run agent id should be reported");
            assert!(session.agents().iter().any(|agent| {
                agent.id() == planner_agent_id
                    && agent.provider() == "dev-stub"
                    && agent.model() == Some("default")
            }));
            let queue_id = result
                .apply
                .apply
                .queue_ids
                .get("fast_lane")
                .expect("script queue handle should map to a runtime queue id");
            assert!(session.workflow_prompt_queues().iter().any(|queue| {
                queue.id() == queue_id && queue.alias() == "urgent" && queue.priority() == 9
            }));
            result
        }
        _ => panic!("unexpected local response"),
    };
    assert!(matches!(
        run_result.invocation,
        crate::workflow_code::WorkflowCodeRunInvocation::Started { .. }
    ));

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_invalid_workflow_code_artifact_create_without_persisting() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping invalid workflow-code artifact create test because node is unavailable"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "chariox-workflow-code-invalid-create-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-invalid"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let invalid_source = r#"
workflow.define({ alias: "invalid_artifact" })
workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true
})
"#;
    let name = "invalid-create";

    let error = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: invalid_source.to_string(),
            },
        ))
        .expect_err("invalid workflow-code artifact should not create");

    assert!(format!("{error}").contains("missing_endpoint"));
    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflowCodeArtifacts(
            crate::local::ListWorkflowCodeArtifactsRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("workflow-code artifacts should list");
    match listed {
        LocalDaemonResponse::WorkflowCodeArtifactsListed { artifacts } => {
            assert!(!artifacts.iter().any(|artifact| artifact.name == name));
        }
        _ => panic!("unexpected local response"),
    }

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_invalid_workflow_code_artifact_update_without_overwriting() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!(
            "skipping invalid workflow-code artifact update test because node is unavailable"
        );
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "chariox-workflow-code-invalid-update-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-invalid"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let name = "invalid-update";
    let valid_source = r#"
workflow.define({ alias: "valid_artifact" })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;
    let invalid_source = valid_source.replace(
        r#"workflow.endpoint(worker, { handle: "entry", alias: "entry" })"#,
        "",
    );

    harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: valid_source.to_string(),
            },
        ))
        .expect("valid workflow-code artifact should create");
    let error = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCodeArtifact(
            crate::local::UpdateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: invalid_source,
            },
        ))
        .expect_err("invalid workflow-code artifact update should fail");

    assert!(format!("{error}").contains("missing_endpoint"));
    let loaded = harness
        .dispatch(LocalDaemonRequest::GetWorkflowCodeArtifact(
            crate::local::GetWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: name.to_string(),
            },
        ))
        .expect("workflow-code artifact should load");
    match loaded {
        LocalDaemonResponse::WorkflowCodeArtifact { artifact } => {
            assert_eq!(
                artifact.definition.workflow.alias.as_deref(),
                Some("valid_artifact")
            );
            assert_eq!(artifact.source, valid_source);
            assert!(artifact.metadata.validation.ok);
        }
        _ => panic!("unexpected local response"),
    }

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_rejects_invalid_workflow_code_artifact_import() {
    let workspace_root = std::env::temp_dir().join(format!(
        "chariox-workflow-code-invalid-import-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-invalid"),
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
            alias: Some("invalid_import".to_string()),
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
        endpoints: Vec::new(),
        queues: Vec::new(),
        schedules: Vec::new(),
    };
    let source = "workflow.define({ alias: \"invalid_import\" })";
    let package = crate::workflow_code::WorkflowCodeArtifactPackage {
        package_version: crate::workflow_code::WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION,
        name: "invalid-import".to_string(),
        language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
        source: source.to_string(),
        source_sha256: workflow_code_test_sha256_hex(source.as_bytes()),
        source_bytes: source.len() as u64,
        definition_sha256: crate::workflow_code::workflow_code_definition_sha256_hex(&definition),
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
        .expect_err("invalid workflow-code package should not import");

    assert!(format!("{error}").contains("missing_endpoint"));
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
                .any(|artifact| artifact.name == "invalid-import"));
        }
        _ => panic!("unexpected local response"),
    }

    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}
