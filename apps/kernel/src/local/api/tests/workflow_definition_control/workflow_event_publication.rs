use super::*;
use crate::local::{
    DisableWorkflowPublicationRequest, GetEventConnectionRequest, GetEventDeliveryStatusRequest,
    ListEventConnectionDependenciesRequest, ListWorkflowEventBindingsRequest,
    ObserveEventConnectionAuthorizationRequest, ReconnectEventConnectionRequest,
    RefreshEventConnectionRequest, RemoveEventConnectionRequest,
    SetWorkflowEventBindingStatusRequest,
};
use crate::session::WorkflowEventBindingStatus;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct ReadyConnectionServer {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    revoked: Arc<AtomicBool>,
    unavailable: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ReadyConnectionServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let revoked = Arc::new(AtomicBool::new(false));
        let unavailable = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_revoked = Arc::clone(&revoked);
        let thread_unavailable = Arc::clone(&unavailable);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        serve_ready_connection(&mut stream, &thread_revoked, &thread_unavailable)
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

fn serve_ready_connection(stream: &mut TcpStream, revoked: &AtomicBool, unavailable: &AtomicBool) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return;
            }
            Err(error) => panic!("event connection test request read failed: {error}"),
        };
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    if request.is_empty() {
        return;
    }
    let request = String::from_utf8_lossy(&request);
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer test-management-token"));
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
    let body = if request.starts_with("POST /v1/connections/query HTTP/1.1") {
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
            "scopes": [],
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
            assert_eq!(package_version, 3);
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
    assert!(!binding_template.contains("connection-local"));
    let publication_json = package_json_file(&package_files, "publication.json");
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
