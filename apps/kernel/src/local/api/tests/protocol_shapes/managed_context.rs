use super::*;

#[test]
fn local_daemon_managed_context_outbound_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 274);
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
                environment_id: "environment-1".to_string(),
                kernel_id: "target-kernel".to_string(),
                context_id: "context-1".to_string(),
                plan_digest: status.plan_digest,
                development: crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
                    project_id: "project-1".to_string(),
                    destination_root: "/managed/context".to_string(),
                    primary_repository_id: "repository-1".to_string(),
                    repositories: vec![crate::local::ManagedContextRepositoryLaunchTarget {
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
        snapshot.pointer(
            "/5/ManagedContextLaunchTarget/target/development/repositories/0/workspacePath"
        ),
        Some(&serde_json::json!("/managed/context/primary"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("managed-context shape should encode");
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "02a90203fa10041e3c7a5b239603b26587da9f540fb3741e8bcaa80f09a8f9f4"
    );
}
