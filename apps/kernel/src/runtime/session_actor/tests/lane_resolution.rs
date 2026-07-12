use super::*;

#[tokio::test]
async fn direct_session_lane_resolution_rejects_warmed_missing_session_without_lane() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let session_projection = SessionStateProjectionStore::default();
    session_projection.update_list(Vec::new());
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection,
        AgentRuntimeProjectionStore::default(),
        {
            let app = app.lock().await;
            app.terminal_stream_store()
        },
    );

    let _locked_app = app.lock().await;
    let request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
        session_id: "missing-session".to_string(),
        provider_run_id: None,
        cols: 80,
        rows: 24,
    });
    let error = timeout(
        Duration::from_millis(100),
        runtime.resolve_session_lane_key(&request),
    )
    .await
    .expect("warmed missing session lane resolution should not wait for the app lock")
    .expect_err("missing direct session lane should fail");

    match error {
        DaemonError::SessionNotFound { session_id } => {
            assert_eq!(session_id, "missing-session");
        }
        error => panic!("unexpected error: {error}"),
    }
    assert!(
        !runtime.has_lane("missing-session").await,
        "missing direct session should be rejected before creating a session lane"
    );
}

#[tokio::test]
async fn attachment_scoped_session_lane_resolution_rejects_warmed_missing_attachment_without_lane()
{
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let session_projection = SessionStateProjectionStore::default();
    session_projection.update_list(Vec::new());
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection,
        AgentRuntimeProjectionStore::default(),
        {
            let app = app.lock().await;
            app.terminal_stream_store()
        },
    );

    let _locked_app = app.lock().await;
    let request = LocalDaemonRequest::PollRuntimeNotices(PollRuntimeNoticesRequest {
        session_id: "missing-session".to_string(),
        attachment_id: "missing-attachment".to_string(),
    });
    let error = timeout(
        Duration::from_millis(100),
        runtime.resolve_session_lane_key(&request),
    )
    .await
    .expect("warmed missing attachment lane resolution should not wait for the app lock")
    .expect_err("missing attachment should fail before lane creation");

    match error {
        DaemonError::AttachmentNotFound { attachment_id } => {
            assert_eq!(attachment_id, "missing-attachment");
        }
        error => panic!("unexpected error: {error}"),
    }
    assert!(
        !runtime.has_lane("missing-session").await,
        "missing attachment should be rejected before creating a session lane"
    );
}
