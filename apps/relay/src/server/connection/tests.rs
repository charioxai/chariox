use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::sync::{mpsc, RwLock};

use super::support::*;
use super::*;
use crate::auth::DEFAULT_RELAY_REALM_ID;
use crate::protocol::{ClientTarget, DaemonRegistration, EncryptedRelayPayload};
use crate::registry::{PendingClientRequest, RelaySender};

#[test]
fn outgoing_queue_capacity_override_is_positive_and_defaults_safely() {
    assert_eq!(parse_relay_outgoing_queue_capacity(Some("32")), 32);
    assert_eq!(
        parse_relay_outgoing_queue_capacity(None),
        DEFAULT_RELAY_OUTGOING_QUEUE_CAPACITY
    );
    assert_eq!(
        parse_relay_outgoing_queue_capacity(Some("0")),
        DEFAULT_RELAY_OUTGOING_QUEUE_CAPACITY
    );
    assert_eq!(
        parse_relay_outgoing_queue_capacity(Some("invalid")),
        DEFAULT_RELAY_OUTGOING_QUEUE_CAPACITY
    );
}

#[tokio::test]
async fn saturated_display_viewer_is_closed_without_blocking_the_daemon_lane() {
    let registry = Arc::new(RwLock::new(RelayRegistry::default()));
    let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "worker-1");
    let (display_tx, _display_rx) = mpsc::channel(1);
    display_tx
        .try_send(DisplayStreamEvent::Chunk {
            data: "first".to_string(),
            message_kind: Some("binary".to_string()),
        })
        .expect("first display packet should fill the queue");
    registry.write().await.insert_pending_display_stream(
        "stream-1".to_string(),
        daemon_key.clone(),
        display_tx,
    );
    let (daemon_tx, mut daemon_rx) = mpsc::channel(2);

    try_forward_display_stream_event(
        &registry,
        &daemon_tx,
        &daemon_key,
        "stream-1",
        DisplayStreamEvent::Chunk {
            data: "second".to_string(),
            message_kind: Some("binary".to_string()),
        },
    )
    .await;

    assert!(registry
        .read()
        .await
        .display_stream_sender_for_daemon("stream-1", &daemon_key)
        .is_none());
    let message = daemon_rx
        .recv()
        .await
        .expect("daemon should receive a bounded backpressure close");
    let Message::Text(text) = message else {
        panic!("expected a relay text envelope");
    };
    assert!(matches!(
        serde_json::from_str::<RelayEnvelope>(&text).expect("close envelope should decode"),
        RelayEnvelope::DaemonDisplayTunnelClientClose {
            stream_id,
            error: Some(error),
        } if stream_id == "stream-1" && error.code == "display_stream_backpressure"
    ));
}

#[tokio::test]
async fn saturated_display_viewer_observes_channel_close_after_daemon_terminal_event() {
    let registry = Arc::new(RwLock::new(RelayRegistry::default()));
    let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "worker-1");
    let (display_tx, mut display_rx) = mpsc::channel(1);
    display_tx
        .try_send(DisplayStreamEvent::Chunk {
            data: "last-frame".to_string(),
            message_kind: Some("binary".to_string()),
        })
        .expect("last display packet should fill the queue");
    registry.write().await.insert_pending_display_stream(
        "stream-1".to_string(),
        daemon_key.clone(),
        display_tx,
    );

    close_display_stream_from_daemon(&registry, &daemon_key, "stream-1", None).await;

    assert!(matches!(
        display_rx.recv().await,
        Some(DisplayStreamEvent::Chunk { data, .. }) if data == "last-frame"
    ));
    assert!(display_rx.recv().await.is_none());
    assert!(registry
        .read()
        .await
        .display_stream_sender_for_daemon("stream-1", &daemon_key)
        .is_none());
}

fn peer_addr(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn daemon_registration(daemon_id: &str) -> DaemonRegistration {
    DaemonRegistration {
        auth_token: "token".to_string(),
        daemon_id: daemon_id.to_string(),
        machine_id: "machine-1".to_string(),
        machine_alias: None,
        os_name: None,
        kernel_started_at_ms: 0,
        daemon_alias: None,
        kernel_alias: None,
        public_key: "public-key".to_string(),
        capabilities: Vec::new(),
        available_providers: Vec::new(),
        provider_accounts: Vec::new(),
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
    }
}

fn daemon_peer(sender: RelaySender, registration: DaemonRegistration) -> PeerHandle {
    PeerHandle {
        sender,
        role: RelayConnectionRole::Daemon,
        realm_id: Some(DEFAULT_RELAY_REALM_ID.to_string()),
        identity: None,
        allowed_actions: Vec::new(),
        daemon_registration: Some(registration),
        client_daemon_key: None,
    }
}

fn client_peer(sender: RelaySender) -> PeerHandle {
    PeerHandle {
        sender,
        role: RelayConnectionRole::Client,
        realm_id: Some(DEFAULT_RELAY_REALM_ID.to_string()),
        identity: None,
        allowed_actions: Vec::new(),
        daemon_registration: None,
        client_daemon_key: None,
    }
}

#[test]
fn resolve_daemon_sender_uses_daemon_peer_index() {
    let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-1");
    let registration = daemon_registration("daemon-1");
    let daemon_addr = peer_addr(10_001);
    let (sender, _receiver) = mpsc::channel::<Message>(1);
    let mut registry = RelayRegistry::default();
    registry
        .daemons
        .insert(daemon_key.clone(), registration.clone());
    registry
        .peers
        .insert(daemon_addr, daemon_peer(sender, registration));
    registry
        .daemon_peers
        .insert(daemon_key.clone(), daemon_addr);

    assert!(resolve_daemon_sender_locked(&registry, &daemon_key).is_some());

    registry
        .daemon_peers
        .insert(daemon_key.clone(), peer_addr(10_002));

    assert!(
        resolve_daemon_sender_locked(&registry, &daemon_key).is_none(),
        "stale route index entries must not fall back to scanning all peers"
    );
}

#[test]
fn performance_drill_relay_fanout_resolves_daemon_routes_from_index() {
    let mut registry = RelayRegistry::default();
    for index in 0..2_000 {
        let daemon_id = format!("daemon-{index}");
        let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, daemon_id.clone());
        let registration = daemon_registration(&daemon_id);
        let addr = peer_addr(20_000 + index as u16);
        let (sender, _receiver) = mpsc::channel::<Message>(1);
        registry
            .daemons
            .insert(daemon_key.clone(), registration.clone());
        registry
            .peers
            .insert(addr, daemon_peer(sender, registration));
        registry.daemon_peers.insert(daemon_key, addr);
    }

    let target_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-1999");
    assert!(resolve_daemon_sender_locked(&registry, &target_key).is_some());
    assert_eq!(registry.daemon_peers.len(), registry.daemons.len());

    registry
        .daemon_peers
        .insert(target_key.clone(), peer_addr(65_000));
    assert!(
        resolve_daemon_sender_locked(&registry, &target_key).is_none(),
        "indexed routing must not scan all relay peers when an index entry is stale"
    );
}

#[tokio::test]
async fn remove_daemon_peer_clears_daemon_route_index() {
    let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-1");
    let registration = daemon_registration("daemon-1");
    let peer_addr = peer_addr(10_003);
    let (sender, _receiver) = mpsc::channel::<Message>(1);
    let mut registry = RelayRegistry::default();
    registry
        .daemons
        .insert(daemon_key.clone(), registration.clone());
    registry
        .peers
        .insert(peer_addr, daemon_peer(sender, registration));
    registry.daemon_peers.insert(daemon_key.clone(), peer_addr);
    let routes = registry.route_index();
    let registry = Arc::new(RwLock::new(registry));
    let relay_request_counter = AtomicU64::new(0);

    let _ = remove_peer(
        &registry,
        &routes,
        peer_addr,
        Some(&daemon_key),
        &relay_request_counter,
    )
    .await;

    let guard = registry.read().await;
    assert!(!guard.daemons.contains_key(&daemon_key));
    assert!(!guard.daemon_peers.contains_key(&daemon_key));
    assert!(!guard.peers.contains_key(&peer_addr));
}

#[tokio::test]
async fn remove_client_peer_unsubscribes_active_and_pending_daemon_subscriptions() {
    let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-1");
    let client_addr = peer_addr(10_007);
    let other_client_addr = peer_addr(10_008);
    let (client_sender, _client_receiver) = mpsc::channel::<Message>(4);
    let (other_client_sender, _other_client_receiver) = mpsc::channel::<Message>(4);
    let (daemon_sender, mut daemon_receiver) = mpsc::channel::<Message>(4);
    let mut registry = RelayRegistry::default();
    registry
        .peers
        .insert(client_addr, client_peer(client_sender.clone()));
    registry
        .peers
        .insert(other_client_addr, client_peer(other_client_sender));
    registry.subscriptions.insert(
        "active-subscription".to_string(),
        ActiveSubscription {
            client_addr,
            daemon_key: daemon_key.clone(),
            client_public_key: "active-public-key".to_string(),
        },
    );
    registry.subscriptions.insert(
        "other-subscription".to_string(),
        ActiveSubscription {
            client_addr: other_client_addr,
            daemon_key: daemon_key.clone(),
            client_public_key: "other-public-key".to_string(),
        },
    );
    let routes = registry.route_index();
    routes.set_client_sender(client_addr, client_sender.clone());
    routes.set_daemon_sender(daemon_key.clone(), daemon_sender);
    routes.set_subscription(
        "active-subscription".to_string(),
        ActiveEventRoute {
            daemon_key: daemon_key.clone(),
            client_sender,
        },
    );
    routes.insert_pending_client(
        "relay-request-pending".to_string(),
        PendingClientRequest {
            client_addr,
            client_request_id: "client-request-pending".to_string(),
            daemon_key: daemon_key.clone(),
            kind: PendingRequestKind::Subscribe {
                subscription_id: "pending-subscription".to_string(),
                client_public_key: "pending-public-key".to_string(),
            },
        },
    );
    let registry = Arc::new(RwLock::new(registry));
    let relay_request_counter = AtomicU64::new(0);

    let (_, _, _, _, cleanup_count, dropped_pending_count) = remove_peer(
        &registry,
        &routes,
        client_addr,
        None,
        &relay_request_counter,
    )
    .await;

    assert_eq!(cleanup_count, 2);
    assert_eq!(dropped_pending_count, 1);
    let guard = registry.read().await;
    assert!(!guard.subscriptions.contains_key("active-subscription"));
    assert!(guard.subscriptions.contains_key("other-subscription"));
    drop(guard);
    let mut cleanups = (0..2)
        .map(|_| {
            match daemon_receiver
                .try_recv()
                .expect("daemon cleanup should enqueue")
            {
                Message::Text(text) => match serde_json::from_str::<RelayEnvelope>(&text)
                    .expect("daemon cleanup should decode")
                {
                    RelayEnvelope::DaemonUnsubscribe {
                        relay_subscription_id,
                        client_public_key,
                        ..
                    } => (relay_subscription_id, client_public_key),
                    other => panic!("expected daemon unsubscribe, got {other:?}"),
                },
                other => panic!("expected daemon cleanup text, got {other:?}"),
            }
        })
        .collect::<Vec<_>>();
    cleanups.sort();
    assert_eq!(
        cleanups,
        vec![
            (
                "active-subscription".to_string(),
                "active-public-key".to_string()
            ),
            (
                "pending-subscription".to_string(),
                "pending-public-key".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn alias_resolution_ignores_temporary_peer_transport_registrations() {
    let real_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "home-kernel");
    let temp_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "home-kernel:peer-tmp:req-1");
    let mut real = daemon_registration("home-kernel");
    real.daemon_alias = Some("home".to_string());
    real.capabilities = vec!["kernel_websocket".to_string()];
    let mut temporary = daemon_registration("home-kernel:peer-tmp:req-1");
    temporary.daemon_alias = Some("home".to_string());
    temporary.capabilities = vec!["relay_peer_transport".to_string()];
    let mut registry = RelayRegistry::default();
    registry.daemons.insert(real_key.clone(), real);
    registry.daemons.insert(temp_key, temporary);
    let real_addr = peer_addr(10_006);
    let real_registration = registry
        .daemons
        .get(&real_key)
        .expect("real registration should exist")
        .clone();
    let (real_sender, _receiver) = mpsc::channel::<Message>(1);
    registry.peers.insert(
        real_addr,
        daemon_peer(real_sender.clone(), real_registration),
    );
    registry.daemon_peers.insert(real_key.clone(), real_addr);
    registry
        .route_index()
        .set_daemon_sender(real_key.clone(), real_sender);
    let registry = Arc::new(RwLock::new(registry));

    let resolved = resolve_target_daemon_key(
        &registry,
        DEFAULT_RELAY_REALM_ID,
        &ClientTarget {
            daemon_id: None,
            daemon_alias: Some("home".to_string()),
        },
    )
    .await;

    assert_eq!(resolved, Some(real_key));
}

#[tokio::test]
async fn target_resolution_and_live_metadata_ignore_stale_daemon_registration() {
    let stale_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-stale");
    let live_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-live");
    let mut stale_registration = daemon_registration("daemon-stale");
    stale_registration.daemon_alias = Some("stale".to_string());
    stale_registration.capabilities = vec!["kernel_ws".to_string()];
    let mut live_registration = daemon_registration("daemon-live");
    live_registration.daemon_alias = Some("live".to_string());
    live_registration.capabilities = vec!["kernel_ws".to_string()];
    let live_addr = peer_addr(10_006);
    let (live_sender, _receiver) = mpsc::channel::<Message>(1);
    let mut registry = RelayRegistry::default();
    registry
        .daemons
        .insert(stale_key.clone(), stale_registration);
    registry
        .daemons
        .insert(live_key.clone(), live_registration.clone());
    registry.peers.insert(
        live_addr,
        daemon_peer(live_sender.clone(), live_registration),
    );
    registry.daemon_peers.insert(live_key.clone(), live_addr);
    registry
        .route_index()
        .set_daemon_sender(live_key.clone(), live_sender);
    let registry = Arc::new(RwLock::new(registry));

    let stale_exact = resolve_target_daemon_key(
        &registry,
        DEFAULT_RELAY_REALM_ID,
        &ClientTarget {
            daemon_id: Some("daemon-stale".to_string()),
            daemon_alias: None,
        },
    )
    .await;
    let stale_alias = resolve_target_daemon_key(
        &registry,
        DEFAULT_RELAY_REALM_ID,
        &ClientTarget {
            daemon_id: None,
            daemon_alias: Some("stale".to_string()),
        },
    )
    .await;
    let live_exact = resolve_target_daemon_key(
        &registry,
        DEFAULT_RELAY_REALM_ID,
        &ClientTarget {
            daemon_id: Some("daemon-live".to_string()),
            daemon_alias: None,
        },
    )
    .await;

    let guard = registry.read().await;
    assert_eq!(stale_exact, None);
    assert_eq!(stale_alias, None);
    assert_eq!(live_exact, Some(live_key));
    assert_eq!(
        guard.live_machines_in_realm(DEFAULT_RELAY_REALM_ID).len(),
        1
    );
    assert_eq!(
        guard.live_kernel_in_realm(DEFAULT_RELAY_REALM_ID, "daemon-stale"),
        None
    );
    assert_eq!(
        guard
            .live_kernel_in_realm(DEFAULT_RELAY_REALM_ID, "daemon-live")
            .expect("live daemon should remain visible")
            .kernel_id,
        "daemon-live"
    );
}

#[tokio::test]
async fn slow_event_consumer_cleanup_removes_matching_subscription_only() {
    let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-1");
    let other_daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-2");
    let client_addr = peer_addr(10_004);
    let other_client_addr = peer_addr(10_005);
    let (sender, mut receiver) = mpsc::channel::<Message>(1);
    sender
        .try_send(Message::Text("occupied".to_string().into()))
        .expect("test queue should accept first message");
    let (other_sender, _other_receiver) = mpsc::channel::<Message>(1);
    let mut registry = RelayRegistry::default();
    registry
        .peers
        .insert(client_addr, client_peer(sender.clone()));
    registry
        .peers
        .insert(other_client_addr, client_peer(other_sender));
    registry.subscriptions.insert(
        "slow-subscription".to_string(),
        ActiveSubscription {
            client_addr,
            daemon_key: daemon_key.clone(),
            client_public_key: "slow-public-key".to_string(),
        },
    );
    registry.subscriptions.insert(
        "other-subscription".to_string(),
        ActiveSubscription {
            client_addr: other_client_addr,
            daemon_key: other_daemon_key.clone(),
            client_public_key: "other-public-key".to_string(),
        },
    );
    let routes = registry.route_index();
    let (daemon_sender, mut daemon_receiver) = mpsc::channel::<Message>(1);
    routes.set_daemon_sender(daemon_key.clone(), daemon_sender);
    routes.set_client_sender(client_addr, sender.clone());
    routes.set_subscription(
        "slow-subscription".to_string(),
        ActiveEventRoute {
            daemon_key: daemon_key.clone(),
            client_sender: sender.clone(),
        },
    );
    let registry = Arc::new(RwLock::new(registry));
    let relay_request_counter = AtomicU64::new(0);

    let result = send_envelope(
        &sender,
        &RelayEnvelope::ClientEvent {
            subscription_id: "slow-subscription".to_string(),
            event_id: 1,
            encrypted_event: EncryptedRelayPayload {
                sender_public_key: "daemon-public".to_string(),
                nonce: "nonce".to_string(),
                ciphertext: "ciphertext".to_string(),
            },
        },
    );
    assert!(result.is_err(), "full client queue should reject event");

    close_slow_subscription(
        &registry,
        &routes,
        "slow-subscription",
        &daemon_key,
        &relay_request_counter,
    )
    .await;

    let guard = registry.read().await;
    assert!(!guard.subscriptions.contains_key("slow-subscription"));
    assert!(guard.subscriptions.contains_key("other-subscription"));
    assert_eq!(
        guard.backpressure_metrics().slow_subscription_close_count,
        1
    );
    assert_eq!(guard.backpressure_metrics().target_queue_full_count, 0);
    drop(guard);
    assert!(matches!(
        receiver.try_recv(),
        Ok(Message::Text(text)) if text == "occupied"
    ));
    let cleanup = daemon_receiver
        .try_recv()
        .expect("slow subscription cleanup should reach daemon");
    let Message::Text(payload) = cleanup else {
        panic!("expected daemon cleanup text")
    };
    assert!(matches!(
        serde_json::from_str::<RelayEnvelope>(&payload).expect("daemon cleanup should decode"),
        RelayEnvelope::DaemonUnsubscribe {
            relay_subscription_id,
            client_public_key,
            ..
        } if relay_subscription_id == "slow-subscription"
            && client_public_key == "slow-public-key"
    ));
}

#[tokio::test]
async fn target_backpressure_rejects_client_pending_request_without_client_close() {
    let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-1");
    let client_addr = peer_addr(10_006);
    let (client_sender, mut client_receiver) = mpsc::channel::<Message>(4);
    let registry = RelayRegistry::default();
    registry.route_index().insert_pending_client(
        "relay-request-1".to_string(),
        PendingClientRequest {
            client_addr,
            client_request_id: "client-request-1".to_string(),
            daemon_key,
            kind: PendingRequestKind::Request,
        },
    );
    let registry = Arc::new(RwLock::new(registry));

    reject_client_pending_on_target_backpressure(
        &registry,
        &client_sender,
        "relay-request-1",
        "client-request-1".to_string(),
    )
    .await
    .expect("client rejection should enqueue");

    {
        let guard = registry.read().await;
        assert_eq!(guard.pending_request_count(), 0);
        assert_eq!(guard.backpressure_metrics().target_queue_full_count, 1);
        assert_eq!(
            guard.backpressure_metrics().slow_subscription_close_count,
            0
        );
    }
    let payload = match client_receiver.try_recv() {
        Ok(Message::Text(text)) => text,
        other => panic!("unexpected client rejection frame: {other:?}"),
    };
    match serde_json::from_str::<RelayEnvelope>(&payload).expect("client rejection should decode") {
        RelayEnvelope::ClientResponse {
            request_id,
            encrypted_response: None,
            error: Some(error),
        } => {
            assert_eq!(request_id, "client-request-1");
            assert_eq!(error.code, "target_backpressure");
            assert!(error.retryable);
        }
        other => panic!("unexpected client rejection envelope: {other:?}"),
    }
}

#[tokio::test]
async fn target_backpressure_rejects_peer_pending_request_without_requester_close() {
    let requester_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-a");
    let target_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-b");
    let (requester_sender, mut requester_receiver) = mpsc::channel::<Message>(4);
    let registry = RelayRegistry::default();
    registry.route_index().insert_pending_daemon(
        "relay-peer-request-1".to_string(),
        PendingDaemonPeerRequest {
            requester_daemon_key: requester_key,
            requester_request_id: "peer-request-1".to_string(),
            target_daemon_key: target_key.clone(),
        },
    );
    let registry = Arc::new(RwLock::new(registry));

    reject_peer_pending_on_target_backpressure(
        &registry,
        &requester_sender,
        "relay-peer-request-1",
        "peer-request-1".to_string(),
        target_key.daemon_id,
    )
    .await
    .expect("peer rejection should enqueue");

    {
        let guard = registry.read().await;
        assert_eq!(guard.pending_request_count(), 0);
        assert_eq!(guard.backpressure_metrics().target_queue_full_count, 1);
        assert_eq!(
            guard.backpressure_metrics().slow_subscription_close_count,
            0
        );
    }
    let payload = match requester_receiver.try_recv() {
        Ok(Message::Text(text)) => text,
        other => panic!("unexpected peer rejection frame: {other:?}"),
    };
    match serde_json::from_str::<RelayEnvelope>(&payload).expect("peer rejection should decode") {
        RelayEnvelope::DaemonPeerResponse {
            request_id,
            from_daemon_id,
            encrypted_response: None,
            error: Some(error),
        } => {
            assert_eq!(request_id, "peer-request-1");
            assert_eq!(from_daemon_id, "daemon-b");
            assert_eq!(error.code, "target_backpressure");
            assert!(error.retryable);
        }
        other => panic!("unexpected peer rejection envelope: {other:?}"),
    }
}

#[test]
fn peer_event_target_backpressure_is_nonfatal() {
    let target_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-b");
    let target_registration = daemon_registration("daemon-b");
    let target_addr = peer_addr(10_007);
    let (target_sender, mut target_receiver) = mpsc::channel::<Message>(1);
    target_sender
        .try_send(Message::Text("occupied".to_string().into()))
        .expect("test queue should accept first message");
    let mut registry = RelayRegistry::default();
    registry
        .daemons
        .insert(target_key.clone(), target_registration.clone());
    registry.peers.insert(
        target_addr,
        daemon_peer(target_sender.clone(), target_registration),
    );
    registry
        .daemon_peers
        .insert(target_key.clone(), target_addr);
    let target_sender = resolve_daemon_sender_locked(&registry, &target_key)
        .expect("target daemon sender should resolve");

    let result = send_envelope(
        &target_sender,
        &RelayEnvelope::DaemonIncomingPeerEvent {
            from_daemon_id: "daemon-a".to_string(),
            caller_identity: None,
            encrypted_event: EncryptedRelayPayload {
                sender_public_key: "daemon-a-public".to_string(),
                nonce: "nonce".to_string(),
                ciphertext: "ciphertext".to_string(),
            },
        },
    );

    assert!(
        result.is_err(),
        "full target daemon queue should reject event"
    );
    log_daemon_sender_backpressure("daemon_peer_event", peer_addr(10_008), &target_key);
    assert_eq!(registry.pending_request_count(), 0);
    assert!(matches!(
        target_receiver.try_recv(),
        Ok(Message::Text(text)) if text == "occupied"
    ));
}
