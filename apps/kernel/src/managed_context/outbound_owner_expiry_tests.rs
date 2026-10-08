//! Expired consumed owner operations must not reuse Cloud's unique context ID.
use super::tests::{persisted_test_artifact, write_persisted_test_artifact};
use super::*;
use crate::config::PersistedCloudRelayProfile;

#[tokio::test(flavor = "current_thread")]
async fn expired_consumed_owner_context_retires_before_unique_context_reissuance() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    use crate::managed_context::owner_managed::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let parent = std::env::temp_dir().join(format!(
        "chariox-owner-expiry-{:032x}",
        rand::random::<u128>()
    ));
    create_private_directory(&parent).unwrap();
    let _cleanup = ArtifactRootCleanup::new(parent.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = DaemonConfig::for_tests();
    config.user_config.state.path = Some(parent.join("state.db").display().to_string());
    config.user_config_path = parent.join("config.toml");
    config.user_config.history.operational.path =
        Some(parent.join("history.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(parent.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(parent.join("artifacts.db").display().to_string());
    config = config.with_session_history_root(parent.join("sessions"));
    config.cloud_relay = Some(PersistedCloudRelayProfile {
        api_url: format!("http://{}", listener.local_addr().unwrap()),
        account_id: "account".into(),
        user_id: "owner".into(),
        realm_id: "realm".into(),
        machine_id: Some(config.host_machine_id.clone()),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-test-enrollment".into()),
        kernel_public_key_thumbprint: Some(public_key_thumbprint(&config.relay_public_key)),
        ..Default::default()
    });
    let app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let runtime = router.runtime_state();
    let selection = OwnerManagedTransfer {
        target: ManagedContextTransferTarget {
            relay_realm_id: "realm".into(),
            machine_id: "target-machine".into(),
            kernel_id: "target-kernel".into(),
            relay_public_key: config.relay_public_key.clone(),
            key_thumbprint: public_key_thumbprint(&config.relay_public_key),
        },
        context_selection: OwnerManagedContextSelection {
            kernel_context: OwnerManagedKernelSelection::Empty,
            development_setup: OwnerManagedDevelopmentSelection::Empty,
        },
    };
    let store = ManagedContextOutboundOperationStore::open(parent.join("outbound")).unwrap();
    let ticket = store
        .prepare_owner_ticket(&config, &runtime, selection.clone())
        .unwrap();
    let plan = ticket.context_plan.package_binding();
    let (_, permit) = store.start(&plan.context_id, &plan.plan_digest).unwrap();
    store.update(&plan.context_id, |status| {
        status.phase = ManagedContextOutboundOperationPhase::Failed;
        status.retryable = true;
    });
    drop(permit);
    store.finish(&plan.context_id);
    let root = parent.join("outbound").join(&plan.context_id);
    write_persisted_test_artifact(
        &root,
        &ticket,
        crate::session::unix_epoch_ms(),
        Some(b"package"),
    );
    let mut persisted = persisted_test_artifact(&ticket, crate::session::unix_epoch_ms(), 7);
    persisted.destination = plan.destination.clone();
    crate::config::write_private_file(
        &root.join("state.json"),
        &serde_json::to_vec(&persisted).unwrap(),
    )
    .unwrap();
    crate::config::write_private_file(&root.join("transfer.json"), b"original-checkpoint").unwrap();
    // Cloud retains the expired consumed row and keeps contextId unique.
    // A missing row and an expired row both return authorization_expired.
    let consumed_ids = Arc::new(Mutex::new(BTreeSet::from([plan.context_id.clone()])));
    let attempts = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = attempts.clone();
    let source = serde_json::json!({"kernelId": config.daemon_id, "machineId": config.host_machine_id,
            "relayRealmId": "realm", "relayPublicKey": config.relay_public_key, "keyThumbprint": public_key_thumbprint(&config.relay_public_key)});
    let serialized_plan = serde_json::to_value(&ticket.context_plan).unwrap();
    let binding = serde_json::json!({"kind": "owner_managed_machine", "sourceTargetId": "source-directory-id", "source": source,
            "target": ticket.target, "contextSelection": {"kernelContext": serialized_plan["kernelContext"], "developmentSetup": serialized_plan["developmentSetup"]}});
    let ids = consumed_ids.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let (headers, body) = loop {
                let mut bytes = [0; 4096];
                let n = stream.read(&mut bytes).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&bytes[..n]);
                let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8(request[..end].to_vec()).unwrap();
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() < end + 4 + length {
                    continue;
                }
                let body = if length == 0 {
                    serde_json::json!({})
                } else {
                    serde_json::from_slice(&request[end + 4..end + 4 + length]).unwrap()
                };
                break (headers, body);
            };
            let (code, response) = if headers
                .starts_with("GET /v1/owner-managed-context-tickets/peers ")
            {
                (
                    200,
                    serde_json::json!({"peers": [{"targetId": "source-directory-id", "peer": source}]}),
                )
            } else if headers.starts_with("POST /v1/owner-managed-context-tickets ") {
                let mut issued = binding.clone();
                issued["ticket"] = serde_json::json!("synthetic-single-use-ticket");
                (200, issued)
            } else if body.get("ownerManagedExport").is_some() {
                (
                    403,
                    serde_json::json!({"error": {"code": "authorization_expired", "message": "Binding missing or expired"}}),
                )
            } else {
                let id = body["ownerManaged"]["contextId"]
                    .as_str()
                    .unwrap()
                    .to_string();
                observed.lock().unwrap().push(id.clone());
                if ids.lock().unwrap().insert(id) {
                    (200, binding.clone())
                } else {
                    (
                        500,
                        serde_json::json!({"error": {"code": "internal_error", "message": "Unique contextId"}}),
                    )
                }
            };
            let response = serde_json::to_vec(&response).unwrap();
            stream.write_all(format!("HTTP/1.1 {code} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).as_bytes()).await.unwrap();
            stream.write_all(&response).await.unwrap();
        }
    });
    async fn failure(
        store: &ManagedContextOutboundOperationStore,
        id: &str,
    ) -> ManagedContextOutboundOperationStatus {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let status = store.get(id).unwrap();
                if status.phase == ManagedContextOutboundOperationPhase::Failed
                    && !store.active_context_ids().contains(id)
                {
                    break status;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    // Reopen to exercise the persisted retry binding, not just in-memory state.
    let store = ManagedContextOutboundOperationStore::open(parent.join("outbound")).unwrap();
    assert_eq!(
        store
            .prepare_owner_ticket(&config, &runtime, selection.clone())
            .unwrap(),
        ticket
    );
    start_managed_context_outbound_operation(
        config.clone(),
        Arc::new(RwLock::new(RelayClientState::default())),
        store.clone(),
        runtime.provider_account_profile_registry().clone(),
        ticket,
        None,
        false,
    )
    .unwrap();
    let expired = failure(&store, &plan.context_id).await;
    assert!(!expired.retryable, "expired consumed context must not become a retryable unique-contextId failure: {expired:?}");
    assert_eq!(
        expired.failure_code.as_deref(),
        Some("owner_context_authorization_expired")
    );
    assert!(
        attempts.lock().unwrap().is_empty(),
        "never consume a replacement ticket for an existing expired context"
    );
    assert!(root.join("retired").exists());
    assert!(!root.join("managed-context.pkg").exists());
    assert!(!root.join("transfer.json").exists());
    let replacement = store
        .prepare_owner_ticket(&config, &runtime, selection)
        .unwrap();
    let fresh_id = replacement.context_plan.context_id().to_string();
    assert_ne!(fresh_id, plan.context_id);
    start_managed_context_outbound_operation(
        config,
        Arc::new(RwLock::new(RelayClientState::default())),
        store.clone(),
        runtime.provider_account_profile_registry().clone(),
        replacement,
        None,
        false,
    )
    .unwrap();
    let fresh = failure(&store, &fresh_id).await;
    // Ticket consumption succeeds; native owner confirmation is the next gate.
    assert!(fresh
        .failure_message
        .unwrap()
        .contains("native source confirmation"));
    assert_eq!(*attempts.lock().unwrap(), vec![fresh_id]);
    assert_eq!(consumed_ids.lock().unwrap().len(), 2);
    server.abort();
}
