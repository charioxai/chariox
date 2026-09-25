use super::*;

#[test]
fn plain_workspace_launch_and_relay_shapes_are_versioned() {
    use crate::managed_context::development::{
        DevelopmentRepositoryRole, DevelopmentWorkspaceKind,
    };
    use crate::transport::relay_peer::RelayManagedContextImportedRepository;
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 347);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        57
    );
    let local = crate::local::ManagedContextRepositoryLaunchTarget {
        workspace_kind: DevelopmentWorkspaceKind::Directory,
        repository_id: "office".into(),
        role: DevelopmentRepositoryRole::Primary,
        target_directory: "office".into(),
        workspace_path: "/managed/context/office".into(),
        head_sha: String::new(),
    };
    assert_eq!(
        serde_json::to_value(&local).unwrap(),
        serde_json::json!({
            "workspaceKind": "directory", "repositoryId": "office", "role": "primary",
            "targetDirectory": "office", "workspacePath": "/managed/context/office", "headSha": ""
        })
    );
    let relay = RelayManagedContextImportedRepository {
        workspace_kind: local.workspace_kind,
        repository_id: local.repository_id,
        role: local.role,
        target_directory: local.target_directory,
        destination_path: local.workspace_path,
        head_sha: local.head_sha,
    };
    assert_eq!(
        serde_json::to_value(&relay).unwrap(),
        serde_json::json!({
            "workspace_kind": "directory", "repository_id": "office", "role": "primary",
            "target_directory": "office", "destination_path": "/managed/context/office", "head_sha": ""
        })
    );
    let mut legacy = serde_json::to_value(&relay).unwrap();
    legacy.as_object_mut().unwrap().remove("workspace_kind");
    legacy["head_sha"] = serde_json::json!("a".repeat(40));
    let restored: RelayManagedContextImportedRepository =
        serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(restored.workspace_kind, DevelopmentWorkspaceKind::Git);
    assert_eq!(serde_json::to_value(restored).unwrap(), legacy);
}

#[test]
fn local_daemon_managed_context_outbound_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 347);
    let plan = crate::managed_bootstrap::ManagedKernelContextPlan::source_project_for_tests(
        "context-1",
        "realm-1",
        "source-kernel",
        &"a".repeat(64),
        "project-1",
    );
    let ticket = crate::managed_context::outbound_service::ManagedContextTransferTicket {
        environment_id: "environment-1".to_string(),
        context_plan: plan,
        target: crate::managed_context::outbound_service::ManagedContextTransferTarget {
            relay_realm_id: "realm-1".to_string(),
            machine_id: "target-machine".to_string(),
            kernel_id: "target-kernel".to_string(),
            relay_public_key: "target-public-key".to_string(),
            key_thumbprint: "b".repeat(64),
        },
    };
    let status = crate::managed_context::outbound_service::ManagedContextOutboundOperationStatus {
        context_id: "context-1".to_string(),
        plan_digest: ticket.context_plan.package_binding().plan_digest,
        phase: crate::managed_context::outbound_service::ManagedContextOutboundOperationPhase::Uploading,
        accepted_bytes: 512,
        package_size_bytes: 1_024,
        receipt: None,
        failure_code: None,
        failure_message: None,
        retryable: false,
        updated_at_ms: 1_234,
    };
    let snapshot = serde_json::json!([
        LocalDaemonRequest::StartManagedContextTransfer(
            crate::local::StartManagedContextTransferRequest { ticket },
        ),
        LocalDaemonRequest::GetManagedContextTransferStatus(
            crate::local::GetManagedContextTransferStatusRequest {
                context_id: "context-1".to_string(),
            },
        ),
        LocalDaemonRequest::GetManagedContextLaunchTarget(
            crate::local::GetManagedContextLaunchTargetRequest {
                context_id: "context-1".to_string(),
                plan_digest: status.plan_digest.clone(),
            },
        ),
        LocalDaemonResponse::ManagedContextTransferStarted {
            status: status.clone(),
        },
        LocalDaemonResponse::ManagedContextTransferStatus {
            status: status.clone(),
        },
        LocalDaemonResponse::ManagedContextLaunchTarget {
            target: crate::local::ManagedContextLaunchTarget {
                environment_id: "environment-empty".to_string(),
                kernel_id: "target-kernel".to_string(),
                context_id: "context-empty".to_string(),
                plan_digest: status.plan_digest.clone(),
                development: crate::local::ManagedContextDevelopmentLaunchTarget::Empty {
                    workspace_path: "/managed/empty-workspace".to_string(),
                },
            },
        },
        LocalDaemonResponse::ManagedContextLaunchTarget {
            target: crate::local::ManagedContextLaunchTarget {
                environment_id: "environment-1".to_string(),
                kernel_id: "target-kernel".to_string(),
                context_id: "context-1".to_string(),
                plan_digest: status.plan_digest,
                development: crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
                    project_id: "project-1".to_string(),
                    destination_root: "/managed/context".to_string(),
                    primary_repository_id: "repository-1".to_string(),
                    repositories: vec![crate::local::ManagedContextRepositoryLaunchTarget {
                        workspace_kind:
                            crate::managed_context::development::DevelopmentWorkspaceKind::Git,
                        repository_id: "repository-1".to_string(),
                        role:
                            crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        target_directory: "primary".to_string(),
                        workspace_path: "/managed/context/primary".to_string(),
                        head_sha: "c".repeat(40),
                    }],
                },
            },
        },
    ]);
    assert_eq!(
        snapshot.pointer("/0/StartManagedContextTransfer/ticket/environmentId"),
        Some(&serde_json::json!("environment-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/StartManagedContextTransfer/ticket/contextPlan/developmentSetup/kind"),
        Some(&serde_json::json!("source_project"))
    );
    assert_eq!(
        snapshot.pointer("/3/ManagedContextTransferStarted/status/phase"),
        Some(&serde_json::json!("uploading"))
    );
    assert_eq!(
        snapshot.pointer("/5/ManagedContextLaunchTarget/target/development/workspacePath"),
        Some(&serde_json::json!("/managed/empty-workspace"))
    );
    assert_eq!(
        snapshot.pointer(
            "/6/ManagedContextLaunchTarget/target/development/repositories/0/workspacePath"
        ),
        Some(&serde_json::json!("/managed/context/primary"))
    );
    assert_eq!(
        snapshot.pointer("/6/ManagedContextLaunchTarget/target/development/projectId"),
        Some(&serde_json::json!("project-1"))
    );
    assert_eq!(
        snapshot.pointer("/6/ManagedContextLaunchTarget/target/development/destinationRoot"),
        Some(&serde_json::json!("/managed/context"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("managed-context shape should encode");
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "177b7975c738d66d7ed3245f52a3fbf7e660ff1006bbd47e4ae7f0aee5862d89"
    );
}

#[test]
fn local_daemon_managed_context_completed_receipt_uses_public_camel_case_shape() {
    use crate::transport::relay_peer::{
        RelayManagedContextImportReceipt, RelayManagedContextImportedRepository,
        RelayManagedDevelopmentContextImportReceipt, RelayManagedKernelContextImportReceipt,
    };

    let relay_receipt = RelayManagedContextImportReceipt {
        transfer_id: "transfer-1".to_string(),
        archive_sha256: "a".repeat(64),
        plan_digest: "sha256:plan".to_string(),
        development: RelayManagedDevelopmentContextImportReceipt::FromSource {
            project_id: "project-1".to_string(),
            destination_root: "/managed/context".to_string(),
            primary_repository_id: "repository-1".to_string(),
            repositories: vec![RelayManagedContextImportedRepository {
                workspace_kind: crate::managed_context::development::DevelopmentWorkspaceKind::Git,
                repository_id: "repository-1".to_string(),
                role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                target_directory: "primary".to_string(),
                destination_path: "/managed/context/primary".to_string(),
                head_sha: "b".repeat(40),
            }],
        },
        kernel_context: RelayManagedKernelContextImportReceipt::FromKernel {
            context_id: "context-1".to_string(),
            source_kernel_id: "home-kernel".to_string(),
            source_key_thumbprint: "c".repeat(64),
            snapshot_sha256: "d".repeat(64),
            extension_count: 1,
            dependency_count: 2,
        },
        receipt_sha256: "e".repeat(64),
    };
    let relay_serialized = serde_json::to_value(&relay_receipt)
        .expect("relay receipt should retain its snake_case wire shape");
    assert_eq!(
        relay_serialized.pointer("/transfer_id"),
        Some(&serde_json::json!("transfer-1"))
    );
    assert_eq!(
        relay_serialized.pointer("/development/project_id"),
        Some(&serde_json::json!("project-1"))
    );
    assert_eq!(
        relay_serialized.pointer("/development/repositories/0/destination_path"),
        Some(&serde_json::json!("/managed/context/primary"))
    );
    assert_eq!(
        relay_serialized.pointer("/kernel_context/source_kernel_id"),
        Some(&serde_json::json!("home-kernel"))
    );
    assert!(relay_serialized.get("transferId").is_none());
    assert!(relay_serialized.pointer("/development/projectId").is_none());
    assert!(relay_serialized
        .pointer("/development/repositories/0/workspacePath")
        .is_none());
    assert!(relay_serialized
        .pointer("/kernelContext/sourceKernelId")
        .is_none());

    let response = LocalDaemonResponse::ManagedContextTransferStatus {
        status: crate::managed_context::outbound_service::ManagedContextOutboundOperationStatus {
            context_id: "context-1".to_string(),
            plan_digest: "sha256:plan".to_string(),
            phase: crate::managed_context::outbound_service::ManagedContextOutboundOperationPhase::Completed,
            accepted_bytes: 1_024,
            package_size_bytes: 1_024,
            receipt: Some(relay_receipt.into()),
            failure_code: None,
            failure_message: None,
            retryable: false,
            updated_at_ms: 42,
        },
    };

    let serialized =
        serde_json::to_value(response).expect("public managed-context response should serialize");
    let receipt = serialized
        .pointer("/ManagedContextTransferStatus/status/receipt")
        .expect("completed response should include its receipt");
    assert_eq!(
        receipt.pointer("/transferId"),
        Some(&serde_json::json!("transfer-1"))
    );
    assert_eq!(
        receipt.pointer("/archiveSha256"),
        Some(&serde_json::json!("a".repeat(64)))
    );
    assert_eq!(
        receipt.pointer("/development/projectId"),
        Some(&serde_json::json!("project-1"))
    );
    assert_eq!(
        receipt.pointer("/development/repositories/0/targetDirectory"),
        Some(&serde_json::json!("primary"))
    );
    assert_eq!(
        receipt.pointer("/kernelContext/sourceKernelId"),
        Some(&serde_json::json!("home-kernel"))
    );
    assert_eq!(
        receipt.pointer("/receiptSha256"),
        Some(&serde_json::json!("e".repeat(64)))
    );
    assert!(receipt.get("transfer_id").is_none());
    assert!(receipt.pointer("/development/project_id").is_none());
    assert!(receipt
        .pointer("/kernel_context/source_kernel_id")
        .is_none());
    let serialized = serde_json::to_string(&serialized)
        .expect("public managed-context receipt snapshot should encode");
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "9f48b35ae27b36687176123ed44b341d4bc3e5545b9753d88104f16a06b03a31"
    );
}

#[test]
fn managed_context_launch_target_reads_schema_v4_variant_fields() {
    let development =
        serde_json::from_value::<crate::local::ManagedContextDevelopmentLaunchTarget>(
            serde_json::json!({
                "kind": "from_source",
                "project_id": "project-1",
                "destination_root": "/managed/context",
                "primary_repository_id": "repository-1",
                "repositories": [{
                    "repositoryId": "repository-1",
                    "role": "primary",
                    "targetDirectory": "primary",
                    "workspacePath": "/managed/context/primary",
                    "headSha": "c".repeat(40),
                }],
            }),
        )
        .expect("schema-v4 launch target fields remain readable");
    assert!(matches!(
        development,
        crate::local::ManagedContextDevelopmentLaunchTarget::FromSource { .. }
    ));
    let empty = serde_json::from_value::<crate::local::ManagedContextDevelopmentLaunchTarget>(
        serde_json::json!({ "kind": "empty" }),
    )
    .expect("schema-v4 empty launch target remains readable");
    assert_eq!(
        empty,
        crate::local::ManagedContextDevelopmentLaunchTarget::Empty {
            workspace_path: String::new(),
        }
    );
}
