//! The Project Environment wire (local protocol 982): requests, views and the deleted requests.
use crate::local::*;
use crate::project_environment::ExtensionSource;
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
        mcps: vec![
            ProjectEnvironmentExtensionView {
                name: "github".into(),
                source: ExtensionSource::File {
                    path: ".mcp.json".into(),
                },
                presence: ExtensionPresence::Present,
            },
            ProjectEnvironmentExtensionView {
                name: "notion".into(),
                source: ExtensionSource::Registration,
                presence: ExtensionPresence::Missing,
            },
        ],
        skills: vec![ProjectEnvironmentExtensionView {
            name: "dataviz".into(),
            source: ExtensionSource::File {
                path: ".claude/skills/dataviz/SKILL.md".into(),
            },
            presence: ExtensionPresence::Present,
        }],
        changed_at_ms: 7,
        machine_label: "b3 builder".into(),
        checkout_path: "/work/p".into(),
        setup_state: ProjectEnvironmentSetupState::Failed {
            operation_id: "op".into(),
            ended_at_ms: 9,
            exit_code: Some(1),
            reason: None,
        },
        proposals: vec![EnvironmentProposal {
            id: "0123456789abcdef".into(),
            field: EnvironmentProposalField::Secret,
            change: EnvironmentProposalChange::Add,
            text: "STRIPE_SECRET_KEY".into(),
            evidence: EnvironmentProposalEvidence {
                path: "lib/billing.ts".into(),
                excerpt: "process.env.STRIPE_SECRET_KEY".into(),
            },
        }],
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
                mcps: vec![
                    ProjectEnvironmentExtension {
                        name: "github".into(),
                        source: ExtensionSource::File {
                            path: ".mcp.json".into(),
                        },
                    },
                    ProjectEnvironmentExtension {
                        name: "notion".into(),
                        source: ExtensionSource::Registration,
                    },
                ],
                skills: vec![],
            }),
            json!({"SaveProjectEnvironment": {"projectId": "p", "expectedRevision": 1,
                "setup": {"script": "make"}, "variables": "A=1",
                "secrets": [{"name": "T"}], "files": [{"path": ".env"}],
                "mcps": [{"name": "github", "source": {"kind": "file", "path": ".mcp.json"}}, {"name": "notion", "source": {"kind": "registration"}}], "skills": []}}),
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
            LocalDaemonRequest::ImportProjectEnvironmentEntries(
                ImportProjectEnvironmentEntriesRequest {
                    project_id: "p".into(),
                    operation_id: "op".into(),
                    source_kernel_id: "kernel-a".into(),
                    entries: Some(vec![EnvironmentEntryRef {
                        kind: EnvironmentEntryKind::Secret,
                        name: "NOTION_TOKEN".into(),
                    }]),
                    checkout_path: None,
                },
            ),
            json!({"ImportProjectEnvironmentEntries": {"projectId": "p", "operationId": "op",
                "sourceKernelId": "kernel-a", "entries": [{"kind": "secret", "name": "NOTION_TOKEN"}]}}),
        ),
        (
            LocalDaemonRequest::ImportProjectEnvironmentEntries(
                ImportProjectEnvironmentEntriesRequest {
                    project_id: "p".into(),
                    operation_id: "op".into(),
                    source_kernel_id: "kernel-a".into(),
                    entries: None,
                    checkout_path: Some("/work/p".into()),
                },
            ),
            json!({"ImportProjectEnvironmentEntries": {"projectId": "p", "operationId": "op",
                "sourceKernelId": "kernel-a", "checkoutPath": "/work/p"}}),
        ),
        (
            LocalDaemonRequest::DraftProjectEnvironment(DraftProjectEnvironmentRequest {
                project_id: "p".into(),
                operation_id: "op".into(),
            }),
            json!({"DraftProjectEnvironment": {"projectId": "p", "operationId": "op"}}),
        ),
        (
            LocalDaemonRequest::RunRemoteProjectEnvironmentSetup(
                RunRemoteProjectEnvironmentSetupRequest {
                    project_id: "p".into(),
                    operation_id: "op".into(),
                    target_kernel_id: "kernel-b".into(),
                    checkout_path: Some("/work/p".into()),
                },
            ),
            json!({"RunRemoteProjectEnvironmentSetup": {"projectId": "p", "operationId": "op",
                "targetKernelId": "kernel-b", "checkoutPath": "/work/p"}}),
        ),
        (
            LocalDaemonRequest::RemoteEnvironmentOperation(RemoteEnvironmentOperationRequest {
                operation_id: "op".into(),
                target_kernel_id: "kernel-b".into(),
                stop: true,
            }),
            json!({"RemoteEnvironmentOperation": {"operationId": "op",
                "targetKernelId": "kernel-b", "stop": true}}),
        ),
        (
            LocalDaemonRequest::RemoteEnvironmentOperation(RemoteEnvironmentOperationRequest {
                operation_id: "op".into(),
                target_kernel_id: "kernel-b".into(),
                stop: false,
            }),
            json!({"RemoteEnvironmentOperation": {"operationId": "op", "targetKernelId": "kernel-b"}}),
        ),
        (
            LocalDaemonRequest::CheckProjectEnvironment(CheckProjectEnvironmentRequest {
                project_id: "p".into(),
                operation_id: "op".into(),
                with_agent: false,
            }),
            json!({"CheckProjectEnvironment": {"projectId": "p", "operationId": "op", "withAgent": false}}),
        ),
        (
            LocalDaemonRequest::ResolveEnvironmentProposal(ResolveEnvironmentProposalRequest {
                project_id: "p".into(),
                proposal_id: "0123456789abcdef".into(),
                accept: false,
            }),
            json!({"ResolveEnvironmentProposal": {"projectId": "p", "proposalId": "0123456789abcdef", "accept": false}}),
        ),
        (
            LocalDaemonRequest::GetEnvironmentOperation(GetEnvironmentOperationRequest {
                operation_id: "op".into(),
                wait_after_updates: None,
            }),
            json!({"GetEnvironmentOperation": {"operationId": "op"}}),
        ),
        (
            LocalDaemonRequest::GetEnvironmentOperation(GetEnvironmentOperationRequest {
                operation_id: "op".into(),
                wait_after_updates: Some(3),
            }),
            json!({"GetEnvironmentOperation": {"operationId": "op", "waitAfterUpdates": 3}}),
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
            "mcps": [{"name": "github", "source": {"kind": "file", "path": ".mcp.json"}, "presence": "present"},
                     {"name": "notion", "source": {"kind": "registration"}, "presence": "missing"}],
            "skills": [{"name": "dataviz", "source": {"kind": "file", "path": ".claude/skills/dataviz/SKILL.md"}, "presence": "present"}],
            "changedAtMs": 7, "machineLabel": "b3 builder", "checkoutPath": "/work/p",
            "setupState": {"state": "failed", "operationId": "op", "endedAtMs": 9, "exitCode": 1, "reason": null},
            "proposals": [{"id": "0123456789abcdef", "field": "secret", "change": "add", "text": "STRIPE_SECRET_KEY",
                "evidence": {"path": "lib/billing.ts", "excerpt": "process.env.STRIPE_SECRET_KEY"}}]
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
        updates: 0,
        output_tail: "line\n".into(),
        entries: Vec::new(),
        source_label: None,
    };
    assert_eq!(
        serde_json::to_value(LocalDaemonResponse::EnvironmentOperation { operation }).unwrap(),
        json!({"EnvironmentOperation": {"operation": {
            "operationId": "op", "projectId": "p", "checkoutPath": "/work/p", "kind": "setup",
            "state": "running", "startedAtMs": 1, "endedAtMs": null, "exitCode": null,
            "reason": null, "updates": 0, "outputTail": "line\n", "entries": [], "sourceLabel": null
        }}})
    );
    let import = EnvironmentOperationView {
        kind: EnvironmentOperationKind::Import,
        entries: vec![
            EnvironmentOperationEntry {
                kind: EnvironmentEntryKind::File,
                name: "config/secrets.json".into(),
                state: EnvironmentOperationEntryState::Imported,
                reason: None,
            },
            EnvironmentOperationEntry {
                kind: EnvironmentEntryKind::Secret,
                name: "NOTION_TOKEN".into(),
                state: EnvironmentOperationEntryState::Failed,
                reason: Some("Source offline".into()),
            },
        ],
        source_label: Some("Miguel's Mac".into()),
        ..EnvironmentOperationView {
            operation_id: "op".into(),
            project_id: "p".into(),
            checkout_path: "/work/p".into(),
            kind: EnvironmentOperationKind::Draft,
            state: EnvironmentOperationState::Failed,
            started_at_ms: 1,
            ended_at_ms: Some(2),
            exit_code: None,
            reason: Some("Source offline".into()),
            updates: 0,
            output_tail: String::new(),
            entries: Vec::new(),
            source_label: None,
        }
    };
    assert_eq!(
        serde_json::to_value(&import).unwrap(),
        json!({
            "operationId": "op", "projectId": "p", "checkoutPath": "/work/p", "kind": "import",
            "state": "failed", "startedAtMs": 1, "endedAtMs": 2, "exitCode": null,
            "reason": "Source offline", "updates": 0, "outputTail": "",
            "entries": [
                {"kind": "file", "name": "config/secrets.json", "state": "imported", "reason": null},
                {"kind": "secret", "name": "NOTION_TOKEN", "state": "failed", "reason": "Source offline"}
            ],
            "sourceLabel": "Miguel's Mac"
        })
    );
    assert_eq!(
        serde_json::to_value(EnvironmentOperationKind::Draft).unwrap(),
        json!("draft")
    );
}

/// The kernel-to-kernel requests that carry an Environment (relay peer 422): the wire, and that no
/// Debug output of a response can show a Secret value or a file's bytes.
#[test]
fn project_environment_peer_wire_and_redaction() {
    use crate::transport::relay_peer::{
        RelayPeerRequest, RelayPeerResponse, RemoteCredentialSecretInput, RemoteFileBytes,
    };
    for (request, wire) in [
        (
            RelayPeerRequest::RunProjectEnvironmentSetup {
                project_id: "p".into(),
                operation_id: "op".into(),
                checkout_path: None,
            },
            json!({"kind": "run_project_environment_setup", "project_id": "p", "operation_id": "op", "checkout_path": null}),
        ),
        (
            RelayPeerRequest::GetProjectEnvironmentOperation {
                operation_id: "op".into(),
            },
            json!({"kind": "get_project_environment_operation", "operation_id": "op"}),
        ),
        (
            RelayPeerRequest::CancelProjectEnvironmentOperation {
                operation_id: "op".into(),
            },
            json!({"kind": "cancel_project_environment_operation", "operation_id": "op"}),
        ),
        (
            RelayPeerRequest::ReadProjectEnvironment {
                project_id: "p".into(),
            },
            json!({"kind": "read_project_environment", "project_id": "p"}),
        ),
        (
            RelayPeerRequest::PullProjectEnvironmentSecret {
                project_id: "p".into(),
                name: "T".into(),
            },
            json!({"kind": "pull_project_environment_secret", "project_id": "p", "name": "T"}),
        ),
        (
            RelayPeerRequest::PullProjectEnvironmentFile {
                project_id: "p".into(),
                path: ".env".into(),
                offset: 4_194_304,
            },
            json!({"kind": "pull_project_environment_file", "project_id": "p", "path": ".env", "offset": 4_194_304}),
        ),
        (
            RelayPeerRequest::PullProjectEnvironmentRegistration {
                project_id: "p".into(),
                entry_kind: EnvironmentEntryKind::Mcp,
                name: "notion".into(),
            },
            json!({"kind": "pull_project_environment_registration", "project_id": "p", "entry_kind": "mcp", "name": "notion"}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(wire).unwrap(),
            request
        );
    }
    let secret = RelayPeerResponse::ProjectEnvironmentSecretPulled {
        value: Some(RemoteCredentialSecretInput::new("synthetic-secret".into())),
    };
    let file = RelayPeerResponse::ProjectEnvironmentFilePulled {
        size_bytes: 14,
        sha256: Some("0f".into()),
        content: Some(RemoteFileBytes::encode(b"synthetic-file")),
    };
    assert_eq!(
        serde_json::to_value(&secret).unwrap(),
        json!({"kind": "project_environment_secret_pulled", "value": "synthetic-secret"})
    );
    assert_eq!(
        serde_json::to_value(&file).unwrap(),
        json!({"kind": "project_environment_file_pulled", "size_bytes": 14, "sha256": "0f", "content": "c3ludGhldGljLWZpbGU="})
    );
    let registration = RelayPeerResponse::ProjectEnvironmentRegistrationPulled {
        registration: Some(crate::transport::relay_peer::RemoteProjectRegistration(
            crate::project_environment::Registration::Mcp(
                crate::mcp::CharioxMcpServerConfig::streamable_http(
                    "notion",
                    "https://example.invalid/mcp",
                ),
            ),
        )),
    };
    let wire = serde_json::to_value(&registration).unwrap();
    assert_eq!(wire["kind"], "project_environment_registration_pulled");
    assert_eq!(wire["registration"]["mcp"]["name"], "notion");
    assert!(
        !format!("{registration:?}").contains("example.invalid"),
        "a registration is redacted in debug output"
    );
    assert_eq!(
        serde_json::to_value(RelayPeerResponse::ProjectEnvironmentRefused {
            reason: "Vault locked".into()
        })
        .unwrap(),
        json!({"kind": "project_environment_refused", "reason": "Vault locked"})
    );
    for response in [&secret, &file] {
        let debug = format!("{response:?}");
        assert!(
            !debug.contains("synthetic") && !debug.contains("c3ludGhl"),
            "{debug}"
        );
    }
}
