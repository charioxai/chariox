use super::*;
use std::sync::Arc;
use tokio::sync::Mutex;

#[path = "tests/automatic_substitutes.rs"]
mod automatic_substitutes;

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
            None,
            Some("model-next".to_string()),
            None,
        )
        .await
        .expect("stale legacy processing alone should not block profile update");

    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), Some("model-next"));
    let durable_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_by_kind("agent.updated")
        .expect("agent update events should load");
    assert!(durable_events.iter().any(|event| {
        event
            .payload
            .get("agent")
            .and_then(|agent| agent.get("id"))
            .and_then(|id| id.as_str())
            == Some(agent_id.as_str())
            && event
                .payload
                .get("agent")
                .and_then(|agent| agent.get("provider"))
                .and_then(|provider| provider.as_str())
                == Some("opencode")
            && event
                .payload
                .get("agent")
                .and_then(|agent| agent.get("model"))
                .and_then(|model| model.as_str())
                == Some("model-next")
    }));
}

#[tokio::test]
async fn cloud_owner_agent_profile_update_resolves_host_account_namespace() {
    let cloud_owner = "cloud-owner";
    let mut config = crate::config::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: cloud_owner.to_string(),
        ..Default::default()
    });
    let (app, runtime, session_id, agent_id) =
        agent_config_runtime_with_config_and_owner(config, cloud_owner).await;
    let profile = app
        .lock()
        .await
        .provider_account_profile_registry()
        .create_managed(crate::session::DEFAULT_LOCAL_USER_ID, "codex", "Validation")
        .expect("host account profile should be created");

    let agent = runtime
        .update_agent_profile(
            &session_id,
            &agent_id,
            cloud_owner,
            Some("codex".to_string()),
            Some(profile.profile_id.clone()),
            Some("gpt-5.6-luna".to_string()),
            Some(Some("low".to_string())),
        )
        .await
        .expect("Cloud owner should resolve the host-local account profile");

    assert_eq!(agent.provider_account_profile(), profile.profile_id);
    assert_eq!(agent.model(), Some("gpt-5.6-luna"));
    assert_eq!(agent.effort(), Some("low"));
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
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
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
    let chariox_relay::protocol::RelayEnvelope::DaemonPeerRequest {
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
            account_profile: "default".to_string(),
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

    // Profile updates resolve the provider default through the account
    // authority seam, so the exact stable profile ID — not the literal
    // "default" sentinel — must cross the relay.
    let resolved_default_profile_id = {
        let app = app.lock().await;
        app.provider_account_profile_registry()
            .get(crate::session::DEFAULT_LOCAL_USER_ID, "codex", "default")
            .expect("seeded codex default should resolve")
            .profile_id
    };
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
                    None,
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
    let chariox_relay::protocol::RelayEnvelope::DaemonPeerRequest {
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
            account_profile,
            model,
            effort,
        } if leased_agent_id == "leased-agent-1"
            && provider == "codex"
            && account_profile == resolved_default_profile_id
            && account_profile != "default"
            && model.as_deref() == Some("gpt-5.4")
            && effort.as_deref() == Some("high")
    ));
    let response = crate::transport::relay_peer::RelayPeerResponse::LeasedAgentProfileUpdated {
        leased_agent: crate::execution_lease::LeasedAgent {
            id: "leased-agent-1".to_string(),
            lease_id: "lease-1".to_string(),
            home_agent_id: agent_id.clone(),
            provider: "codex".to_string(),
            account_profile: resolved_default_profile_id.clone(),
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
            None,
            Some("model-next".to_string()),
            None,
        )
        .await
        .expect_err("active prompt ownership should block profile update");

    assert_active_turn_error(error, "update agent profile");
}

#[tokio::test]
async fn substitute_add_rejects_account_not_registered_for_provider() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    let _ = app;

    let error = runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Add {
                provider: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                variant: None,
                account_profile: Some("ghost-profile".to_string()),
                kernel_id: None,
                worktree_id: None,
            },
        )
        .await
        .expect_err("unregistered substitute account must be rejected");

    match error {
        DaemonError::LocalTransport { message, .. } => {
            assert!(
                !message.contains("ghost-profile"),
                "error must not echo the internal profile id: {message}"
            );
            assert!(
                !message.contains("default"),
                "error must not leak the literal sentinel: {message}"
            );
            assert!(
                message.contains("choose an available account alias") && message.contains("codex"),
                "error should name the provider and be actionable: {message}"
            );
        }
        other => panic!("expected registry rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn substitute_lifecycle_binds_stable_account_and_primary_edit_targets_snapshot() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    let stable_profile_id = app
        .lock()
        .await
        .provider_account_profile_registry()
        .create_managed(crate::session::DEFAULT_LOCAL_USER_ID, "codex", "Sub Work")
        .expect("host account profile should be created")
        .profile_id;

    // Establish a concrete primary profile before substituting.
    let primary = runtime
        .update_agent_profile(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            Some("opencode".to_string()),
            None,
            Some("gpt-5.4".to_string()),
            Some(Some("high".to_string())),
        )
        .await
        .expect("initial primary profile should apply");
    let primary_account = primary.account_profile().map(str::to_string);

    runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Add {
                provider: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                variant: None,
                account_profile: Some(stable_profile_id.clone()),
                kernel_id: None,
                worktree_id: None,
            },
        )
        .await
        .expect("registered substitute account should bind");

    runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Activate {
                index: 0,
                reason: Some("manual".to_string()),
            },
        )
        .await
        .expect("manual activation should succeed");
    {
        let app = app.lock().await;
        let agent = app.agents().get_agent(&agent_id).expect("agent exists");
        assert_eq!(agent.provider(), "codex");
        assert_eq!(agent.provider_account_profile(), stable_profile_id);
    }

    // Primary edit while substituted retargets the primary snapshot only.
    let agent = runtime
        .update_agent_profile(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some("gpt-5.6-edited".to_string()),
            None,
        )
        .await
        .expect("primary edit while substituted should update the snapshot");
    assert_eq!(agent.model(), Some("gpt-5.4"));
    assert_eq!(agent.primary_model(), Some("gpt-5.6-edited"));
    assert_eq!(agent.active_substitute_index(), Some(0));

    // Returning to primary lands on the edited values with the exact
    // primary account.
    let agent = runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Primary {},
        )
        .await
        .expect("return to primary should succeed");
    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), Some("gpt-5.6-edited"));
    assert_eq!(agent.effort(), Some("high"));
    assert_eq!(agent.account_profile().map(str::to_string), primary_account);
}

#[tokio::test]
async fn substitute_move_updates_the_durable_order_and_active_index_atomically() {
    let (_app, runtime, session_id, agent_id) = agent_config_runtime().await;
    for (provider, model) in [("provider-a", "model-a"), ("provider-b", "model-b")] {
        runtime
            .update_agent_substitutes(
                &session_id,
                &agent_id,
                crate::session::DEFAULT_LOCAL_USER_ID,
                crate::local::AgentSubstituteAction::Add {
                    provider: provider.to_string(),
                    model: model.to_string(),
                    variant: None,
                    account_profile: None,
                    kernel_id: None,
                    worktree_id: None,
                },
            )
            .await
            .expect("substitute should be added");
    }
    runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Activate {
                index: 1,
                reason: Some("resource exhausted".to_string()),
            },
        )
        .await
        .expect("second substitute should activate");

    let agent = runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Move {
                from_index: 1,
                to_index: 0,
            },
        )
        .await
        .expect("active substitute should move");

    assert_eq!(agent.substitutes()[0].model, "model-b");
    assert_eq!(agent.active_substitute_index(), Some(0));
    assert_eq!(agent.model(), Some("model-b"));
}

#[tokio::test]
async fn substitute_add_resolves_current_default_to_stable_profile_id() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    let first_default = app
        .lock()
        .await
        .provider_account_profile_registry()
        .create_managed(crate::session::DEFAULT_LOCAL_USER_ID, "codex", "First")
        .expect("account profile should be created");
    app.lock()
        .await
        .provider_account_profile_registry()
        .set_default(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &first_default.profile_id,
        )
        .expect("default should be set");

    let agent = runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Add {
                provider: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                variant: None,
                account_profile: None,
                kernel_id: None,
                worktree_id: None,
            },
        )
        .await
        .expect("omitted alias should resolve the current default");
    assert_eq!(
        agent.substitutes()[0].account_profile.as_deref(),
        Some(first_default.profile_id.as_str())
    );

    // Changing the default afterwards must not move the bound substitute.
    let second = app
        .lock()
        .await
        .provider_account_profile_registry()
        .create_managed(crate::session::DEFAULT_LOCAL_USER_ID, "codex", "Second")
        .expect("second profile should be created");
    app.lock()
        .await
        .provider_account_profile_registry()
        .set_default(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &second.profile_id,
        )
        .expect("default switch should succeed");
    {
        let app = app.lock().await;
        let agent = app.agents().get_agent(&agent_id).expect("agent exists");
        assert_eq!(
            agent.substitutes()[0].account_profile.as_deref(),
            Some(first_default.profile_id.as_str())
        );
    }

    // Activation launches with the originally resolved stable ID.
    runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Activate {
                index: 0,
                reason: Some("manual".to_string()),
            },
        )
        .await
        .expect("activation should succeed");
    let app = app.lock().await;
    let agent = app.agents().get_agent(&agent_id).expect("agent exists");
    assert_eq!(agent.provider_account_profile(), first_default.profile_id);
}

#[tokio::test]
async fn substitute_add_without_any_registered_account_rejects_with_actionable_error() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    {
        // Deregister every codex account so no usable default exists.
        let app = app.lock().await;
        let registry = app.provider_account_profile_registry();
        for profile in registry
            .list(crate::session::DEFAULT_LOCAL_USER_ID, Some("codex"))
            .expect("codex accounts should list")
        {
            registry
                .remove_registration(
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    "codex",
                    &profile.profile_id,
                )
                .expect("codex account should be removable");
        }
    }

    let error = runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Add {
                provider: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                variant: None,
                account_profile: None,
                kernel_id: None,
                worktree_id: None,
            },
        )
        .await
        .expect_err("a missing default must be rejected");

    match error {
        DaemonError::LocalTransport { message, .. } => {
            assert!(
                message.contains("no usable account profile is registered")
                    && message.contains("codex"),
                "error should name the provider and be actionable: {message}"
            );
            assert!(
                !message.contains("default"),
                "error must not leak the literal sentinel: {message}"
            );
        }
        other => panic!("expected actionable default error, got {other:?}"),
    }
}

#[tokio::test]
async fn substitute_add_rejects_profile_from_mismatched_provider() {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    let opencode_profile = app
        .lock()
        .await
        .provider_account_profile_registry()
        .create_managed(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "opencode",
            "Wrong Family",
        )
        .expect("opencode profile should be created");

    let error = runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Add {
                provider: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                variant: None,
                account_profile: Some(opencode_profile.profile_id.clone()),
                kernel_id: None,
                worktree_id: None,
            },
        )
        .await
        .expect_err("cross-provider profile binding must be rejected");

    match error {
        DaemonError::LocalTransport { message, .. } => {
            assert!(
                !message.contains(&opencode_profile.profile_id),
                "error must not echo the internal profile id: {message}"
            );
            assert!(
                !message.contains("default"),
                "error must not leak the literal sentinel: {message}"
            );
            assert!(
                message.contains("choose an available account alias") && message.contains("codex"),
                "error should name the provider and be actionable: {message}"
            );
            // The public label may be shown as a hint; the id never is.
            assert!(
                message.contains("Wrong Family"),
                "error may surface the public label as a hint: {message}"
            );
        }
        other => panic!("expected mismatch rejection, got {other:?}"),
    }
}

async fn agent_config_runtime() -> (Arc<Mutex<DaemonApp>>, KernelRuntimeState, String, String) {
    agent_config_runtime_with_config(crate::config::DaemonConfig::for_tests()).await
}

async fn agent_config_runtime_with_config(
    config: crate::config::DaemonConfig,
) -> (Arc<Mutex<DaemonApp>>, KernelRuntimeState, String, String) {
    agent_config_runtime_with_config_and_owner(config, crate::session::DEFAULT_LOCAL_USER_ID).await
}

async fn agent_config_runtime_with_config_and_owner(
    config: crate::config::DaemonConfig,
    owner_user_id: &str,
) -> (Arc<Mutex<DaemonApp>>, KernelRuntimeState, String, String) {
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            crate::session::CreateSessionRequest::new("workspace-1", "worktree-1")
                .with_owner_user_id(owner_user_id),
        )
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
