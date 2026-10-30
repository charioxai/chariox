//! MP-08 / MP-09 / MP-10 / MP-11: managed worker fences use their authenticated relay.

use super::*;
use chariox_relay::protocol::RelayEnvelope;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn managed_slice_sudo_fence_uses_connected_peer_without_metadata_discovery() {
    check_connected_fence(false).await;
}

#[tokio::test]
async fn managed_slice_sudo_fence_rejects_another_peer_reply() {
    check_connected_fence(true).await;
}

async fn check_connected_fence(wrong_peer: bool) {
    let f = super::super::tests::fixture();
    let worker = crate::config::DaemonConfig::for_tests();
    let home_public_key = f.state.config_snapshot().await.relay_public_key;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("ws://{}", listener.local_addr().unwrap());
    let metadata_queries = Arc::new(AtomicUsize::new(0));
    let queries = Arc::clone(&metadata_queries);
    // A live managed worker need not appear in the Cloud directory grant.
    let metadata = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            let RelayEnvelope::ClientMetadataRequest { request_id, .. } =
                serde_json::from_str(message.to_text().unwrap()).unwrap()
            else {
                panic!("unexpected metadata request");
            };
            queries.fetch_add(1, Ordering::Relaxed);
            let response = RelayEnvelope::ClientMetadataResponse {
                request_id,
                machines: None,
                kernels: None,
                kernel: None,
                error: None,
            };
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::to_string(&response).unwrap().into(),
                ))
                .await
                .unwrap();
            let _ = socket.close(None).await;
        }
    });
    let worker_id = "slice:owner-machine:managed-worker";
    let agent_id = f.request.target_agent_id.as_deref().unwrap();
    f.state
        .owned
        .agent_store
        .bind_remote_execution(
            agent_id,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker_id.into(),
                worker_machine_id: "owner-machine".into(),
                execution_lease_id: "execution-lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: None,
                relay_url: Some(relay_url.clone()),
                relay_token: Some("fixture-relay-token".into()),
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let relay_state = Arc::clone(&f.state.owned.relay_state);
    let (sender, mut requests, _events) =
        crate::transport::relay_client::RelayOutgoingSender::channel(4);
    {
        let mut relay = relay_state.write().await;
        relay.test_set_connected_sender(sender, &relay_url);
        // Hosted slice activation already authenticated and pinned this key.
        assert!(relay.claim_peer_public_key(worker_id, &worker.relay_public_key));
    }
    let responder = tokio::spawn(async move {
        let Some(RelayEnvelope::DaemonPeerRequest {
            request_id,
            target,
            encrypted_request,
            ..
        }) = requests.recv().await
        else {
            panic!("expected the fence on the connected relay");
        };
        assert_eq!(target.daemon_id.as_deref(), Some(worker_id));
        let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
            &worker.relay_private_key,
            &encrypted_request,
        )
        .unwrap();
        let request: RelayPeerRequest = serde_json::from_slice(&decrypted.plaintext).unwrap();
        let RelayPeerRequest::UpdateLeasedSudo {
            leased_agent_id,
            home_prompt_id,
            grant,
        } = request
        else {
            panic!("expected the leased sudo fence");
        };
        assert_eq!(leased_agent_id, "leased-agent");
        assert_eq!(home_prompt_id, "home-prompt");
        assert_eq!(grant.entry_id, "sudo-window");
        assert_eq!(grant.remaining_ms, 60_000);
        let response = crate::transport::relay_crypto::encrypt_payload_for_peer(
            &worker.relay_private_key,
            &home_public_key,
            &serde_json::to_vec(&RelayPeerResponse::LeasedSudoUpdated).unwrap(),
        )
        .unwrap();
        crate::transport::relay_client::resolve_pending_peer_response_for_test(
            &relay_state,
            request_id,
            if wrong_peer {
                "another-worker"
            } else {
                worker_id
            }
            .into(),
            response,
        )
        .await;
    });
    let result = f
        .state
        .push_leased_sudo(
            agent_id,
            "home-prompt",
            LeasedSudoGrant {
                entry_id: "sudo-window".into(),
                revision: 0,
                remaining_ms: 60_000,
                initial: true,
            },
        )
        .await;
    metadata.abort();
    let _ = metadata.await;
    if wrong_peer {
        assert!(
            result.unwrap_err().to_string().contains("another kernel"),
            "a fence cannot be acknowledged by another worker"
        );
    } else {
        assert!(
            result.is_ok(),
            "a registered managed peer must not need owner-directory metadata discovery: {result:?}"
        );
    }
    responder.await.unwrap();
    assert_eq!(metadata_queries.load(Ordering::Relaxed), 0);
}
