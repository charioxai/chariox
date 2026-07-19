use super::*;
use std::sync::Arc;
use tokio::sync::Mutex;

#[test]
fn remote_extension_manifest_pending_revoke_uses_explicit_intent_not_hash_change() {
    let previous = crate::extension::RemoteExtensionManifestSyncStatus::synced(
        "hash-before-grant".to_string(),
    );

    assert!(!remote_extension_manifest_pending_revoke(
        Some(&previous),
        Some(false),
    ));
    assert!(remote_extension_manifest_pending_revoke(
        Some(&previous),
        Some(true),
    ));
}

#[test]
fn remote_extension_manifest_pending_revoke_preserves_retry_state_only_without_intent() {
    let pending_revoke = crate::extension::RemoteExtensionManifestSyncStatus::pending(
        "hash-after-revoke".to_string(),
        true,
    )
    .failed("worker unavailable".to_string());

    assert!(remote_extension_manifest_pending_revoke(
        Some(&pending_revoke),
        None,
    ));
    assert!(!remote_extension_manifest_pending_revoke(
        Some(&pending_revoke),
        Some(false),
    ));
}

#[test]
fn worker_sync_restart_rejects_persisted_synced_status_for_stale_manifest() {
    let mut agent = worker_sync_test_agent();
    agent.grant_extension(
        crate::extension::ExtensionGrant::new(crate::extension::ExtensionKind::Mcp, "worker-a")
            .from_source(crate::extension::ExtensionSource::Worker),
    );
    agent.set_worker_extension_grant_sync(Some(
        crate::extension::RemoteExtensionManifestSyncStatus::synced("stale-hash".to_string()),
    ));
    let restored: crate::agent::AgentInstance =
        serde_json::from_value(serde_json::to_value(&agent).expect("agent should serialize"))
            .expect("agent should restore");
    let current_hash = crate::extension::extension_grant_manifest_hash(
        &restored.extension_grants_from(crate::extension::ExtensionSource::Worker),
    )
    .expect("manifest should hash");

    assert!(!worker_extension_grants_are_synced(
        &restored,
        &current_hash
    ));
}

#[test]
fn worker_sync_add_rejects_synced_status_for_pre_add_manifest() {
    let mut agent = worker_sync_test_agent();
    let empty_hash =
        crate::extension::extension_grant_manifest_hash(&[]).expect("empty manifest should hash");
    agent.grant_extension(
        crate::extension::ExtensionGrant::new(crate::extension::ExtensionKind::Skill, "worker-a")
            .from_source(crate::extension::ExtensionSource::Worker),
    );
    agent.set_worker_extension_grant_sync(Some(
        crate::extension::RemoteExtensionManifestSyncStatus::synced(empty_hash),
    ));
    let current_hash = crate::extension::extension_grant_manifest_hash(
        &agent.extension_grants_from(crate::extension::ExtensionSource::Worker),
    )
    .expect("manifest should hash");

    assert!(!worker_extension_grants_are_synced(&agent, &current_hash));
}

#[test]
fn worker_sync_revoke_rejects_synced_status_for_pre_revoke_manifest() {
    let mut agent = worker_sync_test_agent();
    agent.grant_extension(
        crate::extension::ExtensionGrant::new(crate::extension::ExtensionKind::Mcp, "worker-a")
            .from_source(crate::extension::ExtensionSource::Worker),
    );
    let granted_hash = crate::extension::extension_grant_manifest_hash(
        &agent.extension_grants_from(crate::extension::ExtensionSource::Worker),
    )
    .expect("manifest should hash");
    agent.revoke_extension(
        crate::extension::ExtensionSource::Worker,
        crate::extension::ExtensionKind::Mcp,
        "worker-a",
    );
    agent.set_worker_extension_grant_sync(Some(
        crate::extension::RemoteExtensionManifestSyncStatus::synced(granted_hash),
    ));
    let current_hash =
        crate::extension::extension_grant_manifest_hash(&[]).expect("empty manifest should hash");

    assert!(!worker_extension_grants_are_synced(&agent, &current_hash));
}

fn worker_sync_test_agent() -> crate::agent::AgentInstance {
    crate::agent::AgentInstance::new(
        "agent-worker-sync",
        "agent-worker-sync",
        "session-worker-sync",
        None,
        "codex",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    )
}

#[tokio::test]
async fn agent_config_update_ignores_legacy_processing_without_active_prompt() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    {
        let app = app.lock().await;
        app.agents_mut()
            .set_agent_processing(&agent_id, true)
            .expect("agent processing should update");
    }

    let agent = runtime
        .update_agent_config(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some(Some("workspace-next".to_string())),
            None,
        )
        .await
        .expect("stale legacy processing alone should not block config update");

    assert_eq!(agent.workspace_id(), Some("workspace-next"));
}

#[tokio::test]
async fn agent_profile_update_ignores_legacy_processing_without_active_prompt() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    {
        let app = app.lock().await;
        app.agents_mut()
            .set_agent_processing(&agent_id, true)
            .expect("agent processing should update");
    }

    let agent = runtime
        .update_agent_profile(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            Some("opencode".to_string()),
            Some("model-next".to_string()),
            None,
        )
        .await
        .expect("stale legacy processing alone should not block profile update");

    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), Some("model-next"));
}

#[tokio::test]
async fn agent_config_update_still_blocks_active_prompt_owner() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    sync_active_prompt(&app, &session_id, &agent_id).await;

    let error = runtime
        .update_agent_config(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some(Some("workspace-next".to_string())),
            None,
        )
        .await
        .expect_err("active prompt ownership should block config update");

    assert_active_turn_error(error, "update agent config");
}

#[tokio::test]
async fn remote_agent_config_update_uses_connected_relay_without_metadata_socket() {
    let mut config = crate::config::DaemonConfig::for_tests();
    let relay_url = "ws://127.0.0.1:1".to_string();
    config.relay_url = Some(relay_url.clone());
    config.relay_token = Some("relay-token".to_string());
    let home_public_key = config.relay_public_key.clone();
    let target_config = crate::config::DaemonConfig::for_tests();
    let (app, runtime, session_id, agent_id) = agent_config_runtime_with_config(config).await;
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
                active_worker_provider_run_id: Some("worker-run-old".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("agent should bind to remote execution");

    let relay_state = {
        let app = app.lock().await;
        app.relay_client_state()
    };
    let (outgoing_tx, mut priority_rx, _event_rx) =
        crate::transport::relay_client::RelayOutgoingSender::channel(4);
    {
        let mut relay_state = relay_state.write().await;
        relay_state.test_set_connected_sender(outgoing_tx, relay_url);
        relay_state.remember_peer_public_key("worker-1", target_config.relay_public_key.clone());
    }

    let update = tokio::spawn({
        let runtime = runtime.clone();
        let session_id = session_id.clone();
        let agent_id = agent_id.clone();
        async move {
            runtime
                .update_agent_config(
                    &session_id,
                    &agent_id,
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    Some(Some(crate::provider::AgentExecutionMode::Plan)),
                    Some(Some(crate::provider::AgentPermissionLevel::Required)),
                    None,
                    None,
                )
                .await
        }
    });

    let envelope = tokio::time::timeout(std::time::Duration::from_millis(500), priority_rx.recv())
        .await
        .expect("config update should use the connected relay instead of opening metadata sockets")
        .expect("connected relay request should be queued");
    let arroba_relay::protocol::RelayEnvelope::DaemonPeerRequest {
        request_id,
        target,
        encrypted_request,
    } = envelope
    else {
        panic!("expected a daemon peer request");
    };
    assert_eq!(target.daemon_id.as_deref(), Some("worker-1"));
    let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        &target_config.relay_private_key,
        &encrypted_request,
    )
    .expect("worker should decrypt config request");
    assert!(matches!(
        serde_json::from_slice::<crate::transport::relay_peer::RelayPeerRequest>(
            &decrypted.plaintext
        )
        .expect("config request should decode"),
        crate::transport::relay_peer::RelayPeerRequest::UpdateLeasedAgentConfig {
            leased_agent_id,
            execution_mode: crate::provider::AgentExecutionMode::Plan,
            permission_level: crate::provider::AgentPermissionLevel::Required,
        } if leased_agent_id == "leased-agent-1"
    ));

    let response = crate::transport::relay_peer::RelayPeerResponse::LeasedAgentConfigUpdated {
        leased_agent: crate::execution_lease::LeasedAgent {
            id: "leased-agent-1".to_string(),
            lease_id: "lease-1".to_string(),
            home_agent_id: agent_id.clone(),
            provider: "dev-stub".to_string(),
            model: None,
            effort: None,
            execution_mode: Some(crate::provider::AgentExecutionMode::Plan),
            permission_level: Some(crate::provider::AgentPermissionLevel::Required),
            backing_session_id: "worker-session-1".to_string(),
            backing_agent_id: "worker-agent-1".to_string(),
            backing_attachment_id: "worker-attachment-1".to_string(),
            projected_prompt_ids: Vec::new(),
            projected_completion_keys: Vec::new(),
            projected_output_history_keys: Vec::new(),
            projected_provider_run: None,
            active_home_prompt_id: None,
            active_home_prompt_started_at_ms: None,
            applied_home_steer_ids: Vec::new(),
            replayable_completion: None,
            created_at_ms: 1,
        },
    };
    let encrypted_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &target_config.relay_private_key,
        &home_public_key,
        &serde_json::to_vec(&response).expect("config response should encode"),
    )
    .expect("worker should encrypt config response");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &relay_state,
        request_id,
        "worker-1".to_string(),
        encrypted_response,
    )
    .await;

    let updated = update
        .await
        .expect("config update task should join")
        .expect("config update should complete through the connected relay");
    assert_eq!(
        updated.execution_mode_override(),
        Some(crate::provider::AgentExecutionMode::Plan)
    );
    assert_eq!(
        updated.permission_level_override(),
        Some(crate::provider::AgentPermissionLevel::Required)
    );

    let profile_update = tokio::spawn({
        let runtime = runtime.clone();
        let session_id = session_id.clone();
        let agent_id = agent_id.clone();
        async move {
            runtime
                .update_agent_profile(
                    &session_id,
                    &agent_id,
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    Some("codex".to_string()),
                    Some("gpt-5.4".to_string()),
                    Some(Some("high".to_string())),
                )
                .await
        }
    });
    let envelope = tokio::time::timeout(std::time::Duration::from_millis(500), priority_rx.recv())
        .await
        .expect("profile update should use the connected relay")
        .expect("connected relay profile request should be queued");
    let arroba_relay::protocol::RelayEnvelope::DaemonPeerRequest {
        request_id,
        target: _,
        encrypted_request,
    } = envelope
    else {
        panic!("expected a daemon peer request");
    };
    let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        &target_config.relay_private_key,
        &encrypted_request,
    )
    .expect("worker should decrypt profile request");
    assert!(matches!(
        serde_json::from_slice::<crate::transport::relay_peer::RelayPeerRequest>(
            &decrypted.plaintext
        )
        .expect("profile request should decode"),
        crate::transport::relay_peer::RelayPeerRequest::UpdateLeasedAgentProfile {
            leased_agent_id,
            provider,
            model,
            effort,
        } if leased_agent_id == "leased-agent-1"
            && provider == "codex"
            && model.as_deref() == Some("gpt-5.4")
            && effort.as_deref() == Some("high")
    ));
    let response = crate::transport::relay_peer::RelayPeerResponse::LeasedAgentProfileUpdated {
        leased_agent: crate::execution_lease::LeasedAgent {
            id: "leased-agent-1".to_string(),
            lease_id: "lease-1".to_string(),
            home_agent_id: agent_id.clone(),
            provider: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
            effort: Some("high".to_string()),
            execution_mode: Some(crate::provider::AgentExecutionMode::Plan),
            permission_level: Some(crate::provider::AgentPermissionLevel::Required),
            backing_session_id: "worker-session-1".to_string(),
            backing_agent_id: "worker-agent-1".to_string(),
            backing_attachment_id: "worker-attachment-1".to_string(),
            projected_prompt_ids: Vec::new(),
            projected_completion_keys: Vec::new(),
            projected_output_history_keys: Vec::new(),
            projected_provider_run: None,
            active_home_prompt_id: None,
            active_home_prompt_started_at_ms: None,
            applied_home_steer_ids: Vec::new(),
            replayable_completion: None,
            created_at_ms: 1,
        },
    };
    let encrypted_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &target_config.relay_private_key,
        &home_public_key,
        &serde_json::to_vec(&response).expect("profile response should encode"),
    )
    .expect("worker should encrypt profile response");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &relay_state,
        request_id,
        "worker-1".to_string(),
        encrypted_response,
    )
    .await;
    let updated = profile_update
        .await
        .expect("profile update task should join")
        .expect("profile update should complete through the connected relay");
    assert_eq!(updated.provider(), "codex");
    assert_eq!(updated.model(), Some("gpt-5.4"));
    assert_eq!(updated.effort(), Some("high"));
    assert_eq!(
        updated
            .remote_execution()
            .and_then(|binding| binding.active_worker_provider_run_id.as_deref()),
        None
    );
}

#[tokio::test(flavor = "current_thread")]
async fn home_manifest_sync_replaces_stale_definition_response_before_marking_synced() {
    let _guard = crate::env_lock::lock();
    let isolation_root = std::env::temp_dir().join(format!(
        "arroba-home-manifest-cas-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_dir_all(&isolation_root);
    std::env::set_var("ARROBA_CAPABILITY_ISOLATION_ROOT", &isolation_root);

    let registry =
        crate::mcp::ArrobaMcpRegistry::new(vec![crate::mcp::ArrobaMcpRegistry::project_root(
            "workspace-1",
        )]);
    let definition_a = crate::mcp::ArrobaMcpServerConfig::stdio(
        "definition-cas",
        "node",
        vec!["definition-a.js".to_string()],
    );
    registry
        .install(&definition_a)
        .expect("definition A should install");

    let mut config = crate::config::DaemonConfig::for_tests();
    let relay_url = "ws://127.0.0.1:1".to_string();
    config.relay_url = Some(relay_url.clone());
    config.relay_token = Some("relay-token".to_string());
    let home_public_key = config.relay_public_key.clone();
    let target_config = crate::config::DaemonConfig::for_tests();
    let (app, runtime, _session_id, agent_id) = agent_config_runtime_with_config(config).await;
    let agent = {
        let app = app.lock().await;
        let bound = app.agents_mut().bind_remote_execution(
            &agent_id,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-definition-cas".to_string(),
                worker_machine_id: "machine-definition-cas".to_string(),
                execution_lease_id: "lease-definition-cas".to_string(),
                leased_agent_id: "leased-agent-definition-cas".to_string(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
            },
        );
        bound.expect("agent should bind")
    };
    let agent = {
        let app = app.lock().await;
        let granted = app.agents_mut().grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::new(
                crate::extension::ExtensionKind::Mcp,
                "definition-cas",
            ),
        );
        granted.expect("home MCP should grant")
    };

    let relay_state = app.lock().await.relay_client_state();
    let (outgoing_tx, mut priority_rx, _event_rx) =
        crate::transport::relay_client::RelayOutgoingSender::channel(4);
    {
        let mut relay_state = relay_state.write().await;
        relay_state.test_set_connected_sender(outgoing_tx, relay_url);
        relay_state.remember_peer_public_key(
            "worker-definition-cas",
            target_config.relay_public_key.clone(),
        );
    }

    let sync = tokio::spawn({
        let runtime = runtime.clone();
        let agent = agent.clone();
        async move {
            runtime
                .sync_remote_extension_manifest_for_agent(&agent, None, Some(false))
                .await
        }
    });
    let first = receive_home_manifest_request(&mut priority_rx, &target_config).await;

    let definition_b = crate::mcp::ArrobaMcpServerConfig::stdio(
        "definition-cas",
        "node",
        vec!["definition-b.js".to_string()],
    );
    registry
        .update(&definition_b)
        .expect("definition B should replace A");
    resolve_home_manifest_response(&relay_state, &target_config, &home_public_key, first.0).await;

    let second = receive_home_manifest_request(&mut priority_rx, &target_config).await;
    let first_manifest_hash = first.1.manifest_hash();
    let second_manifest_hash = second.1.manifest_hash();
    assert_ne!(
        first_manifest_hash, second_manifest_hash,
        "the registry definition must participate in the synchronized manifest hash"
    );
    resolve_home_manifest_response(&relay_state, &target_config, &home_public_key, second.0).await;
    sync.await
        .expect("sync task should join")
        .expect("replacement sync should succeed");

    let current = app
        .lock()
        .await
        .agents()
        .get_agent(&agent_id)
        .expect("agent should remain available");
    assert_eq!(
        current
            .remote_extension_manifest_sync()
            .and_then(|status| status.manifest_hash.as_deref()),
        Some(second_manifest_hash.as_str()),
    );

    std::env::remove_var("ARROBA_CAPABILITY_ISOLATION_ROOT");
    let _ = std::fs::remove_dir_all(isolation_root);
}

async fn receive_home_manifest_request(
    priority_rx: &mut tokio::sync::mpsc::Receiver<arroba_relay::protocol::RelayEnvelope>,
    target_config: &crate::config::DaemonConfig,
) -> (String, crate::extension::RemoteExtensionManifest) {
    let envelope = tokio::time::timeout(std::time::Duration::from_millis(500), priority_rx.recv())
        .await
        .expect("home manifest request should use the connected relay")
        .expect("connected relay request should be queued");
    let arroba_relay::protocol::RelayEnvelope::DaemonPeerRequest {
        request_id,
        encrypted_request,
        ..
    } = envelope
    else {
        panic!("expected a daemon peer request");
    };
    let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        &target_config.relay_private_key,
        &encrypted_request,
    )
    .expect("worker should decrypt manifest request");
    let request = serde_json::from_slice::<crate::transport::relay_peer::RelayPeerRequest>(
        &decrypted.plaintext,
    )
    .expect("manifest request should decode");
    let crate::transport::relay_peer::RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
        leased_agent_id,
        remote_extension_manifest,
    } = request
    else {
        panic!("expected a home extension manifest request");
    };
    assert_eq!(leased_agent_id, "leased-agent-definition-cas");
    (request_id, remote_extension_manifest)
}

async fn resolve_home_manifest_response(
    relay_state: &std::sync::Arc<
        tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>,
    >,
    target_config: &crate::config::DaemonConfig,
    home_public_key: &str,
    request_id: String,
) {
    let response =
        crate::transport::relay_peer::RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated {
            leased_agent_id: "leased-agent-definition-cas".to_string(),
        };
    let encrypted_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &target_config.relay_private_key,
        home_public_key,
        &serde_json::to_vec(&response).expect("manifest response should encode"),
    )
    .expect("worker should encrypt manifest response");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        relay_state,
        request_id,
        "worker-definition-cas".to_string(),
        encrypted_response,
    )
    .await;
}

#[tokio::test]
async fn agent_profile_update_still_blocks_active_prompt_owner() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    sync_active_prompt(&app, &session_id, &agent_id).await;

    let error = runtime
        .update_agent_profile(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            Some("opencode".to_string()),
            Some("model-next".to_string()),
            None,
        )
        .await
        .expect_err("active prompt ownership should block profile update");

    assert_active_turn_error(error, "update agent profile");
}

async fn agent_config_runtime() -> (Arc<Mutex<DaemonApp>>, KernelRuntimeState, String, String) {
    agent_config_runtime_with_config(crate::config::DaemonConfig::for_tests()).await
}

async fn agent_config_runtime_with_config(
    config: crate::config::DaemonConfig,
) -> (Arc<Mutex<DaemonApp>>, KernelRuntimeState, String, String) {
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    (app, runtime, session_id, agent_id)
}

async fn sync_active_prompt(app: &Arc<Mutex<DaemonApp>>, session_id: &str, agent_id: &str) {
    let prompt = crate::session::PromptQueueItem::new(
        "active-prompt",
        "attachment-1",
        agent_id,
        "active prompt",
        crate::session::PromptStatus::Running,
    );
    app.lock()
        .await
        .prompt_owner_sync_external_active_prompt(session_id, agent_id, Some(prompt))
        .expect("active prompt should sync");
}

fn assert_active_turn_error(error: DaemonError, operation: &'static str) {
    match error {
        DaemonError::LocalTransport {
            operation: actual,
            message,
        } => {
            assert_eq!(actual, operation);
            assert!(message.contains("has an active turn"));
        }
        other => panic!("expected active turn error, got {other:?}"),
    }
}

async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
    let (
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        history_store,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    ) = {
        let app_locked = app.lock().await;
        (
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.history_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workflow_design_event_store(),
            app_locked.metaagent_event_store(),
            app_locked.workspace_coordinator(),
        )
    };
    KernelRuntimeState::new_with_owned_state(
        Arc::clone(app),
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        history_store,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    )
}
