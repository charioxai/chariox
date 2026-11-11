use super::*;
use crate::local::{
    GetEventConnectionRequest, ListEventConnectionDependenciesRequest,
    ListEventConnectionEventsRequest, ObserveEventConnectionAuthorizationRequest,
    ReconnectEventConnectionRequest, RefreshEventConnectionRequest, RemoveEventConnectionRequest,
    ReplayEventConnectionEventRequest,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct ReadyConnectionServer {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    revoked: Arc<AtomicBool>,
    unavailable: Arc<AtomicBool>,
    scopes_reduced: Arc<AtomicBool>,
    management_url_discovered: Arc<AtomicBool>,
    management_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ReadyConnectionServer {
    fn start() -> Self {
        Self::start_with_management_url(|address| format!("http://{address}"))
    }

    fn start_with_management_url(
        management_url: impl FnOnce(std::net::SocketAddr) -> String,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let management_url = management_url(address);
        let stop = Arc::new(AtomicBool::new(false));
        let revoked = Arc::new(AtomicBool::new(false));
        let unavailable = Arc::new(AtomicBool::new(false));
        let scopes_reduced = Arc::new(AtomicBool::new(false));
        let management_url_discovered = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_revoked = Arc::clone(&revoked);
        let thread_unavailable = Arc::clone(&unavailable);
        let thread_scopes_reduced = Arc::clone(&scopes_reduced);
        let thread_management_url_discovered = Arc::clone(&management_url_discovered);
        let thread_management_url = management_url.clone();
        let thread_requests = Arc::clone(&requests);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        serve_ready_connection(
                            &mut stream,
                            &thread_revoked,
                            &thread_unavailable,
                            &thread_scopes_reduced,
                            &thread_management_url_discovered,
                            &thread_management_url,
                            &thread_requests,
                        )
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("event connection test server failed: {error}"),
                }
            }
        });
        Self {
            address,
            stop,
            revoked,
            unavailable,
            scopes_reduced,
            management_url_discovered,
            management_url,
            requests,
            thread: Some(thread),
        }
    }

    fn target(&self) -> crate::config::EventGeneratorManagementTarget {
        crate::config::EventGeneratorManagementTarget {
            url: format!("http://{}", self.address),
            registry_discovered: false,
        }
    }
}

impl Drop for ReadyConnectionServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

#[derive(Debug)]
struct PassthroughTlsConnector;

impl ureq::TlsConnector for PassthroughTlsConnector {
    fn connect(
        &self,
        _dns_name: &str,
        stream: Box<dyn ureq::ReadWrite>,
    ) -> Result<Box<dyn ureq::ReadWrite>, ureq::Error> {
        Ok(stream)
    }
}

fn dynamic_registry_config(server_url: String) -> crate::DaemonConfig {
    let mut config = crate::DaemonConfig::for_tests();
    config.event_registry_url = Some(server_url.clone());
    config.event_generator_management_targets.clear();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
        api_url: server_url,
        email: "external@example.test".to_string(),
        account_id: "account-external".to_string(),
        user_id: "user-external".to_string(),
        account_slug: "external".to_string(),
        realm_id: "realm-external".to_string(),
        relay_url: "wss://relay.example.test".to_string(),
        issuer_id: "issuer-external".to_string(),
        client_id: Some("client-external".to_string()),
        client_alias: Some("external-client".to_string()),
        machine_id: Some("machine-external".to_string()),
        machine_credential: None,
        cloud_session_token: Some("cloud-session-token".to_string()),
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: None,
    });
    config
}

fn serve_ready_connection(
    stream: &mut TcpStream,
    revoked: &AtomicBool,
    unavailable: &AtomicBool,
    scopes_reduced: &AtomicBool,
    management_url_discovered: &AtomicBool,
    management_url: &str,
    requests: &Mutex<Vec<String>>,
) {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => panic!("event connection test request read failed: {error}"),
        };
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if http_request_complete(&request) {
            break;
        }
    }
    if request.is_empty() {
        return;
    }
    let request = String::from_utf8_lossy(&request);
    requests.lock().unwrap().push(request.to_string());
    let catalog_detail_request = request
        .starts_with("GET /v1/event-generators/dev.chariox.dummy HTTP/1.1")
        || request.starts_with("GET /v1/event-generators/dev.chariox.dummy?");
    if catalog_detail_request {
        management_url_discovered.store(true, Ordering::Relaxed);
    } else {
        assert_signed_by_the_kernel(&request);
    }
    if unavailable.load(Ordering::Relaxed) {
        let body = serde_json::json!({
            "error": {"code": "temporarily_unavailable", "message": "test outage"}
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        return;
    }
    let body = if catalog_detail_request {
        serde_json::json!({
            "schema_version": 1,
            "generator_id": "dev.chariox.dummy",
            "version": "1.0.0",
            "name": "Scoped test events",
            "summary": "Exercises binding scope revalidation.",
            "provider": "Chariox test harness",
            "publisher": {"id": "dev.chariox", "name": "Chariox"},
            "operator": {"id": "local", "name": "Local operator"},
            "verification": "chariox",
            "manifest_digest": crate::runtime::event_catalog_control::BUILTIN_DUMMY_MANIFEST_DIGEST,
            "protocol_version": chariox_event_protocol::AEGS_MANAGEMENT_PROTOCOL_VERSION,
            "categories": ["Testing"],
            "installed_count": 0,
            "recommended": false,
            "availability": "available",
            "management_url": management_url,
            "authorization": {"kind": "none"},
            "events": [{
                "event_type": "dummy.test",
                "version": 1,
                "name": "Test event",
                "description": "Requires a provider grant.",
                "filter_schema": {"type": "object"},
                "required_scopes": ["events:read"]
            }],
            "actions": [{
                "action_id": "dummy.ping",
                "name": "Ping",
                "description": "A fixture action.",
                "target": "connection",
                "mutation": false,
                "idempotent": true,
                "input_schema": {"type": "object"}
            }],
            "signature": {"key_id": "test", "algorithm": "ed25519", "value": "test"}
        })
        .to_string()
    } else if request.starts_with("POST /v1/authorizations HTTP/1.1") {
        serde_json::json!({
            "generator_id": "dev.chariox.dummy",
            "status": "user_action_required",
            "connection_id": "connection-dynamic",
            "authorization_url": "https://example.test/authorize",
            "expires_at_ms": 4_000_000_000_000_u64
        })
        .to_string()
    } else if request.starts_with("POST /v1/connections/query HTTP/1.1") {
        serde_json::json!({
            "connections": [{
                "generator_id": "dev.chariox.dummy",
                "connection_id": "connection-local",
                "status": if revoked.load(Ordering::Relaxed) { "revoked" } else { "ready" },
                "metadata": {"account": "local"},
                "updated_at_ms": 1
            }]
        })
        .to_string()
    } else if request.starts_with("POST /v1/connections/inspect HTTP/1.1")
        || request.starts_with("POST /v1/connections/refresh HTTP/1.1")
    {
        serde_json::json!({
            "generator_id": "dev.chariox.dummy",
            "connection_id": "connection-local",
            "lifecycle_state": if revoked.load(Ordering::Relaxed) { "disconnected" } else { "connected" },
            "scopes": if scopes_reduced.load(Ordering::Relaxed) {
                serde_json::json!([])
            } else {
                serde_json::json!([{
                    "id": "events:read",
                    "label": "Read events",
                    "granted": true,
                    "required": true
                }])
            },
            "resources": [],
            "last_successful_health_check_at_ms": 1,
            "test_event_supported": false
        })
        .to_string()
    } else if request.starts_with("POST /v1/events/query HTTP/1.1") {
        assert!(request.contains("\"owner_id\":"));
        serde_json::json!({"events": [received_event_fixture("pending")]}).to_string()
    } else if request.starts_with("POST /v1/events/replay HTTP/1.1") {
        assert!(request.contains("\"delivery_id\":\"delivery-1\""));
        serde_json::json!({"event": received_event_fixture("published")}).to_string()
    } else if request.starts_with("POST /v1/connections/revoke HTTP/1.1") {
        revoked.store(true, Ordering::Relaxed);
        serde_json::json!({"revoked": true}).to_string()
    } else if request.starts_with("POST /v1/connections/reconnect HTTP/1.1") {
        serde_json::json!({
            "generator_id": "dev.chariox.dummy",
            "status": "pending",
            "connection_id": "connection-local",
            "authorization_url": "https://example.test/reconnect",
            "expires_at_ms": 4_000_000_000_000_u64
        })
        .to_string()
    } else {
        panic!("unexpected event connection request: {request}");
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
}

fn received_event_fixture(status: &str) -> serde_json::Value {
    serde_json::json!({
        "delivery_id": "delivery-1",
        "event_type": "pull_request.synchronize",
        "occurrence_id": "occurrence-1",
        "connection_scope": "charioxai/chariox",
        "subject": "#1026",
        "received_at_ms": 10,
        "status": status,
        "attempts": 1,
        "publication_count": 0
    })
}

#[test]
fn kernel_lists_and_replays_the_webhooks_an_integration_received() {
    let server = ReadyConnectionServer::start();
    let mut config = crate::DaemonConfig::for_tests();
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), server.target());
    let harness = LocalRouterTestHarness::with_config(config);
    register_ready_connection(&harness);

    let page = match harness
        .dispatch(LocalDaemonRequest::ListEventConnectionEvents(
            ListEventConnectionEventsRequest {
                connection_id: "connection-local".to_string(),
                limit: 20,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::EventConnectionEvents { page } => page,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].delivery_id, "delivery-1");
    assert_eq!(
        page.events[0].status,
        chariox_event_protocol::AegsReceivedEventStatus::Pending
    );

    let replayed = match harness
        .dispatch(LocalDaemonRequest::ReplayEventConnectionEvent(
            ReplayEventConnectionEventRequest {
                connection_id: "connection-local".to_string(),
                delivery_id: "delivery-1".to_string(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::EventConnectionEventReplayed { result } => result,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        replayed.event.status,
        chariox_event_protocol::AegsReceivedEventStatus::Published
    );
}

#[test]
fn kernel_rejects_insecure_registry_issued_management_target_after_capability_issue() {
    let server = ReadyConnectionServer::start();
    let server_url = server.target().url;
    let harness = LocalRouterTestHarness::with_config(dynamic_registry_config(server_url));

    let error = harness
        .dispatch(LocalDaemonRequest::InstallEventConnection(
            crate::local::InstallEventConnectionRequest {
                generator_id: "dev.chariox.dummy".to_string(),
                return_url: Some("https://terminal.chariox.com/notifications".to_string()),
            },
        ))
        .expect_err("registry-issued plaintext management targets must be rejected");

    assert!(
        error
            .to_string()
            .contains("Insecure request attempted with https_only set"),
        "unexpected error: {error}"
    );
    assert!(server.management_url_discovered.load(Ordering::Relaxed));
}

#[test]
fn kernel_bootstraps_registry_issued_https_management_target_end_to_end() {
    let server = ReadyConnectionServer::start_with_management_url(|address| {
        format!("https://aegs-public.test:{}", address.port())
    });
    let server_address = server.address;
    let client =
        crate::runtime::event_catalog_control::AegsManagementHttpClient::with_agent_builder(
            move |target| {
                crate::runtime::event_catalog_control::aegs_management_agent_builder_with_resolver(
                    target,
                    move |netloc: &str| {
                        assert_eq!(
                            netloc,
                            format!("aegs-public.test:{}", server_address.port())
                        );
                        Ok(vec![server_address])
                    },
                )
                .tls_connector(Arc::new(PassthroughTlsConnector))
            },
        );
    let harness = LocalRouterTestHarness::with_config_and_aegs_management_http_client(
        dynamic_registry_config(server.target().url),
        client,
    );
    let caller_user_id = "external-user";

    let response = harness
        .dispatch_as_user(
            caller_user_id,
            LocalDaemonRequest::InstallEventConnection(
                crate::local::InstallEventConnectionRequest {
                    generator_id: "dev.chariox.dummy".to_string(),
                    return_url: Some("https://terminal.chariox.com/notifications".to_string()),
                },
            ),
        )
        .expect("trusted HTTPS Store target should start authorization");

    let LocalDaemonResponse::EventConnectionAuthorizationStarted { authorization } = response
    else {
        panic!("unexpected response: {response:?}");
    };
    assert_eq!(authorization.generator_id, "dev.chariox.dummy");
    assert_eq!(
        authorization.connection_id.as_deref(),
        Some("connection-dynamic")
    );
    // The owner the AEGS was asked under is the one the authorization (and so
    // the connection) keeps: it is resolved once per request.
    let expected_owner_id = harness
        .runtime_state()
        .event_connection_registry()
        .authorization_event_owner_id(caller_user_id, &authorization.authorization_id)
        .unwrap()
        .expect("the authorization records its owner");
    assert_eq!(expected_owner_id.len(), 32);
    assert!(server.management_url_discovered.load(Ordering::Relaxed));
    assert_eq!(
        server.management_url,
        format!("https://aegs-public.test:{}", server.address.port())
    );

    let requests = server.requests.lock().unwrap();
    let authorization_request = requests
        .iter()
        .find(|request| request.starts_with("POST /v1/authorizations HTTP/1.1"))
        .expect("authorization request should reach the AEGS");
    let normalized = authorization_request.to_ascii_lowercase();
    assert!(
        !normalized.contains("authorization:"),
        "no bearer token is sent"
    );
    assert!(normalized.contains(&format!("x-chariox-owner-id: {expected_owner_id}")));
    assert!(authorization_request.contains(&format!("\"owner_id\":\"{expected_owner_id}\"")));
}

/// The request carries the kernel's identity-key signature and it verifies over the request's
/// method, path, body and owner.
fn assert_signed_by_the_kernel(request: &str) {
    use chariox_event_protocol::kernel_identity::{
        kernel_id_for_public_key, verify, ManagementRequest, KERNEL_KEY_HEADER,
        KERNEL_NONCE_HEADER, KERNEL_SIGNATURE_HEADER, KERNEL_TIMESTAMP_HEADER, OWNER_ID_HEADER,
    };
    let (head, body) = request.split_once("\r\n\r\n").expect("a complete request");
    let mut lines = head.lines();
    let mut first = lines.next().expect("a request line").split_whitespace();
    let (method, path) = (first.next().unwrap(), first.next().unwrap());
    let header = |name: &str| {
        lines
            .clone()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.trim()
                    .eq_ignore_ascii_case(name)
                    .then(|| value.trim().to_string())
            })
            .unwrap_or_else(|| panic!("the request has no {name} header"))
    };
    assert!(
        !head.to_ascii_lowercase().contains("authorization:"),
        "no bearer token is sent"
    );
    let public_key = header(KERNEL_KEY_HEADER);
    assert_eq!(kernel_id_for_public_key(&public_key).len(), 64);
    let nonce = header(KERNEL_NONCE_HEADER);
    let message = ManagementRequest {
        method,
        path,
        generator_id: "dev.chariox.dummy",
        owner_id: &header(OWNER_ID_HEADER),
        timestamp_ms: header(KERNEL_TIMESTAMP_HEADER).parse().unwrap(),
        nonce: &nonce,
        body: body.as_bytes(),
    }
    .message();
    verify(&public_key, &message, &header(KERNEL_SIGNATURE_HEADER))
        .expect("the request is signed by the key it presents");
}

fn http_request_complete(request: &[u8]) -> bool {
    let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let body_start = header_end + 4;
    let headers = String::from_utf8_lossy(&request[..body_start]).to_ascii_lowercase();
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    request.len() >= body_start + content_length
}

#[test]
fn kernel_reconciles_completed_event_authorization_without_a_client_observer() {
    let server = ReadyConnectionServer::start();
    let mut config = crate::DaemonConfig::for_tests();
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), server.target());
    let harness = LocalRouterTestHarness::with_config(config);
    let authorization = harness
        .runtime_state()
        .event_connection_registry()
        .start_authorization(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "event-owner-test",
            chariox_event_protocol::AegsAuthorizationFlow {
                generator_id: "dev.chariox.dummy".to_string(),
                status: "user_action_required".to_string(),
                connection_id: Some("connection-local".to_string()),
                authorization_url: Some("https://example.test/authorize".to_string()),
                user_code: None,
                expires_at_ms: Some(4_000_000_000_000_u64),
            },
        )
        .expect("pending authorization should be durable");

    let reconciliation = harness.reconcile_pending_event_connections();
    assert_eq!(reconciliation.attempted, 1);
    assert_eq!(reconciliation.observed, 1);
    assert_eq!(reconciliation.completed, 1);
    assert_eq!(reconciliation.failed, 0);

    let connection = match harness
        .dispatch(LocalDaemonRequest::GetEventConnection(
            GetEventConnectionRequest {
                connection_id: "connection-local".to_string(),
            },
        ))
        .expect("background reconciliation must durably register the connection")
    {
        LocalDaemonResponse::EventConnection { connection } => connection,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        connection.status,
        crate::local::EventConnectionStatus::Ready
    );
    assert!(harness
        .runtime_state()
        .event_connection_registry()
        .reconcilable_authorizations()
        .unwrap()
        .is_empty());
    assert_eq!(
        harness
            .runtime_state()
            .event_connection_registry()
            .authorization(
                crate::session::DEFAULT_LOCAL_USER_ID,
                &authorization.authorization_id,
            )
            .unwrap()
            .expect("completed authorization remains observable")
            .status,
        "ready"
    );
}

fn harness_with_ready_connection(
    server: &ReadyConnectionServer,
) -> crate::local::test_support::LocalRouterTestHarness {
    let mut config = crate::DaemonConfig::for_tests();
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), server.target());
    let harness = LocalRouterTestHarness::with_config(config);
    harness
        .runtime_state()
        .event_connection_registry()
        .upsert(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "event-owner-test",
            chariox_event_protocol::AegsConnectionSummary {
                generator_id: "dev.chariox.dummy".to_string(),
                connection_id: "connection-local".to_string(),
                status: chariox_event_protocol::AegsConnectionStatus::Ready,
                metadata: serde_json::json!({"account": "local"}),
                expires_at_ms: None,
                updated_at_ms: 1,
            },
        )
        .unwrap();
    harness
}

fn connection_status(
    harness: &crate::local::test_support::LocalRouterTestHarness,
) -> crate::local::EventConnectionStatus {
    harness
        .runtime_state()
        .event_connection_registry()
        .get(crate::session::DEFAULT_LOCAL_USER_ID, "connection-local")
        .unwrap()
        .expect("the connection record stays")
        .status
}

fn list_connection_ids(
    harness: &crate::local::test_support::LocalRouterTestHarness,
    user_id: &str,
) -> Vec<String> {
    match harness
        .dispatch_as_user(
            user_id,
            LocalDaemonRequest::ListEventConnections(crate::local::ListEventConnectionsRequest {
                generator_id: None,
                cursor: None,
                limit: 50,
            }),
        )
        .unwrap()
    {
        LocalDaemonResponse::EventConnectionsPage { page } => page
            .connections
            .into_iter()
            .map(|connection| connection.connection_id)
            .collect(),
        response => panic!("unexpected response: {response:?}"),
    }
}

#[test]
fn the_local_cli_and_the_owners_cloud_clients_manage_the_same_event_connections() {
    let server = ReadyConnectionServer::start();
    let mut config = crate::DaemonConfig::for_tests();
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), server.target());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "cloud-owner".to_string(),
        ..Default::default()
    });
    config.event_registry_url = Some("https://registry.test".to_string());
    let harness = LocalRouterTestHarness::with_config(config);
    let registry = harness.runtime_state().event_connection_registry().clone();
    let local = crate::session::DEFAULT_LOCAL_USER_ID;
    // Made through the local CLI, listed by the owner's Cloud client and back.
    registry
        .upsert(
            local,
            "event-owner-test",
            chariox_event_protocol::AegsConnectionSummary {
                generator_id: "dev.chariox.dummy".to_string(),
                connection_id: "connection-local".to_string(),
                status: chariox_event_protocol::AegsConnectionStatus::Ready,
                metadata: serde_json::json!({}),
                expires_at_ms: None,
                updated_at_ms: 1,
            },
        )
        .unwrap();
    assert_eq!(list_connection_ids(&harness, local), ["connection-local"]);
    assert_eq!(
        list_connection_ids(&harness, "cloud-owner"),
        ["connection-local"]
    );
    // Another user of the kernel has their own space.
    assert!(list_connection_ids(&harness, "collaborator").is_empty());
    // The owner's Cloud client manages the record the local CLI made.
    let refreshed = harness
        .dispatch_as_user(
            "cloud-owner",
            LocalDaemonRequest::RefreshEventConnection(
                crate::local::RefreshEventConnectionRequest {
                    connection_id: "connection-local".to_string(),
                },
            ),
        )
        .expect("the Cloud client refreshes the local CLI's connection");
    assert!(matches!(
        refreshed,
        LocalDaemonResponse::EventConnection { .. }
    ));
    // A connection the Cloud client starts is the local CLI's too.
    let LocalDaemonResponse::EventConnectionAuthorizationStarted { authorization } = harness
        .dispatch_as_user(
            "cloud-owner",
            LocalDaemonRequest::InstallEventConnection(
                crate::local::InstallEventConnectionRequest {
                    generator_id: "dev.chariox.dummy".to_string(),
                    return_url: None,
                },
            ),
        )
        .unwrap()
    else {
        panic!("expected a started authorization")
    };
    assert!(registry
        .authorization(local, &authorization.authorization_id)
        .unwrap()
        .is_some());
    assert!(registry
        .authorization("cloud-owner", &authorization.authorization_id)
        .unwrap()
        .is_none());
    let LocalDaemonResponse::EventConnectionRemoved { .. } = harness
        .dispatch_as_user(
            "cloud-owner",
            LocalDaemonRequest::RemoveEventConnection(crate::local::RemoveEventConnectionRequest {
                connection_id: "connection-local".to_string(),
                confirm: true,
            }),
        )
        .unwrap()
    else {
        panic!("expected the connection to be removed")
    };
    assert!(list_connection_ids(&harness, local).is_empty());
}

fn connection_of(
    harness: &crate::local::test_support::LocalRouterTestHarness,
) -> crate::local::EventConnection {
    match harness
        .dispatch(LocalDaemonRequest::GetEventConnection(
            GetEventConnectionRequest {
                connection_id: "connection-local".to_string(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::EventConnection { connection } => connection,
        response => panic!("unexpected response: {response:?}"),
    }
}

#[test]
fn an_imported_connection_shows_the_sources_state_right_after_the_import() {
    let server = ReadyConnectionServer::start();
    let harness_with_target = || {
        let mut config = crate::DaemonConfig::for_tests();
        config
            .event_generator_management_targets
            .insert("dev.chariox.dummy".to_string(), server.target());
        LocalRouterTestHarness::with_config(config)
    };
    let source = harness_with_target();
    register_ready_connection(&source);
    let target = harness_with_target();
    // The transfer's own export and import code moves the record.
    let selection =
        crate::managed_context::package::ManagedContextEventConnectionSelection::Selected {
            connection_ids: vec!["connection-local".to_string()],
        };
    let local = crate::session::DEFAULT_LOCAL_USER_ID;
    let records = crate::managed_context::event_connections::export_selected(
        &selection,
        source.runtime_state().event_connection_registry(),
        local,
    )
    .unwrap();
    crate::managed_context::event_connections::import(
        &crate::managed_context::event_connections::ManagedContextEventConnectionImportTarget {
            registry: target.runtime_state().event_connection_registry().clone(),
            owner_user_id: local.to_string(),
        },
        "ctx_event_publication",
        &records,
    )
    .unwrap();
    let before = connection_of(&target);
    assert!(before.scopes.is_empty() && before.metadata.is_null());

    target.load_imported_event_connections(local, &["connection-local".to_string()]);

    let (copy, original) = (connection_of(&target), connection_of(&source));
    assert_eq!(copy.status, original.status);
    assert_eq!(copy.lifecycle_state, original.lifecycle_state);
    assert_eq!(copy.metadata, original.metadata);
    assert_eq!(copy.scopes, original.scopes);
    assert_eq!(copy.resources, original.resources);
    assert!(!copy.scopes.is_empty());
}

#[test]
fn an_unreachable_event_service_leaves_an_imported_connection_as_imported() {
    let server = ReadyConnectionServer::start();
    let harness_with_target = || {
        let mut config = crate::DaemonConfig::for_tests();
        config
            .event_generator_management_targets
            .insert("dev.chariox.dummy".to_string(), server.target());
        LocalRouterTestHarness::with_config(config)
    };
    let (source, target) = (harness_with_target(), harness_with_target());
    register_ready_connection(&source);
    let local = crate::session::DEFAULT_LOCAL_USER_ID;
    let records = crate::managed_context::event_connections::export_selected(
        &crate::managed_context::package::ManagedContextEventConnectionSelection::Selected {
            connection_ids: vec!["connection-local".to_string()],
        },
        source.runtime_state().event_connection_registry(),
        local,
    )
    .unwrap();
    crate::managed_context::event_connections::import(
        &crate::managed_context::event_connections::ManagedContextEventConnectionImportTarget {
            registry: target.runtime_state().event_connection_registry().clone(),
            owner_user_id: local.to_string(),
        },
        "ctx_event_publication",
        &records,
    )
    .unwrap();
    server.unavailable.store(true, Ordering::Relaxed);
    target.load_imported_event_connections(local, &["connection-local".to_string()]);
    let kept = connection_of(&target);
    assert_eq!(kept.status, crate::local::EventConnectionStatus::Ready);
    assert!(kept.scopes.is_empty());
}

#[test]
fn a_copy_notices_a_revoked_connection_without_being_queried() {
    let server = ReadyConnectionServer::start();
    let harness = harness_with_ready_connection(&server);
    // Nothing changed at the event service: the copy stays ready.
    let quiet = harness.refresh_ready_event_connections();
    assert_eq!((quiet.attempted, quiet.failed), (1, 0));
    assert_eq!(
        connection_status(&harness),
        crate::local::EventConnectionStatus::Ready
    );
    // Another kernel (or the user) revokes the connection.
    server.revoked.store(true, Ordering::Relaxed);
    let summary = harness.refresh_ready_event_connections();
    assert_eq!((summary.attempted, summary.completed), (1, 1));
    assert_eq!(
        connection_status(&harness),
        crate::local::EventConnectionStatus::Revoked
    );
    // A revoked connection is not asked about again.
    assert_eq!(harness.refresh_ready_event_connections().attempted, 0);
}

#[test]
fn ready_refreshes_ask_about_every_connection_beyond_one_batch_even_when_questions_fail() {
    let server = ReadyConnectionServer::start();
    let harness = harness_with_ready_connection(&server);
    let mut ids = (0..129)
        .map(|n| format!("connection-{n:03}"))
        .collect::<Vec<_>>();
    for id in &ids {
        harness
            .runtime_state()
            .event_connection_registry()
            .upsert(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "event-owner-test",
                chariox_event_protocol::AegsConnectionSummary {
                    generator_id: "dev.chariox.dummy".to_string(),
                    connection_id: id.clone(),
                    status: chariox_event_protocol::AegsConnectionStatus::Ready,
                    metadata: serde_json::json!({"account": "local"}),
                    expires_at_ms: None,
                    updated_at_ms: 1,
                },
            )
            .unwrap();
    }
    ids.push("connection-local".to_string());
    // Every question fails, so every connection stays ready: the batches must move on anyway.
    server.unavailable.store(true, Ordering::Relaxed);
    for _ in 0..ids.len().div_ceil(64) {
        let summary = harness.refresh_ready_event_connections();
        assert_eq!((summary.attempted, summary.failed), (64, 64));
    }
    let requests = server.requests.lock().unwrap().join("\n");
    let never_asked = ids
        .iter()
        .filter(|id| !requests.contains(id.as_str()))
        .collect::<Vec<_>>();
    assert!(never_asked.is_empty(), "never asked about: {never_asked:?}");
}

#[test]
fn an_unreachable_event_service_leaves_a_ready_connection_ready() {
    let server = ReadyConnectionServer::start();
    let harness = harness_with_ready_connection(&server);
    server.unavailable.store(true, Ordering::Relaxed);
    let summary = harness.refresh_ready_event_connections();
    assert_eq!((summary.attempted, summary.failed), (1, 1));
    assert_eq!(
        connection_status(&harness),
        crate::local::EventConnectionStatus::Ready,
        "an outage is not a revocation: keep the last known state"
    );
}

#[test]
fn failed_event_connection_validation_is_durable_and_recovers_in_place() {
    let server = ReadyConnectionServer::start();
    let mut config = crate::DaemonConfig::for_tests();
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), server.target());
    let harness = LocalRouterTestHarness::with_config(config);
    harness
        .runtime_state()
        .event_connection_registry()
        .upsert(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "event-owner-test",
            chariox_event_protocol::AegsConnectionSummary {
                generator_id: "dev.chariox.dummy".to_string(),
                connection_id: "connection-local".to_string(),
                status: chariox_event_protocol::AegsConnectionStatus::Ready,
                metadata: serde_json::json!({"account": "local"}),
                expires_at_ms: None,
                updated_at_ms: 1,
            },
        )
        .unwrap();

    server.unavailable.store(true, Ordering::Relaxed);
    let refresh_error = harness
        .dispatch(LocalDaemonRequest::RefreshEventConnection(
            RefreshEventConnectionRequest {
                connection_id: "connection-local".to_string(),
            },
        ))
        .expect_err("AEGS outage should fail validation");
    assert!(refresh_error.to_string().contains("503"));
    let unavailable = match harness
        .dispatch(LocalDaemonRequest::GetEventConnection(
            GetEventConnectionRequest {
                connection_id: "connection-local".to_string(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::EventConnection { connection } => connection,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        unavailable.status,
        crate::local::EventConnectionStatus::Unavailable
    );

    server.unavailable.store(false, Ordering::Relaxed);
    let recovered = match harness
        .dispatch(LocalDaemonRequest::RefreshEventConnection(
            RefreshEventConnectionRequest {
                connection_id: "connection-local".to_string(),
            },
        ))
        .expect("connection should recover without changing identity")
    {
        LocalDaemonResponse::EventConnection { connection } => connection,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(recovered.connection_id, unavailable.connection_id);
    assert_eq!(recovered.status, crate::local::EventConnectionStatus::Ready);
}

#[test]
fn event_connection_removal_waits_for_the_apps_that_use_it() {
    let server = ReadyConnectionServer::start();
    let mut config = crate::DaemonConfig::for_tests();
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), server.target());
    let harness = LocalRouterTestHarness::with_config(config);
    register_ready_connection(&harness);
    install_app_route(&harness, "removal");
    let store = harness.with_app(|app| app.durable_state_store());
    let database = rusqlite::Connection::open(store.path()).unwrap();
    database
        .execute_batch(
            "INSERT INTO app_connection_grants(owner_id,installation_id,connection_id,generator_id,event_owner_id,granted_at_ms)
             VALUES('local','app_route_first','connection-local','dev.chariox.dummy','event-owner-test',1)",
        )
        .unwrap();
    let connection_id = "connection-local".to_string();

    let dependencies = match harness
        .dispatch(LocalDaemonRequest::ListEventConnectionDependencies(
            ListEventConnectionDependenciesRequest {
                connection_id: connection_id.clone(),
            },
        ))
        .expect("connection dependencies should resolve")
    {
        LocalDaemonResponse::EventConnectionDependencies { dependencies, .. } => dependencies,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        dependencies,
        vec![
            crate::local::EventConnectionDependency {
                installation_id: "app_route_first".to_string(),
                route_id: Some("taken".to_string()),
                active: true,
            },
            crate::local::EventConnectionDependency {
                installation_id: "app_route_first".to_string(),
                route_id: None,
                active: true,
            },
        ]
    );
    match harness
        .dispatch(LocalDaemonRequest::GetEventConnection(
            GetEventConnectionRequest {
                connection_id: connection_id.clone(),
            },
        ))
        .expect("the connection should resolve")
    {
        LocalDaemonResponse::EventConnection { connection } => {
            assert_eq!(connection.attached_trigger_count, 2)
        }
        response => panic!("unexpected response: {response:?}"),
    }

    let pending_authorization = match harness
        .dispatch(LocalDaemonRequest::ReconnectEventConnection(
            ReconnectEventConnectionRequest {
                connection_id: connection_id.clone(),
                return_url: Some("http://127.0.0.1:4321/notifications".to_string()),
            },
        ))
        .expect("reconnect should create a connection-scoped authorization")
    {
        LocalDaemonResponse::EventConnectionAuthorizationStarted { authorization } => authorization,
        response => panic!("unexpected response: {response:?}"),
    };

    let remove = |confirm: bool| {
        harness.dispatch(LocalDaemonRequest::RemoveEventConnection(
            RemoveEventConnectionRequest {
                connection_id: connection_id.clone(),
                confirm,
            },
        ))
    };
    let unconfirmed = remove(false).expect_err("removal must require explicit confirmation");
    assert!(
        unconfirmed
            .to_string()
            .contains("2 App route(s) or grant(s) use it"),
        "{unconfirmed}"
    );
    let in_use = remove(true).expect_err("an App still uses the connection");
    assert!(
        in_use.to_string().contains("remove the App inbox routes"),
        "{in_use}"
    );
    assert!(!server.revoked.load(Ordering::Relaxed));

    // The App is uninstalled: its route stops counting, its grant is gone.
    database
        .execute_batch(
            "UPDATE app_installations SET active_json=NULL WHERE installation_id='app_route_first';
             DELETE FROM app_connection_grants;",
        )
        .unwrap();
    let removed = remove(true).expect("confirmed connection removal should succeed");
    let LocalDaemonResponse::EventConnectionRemoved { connection } = removed else {
        panic!("unexpected response: {removed:?}");
    };
    assert_eq!(connection.connection_id, connection_id);
    assert!(server.revoked.load(Ordering::Relaxed));

    let missing_connection = harness
        .dispatch(LocalDaemonRequest::GetEventConnection(
            GetEventConnectionRequest {
                connection_id: connection_id.clone(),
            },
        ))
        .expect_err("removed connection must leave the kernel registry");
    assert!(missing_connection.to_string().contains("was not found"));

    let missing_authorization = harness
        .dispatch(LocalDaemonRequest::ObserveEventConnectionAuthorization(
            ObserveEventConnectionAuthorizationRequest {
                authorization_id: pending_authorization.authorization_id,
            },
        ))
        .expect_err("removal must cancel reconnect attempts for the same connection");
    assert!(missing_authorization
        .to_string()
        .contains("authorization was not found"));
}

fn register_ready_connection(harness: &LocalRouterTestHarness) {
    let runtime_state = harness.runtime_state();
    let registry = runtime_state.event_connection_registry();
    registry
        .upsert(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "event-owner-test",
            chariox_event_protocol::AegsConnectionSummary {
                generator_id: "dev.chariox.dummy".to_string(),
                connection_id: "connection-local".to_string(),
                status: chariox_event_protocol::AegsConnectionStatus::Ready,
                metadata: serde_json::json!({"account": "local"}),
                expires_at_ms: None,
                updated_at_ms: 1,
            },
        )
        .unwrap();
    registry
        .apply_inspection(
            crate::session::DEFAULT_LOCAL_USER_ID,
            chariox_event_protocol::AegsConnectionInspection {
                generator_id: "dev.chariox.dummy".to_string(),
                connection_id: "connection-local".to_string(),
                lifecycle_state: chariox_event_protocol::AegsConnectionLifecycleState::Connected,
                scopes: vec![chariox_event_protocol::AegsConnectionScope {
                    id: "events:read".to_string(),
                    label: "Read events".to_string(),
                    granted: true,
                    required: true,
                }],
                resources: Vec::new(),
                last_successful_health_check_at_ms: Some(1),
                last_accepted_event_at_ms: None,
                problem_code: None,
                problem_message: None,
                recovery_action: None,
                test_event_supported: false,
            },
        )
        .unwrap();
}

/// An installed App with an inbox route on `connection-local` for the
/// `dummy.test` events of `channel`.
fn install_app_route(harness: &LocalRouterTestHarness, channel: &str) {
    let store = harness.with_app(|app| app.durable_state_store());
    rusqlite::Connection::open(store.path())
        .unwrap()
        .execute_batch(
            "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
             VALUES('app_route_first','com.example.first','local',1,1,'{}')",
        )
        .unwrap();
    store
        .app_inbox(
            crate::durable_state::app_inbox::AppInboxOperation::CreateRoute {
                route: chariox_app_runtime::app_inbox::InboxRoute {
                    route_id: "taken".into(),
                    owner_id: crate::session::DEFAULT_LOCAL_USER_ID.into(),
                    installation_id: "app_route_first".into(),
                    event_name: "received".into(),
                    source_event_type: "dummy.test".into(),
                    source_event_version: 1,
                    active: true,
                    source: Some(chariox_app_runtime::app_inbox::InboxSource {
                        generator_id: "dev.chariox.dummy".into(),
                        connection_id: "connection-local".into(),
                        connection_scope: "tenant:local".into(),
                        filter_json: serde_json::json!({"channel": channel}).to_string(),
                        payload_schema_json: None,
                        manifest_digest: None,
                    }),
                },
                now_ms: 1,
            },
        )
        .unwrap();
}

#[test]
fn app_inbox_routes_are_checked_with_the_generator_before_they_are_stored() {
    let server = ReadyConnectionServer::start();
    let target = server.target();
    let mut config = crate::DaemonConfig::for_tests();
    config.event_registry_url = Some(target.url.clone());
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), target);
    let harness = LocalRouterTestHarness::with_config(config);
    register_ready_connection(&harness);
    // An installed App's route takes the interest first.
    install_app_route(&harness, "taken");
    let route = |event_type: &str, channel: &str| {
        harness.dispatch(LocalDaemonRequest::CreateAppInboxRoute(
            crate::local::CreateAppInboxRouteRequest {
                installation_id: "app_missing".to_string(),
                route_id: "route".to_string(),
                event_name: "received".to_string(),
                source_event_type: event_type.to_string(),
                source_event_version: 1,
                connection: Some(crate::local::AppInboxConnection {
                    generator_id: "dev.chariox.dummy".to_string(),
                    connection_id: "connection-local".to_string(),
                    connection_scope: "tenant:local".to_string(),
                    filter: serde_json::json!({"channel": channel}),
                }),
            },
        ))
    };
    let message = |result: Result<LocalDaemonResponse, DaemonError>| match result {
        Err(error) => error.to_string(),
        Ok(response) => format!("{response:?}"),
    };
    let undeclared = message(route("dummy.unknown", "free"));
    assert!(undeclared.contains("is not declared"), "{undeclared}");
    // App interests fan out: another App's matching route is not exclusive.
    // Both filters pass generator admission and reach the missing installation.
    let taken = message(route("dummy.test", "taken"));
    assert!(taken.contains("AppRequestFailed"), "{taken}");
    // Past every generator check, the App lane answers: nothing is installed.
    let free = message(route("dummy.test", "free"));
    assert!(free.contains("AppRequestFailed"), "{free}");
    // Protocol 359: a grant is checked with the generator the same way.
    let grant = |connection: &str| {
        message(harness.dispatch(LocalDaemonRequest::GrantAppConnection(
            crate::local::GrantAppConnectionRequest {
                installation_id: "app_missing".to_string(),
                generator_id: "dev.chariox.dummy".to_string(),
                connection_id: connection.to_string(),
            },
        )))
    };
    let unknown = grant("connection-unknown");
    assert!(!unknown.contains("AppRequestFailed"), "{unknown}");
    let known = grant("connection-local");
    assert!(known.contains("AppRequestFailed"), "{known}");
    // The connection lost the event's required scope.
    server.scopes_reduced.store(true, Ordering::Relaxed);
    harness
        .dispatch(LocalDaemonRequest::RefreshEventConnection(
            RefreshEventConnectionRequest {
                connection_id: "connection-local".to_string(),
            },
        ))
        .ok();
    let unscoped = message(route("dummy.test", "free"));
    assert!(unscoped.contains("missing required scopes"), "{unscoped}");
}
