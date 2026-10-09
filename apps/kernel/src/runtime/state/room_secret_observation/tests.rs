use super::*;

mod lifecycle;

#[test]
fn a02_forwarded_task_answers_protect_recovered_room_history() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let root = TestRoot::new();
    let mut room = TestRoom::new("am2-protected-task-answer");
    room.runtime.owned.room_secret_observations =
        RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new());
    let mut prompt = crate::history::SessionHistoryEntry::user_prompt(
        &room.session_id,
        "fixture-attachment",
        &room.agent_id,
        "task",
    );
    prompt.merge_key = Some("prompt:task-prompt".into());
    let output = crate::history::SessionHistoryEntry::provider_output(
        &room.session_id,
        "run",
        Some(&room.agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        "synthetic-only",
    );
    for entry in [prompt, output] {
        room.runtime
            .owned
            .operational_history_store
            .append_transcript(&entry, Default::default())
            .unwrap();
    }
    // Recovered history must be protected even if it predates the observation fence.
    room.runtime
        .owned
        .room_secret_observations
        .register(&room.session_id, "synthetic-only")
        .unwrap();
    let store = &room.runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Begin {
            owner: "owner".into(),
            room: room.session_id.clone(),
            agent: room.agent_id.clone(),
            prompt: "task-prompt".into(),
            run: Some("run".into()),
            now: 1,
        })
        .unwrap();
    let Outcome::Settled { task, .. } = store
        .agent_lifecycle(Operation::Settle {
            room: room.session_id.clone(),
            agent: room.agent_id.clone(),
            prompt: "task-prompt".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: 2,
        })
        .unwrap()
    else {
        panic!()
    };
    let answer = room
        .runtime
        .owned
        .public_agent_task_answer(&task)
        .unwrap()
        .unwrap();
    let excerpt = answer["excerpt"].as_str().unwrap();
    assert!(!excerpt.contains("synthetic-only"));
    assert!(excerpt.contains("withheld"));
}

// MP-08/MP-10/MP-11: even non-Vault peer requests construct these shared
// futures. Keep relay delivery out of their inline state on kernel stacks.
#[tokio::test]
async fn revocation_transport_keeps_room_controller_future_bounded() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    let room = TestRoom::new("vault-retirement-stack");
    let future = room.runtime.room_browser_controller_command(
        &room.session_id,
        Command::ClearSecretObservation {
            disposition: Disposition::Retire,
        },
    );
    let bytes = std::mem::size_of_val(&future);
    assert!(
        bytes <= 32 * 1024,
        "Room controller future embeds relay transport: {bytes} bytes"
    );
    assert!(matches!(
        future.await.unwrap(),
        Response::SecretObservationCleared
    ));
}

#[tokio::test]
async fn revocation_transport_keeps_slice_admission_future_bounded() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    let room = TestRoom::new("vault-admission-stack");
    let future = room
        .runtime
        .settle_slice_observation_revocations("missing-slice");
    let bytes = std::mem::size_of_val(&future);
    assert!(
        bytes <= 4096,
        "Revocation admission embeds relay delivery: {bytes} bytes"
    );
    assert!(future.await.is_err());

    let future = room
        .runtime
        .guard_slice_execution(None, [(None, None)], "agent.spawn");
    let bytes = std::mem::size_of_val(&future);
    assert!(
        bytes <= 4096,
        "Ordinary admission embeds relay delivery: {bytes} bytes"
    );
    assert_eq!(future.await.unwrap().slice_ids, vec![None]);
}

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
async fn vault_delete_and_rotation_retire_values_without_disabling_affected_rooms() {
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
            "MP-08/MP-10/MP-11: retired Vault value leaves active provenance"
        );
        assert!(
            store.require(&room.session_id, false).is_ok(),
            "MP-08/MP-10/MP-11: rotation must remain autonomous and observable"
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
    let _environment = crate::env_lock::lock();
    let _home = lifecycle::IsolatedHome::new(root.path());
    crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .upsert(browser_credential("login", "login"))
        .unwrap();
    let mutation = room.runtime.vault_observation_mutation_guard().await;
    let input = room
        .runtime
        .room_secret_input_service(&room.session_id, "login");
    tokio::pin!(input);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut input)
            .await
            .is_err()
    );
    drop(mutation);
    let (_service, first) = input.await.unwrap();
    let (_service, second) = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        room.runtime
            .room_secret_input_service("other-room", "login"),
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
        let _home = lifecycle::IsolatedHome::new(root.path());
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

// MP-08/MP-10/MP-11: empty and revoked Rooms cannot block unrelated Vault writes.
#[test]
fn empty_or_revoked_rooms_never_match_vault_keys() {
    let root = TestRoot::new();
    let key = crate::transport::relay_crypto::generate_private_key_base64();
    let path = root.path().join("observations");
    let store = RoomSecretObservations::new(path.clone(), BTreeSet::new()).with_identity(&key);
    store
        .register_credential_source("room", Some("login"))
        .unwrap();
    assert!(!store.uses_vault_key("room", "login").unwrap());
    store.register("room", "synthetic-only").unwrap();
    store.forget("room").unwrap();
    for store in [
        &store,
        &RoomSecretObservations::new(path, BTreeSet::new()).with_identity(&key),
    ] {
        for key in ["login", "unrelated"] {
            assert!(!store.uses_vault_key("room", key).unwrap());
        }
    }
}

// MP-08/MP-10/MP-11: revoked values are scrub-only, including after restart.
#[test]
fn retired_values_scrub_prior_echoes_without_fencing_runtime_results() {
    let root = TestRoot::new();
    let key = crate::transport::relay_crypto::generate_private_key_base64();
    let path = root.path().join("observations");
    let store = RoomSecretObservations::new(path.clone(), BTreeSet::new()).with_identity(&key);
    store
        .register_credential_source("room", Some("login"))
        .unwrap();
    store.register("room", "synthetic-retired-value").unwrap();
    store.forget("room").unwrap();
    let restarted = RoomSecretObservations::new(path, BTreeSet::new()).with_identity(&key);
    for store in [&store, &restarted] {
        assert_eq!(
            store
                .scrub("room", "workflow ack committed".to_string())
                .unwrap(),
            "workflow ack committed"
        );
        assert_eq!(
            store
                .scrub("room", "prior synthetic-retired-value echo".to_string())
                .unwrap(),
            "prior [redacted] echo"
        );
        assert!(store.require("room", true).is_ok());
        let policy: serde_json::Value =
            serde_json::from_str(&store.capture_policy("room").unwrap()).unwrap();
        assert_eq!(policy["unknown"], false);
        assert!(policy["values"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("synthetic-retired-value")));
        assert!(!store.uses_vault_key("room", "login").unwrap());
        assert!(store.protects_bytes("room"));
        assert!(store.withholds_history("room", 0));
    }
    store
        .register_credential_source("room", Some("new-login"))
        .unwrap();
    store.register("room", "synthetic-new-value").unwrap();
    assert!(store.uses_vault_key("room", "new-login").unwrap());
    assert!(!store.uses_vault_key("room", "login").unwrap());
    assert_eq!(
        store
            .scrub(
                "room",
                "synthetic-retired-value synthetic-new-value".to_string()
            )
            .unwrap(),
        "[redacted] [redacted]"
    );
}

#[tokio::test]
async fn worker_observation_reset_and_delete_are_authenticated_and_autonomous() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    for disposition in ["reset_environment", "delete_room"] {
        let room = TestRoom::new("observation-reset");
        let mut config = room.runtime.owned.config_projection.snapshot();
        config.room_environment_worker_binding =
            Some(crate::config::RoomEnvironmentWorkerBinding {
                home_kernel_id: "synthetic-home".into(),
                home_public_key: config.relay_public_key.clone(),
                session_id: room.session_id.clone(),
                slice_id: "synthetic-slice".into(),
                provisioned_slice_id: None,
            });
        room.runtime.owned.config_projection.update(config.clone());
        let store = &room.runtime.owned.room_secret_observations;
        store.register(&room.session_id, "synthetic-only").unwrap();
        // Missing registries are a real lasting fence; a verified environment reset clears it.
        store
            .rooms
            .lock()
            .unwrap()
            .get_mut(&room.session_id)
            .unwrap()
            .unknown = true;
        let command: Command = serde_json::from_value(
            serde_json::json!({"kind":"clear_secret_observation", "disposition":disposition}),
        )
        .unwrap();
        assert!(room
            .runtime
            .execute_bound_room_browser_controller(
                "wrong-home",
                &config.relay_public_key,
                &room.session_id,
                "synthetic-slice",
                command.clone()
            )
            .await
            .is_err());
        room.runtime
            .execute_bound_room_browser_controller(
                "synthetic-home",
                &config.relay_public_key,
                &room.session_id,
                "synthetic-slice",
                command,
            )
            .await
            .unwrap();
        if disposition == "reset_environment" {
            assert!(
                store.require(&room.session_id, true).is_ok(),
                "MP-08/MP-10/MP-11: fresh environment is autonomously observable"
            );
            assert_eq!(
                store
                    .scrub(&room.session_id, "workflow ack".to_string())
                    .unwrap(),
                "workflow ack"
            );
            assert!(store.withholds_history(&room.session_id, 0));
        } else {
            assert!(
                !store.registry_path(&room.session_id).exists(),
                "MP-08/MP-10/MP-11: deleted worker Room retains no sealed values"
            );
            assert!(store.require(&room.session_id, true).is_err());
        }
    }
}

#[tokio::test]
async fn stopped_or_unreachable_slice_defers_revocation_and_allows_room_deletion() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    use crate::slice::{CreateSliceInput, SliceBackendKind, SliceDisplayMode, SliceStatus};
    for (stopped, destroyed) in [(true, false), (false, false), (true, true)] {
        let room = TestRoom::new("vault-offline-revocation");
        let slices = &room.runtime.owned.slice_store;
        let slice = slices
            .create(
                "home",
                "machine",
                CreateSliceInput {
                    source_slice_ref: None,
                    name: "offline".into(),
                    backend: SliceBackendKind::LocalDocker,
                    os: "linux".into(),
                    display_mode: SliceDisplayMode::Headed,
                    display_backend: Default::default(),
                    workspace_id: None,
                    worktree_id: None,
                    workspace_mount: None,
                    development: None,
                    worker_kernel_ref: Some("unreachable-fixture-worker".into()),
                    display_url: None,
                    provider_auth: vec![],
                    from_saved_state: None,
                    now_ms: 1,
                },
            )
            .unwrap();
        slices
            .bind_environment(&room.session_id, &slice.id, 2, |_| Ok(()))
            .unwrap();
        if !stopped {
            slices
                .set_status(&slice.id, SliceStatus::Running, 3)
                .unwrap();
        }
        // This lane owns the loopback sink: it never completes a worker handshake.
        let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        slices
            .set_relay_endpoint(
                &slice.id,
                Some(crate::slice::SliceRelayEndpoint {
                    url: format!("ws://{}", sink.local_addr().unwrap()),
                    private: false,
                }),
                4,
            )
            .unwrap();
        let store = &room.runtime.owned.room_secret_observations;
        store
            .register_credential_source(&room.session_id, Some("login"))
            .unwrap();
        store.register(&room.session_id, "synthetic-only").unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            room.runtime.revoke_vault_observation_values("login"),
        )
        .await
        .expect("MP-08/MP-10/MP-11: offline revocation is bounded")
        .expect("MP-08/MP-10/MP-11: offline worker cannot veto a home Vault mutation");
        assert!(!store.uses_vault_key(&room.session_id, "unrelated").unwrap());
        assert!(store
            .restore_registry(&room.session_id)
            .unwrap()
            .unwrap()
            .values
            .is_empty());
        assert_eq!(store.pending_revocations(&slice).unwrap().len(), 1);
        let config = room.runtime.owned.config_projection.snapshot();
        let restarted = RoomSecretObservations::new(store.root.clone(), BTreeSet::new())
            .with_identity(&config.relay_private_key);
        assert_eq!(restarted.pending_revocations(&slice).unwrap().len(), 1);
        assert!(!restarted.uses_vault_key(&room.session_id, "login").unwrap());
        let error = room
            .runtime
            .guard_slice_execution(
                Some(&room.session_id),
                [(Some(slice.id.as_str()), None)],
                "agent.spawn",
            )
            .await
            .err()
            .expect("pending revocation must gate the next admission");
        assert!(
            error.to_string().contains("revocation is pending"),
            "{error}"
        );
        if destroyed {
            // Destroyed slices also cannot hold the home Room hostage.
            slices.delete(&slice.id).unwrap();
        }
        room.runtime
            .delete_session_id(&room.session_id)
            .await
            .expect("MP-08/MP-10/MP-11: an offline Room remains deletable");
        assert!(!store.registry_path(&room.session_id).exists());
        let pending = store.pending_revocations(&slice).unwrap();
        if destroyed {
            room.runtime.retry_pending_slice_observation_revocations();
            assert!(store.pending_revocations(&slice).unwrap().is_empty());
        } else {
            assert!(
                pending
                    .iter()
                    .any(|record| record.disposition == Disposition::DeleteRoom),
                "MP-08/MP-10/MP-11: Room teardown retains a worker wipe obligation"
            );
        }
    }
}

#[tokio::test]
async fn room_deletion_does_not_report_failure_after_observation_cleanup_error() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    let room = TestRoom::new("vault-delete-cleanup-error");
    let store = &room.runtime.owned.room_secret_observations;
    store.require(&room.session_id, false).unwrap();
    // A synthetic I/O fault during post-commit cleanup, not a Vault secret.
    std::fs::create_dir(store.registry_path(&room.session_id)).unwrap();
    let result = room.runtime.delete_session_id(&room.session_id).await;
    assert!(room
        .runtime
        .owned
        .session_store
        .get_session(&room.session_id)
        .is_err());
    result.expect("MP-08/MP-10/MP-11: committed deletion must report success");
}

#[tokio::test]
async fn dead_native_targets_are_pruned_from_the_sealed_kernel_registry() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::{TestRoom, TestTools, install_screen_tool};
    for mode in ["ocr", "screenshot", "failed-ocr"] {
        let room = TestRoom::new("vault-native-target-pruning");
        let tools = TestTools::new("native-target-pruning");
        let exit_code = if mode == "failed-ocr" { 75 } else { 0 };
        std::fs::write(&tools.screen_tool, format!("#!/bin/sh\ncat >/dev/null\nif [ \"$1\" = protected-screenshot ]; then printf '\\211PNG\\r\\n\\032\\nfixture' > \"$2\"; fi\nprintf 'CHARIOX_OBSERVATION_PRUNED_NATIVE:[42]\\n' >&2\nprintf 'benign observation\\n'\nexit {exit_code}\n")).unwrap();
        let _environment = install_screen_tool(&tools.screen_tool);
        let mut config = room.runtime.owned.config_projection.snapshot();
        config.room_environment_worker_binding =
            Some(crate::config::RoomEnvironmentWorkerBinding {
                home_kernel_id: "synthetic-home".into(),
                home_public_key: config.relay_public_key.clone(),
                session_id: room.session_id.clone(),
                slice_id: "synthetic-slice".into(),
                provisioned_slice_id: None,
            });
        room.runtime.owned.config_projection.update(config.clone());
        let store = &room.runtime.owned.room_secret_observations;
        store.register(&room.session_id, "synthetic-only").unwrap();
        {
            let mut rooms = store.rooms.lock().unwrap();
            let protection = rooms.get_mut(&room.session_id).unwrap();
            protection.targets = vec![
                serde_json::json!({"kind":"native", "target":{"focus_window":42, "active_window":41}}),
                serde_json::json!({"kind":"native", "target":{"focus_window":43, "active_window":41}}),
                serde_json::json!({"kind":"browser", "target_id":"tab", "document_id":"doc", "node_ref":"backend:42"}),
            ];
            store
                .persist_registry(&room.session_id, protection)
                .unwrap();
        }
        if mode == "screenshot" {
            room.runtime
                .execute_bound_room_screenshot_capture(
                    "synthetic-home",
                    &config.relay_public_key,
                    &room.session_id,
                    "synthetic-slice",
                )
                .await
                .unwrap();
        } else {
            let result = room
                .runtime
                .execute_bound_room_computer_observation(
                    "synthetic-home",
                    &config.relay_public_key,
                    &room.session_id,
                    "synthetic-slice",
                    crate::transport::relay_peer::RemoteRoomComputerObservationCall::Ocr {
                        artifact_id: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.ok, mode == "ocr");
        }
        let restored = store.restore_registry(&room.session_id).unwrap().unwrap();
        assert_eq!(
            restored.targets.len(),
            2,
            "MP-08/MP-10/MP-11: dead XIDs must not survive a capture or restart"
        );
        assert_eq!(restored.values.len(), 1);
        assert_eq!(restored.targets[0]["target"]["focus_window"], 43);
        assert_eq!(restored.targets[1]["kind"], "browser");
    }
}

#[test]
fn pending_revocation_does_not_fence_a_new_slice_with_a_reused_local_id() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    let slices = crate::slice::SliceStore::default();
    let input = crate::slice::CreateSliceInput {
        source_slice_ref: None,
        name: "reused".into(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".into(),
        display_mode: crate::slice::SliceDisplayMode::Headed,
        display_backend: Default::default(),
        workspace_id: None,
        worktree_id: None,
        workspace_mount: None,
        development: None,
        worker_kernel_ref: None,
        display_url: None,
        provider_auth: vec![],
        from_saved_state: None,
        now_ms: 1,
    };
    let original = slices.create("home", "machine", input.clone()).unwrap();
    store
        .defer_revocation("deleted-room", &original, Disposition::Retire)
        .unwrap();
    // The product recovers its local sequence from retained slice records only.
    slices.delete(&original.id).unwrap();
    slices.restore_records(Vec::new());
    let recreated = slices
        .create(
            "home",
            "machine",
            crate::slice::CreateSliceInput { now_ms: 2, ..input },
        )
        .unwrap();
    assert_eq!(recreated.id, original.id);
    assert_ne!(recreated.worker_kernel_ref, original.worker_kernel_ref);
    assert!(
        store.pending_revocations(&recreated).unwrap().is_empty(),
        "MP-08/MP-10/MP-11: an old worker revocation cannot fence a fresh physical slice"
    );
}

// MP-08/MP-10/MP-11: pending storage and network delivery are isolated per slice.
fn pending_test_slice(slices: &crate::slice::SliceStore, name: &str) -> crate::slice::SliceRecord {
    slices
        .create(
            "home",
            "machine",
            crate::slice::CreateSliceInput {
                source_slice_ref: None,
                name: name.into(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".into(),
                display_mode: crate::slice::SliceDisplayMode::Headed,
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: None,
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: vec![],
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .unwrap()
}

fn find_pending_record(root: &std::path::Path) -> PathBuf {
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|ext| ext == "pending-revocation")
        {
            return path;
        }
        if entry.file_type().unwrap().is_dir() {
            let found = find_pending_record(&path);
            if found.exists() {
                return found;
            }
        }
    }
    root.join("absent")
}

#[test]
fn malformed_pending_revocation_only_blocks_its_own_slice() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    let slices = crate::slice::SliceStore::default();
    let bad = pending_test_slice(&slices, "bad");
    let good = pending_test_slice(&slices, "good");
    store
        .defer_revocation("bad-room", &bad, Disposition::Retire)
        .unwrap();
    std::fs::write(
        find_pending_record(&store.root),
        b"invalid synthetic record",
    )
    .unwrap();
    assert!(store.pending_revocations(&bad).is_err());
    assert!(
        store.pending_revocations(&good).unwrap().is_empty(),
        "MP-08/MP-10/MP-11: another slice's damaged record cannot close admission"
    );
}

#[tokio::test]
async fn unreachable_worker_does_not_hold_another_slices_delivery_lock() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    let room = TestRoom::new("per-slice-revocation-lock");
    let slices = &room.runtime.owned.slice_store;
    let slow = pending_test_slice(slices, "slow");
    let stopped = pending_test_slice(slices, "stopped");
    let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    slices
        .set_status(&slow.id, crate::slice::SliceStatus::Running, 2)
        .unwrap();
    slices
        .set_relay_endpoint(
            &slow.id,
            Some(crate::slice::SliceRelayEndpoint {
                url: format!("ws://{}", sink.local_addr().unwrap()),
                private: true,
            }),
            3,
        )
        .unwrap();
    let store = &room.runtime.owned.room_secret_observations;
    store
        .defer_revocation("slow-room", &slow, Disposition::Retire)
        .unwrap();
    store
        .defer_revocation("stopped-room", &stopped, Disposition::Retire)
        .unwrap();
    let slow_delivery = room.runtime.settle_slice_observation_revocations(&slow.id);
    let fast_delivery = async {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            room.runtime
                .settle_slice_observation_revocations(&stopped.id),
        )
        .await
        .expect("MP-08/MP-10/MP-11: stopped slice must not wait for unrelated network I/O")
        .unwrap_err();
    };
    let _ = tokio::join!(slow_delivery, fast_delivery);
}

#[test]
fn deleting_slice_garbage_collects_its_pending_revocations() {
    use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    let room = TestRoom::new("deleted-slice-revocation-gc");
    let slice = pending_test_slice(&room.runtime.owned.slice_store, "deleted");
    let store = &room.runtime.owned.room_secret_observations;
    store
        .defer_revocation("deleted-room", &slice, Disposition::Retire)
        .unwrap();
    room.runtime.delete_slice(&slice.id).unwrap();
    assert!(
        store.pending_revocations(&slice).unwrap().is_empty(),
        "MP-08/MP-10/MP-11: destroyed slices leave no undeliverable records"
    );
}

#[test]
fn missing_value_fences_observations_but_not_unrelated_runtime_results() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    store.register("room", "synthetic-known").unwrap();
    store.rooms.lock().unwrap().get_mut("room").unwrap().unknown = true;
    assert!(store.require("room", true).is_err());
    assert!(store.scrub("room", "page text".to_string()).is_err());
    assert_eq!(
        store
            .scrub_runtime_result("room", "workflow submitted synthetic-known".to_string())
            .unwrap(),
        "workflow submitted [redacted]"
    );
    assert_eq!(
        store
            .scrub_runtime_result("room", "credential rotation committed".to_string())
            .unwrap(),
        "credential rotation committed"
    );
}

// MP-08/MP-10/MP-11: valid unshipped obligations survive the index migration.
#[test]
fn predecessor_flat_revocation_migrates_to_its_physical_slice_index() {
    #[derive(Serialize)]
    struct Legacy<'a> {
        room: &'a str,
        slice: &'a str,
        worker_ref: &'a str,
        created_at_ms: u64,
    }
    let root = TestRoot::new();
    let path = root.path().join("observations");
    let slices = crate::slice::SliceStore::default();
    let slice = pending_test_slice(&slices, "legacy");
    let store = RoomSecretObservations::new(path.clone(), BTreeSet::new());
    let bytes = serde_json::to_vec(&Legacy {
        room: "legacy-room",
        slice: &slice.id,
        worker_ref: &slice.worker_kernel_ref,
        created_at_ms: slice.created_at_ms,
    })
    .unwrap();
    let flat = path.join(format!("{:x}.pending-revocation", Sha256::digest(&bytes)));
    std::fs::write(&flat, bytes).unwrap();
    drop(store);
    let restarted = RoomSecretObservations::new(path, BTreeSet::new());
    assert!(!flat.exists());
    assert_eq!(restarted.pending_revocations(&slice).unwrap().len(), 1);
}

#[test]
fn later_retirement_cannot_replace_a_deleted_rooms_pending_wipe() {
    let root = TestRoot::new();
    let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
    let slice = pending_test_slice(&crate::slice::SliceStore::default(), "deleted");
    store
        .defer_revocation("room", &slice, Disposition::Retire)
        .unwrap();
    store
        .defer_revocation("room", &slice, Disposition::DeleteRoom)
        .unwrap();
    store
        .defer_revocation("room", &slice, Disposition::Retire)
        .unwrap();
    let pending = store.pending_revocations(&slice).unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].disposition, Disposition::Retire);
    assert_eq!(pending[1].disposition, Disposition::DeleteRoom);
}

// MP-08/MP-10/MP-11: reset/wipe must repair an unreadable registry without reading values.
#[test]
fn fresh_environment_reset_and_room_delete_recover_a_corrupt_sealed_registry() {
    for delete in [false, true] {
        let root = TestRoot::new();
        let key = crate::transport::relay_crypto::generate_private_key_base64();
        let path = root.path().join("observations");
        let store = RoomSecretObservations::new(path.clone(), BTreeSet::new()).with_identity(&key);
        store.register("room", "synthetic-only").unwrap();
        std::fs::write(
            store.registry_path("room"),
            b"synthetic corrupt sealed registry",
        )
        .unwrap();
        let restarted = RoomSecretObservations::new(path, BTreeSet::new()).with_identity(&key);
        assert!(restarted.require("room", true).is_err());
        if delete {
            restarted.delete_room("room").expect(
                "MP-08/MP-10/MP-11: deletion wipes corrupt registries without restoring them",
            );
            assert!(!restarted.registry_path("room").exists());
        } else {
            restarted.reset_environment("room").expect(
                "MP-08/MP-10/MP-11: proven fresh environment repairs unavailable protection",
            );
            assert!(restarted.require("room", true).is_ok());
            assert!(restarted.withholds_history("room", 0));
            assert!(restarted
                .restore_registry("room")
                .unwrap()
                .unwrap()
                .values
                .is_empty());
        }
    }
}
