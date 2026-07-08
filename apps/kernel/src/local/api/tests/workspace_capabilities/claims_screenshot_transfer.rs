use super::*;

#[test]
fn local_request_api_rejects_conflicting_workspace_write_claims() {
    run_workspace_capability_test(
        "local_request_api_rejects_conflicting_workspace_write_claims",
        local_request_api_rejects_conflicting_workspace_write_claims_inner,
    );
}

fn local_request_api_rejects_conflicting_workspace_write_claims_inner() {
    let worktree_root = std::env::temp_dir().join("arroba-workspace-claim-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src")).expect("worktree should exist");
    std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-workspace-claim".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let _claim = harness.with_app_mut(|app| {
        app.workspace_coordinator()
            .acquire_worktree_write_claim(
                session.workspace_id().to_string(),
                worktree_root.display().to_string(),
                "other-session",
                Some("other-attachment".to_string()),
                "file_edit",
            )
            .expect("existing claim should acquire")
    });

    let health = harness
        .dispatch(LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest))
        .expect("health should be available while claim is active");
    match health {
        LocalDaemonResponse::DaemonHealth { projection } => {
            assert_eq!(
                projection
                    .workspace_coordination
                    .active_operation_claims
                    .len(),
                1
            );
        }
        _ => panic!("unexpected health response"),
    }

    let error = harness
        .dispatch(LocalDaemonRequest::EditFile(EditFileCapabilityRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            path: worktree_root.join("src/lib.rs"),
            contents: "after".to_string(),
        }))
        .expect_err("conflicting write should be rejected");

    match error {
        DaemonError::WorkspaceClaimConflict {
            requested_session_id,
            existing_session_id,
            ..
        } => {
            assert_eq!(requested_session_id, session.id());
            assert_eq!(existing_session_id, "other-session");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_returns_structured_screenshot_unavailable_result() {
    run_workspace_capability_test(
        "local_request_api_returns_structured_screenshot_unavailable_result",
        local_request_api_returns_structured_screenshot_unavailable_result_inner,
    );
}

fn local_request_api_returns_structured_screenshot_unavailable_result_inner() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("ARROBA_SCREENSHOT_DISABLE", "1");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", std::env::temp_dir().display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-screenshot".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::CaptureScreenshot(
            CaptureScreenshotCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("screenshot request should succeed with unavailable result");
    std::env::remove_var("ARROBA_SCREENSHOT_DISABLE");

    match response {
        LocalDaemonResponse::ScreenshotCaptured { result } => {
            assert_eq!(
                result.status,
                crate::capability::ScreenshotStatus::Unavailable
            );
        }
        _ => panic!("unexpected screenshot response"),
    }
}

#[test]
fn local_request_api_stores_transferred_file_under_session_artifacts() {
    run_workspace_capability_test(
        "local_request_api_stores_transferred_file_under_session_artifacts",
        local_request_api_stores_transferred_file_under_session_artifacts_inner,
    );
}

fn local_request_api_stores_transferred_file_under_session_artifacts_inner() {
    let worktree_root = std::env::temp_dir().join("arroba-transfer-local-api-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    let source = worktree_root.join("artifact.txt");
    std::fs::write(&source, "artifact").expect("file should exist");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-transfer".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::StoreTransferredFile(
            StoreTransferredFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                source_path: source,
                display_name: None,
            },
        ))
        .expect("transfer should succeed");

    match response {
        LocalDaemonResponse::FileTransferred { result } => {
            assert!(
                result
                    .stored_path
                    .to_string_lossy()
                    .contains("arroba-session-artifacts")
            );
            assert_eq!(result.bytes, 8);
        }
        _ => panic!("unexpected transfer response"),
    }
}
