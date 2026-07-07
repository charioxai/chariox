use super::*;
use crate::local::{
    CreateWorkflowPublicationRequest, CreateWorkflowScheduleRequest,
    ExportWorkflowPublicationPackageRequest, InstallSkillRequest, RegisterEnvironmentRequest,
    RegisterScriptRequest, RegisterWorkflowPublicationEndpointRequest,
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

mod workflow_code_apply_run;
mod workflow_code_artifact_imports;
mod workflow_code_artifact_persistence;
mod workflow_code_extensions_queues;
mod workflow_code_validation_limits;
mod workflow_graph_runtime;
mod workflow_publication_package;

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

fn find_python_for_workflow_code_local_api_test() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("PYTHON") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/python3"),
        PathBuf::from("/usr/local/bin/python3"),
        PathBuf::from("/usr/bin/python3"),
        PathBuf::from("python3"),
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

fn node_supports_workflow_code_typescript(node: &std::path::Path) -> bool {
    std::process::Command::new(node)
        .arg("--no-warnings")
        .arg("--input-type=module")
        .arg("-e")
        .arg("const mod = await import('node:module'); if (typeof mod.stripTypeScriptTypes !== 'function') process.exit(1)")
        .status()
        .is_ok_and(|status| status.success())
}

fn workflow_code_test_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct PublicationTestGraph {
    session_id: String,
    workflow_id: String,
    endpoint_id: String,
}

fn create_publication_test_graph(
    harness: &LocalRouterTestHarness,
    label: &str,
) -> PublicationTestGraph {
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(&format!("workspace-{label}"), &format!("worktree-{label}")),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some(format!("agent-{label}")),
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
        .expect("agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some(format!("workflow-{label}")),
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
                alias: Some(format!("endpoint-{label}")),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    PublicationTestGraph {
        session_id: session.id().to_string(),
        workflow_id: workflow.id().to_string(),
        endpoint_id: endpoint.id().to_string(),
    }
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
