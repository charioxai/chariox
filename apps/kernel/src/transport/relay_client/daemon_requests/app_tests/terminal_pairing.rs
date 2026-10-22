use super::*;
use crate::local::*;
use chariox_relay::auth::{
    KernelIssuerKeys, RelayAction, RelayAuthRequest, RelayAuthVerifier, SelfHostedVerifier,
};

async fn remote(
    router: &CommandRouter,
    sender: &str,
    identity: RelayCallerIdentity,
    request: LocalDaemonRequest,
) -> Value {
    let receiver =
        relay_crypto::public_key_from_private_key_base64(&router.relay_private_key()).unwrap();
    let encrypted = relay_crypto::encrypt_payload_for_peer(
        sender,
        &receiver,
        &serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    let outcome = handle_daemon_request(
        router,
        &AtomicU64::new(1),
        Some(identity),
        encrypted,
        &Arc::new(CommandResultCache::default()),
        &Default::default(),
        None,
    )
    .await;
    if let Some(error) = outcome.error {
        return serde_json::json!({"refused": error.code});
    }
    let decrypted =
        relay_crypto::decrypt_payload_for_private_key(sender, &outcome.encrypted_response.unwrap())
            .unwrap();
    serde_json::from_slice(&decrypted.plaintext).unwrap()
}

#[test]
fn two_pairing_links_with_one_cli_key_renew_through_relay_admission() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let root = TestRoot::new();
                    let mut config = root.config();
                    config.relay_url = Some("ws://127.0.0.1:1".into());
                    config.relay_token = Some("synthetic-operator-transport".into());
                    config.cloud_relay = None;
                    let app = crate::DaemonApp::bootstrap(config.clone()).unwrap();
                    let router = CommandRouter::with_interactive_capacity(
                        Arc::new(tokio::sync::Mutex::new(app)),
                        8,
                    );
                    let local = |id: &str, request: &LocalDaemonRequest| {
                        KernelCommand::from_local_request(id, None, None, request)
                    };
                    let enable =
                        LocalDaemonRequest::ConfigureRelayIssuer(ConfigureRelayIssuerRequest {
                            step: RelayIssuerStep::Enable,
                        });
                    let LocalDaemonResponse::RelayIssuer {
                        issuer: Some(issuer),
                    } = router
                        .dispatch(local("enable", &enable), enable)
                        .await
                        .unwrap()
                    else {
                        panic!("expected issuer")
                    };
                    let verifier = RelayAuthVerifier::SelfHosted(SelfHostedVerifier::new(
                        "synthetic-operator-transport".into(),
                        KernelIssuerKeys::parse(&issuer.relay_issuer_keys).unwrap(),
                        None,
                    ));
                    let authenticated = |token: &str| -> RelayCallerIdentity {
                        verifier
                            .verify(RelayAuthRequest {
                                token,
                                action: RelayAction::ClientConnect,
                                target: Some(&config.daemon_id),
                            })
                            .unwrap()
                            .into()
                    };
                    let sender = relay_crypto::generate_private_key_base64();
                    let public = relay_crypto::public_key_from_private_key_base64(&sender).unwrap();
                    let thumbprint =
                        crate::runtime::terminal_pairings::public_key_thumbprint(&public);
                    let mut identities = Vec::new();
                    for index in 0..2 {
                        let create = LocalDaemonRequest::CreateTerminalPairingLink(
                            CreateTerminalPairingLinkRequest {
                                terminal_type: Some(TerminalType::Cli),
                                alias: Some(format!("link-{index}")),
                                expires_in_ms: Some(60_000),
                            },
                        );
                        let LocalDaemonResponse::TerminalPairingLinkCreated { pairing } = router
                            .dispatch(local("create", &create), create)
                            .await
                            .unwrap()
                        else {
                            panic!("expected link")
                        };
                        let bootstrap = crate::runtime::invite_tokens::decode_pairing_invite_token(
                            &pairing.pairing_link,
                        )
                        .unwrap();
                        let joined = remote(
                            &router,
                            &sender,
                            authenticated(&bootstrap.relay_token),
                            LocalDaemonRequest::JoinTerminalPairingLink(
                                JoinTerminalPairingLinkRequest {
                                    pairing_link: pairing.pairing_link,
                                    terminal_id: Some(pairing.terminal_id.clone()),
                                    terminal_type: Some(TerminalType::Cli),
                                    alias: None,
                                    public_key_thumbprint: Some(thumbprint.clone()),
                                },
                            ),
                        )
                        .await;
                        let identity = authenticated(
                            joined["TerminalPairingLinkJoined"]["relay_token"]
                                .as_str()
                                .unwrap(),
                        );
                        assert_eq!(identity.subject, pairing.terminal_id);
                        identities.push(identity);
                    }
                    assert_ne!(identities[0].subject, identities[1].subject);
                    let renew = |id: &str| {
                        LocalDaemonRequest::IssueRelayClientToken(IssueRelayClientTokenRequest {
                            client_id: id.into(),
                            public_key_thumbprint: thumbprint.clone(),
                            scope: RelayClientTokenScope::Attach,
                            lifetime_ms: None,
                        })
                    };
                    for identity in &identities {
                        let response =
                            remote(&router, &sender, identity.clone(), renew(&identity.subject))
                                .await;
                        assert!(
                            response.get("RelayClientTokenIssued").is_some(),
                            "each link must renew through relay admission: {response}"
                        );
                    }
                    let cross = remote(
                        &router,
                        &sender,
                        identities[0].clone(),
                        renew(&identities[1].subject),
                    )
                    .await;
                    assert!(
                        cross.get("RelayClientTokenIssued").is_none(),
                        "one link cannot renew another link's subject"
                    );
                    crate::runtime::self_host_terminal_grants::revoke(
                        &config,
                        &identities[1].subject,
                    )
                    .unwrap();
                    for request in [
                        renew(&identities[1].subject),
                        LocalDaemonRequest::ListSessions(ListSessionsRequest),
                    ] {
                        let response =
                            remote(&router, &sender, identities[1].clone(), request).await;
                        assert_eq!(response["refused"], "authorization_denied");
                    }
                    let response = remote(
                        &router,
                        &sender,
                        identities[0].clone(),
                        renew(&identities[0].subject),
                    )
                    .await;
                    assert!(
                        response.get("RelayClientTokenIssued").is_some(),
                        "revoking one link preserves the other"
                    );
                    crate::runtime::self_host_terminal_grants::revoke(
                        &config,
                        &identities[0].subject,
                    )
                    .unwrap();
                    for identity in identities {
                        let response =
                            remote(&router, &sender, identity.clone(), renew(&identity.subject))
                                .await;
                        assert_eq!(
                            response["refused"], "authorization_denied",
                            "revoking every grant revokes the device"
                        );
                    }
                    router.runtime_state().shutdown_cleanup().await.unwrap();
                });
        })
        .unwrap()
        .join()
        .unwrap();
}
