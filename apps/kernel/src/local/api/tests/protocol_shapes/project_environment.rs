//! The Project Environment wire (local protocol 972): requests, views and the deleted requests.
use crate::local::*;
use serde_json::json;

fn view() -> ProjectEnvironmentView {
    ProjectEnvironmentView {
        project_id: "p".into(),
        exists: true,
        revision: 2,
        setup: ProjectEnvironmentSetupField {
            script: "mise install".into(),
        },
        variables: "LOG_LEVEL=debug".into(),
        secrets: vec![
            ProjectEnvironmentSecretView {
                name: "NOTION_TOKEN".into(),
                presence: crate::project_environment::SecretPresence::Missing,
            },
            ProjectEnvironmentSecretView {
                name: "DATABASE_URL".into(),
                presence: crate::project_environment::SecretPresence::Locked,
            },
        ],
        files: vec![
            ProjectEnvironmentFileView::new(
                "config/secrets.json".into(),
                crate::project_environment::FilePresence::Present { size_bytes: 2800 },
            ),
            ProjectEnvironmentFileView::new(
                ".env".into(),
                crate::project_environment::FilePresence::Missing,
            ),
        ],
        changed_at_ms: 7,
        machine_label: "b3 builder".into(),
        checkout_path: "/work/p".into(),
        setup_state: ProjectEnvironmentSetupState::Failed {
            operation_id: "op".into(),
            ended_at_ms: 9,
            exit_code: Some(1),
            reason: None,
        },
    }
}

#[test]
fn environment_requests_have_the_camel_case_wire_of_protocol_972() {
    let cases = [
        (
            LocalDaemonRequest::GetProjectEnvironment(GetProjectEnvironmentRequest {
                project_id: "p".into(),
                checkout_path: Some("/work/p".into()),
            }),
            json!({"GetProjectEnvironment": {"projectId": "p", "checkoutPath": "/work/p"}}),
        ),
        (
            LocalDaemonRequest::SaveProjectEnvironment(SaveProjectEnvironmentRequest {
                project_id: "p".into(),
                expected_revision: 1,
                setup: ProjectEnvironmentSetupField {
                    script: "make".into(),
                },
                variables: "A=1".into(),
                secrets: vec![ProjectEnvironmentSecretName { name: "T".into() }],
                files: vec![ProjectEnvironmentFilePath {
                    path: ".env".into(),
                }],
            }),
            json!({"SaveProjectEnvironment": {"projectId": "p", "expectedRevision": 1,
                "setup": {"script": "make"}, "variables": "A=1",
                "secrets": [{"name": "T"}], "files": [{"path": ".env"}]}}),
        ),
        (
            LocalDaemonRequest::SetProjectEnvironmentSecret(SetProjectEnvironmentSecretRequest {
                project_id: "p".into(),
                name: "T".into(),
                value: VaultPassphrase::new("synthetic"),
            }),
            json!({"SetProjectEnvironmentSecret": {"projectId": "p", "name": "T", "value": "synthetic"}}),
        ),
        (
            LocalDaemonRequest::RunProjectEnvironmentSetup(RunProjectEnvironmentSetupRequest {
                project_id: "p".into(),
                operation_id: "op".into(),
                checkout_path: None,
            }),
            json!({"RunProjectEnvironmentSetup": {"projectId": "p", "operationId": "op"}}),
        ),
        (
            LocalDaemonRequest::GetEnvironmentOperation(EnvironmentOperationRequest {
                operation_id: "op".into(),
            }),
            json!({"GetEnvironmentOperation": {"operationId": "op"}}),
        ),
        (
            LocalDaemonRequest::CancelEnvironmentOperation(EnvironmentOperationRequest {
                operation_id: "op".into(),
            }),
            json!({"CancelEnvironmentOperation": {"operationId": "op"}}),
        ),
    ];
    for (request, wire) in cases {
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<LocalDaemonRequest>(wire).unwrap(),
            request
        );
    }
}

#[test]
fn environment_views_carry_names_and_presence_never_a_value() {
    let wire = serde_json::to_value(LocalDaemonResponse::ProjectEnvironment {
        environment: view(),
    })
    .unwrap();
    assert_eq!(
        wire,
        json!({"ProjectEnvironment": {"environment": {
            "projectId": "p", "exists": true, "revision": 2,
            "setup": {"script": "mise install"}, "variables": "LOG_LEVEL=debug",
            "secrets": [{"name": "NOTION_TOKEN", "presence": "missing"},
                        {"name": "DATABASE_URL", "presence": "locked"}],
            "files": [{"path": "config/secrets.json", "presence": "present", "sizeBytes": 2800},
                      {"path": ".env", "presence": "missing", "sizeBytes": null}],
            "changedAtMs": 7, "machineLabel": "b3 builder", "checkoutPath": "/work/p",
            "setupState": {"state": "failed", "operationId": "op", "endedAtMs": 9, "exitCode": 1, "reason": null}
        }}})
    );
    for (state, wire) in [
        (
            ProjectEnvironmentSetupState::NotRun,
            json!({"state": "notRun"}),
        ),
        (
            ProjectEnvironmentSetupState::Ok {
                operation_id: "op".into(),
                ended_at_ms: 5,
            },
            json!({"state": "ok", "operationId": "op", "endedAtMs": 5}),
        ),
        (
            ProjectEnvironmentSetupState::Running {
                operation_id: "op".into(),
                started_at_ms: 3,
            },
            json!({"state": "running", "operationId": "op", "startedAtMs": 3}),
        ),
        (
            ProjectEnvironmentSetupState::Cancelled {
                operation_id: "op".into(),
                ended_at_ms: 6,
            },
            json!({"state": "cancelled", "operationId": "op", "endedAtMs": 6}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&state).unwrap(), wire);
    }
    let operation = EnvironmentOperationView {
        operation_id: "op".into(),
        project_id: "p".into(),
        checkout_path: "/work/p".into(),
        kind: EnvironmentOperationKind::Setup,
        state: EnvironmentOperationState::Running,
        started_at_ms: 1,
        ended_at_ms: None,
        exit_code: None,
        reason: None,
        output_tail: "line\n".into(),
    };
    assert_eq!(
        serde_json::to_value(LocalDaemonResponse::EnvironmentOperation { operation }).unwrap(),
        json!({"EnvironmentOperation": {"operation": {
            "operationId": "op", "projectId": "p", "checkoutPath": "/work/p", "kind": "setup",
            "state": "running", "startedAtMs": 1, "endedAtMs": null, "exitCode": null,
            "reason": null, "outputTail": "line\n"
        }}})
    );
}
