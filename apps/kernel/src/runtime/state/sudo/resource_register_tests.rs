//! MP-08 / MP-09 / MP-10 / MP-11 AU-11: public MCP entry points, no secret outputs.
use super::tests::{fixture, running, Fixture, PASSKEY};
use super::*;
use serde_json::{json, Value};

const TOKEN: &str = "sudo-fixture-bearer";
const RECORD: &str = "chariox.mandate.record_resource";
const LIST: &str = "chariox.mandate.list_resources";
const REVOKE: &str = "chariox.mandate.revoke_resource";

async fn call(f: &Fixture, name: &str, args: Value) -> Result<Value, DaemonError> {
    Box::pin(
        f.router
            .dispatch_authenticated_runtime_tool_call(TOKEN, name, args),
    )
    .await
    .map(|r| r.payload)
}

fn elevate(f: &Fixture) -> KernelSudoTurn {
    let turn = running(f);
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let prompts = &f.state.owned.prompt_state_owner;
    let active = prompts
        .active_prompt_for_agent_snapshot(&session, &turn.agent_id)
        .unwrap();
    let mut started = active.clone();
    started.set_status(crate::session::PromptStatus::Running);
    assert!(prompts.replace_active_prompt_if_matches(&session, &turn.agent_id, &active, started));
    turn
}

fn resource() -> Value {
    json!({"request_id":"external-project", "service":"https://example.com", "kind":"project",
        "name":"demo", "url":"https://example.com/projects/demo",
        "revoke":{"kind":"url", "url":"https://example.com/projects/demo/settings"}})
}

#[cfg(unix)]
#[tokio::test]
async fn resource_register_command_capture_is_automatic_and_idempotent() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevate(&f);
    let path = f
        .state
        .owned
        .config_projection
        .snapshot()
        .user_config
        .credential_vault
        .path;
    crate::secret::unlock_chariox_encrypted_vault(
        path,
        PASSKEY,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let args = json!({"request_id":"register-command", "host":"api.example.com",
        "argv":["/usr/bin/printf", "register-capture-synthetic"],"use_binding":{"target":{"kind":"command_environment","name":"KEY","origin":"https://api.example.com"}}});
    let captured = call(&f, "chariox.vault.capture_command", args.clone())
        .await
        .unwrap();
    call(&f, "chariox.vault.capture_command", args)
        .await
        .unwrap();
    let listed = call(&f, LIST, json!({})).await.unwrap();
    let rows = listed["resources"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["credential_id"], captured["credential_id"]);
    assert_eq!(rows[0]["kind"], "credential");
    assert_eq!(rows[0]["service"], "api.example.com");
    assert!(!listed.to_string().contains("register-capture-synthetic"));
}

#[tokio::test]
async fn resource_register_forwarded_calls_use_home_state_and_reject_stale_context() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let mut turn = elevate(&f);
    let worker = crate::config::DaemonConfig::for_tests();
    crate::config::DaemonConfig::approve_remote_machine("worker-machine").unwrap();
    super::forgotten_machine_tests::lease_agent_to_machine(&f, &worker.daemon_id);
    turn.placement = Some("lease".into());
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .insert(turn.entry_id.clone(), turn.clone());
    let state = f.state.with_relay_peer_authority(
        crate::runtime::relay_peer_authority::test_peer_authority(&worker.daemon_id),
    );
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: f.state.owned.config_projection.snapshot().daemon_id,
        home_session_id: turn.session_id,
        home_agent_id: turn.agent_id.clone(),
        home_prompt_id: turn.prompt_id,
        leased_agent_id: "leased-agent".into(),
        worker_kernel_id: worker.daemon_id,
        worker_machine_id: "worker-machine".into(),
        worker_provider_run_id: "worker-run".into(),
        worker_worktree_path: "/work".into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("/work".to_string()),
    };
    let row = state
        .dispatch_forwarded_meta_runtime_tool_call(context.clone(), RECORD.into(), resource())
        .await
        .unwrap()
        .payload;
    assert_eq!(row["resource"]["created_by"], turn.agent_id);
    let listed = state
        .dispatch_forwarded_meta_runtime_tool_call(context.clone(), LIST.into(), json!({}))
        .await
        .unwrap()
        .payload;
    assert_eq!(listed["resources"][0], row["resource"]);
    let mut stale = context.clone();
    stale.home_prompt_id = Some("old-prompt".into());
    assert!(state
        .dispatch_forwarded_meta_runtime_tool_call(stale, LIST.into(), json!({}))
        .await
        .is_err());
    f.state.owned.sudo_turns.lock().unwrap().clear();
    state
        .dispatch_forwarded_meta_runtime_tool_call(context, LIST.into(), json!({}))
        .await
        .unwrap();
}

#[tokio::test]
async fn resource_register_record_list_retry_and_owner_url() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevate(&f);
    let row = call(&f, RECORD, resource())
        .await
        .expect("record tool must exist");
    let id = row["resource"]["id"].as_str().unwrap();
    assert_eq!(row["resource"]["created_by"], turn.agent_id);
    assert_eq!(row["resource"]["state"], "active");
    let again = call(&f, RECORD, resource()).await.unwrap();
    assert_eq!(again["resource"], row["resource"]);
    let listed = call(&f, LIST, json!({})).await.unwrap();
    assert_eq!(listed["resources"].as_array().unwrap().len(), 1);
    assert_eq!(listed["resources"][0], row["resource"]);
    let revoked = call(&f, REVOKE, json!({"resource_id":id})).await.unwrap();
    assert_eq!(revoked["status"], "owner_action_required");
    assert_eq!(revoked["url"], "https://example.com/projects/demo/settings");
    assert_eq!(
        call(&f, LIST, json!({})).await.unwrap()["resources"][0]["state"],
        "active"
    );
    let mut conflict = resource();
    conflict["name"] = json!("a different project");
    assert!(call(&f, RECORD, conflict).await.is_err());
}

#[tokio::test]
async fn resource_register_ordinary_agents_keep_project_scope() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    call(&f, RECORD, resource()).await.unwrap();
    assert_eq!(
        call(&f, LIST, json!({})).await.unwrap()["resources"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        call(&f, REVOKE, json!({"resource_id":"someone-elses-resource"}))
            .await
            .is_err()
    );
    let mut invalid = resource();
    invalid["revoke"] = json!({"kind":"url", "url":"https://user:password@example.com/"});
    assert!(call(&f, RECORD, invalid).await.is_err());
}

#[tokio::test]
async fn resource_register_generated_password_is_automatic_and_idempotent() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevate(&f);
    let path = f
        .state
        .owned
        .config_projection
        .snapshot()
        .user_config
        .credential_vault
        .path;
    crate::secret::unlock_chariox_encrypted_vault(
        path,
        PASSKEY,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let args = json!({"request_id":"register-password", "origin":"https://example.com"});
    let generated = call(&f, "chariox.vault.generate", args.clone())
        .await
        .unwrap();
    call(&f, "chariox.vault.generate", args).await.unwrap();
    let rows = call(&f, LIST, json!({}))
        .await
        .expect("list tool must exist");
    let rows = rows["resources"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["credential_id"], generated["credential_id"]);
    assert_eq!(rows[0]["kind"], "password");
    assert_eq!(rows[0]["service"], "https://example.com");
    let secret = zeroize::Zeroizing::new(
        f.state
            .home_runtime_secret_service()
            .unwrap()
            .browser_secret_input_for_target_url(
                generated["credential_id"].as_str().unwrap(),
                "https://example.com/login",
            )
            .unwrap(),
    );
    assert!(!serde_json::to_string(rows)
        .unwrap()
        .contains(secret.as_str()));
}

#[test]
fn resource_register_protocol_snapshot_and_hash() {
    use sha2::{Digest, Sha256};
    let snapshot: Value = serde_json::from_str(include_str!(
        "../../../../../../fixtures/created-resource-register-protocol.json"
    ))
    .unwrap();
    assert_eq!(
        snapshot["local"],
        crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
    );
    assert_eq!(
        snapshot["relayPeer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let row: crate::local::CreatedResource =
        serde_json::from_value(snapshot["resource"].clone()).unwrap();
    assert_eq!(serde_json::to_value(row).unwrap(), snapshot["resource"]);
    let names: Vec<_> = super::super::resource_register::specs()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(json!(names), snapshot["tools"]);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "48ec83da5c154210ef39ff3d7a1ff819034535f35cb1641ec02e2fd254cd4256"
    );
}

#[tokio::test]
async fn resource_register_http_revoke_uses_vault_and_never_echoes_response() {
    use crate::config::{UserCredentialConfig, UserCredentialMetadataConfig};
    use std::io::{Read, Write};
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevate(&f);
    let path = f
        .state
        .owned
        .config_projection
        .snapshot()
        .user_config
        .credential_vault
        .path;
    crate::secret::unlock_chariox_encrypted_vault(
        path,
        PASSKEY,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let secret = zeroize::Zeroizing::new(format!("service-key-{}", rand::random::<u128>()));
    let expected_secret = secret.clone();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut data = vec![0u8; 8192];
        let size = socket.read(&mut data).unwrap();
        let text = String::from_utf8_lossy(&data[..size]);
        let authenticated = text.to_lowercase().contains(&format!(
            "authorization: bearer {}",
            expected_secret.to_lowercase()
        ));
        let deletes = text.starts_with("DELETE /keys/demo ");
        // The service deliberately echoes the Vault value. The register returns only status.
        let body = format!("{{\"echo\":\"{}\"}}", expected_secret.as_str());
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        (authenticated, deletes)
    });
    let credential = UserCredentialConfig {
        id: "service-revoke-key".into(),
        description: None,
        source: crate::config::UserCredentialSourceConfig::Vault {
            key: "service-revoke-key".into(),
        },
        allowed_hosts: vec![origin.trim_start_matches("http://").into()],
        allowed_uses: vec![crate::config::UserCredentialUse::Http],
        injection: crate::config::UserCredentialInjectionConfig::Header {
            name: "Authorization".into(),
            value: "Bearer ${secret}".into(),
        },
        metadata: Some(UserCredentialMetadataConfig {
            payment_card: None,
            use_binding: None,
            created_by_kind: Some("captured".into()),
            created_by_id: Some(turn.agent_id.clone()),
            session_id: Some(turn.session_id.clone()),
            created_at_ms: Some(42),
            provider: None,
            provider_run_id: None,
            vault_key: Some("service-revoke-key".into()),
            updated_at_ms: Some(42),
        }),
    };
    f.state
        .upsert_observed_vault_credential(
            &f.state.home_runtime_secret_service().unwrap(),
            &crate::credential::CharioxCredentialRegistry::user().unwrap(),
            credential,
            secret.as_str(),
            false,
        )
        .await
        .unwrap();
    let mut invalid = resource();
    invalid["revoke"] = json!({"kind":"http","request":{"credential_id":"service-revoke-key",
        "url":format!("{origin}/keys/demo"),"method":"DELETE","body_text":"one","body_json":{"two":true}}});
    assert!(call(&f, RECORD, invalid).await.is_err());
    let resources = call(&f, LIST, json!({})).await.unwrap();
    let id = resources["resources"][0]["id"].as_str().unwrap();
    call(&f, "chariox.mandate.set_resource_revoke", json!({"resource_id":id,"revoke":{
        "kind":"http","request":{"credential_id":"service-revoke-key","url":format!("{origin}/keys/demo"),"method":"DELETE"}}})).await.unwrap();
    let response = call(&f, REVOKE, json!({"resource_id":id})).await.unwrap();
    assert_eq!(response["status"], "revoked");
    assert_eq!(response["http_status"], 200);
    assert!(!response.to_string().contains(secret.as_str()));
    assert_eq!(server.join().unwrap(), (true, true));
    assert_eq!(
        call(&f, REVOKE, json!({"resource_id":id})).await.unwrap()["status"],
        "revoked"
    );
    assert_eq!(
        call(&f, LIST, json!({})).await.unwrap()["resources"][0]["state"],
        "revoked"
    );
    assert!(call(
        &f,
        "chariox.mandate.set_resource_revoke",
        json!({"resource_id":id,
        "revoke":{"kind":"url","url":"https://example.com/settings"}})
    )
    .await
    .is_err());
}
