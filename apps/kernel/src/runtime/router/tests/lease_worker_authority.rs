use super::*;

#[tokio::test]
async fn lease_worker_rejects_public_session_invites_and_imports_before_dispatch() {
    let mut config = DaemonConfig::for_tests();
    config.lease_worker_capacity = Some(1);
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("lease worker should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(app, 1);
    let requests = vec![
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new("workspace", "worktree")),
        LocalDaemonRequest::JoinSessionInvite(crate::local::JoinSessionInviteRequest {
            invite_token: "invite".to_string(),
            user_id: "user".to_string(),
        }),
        LocalDaemonRequest::AcceptCloudSessionInvite(
            crate::local::AcceptCloudSessionInviteRequest {
                invite_token: "invite".to_string(),
            },
        ),
        LocalDaemonRequest::ImportExternalProviderSession(
            crate::local::ImportExternalProviderSessionRequest {
                external_session_id: "external".to_string(),
                alias: None,
                provider: None,
                model: None,
                effort: None,
                worktree_id: None,
            },
        ),
        LocalDaemonRequest::ImportExternalProviderAgent(
            crate::local::ImportExternalProviderAgentRequest {
                session_id: "session".to_string(),
                external_session_id: "external".to_string(),
                alias: None,
                provider: None,
                model: None,
                effort: None,
                focus: None,
            },
        ),
    ];
    for (index, request) in requests.into_iter().enumerate() {
        let command = KernelCommand::from_local_request(
            format!("lease-worker-denied-{index}"),
            None,
            None,
            &request,
        );
        assert!(matches!(
            router.dispatch(command, request).await,
            Err(DaemonError::LeaseWorkerOperationDenied { .. })
        ));
    }
}

#[tokio::test]
async fn lease_worker_rejects_relay_managed_context_import() {
    let mut config = DaemonConfig::for_tests();
    config.lease_worker_capacity = Some(1);
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("lease worker should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(app, 1);
    let request = crate::runtime::router::RelayManagedContextArmRequest {
        identity: chariox_relay::protocol::RelayCallerIdentity {
            realm_id: "realm".to_string(),
            subject: "home-kernel".to_string(),
            subject_kind: chariox_relay::auth::RelaySubjectKind::Kernel,
            expires_at_ms: u64::MAX,
            token_id: None,
            user_id: Some("owner".to_string()),
            public_key_thumbprint: None,
        },
        source_kernel_id: "home-kernel".to_string(),
        context_id: "context".to_string(),
        plan_digest: "plan".to_string(),
        target_environment_id: "environment".to_string(),
        target_kernel_id: "worker".to_string(),
        target_key_thumbprint: "thumbprint".to_string(),
        capability: "capability".to_string(),
        archive_sha256: "digest".to_string(),
        archive_size_bytes: 1,
    };
    assert!(matches!(
        router.relay_arm_managed_context_import(request).await,
        Err(DaemonError::LeaseWorkerOperationDenied {
            operation: "import managed context"
        })
    ));
}
