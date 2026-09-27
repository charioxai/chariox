use super::*;

use crate::local::{
    CancelProjectEnvironmentSetupRequest, GetProjectEnvironmentSetupStatusRequest,
    ProjectEnvironmentCommandResult, ProjectEnvironmentDefinition,
    ProjectEnvironmentDefinitionOrigin, ProjectEnvironmentDefinitionSource,
    ProjectEnvironmentInput, ProjectEnvironmentInputKind, ProjectEnvironmentPathBase,
    ProjectEnvironmentPathEntry, ProjectEnvironmentSetupPhase, ProjectEnvironmentSetupStatus,
    ProjectEnvironmentSetupStep, ProjectEnvironmentSetupStepKind, ProjectEnvironmentValidation,
    RetryProjectEnvironmentSetupRequest, StartProjectEnvironmentSetupRequest,
    LOCAL_DAEMON_PROTOCOL_VERSION,
};

fn definition() -> ProjectEnvironmentDefinition {
    ProjectEnvironmentDefinition {
        schema_version: 1,
        origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
        source: ProjectEnvironmentDefinitionSource::Devcontainer,
        target_platform: "linux-x86_64".to_string(),
        source_path: Some(".devcontainer/devcontainer.json".to_string()),
        inputs: vec![
            ProjectEnvironmentInput {
                kind: ProjectEnvironmentInputKind::Recipe,
                path: ".devcontainer/devcontainer.json".to_string(),
                sha256: format!("sha256:{}", "a".repeat(64)),
            },
            ProjectEnvironmentInput {
                kind: ProjectEnvironmentInputKind::Lockfile,
                path: "Cargo.lock".to_string(),
                sha256: format!("sha256:{}", "b".repeat(64)),
            },
        ],
        path_entries: vec![ProjectEnvironmentPathEntry {
            base: ProjectEnvironmentPathBase::PreparationHome,
            path: "go/bin".to_string(),
        }],
        setup_steps: vec![ProjectEnvironmentSetupStep {
            kind: ProjectEnvironmentSetupStepKind::NativeDependency,
            command: "apt-get install -y pkg-config libssl-dev".to_string(),
        }],
        validation_commands: vec!["timeout 180s cargo check --workspace --locked".to_string()],
    }
}

fn status() -> ProjectEnvironmentSetupStatus {
    ProjectEnvironmentSetupStatus {
        operation_id: "setup-1".to_string(),
        project_id: "project-1".to_string(),
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        worker_id: "machine-1".to_string(),
        platform: "linux-x86_64".to_string(),
        phase: ProjectEnvironmentSetupPhase::Ready,
        attempt: 1,
        progress_percent: 100,
        definition_digest: Some("sha256:definition".to_string()),
        validation: Some(ProjectEnvironmentValidation {
            worker_id: "machine-1".to_string(),
            platform: "linux-x86_64".to_string(),
            commands: vec![ProjectEnvironmentCommandResult {
                command_digest: "sha256:check".to_string(),
                exit_code: 0,
                stdout_bytes: 0,
                stderr_bytes: 0,
            }],
        }),
        message: Some("ready".to_string()),
        failure_code: None,
        failure_message: None,
        retryable: false,
        created_at_ms: 10,
        updated_at_ms: 20,
    }
}

#[test]
fn project_environment_setup_protocol_shape_is_versioned_and_explicit() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 351);
    let start =
        LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
            operation_id: "setup-1".to_string(),
            project_id: "project-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            target_worker_id: "machine-1".to_string(),
            target_platform: "linux-x86_64".to_string(),
            definition: Some(definition()),
            validation_commands: Vec::new(),
        });
    assert_eq!(
        serde_json::to_value(start).expect("setup start should encode"),
        serde_json::json!({
            "StartProjectEnvironmentSetup": {
                "operationId": "setup-1",
                "projectId": "project-1",
                "sessionId": "session-1",
                "agentId": "agent-1",
                "targetWorkerId": "machine-1",
                "targetPlatform": "linux-x86_64",
                "definition": {
                    "schema_version": 1,
                    "origin": "user_authored",
                    "source": "devcontainer",
                    "target_platform": "linux-x86_64",
                    "source_path": ".devcontainer/devcontainer.json",
                    "inputs": [{
                        "kind": "recipe",
                        "path": ".devcontainer/devcontainer.json",
                        "sha256": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    }, {
                        "kind": "lockfile",
                        "path": "Cargo.lock",
                        "sha256": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    }],
                    "path_entries": [{
                        "base": "preparation_home",
                        "path": "go/bin"
                    }],
                    "setup_steps": [{
                        "kind": "native_dependency",
                        "command": "apt-get install -y pkg-config libssl-dev"
                    }],
                    "validation_commands": ["timeout 180s cargo check --workspace --locked"]
                }
            }
        })
    );
    let response = LocalDaemonResponse::ProjectEnvironmentSetupStatus { status: status() };
    assert_eq!(
        serde_json::to_value(response).expect("setup status should encode"),
        serde_json::json!({
            "ProjectEnvironmentSetupStatus": {
                "status": {
                    "operation_id": "setup-1",
                    "project_id": "project-1",
                    "session_id": "session-1",
                    "agent_id": "agent-1",
                    "worker_id": "machine-1",
                    "platform": "linux-x86_64",
                    "phase": "ready",
                    "attempt": 1,
                    "progress_percent": 100,
                    "definition_digest": "sha256:definition",
                    "validation": {
                    "worker_id": "machine-1",
                    "platform": "linux-x86_64",
                    "commands": [{
                        "command_digest": "sha256:check",
                        "exit_code": 0,
                        "stdout_bytes": 0,
                        "stderr_bytes": 0
                    }]
                    },
                    "message": "ready",
                    "failure_code": null,
                    "failure_message": null,
                    "retryable": false,
                    "created_at_ms": 10,
                    "updated_at_ms": 20
                }
            }
        })
    );
    let mut controls = vec![
        LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
            GetProjectEnvironmentSetupStatusRequest {
                operation_id: "setup-1".to_string(),
            },
        ),
        LocalDaemonRequest::CancelProjectEnvironmentSetup(CancelProjectEnvironmentSetupRequest {
            operation_id: "setup-1".to_string(),
            session_id: "session-1".to_string(),
        }),
        LocalDaemonRequest::RetryProjectEnvironmentSetup(RetryProjectEnvironmentSetupRequest {
            operation_id: "setup-1".to_string(),
            session_id: "session-1".to_string(),
        }),
    ];
    assert_eq!(
        controls
            .drain(..)
            .map(|request| serde_json::to_value(request).expect("setup control should encode"))
            .collect::<Vec<_>>(),
        vec![
            serde_json::json!({"GetProjectEnvironmentSetupStatus": {"operationId": "setup-1"}}),
            serde_json::json!({"CancelProjectEnvironmentSetup": {"operationId": "setup-1", "sessionId": "session-1"}}),
            serde_json::json!({"RetryProjectEnvironmentSetup": {"operationId": "setup-1", "sessionId": "session-1"}}),
        ]
    );
}
