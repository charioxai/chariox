use super::support::*;
use crate::account_profile::*;
use base64::Engine;

#[test]
fn secrev_f2_encrypted_slice_import_replay_cannot_restore_removed_login() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("slice-copy-replay", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let root = crate::test_support::TestWorktree::new("slice-copy-replay");
        let source = DaemonConfig::for_tests();
        let mut target = DaemonConfig::for_tests();
        target.daemon_id = "replay-slice-kernel".into();
        target.host_machine_id = "replay-slice-machine".into();
        target.managed_slice_relay_owner_public_key = Some(source.relay_public_key.clone());
        std::env::set_var("CHARIOX_SLICE_ID", "replay-slice");
        std::env::set_var("CHARIOX_SLICE_OWNER_KERNEL_ID", &source.daemon_id);
        std::env::set_var("CHARIOX_SLICE_OWNER_MACHINE_ID", &source.host_machine_id);
        let binary = root.path().join("opencode");
        std::fs::write(&binary, "#!/bin/sh\nprintf 'fixture-opencode\\n'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("CHARIOX_OPENCODE_BIN", binary);
        let app = Arc::new(Mutex::new(DaemonApp::bootstrap(target.clone()).unwrap()));
        let router =
            Arc::new(crate::runtime::router::CommandRouter::with_interactive_capacity(app, 2));
        let registry = router
            .runtime_state()
            .provider_account_profile_registry()
            .clone();
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing, _priority, _events) = RelayOutgoingSender::channel(32);
        let mut materialization = ProviderAccountMaterialization {
            copy_source: Some(ProviderAccountCopySource {
                machine_id: source.host_machine_id.clone(),
                kernel_id: source.daemon_id.clone(),
            }),
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: "local".into(),
                provider: "opencode".into(),
                profile_id: "replay-account".into(),
                label: "Synthetic copy".into(),
                origin: ProviderAccountProfileOrigin::CharioxCreated,
                is_default: false,
            },
            files: vec![ProviderAccountMaterializationFile {
                relative_path: "data/opencode/auth.json".into(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(br#"{"openai":{"type":"api","key":"synthetic"}}"#),
            }],
            generated_at_ms: 1,
        };
        let frame = relay_crypto::encrypt_payload_for_peer(
            &source.relay_private_key,
            &target.relay_public_key,
            &serde_json::to_vec(&RelayPeerRequest::ImportManagedSliceProviderAccountCopy {
                slice_id: "replay-slice".into(),
                materialization: materialization.clone(),
            })
            .unwrap(),
        )
        .unwrap();
        let imported = super::super::peer_requests::handle_daemon_peer_request(
            &router,
            &state,
            &outgoing,
            &source.daemon_id,
            None,
            frame.clone(),
        )
        .await;
        assert!(
            imported.error.is_none(),
            "initial import: {:?}",
            imported.error
        );
        materialization.generated_at_ms = 2;
        let no_op_frame = relay_crypto::encrypt_payload_for_peer(
            &source.relay_private_key,
            &target.relay_public_key,
            &serde_json::to_vec(&RelayPeerRequest::ImportManagedSliceProviderAccountCopy {
                slice_id: "replay-slice".into(),
                materialization,
            })
            .unwrap(),
        )
        .unwrap();
        let no_op = super::super::peer_requests::handle_daemon_peer_request(
            &router,
            &state,
            &outgoing,
            &source.daemon_id,
            None,
            no_op_frame.clone(),
        )
        .await;
        assert!(
            no_op.error.is_none(),
            "a fresh owner import may preserve the receiving login"
        );
        let environment = registry
            .resolve_environment("local", "opencode", "replay-account")
            .unwrap();
        let auth = std::path::Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json");
        assert!(auth.is_file());
        // The Docker removal helper removes this official artifact. Keep persisted
        // copy metadata and replay the exact encrypted frame.
        std::fs::remove_file(&auth).unwrap();
        let replay = super::super::peer_requests::handle_daemon_peer_request(
            &router,
            &state,
            &outgoing,
            &source.daemon_id,
            None,
            frame,
        )
        .await;
        assert!(
            !auth.exists(),
            "relay replay restored the removed credential"
        );
        assert!(replay.error.is_some(), "stale import must be rejected");
        let replay = super::super::peer_requests::handle_daemon_peer_request(
            &router,
            &state,
            &outgoing,
            &source.daemon_id,
            None,
            no_op_frame,
        )
        .await;
        assert!(
            replay.error.is_some(),
            "a no-op import must also be consumed"
        );
        assert!(
            !auth.exists(),
            "replaying a no-op request restored the credential"
        );
    });
}

/// Opt-in real path: persistent kernel homes, a real websocket relay and the
/// existing official Codex login. Never opens/copies provider files in the drill.
#[test]
#[ignore = "requires explicit persistent runtime/evidence roots and a real linked Codex login"]
fn secrev_f2_live_linked_codex_replay_after_local_removal() {
    let arguments: Vec<_> = std::env::args().collect();
    assert!(arguments.iter().any(|argument| argument == "--exact")
        && arguments.iter().any(|argument| argument ==
            "transport::relay_client::tests::managed_copy_security::secrev_f2_live_linked_codex_replay_after_local_removal"),
        "run this live drill alone with its exact test name");
    // A real-login drill must retain its inherited provider environment. Running
    // alone avoids neighboring fixtures; never hold EnvGuard over async provider
    // work, whose blocking thread must acquire the same environment lock.
    run_async_with_large_test_stack("live-codex-copy-replay", || async {
        let _guard = relay_client_test_guard().await;
        let root = std::path::PathBuf::from(
            std::env::var("CREDCOPIES_LIVE_REPLAY_ROOT").expect("explicit runtime root"),
        );
        let evidence = std::path::PathBuf::from(
            std::env::var("CREDCOPIES_LIVE_REPLAY_EVIDENCE").expect("explicit evidence path"),
        );
        assert!(
            root.is_absolute()
                && root.starts_with(
                    std::path::Path::new(&std::env::var("HOME").unwrap()).join(".chariox/dev")
                )
        );
        assert!(evidence.is_absolute() && !evidence.starts_with(std::env::current_dir().unwrap()));
        let native_home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        for (key, _) in std::env::vars_os()
            .filter(|(key, _)| key.to_string_lossy().starts_with("CHARIOX_"))
            .collect::<Vec<_>>()
        {
            std::env::remove_var(key);
        }
        let source_native_status = std::process::Command::new("codex")
            .args(["login", "status"])
            .output()
            .unwrap();
        assert!(
            source_native_status.status.success(),
            "official source Codex login is required"
        );
        std::env::set_var("CHARIOX_HOME", root.join("source"));
        std::env::set_var("CHARIOX_KERNEL_PORT", "58581");
        let source = DaemonConfig::load_from_env();
        let source_app = DaemonApp::bootstrap(source.clone()).unwrap();
        let source_registry = source_app.provider_account_profile_registry();
        let linked = source_registry
            .import_native_default("local", "codex", &native_home)
            .unwrap();
        // The production Chariox exporter owns all credential access and transfer.
        let materialization = source_registry
            .export_managed_context_materialization("local", "codex", &linked.profile_id)
            .unwrap();
        let account = materialization.profile.profile_id.clone();
        let receiving_home = root.join("receiver/provider-home");
        std::fs::create_dir_all(&receiving_home).unwrap();
        std::env::set_var("HOME", &receiving_home);
        std::env::remove_var("CODEX_HOME");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::set_var("CHARIOX_HOME", root.join("receiver"));
        std::env::set_var("CHARIOX_KERNEL_PORT", "58582");
        std::env::set_var("CHARIOX_CODEX_PORT", "58782");
        let mut target = DaemonConfig::load_from_env();
        assert_ne!(source.host_machine_id, target.host_machine_id);
        target.managed_slice_relay_owner_public_key = Some(source.relay_public_key.clone());
        std::env::set_var("CHARIOX_SLICE_ID", "live-security-receiver");
        std::env::set_var("CHARIOX_SLICE_OWNER_KERNEL_ID", &source.daemon_id);
        std::env::set_var("CHARIOX_SLICE_OWNER_MACHINE_ID", &source.host_machine_id);
        let mut router = Arc::new(
            crate::runtime::router::CommandRouter::with_interactive_capacity(
                Arc::new(Mutex::new(DaemonApp::bootstrap(target.clone()).unwrap())),
                2,
            ),
        );
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing, _priority, _events) = RelayOutgoingSender::channel(32);
        let server = Arc::new(RelayServer::new(RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: Some("live-copy-replay".into()),
        }));
        let listener = server.bind_listener().await.unwrap();
        let address = listener.local_addr().unwrap();
        let presence = server.registry();
        let (stop, stopped) = oneshot::channel();
        let relay = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let mut source_socket = register_live_copy_peer(address, &source).await;
        let mut target_socket = register_live_copy_peer(address, &target).await;
        wait_for_daemon_registration(presence.clone(), &source.daemon_id).await;
        wait_for_daemon_registration(presence, &target.daemon_id).await;
        let first_generation = materialization.generated_at_ms;
        source_registry
            .prepare_account_copy(
                "local",
                &materialization,
                ProviderAccountMaterializationTargetKind::Slice,
                &target.host_machine_id,
                &target.daemon_id,
            )
            .unwrap();
        let frame = relay_crypto::encrypt_payload_for_peer(
            &source.relay_private_key,
            &target.relay_public_key,
            &serde_json::to_vec(&RelayPeerRequest::ImportManagedSliceProviderAccountCopy {
                slice_id: "live-security-receiver".into(),
                materialization,
            })
            .unwrap(),
        )
        .unwrap();
        let initial = deliver_live_copy_frame(
            &mut source_socket,
            &mut target_socket,
            &target,
            &router,
            &state,
            &outgoing,
            "initial",
            frame.clone(),
        )
        .await;
        assert!(
            initial.error.is_none(),
            "real receiving kernel rejected initial copy"
        );
        let registry = router
            .runtime_state()
            .provider_account_profile_registry()
            .clone();
        let environment = registry
            .resolve_environment("local", "codex", &account)
            .unwrap();
        let codex_home = &environment["CODEX_HOME"];
        let before = std::process::Command::new("codex")
            .args(["login", "status"])
            .env("CODEX_HOME", codex_home)
            .output()
            .unwrap();
        assert!(
            before.status.success(),
            "official receiving Codex CLI did not see the copied login"
        );
        sleep(Duration::from_millis(2)).await;
        let no_op_materialization = source_registry
            .export_managed_context_materialization("local", "codex", &account)
            .unwrap();
        assert!(no_op_materialization.generated_at_ms > first_generation);
        let retry_expected = source_registry
            .prepare_account_copy(
                "local",
                &no_op_materialization,
                ProviderAccountMaterializationTargetKind::Slice,
                &target.host_machine_id,
                &target.daemon_id,
            )
            .unwrap();
        // The first successful receipt deliberately remains unconfirmed at home.
        assert!(!source_registry
            .get("local", "codex", &account)
            .unwrap()
            .materializations
            .iter()
            .filter_map(|status| status.copy.as_ref())
            .any(|copy| copy.copied_at_ms == first_generation));
        let no_op_frame = relay_crypto::encrypt_payload_for_peer(
            &source.relay_private_key,
            &target.relay_public_key,
            &serde_json::to_vec(&RelayPeerRequest::ImportManagedSliceProviderAccountCopy {
                slice_id: "live-security-receiver".into(),
                materialization: no_op_materialization,
            })
            .unwrap(),
        )
        .unwrap();
        let no_op = deliver_live_copy_frame(
            &mut source_socket,
            &mut target_socket,
            &target,
            &router,
            &state,
            &outgoing,
            "preserve",
            no_op_frame.clone(),
        )
        .await;
        assert!(
            no_op.error.is_none(),
            "fresh owner request should preserve the receiving login"
        );
        let decrypted = relay_crypto::decrypt_payload_for_private_key(
            &source.relay_private_key,
            no_op.encrypted_response.as_ref().unwrap(),
        )
        .unwrap();
        let RelayPeerResponse::ManagedSliceProviderAccountCopyImported { profile: received } =
            serde_json::from_slice::<RelayPeerResponse>(&decrypted.plaintext).unwrap()
        else {
            panic!("receiver did not acknowledge the retry")
        };
        let copy = received
            .materializations
            .iter()
            .find(|status| status.copy.is_some())
            .unwrap()
            .clone();
        assert_eq!(copy.copy.as_ref().unwrap().copied_at_ms, first_generation);
        source_registry
            .record_confirmed_account_copy(
                "local",
                &retry_expected,
                ProviderAccountMaterializationTargetKind::Slice,
                &target.host_machine_id,
                &target.daemon_id,
                &received.profile_id,
                copy,
            )
            .unwrap();
        assert!(source_registry
            .get("local", "codex", &account)
            .unwrap()
            .is_installed_at(
                ProviderAccountMaterializationTargetKind::Slice,
                &target.daemon_id
            ));
        // Delete only the Chariox-created receiving profile through the kernel.
        // This is local removal: never invoke provider logout/revocation/re-login.
        let removed = crate::runtime::provider_account_control::execute_provider_account_request(
            &router.runtime_state(),
            "local",
            serde_json::from_value(serde_json::json!({
                "DeleteProviderAccountProfileData": {
                    "provider": "codex",
                    "account_profile": account,
                    "confirmation_profile_id": account,
                }
            }))
            .unwrap(),
        )
        .await
        .expect("kernel must remove only its managed receiving profile");
        assert!(matches!(
            removed,
            crate::local::LocalDaemonResponse::ProviderAccountProfileDataDeleted { .. }
        ));
        for observation in registry
            .received_copy_observations("local", &source.daemon_id)
            .unwrap()
        {
            source_registry
                .apply_remote_account_copy_observation(
                    "local",
                    "codex",
                    &account,
                    ProviderAccountMaterializationTargetKind::Slice,
                    &target.host_machine_id,
                    &target.daemon_id,
                    &observation,
                )
                .unwrap();
        }
        assert!(!source_registry
            .get("local", "codex", &account)
            .unwrap()
            .is_installed_at(
                ProviderAccountMaterializationTargetKind::Slice,
                &target.daemon_id
            ));
        assert!(registry.get("local", "codex", &account).is_err());
        assert!(!std::path::Path::new(codex_home).join("auth.json").exists());
        let replay = deliver_live_copy_frame(
            &mut source_socket,
            &mut target_socket,
            &target,
            &router,
            &state,
            &outgoing,
            "replay",
            frame,
        )
        .await;
        assert!(replay
            .error
            .as_ref()
            .is_some_and(|error| error.message.contains("stale account copy import")));
        assert!(
            !std::path::Path::new(codex_home).join("auth.json").exists(),
            "relay replay restored a real credential"
        );
        drop(registry);
        drop(router);
        // Same retained identity and registry; no pin substitution or credential recopy.
        router = Arc::new(
            crate::runtime::router::CommandRouter::with_interactive_capacity(
                Arc::new(Mutex::new(DaemonApp::bootstrap(target.clone()).unwrap())),
                2,
            ),
        );
        let replay_no_op = deliver_live_copy_frame(
            &mut source_socket,
            &mut target_socket,
            &target,
            &router,
            &state,
            &outgoing,
            "replay-after-restart",
            no_op_frame,
        )
        .await;
        assert!(replay_no_op
            .error
            .as_ref()
            .is_some_and(|error| error.message.contains("stale account copy import")));
        assert!(!std::path::Path::new(codex_home).join("auth.json").exists());
        source_socket.close(None).await.unwrap();
        target_socket.close(None).await.unwrap();
        stop.send(()).unwrap();
        relay.await.unwrap();
        let receipt = serde_json::json!({"local_protocol": crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, "peer_protocol": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION, "official_source_login":true, "official_receiving_login_before_local_removal":true, "receiving_profile_removed":true,"first_receipt_left_unconfirmed":true,"preserved_first_generation_reconciled":true,"home_observed_local_removal":true, "official_logout_invoked":false, "original_frame_rejected":true, "no_op_frame_rejected_after_kernel_restart":true, "credential_restored":false, "source_machine":source.host_machine_id,"source_kernel":source.daemon_id,"receiving_machine":target.host_machine_id,"receiving_kernel":target.daemon_id,"receiving_account":account,"transport":"real websocket relay; production encrypted peer dispatcher","placement":"home-managed receiving kernel on this host with pinned slice bootstrap; no Docker container","runtime_state":"retained; receiving copy removed locally by kernel-managed profile deletion; provider login never revoked"});
        crate::config::write_private_file(&evidence, &serde_json::to_vec_pretty(&receipt).unwrap())
            .unwrap();
        println!("live Codex replay-after-removal: PASS; credential-free receipt written");
    });
}

type LiveCopySocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn register_live_copy_peer(
    address: std::net::SocketAddr,
    config: &DaemonConfig,
) -> LiveCopySocket {
    let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
    let registration = chariox_relay::protocol::DaemonRegistration {
        auth_token: "live-copy-replay".into(),
        daemon_id: config.daemon_id.clone(),
        machine_id: config.host_machine_id.clone(),
        machine_alias: None,
        os_name: Some(config.os_name.clone()),
        kernel_started_at_ms: crate::session::unix_epoch_ms(),
        daemon_alias: None,
        kernel_alias: None,
        public_key: config.relay_public_key.clone(),
        capabilities: vec!["relay_peer_transport".into()],
        available_providers: vec!["codex".into()],
        provider_accounts: Vec::new(),
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister { registration })
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    socket
}

async fn deliver_live_copy_frame(
    source: &mut LiveCopySocket,
    target_socket: &mut LiveCopySocket,
    target: &DaemonConfig,
    router: &Arc<crate::runtime::router::CommandRouter>,
    state: &Arc<RwLock<RelayClientState>>,
    outgoing: &RelayOutgoingSender,
    request_id: &str,
    frame: chariox_relay::protocol::EncryptedRelayPayload,
) -> super::super::daemon_requests::RelayRequestOutcome {
    target_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonHeartbeat {
                daemon_id: target.daemon_id.clone(),
                registration: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    sleep(Duration::from_millis(25)).await;
    source
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                request_id: request_id.into(),
                target: ClientTarget {
                    daemon_id: Some(target.daemon_id.clone()),
                    daemon_alias: None,
                },
                encrypted_request: frame,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let (relay_request_id, from_daemon_id, caller_identity, encrypted_request) =
        tokio::time::timeout(Duration::from_secs(15), async {
            while let Some(message) = target_socket.next().await {
                let Ok(Message::Text(text)) = message else {
                    continue;
                };
                if let RelayEnvelope::DaemonIncomingPeerRequest {
                    relay_request_id,
                    from_daemon_id,
                    caller_identity,
                    encrypted_request,
                } = serde_json::from_str(&text).unwrap()
                {
                    return (
                        relay_request_id,
                        from_daemon_id,
                        caller_identity,
                        encrypted_request,
                    );
                }
            }
            panic!("real relay closed before delivering account copy");
        })
        .await
        .expect("real relay peer delivery remained bounded");
    // Provider-native observation has its own bounds; transport timeout must not
    // interrupt a credential installation that is already running.
    let request = super::super::peer_requests::handle_daemon_peer_request(
        router,
        state,
        outgoing,
        &from_daemon_id,
        caller_identity,
        encrypted_request,
    );
    tokio::pin!(request);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    let outcome = loop {
        tokio::select! {
            outcome = &mut request => break outcome,
            _ = heartbeat.tick() => {
                for (socket, daemon_id) in [
                    (&mut *source, &from_daemon_id),
                    (&mut *target_socket, &target.daemon_id),
                ] {
                    socket.send(Message::Text(serde_json::to_string(
                        &RelayEnvelope::DaemonHeartbeat {
                            daemon_id: daemon_id.clone(), registration: None,
                        },
                    ).unwrap().into())).await.unwrap();
                }
            }
        }
    };
    target_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonIncomingPeerResponse {
                relay_request_id,
                encrypted_response: outcome.encrypted_response.clone(),
                error: outcome.error.clone(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(message) = source.next().await {
            let Ok(Message::Text(text)) = message else {
                continue;
            };
            if let RelayEnvelope::DaemonPeerResponse {
                request_id: returned_id,
                ..
            } = serde_json::from_str(&text).unwrap()
            {
                if returned_id == request_id {
                    return;
                }
            }
        }
        panic!("real relay closed before returning the copy receipt");
    })
    .await
    .expect("real relay peer receipt remained bounded");
    outcome
}
