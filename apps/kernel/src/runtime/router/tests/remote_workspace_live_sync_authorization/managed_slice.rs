use super::*;

#[tokio::test]
async fn managed_slice_plain_workspace_write_coordinates() {
    let workspace = crate::test_temp_dir::TestTempDir::new("plain-slice-workspace");
    let result = run_plain_slice_write(workspace.path(), workspace.path())
        .await
        .unwrap();
    assert!(result.ok, "expected coordinated write: {result:?}");
}

#[tokio::test]
async fn managed_slice_plain_workspace_requires_matching_mount() {
    let workspace = crate::test_temp_dir::TestTempDir::new("plain-slice-workspace");
    let other = crate::test_temp_dir::TestTempDir::new("other-slice-mount");
    let result = run_plain_slice_write(workspace.path(), other.path())
        .await
        .unwrap();
    assert!(!result.ok, "an unrelated mount must be refused");
}

async fn run_plain_slice_write(
    root: &std::path::Path,
    mount: &std::path::Path,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "home-kernel".into();
    config.host_machine_id = "home-machine".into();
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .unwrap();
    let agent = spawn_test_agent(&mut app, session.id(), "office", "codex");
    focus_test_agent(&mut app, session.id(), agent.id());
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease-1".into(),
                leased_agent_id: "leased-agent-1".into(),
                active_worker_provider_run_id: Some("worker-provider-run".into()),
                relay_url: None,
                relay_token: None,
                worker_worktree_id: Some("/workspace".into()),
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let slices = app.slices();
    let slice = slices
        .create(
            "home-kernel",
            "home-machine",
            crate::slice::CreateSliceInput {
                source_slice_ref: None,
                name: "office".into(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".into(),
                display_mode: crate::slice::SliceDisplayMode::Headless,
                display_backend: crate::slice::SliceDisplayBackend::Novnc,
                workspace_id: Some(root.to_string_lossy().into()),
                worktree_id: Some(root.to_string_lossy().into()),
                workspace_mount: Some(mount.to_string_lossy().into()),
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: vec![],
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .unwrap();
    slices
        .attach_agent(&slice.id, session.id(), agent.id(), 2)
        .unwrap();
    slices
        .set_worker_presence(
            &slice.id,
            Some("worker-kernel".into()),
            Some("worker-machine".into()),
            vec![],
            3,
        )
        .unwrap();
    slices
        .set_status(&slice.id, crate::slice::SliceStatus::Running, 4)
        .unwrap();
    let mut context = remote_workspace_live_sync_context(session.id(), agent.id(), root);
    context.worker_worktree_path = "/workspace".into();
    context.worker_workspace_identity = crate::io::WorkspaceIdentity::local("/workspace");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2)
        .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
            "worker-kernel",
        ));
    let metadata = crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata {
        invocation_id: "plain-write-1".into(),
        provider_tool_call_id: Some("tool-1".into()),
        attempt: 1,
        idempotency_key: None,
    };
    let (result, states) = router.dispatch_forwarded_workspace_live_sync_runtime_tool_call(
        context, metadata, crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL.into(),
        serde_json::json!({"path":"release.py", "content_text":"# agent-authored\n", "domain":"text"}),
        vec![crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState {
            path: "release.py".into(), exists: false, domain: Some("text".into()), content_text: None, content_base64: None,
        }],
    ).await?;
    if result.ok {
        assert_eq!(
            states.first().and_then(|s| s.content_text.as_deref()),
            Some("# agent-authored\n")
        );
        assert_eq!(
            result
                .payload
                .pointer("/workspace/worktree_root_fingerprint")
                .and_then(|value| value.as_str()),
            root.canonicalize().unwrap().to_str()
        );
    }
    Ok(result)
}
