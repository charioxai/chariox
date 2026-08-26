use super::*;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::sync::mpsc;

use crate::protocol::RelayConnectionRole;
use crate::registry::PeerHandle;

fn peer_addr(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn insert_live_registration(
    registry: &mut RelayRegistry,
    realm_id: &str,
    registration: DaemonRegistration,
    port: u16,
) -> DaemonKey {
    let key = DaemonKey::new(realm_id, registration.daemon_id.clone());
    let addr = peer_addr(port);
    let (sender, _receiver) = mpsc::channel::<Message>(1);
    registry.daemons.insert(key.clone(), registration.clone());
    registry.peers.insert(
        addr,
        PeerHandle {
            sender: sender.clone(),
            role: RelayConnectionRole::Daemon,
            realm_id: Some(realm_id.to_string()),
            identity: None,
            allowed_actions: Vec::new(),
            allowed_targets: None,
            daemon_registration: Some(registration),
            client_daemon_key: None,
        },
    );
    registry.daemon_peers.insert(key.clone(), addr);
    registry
        .route_index()
        .set_daemon_sender(key.clone(), sender);
    key
}

#[test]
fn server_revocation_registry_gates_the_attached_verifier() {
    let mut claims = BTreeMap::new();
    claims.insert(
        "client-token".to_string(),
        scoped_claim(
            "client-token-id",
            "client-1",
            RelaySubjectKind::Client,
            "realm-a",
            vec![RelayAction::ClientConnect],
            Some(vec!["daemon-1"]),
        ),
    );
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), Some(10))),
    );
    let request = || RelayAuthRequest {
        token: "client-token",
        action: RelayAction::ClientConnect,
        target: Some("daemon-1"),
    };

    server
        .auth_verifier()
        .verify(request())
        .expect("token verifies before revocation");

    // Feeding the server's registry gates the verifier the server actually
    // uses, proving the registry is wired into construction.
    server.revocations().revoke_token_id("client-token-id", 100);
    assert_eq!(
        server
            .auth_verifier()
            .verify(request())
            .expect_err("revoked token is rejected by the server verifier"),
        RelayAuthError::TokenRevoked
    );
}

#[tokio::test(flavor = "current_thread")]
async fn server_binds_listener() {
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: None,
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let local_addr = listener
        .local_addr()
        .expect("listener should have local addr");
    assert_eq!(local_addr.ip().to_string(), "127.0.0.1");
}

#[test]
fn relay_aliases_differentiate_kernels_on_same_machine() {
    let mut registry = RelayRegistry::default();
    insert_live_registration(
        &mut registry,
        DEFAULT_RELAY_REALM_ID,
        test_registration("daemon-a", "shared-machine", "macOS", 10),
        10_001,
    );
    insert_live_registration(
        &mut registry,
        DEFAULT_RELAY_REALM_ID,
        test_registration("daemon-b", "shared-machine", "macOS", 20),
        10_002,
    );

    let machines = registry.live_machines();
    assert_eq!(machines.len(), 1);
    assert_eq!(machines[0].machine_id, "shared-machine");
    assert_eq!(machines[0].kernel_count, 2);
    assert_eq!(
        machines[0].machine_alias.as_deref(),
        Some("machine 1 (macOS)")
    );

    let kernels = registry.live_kernels_for_machine("shared-machine");
    assert_eq!(kernels.len(), 2);
    assert_eq!(kernels[0].kernel_id, "daemon-a");
    assert_eq!(kernels[0].relay_alias.as_deref(), Some("machine 1 (macOS)"));
    assert_eq!(kernels[1].kernel_id, "daemon-b");
    assert_eq!(kernels[1].relay_alias.as_deref(), Some("machine 2 (macOS)"));

    let exact = registry.live_kernels_for_machine("machine 2 (macOS)");
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].kernel_id, "daemon-b");
    assert_eq!(
        registry
            .live_kernel("machine 2 (macOS)")
            .expect("relay alias should resolve to a kernel")
            .kernel_id,
        "daemon-b"
    );
}

#[test]
fn relay_registry_scopes_metadata_and_aliases_by_realm() {
    let mut registry = RelayRegistry::default();
    let mut realm_a = test_registration("daemon-a", "machine-a", "Linux", 10);
    realm_a.daemon_alias = Some("shared".to_string());
    realm_a.public_key = "public-key-a".to_string();
    let mut realm_b = test_registration("daemon-b", "machine-b", "Linux", 10);
    realm_b.daemon_alias = Some("shared".to_string());
    realm_b.public_key = "public-key-b".to_string();

    insert_live_registration(&mut registry, "realm-a", realm_a, 10_003);
    insert_live_registration(&mut registry, "realm-b", realm_b, 10_004);

    assert_eq!(registry.daemon_count(), 2);
    assert_eq!(registry.live_machines_in_realm("realm-a").len(), 1);
    assert_eq!(
        registry.live_machines_in_realm("realm-a")[0].machine_id,
        "machine-a"
    );
    assert_eq!(registry.live_machines_in_realm("realm-b").len(), 1);
    assert_eq!(
        registry.live_machines_in_realm("realm-b")[0].machine_id,
        "machine-b"
    );

    assert_eq!(
        registry
            .live_kernel_in_realm("realm-a", "shared")
            .expect("realm A alias should resolve")
            .public_key,
        "public-key-a"
    );
    assert_eq!(
        registry
            .live_kernel_in_realm("realm-b", "shared")
            .expect("realm B alias should resolve")
            .public_key,
        "public-key-b"
    );
}

#[test]
fn relay_metadata_ignores_temporary_peer_transport_registrations() {
    let mut registry = RelayRegistry::default();
    let mut home = test_registration("home-kernel", "machine-a", "Linux", 10);
    home.daemon_alias = Some("home".to_string());
    let mut temporary = test_registration("home-kernel:peer-tmp:req-1", "machine-a", "Linux", 20);
    temporary.daemon_alias = Some("home".to_string());
    temporary.kernel_alias = Some("home".to_string());
    temporary.capabilities = vec!["relay_peer_transport".to_string()];
    temporary.available_providers = Vec::new();
    temporary.accepting_remote_leases = false;

    insert_live_registration(&mut registry, DEFAULT_RELAY_REALM_ID, home, 10_005);
    registry.daemons.insert(
        DaemonKey::new(DEFAULT_RELAY_REALM_ID, "home-kernel:peer-tmp:req-1"),
        temporary,
    );

    let machines = registry.live_machines();
    assert_eq!(machines.len(), 1);
    assert_eq!(machines[0].kernel_count, 1);

    let kernels = registry.live_kernels_for_machine("machine-a");
    assert_eq!(kernels.len(), 1);
    assert_eq!(kernels[0].kernel_id, "home-kernel");

    assert_eq!(
        registry
            .live_kernel("home")
            .expect("real home alias should resolve")
            .kernel_id,
        "home-kernel"
    );
    assert!(
        registry.live_kernel("home-kernel:peer-tmp:req-1").is_none(),
        "temporary peer transport sockets are not live kernels"
    );
}
