//! MP-08 / MP-11: ordinary enrolled kernels issue only key-bound terminal grants.
use super::support::*;
use chariox_relay::auth::{
    encode_scoped_hmac_token, RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims,
};
use chariox_relay::protocol::DaemonRegistration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn claims(subject: &str, kind: RelaySubjectKind, target: &str, pin: &str) -> RelayTokenClaims {
    let now = crate::session::unix_epoch_ms();
    RelayTokenClaims {
        issuer: "pairing-fixture".into(),
        subject: subject.into(),
        subject_kind: kind,
        realm_id: "realm-fixture".into(),
        allowed_actions: if kind == RelaySubjectKind::Kernel {
            vec![
                RelayAction::DaemonRegister,
                RelayAction::DaemonHeartbeat,
                RelayAction::PacketRoute,
            ]
        } else {
            vec![RelayAction::ClientConnect, RelayAction::PacketRoute]
        },
        allowed_targets: Some(vec![target.into()]),
        issued_at_ms: now,
        expires_at_ms: now + 300_000,
        token_id: format!("fixture-{subject}"),
        account_id: Some("account-fixture".into()),
        organization_id: None,
        user_id: Some("local".into()),
        device_id: None,
        machine_id: Some("machine-fixture".into()),
        client_id: (kind == RelaySubjectKind::Client).then(|| subject.into()),
        session_id: None,
        public_key_thumbprint: Some(pin.into()),
        entitlements_version: None,
    }
}

#[test]
fn cloud_enrolled_kernel_pairing_creates_safe_links_and_redeems_over_scoped_relay() {
    run_async_with_large_test_stack("cloud-terminal-pairing", || async {
        let _relay_guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let root = std::path::PathBuf::from(std::env::var_os("CHARIOX_HOME").unwrap());
        let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("sessions"));
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        let terminal_private = relay_crypto::generate_private_key_base64();
        let terminal_public =
            relay_crypto::public_key_from_private_key_base64(&terminal_private).unwrap();
        let pin = crate::runtime::terminal_pairings::public_key_thumbprint(&terminal_public);
        let secret = "synthetic-pairing-fixture-secret";
        let verifier = RelayAuthVerifier::scoped_hmac(
            BTreeMap::from([("pairing-fixture".into(), secret.into())]),
            None,
        );
        let server = Arc::new(RelayServer::with_auth_verifier(
            RelayConfig {
                host: "127.0.0.1".into(),
                port: 0,
                shared_token: None,
            },
            verifier,
        ));
        let listener = server.bind_listener().await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (stop, stopped) = oneshot::channel();
        let serving = {
            let server = server.clone();
            tokio::spawn(async move {
                server
                    .run_listener_until(listener, async {
                        let _ = stopped.await;
                    })
                    .await
                    .unwrap()
            })
        };
        let cloud = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}", cloud.local_addr().unwrap());
        let target = config.daemon_id.clone();
        let cloud_pin = pin.clone();
        let cloud_target = target.clone();
        let issuing = tokio::spawn(async move {
            let (mut socket, _) = cloud.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (end, length) = loop {
                let mut chunk = [0u8; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let header = std::str::from_utf8(&bytes[..end]).unwrap();
                    assert!(header.starts_with("POST /relay/token "));
                    let length = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    break (end + 4, length);
                }
            };
            while bytes.len() < end + length {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&bytes[end..end + length]).unwrap();
            assert_eq!(body["kernelCredential"], "synthetic-kernel-grant");
            assert!(body.get("sessionToken").is_none());
            assert!(body.get("machineCredential").is_none());
            assert_eq!(body["subjectKind"], "client");
            assert_eq!(body["publicKeyThumbprint"], cloud_pin);
            assert_eq!(body["allowedTargets"], serde_json::json!([cloud_target]));
            let issued = encode_scoped_hmac_token(
                &claims(
                    body["subject"].as_str().unwrap(),
                    RelaySubjectKind::Client,
                    &cloud_target,
                    &cloud_pin,
                ),
                secret,
            )
            .unwrap();
            let response =
                serde_json::json!({"token":issued,"expiresAt":"2099-01-01T00:00:00Z"}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        });
        config.relay_url = Some(url.clone());
        config.relay_token = Some(
            encode_scoped_hmac_token(
                &claims(
                    &target,
                    RelaySubjectKind::Kernel,
                    &target,
                    &crate::runtime::terminal_pairings::public_key_thumbprint(
                        &config.relay_public_key,
                    ),
                ),
                secret,
            )
            .unwrap(),
        );
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url,
            account_id: "account-fixture".into(),
            realm_id: "realm-fixture".into(),
            user_id: "local".into(),
            relay_url: url.clone(),
            kernel_id: Some(target.clone()),
            kernel_credential: Some("synthetic-kernel-grant".into()),
            machine_id: Some("machine-fixture".into()),
            ..Default::default()
        });
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        let router = Arc::new(
            crate::runtime::router::CommandRouter::with_interactive_capacity(
                Arc::new(Mutex::new(app)),
                8,
            ),
        );
        let create = LocalDaemonRequest::CreateTerminalPairingLink(
            crate::local::CreateTerminalPairingLinkRequest {
                terminal_type: None,
                alias: None,
                expires_in_ms: Some(60_000),
            },
        );
        let response = router
            .dispatch(
                KernelCommand::from_local_request("create", None, None, &create),
                create,
            )
            .await
            .unwrap();
        let LocalDaemonResponse::TerminalPairingLinkCreated { pairing } = response else {
            panic!("expected link")
        };
        let token =
            crate::runtime::invite_tokens::decode_pairing_invite_token(&pairing.pairing_link)
                .unwrap();
        assert!(
            token.relay_token == "cloud-client-token-required",
            "no kernel transport authority leaves the kernel"
        );
        let (mut kernel, _) = connect_async(&url).await.unwrap();
        send_client_envelope(
            &mut kernel,
            &RelayEnvelope::DaemonRegister {
                registration: DaemonRegistration {
                    auth_token: config.relay_token.clone().unwrap(),
                    daemon_id: target.clone(),
                    machine_id: "machine-fixture".into(),
                    machine_alias: None,
                    os_name: None,
                    kernel_started_at_ms: 0,
                    daemon_alias: None,
                    kernel_alias: None,
                    public_key: config.relay_public_key.clone(),
                    capabilities: vec![],
                    available_providers: vec![],
                    provider_accounts: vec![],
                    accepting_remote_leases: false,
                    leased_agent_count: 0,
                    local_session_count: 0,
                },
            },
        )
        .await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while server
                .registry()
                .read()
                .await
                .daemon_in_realm("realm-fixture", &target)
                .is_none()
            {
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("kernel registered in its scoped realm");
        let pump = {
            let router = router.clone();
            tokio::spawn(async move {
                let cache = Arc::new(
                    crate::runtime_transport::command_cache::CommandResultCache::default(),
                );
                while let Some(Ok(Message::Text(text))) = kernel.next().await {
                    if let RelayEnvelope::DaemonRequest {
                        relay_request_id,
                        caller_identity,
                        encrypted_request,
                    } = serde_json::from_str(&text).unwrap()
                    {
                        let result = super::super::daemon_requests::handle_daemon_request(
                            &router,
                            &std::sync::atomic::AtomicU64::new(1),
                            caller_identity,
                            encrypted_request,
                            &cache,
                        )
                        .await;
                        send_client_envelope(
                            &mut kernel,
                            &RelayEnvelope::DaemonResponse {
                                relay_request_id,
                                encrypted_response: result.encrypted_response,
                                error: result.error,
                            },
                        )
                        .await;
                    }
                }
            })
        };
        // The receiver supplies its own CLIENT transport authority; the link
        // never tries to export an unkeyed grant using the kernel credential.
        let bootstrap = encode_scoped_hmac_token(
            &claims("receiver", RelaySubjectKind::Client, &target, &pin),
            secret,
        )
        .unwrap();
        // MP-08 / MP-11: a session collaborator's key/target pins do not
        // authorize spending the owner's enrollment credential, for forged
        // links or for links that have been revoked.
        let revoked_create = LocalDaemonRequest::CreateTerminalPairingLink(
            crate::local::CreateTerminalPairingLinkRequest {
                terminal_type: None,
                alias: None,
                expires_in_ms: Some(60_000),
            },
        );
        let revoked_response = router
            .dispatch(
                KernelCommand::from_local_request("create-revoked", None, None, &revoked_create),
                revoked_create,
            )
            .await
            .unwrap();
        let LocalDaemonResponse::TerminalPairingLinkCreated { pairing: revoked } = revoked_response
        else {
            panic!("expected link")
        };
        crate::config::DaemonConfig::revoke_paired_client(&revoked.terminal_id).unwrap();
        let mut forged = token.clone();
        forged.invite_id = "fabricated".into();
        let forged_link =
            crate::runtime::invite_tokens::encode_terminal_pairing_link(&forged).unwrap();
        for link in [forged_link, revoked.pairing_link] {
            let mut viewer_claims =
                claims("shared-viewer", RelaySubjectKind::Client, &target, &pin);
            viewer_claims.user_id = Some("collaborator".into());
            viewer_claims.session_id = Some("shared-session".into());
            let (mut viewer, _) = connect_async(&url).await.unwrap();
            send_client_envelope(
                &mut viewer,
                &RelayEnvelope::ClientConnect {
                    auth_token: encode_scoped_hmac_token(&viewer_claims, secret).unwrap(),
                    target: ClientTarget {
                        daemon_id: Some(target.clone()),
                        daemon_alias: None,
                    },
                },
            )
            .await;
            let public = expect_client_connected(&mut viewer).await;
            let request = LocalDaemonRequest::JoinTerminalPairingLink(
                crate::local::JoinTerminalPairingLinkRequest {
                    pairing_link: link,
                    terminal_id: Some("viewer-terminal".into()),
                    terminal_type: None,
                    alias: None,
                    public_key_thumbprint: Some(pin.clone()),
                },
            );
            send_client_envelope(
                &mut viewer,
                &RelayEnvelope::ClientRequest {
                    request_id: "deny-owner-grant".into(),
                    target: ClientTarget {
                        daemon_id: Some(target.clone()),
                        daemon_alias: None,
                    },
                    encrypted_request: relay_crypto::encrypt_payload_for_peer(
                        &terminal_private,
                        &public,
                        &serde_json::to_vec(&request).unwrap(),
                    )
                    .unwrap(),
                },
            )
            .await;
            let error = tokio::time::timeout(
                Duration::from_secs(5),
                expect_client_response_error(&mut viewer, "deny-owner-grant"),
            )
            .await
            .unwrap();
            assert!(
                error.message.contains("only this kernel's owner"),
                "owner gate is the first failing seam"
            );
            viewer.close(None).await.unwrap();
        }
        let (mut client, _) = connect_async(&url).await.unwrap();
        let client_target = ClientTarget {
            daemon_id: Some(target.clone()),
            daemon_alias: None,
        };
        send_client_envelope(
            &mut client,
            &RelayEnvelope::ClientConnect {
                auth_token: bootstrap,
                target: client_target.clone(),
            },
        )
        .await;
        let kernel_public = expect_client_connected(&mut client).await;
        let join = LocalDaemonRequest::JoinTerminalPairingLink(
            crate::local::JoinTerminalPairingLinkRequest {
                pairing_link: pairing.pairing_link,
                terminal_id: Some(pairing.terminal_id),
                terminal_type: None,
                alias: None,
                public_key_thumbprint: Some(pin.clone()),
            },
        );
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &terminal_private,
            &kernel_public,
            &serde_json::to_vec(&join).unwrap(),
        )
        .unwrap();
        send_client_envelope(
            &mut client,
            &RelayEnvelope::ClientRequest {
                request_id: "redeem".into(),
                target: client_target.clone(),
                encrypted_request: encrypted,
            },
        )
        .await;
        let joined = tokio::time::timeout(
            Duration::from_secs(10),
            expect_client_response(&mut client, "redeem", &terminal_private),
        )
        .await
        .unwrap();
        let LocalDaemonResponse::TerminalPairingLinkJoined {
            relay_token: Some(grant),
            pairing: joined,
            ..
        } = joined
        else {
            panic!("expected fresh terminal token")
        };
        assert!(joined.subject_id.starts_with("kernel-client:"));
        assert_eq!(joined.public_key_thumbprint, pin);
        issuing.await.unwrap();
        client.close(None).await.unwrap();
        let (mut reconnected, _) = connect_async(&url).await.unwrap();
        send_client_envelope(
            &mut reconnected,
            &RelayEnvelope::ClientConnect {
                auth_token: grant,
                target: client_target,
            },
        )
        .await;
        expect_client_connected(&mut reconnected).await;
        reconnected.close(None).await.unwrap();
        pump.abort();
        let _ = pump.await;
        let _ = stop.send(());
        serving.await.unwrap();
    });
}
