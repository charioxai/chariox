use super::*;
use crate::local::{
    DisableWorkflowPublicationRequest, GetEventDeliveryStatusRequest,
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
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ReadyConnectionServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => serve_ready_connection(&mut stream),
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
            thread: Some(thread),
        }
    }

    fn target(&self) -> crate::config::EventGeneratorManagementTarget {
        crate::config::EventGeneratorManagementTarget {
            url: format!("http://{}", self.address),
            token: "test-management-token".to_string(),
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

fn serve_ready_connection(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&request);
    assert!(request.starts_with("POST /v1/connections/query HTTP/1.1"));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer test-management-token"));
    let body = serde_json::json!({
        "connections": [{
            "generator_id": "dev.arroba.dummy",
            "connection_id": "connection-local",
            "status": "ready",
            "metadata": {"account": "local"},
            "updated_at_ms": 1
        }]
    })
    .to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
}

#[test]
fn event_publication_binding_is_environment_exclusive_and_uses_workflow_queue() {
    let server = ReadyConnectionServer::start();
    let mut config = crate::DaemonConfig::for_tests();
    config
        .event_generator_management_targets
        .insert("dev.arroba.dummy".to_string(), server.target());
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
        generator_id: "dev.arroba.dummy".to_string(),
        generator_version: "1.0.0".to_string(),
        manifest_digest: format!("sha256:{}", "a".repeat(64)),
        connection_id: "connection-local".to_string(),
        connection_scope: "tenant:local".to_string(),
        event_type: "dummy.test".to_string(),
        event_type_version: 1,
        filter: serde_json::json!({"channel": "default"}),
        environment_id: Some("environment-local".to_string()),
        queue_ref: Some("default".to_string()),
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
            assert_eq!(session.workflow_event_bindings(), [binding.clone()]);
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
    let duplicate_error = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEventBinding(
            binding_request(second.id()),
        ))
        .expect_err("same event interest must not be active twice in one environment");
    assert!(duplicate_error.to_string().contains("event route conflict"));
    assert!(duplicate_error.to_string().contains(first.id()));

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
    assert_eq!(tested.workflow_event_bindings().len(), 1);

    let active_route_count = || match harness
        .dispatch(LocalDaemonRequest::GetEventDeliveryStatus(
            GetEventDeliveryStatusRequest,
        ))
        .expect("event delivery status should resolve")
    {
        LocalDaemonResponse::EventDeliveryStatus { status } => status.active_route_count,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(active_route_count(), 1);
    harness.runtime_state().apply_event_route_conflicts(&[
        arroba_event_protocol::EventRouteConflict {
            environment_id: binding.environment_id.clone(),
            event_interest_key: binding.event_interest_key.clone(),
            requested_binding_id: binding.id.clone(),
            existing_binding_id: "binding-on-new-kernel".to_string(),
            existing_publication_id: second.id().to_string(),
        },
    ]);
    assert_eq!(
        active_route_count(),
        0,
        "an AEDS ownership conflict must fence the old kernel route"
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
    assert_eq!(active_route_count(), 1);

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
        0,
        "a disabled publication must not continue advertising an active AEDS route"
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
}
