use super::*;

#[test]
fn local_daemon_managed_context_outbound_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 272);
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
        LocalDaemonResponse::ManagedContextTransferStarted {
            status: status.clone(),
        },
        LocalDaemonResponse::ManagedContextTransferStatus { status },
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
        snapshot.pointer("/2/ManagedContextTransferStarted/status/phase"),
        Some(&serde_json::json!("uploading"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("managed-context shape should encode");
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "7f33c41160efee83aa38a0022308a4fe439ba3b7318a9a5bedd9b58e710802af"
    );
}
