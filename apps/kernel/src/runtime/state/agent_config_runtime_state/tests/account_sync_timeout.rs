//! MP-08 / MP-10 / MP-11: exercise the actual off-lock profile sender budget.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_profile_account_sync_waits_off_lock_past_the_ordinary_deadline() {
    let mut config = crate::config::DaemonConfig::for_tests();
    let worker = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some("ws://127.0.0.1:1".into());
    config.relay_token = Some("account-sync-fixture-admission".into());
    config.relay_request_timeout_ms = 50;
    let home_public_key = config.relay_public_key.clone();
    let (app, runtime, _, _) = agent_config_runtime_with_config(config.clone()).await;
    let relay_state = app.lock().await.relay_client_state();
    let (sender, mut replies, _events) =
        crate::transport::relay_client::RelayOutgoingSender::channel(4);
    {
        let mut state = relay_state.write().await;
        state.test_set_connected_sender(sender, "ws://127.0.0.1:1");
        state.remember_peer_public_key("worker", worker.relay_public_key.clone());
    }
    let task = tokio::spawn(async move {
        runtime
            .send_remote_profile_request(
                &config,
                "worker",
                crate::transport::relay_peer::RelayPeerRequest::EnsureRemoteProviderAccount {
                    context: crate::transport::relay_peer::RemoteProviderAccountSyncContext {
                        home_kernel_id: "home".into(),
                        home_session_id: "session".into(),
                        home_agent_id: "agent".into(),
                        execution_lease_id: "lease".into(),
                    },
                    materialization: crate::account_profile::ProviderAccountMaterialization {
                        profile: crate::account_profile::ProviderAccountReplicaMetadata {
                            owner_user_id: "owner".into(),
                            provider: "opencode".into(),
                            profile_id: "go".into(),
                            label: "Go".into(),
                            origin: crate::account_profile::ProviderAccountProfileOrigin::Default,
                            is_default: true,
                        },
                        files: vec![],
                        generated_at_ms: 0,
                    },
                },
            )
            .await
    });
    let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), replies.recv())
        .await
        .unwrap()
        .unwrap();
    let chariox_relay::protocol::RelayEnvelope::DaemonPeerRequest { request_id, .. } = envelope
    else {
        panic!("expected encrypted account sync");
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        app.try_lock().is_ok(),
        "profile account sync held the app mutex"
    );
    let reply = crate::transport::relay_peer::RelayPeerResponse::RemoteProviderAccountEnsured {
        provider: "opencode".into(),
        account_profile: "go".into(),
    };
    let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &worker.relay_private_key,
        &home_public_key,
        &serde_json::to_vec(&reply).unwrap(),
    )
    .unwrap();
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &relay_state,
        request_id,
        "worker".into(),
        encrypted,
    )
    .await;
    assert!(matches!(
        task.await.unwrap().unwrap(),
        crate::transport::relay_peer::RelayPeerResponse::RemoteProviderAccountEnsured { .. }
    ));
}
