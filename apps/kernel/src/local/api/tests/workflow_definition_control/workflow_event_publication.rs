use super::*;
use crate::local::{
    DisableWorkflowPublicationRequest, GetEventConnectionRequest, GetEventDeliveryStatusRequest,
    ListEventConnectionDependenciesRequest, ListWorkflowEventBindingsRequest,
    ObserveEventConnectionAuthorizationRequest, ReconnectEventConnectionRequest,
    RefreshEventConnectionRequest, RemoveEventConnectionRequest,
    SetWorkflowEventBindingStatusRequest, TransferWorkflowEventBindingRequest,
};
use crate::session::WorkflowEventBindingStatus;
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
    capability_issued: Arc<AtomicBool>,
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
        let capability_issued = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_revoked = Arc::clone(&revoked);
        let thread_unavailable = Arc::clone(&unavailable);
        let thread_scopes_reduced = Arc::clone(&scopes_reduced);
        let thread_capability_issued = Arc::clone(&capability_issued);
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
                            &thread_capability_issued,
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
            capability_issued,
            management_url,
            requests,
            thread: Some(thread),
        }
    }

    fn target(&self) -> crate::config::EventGeneratorManagementTarget {
        crate::config::EventGeneratorManagementTarget {
            url: format!("http://{}", self.address),
            token: "test-management-token".to_string(),
            expires_at_ms: None,
            owner_ids: None,
            owner_scoped: None,
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
        machine_alias: Some("external-machine".to_string()),
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
    capability_issued: &AtomicBool,
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
    let capability_request = request
        .starts_with("POST /v1/event-generators/dev.chariox.dummy/management-capability HTTP/1.1");
    if !catalog_detail_request && !capability_request {
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer test-management-token")
                || request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer registry-issued-token")
        );
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
            "actions": [],
            "signature": {"key_id": "test", "algorithm": "ed25519", "value": "test"}
        })
        .to_string()
    } else if capability_request {
        assert!(request.contains("\"sessionToken\":\"cloud-session-token\""));
        assert!(
            !request.contains("\"generatorId\":"),
            "the path-bound generator ID must not be duplicated into the strict Cloud request body"
        );
        assert!(request.contains("\"version\":\"1.0.0\""));
        assert!(request.contains(&format!(
            "\"manifestDigest\":\"{}\"",
            crate::runtime::event_catalog_control::BUILTIN_DUMMY_MANIFEST_DIGEST
        )));
        assert!(request.contains(&format!("\"managementUrl\":\"{management_url}\"")));
        capability_issued.store(true, Ordering::Relaxed);
        serde_json::json!({
            "token": "registry-issued-token",
            "expiresAt": "2100-01-01T00:00:00Z"
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
    assert!(server.capability_issued.load(Ordering::Relaxed));
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
    let expected_owner_id = crate::runtime::event_catalog_control::event_connection_owner_id(
        "daemon-test",
        caller_user_id,
    );

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
    assert!(server.capability_issued.load(Ordering::Relaxed));
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
    assert!(normalized.contains("authorization: bearer registry-issued-token"));
    assert!(normalized.contains(&format!("x-chariox-owner-id: {expected_owner_id}")));
    assert!(authorization_request.contains(&format!("\"owner_id\":\"{expected_owner_id}\"")));
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
fn event_publication_binding_supports_fanout_and_uses_workflow_queue() {
    let server = ReadyConnectionServer::start();
    let mut config = crate::DaemonConfig::for_tests();
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), server.target());
    let harness = LocalRouterTestHarness::with_config(config);
    let graph = create_publication_test_graph(&harness, "event-publication");
    let create_publication = |alias: &str| match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                expected_workflow_revision: None,
                operation_key: Some(format!("publish-{alias}")),
                queue_ref: Some("default".to_string()),
                alias: Some(alias.to_string()),
                kind: Some("event_based".to_string()),
                route: None,
                methods: Vec::new(),
                transport: None,
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: None,
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("event publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        response => panic!("unexpected response: {response:?}"),
    };
    let first = create_publication("event-first");
    let second = create_publication("event-second");
    let binding_request = |publication_ref: &str| CreateWorkflowEventBindingRequest {
        session_id: graph.session_id.clone(),
        publication_ref: publication_ref.to_string(),
        generator_id: "dev.chariox.dummy".to_string(),
        generator_version: "1.0.0".to_string(),
        manifest_digest: crate::runtime::event_catalog_control::BUILTIN_DUMMY_MANIFEST_DIGEST
            .to_string(),
        connection_id: "connection-local".to_string(),
        connection_scope: "tenant:local".to_string(),
        event_type: "dummy.test".to_string(),
        event_type_version: 1,
        filter: serde_json::json!({"channel": "default"}),
        environment_id: Some("environment-local".to_string()),
        queue_ref: Some("default".to_string()),
        reply_mode: None,
        action_ids: Vec::new(),
    };
    let binding = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEventBinding(
            binding_request(first.id()),
        ))
        .expect("first event binding should be created")
    {
        LocalDaemonResponse::WorkflowEventBindingCreated {
            binding, session, ..
        } => {
            assert_eq!(
                session.workflow_event_bindings(),
                std::slice::from_ref(&binding)
            );
            binding
        }
        response => panic!("unexpected response: {response:?}"),
    };
    let idempotent = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEventBinding(
            binding_request(first.id()),
        ))
        .expect("repeating the same binding should be idempotent")
    {
        LocalDaemonResponse::WorkflowEventBindingCreated { binding, .. } => binding,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(idempotent.id, binding.id);
    let fan_out = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEventBinding(
            binding_request(second.id()),
        ))
        .expect("same event interest should fan out to a second workflow");
    let fan_out_binding = match fan_out {
        LocalDaemonResponse::WorkflowEventBindingCreated { binding, .. } => binding,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_ne!(fan_out_binding.id, binding.id);
    assert_eq!(
        fan_out_binding.event_interest_key,
        binding.event_interest_key
    );

    let persisted = harness.with_app(|app| {
        app.durable_state_store()
            .load_workflow_hot_states("daemon-test")
            .expect("durable workflow state should load")
            .into_iter()
            .find(|(session_id, _)| session_id == &graph.session_id)
            .map(|(_, state)| state)
            .expect("event publication session should be persisted")
    });
    assert_eq!(persisted.workflow_publications.len(), 2);
    assert_eq!(persisted.workflow_publication_snapshots.len(), 2);
    assert_eq!(persisted.workflow_event_bindings.len(), 2);

    let package_files = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: first.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("event publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_version,
            package_files,
            ..
        } => {
            assert_eq!(package_version, 4);
            package_files
        }
        response => panic!("unexpected response: {response:?}"),
    };
    let binding_template = package_files
        .iter()
        .find(|file| file.path == "event-bindings.example.json")
        .expect("event binding activation template should travel with the publication");
    let binding_template = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(&binding_template.content_base64)
            .unwrap(),
    )
    .unwrap();
    assert!(binding_template.contains("\"requested_scope\": \"tenant:local\""));
    assert!(binding_template.contains("\"connection_id\": null"));
    assert!(binding_template.contains("\"reply_mode\": \"disabled\""));
    assert!(binding_template.contains("\"action_ids\": []"));
    assert!(!binding_template.contains("connection-local"));
    let publication_json = package_json_file(&package_files, "publication.json");
    assert_eq!(
        publication_json["event_bindings_path"],
        serde_json::json!("event-bindings.local.json")
    );
    assert_eq!(
        publication_json["hooks"][0]["transport"],
        serde_json::json!("event_based")
    );
    assert!(publication_json["hooks"][0].get("route").is_none());
    assert!(publication_json["hooks"][0].get("methods").is_none());
    assert!(publication_json["hooks"][0].get("parser").is_none());
    assert!(publication_json["hooks"][0].get("mode").is_none());
    assert!(!package_files
        .iter()
        .any(|file| file.path.starts_with("public/")));
    let deployment_contract = package_json_file(&package_files, "deployment-contract.json");
    assert_eq!(
        deployment_contract["routes"][0]["transport"],
        serde_json::json!("event_based")
    );
    assert_eq!(
        deployment_contract["routes"][0]["methods"],
        serde_json::json!([])
    );
    assert!(deployment_contract["routes"][0].get("path").is_none());

    let tested = match harness
        .dispatch(LocalDaemonRequest::TestWorkflowEventBinding(
            TestWorkflowEventBindingRequest {
                session_id: graph.session_id.clone(),
                binding_id: binding.id.clone(),
                prompt: Some("Process the deterministic event.".to_string()),
            },
        ))
        .expect("test delivery should enter the workflow queue")
    {
        LocalDaemonResponse::WorkflowEventBindingTested {
            queued_prompt_id,
            duplicate,
            session,
            ..
        } => {
            assert!(!duplicate);
            assert!(session
                .workflow_event_delivery_receipts()
                .values()
                .any(|receipt| receipt.queued_prompt_id == queued_prompt_id));
            assert!(
                session.workflow_runs().iter().any(|run| {
                    run.publication_invocation().is_some_and(|invocation| {
                        invocation.transport == "event"
                            && invocation.hook_id.as_deref() == Some(binding.id.as_str())
                    })
                }) || session.workflow_queued_prompts().iter().any(|prompt| {
                    prompt.publication_invocation().is_some_and(|invocation| {
                        invocation.transport == "event"
                            && invocation.hook_id.as_deref() == Some(binding.id.as_str())
                    })
                })
            );
            session
        }
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        tested.workflow_event_bindings().len(),
        2,
        "a second publication may intentionally fan out the same event interest"
    );
    let accepted_receipt = tested
        .workflow_event_delivery_receipts()
        .values()
        .next()
        .expect("test delivery receipt should remain available");
    let duplicate = harness
        .runtime_state()
        .accept_workflow_event_delivery(chariox_event_protocol::EventDeliveryEnvelope {
            delivery_id: accepted_receipt.delivery_id.clone(),
            binding_id: binding.id.clone(),
            event_type: binding.event_type.clone(),
            event_type_version: binding.event_type_version,
            occurrence_id: accepted_receipt.occurrence_id.clone(),
            occurred_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            prompt: "Replay the already accepted test event.".to_string(),
            artifacts: Vec::new(),
            metadata: serde_json::json!({"test": true, "duplicate": true}),
            reply_context: None,
            expires_at_ms: u64::MAX,
        })
        .expect("a duplicate delivery should return its durable receipt without blocking");
    assert!(duplicate.duplicate);
    assert_eq!(
        duplicate.queued_prompt_id,
        accepted_receipt.queued_prompt_id
    );

    let active_route_count = || match harness
        .dispatch(LocalDaemonRequest::GetEventDeliveryStatus(
            GetEventDeliveryStatusRequest,
        ))
        .expect("event delivery status should resolve")
    {
        LocalDaemonResponse::EventDeliveryStatus { status } => status.active_route_count,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        active_route_count(),
        2,
        "both active publications advertise the intentional fan-out"
    );
    harness.runtime_state().apply_event_route_conflicts(&[
        chariox_event_protocol::EventRouteConflict {
            environment_id: binding.environment_id.clone(),
            event_interest_key: binding.event_interest_key.clone(),
            requested_binding_id: binding.id.clone(),
            existing_binding_id: "binding-on-new-kernel".to_string(),
            existing_publication_id: second.id().to_string(),
        },
    ]);
    assert_eq!(
        active_route_count(),
        1,
        "an AEDS ownership conflict must fence only the old kernel route"
    );
    harness
        .dispatch(LocalDaemonRequest::SetWorkflowEventBindingStatus(
            SetWorkflowEventBindingStatusRequest {
                session_id: graph.session_id.clone(),
                binding_id: binding.id.clone(),
                status: WorkflowEventBindingStatus::Active,
            },
        ))
        .expect("a locally resolved conflict should be resumable");
    assert_eq!(active_route_count(), 2);

    let disabled = match harness
        .dispatch(LocalDaemonRequest::DisableWorkflowPublication(
            DisableWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                publication_ref: first.id().to_string(),
            },
        ))
        .expect("event publication should disable")
    {
        LocalDaemonResponse::WorkflowPublicationDisabled { session, .. } => session,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        disabled.workflow_event_bindings()[0].status,
        WorkflowEventBindingStatus::Tombstoned,
        "disabling a publication must retain a durable tombstone for reconciliation"
    );
    assert_eq!(
        active_route_count(),
        1,
        "a disabled publication must not continue advertising its active AEDS route"
    );
    let subscribe_error = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEventBinding(
            binding_request(first.id()),
        ))
        .expect_err("a disabled publication must reject new event bindings");
    assert!(
        subscribe_error
            .to_string()
            .contains("workflow publication is disabled"),
        "unexpected subscribe error: {subscribe_error}"
    );
    let test_error = harness
        .dispatch(LocalDaemonRequest::TestWorkflowEventBinding(
            TestWorkflowEventBindingRequest {
                session_id: graph.session_id.clone(),
                binding_id: binding.id.clone(),
                prompt: None,
            },
        ))
        .expect_err("a tombstoned event binding must reject test delivery");
    assert!(
        test_error
            .to_string()
            .contains("workflow event binding is tombstoned"),
        "unexpected test error: {test_error}"
    );

    harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowDesignOp(
            ApplyWorkflowDesignOpRequest {
                session_id: graph.session_id.clone(),
                origin_client_id: "event-connection-lifecycle-test".to_string(),
                op_id: "remove-event-workflow".to_string(),
                op: WorkflowDesignOp::WorkflowRemove {
                    workflow_id: graph.workflow_id,
                },
            },
        ))
        .expect("workflow deletion should tombstone bindings without removing the connection");
    let preserved_connection = match harness
        .dispatch(LocalDaemonRequest::GetEventConnection(
            GetEventConnectionRequest {
                connection_id: binding.connection_id,
            },
        ))
        .expect("workflow deletion must preserve the installed event connection")
    {
        LocalDaemonResponse::EventConnection { connection } => connection,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        preserved_connection.status,
        crate::local::EventConnectionStatus::Ready
    );
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
fn confirmed_event_connection_removal_tombstones_dependent_bindings_before_revocation() {
    let server = ReadyConnectionServer::start();
    let mut config = crate::DaemonConfig::for_tests();
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), server.target());
    let harness = LocalRouterTestHarness::with_config(config);
    let graph = create_publication_test_graph(&harness, "event-connection-removal");
    let publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                expected_workflow_revision: None,
                operation_key: Some("publish-removal".to_string()),
                queue_ref: Some("default".to_string()),
                alias: Some("event-removal".to_string()),
                kind: Some("event_based".to_string()),
                route: None,
                methods: Vec::new(),
                transport: None,
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: None,
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("event publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        response => panic!("unexpected response: {response:?}"),
    };
    let binding =
        match harness
            .dispatch(LocalDaemonRequest::CreateWorkflowEventBinding(
                CreateWorkflowEventBindingRequest {
                    session_id: graph.session_id.clone(),
                    publication_ref: publication.id().to_string(),
                    generator_id: "dev.chariox.dummy".to_string(),
                    generator_version: "1.0.0".to_string(),
                    manifest_digest:
                        crate::runtime::event_catalog_control::BUILTIN_DUMMY_MANIFEST_DIGEST
                            .to_string(),
                    connection_id: "connection-local".to_string(),
                    connection_scope: "tenant:local".to_string(),
                    event_type: "dummy.test".to_string(),
                    event_type_version: 1,
                    filter: serde_json::json!({"channel": "removal"}),
                    environment_id: Some("environment-removal".to_string()),
                    queue_ref: Some("default".to_string()),
                    reply_mode: None,
                    action_ids: Vec::new(),
                },
            ))
            .expect("event binding should be created")
        {
            LocalDaemonResponse::WorkflowEventBindingCreated { binding, .. } => binding,
            response => panic!("unexpected response: {response:?}"),
        };

    let dependencies = match harness
        .dispatch(LocalDaemonRequest::ListEventConnectionDependencies(
            ListEventConnectionDependenciesRequest {
                connection_id: binding.connection_id.clone(),
            },
        ))
        .expect("connection dependencies should resolve")
    {
        LocalDaemonResponse::EventConnectionDependencies { dependencies, .. } => dependencies,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].binding_id, binding.id);
    assert_eq!(dependencies[0].status, WorkflowEventBindingStatus::Active);

    let pending_authorization = match harness
        .dispatch(LocalDaemonRequest::ReconnectEventConnection(
            ReconnectEventConnectionRequest {
                connection_id: binding.connection_id.clone(),
                return_url: Some("http://127.0.0.1:4321/notifications".to_string()),
            },
        ))
        .expect("reconnect should create a connection-scoped authorization")
    {
        LocalDaemonResponse::EventConnectionAuthorizationStarted { authorization } => authorization,
        response => panic!("unexpected response: {response:?}"),
    };

    let confirmation_error = harness
        .dispatch(LocalDaemonRequest::RemoveEventConnection(
            RemoveEventConnectionRequest {
                connection_id: binding.connection_id.clone(),
                confirm: false,
            },
        ))
        .expect_err("connection removal must require explicit confirmation");
    assert!(confirmation_error
        .to_string()
        .contains("will deactivate 1 workflow binding(s)"));
    assert!(!server.revoked.load(Ordering::Relaxed));

    let removed = harness
        .dispatch(LocalDaemonRequest::RemoveEventConnection(
            RemoveEventConnectionRequest {
                connection_id: binding.connection_id.clone(),
                confirm: true,
            },
        ))
        .expect("confirmed connection removal should succeed");
    let LocalDaemonResponse::EventConnectionRemoved {
        connection,
        deactivated_bindings,
    } = removed
    else {
        panic!("unexpected response: {removed:?}");
    };
    assert_eq!(connection.connection_id, binding.connection_id);
    assert_eq!(deactivated_bindings.len(), 1);
    assert_eq!(
        deactivated_bindings[0].status,
        WorkflowEventBindingStatus::Tombstoned
    );
    assert!(server.revoked.load(Ordering::Relaxed));

    let bindings = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowEventBindings(
            ListWorkflowEventBindingsRequest {
                session_id: graph.session_id.clone(),
                publication_ref: Some(publication.id().to_string()),
            },
        ))
        .expect("workflow bindings should remain as reconciliation tombstones")
    {
        LocalDaemonResponse::WorkflowEventBindingsListed { bindings } => bindings,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].status, WorkflowEventBindingStatus::Tombstoned);

    let missing_connection = harness
        .dispatch(LocalDaemonRequest::GetEventConnection(
            GetEventConnectionRequest {
                connection_id: binding.connection_id.clone(),
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

    let fresh_attach_error =
        harness
            .dispatch(LocalDaemonRequest::CreateWorkflowEventBinding(
                CreateWorkflowEventBindingRequest {
                    session_id: graph.session_id.clone(),
                    publication_ref: publication.id().to_string(),
                    generator_id: "dev.chariox.dummy".to_string(),
                    generator_version: "1.0.0".to_string(),
                    manifest_digest:
                        crate::runtime::event_catalog_control::BUILTIN_DUMMY_MANIFEST_DIGEST
                            .to_string(),
                    connection_id: binding.connection_id.clone(),
                    connection_scope: "tenant:local".to_string(),
                    event_type: "dummy.test".to_string(),
                    event_type_version: 1,
                    filter: serde_json::json!({"channel": "reattach"}),
                    environment_id: Some("environment-removal".to_string()),
                    queue_ref: Some("default".to_string()),
                    reply_mode: None,
                    action_ids: Vec::new(),
                },
            ))
            .expect_err("a revoked connection must reject a fresh attachment");
    assert!(fresh_attach_error.to_string().contains("Revoked"));
    harness
        .dispatch(LocalDaemonRequest::GetEventConnection(
            GetEventConnectionRequest {
                connection_id: binding.connection_id.clone(),
            },
        ))
        .expect_err("failed fresh attachment must not resurrect the removed connection");

    let reactivate_error = harness
        .dispatch(LocalDaemonRequest::SetWorkflowEventBindingStatus(
            SetWorkflowEventBindingStatusRequest {
                session_id: graph.session_id,
                binding_id: binding.id,
                status: WorkflowEventBindingStatus::Active,
            },
        ))
        .expect_err("a removed connection must not allow a binding to reactivate");
    assert!(reactivate_error
        .to_string()
        .contains("removed or is not installed"));
}

#[test]
fn reduced_connection_scopes_block_binding_reactivation_and_transfer() {
    let server = ReadyConnectionServer::start();
    let target = server.target();
    let mut config = crate::DaemonConfig::for_tests();
    config.event_registry_url = Some(target.url.clone());
    config
        .event_generator_management_targets
        .insert("dev.chariox.dummy".to_string(), target);
    let harness = LocalRouterTestHarness::with_config(config);
    let source = create_publication_test_graph(&harness, "event-scope-source");
    let target = create_publication_test_graph(&harness, "event-scope-target");

    let create_publication =
        |session_id: &str, workflow_id: &str, endpoint_id: &str, alias: &str| match harness
            .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
                CreateWorkflowPublicationRequest {
                    session_id: session_id.to_string(),
                    workflow_ref: workflow_id.to_string(),
                    endpoint_ref: endpoint_id.to_string(),
                    expected_workflow_revision: None,
                    operation_key: Some(format!("publish-{alias}")),
                    queue_ref: Some("default".to_string()),
                    alias: Some(alias.to_string()),
                    kind: Some("event_based".to_string()),
                    route: None,
                    methods: Vec::new(),
                    transport: None,
                    parser: None,
                    input_schema: None,
                    trace_exposure: None,
                    mode: None,
                    sync_timeout_ms: None,
                    poll_ms: None,
                },
            ))
            .expect("event publication should be created")
        {
            LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
            response => panic!("unexpected response: {response:?}"),
        };
    let source_publication = create_publication(
        &source.session_id,
        &source.workflow_id,
        &source.endpoint_id,
        "event-scope-source",
    );
    let target_publication = create_publication(
        &target.session_id,
        &target.workflow_id,
        &target.endpoint_id,
        "event-scope-target",
    );
    let runtime_state = harness.runtime_state();
    let connection_registry = runtime_state.event_connection_registry();
    connection_registry
        .upsert(
            crate::session::DEFAULT_LOCAL_USER_ID,
            chariox_event_protocol::AegsConnectionSummary {
                generator_id: "dev.chariox.dummy".to_string(),
                connection_id: "connection-local".to_string(),
                status: chariox_event_protocol::AegsConnectionStatus::Ready,
                metadata: serde_json::json!({"account": "local"}),
                expires_at_ms: None,
                updated_at_ms: 1,
            },
        )
        .expect("installed connection should be registered");
    connection_registry
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
        .expect("installed connection grants should be recorded");
    let binding =
        match harness
            .dispatch(LocalDaemonRequest::CreateWorkflowEventBinding(
                CreateWorkflowEventBindingRequest {
                    session_id: source.session_id.clone(),
                    publication_ref: source_publication.id().to_string(),
                    generator_id: "dev.chariox.dummy".to_string(),
                    generator_version: "1.0.0".to_string(),
                    manifest_digest:
                        crate::runtime::event_catalog_control::BUILTIN_DUMMY_MANIFEST_DIGEST
                            .to_string(),
                    connection_id: "connection-local".to_string(),
                    connection_scope: "tenant:local".to_string(),
                    event_type: "dummy.test".to_string(),
                    event_type_version: 1,
                    filter: serde_json::json!({"channel": "scoped"}),
                    environment_id: Some("environment-scoped".to_string()),
                    queue_ref: Some("default".to_string()),
                    reply_mode: None,
                    action_ids: Vec::new(),
                },
            ))
            .expect("initial grants should allow binding creation")
        {
            LocalDaemonResponse::WorkflowEventBindingCreated { binding, .. } => binding,
            response => panic!("unexpected response: {response:?}"),
        };
    harness
        .dispatch(LocalDaemonRequest::SetWorkflowEventBindingStatus(
            SetWorkflowEventBindingStatusRequest {
                session_id: source.session_id.clone(),
                binding_id: binding.id.clone(),
                status: WorkflowEventBindingStatus::Paused,
            },
        ))
        .expect("binding should pause before the provider grant changes");

    server.scopes_reduced.store(true, Ordering::Relaxed);
    harness
        .dispatch(LocalDaemonRequest::RefreshEventConnection(
            RefreshEventConnectionRequest {
                connection_id: binding.connection_id.clone(),
            },
        ))
        .expect("connection refresh should persist the reduced grants");

    let reactivate_error = harness
        .dispatch(LocalDaemonRequest::SetWorkflowEventBindingStatus(
            SetWorkflowEventBindingStatusRequest {
                session_id: source.session_id.clone(),
                binding_id: binding.id.clone(),
                status: WorkflowEventBindingStatus::Active,
            },
        ))
        .expect_err("reactivation must revalidate the stored event contract");
    assert!(reactivate_error
        .to_string()
        .contains("missing required scopes: events:read"));

    let transfer_error = harness
        .dispatch(LocalDaemonRequest::TransferWorkflowEventBinding(
            TransferWorkflowEventBindingRequest {
                source_session_id: source.session_id.clone(),
                binding_id: binding.id.clone(),
                target_session_id: target.session_id.clone(),
                target_publication_ref: target_publication.id().to_string(),
            },
        ))
        .expect_err("transfer must revalidate the stored event contract");
    assert!(transfer_error
        .to_string()
        .contains("missing required scopes: events:read"));

    let source_bindings = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowEventBindings(
            ListWorkflowEventBindingsRequest {
                session_id: source.session_id,
                publication_ref: Some(source_publication.id().to_string()),
            },
        ))
        .expect("source bindings should resolve")
    {
        LocalDaemonResponse::WorkflowEventBindingsListed { bindings } => bindings,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(source_bindings.len(), 1);
    assert_eq!(
        source_bindings[0].status,
        WorkflowEventBindingStatus::Paused
    );
    let target_bindings = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowEventBindings(
            ListWorkflowEventBindingsRequest {
                session_id: target.session_id,
                publication_ref: Some(target_publication.id().to_string()),
            },
        ))
        .expect("target bindings should resolve")
    {
        LocalDaemonResponse::WorkflowEventBindingsListed { bindings } => bindings,
        response => panic!("unexpected response: {response:?}"),
    };
    assert!(target_bindings.is_empty());
}
