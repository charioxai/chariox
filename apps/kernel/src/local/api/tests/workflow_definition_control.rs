use super::*;
use crate::local::{
    CreateWorkflowPublicationRequest, ExportWorkflowPublicationPackageRequest,
    RegisterWorkflowPublicationEndpointRequest,
};
use base64::Engine;
use std::path::PathBuf;

fn find_node_for_workflow_code_local_api_test() -> Option<PathBuf> {
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
        std::process::Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

#[test]
fn local_request_api_validates_and_applies_workflow_code() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code local API test because node is not available");
        return;
    };
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-workflow-code", "worktree-workflow-code"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source = r#"
workflow.define({
  alias: "scripted_flow",
  flushAgentContextBeforeRun: true,
  maxConcurrent: 8
})
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "planner", provider: "dev-stub", model: "default" }),
  instructions: "Plan the task and produce a concise answer.",
  canCompleteWorkflowRun: true,
  canvas: { x: 20, y: 40 }
})
workflow.endpoint(planner, { handle: "entry", alias: "entry" })
"#;

    let validated = harness
        .dispatch(LocalDaemonRequest::ValidateWorkflowCode(
            crate::local::ValidateWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
            },
        ))
        .expect("workflow-code should validate");
    match validated {
        LocalDaemonResponse::WorkflowCodeValidated { result } => {
            assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
            assert_eq!(
                result.definition.workflow.alias.as_deref(),
                Some("scripted_flow")
            );
        }
        _ => panic!("unexpected local response"),
    }
    let listed = harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed");
    match listed {
        LocalDaemonResponse::WorkflowsListed { workflows } => assert!(workflows.is_empty()),
        _ => panic!("unexpected local response"),
    }

    let applied = harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowCode(
            crate::local::ApplyWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
            },
        ))
        .expect("workflow-code should apply");
    match applied {
        LocalDaemonResponse::WorkflowCodeApplied { result, session } => {
            assert!(result.compile.validation.ok);
            assert_eq!(result.apply.node_ids.len(), 1);
            assert_eq!(result.apply.agent_ids.len(), 1);
            assert!(session
                .workflows()
                .iter()
                .any(|workflow| workflow.id() == result.apply.workflow_id));
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_persists_workflow_code_artifacts() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code artifact local API test because node is not available");
        return;
    };
    let workspace_root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-artifact-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace_root).expect("temporary workspace should be created");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(workspace_root.display().to_string(), "worktree-artifact"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let name = format!("toy-{}", crate::session::unix_epoch_ms());
    let source = r#"
workflow.define({ alias: "artifact_flow" })
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "planner", provider: "dev-stub", model: "default" }),
  instructions: "Plan."
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
    std::fs::remove_dir_all(&workspace_root).expect("temporary workspace should be removed");
}

#[test]
fn local_request_api_exports_agent_app_publication_package() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-agent-app", "worktree-agent-app"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("shopper".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("shopping".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("add".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                queue_ref: Some("default".to_string()),
                alias: Some("shopping-app".to_string()),
                route: Some("/add/*".to_string()),
                methods: vec!["GET".to_string()],
                transport: Some(serde_json::json!({ "kind": "human_http" })),
                parser: Some(serde_json::json!({
                    "kind": "regex",
                    "source": "path",
                    "pattern": "^/add/(?<prompt>.+)$"
                })),
                input_schema: None,
                trace_exposure: None,
                mode: Some("async".to_string()),
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("workflow publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    let assets_root = std::env::temp_dir().join(format!(
        "arroba-agent-app-assets-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(assets_root.join("assets")).expect("asset directory should be created");
    std::fs::write(
        assets_root.join("index.html"),
        "<!doctype html><main>shop</main>",
    )
    .expect("index asset should write");
    std::fs::write(assets_root.join("assets/catalog.json"), "{\"items\":[]}")
        .expect("nested asset should write");

    let exported = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: session.id().to_string(),
                publication_ref: publication.id().to_string(),
                kernel_url: Some("ws://127.0.0.1:43118".to_string()),
                agent_app: Some(serde_json::json!({
                    "enabled": true,
                    "assets": {
                        "public_dir": "app",
                        "index": "index.html"
                    },
                    "routes": [{
                        "path": "/add/*",
                        "hook_id": format!("{}-hook", publication.id()),
                        "prompt_source": "path_tail",
                        "response": "streaming_shell",
                        "required_role": "public",
                        "manipulation": {
                            "level": "state_and_overlay",
                            "scope": "session",
                            "allowed_actions": ["cart.search", "cart.add"]
                        }
                    }],
                    "replicas": {
                        "count": 1,
                        "per_caller_ordering": true
                    },
                    "persistent_patch": {
                        "enabled": false
                    }
                })),
                agent_app_assets_dir: Some(assets_root.to_string_lossy().to_string()),
            },
        ))
        .expect("agent app publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_version,
            package_files,
            ..
        } => {
            assert_eq!(package_version, 2);
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let publication_json = package_json_file(&exported, "publication.json");
    assert_eq!(publication_json["package_version"], serde_json::json!(2));
    assert_eq!(
        publication_json["agent_app"]["routes"][0]["path"],
        serde_json::json!("/add/*")
    );
    assert_eq!(
        package_text_file(&exported, "app/index.html"),
        "<!doctype html><main>shop</main>"
    );
    assert_eq!(
        package_text_file(&exported, "app/assets/catalog.json"),
        "{\"items\":[]}"
    );
    std::fs::remove_dir_all(assets_root).expect("asset directory should clean up");
}

#[test]
fn workflow_node_add_rejects_metaagents() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-meta-workflow", "worktree-meta-workflow"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let metaagent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("meta".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: true,
        }))
        .expect("metaagent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("graph".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let error = harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: metaagent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect_err("metaagent workflow node should be rejected");
    assert!(
        error
            .to_string()
            .contains("metaagents cannot be added as workflow nodes"),
        "unexpected error: {error}"
    );
}

fn package_text_file(files: &[crate::local::WorkflowPublicationPackageFile], path: &str) -> String {
    let file = files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("package file `{path}` should exist"));
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&file.content_base64)
        .expect("package file should decode");
    String::from_utf8(bytes).expect("package file should be UTF-8")
}

fn package_json_file(
    files: &[crate::local::WorkflowPublicationPackageFile],
    path: &str,
) -> serde_json::Value {
    serde_json::from_str(&package_text_file(files, path)).expect("package JSON should parse")
}

#[test]
fn local_request_api_manages_workflows_endpoints_and_graph_edits() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("review".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("workflow list should succeed")
    {
        LocalDaemonResponse::WorkflowsListed { workflows } => workflows,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(listed.len(), 1);

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveWorkflow(
            ResolveWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: "review".to_string(),
            },
        ))
        .expect("workflow resolve should succeed")
    {
        LocalDaemonResponse::WorkflowResolved { workflow } => workflow,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(resolved.id(), workflow.id());

    let node_a = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("first workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    let duplicate_node = harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect_err("duplicate workflow node should be rejected");
    assert!(matches!(
        duplicate_node,
        DaemonError::WorkflowNodeConflict { .. }
    ));

    match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: node_a.id().to_string(),
                instructions: Some("You are the reviewer.".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node instructions should update")
    {
        LocalDaemonResponse::WorkflowNodeInstructionsUpdated { node, .. } => {
            assert_eq!(node.instructions(), Some("You are the reviewer."));
        }
        _ => panic!("unexpected local response"),
    };

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer-2".to_string()),
            provider: Some("opencode".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let node_b = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: spawned.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("second workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };

    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node_a.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(endpoint.entry_node_id(), node_a.id());

    let aliased_workflow = match harness
        .dispatch(LocalDaemonRequest::AliasWorkflow(AliasWorkflowRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            alias: "qa".to_string(),
            expected_workflow_revision: None,
        }))
        .expect("workflow alias should succeed")
    {
        LocalDaemonResponse::WorkflowAliased { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased_workflow.alias(), Some("qa"));

    let aliased_endpoint = match harness
        .dispatch(LocalDaemonRequest::AliasWorkflowEndpoint(
            AliasWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                alias: "start".to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint alias should succeed")
    {
        LocalDaemonResponse::WorkflowEndpointAliased { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased_endpoint.alias(), Some("start"));

    let edge = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowEdge(
            AddWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                from_node_id: node_a.id().to_string(),
                to_node_id: node_b.id().to_string(),
                handoff_schema_ref: None,
                validation_policy: None,
                expected_workflow_revision: None,
                source_side: Some(crate::session::WorkflowEdgeEndpointSide::Right),
                target_side: Some(crate::session::WorkflowEdgeEndpointSide::Left),
            },
        ))
        .expect("workflow edge should be added")
    {
        LocalDaemonResponse::WorkflowEdgeAdded { edge, .. } => edge,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        edge.source_side(),
        Some(crate::session::WorkflowEdgeEndpointSide::Right)
    );
    assert_eq!(
        edge.target_side(),
        Some(crate::session::WorkflowEdgeEndpointSide::Left)
    );

    match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCanvasLayout(
            UpdateWorkflowCanvasLayoutRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                base_layout_revision: None,
                patches: vec![
                    crate::session::WorkflowCanvasLayoutPatch::NodePosition {
                        node_id: node_a.id().to_string(),
                        x: 120,
                        y: 80,
                    },
                    crate::session::WorkflowCanvasLayoutPatch::NodePosition {
                        node_id: node_b.id().to_string(),
                        x: 420,
                        y: 80,
                    },
                    crate::session::WorkflowCanvasLayoutPatch::EndpointPosition {
                        endpoint_id: endpoint.id().to_string(),
                        x: 180,
                        y: 36,
                    },
                ],
            },
        ))
        .expect("workflow canvas layout should update")
    {
        LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout, workflow, ..
        } => {
            assert_eq!(layout.revision, 1);
            assert_eq!(
                layout.nodes.get(node_a.id()).map(|point| point.x),
                Some(120)
            );
            assert_eq!(
                workflow
                    .canvas_layout()
                    .and_then(|stored| stored.endpoints.get(endpoint.id()))
                    .map(|point| point.y),
                Some(36)
            );
        }
        _ => panic!("unexpected local response"),
    }

    match harness
        .dispatch(LocalDaemonRequest::RemoveWorkflowEdge(
            RemoveWorkflowEdgeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                edge_id: edge.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow edge should be removed")
    {
        LocalDaemonResponse::WorkflowEdgeRemoved { .. } => {}
        _ => panic!("unexpected local response"),
    }

    match harness
        .dispatch(LocalDaemonRequest::RemoveWorkflowNode(
            RemoveWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                node_id: node_a.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be removed")
    {
        LocalDaemonResponse::WorkflowNodeRemoved { .. } => {}
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn local_request_api_materializes_workflow_publication_as_hidden_runtime_session() {
    let harness = LocalRouterTestHarness::new();
    let source_session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("source session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: source_session.id().to_string(),
            alias: Some("published_worker".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("source workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: source_session.id().to_string(),
            alias: Some("publishable".to_string()),
        }))
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: source_session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: source_agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let (workflow, endpoint) = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: source_session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("entry".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated {
            workflow, endpoint, ..
        } => (workflow, endpoint),
        _ => panic!("unexpected local response"),
    };
    let source_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: source_session.id().to_string(),
            },
        ))
        .expect("source state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let source_queue = source_state
        .workflow_prompt_queues()
        .iter()
        .find(|queue| queue.workflow_id() == workflow.id() && queue.alias() == "default")
        .expect("source workflow should have a default queue")
        .clone();
    let source_watchdog = crate::session::WorkflowWatchdogDefinition::new(
        "watchdog-1",
        workflow.id(),
        endpoint.id(),
        60,
        "publication watchdog",
        crate::session::WorkflowWatchdogPolicy::Queue,
        Some(1),
    );
    match harness
        .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: source_session.id().to_string(),
            workspace_id: None,
        }))
        .expect("source session should be deletable before materialization")
    {
        LocalDaemonResponse::SessionDeleted { .. } => {}
        _ => panic!("unexpected local response"),
    };

    let runtime_owner_user_id = "published-runtime-user";
    let materialized = match harness
        .dispatch_as_user(
            runtime_owner_user_id,
            LocalDaemonRequest::MaterializeWorkflowPublication(
                MaterializeWorkflowPublicationRequest {
                    publication_id: "publication-1".to_string(),
                    snapshot: WorkflowPublicationSnapshot {
                        schema_version: 1,
                        captured_at_ms: Some(42),
                        source_session: Some(WorkflowPublicationSourceSessionSnapshot {
                            id: Some(source_session.id().to_string()),
                            alias: source_session.alias().map(str::to_string),
                            workspace_id: source_session.workspace_id().to_string(),
                            worktree_id: source_session.worktree_id().to_string(),
                        }),
                        workflow: workflow.clone(),
                        endpoint: Some(endpoint.clone()),
                        queues: vec![source_queue],
                        watchdogs: vec![source_watchdog.clone()],
                        agents: vec![source_agent.clone()],
                    },
                },
            ),
        )
        .expect("publication should materialize")
    {
        LocalDaemonResponse::WorkflowPublicationMaterialized {
            session,
            agent_id_map,
            ..
        } => {
            assert_eq!(
                agent_id_map.get(source_agent.id()).map(String::as_str),
                session
                    .workflows()
                    .first()
                    .and_then(|workflow| workflow.nodes().first())
                    .map(|node| node.agent_id())
            );
            session
        }
        _ => panic!("unexpected local response"),
    };
    assert!(materialized.is_hidden());
    assert_eq!(materialized.owner_user_id(), runtime_owner_user_id);
    assert!(materialized.has_member(runtime_owner_user_id));
    assert_ne!(materialized.id(), source_session.id());
    assert_eq!(materialized.workflows().len(), 1);
    assert_eq!(materialized.workflow_publications().len(), 1);
    assert_eq!(
        materialized.workflow_publications()[0].id(),
        "publication-1"
    );
    assert_eq!(
        materialized.workflow_publications()[0].endpoint_id(),
        endpoint.id()
    );
    assert_eq!(materialized.workflow_watchdogs().len(), 1);
    assert_eq!(
        materialized.workflow_watchdogs()[0].invocation_prompt(),
        source_watchdog.invocation_prompt()
    );
    assert_eq!(
        materialized.workflow_watchdogs()[0].endpoint_id(),
        endpoint.id()
    );
    assert_eq!(materialized.agents().len(), 1);
    assert_ne!(materialized.agents()[0].id(), source_agent.id());
    assert_eq!(
        materialized.agents()[0].owner_user_id(),
        runtime_owner_user_id
    );
    let materialized_workflow = materialized
        .workflows()
        .first()
        .expect("materialized workflow should exist");
    assert_eq!(
        materialized_workflow.nodes()[0].owner_user_id(),
        runtime_owner_user_id
    );
    assert_eq!(
        materialized_workflow.nodes()[0].created_by_user_id(),
        runtime_owner_user_id
    );
    assert_eq!(
        materialized_workflow.endpoints()[0].owner_user_id(),
        runtime_owner_user_id
    );

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListSessions(ListSessionsRequest))
        .expect("list sessions should succeed")
    {
        LocalDaemonResponse::SessionsListed { sessions } => sessions,
        _ => panic!("unexpected local response"),
    };
    assert!(listed.is_empty());

    let hidden_state = match harness
        .dispatch_as_user(
            runtime_owner_user_id,
            LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                session_id: materialized.id().to_string(),
            }),
        )
        .expect("hidden runtime session should still load by id")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert!(hidden_state.is_hidden());

    match harness
        .dispatch_as_user(
            runtime_owner_user_id,
            LocalDaemonRequest::RegisterWorkflowPublicationEndpoint(
                RegisterWorkflowPublicationEndpointRequest {
                    session_id: materialized.id().to_string(),
                    publication_ref: "publication-1".to_string(),
                    local_url: "http://127.0.0.1:3000/".to_string(),
                    runtime_session_id: Some(materialized.id().to_string()),
                    ttl_ms: None,
                },
            ),
        )
        .expect("materialized publication endpoint should register")
    {
        LocalDaemonResponse::WorkflowPublicationEndpointRegistered {
            publication,
            open_url,
            ..
        } => {
            assert_eq!(publication.id(), "publication-1");
            assert_eq!(publication.open_url(), Some(open_url.as_str()));
        }
        _ => panic!("unexpected local response"),
    }
}
