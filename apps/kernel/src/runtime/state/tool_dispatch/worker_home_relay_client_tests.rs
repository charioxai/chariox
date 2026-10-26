use super::*;
use crate::transport::{relay_client, relay_crypto};

#[test]
fn mp08_mp10_standalone_slice_home_pin_requires_its_provisioned_owner() {
    let mut worker = crate::config::DaemonConfig::for_tests();
    let home = crate::config::DaemonConfig::for_tests();
    worker.managed_slice_relay_owner_public_key = Some(home.relay_public_key.clone());
    let identity = crate::runtime::router::ManagedSliceRelayIdentity {
        slice_id: "owned-slice".into(),
        relay_subject: worker.daemon_id.clone(),
        owner_kernel_id: home.daemon_id.clone(),
        owner_machine_id: worker.host_machine_id.clone(),
    };
    assert_eq!(
        managed_slice_home_owner_public_key_for_identity(&worker, &home.daemon_id, Some(&identity))
            .unwrap(),
        Some(home.relay_public_key.clone())
    );
    assert!(managed_slice_home_owner_public_key_for_identity(
        &worker,
        "foreign-home",
        Some(&identity)
    )
    .is_err());
    assert!(
        managed_slice_home_owner_public_key_for_identity(&worker, &home.daemon_id, None).is_err()
    );
    worker.room_environment_worker_binding = Some(crate::config::RoomEnvironmentWorkerBinding {
        home_kernel_id: "foreign-home".into(),
        home_public_key: home.relay_public_key,
        session_id: "room-session".into(),
        slice_id: "owned-slice".into(),
        provisioned_slice_id: Some("owned-slice".into()),
    });
    assert!(managed_slice_home_owner_public_key_for_identity(
        &worker,
        &home.daemon_id,
        Some(&identity)
    )
    .is_err());
}

#[tokio::test]
async fn mp08_mp10_slice_home_request_uses_provisioned_pin_without_cloud_metadata() {
    let mut worker = crate::config::DaemonConfig::for_tests();
    let home = crate::config::DaemonConfig::for_tests();
    worker.relay_url = Some("ws://127.0.0.1:1".into());
    worker.relay_token = Some("worker-registration-fixture".into());
    worker.cloud_relay = None;
    worker.managed_slice_relay_owner_public_key = Some(home.relay_public_key.clone());
    worker.room_environment_worker_binding = Some(crate::config::RoomEnvironmentWorkerBinding {
        home_kernel_id: home.daemon_id.clone(),
        home_public_key: home.relay_public_key.clone(),
        session_id: "home-session".into(),
        slice_id: "owned-slice".into(),
        provisioned_slice_id: Some("owned-slice".into()),
    });
    let app = crate::DaemonApp::bootstrap(worker.clone()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let runtime = router.runtime_state();
    let state = runtime.owned.relay_state.clone();
    let (sender, mut incoming, _events) = relay_client::RelayOutgoingSender::channel(4);
    state
        .write()
        .await
        .test_set_connected_sender(sender, "ws://127.0.0.1:1");
    let responder_state = state.clone();
    let worker_public_key = worker.relay_public_key.clone();
    let home_id = home.daemon_id.clone();
    let responder = tokio::spawn(async move {
        let chariox_relay::protocol::RelayEnvelope::DaemonPeerRequest {
            request_id,
            encrypted_request,
            ..
        } = incoming.recv().await.unwrap()
        else {
            panic!("expected peer request")
        };
        let decrypted = relay_crypto::decrypt_payload_for_private_key(
            &home.relay_private_key,
            &encrypted_request,
        )
        .unwrap();
        assert!(matches!(
            serde_json::from_slice::<RelayPeerRequest>(&decrypted.plaintext).unwrap(),
            RelayPeerRequest::Ping { .. }
        ));
        let response = RelayPeerResponse::Pong {
            value: "public-pin".into(),
            daemon_id: home.daemon_id.clone(),
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &home.relay_private_key,
            &worker_public_key,
            &serde_json::to_vec(&response).unwrap(),
        )
        .unwrap();
        relay_client::resolve_pending_peer_response_for_test(
            &responder_state,
            request_id,
            home.daemon_id,
            encrypted,
        )
        .await;
        incoming
    });
    let result = runtime
        .send_worker_home_runtime_request(
            &worker,
            &home_id,
            RelayPeerRequest::Ping {
                value: "public-pin".into(),
            },
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert!(matches!(result, RelayPeerResponse::Pong { value, .. } if value == "public-pin"));
    let mut incoming = responder.await.unwrap();
    // Reject foreign targets and changed pins before anything enters transport.
    for foreign in [false, true] {
        let mut invalid = worker.clone();
        let target = if foreign {
            "foreign-home"
        } else {
            home_id.as_str()
        };
        if !foreign {
            invalid
                .room_environment_worker_binding
                .as_mut()
                .unwrap()
                .home_public_key = worker.relay_public_key.clone();
        }
        let error = runtime
            .send_worker_home_runtime_request(
                &invalid,
                target,
                RelayPeerRequest::Ping {
                    value: "rejected".into(),
                },
                Duration::from_secs(5),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DaemonError::LocalTransport {
                operation: "forward managed slice runtime tool",
                ..
            }
        ));
        assert!(incoming.try_recv().is_err());
    }
}

// MP-08 / MP-10 / MP-11: an authenticated HOME key remains usable when a fresh metadata connection cannot discover it.
#[tokio::test]
async fn mp10_ordinary_worker_home_request_reuses_its_authenticated_peer_key() {
    let mut worker = crate::config::DaemonConfig::for_tests();
    let mut home = crate::config::DaemonConfig::for_tests();
    worker.daemon_id = "ordinary-worker".into();
    home.daemon_id = "ordinary-home".into();
    worker.relay_url = Some("ws://127.0.0.1:1".into());
    worker.relay_token = Some("worker-registration-fixture".into());
    worker.cloud_relay = None;
    let app = crate::DaemonApp::bootstrap(worker.clone()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let runtime = router.runtime_state();
    let state = runtime.owned.relay_state.clone();
    let (sender, mut incoming, _events) = relay_client::RelayOutgoingSender::channel(4);
    state
        .write()
        .await
        .test_set_connected_sender(sender, "ws://127.0.0.1:1");
    state
        .write()
        .await
        .remember_peer_public_key(home.daemon_id.clone(), home.relay_public_key.clone());
    let responder_state = state.clone();
    let worker_public_key = worker.relay_public_key.clone();
    let home_id = home.daemon_id.clone();
    let responder = tokio::spawn(async move {
        let chariox_relay::protocol::RelayEnvelope::DaemonPeerRequest {
            request_id,
            encrypted_request,
            ..
        } = incoming.recv().await.unwrap()
        else {
            panic!("expected peer request")
        };
        let decrypted = relay_crypto::decrypt_payload_for_private_key(
            &home.relay_private_key,
            &encrypted_request,
        )
        .unwrap();
        assert!(matches!(
            serde_json::from_slice::<RelayPeerRequest>(&decrypted.plaintext).unwrap(),
            RelayPeerRequest::Ping { .. }
        ));
        let response = RelayPeerResponse::Pong {
            value: "cached-home-key".into(),
            daemon_id: home.daemon_id.clone(),
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &home.relay_private_key,
            &worker_public_key,
            &serde_json::to_vec(&response).unwrap(),
        )
        .unwrap();
        relay_client::resolve_pending_peer_response_for_test(
            &responder_state,
            request_id,
            home.daemon_id,
            encrypted,
        )
        .await;
        incoming
    });
    let result = runtime
        .send_worker_home_runtime_request(
            &worker,
            &home_id,
            RelayPeerRequest::Ping {
                value: "cached-home-key".into(),
            },
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert!(matches!(result, RelayPeerResponse::Pong { value, .. } if value == "cached-home-key"));
    responder.await.unwrap();
}
