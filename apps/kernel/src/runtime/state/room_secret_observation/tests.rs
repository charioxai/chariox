use super::*;

struct TestRoot(PathBuf);
impl TestRoot {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "chariox-vaultredact-{}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn room_capture_fence_settles_input_without_blocking_another_room() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new());
    let capture = store.barrier("room").unwrap().read_owned().await;
    let pending = store.barrier("room").unwrap().write_owned();
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut pending)
            .await
            .is_err()
    );
    assert!(store
        .barrier("other-room")
        .unwrap()
        .try_write_owned()
        .is_ok());
    drop(capture);
    let input = pending.await;
    store.register("room", "synthetic-only").unwrap();
    assert!(store.barrier("room").unwrap().try_read_owned().is_err());
    drop(input);
    let _after_input = store.barrier("room").unwrap().read_owned().await;
    assert!(store.require("room", true).is_ok());
}

#[tokio::test]
async fn observations_never_create_human_clearance_even_for_false_positives() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    let root = TestRoot::new();
    let mut room = TestRoom::new("vault-autonomous-observation");
    room.runtime.owned.room_secret_observations =
        RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new());
    room.runtime
        .owned
        .room_secret_observations
        .register(&room.session_id, "synthetic-only")
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        room.runtime
            .ensure_room_observation_ready(&room.session_id, &room.agent_id, true),
    )
    .await;
    assert!(
        result.is_ok(),
        "MP-08/MP-10/MP-11: observation must never wait for a human"
    );
    assert!(room
        .runtime
        .owned
        .session_store
        .get_session(&room.session_id)
        .unwrap()
        .active_interactions()
        .is_empty());
    assert_eq!(
        room.runtime
            .owned
            .room_secret_observations
            .scrub_text_or_withhold(&room.session_id, "benign field: synthetic-onl"),
        "benign field: synthetic-onl"
    );
    assert_eq!(
        room.runtime
            .owned
            .room_secret_observations
            .scrub_text_or_withhold(&room.session_id, "synthetic-only"),
        "[redacted]"
    );
}

#[test]
fn sealed_registry_recovers_scrubbing_and_masks_without_human_clearance() {
    let root = TestRoot::new();
    let path = root.path().join("observations");
    let key = crate::transport::relay_crypto::generate_private_key_base64();
    let store = RoomSecretObservations::new(path.clone(), BTreeSet::new()).with_identity(&key);
    let command = Command::Action {
        execution_id: "input".into(),
        target_id: "target".into(),
        document_id: "document".into(),
        node_ref: "backend:42".into(),
        action: crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
            text: "synthetic-only".into(),
            append: false,
            submit: false,
            expected_document_url: Some("https://fixture.test".into()),
        },
        timeout_ms: 1000,
    };
    store.register_command("room", &command).unwrap();
    let bytes = std::fs::read(store.registry_path("room")).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-only"));
    let recovered =
        RoomSecretObservations::new(path, BTreeSet::from(["room".into()])).with_identity(&key);
    assert_eq!(
        recovered.scrub_text_or_withhold("room", "synthetic-only and benign"),
        "[redacted] and benign"
    );
    let policy: serde_json::Value =
        serde_json::from_str(&recovered.capture_policy("room").unwrap()).unwrap();
    assert_eq!(policy["unknown"], false);
    assert_eq!(policy["targets"][0]["node_ref"], "backend:42");
    assert!(recovered
        .scrub_cached_result("room", "old observation".to_string())
        .is_err());
    assert!(recovered.require("room", true).is_ok());
}

#[test]
fn registry_cannot_be_replayed_in_another_room_or_runtime_identity() {
    let root = TestRoot::new();
    let key = crate::transport::relay_crypto::generate_private_key_base64();
    let store =
        RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new()).with_identity(&key);
    store.register("room", "synthetic-only").unwrap();
    std::fs::copy(store.marker("room"), store.marker("other")).unwrap();
    std::fs::copy(store.registry_path("room"), store.registry_path("other")).unwrap();
    assert!(store.require("other", false).is_err());
    let other_key = crate::transport::relay_crypto::generate_private_key_base64();
    let other = RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new())
        .with_identity(&other_key);
    assert!(other.require("room", false).is_err());
}

#[test]
fn worker_local_transcripts_share_home_room_registry_and_recovery_fence() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new())
        .with_worker_room(Some("home-room".into()));
    store.register("home-room", "synthetic-only").unwrap();
    assert_eq!(
        store.scrub_text_or_withhold("worker-run-session", "synthetic-only"),
        "[redacted]"
    );
    assert!(store.require("worker-run-session", true).is_ok());
    assert!(Arc::ptr_eq(
        &store.barrier("home-room").unwrap(),
        &store.barrier("worker-run-session").unwrap()
    ));

    assert!(store.require("worker-run-session", true).is_ok());
    assert_eq!(
        store
            .scrub_cached_result("worker-run-session", "synthetic-only".to_string())
            .unwrap(),
        "[redacted]"
    );
    let restarted = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new())
        .with_worker_room(Some("home-room".into()));
    assert!(restarted.require("worker-run-session", false).is_err());

    assert!(restarted
        .scrub_cached_result("worker-run-session", "old data".to_string())
        .is_err());
}

#[test]
fn split_terminal_chunks_are_withheld_without_human_clearance() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    assert_eq!(store.protect_unframed_bytes("room", b"normal"), b"normal");
    store.register("room", "synthetic-only").unwrap();

    for bytes in [b"synthetic".as_slice(), b"-only".as_slice()] {
        let protected = store.protect_unframed_bytes("room", bytes);
        assert_eq!(protected, b"[sensitive Room terminal stream withheld]");
        let entry = crate::history::SessionHistoryEntry::provider_output(
            "room",
            "run",
            None,
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            String::from_utf8_lossy(bytes),
        );
        assert_eq!(
            store.protect_transcript_entry(entry).text,
            "[sensitive Room terminal stream withheld]"
        );
    }
}

#[test]
fn vault_echo_scrubs_nested_tool_text_and_simple_transforms_and_room_isolation() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    let secret = "vault-page-copy-regression-value";
    store.register("room", secret).unwrap();
    let variants = secret_variants(secret);
    let input = serde_json::json!({"accessibility": variants.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "dom": {secret: secret}, "ocr": secret});
    let scrubbed = store.scrub("room", input.clone()).unwrap();
    assert!(!scrubbed.to_string().contains(secret));
    assert!(scrubbed["accessibility"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s == "[redacted]"));
    assert_eq!(store.scrub("other-room", input.clone()).unwrap(), input);
    assert!(store.require("room", true).is_ok());

    assert!(store.require("room", true).is_ok());
    assert!(!store
        .scrub("room", input)
        .unwrap()
        .to_string()
        .contains(secret));
    store.register("room", "another-input").unwrap();
    assert!(store.require("room", true).is_ok());
}

#[test]
fn clean_rooms_remain_observable_after_restart_and_legacy_migrates_once() {
    let root = TestRoot::new();
    let path = root.path().join("observations");
    let store = RoomSecretObservations::new(path.clone(), BTreeSet::from(["legacy".into()]));
    assert!(store.require("legacy", false).is_ok());
    assert!(store.require("new-room", false).is_ok());
    let restarted =
        RoomSecretObservations::new(path, BTreeSet::from(["legacy".into(), "new-room".into()]));
    assert!(restarted.require("legacy", false).is_ok());
    assert!(restarted.require("new-room", false).is_ok());
    assert!(!restarted.protects_bytes("new-room"));
    assert_eq!(
        restarted
            .scrub("new-room", "benign tool result".to_string())
            .unwrap(),
        "benign tool result"
    );
    assert_eq!(
        restarted
            .scrub_cached_result("new-room", "benign cached result".to_string())
            .unwrap(),
        "benign cached result"
    );
}

#[tokio::test]
async fn room_deletion_removes_sealed_values() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    let root = TestRoot::new();
    let room = TestRoom::new("vault-delete-observations");
    let store = &room.runtime.owned.room_secret_observations;
    store.register(&room.session_id, "synthetic-only").unwrap();
    let path = store.registry_path(&room.session_id);
    assert!(path.exists());
    room.runtime
        .delete_session_ref(&room.session_id, None)
        .await
        .unwrap();
    assert!(
        !path.exists(),
        "MP-08/MP-10/MP-11: Room deletion removes sealed values"
    );
    drop(root);
}

#[tokio::test]
async fn legacy_room_migration_is_autonomous_and_permanently_withholds_only_prior_history() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    let root = TestRoot::new();
    let mut room = TestRoom::new("vault-legacy-autonomous-migration");
    let config = room.runtime.owned.config_projection.snapshot();
    let path = root.path().join("observations");
    room.runtime.owned.room_secret_observations =
        RoomSecretObservations::new(path.clone(), BTreeSet::from([room.session_id.clone()]))
            .with_identity(&config.relay_private_key);
    let store = &room.runtime.owned.room_secret_observations;
    let cutoff = store.epoch;
    for pixels in [false, true] {
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            room.runtime
                .ensure_room_observation_ready(&room.session_id, &room.agent_id, pixels),
        )
        .await
        .expect("MP-08/MP-10/MP-11: migration must never wait for a human")
        .expect("MP-08/MP-10/MP-11: fresh legacy-Room observations flow autonomously");
    }
    assert!(room
        .runtime
        .owned
        .session_store
        .get_session(&room.session_id)
        .unwrap()
        .active_interactions()
        .is_empty());
    let mut restarted =
        RoomSecretObservations::new(path, BTreeSet::from([room.session_id.clone()]))
            .with_identity(&config.relay_private_key);
    // Simulate a later boot: the historical boundary must not advance on restart.
    restarted.epoch = cutoff + 60_000;
    for store in [store, &restarted] {
        assert_eq!(
            store
                .scrub(&room.session_id, "fresh result".to_string())
                .unwrap(),
            "fresh result"
        );
        assert_eq!(
            store.protect_unframed_bytes(&room.session_id, b"fresh terminal"),
            b"fresh terminal"
        );
        let policy: serde_json::Value =
            serde_json::from_str(&store.capture_policy(&room.session_id).unwrap()).unwrap();
        assert_eq!(policy["unknown"], false);
        let entry = crate::history::SessionHistoryEntry::provider_output(
            &room.session_id,
            "run",
            None,
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            "observation",
        );
        for (timestamp_ms, expected) in [
            (0, "[recovered sensitive Room history withheld]"),
            (cutoff, "[recovered sensitive Room history withheld]"),
            (cutoff + 1, "observation"),
        ] {
            let mut transcript = entry.clone();
            transcript.timestamp_ms = timestamp_ms;
            assert_eq!(
                store.protect_transcript_entry(transcript.clone()).text,
                expected
            );
            let mut event =
                crate::history::HistoryEvent::transcript(1, &transcript, Default::default());
            event.timestamp_ms = timestamp_ms;
            event.content_ref = Some("old-image".into());
            event
                .metadata
                .insert("observation".into(), "prior content".into());
            let protected = store.protect_history_events(vec![event]);
            assert_eq!(protected[0].content.as_deref(), Some(expected));
            if timestamp_ms <= cutoff {
                assert!(protected[0].content_ref.is_none());
                assert!(protected[0].metadata.is_empty());
            }
        }
        assert!(store
            .scrub_cached_result(&room.session_id, "old result".to_string())
            .is_err());
    }
    restarted
        .register(&room.session_id, "synthetic-new-value")
        .unwrap();
    assert_eq!(
        restarted.scrub_text_or_withhold(&room.session_id, "fresh synthetic-new-value"),
        "fresh [redacted]"
    );
}

#[tokio::test]
async fn vault_delete_and_rotation_remove_sealed_values_only_in_affected_rooms() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    use crate::local::{LocalDaemonRequest, SetCredentialSecretRequest, DeleteCredentialSecretRequest};
    use crate::runtime::command::KernelCommand;
    for action in ["delete", "rotate", "alias"] {
        let root = TestRoot::new();
        let mut room = TestRoom::new("vault-value-lifecycle");
        let mut config = room.runtime.owned.config_projection.snapshot();
        config.user_config.credential_vault.path = root.path().join("vault").display().to_string();
        crate::secret::create_chariox_encrypted_vault_for_test(
            std::path::Path::new(&config.user_config.credential_vault.path),
            "synthetic-passphrase-only",
        )
        .unwrap();
        crate::secret::unlock_chariox_encrypted_vault(
            std::path::Path::new(&config.user_config.credential_vault.path),
            "synthetic-passphrase-only",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        let service = crate::secret::RuntimeSecretService::with_vault_config(
            vec![],
            &config.user_config.credential_vault,
        )
        .unwrap();
        service
            .set_vault_secret("login", "synthetic-old-value")
            .unwrap();
        room.runtime.owned.config_projection.update(config.clone());
        room.runtime.owned.room_secret_observations =
            RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new())
                .with_identity(&config.relay_private_key);
        let store = &room.runtime.owned.room_secret_observations;
        store
            .register_credential_source(&room.session_id, Some("login"))
            .unwrap();
        store
            .register(&room.session_id, "synthetic-old-value")
            .unwrap();
        store
            .register_credential_source("unrelated", Some("other-key"))
            .unwrap();
        store
            .register("unrelated", "synthetic-unrelated-value")
            .unwrap();
        let request = if action == "rotate" {
            LocalDaemonRequest::SetCredentialSecret(SetCredentialSecretRequest {
                session_id: Some(room.session_id.clone()),
                agent_id: Some(room.agent_id.clone()),
                key: "login".into(),
                value: "synthetic-new-value".into(),
            })
        } else {
            LocalDaemonRequest::DeleteCredentialSecret(DeleteCredentialSecretRequest {
                session_id: Some(room.session_id.clone()),
                agent_id: Some(room.agent_id.clone()),
                key: "login".into(),
            })
        };
        if action == "alias" {
            let registry =
                crate::credential::CharioxCredentialRegistry::new(root.path().join("handles"));
            room.runtime
                .upsert_observed_vault_credential(
                    &service,
                    &registry,
                    browser_credential("another-handle", "login"),
                    "synthetic-new-value",
                    false,
                )
                .await
                .unwrap();
        } else {
            let command =
                KernelCommand::from_local_request("vault-lifecycle", None, None, &request);
            crate::runtime::user_config_executor::execute_user_config_request(
                &room.runtime.owned.config_projection,
                &room.runtime,
                &command,
                request,
            )
            .await
            .unwrap();
        }
        assert!(
            store
                .restore_registry(&room.session_id)
                .unwrap()
                .unwrap()
                .values
                .is_empty(),
            "MP-08/MP-10/MP-11: retired Vault value is not retained on disk"
        );
        assert!(
            store.require(&room.session_id, false).is_err(),
            "MP-08/MP-10/MP-11: revoked page echoes remain protected without a human fallback"
        );
        assert!(store.require("unrelated", false).is_ok());
        assert!(!store
            .restore_registry("unrelated")
            .unwrap()
            .unwrap()
            .values
            .is_empty());
        crate::secret::lock_chariox_encrypted_vault(&config.user_config.credential_vault.path)
            .unwrap();
    }
}

#[test]
fn fresh_capture_never_reauthorizes_pre_upgrade_history() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(
        root.path().join("observations"),
        BTreeSet::from(["old-room".into()]),
    );

    let entry = crate::history::SessionHistoryEntry::provider_output(
        "old-room",
        "run",
        Some("agent"),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        "synthetic old observation",
    );
    let mut event = crate::history::HistoryEvent::transcript(1, &entry, Default::default());
    event.timestamp_ms = 0;
    event.content_ref = Some("old-image".into());
    event
        .metadata
        .insert("observation".into(), "synthetic old observation".into());
    let protected = store.protect_history_events(vec![event]);
    assert_eq!(
        protected[0].content.as_deref(),
        Some("[recovered sensitive Room history withheld]")
    );
    assert!(protected[0].metadata.is_empty());
    assert!(protected[0].content_ref.is_none());
}

#[test]
fn marker_io_failure_aborts_input_and_withholds_capture() {
    let root = TestRoot::new();
    let path = root.path().join("file");
    std::fs::write(&path, b"not a directory").unwrap();
    let store = RoomSecretObservations::new(path, BTreeSet::new());
    assert!(store.register("room", "synthetic-only").is_err());
    assert!(store.require("room", true).is_err());
}

#[tokio::test]
async fn vault_mutation_drains_inputs_and_blocks_new_resolutions_across_rooms() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    let root = TestRoot::new();
    let mut room = TestRoom::new("vault-lifecycle-fence");
    room.runtime.owned.room_secret_observations =
        RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    let credential = crate::config::UserCredentialConfig {
        id: "login".into(),
        description: None,
        source: crate::config::UserCredentialSourceConfig::Vault {
            key: "login".into(),
        },
        allowed_hosts: vec!["fixture.test".into()],
        allowed_uses: vec![crate::config::UserCredentialUse::Browser],
        injection: crate::config::UserCredentialInjectionConfig::Browser,
        metadata: None,
    };
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        vec![credential],
        &crate::config::UserCredentialVaultConfig::default(),
    )
    .unwrap();
    let mutation = room.runtime.vault_observation_mutation_guard().await;
    let input = room
        .runtime
        .track_room_vault_key(&room.session_id, &service, "login");
    tokio::pin!(input);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut input)
            .await
            .is_err()
    );
    drop(mutation);
    let first = input.await.unwrap();
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        room.runtime
            .track_room_vault_key("other-room", &service, "login"),
    )
    .await
    .unwrap()
    .unwrap();
    let mutation = room.runtime.vault_observation_mutation_guard();
    tokio::pin!(mutation);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut mutation)
            .await
            .is_err()
    );
    drop(first);
    drop(second);
    let _mutation = mutation.await;
}

#[test]
fn revocation_io_failure_keeps_memory_protected() {
    let root = TestRoot::new();
    let key = crate::transport::relay_crypto::generate_private_key_base64();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new())
        .with_identity(&key);
    store.register("room", "synthetic-only").unwrap();
    let sealed = store.registry_path("room");
    std::fs::remove_file(&sealed).unwrap();
    std::fs::create_dir(&sealed).unwrap();
    assert!(store.forget("room").is_err());
    assert!(store.require("room", false).is_err());
    assert!(store.protects_bytes("room"));
}

#[test]
fn non_vault_source_does_not_revoke_on_unrelated_vault_mutation() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    store.register_credential_source("file-room", None).unwrap();
    store.register("file-room", "synthetic-file-value").unwrap();
    assert!(!store.uses_vault_key("file-room", "unrelated-key").unwrap());
}

fn browser_credential(id: &str, key: &str) -> crate::config::UserCredentialConfig {
    crate::config::UserCredentialConfig {
        id: id.into(),
        description: None,
        source: crate::config::UserCredentialSourceConfig::Vault { key: key.into() },
        allowed_hosts: vec!["fixture.test".into()],
        allowed_uses: vec![crate::config::UserCredentialUse::Browser],
        injection: crate::config::UserCredentialInjectionConfig::Browser,
        metadata: None,
    }
}

#[test]
fn vault_key_provenance_matches_normalized_store_keys() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    store
        .register_credential_source("room", Some(" login "))
        .unwrap();
    store.register("room", "synthetic-only").unwrap();
    assert!(store.uses_vault_key("room", "login").unwrap());
    assert!(store.uses_vault_key("room", " login ").unwrap());
    assert!(!store.uses_vault_key("room", " ").unwrap());
}

#[tokio::test]
async fn credential_removal_and_metadata_replacement_revoke_sealed_values() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    use crate::local::{LocalDaemonRequest, RemoveCredentialRequest, UpsertCredentialRequest, RegisterCredentialRequest};
    let _environment = crate::env_lock::lock();
    for action in ["remove", "upsert", "register"] {
        let root = TestRoot::new();
        let mut room = TestRoom::new("vault-credential-metadata");
        let config = room.runtime.owned.config_projection.snapshot();
        room.runtime.owned.room_secret_observations =
            RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new())
                .with_identity(&config.relay_private_key);
        struct RestoreHome(Option<std::ffi::OsString>);
        impl Drop for RestoreHome {
            fn drop(&mut self) {
                if let Some(value) = &self.0 {
                    std::env::set_var("CHARIOX_HOME", value);
                } else {
                    std::env::remove_var("CHARIOX_HOME");
                }
            }
        }
        let _home = RestoreHome(std::env::var_os("CHARIOX_HOME"));
        std::env::set_var("CHARIOX_HOME", root.path().join("home"));
        let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
        let store = &room.runtime.owned.room_secret_observations;
        registry
            .upsert(browser_credential("login", "old-key"))
            .unwrap();
        store
            .register_credential_source(&room.session_id, Some("old-key"))
            .unwrap();
        store
            .register(&room.session_id, "synthetic-old-value")
            .unwrap();
        let replacement = browser_credential("login", "new-key");
        let request = match action {
            "remove" => {
                LocalDaemonRequest::RemoveCredential(RemoveCredentialRequest { id: "login".into() })
            }
            "upsert" => LocalDaemonRequest::UpsertCredential(UpsertCredentialRequest {
                credential: replacement,
            }),
            _ => {
                let path = root.path().join("replacement.yaml");
                std::fs::write(&path, serde_yaml::to_string(&replacement).unwrap()).unwrap();
                LocalDaemonRequest::RegisterCredential(RegisterCredentialRequest {
                    source_path: path,
                })
            }
        };
        crate::runtime::capability_registry::execute_capability_registry_request(
            &room.runtime,
            request,
            config.user_config.credential_vault.clone(),
        )
        .await
        .unwrap();
        assert!(
            store
                .restore_registry(&room.session_id)
                .unwrap()
                .unwrap()
                .values
                .is_empty(),
            "MP-08/MP-10/MP-11: credential metadata mutation must retire sealed values"
        );
    }
}

#[test]
fn legacy_provenance_stays_conservative_after_new_credential_input() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    // Registries from the predecessor can contain values without Vault-key provenance.
    store.register("room", "synthetic-legacy-value").unwrap();
    store.register_credential_source("room", None).unwrap();
    assert!(
        store.uses_vault_key("room", "prior-vault-key").unwrap(),
        "MP-08/MP-10/MP-11: new input cannot narrow legacy revocation scope"
    );
    let migrated = RoomSecretObservations::new(
        root.path().join("migrated"),
        BTreeSet::from(["room".into()]),
    );
    assert!(
        !migrated.uses_vault_key("room", "prior-vault-key").unwrap(),
        "MP-08/MP-10/MP-11: historical migration alone must not revoke fresh observation state"
    );
}

#[test]
fn migrated_history_marker_cannot_reopen_a_lost_secret_registry() {
    let root = TestRoot::new();
    let path = root.path().join("observations");
    let key = crate::transport::relay_crypto::generate_private_key_base64();
    let store = RoomSecretObservations::new(path.clone(), BTreeSet::from(["room".into()]))
        .with_identity(&key);
    store.register("room", "synthetic-only").unwrap();
    std::fs::remove_file(store.registry_path("room")).unwrap();
    let restarted =
        RoomSecretObservations::new(path, BTreeSet::from(["room".into()])).with_identity(&key);
    assert!(restarted.require("room", false).is_err());
    assert!(restarted.protects_bytes("room"));
}

#[test]
fn interrupted_migration_preserves_cutoff_and_room_deletion_removes_it() {
    let root = TestRoot::new();
    let path = root.path().join("observations");
    let store = RoomSecretObservations::new(path.clone(), BTreeSet::from(["room".into()]));
    let cutoff = store.legacy_history_cutoff("room").unwrap().unwrap();
    std::fs::remove_file(path.join("migration-v1")).unwrap();
    let restarted = RoomSecretObservations::new(path, BTreeSet::from(["room".into()]));
    assert_eq!(
        restarted.legacy_history_cutoff("room").unwrap(),
        Some(cutoff)
    );
    assert!(restarted.require("room", false).is_ok());
    restarted.delete_room("room").unwrap();
    assert!(restarted.legacy_history_cutoff("room").unwrap().is_none());
    assert!(!restarted.marker("room").exists());
    assert!(!restarted.registry_path("room").exists());
}
