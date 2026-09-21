use super::*;

fn lease_worker_router() -> (CommandRouter, Arc<Mutex<DaemonApp>>) {
    let mut config = DaemonConfig::for_tests();
    config.kernel_runtime_role = crate::config::KernelRuntimeRole::RemoteLeaseWorker;
    config.remote_lease_capacity = Some(1);
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("lease worker should bootstrap"),
    ));
    (
        CommandRouter::with_interactive_capacity(Arc::clone(&app), 1),
        app,
    )
}

fn denied_requests() -> Vec<LocalDaemonRequest> {
    vec![
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new("workspace", "worktree")),
        LocalDaemonRequest::JoinSessionInvite(crate::local::JoinSessionInviteRequest {
            invite_token: "untrusted-invite".to_string(),
            user_id: "user-1".to_string(),
        }),
        LocalDaemonRequest::ImportExternalProviderSession(
            crate::local::ImportExternalProviderSessionRequest {
                external_session_id: "codex:external-1".to_string(),
                alias: None,
                provider: None,
                model: None,
                effort: None,
                worktree_id: None,
            },
        ),
        LocalDaemonRequest::ImportExternalProviderAgent(
            crate::local::ImportExternalProviderAgentRequest {
                session_id: "hidden-session".to_string(),
                external_session_id: "codex:external-1".to_string(),
                alias: None,
                provider: None,
                model: None,
                effort: None,
                focus: None,
            },
        ),
        LocalDaemonRequest::AttachToSession(crate::local::AttachToSessionRequest {
            session_id: "session-1".to_string(),
            client_id: "client-1".to_string(),
            capability_level: crate::attachment::ClientCapabilityLevel::MessageTransport,
        }),
        LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
            session_id: "session-1".to_string(),
            alias: None,
            provider: None,
            account_profile: None,
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }),
        LocalDaemonRequest::ListProjects(crate::local::ListProjectsRequest {
            include_archived: true,
        }),
        LocalDaemonRequest::GetProviderCatalog(crate::local::GetProviderCatalogRequest::default()),
        LocalDaemonRequest::PollRuntimeNotices(crate::local::PollRuntimeNoticesRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
        }),
        LocalDaemonRequest::ListSlices(crate::local::ListSlicesRequest),
        LocalDaemonRequest::ListWorkflows(crate::local::ListWorkflowsRequest {
            session_id: "session-1".to_string(),
        }),
        LocalDaemonRequest::ReadFile(crate::local::ReadFileCapabilityRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            path: "README.md".into(),
        }),
        LocalDaemonRequest::ListMetaagentEvents(crate::local::ListMetaagentEventsRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "metaagent-1".to_string(),
            limit: None,
            status: None,
            kind: None,
        }),
        LocalDaemonRequest::CancelBrowserImport(crate::local::CancelBrowserImportRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            request_id: "browser-import-1".to_string(),
        }),
    ]
}

#[tokio::test]
async fn lease_worker_rejects_public_session_authority_requests_before_side_effects() {
    for request in denied_requests() {
        let (router, app) = lease_worker_router();
        let command =
            KernelCommand::from_local_request("lease-worker-denied", None, None, &request);
        let error = router
            .dispatch(command, request)
            .await
            .expect_err("lease worker must reject public session authority");
        assert!(matches!(
            error,
            DaemonError::KernelRuntimeRoleDenied {
                role: "remote_lease_worker",
                ..
            }
        ));
        assert!(app.lock().await.sessions().list_all_sessions().is_empty());
    }
}

#[tokio::test]
async fn general_kernel_preserves_public_session_creation() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("general kernel should bootstrap"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request =
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new("workspace", "worktree"));
    let command = KernelCommand::from_local_request("general-create", None, None, &request);
    assert!(matches!(
        router.dispatch(command, request).await,
        Ok(LocalDaemonResponse::SessionCreated { .. })
    ));
}

#[tokio::test]
async fn lease_worker_allows_only_local_health() {
    let (router, _) = lease_worker_router();
    let request = LocalDaemonRequest::GetDaemonHealth(crate::local::GetDaemonHealthRequest);
    let command = KernelCommand::from_local_request("lease-worker-health", None, None, &request);
    assert!(matches!(
        router.dispatch(command, request).await,
        Ok(LocalDaemonResponse::DaemonHealth { .. })
    ));
}
