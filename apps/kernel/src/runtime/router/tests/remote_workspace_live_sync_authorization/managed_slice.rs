use super::*;

#[tokio::test]
async fn forwarded_workspace_matching_identity_does_not_authorize_a_stale_lease() {
    let root = std::env::temp_dir().join(format!(
        "chariox-workspace-stale-lease-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let result = run_plain_slice_write(&root, &root, |_, _, context| {
        context.worker_worktree_path = root.to_string_lossy().into();
        context.worker_workspace_identity =
            crate::io::WorkspaceIdentity::local(root.canonicalize().unwrap().to_string_lossy());
        context.leased_agent_id = "stale-lease".into();
    })
    .await;
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        matches!(result, Err(DaemonError::LocalTransport { .. })),
        "matching workspace identity must not authorize a stale lease: {result:?}"
    );
}

#[tokio::test]
async fn managed_slice_plain_workspace_accepts_an_attributed_artifact_write() {
    let root = std::env::temp_dir().join(format!(
        "chariox-plain-slice-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let result = run_plain_slice_write(&root, &root, |_, _, _| {}).await;
    let _ = std::fs::remove_dir_all(&root);
    let result = result.unwrap();
    assert!(
        result.ok,
        "managed non-Git mount should coordinate: {result:?}"
    );
    assert_eq!(
        result.payload.get("applied"),
        Some(&serde_json::json!(true))
    );
}

async fn run_plain_slice_write(
    root: &std::path::Path,
    mount: &std::path::Path,
    change: impl FnOnce(
        &mut DaemonApp,
        &str,
        &mut crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    ),
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
    app.agents_mut()
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
    change(&mut app, &slice.id, &mut context);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);
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

#[tokio::test]
async fn managed_slice_plain_workspace_rejects_stale_or_unrelated_bindings() {
    let root = std::env::temp_dir().join(format!(
        "chariox-plain-slice-denials-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(root.join("other")).unwrap();
    let mut outcomes = Vec::new();
    for case in [
        "mount",
        "stopped",
        "detached",
        "worker",
        "machine",
        "lease",
        "run",
        "home",
        "worker_path",
        "fingerprint",
        "git",
    ] {
        let mount = if case == "mount" {
            root.join("other")
        } else {
            root.clone()
        };
        let result = run_plain_slice_write(&root, &mount, |app, slice_id, context| match case {
            "mount" => {}
            "stopped" => {
                app.slices()
                    .set_status(slice_id, crate::slice::SliceStatus::Stopped, 5)
                    .unwrap();
            }
            "detached" => {
                app.slices()
                    .detach_agent(slice_id, &context.home_agent_id, 5)
                    .unwrap();
            }
            "worker" => context.worker_kernel_id = "unrelated-worker".into(),
            "machine" => context.worker_machine_id = "unrelated-machine".into(),
            "lease" => context.leased_agent_id = "stale-lease".into(),
            "run" => context.worker_provider_run_id = "stale-run".into(),
            "home" => context.home_kernel_id = "unrelated-home".into(),
            "worker_path" => context.worker_worktree_path = "/workspace/other".into(),
            "fingerprint" => {
                context.worker_workspace_identity.worktree_root_fingerprint = "/other".into()
            }
            "git" => context.worker_workspace_identity.vcs_provider = Some("git".into()),
            _ => unreachable!(),
        })
        .await;
        outcomes.push((case, result));
    }
    std::fs::remove_dir_all(root).unwrap();
    for (case, result) in outcomes {
        match result {
            Err(DaemonError::LocalTransport { .. }) => {}
            Ok(result) => {
                assert!(!result.ok, "{case} was incorrectly authorized");
                assert_eq!(
                    result
                        .payload
                        .pointer("/reason/kind")
                        .and_then(|v| v.as_str()),
                    Some("remote_workspace_not_coordinated"),
                    "{case}"
                );
            }
            other => panic!("unexpected denial for {case}: {other:?}"),
        }
    }
}
