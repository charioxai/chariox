//! MP-08/MP-10/MP-11: metadata races and public corrupt-registry recovery.
use super::*;
use super::super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;

pub(super) struct IsolatedHome(Option<std::ffi::OsString>);
impl IsolatedHome {
    pub(super) fn new(root: &std::path::Path) -> Self {
        let previous = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", root.join("home"));
        Self(previous)
    }
}
impl Drop for IsolatedHome {
    fn drop(&mut self) {
        match &self.0 {
            Some(previous) => std::env::set_var("CHARIOX_HOME", previous),
            None => std::env::remove_var("CHARIOX_HOME"),
        }
    }
}

#[tokio::test]
async fn waiting_secret_input_reloads_removed_handle() {
    waiting_secret_input_reloads_metadata("remove").await;
}
#[tokio::test]
async fn waiting_browser_secret_input_reloads_host_policy() {
    waiting_secret_input_reloads_metadata("host").await;
}
#[tokio::test]
async fn waiting_secret_input_reloads_source_and_provenance() {
    waiting_secret_input_reloads_metadata("source").await;
}
#[tokio::test]
async fn waiting_computer_secret_input_reloads_use_policy() {
    waiting_secret_input_reloads_metadata("computer-use").await;
}

async fn waiting_secret_input_reloads_metadata(change: &str) {
    let _environment = crate::env_lock::lock();
    let root = TestRoot::new();
    let _home = IsolatedHome::new(root.path());
    let room = TestRoom::new("vault-input-metadata-race");
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let mut credential = browser_credential("login", "old-key");
    if change == "computer-use" {
        credential.allowed_uses = vec![crate::config::UserCredentialUse::Computer];
        credential.injection = crate::config::UserCredentialInjectionConfig::Computer;
    }
    registry.upsert(credential.clone()).unwrap();
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
    // Same registry writes as the lifecycle APIs, while holding their writer lock.
    match change {
        "remove" => {
            registry.remove("login").unwrap();
        }
        "host" => {
            credential.allowed_hosts = vec!["other.test".into()];
            registry.upsert(credential).unwrap();
        }
        "source" => {
            credential.source = crate::config::UserCredentialSourceConfig::Vault {
                key: "new-key".into(),
            };
            registry.upsert(credential).unwrap();
        }
        _ => {
            credential.allowed_uses = vec![crate::config::UserCredentialUse::Browser];
            credential.injection = crate::config::UserCredentialInjectionConfig::Browser;
            registry.upsert(credential).unwrap();
        }
    }
    drop(mutation);
    let acquired = input.await;
    if change == "remove" {
        assert!(
            acquired.is_err(),
            "MP-08/MP-10/MP-11: waiting input must reject a removed handle"
        );
        return;
    }
    let (service, _guard) = acquired.unwrap();
    match change {
        "host" => assert!(
            service
                .validate_browser_secret_input_for_target_url("login", "https://fixture.test/login")
                .is_err(),
            "MP-08/MP-10/MP-11: waiting fill must obey current host policy"
        ),
        "source" => {
            assert_eq!(
                service.credential_vault_key("login").unwrap(),
                Some("new-key")
            );
            let store = &room.runtime.owned.room_secret_observations;
            let rooms = store.rooms.lock().unwrap();
            assert_eq!(
                rooms[&room.session_id].vault_keys,
                BTreeSet::from(["new-key".into()])
            );
        }
        _ => assert!(
            service.validate_computer_secret_input("login").is_err(),
            "MP-08/MP-10/MP-11: waiting Computer input must obey current use policy"
        ),
    }
}

#[tokio::test]
async fn corrupt_restarted_room_cannot_block_unrelated_credential_removal() {
    corrupt_restarted_room_public_lifecycle(false).await;
}

#[tokio::test]
async fn corrupt_restarted_room_remains_publicly_deletable() {
    corrupt_restarted_room_public_lifecycle(true).await;
}

async fn corrupt_restarted_room_public_lifecycle(delete_only: bool) {
    let _environment = crate::env_lock::lock();
    for corruption in ["truncated", "undecryptable"] {
        let root = TestRoot::new();
        let _home = IsolatedHome::new(root.path());
        let mut room = TestRoom::new("vault-corrupt-public-lifecycle");
        let config = room.runtime.owned.config_projection.snapshot();
        let path = root.path().join("observations");
        let store = RoomSecretObservations::new(path.clone(), BTreeSet::new())
            .with_identity(&config.relay_private_key);
        store
            .register_credential_source(&room.session_id, Some("damaged-key"))
            .unwrap();
        store.register(&room.session_id, "synthetic-only").unwrap();
        if corruption == "truncated" {
            std::fs::write(store.registry_path(&room.session_id), b"{").unwrap();
        } else {
            // Authenticated ciphertext for another Room cannot be opened here.
            store.register("other-room", "synthetic-other").unwrap();
            std::fs::copy(
                store.registry_path("other-room"),
                store.registry_path(&room.session_id),
            )
            .unwrap();
        }
        room.runtime.owned.room_secret_observations =
            RoomSecretObservations::new(path, BTreeSet::new())
                .with_identity(&config.relay_private_key);
        let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
        registry
            .upsert(browser_credential("unrelated", "unrelated-key"))
            .unwrap();
        if !delete_only {
            crate::runtime::capability_registry::execute_capability_registry_request(
                &room.runtime,
                crate::local::LocalDaemonRequest::RemoveCredential(
                    crate::local::RemoveCredentialRequest {
                        id: "unrelated".into(),
                    },
                ),
                config.user_config.credential_vault.clone(),
            )
            .await
            .expect("MP-08/MP-10/MP-11: corrupt Room must not veto unrelated Vault mutation");
            assert!(registry.get("unrelated").unwrap().is_none());
        }
        let store = &room.runtime.owned.room_secret_observations;
        assert!(store.withholds_history(&room.session_id, 0));
        assert!(store
            .require(&room.session_id, true)
            .unwrap_err()
            .to_string()
            .contains("Room observation state is fenced"));
        assert_eq!(
            store
                .scrub_runtime_result(&room.session_id, "workflow ack committed".to_string())
                .unwrap(),
            "workflow ack committed"
        );
        assert!(!store
            .uses_vault_key(&room.session_id, "unrelated-key")
            .unwrap());
        let restarted = RoomSecretObservations::new(store.root.clone(), BTreeSet::new())
            .with_identity(&config.relay_private_key);
        assert!(restarted.require(&room.session_id, true).is_err());
        assert!(restarted.withholds_history(&room.session_id, 0));
        assert!(!restarted
            .uses_vault_key(&room.session_id, "unrelated-key")
            .unwrap());
        if !delete_only {
            // Same callback as verified fresh provisioning, with no human interaction.
            let mut fresh_slice = pending_test_slice(&crate::slice::SliceStore::default(), "fresh");
            fresh_slice.environment_session_id = Some(room.session_id.clone());
            room.runtime
                .reset_fresh_slice_observation_environment(&fresh_slice)
                .await
                .unwrap();
            assert!(store.require(&room.session_id, true).is_ok());
            assert!(store.withholds_history(&room.session_id, 0));
            assert!(room
                .runtime
                .owned
                .session_store
                .get_session(&room.session_id)
                .unwrap()
                .active_interactions()
                .is_empty());
        }
        room.runtime
            .delete_session_id(&room.session_id)
            .await
            .expect("MP-08/MP-10/MP-11: public Room deletion must recover a corrupt registry");
        let store = &room.runtime.owned.room_secret_observations;
        assert!(!store.registry_path(&room.session_id).exists());
        assert!(!store.marker(&room.session_id).exists());
    }
}

#[tokio::test]
async fn public_history_new_vault_credential_retires_prior_public_value() {
    // MP-08 / MP-10 / MP-11: supplementary regression for the real hidden-input
    // Vault creation drill. Only successful creation can invalidate public text.
    let _environment = crate::env_lock::lock();
    let root = TestRoot::new();
    let _home = IsolatedHome::new(root.path());
    let room = TestRoom::new("vault-public-history-creation");
    let owner = room
        .runtime
        .owned
        .session_store
        .get_session(&room.session_id)
        .unwrap()
        .owner_user_id()
        .to_owned();
    let history = &room.runtime.owned.operational_history_store;
    let prior = history
        .append_operational_event(
            crate::history::HistoryEventKind::UserPrompt,
            None,
            Some("before synthetic_vault_creation_value".into()),
            Default::default(),
            crate::history::HistoryEventTurnContext {
                session_id: Some(room.session_id.clone()),
                agent_id: Some(room.agent_id.clone()),
                public_history_owner_user_id: Some(owner.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    {
        let _guard = history.lock_public_history().unwrap();
        assert!(history
            .read_public_history_locked(&owner, &room.session_id, &prior.event_id)
            .unwrap()
            .is_some());
    }
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let mut credential = browser_credential("creation", "creation-key");
    credential.metadata =
        Some(serde_json::from_value(serde_json::json!({"session_id":room.session_id})).unwrap());
    let vault_config = crate::config::UserCredentialVaultConfig {
        path: root.path().join("vault").display().to_string(),
        ..Default::default()
    };
    crate::secret::create_chariox_encrypted_vault_for_test(
        std::path::Path::new(&vault_config.path),
        "synthetic-passphrase-only",
    )
    .unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        std::path::Path::new(&vault_config.path),
        "synthetic-passphrase-only",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let service = crate::secret::RuntimeSecretService::with_vault_config(Vec::new(), &vault_config)
        .unwrap();
    room.runtime
        .upsert_observed_vault_credential(
            &service,
            &registry,
            credential,
            "synthetic_vault_creation_value",
            false,
        )
        .await
        .unwrap();
    assert!(room
        .runtime
        .owned
        .room_secret_observations
        .uses_vault_key(&room.session_id, "creation-key")
        .unwrap());
    let _guard = history.lock_public_history().unwrap();
    assert!(history
        .search_public_history_locked(
            &owner,
            &room.session_id,
            None,
            "synthetic_vault_creation_value",
            50,
            None
        )
        .unwrap()
        .hits
        .is_empty());
    assert!(history
        .read_public_history_locked(&owner, &room.session_id, &prior.event_id)
        .unwrap()
        .is_none());
}
