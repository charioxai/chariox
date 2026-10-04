// MP-08/MP-10/MP-11: real home/worker relay and controller routing with a
// synthetic CDP field. No official provider or durable account is involved.
use super::controller_worker_mcp::ScopedEnvironment;
use super::*;

pub(super) async fn check(
    fixture: &LiveWorker,
    caller: &crate::runtime::state::KernelRuntimeState,
    token: &str,
) {
    let credential_home = fixture.home_state.root.join("room-secret-home");
    let _environment = ScopedEnvironment::set([
        ("CHARIOX_HOME", credential_home.as_os_str().to_os_string()),
        ("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1".into()),
    ]);
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        Vec::new(),
        &fixture.home_state.config.user_config.credential_vault,
    )
    .unwrap();
    service
        .upsert_vault_backed_credential_with_secret(
            &registry,
            crate::config::UserCredentialConfig {
                id: "home-room-login".into(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault {
                    key: "room-regression".into(),
                },
                allowed_hosts: vec!["worker.test".into()],
                allowed_uses: vec![crate::config::UserCredentialUse::Browser],
                injection: crate::config::UserCredentialInjectionConfig::Browser,
                metadata: None,
            },
            "home-room-vault-regression-value",
            false,
        )
        .expect("seed the isolated home Vault");
    let mask = fixture._worker_state.root.join("secret-input-mode");
    std::fs::write(&mask, "masked fixture").unwrap();
    let status = caller
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("discover the Room's masked field");
    let field = status.payload["browser"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["label"] == "Worker note")
        .unwrap()["field_id"]
        .clone();
    // A nonexistent credential makes ordering observable: host mismatch must
    // win before any Vault lookup, not pass because no credential was found.
    let error = caller
        .dispatch_authenticated_runtime_tool_call(
            token,
            "paste_secret_to_slice",
            json!({
                "credential_id":"missing-credential", "field_id":field,
                "expected_host":"wrong-host.test"
            }),
        )
        .await
        .expect_err("reject host mismatch before resolving the credential");
    assert!(
        error.to_string().contains("does not match expected host"),
        "{error}"
    );
    let state_file = fixture._worker_state.root.join("chromium-state.json");
    let before: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert!(before["secretInputCount"].is_null());
    let pasted = caller
        .dispatch_authenticated_runtime_tool_call(
            token,
            "mcp__chariox__paste_secret_to_slice",
            json!({
                "credential_id":"home-room-login", "field_id":field,
                "expected_host":"worker.test"
            }),
        )
        .await
        .expect("insert through the home Room's Vault and bound slice");
    assert!(pasted.ok);
    assert!(!pasted
        .payload
        .to_string()
        .contains("home-room-vault-regression-value"));
    let after: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(after["secretInputMatches"], true);
    assert_eq!(after["secretInputCount"], 1);
    std::fs::remove_file(mask).unwrap();
    service.delete_vault_secret("room-regression").unwrap();
}

// MP-08/MP-10/MP-11: lifecycle revocation reaches both authenticated registry owners.
pub(super) async fn check_revocation(
    fixture: &LiveWorker,
    stopped: bool,
    caller: &crate::runtime::state::KernelRuntimeState,
    token: &str,
) {
    use sha2::{Digest, Sha256};
    let room = &fixture.rooms[0];
    let marker = format!("{:x}", Sha256::digest(room.as_bytes()));
    for state in [&fixture.home_state, &fixture._worker_state] {
        let path = state
            .config
            .private_runtime_state_root()
            .join("room-observation-quarantine")
            .join(&marker)
            .with_extension("sealed");
        assert!(
            path.exists(),
            "synthetic Room value was sealed before revocation"
        );
    }
    let slices = fixture.home.app.lock().await.slices().clone();
    let slice = slices.resolve("desktop").unwrap();
    let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    if stopped {
        slices
            .set_status(&slice.id, SliceStatus::Stopped, 2)
            .unwrap();
    } else {
        slices
            .set_relay_endpoint(
                &slice.id,
                Some(crate::slice::SliceRelayEndpoint {
                    url: format!("ws://{}", sink.local_addr().unwrap()),
                    private: false,
                }),
                2,
            )
            .unwrap();
    }
    fixture
        .home
        .runtime_state
        .revoke_vault_observation_values("room-regression")
        .await
        .unwrap();
    // Worker values remain until its authenticated receipt is delivered.
    let path = fixture
        ._worker_state
        .config
        .private_runtime_state_root()
        .join("room-observation-quarantine")
        .join(&marker)
        .with_extension("sealed");
    let sealed = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let plaintext = crate::transport::relay_crypto::decrypt_payload_for_private_key_bound(
        &fixture._worker_state.config.relay_private_key,
        &sealed,
        b"room-vault-observation-registry-v1",
        room.as_bytes(),
    )
    .unwrap();
    let registry: Value = serde_json::from_slice(&plaintext.plaintext).unwrap();
    assert!(!registry["values"].as_array().unwrap().is_empty());
    let refused = fixture
        .home
        .runtime_state
        .guard_slice_execution(Some(room), [(Some("desktop"), None)], "agent.spawn")
        .await
        .err()
        .unwrap();
    assert!(
        refused.to_string().contains("revocation is pending"),
        "{refused}"
    );
    slices
        .set_relay_endpoint(&slice.id, slice.relay_endpoint, 3)
        .unwrap();
    slices
        .set_status(&slice.id, SliceStatus::Running, 3)
        .unwrap();
    // The reconnect/presence callback settles obligations before any admission.
    fixture
        .home
        .runtime_state
        .retry_pending_slice_observation_revocations();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let sealed = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let plaintext = crate::transport::relay_crypto::decrypt_payload_for_private_key_bound(
                &fixture._worker_state.config.relay_private_key,
                &sealed,
                b"room-vault-observation-registry-v1",
                room.as_bytes(),
            )
            .unwrap();
            let registry: Value = serde_json::from_slice(&plaintext.plaintext).unwrap();
            if registry["values"].as_array().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("MP-08/MP-10/MP-11: reconnect retries retirement independently of Room admission");
    let admission = fixture
        .home
        .runtime_state
        .guard_slice_execution(Some(room), [(Some("desktop"), None)], "agent.spawn")
        .await
        .expect("MP-08/MP-10/MP-11: worker receipt is settled before readmission");
    drop(admission);
    for state in [&fixture.home_state, &fixture._worker_state] {
        let path = state
            .config
            .private_runtime_state_root()
            .join("room-observation-quarantine")
            .join(&marker)
            .with_extension("sealed");
        let sealed = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let plaintext = crate::transport::relay_crypto::decrypt_payload_for_private_key_bound(
            &state.config.relay_private_key,
            &sealed,
            b"room-vault-observation-registry-v1",
            room.as_bytes(),
        )
        .unwrap();
        let registry: Value = serde_json::from_slice(&plaintext.plaintext).unwrap();
        assert!(
            registry["values"].as_array().unwrap().is_empty(),
            "MP-08/MP-10/MP-11: retired values leave active provenance on both home and worker"
        );
        assert!(
            !registry["retired_values"].as_array().unwrap().is_empty(),
            "MP-08/MP-10/MP-11: old echoes remain scrubbed on both home and worker"
        );
        assert!(
            !registry["unknown"].as_bool().unwrap(),
            "MP-08/MP-10/MP-11: retired values preserve autonomous observation"
        );
    }
    let status = caller
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("MP-08/MP-10/MP-11: worker observations continue autonomously after retirement");
    assert!(status.ok);
    assert!(!status
        .payload
        .to_string()
        .contains("home-room-vault-regression-value"));
    if stopped {
        // MP-08/MP-10/MP-11: deletion while stopped must wipe the orphaned worker on start,
        // independently of admission to the now-nonexistent home Room.
        slices
            .set_status(&slice.id, SliceStatus::Stopped, 4)
            .unwrap();
        fixture
            .home
            .runtime_state
            .delete_session_id(room)
            .await
            .unwrap();
        fixture
            .home
            .runtime_state
            .mark_slice_running(&slice.id, None)
            .unwrap();
        let worker_registry = fixture
            ._worker_state
            .config
            .private_runtime_state_root()
            .join("room-observation-quarantine")
            .join(&marker)
            .with_extension("sealed");
        tokio::time::timeout(Duration::from_secs(5), async {
            while worker_registry.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("MP-08/MP-10/MP-11: stopped deleted-Room obligation is delivered on slice start");
    }
}
