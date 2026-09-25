use super::*;
use chariox_relay::protocol::RelayEnvelope;
use tokio::sync::mpsc;

struct RoomManifestFixture {
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    relay_state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    priority_rx: mpsc::Receiver<RelayEnvelope>,
    worker_private_key: String,
    home_public_key: String,
}

async fn room_manifest_fixture() -> RoomManifestFixture {
    let relay_url = "ws://127.0.0.1:1".to_string();
    let mut home_config = crate::config::DaemonConfig::for_tests();
    home_config.relay_url = Some(relay_url.clone());
    home_config.relay_token = Some("room-manifest-test-token".to_string());
    let home_public_key = home_config.relay_public_key.clone();
    let worker_config = crate::config::DaemonConfig::for_tests();
    let worker_private_key = worker_config.relay_private_key.clone();
    let (app, runtime, session_id, agent_id) =
        agent_config_runtime_with_config(home_config).await;
    app.lock()
        .await
        .agents_mut()
        .bind_remote_execution(
            &agent_id,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-1".to_string(),
                worker_machine_id: "machine-1".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("provider-run-current".to_string()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("agent should bind to an active worker run");

    let relay_state = app.lock().await.relay_client_state();
    let (outgoing_tx, priority_rx, _event_rx) =
        crate::transport::relay_client::RelayOutgoingSender::channel(8);
    {
        let mut relay = relay_state.write().await;
        relay.test_set_connected_sender(outgoing_tx, relay_url);
        relay.remember_peer_public_key("worker-1", worker_config.relay_public_key.clone());
    }

    RoomManifestFixture {
        runtime,
        session_id,
        agent_id,
        relay_state,
        priority_rx,
        worker_private_key,
        home_public_key,
    }
}

fn create_room_slice(runtime: &KernelRuntimeState, name: &str) -> crate::slice::SliceRecord {
    runtime
        .owned
        .slice_store
        .create(
            "home-kernel",
            "home-machine",
            crate::slice::CreateSliceInput {
                name: name.to_string(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: crate::slice::SliceDisplayMode::Headed,
                display_backend: crate::slice::SliceDisplayBackend::Selkies,
                workspace_id: None,
                worktree_id: None,
                workspace_mount: None,
                development: None,
                worker_kernel_ref: Some("worker-1".to_string()),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .expect("Room slice should be created")
}

async fn next_manifest_update(
    fixture: &mut RoomManifestFixture,
) -> (String, crate::extension::RemoteExtensionManifest) {
    let envelope = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        fixture.priority_rx.recv(),
    )
    .await
    .expect("manifest refresh should be queued")
    .expect("connected relay queue should remain open");
    let RelayEnvelope::DaemonPeerRequest {
        request_id,
        encrypted_request,
        ..
    } = envelope
    else {
        panic!("expected a daemon peer request");
    };
    let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        &fixture.worker_private_key,
        &encrypted_request,
    )
    .expect("worker should decrypt the manifest update");
    let crate::transport::relay_peer::RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
        leased_agent_id,
        remote_extension_manifest,
    } = serde_json::from_slice(&decrypted.plaintext).expect("manifest request should decode")
    else {
        panic!("expected a manifest update, not a prompt request");
    };
    assert_eq!(leased_agent_id, "leased-agent-1");
    (request_id, remote_extension_manifest)
}

async fn acknowledge_manifest_update(fixture: &RoomManifestFixture, request_id: String) {
    let response =
        crate::transport::relay_peer::RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated {
            leased_agent_id: "leased-agent-1".to_string(),
        };
    let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &fixture.worker_private_key,
        &fixture.home_public_key,
        &serde_json::to_vec(&response).expect("manifest response should encode"),
    )
    .expect("worker should encrypt the manifest response");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &fixture.relay_state,
        request_id,
        "worker-1".to_string(),
        encrypted,
    )
    .await;
}

#[tokio::test]
async fn bind_and_delete_refresh_an_already_leased_agent_without_a_prompt() {
    let mut fixture = room_manifest_fixture().await;
    let slice = create_room_slice(&fixture.runtime, "room-manifest-bind-delete");
    fixture
        .runtime
        .bind_room_environment_slice(
            crate::local::BindRoomEnvironmentSliceRequest {
                session_id: fixture.session_id.clone(),
                slice_ref: slice.id.clone(),
            },
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("Room slice should bind");

    let (request_id, manifest) = next_manifest_update(&mut fixture).await;
    assert!(manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, request_id).await;

    fixture
        .runtime
        .delete_slice(&slice.id)
        .expect("bound Room slice should delete");
    let (request_id, manifest) = next_manifest_update(&mut fixture).await;
    assert!(!manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, request_id).await;
    assert!(fixture
        .runtime
        .room_environment_slice(&fixture.session_id)
        .expect("Room should remain available")
        .is_none());
}

#[tokio::test]
async fn bind_and_delete_enqueue_refresh_without_a_current_tokio_runtime() {
    let mut fixture = room_manifest_fixture().await;
    let slice = create_room_slice(&fixture.runtime, "room-manifest-no-runtime");
    let runtime = fixture.runtime.clone();
    let session_id = fixture.session_id.clone();
    let slice_ref = slice.id.clone();
    std::thread::spawn(move || {
        runtime.bind_room_environment_slice(
            crate::local::BindRoomEnvironmentSliceRequest {
                session_id,
                slice_ref,
            },
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
    })
    .join()
    .expect("bind thread should complete")
    .expect("Room slice should bind without a Tokio runtime");

    let (request_id, manifest) = next_manifest_update(&mut fixture).await;
    assert!(manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, request_id).await;

    let runtime = fixture.runtime.clone();
    let slice_ref = slice.id.clone();
    std::thread::spawn(move || runtime.delete_slice(&slice_ref))
        .join()
        .expect("delete thread should complete")
        .expect("bound Room slice should delete without a Tokio runtime");

    let (request_id, manifest) = next_manifest_update(&mut fixture).await;
    assert!(!manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, request_id).await;
}

#[tokio::test]
async fn delayed_bind_refresh_recomputes_after_delete_before_sending() {
    let mut fixture = room_manifest_fixture().await;
    let slice = create_room_slice(&fixture.runtime, "room-manifest-delayed-refresh");
    let lane = fixture
        .runtime
        .leased_agent_operations
        .lock("leased-agent-1")
        .await;
    let bind_waiting = fixture
        .runtime
        .leased_agent_operations
        .notify_on_next_acquire_for_tests("leased-agent-1");

    fixture
        .runtime
        .bind_room_environment_slice(
            crate::local::BindRoomEnvironmentSliceRequest {
                session_id: fixture.session_id.clone(),
                slice_ref: slice.id.clone(),
            },
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("Room slice should bind");
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        bind_waiting.notified(),
    )
    .await
    .expect("bind refresh should reach the per-agent lane");

    let delete_waiting = fixture
        .runtime
        .leased_agent_operations
        .notify_on_next_acquire_for_tests("leased-agent-1");
    fixture
        .runtime
        .delete_slice(&slice.id)
        .expect("bound Room slice should delete");
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        delete_waiting.notified(),
    )
    .await
    .expect("delete refresh should wait in the same per-agent lane");
    drop(lane);

    let (first_request, first_manifest) = next_manifest_update(&mut fixture).await;
    assert!(!first_manifest.room_browser_available);
    assert!(fixture.priority_rx.try_recv().is_err());
    acknowledge_manifest_update(&fixture, first_request).await;

    let (second_request, second_manifest) = next_manifest_update(&mut fixture).await;
    assert!(!second_manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, second_request).await;
}

#[tokio::test]
async fn failed_bind_and_delete_do_not_enqueue_manifest_refreshes() {
    let mut fixture = room_manifest_fixture().await;
    let missing_bind = fixture.runtime.bind_room_environment_slice(
        crate::local::BindRoomEnvironmentSliceRequest {
            session_id: fixture.session_id.clone(),
            slice_ref: "missing-room-slice".to_string(),
        },
        crate::session::DEFAULT_LOCAL_USER_ID,
    );
    assert!(missing_bind.is_err());

    let slice = create_room_slice(&fixture.runtime, "room-manifest-failed-delete");
    fixture
        .runtime
        .owned
        .slice_store
        .bind_environment(&fixture.session_id, &slice.id, 2, |_| Ok(()))
        .expect("test should bind the slice directly");
    fixture
        .runtime
        .owned
        .slice_store
        .attach_agent(&slice.id, &fixture.session_id, &fixture.agent_id, 3)
        .expect("test should mark the slice as having an active agent");
    assert!(fixture.runtime.delete_slice(&slice.id).is_err());
    assert!(fixture
        .runtime
        .room_environment_slice(&fixture.session_id)
        .expect("Room should remain available")
        .is_some());
    assert!(fixture.priority_rx.try_recv().is_err());
}
