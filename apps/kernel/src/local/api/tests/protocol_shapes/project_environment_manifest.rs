//! MP-08: Snapshot/hash guard for the Project environment metadata query.
use super::*;

#[test]
fn mp08_project_environment_manifest_shape_requires_protocol_371() {
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 488);
    let request = LocalDaemonRequest::GetProjectEnvironmentManifest(
        crate::local::GetProjectEnvironmentManifestRequest {
            project_id: "project-1".into(),
            agent_id: None,
        },
    );
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"GetProjectEnvironmentManifest":{"projectId":"project-1"}})
    );
    let response = LocalDaemonResponse::ProjectEnvironmentManifest { manifest: None };
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        serde_json::json!({"ProjectEnvironmentManifest":{"manifest":null}})
    );
    let encoded = serde_json::to_string(&(request, response)).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(encoded)),
        "92a26223354217c3e97d7c3b8feb5eee3a5118515912f6d6ed2f2e6c2b8d1e4d"
    );
}

#[test]
fn mp08_manifest_fields_are_bound_to_protocol_371_snapshot() {
    use crate::project_environment::*;
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 488);
    let manifest = ProjectEnvironmentManifest {
        schema_version: 1,
        project_id: "project-1".into(),
        evidence_digest: "a".repeat(64),
        entries: vec![ProjectEnvironmentEntry {
            name: "DATABASE_URL".into(),
            workspace_id: "workspace-1".into(),
            kind: ProjectEnvironmentEntryKind::Variable,
            classification: ProjectEnvironmentClassification::Secret,
            excluded: true,
            uses: vec![ProjectEnvironmentUse {
                path: "src/db.ts".into(),
                line: 3,
            }],
            locator: ProjectEnvironmentLocator::EnvFile {
                path: ".env.local".into(),
                key: "DATABASE_URL".into(),
            },
            status: ProjectEnvironmentEntryStatus::Found,
        }],
        private_files: vec![ProjectPrivateFileDecision {
            workspace_id: "workspace-1".into(),
            path: "app.local.json".into(),
            bring: false,
            reason: "Referenced secret configuration; Vault only".into(),
            secret_looking: true,
        }],
        toolchain_hints: vec!["node".into()],
        package_hints: vec!["libpq-dev".into()],
        service_hints: vec!["postgres".into()],
    };
    let response = LocalDaemonResponse::ProjectEnvironmentManifest {
        manifest: Some(manifest),
    };
    let encoded = serde_json::to_string(&response).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(encoded)),
        "9b4f1d96a8c86073a15d30d4c4b8751cca3bb9de78b06e67ad29ea45a0f894d1"
    );
}

#[test]
fn mp08_mp10_mp11_review_and_interactive_export_shapes_require_protocol_371() {
    use crate::project_environment::*;
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 488);
    let review = ProjectEnvironmentReview {
        schema_version: 1,
        project_name: "App".into(),
        target_name: "Slice".into(),
        expanded: true,
        unattended: false,
        changed_only: true,
        rows: vec![ProjectEnvironmentReviewRow {
            label: "Files".into(),
            summary: "bringing 1 / leaving 0".into(),
        }],
        files: vec![ProjectEnvironmentReviewFile {
            id: "file-1".into(),
            path: "CLAUDE.local.md".into(),
            bring: true,
            reason: "Project instructions".into(),
            changed: true,
        }],
        inputs: vec![ProjectEnvironmentReviewInput {
            id: "input-1".into(),
            name: "TOKEN".into(),
            workspace_id: "web".into(),
            missing: true,
            secret: true,
            source: "not found".into(),
            changed: true,
            uses: vec![ProjectEnvironmentUse {
                path: "src/app.ts".into(),
                line: 3,
            }],
        }],
    };
    let encoded = serde_json::to_string(&review).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(encoded)),
        "8d97a5bce37678d80dcf366cb0c8eb1b84a4164cffb990a9dac15b8c087d7e83"
    );
    let interaction = crate::session::RuntimeInteraction::new(
        "review",
        "agent",
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        None,
        "Review",
        vec![],
        None,
        None,
        None,
    )
    .with_project_environment_review(review);
    assert!(serde_json::to_value(interaction)
        .unwrap()
        .get("project_environment_review")
        .is_some());
    let request = LocalDaemonRequest::StartSlice(crate::local::SliceRefRequest {
        slice_ref: "slice".into(),
        interactive: true,
    });
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({"StartSlice":{"slice_ref":"slice","interactive":true}})
    );
    let dependency = crate::managed_context::kernel::KernelExtensionDependency::UserRules {
        body: "Use clear names".into(),
    };
    assert_eq!(
        serde_json::to_value(dependency).unwrap(),
        serde_json::json!({"type":"user_rules","body":"Use clear names"})
    );
}

#[test]
fn mp08_mp10_mp11_adjustment_shape_requires_protocol_372() {
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 488);
    let request = LocalDaemonRequest::AdjustProjectEnvironment(
        crate::local::AdjustProjectEnvironmentRequest {
            session_id: "session-1".into(),
            agent_id: "agent-1".into(),
        },
    );
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"AdjustProjectEnvironment":{"sessionId":"session-1","agentId":"agent-1"}})
    );
    let response = LocalDaemonResponse::ProjectEnvironmentAdjustmentStarted {
        session_id: "session-1".into(),
        agent_id: "agent-1".into(),
    };
    let encoded = serde_json::to_string(&(request, response)).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(encoded)),
        "89c613b04824356e515d81d7d950bea3dca78015773983b7cc3e2ebd501653ee"
    );
}

#[test]
fn mp08_mp10_mp11_worker_environment_query_requires_protocol_373() {
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 488);
    let request = LocalDaemonRequest::GetProjectEnvironmentManifest(
        crate::local::GetProjectEnvironmentManifestRequest {
            project_id: "project-1".into(),
            agent_id: Some("agent-1".into()),
        },
    );
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"GetProjectEnvironmentManifest":{"projectId":"project-1","agentId":"agent-1"}})
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&request).unwrap())
        ),
        "55a9652be15eb4c549c0329268e7c48efec222efb00e5763722413268dd666fb"
    );
}

#[test]
fn mp08_mp10_mp11_slice_source_export_shape_requires_protocol_373() {
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 488);
    let value: serde_json::Value = serde_json::from_str(r#"{"CreateSlice":{"source_slice_ref":"first","name":"second","backend":"local_docker","os":"linux","display_mode":"headless"}}"#).unwrap();
    let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
    let roundtrip = serde_json::to_value(request).unwrap();
    assert_eq!(roundtrip, value);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&roundtrip).unwrap())
        ),
        "26d19c33a9ea613cfb2ecd39cdef89820fe0ed29e6cba7de72815a39e09351bd"
    );
}
