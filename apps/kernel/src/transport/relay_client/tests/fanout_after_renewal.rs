//! Row 563: cross-kernel workspace live sync after a projection-only relay renewal.
//! Renewal installs the new relay token where the connector reads it (the config projection);
//! every other sender must read that same snapshot, or sync stops once the first token expires.
use super::support::*;
use chariox_relay::auth::{
    RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier,
};

fn kernel_claim(config: &DaemonConfig, token: &str, expires_at_ms: u64) -> RelayTokenClaims {
    RelayTokenClaims {
        issuer: "fanout-renewal-test".into(),
        subject: config.daemon_id.clone(),
        subject_kind: RelaySubjectKind::Kernel,
        realm_id: "default".into(),
        allowed_actions: vec![
            RelayAction::DaemonRegister,
            RelayAction::DaemonHeartbeat,
            RelayAction::ClientConnect,
            RelayAction::ClientMetadataRead,
            RelayAction::PacketRoute,
            RelayAction::PeerRequest,
        ],
        allowed_targets: None,
        issued_at_ms: crate::session::unix_epoch_ms(),
        expires_at_ms,
        token_id: token.into(),
        account_id: None,
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: Some(config.host_machine_id.clone()),
        client_id: None,
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    }
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.test",
        ])
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn workspace_live_sync_fanout_uses_the_renewed_relay_token_after_the_first_expires() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("fanout-renewal", || async {
        let _guard = relay_client_test_guard().await;
        let root = crate::test_support::TestWorktree::new("fanout-after-renewal");
        let source_root = root.path().join("source");
        let target_root = root.path().join("target");
        std::fs::create_dir_all(source_root.join("src")).unwrap();
        std::fs::create_dir_all(target_root.join("src")).unwrap();
        std::fs::write(target_root.join("src/lib.rs"), "old\n").unwrap();
        for repository in [&source_root, &target_root] {
            git(repository, &["init", "-q", "-b", "main"]);
            git(
                repository,
                &["commit", "--allow-empty", "-q", "-m", "initial fixture"],
            );
        }

        let mut home_config = DaemonConfig::for_tests();
        home_config.daemon_id = "fanout-home".into();
        home_config.host_machine_id = "fanout-home-machine".into();
        home_config.relay_token = Some("home-first".into());
        home_config.relay_heartbeat_ms = 50;
        let mut worker_config = DaemonConfig::for_tests();
        worker_config.daemon_id = "fanout-worker".into();
        worker_config.host_machine_id = "fanout-worker-machine".into();
        worker_config.relay_token = Some("worker".into());
        worker_config.relay_heartbeat_ms = 50;
        let now = crate::session::unix_epoch_ms();
        let first_token_lifetime_ms = 3_000;
        let server = RelayServer::with_auth_verifier(
            RelayConfig {
                host: "127.0.0.1".into(),
                port: 0,
                shared_token: None,
            },
            RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(
                BTreeMap::from([
                    (
                        "home-first".into(),
                        kernel_claim(&home_config, "home-first", now + first_token_lifetime_ms),
                    ),
                    (
                        "home-renewed".into(),
                        kernel_claim(&home_config, "home-renewed", now + 120_000),
                    ),
                    (
                        "worker".into(),
                        kernel_claim(&worker_config, "worker", now + 120_000),
                    ),
                ]),
                BTreeMap::new(),
                None,
            )),
        );
        let listener = server.bind_listener().await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let registry = server.registry();
        let (stop_server, stopped) = oneshot::channel();
        let relay_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        home_config.relay_url = Some(url.clone());
        worker_config.relay_url = Some(url);
        let home = Arc::new(Mutex::new(
            DaemonApp::bootstrap(home_config.clone()).unwrap(),
        ));
        let worker = Arc::new(Mutex::new(
            DaemonApp::bootstrap(worker_config.clone()).unwrap(),
        ));
        let home_state = Arc::new(RwLock::new(RelayClientState::default()));
        let worker_state = Arc::new(RwLock::new(RelayClientState::default()));
        let (stop_home, home_stopped) = watch::channel(false);
        let (stop_worker, worker_stopped) = watch::channel(false);
        let home_task = tokio::spawn(run_daemon_relay_connector(
            home.clone(),
            home_state,
            home_stopped,
        ));
        let worker_task = tokio::spawn(run_daemon_relay_connector(
            worker.clone(),
            worker_state.clone(),
            worker_stopped,
        ));
        wait_for_daemon_registration(registry.clone(), &home_config.daemon_id).await;
        wait_for_daemon_registration(registry.clone(), &worker_config.daemon_id).await;
        // The worker proves the home key through a normal encrypted response before it accepts the
        // home's filesystem mutation (MP-11), exactly as the product path does.
        let handshake = send_peer_request_via_relay(
            &worker,
            &worker_state,
            ClientTarget {
                daemon_id: Some(home_config.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::Ping {
                value: "fanout-handshake".into(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(handshake, RelayPeerResponse::Pong { .. }));

        let (home_session, link_id) = {
            let mut app = home.lock().await;
            let session_id = create_test_session(
                &mut app,
                source_root.to_string_lossy().as_ref(),
                source_root.to_string_lossy().as_ref(),
            );
            let (_, link) = app
                .sessions_mut()
                .create_workspace_link(&session_id, "team-sync".into(), "user-1".into())
                .unwrap();
            for (user, config, repo) in [
                ("user-1", &home_config, &source_root),
                ("user-2", &worker_config, &target_root),
            ] {
                app.sessions_mut()
                    .attach_workspace_link(
                        &session_id,
                        link.link_id(),
                        user.into(),
                        config.host_machine_id.clone(),
                        config.daemon_id.clone(),
                        repo.to_string_lossy().to_string(),
                        None,
                        None,
                    )
                    .unwrap();
            }
            (session_id, link.link_id().to_string())
        };
        {
            // The worker resolves the target by its own link over the same repository root.
            let mut app = worker.lock().await;
            let session_id = create_test_session(
                &mut app,
                target_root.to_string_lossy().as_ref(),
                target_root.to_string_lossy().as_ref(),
            );
            let (_, link) = app
                .sessions_mut()
                .create_workspace_link(&session_id, "team-sync".into(), "user-2".into())
                .unwrap();
            for (user, config, repo) in [
                ("user-2", &worker_config, &target_root),
                ("user-1", &home_config, &source_root),
            ] {
                app.sessions_mut()
                    .attach_workspace_link(
                        &session_id,
                        link.link_id(),
                        user.into(),
                        config.host_machine_id.clone(),
                        config.daemon_id.clone(),
                        repo.to_string_lossy().to_string(),
                        None,
                        None,
                    )
                    .unwrap();
            }
        }
        let _ = link_id;

        // Renewal installs the renewed token into the projection only: the app's own config still
        // holds the first token, as it does while a stalled disk keeps the durable write waiting.
        let mut renewed = home.lock().await.config().clone();
        renewed.relay_token = Some("home-renewed".into());
        home.lock().await.config_projection_store().update(renewed);
        assert_eq!(
            home.lock().await.config().relay_token.as_deref(),
            Some("home-first")
        );
        // Wait for the first token to pass its expiry.
        sleep(Duration::from_millis(first_token_lifetime_ms + 500)).await;

        let change = crate::git_observer::WorkspaceLiveSyncChange {
            session_id: home_session,
            agent_id: "agent-1".into(),
            provider_run_id: "provider-run-1".into(),
            prompt_id: "prompt-1".into(),
            repo_root: source_root.to_string_lossy().to_string(),
            worktree_path: source_root.to_string_lossy().to_string(),
            branch: Some("main".into()),
            changed_paths: vec!["src/lib.rs".into()],
            file_changes: vec![crate::git_observer::WorkspaceLiveSyncFileChange {
                path: "src/lib.rs".into(),
                previous_path: None,
                kind: crate::git_observer::WorkspaceLiveSyncFileChangeKind::Modified,
                before_content_base64: Some("b2xkCg==".into()),
                after_content_base64: Some("bmV3Cg==".into()),
                binary: false,
            }],
            status_fingerprint: "tracked_workspace_live_sync".into(),
        };
        {
            let mut app = home.lock().await;
            app.fanout_remote_workspace_live_sync_change(change, None);
        }
        let synced = std::fs::read_to_string(target_root.join("src/lib.rs")).unwrap();

        let _ = stop_home.send(true);
        let _ = stop_worker.send(true);
        home_task.await.unwrap();
        worker_task.await.unwrap();
        let _ = stop_server.send(());
        relay_task.await.unwrap();
        assert_eq!(
            synced, "new\n",
            "workspace live sync must send with the renewed relay token once the first one expired"
        );
    });
}
