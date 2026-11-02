//! MP-08 / MP-09 / MP-10 / MP-11: canonical entry, immutable binding and atomic rollback.
use super::*;
use crate::credential::CharioxCredentialRegistry;
use crate::secret::{CredentialVaultStore, RuntimeSecretService, VAULT_ACTIVITY_KEY};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Default)]
struct Store {
    values: Mutex<BTreeMap<String, String>>,
    fail_activity: AtomicBool,
}
impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Store([REDACTED])")
    }
}
impl CredentialVaultStore for Store {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        self.find_secret(service, key)?
            .ok_or_else(|| failure("Missing value"))
    }
    fn find_secret(&self, _: &str, key: &str) -> Result<Option<String>, DaemonError> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }
    fn set_secret(&self, _: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        if key == VAULT_ACTIVITY_KEY && self.fail_activity.swap(false, Ordering::SeqCst) {
            return Err(failure("Injected Activity write failure"));
        }
        self.values.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete_secret(&self, _: &str, key: &str) -> Result<(), DaemonError> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}
fn entry() -> UserCredentialConfig {
    owner_credential(
        "owner".into(),
        "Name".into(),
        "Password",
        VaultUseBinding {
            target: VaultUseTarget::site("https://example.org").unwrap(),
            value_kind: Default::default(),
        },
    )
    .unwrap()
}

#[test]
fn owner_vault_entries_use_the_agent_resolver_and_preserve_their_binding() {
    let root = std::env::temp_dir().join(format!("owner-bound-{:032x}", rand::random::<u128>()));
    let registry = CharioxCredentialRegistry::new(root.clone());
    let store = Arc::new(Store::default());
    let entry = entry();
    let id = entry.id.clone();
    let service =
        RuntimeSecretService::with_vault_store(vec![], format!("test-{id}"), store.clone());
    commit_entry(
        &service,
        &registry,
        None,
        entry.clone(),
        Some("old-value"),
        "Created by owner",
    )
    .unwrap();
    assert_eq!(registry.list().unwrap().len(), 1);
    let mut changed = entry.clone();
    changed
        .metadata
        .as_mut()
        .unwrap()
        .use_binding
        .as_mut()
        .unwrap()
        .target = VaultUseTarget::site("https://other.org").unwrap();
    assert!(registry.upsert(changed).is_err());
    commit_entry(
        &service,
        &registry,
        Some(entry.clone()),
        entry.clone(),
        Some("new-value"),
        "Replaced by owner",
    )
    .unwrap();
    let resolver = RuntimeSecretService::with_vault_store(
        registry.list().unwrap(),
        format!("test-{id}"),
        store,
    );
    assert!(
        resolver
            .browser_secret_input_for_target_url(&id, "https://example.org/login")
            .unwrap()
            == "new-value"
    );
    assert!(resolver
        .browser_secret_input_for_target_url(&id, "https://other.org")
        .is_err());
    assert!(resolver
        .browser_secret_input_for_target_url(&id, "https://sub.example.org")
        .is_err());
    assert_eq!(
        registry
            .get(&id)
            .unwrap()
            .unwrap()
            .metadata
            .unwrap()
            .use_binding,
        entry.metadata.unwrap().use_binding
    );
    assert_eq!(service.vault_activity(&id).unwrap().len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn owner_vault_changes_roll_back_if_activity_cannot_be_committed() {
    for operation in ["create", "replace", "delete"] {
        let root =
            std::env::temp_dir().join(format!("owner-rollback-{:032x}", rand::random::<u128>()));
        let registry = CharioxCredentialRegistry::new(root.clone());
        let store = Arc::new(Store::default());
        let entry = entry();
        let service = RuntimeSecretService::with_vault_store(
            vec![],
            format!("test-{}", entry.id),
            store.clone(),
        );
        let previous = if operation == "create" {
            None
        } else {
            commit_entry(
                &service,
                &registry,
                None,
                entry.clone(),
                Some("old-value"),
                "Created by owner",
            )
            .unwrap();
            Some(entry.clone())
        };
        store.fail_activity.store(true, Ordering::SeqCst);
        assert!(commit_entry(
            &service,
            &registry,
            previous.clone(),
            entry.clone(),
            if operation == "delete" {
                None
            } else {
                Some("new-value")
            },
            "Changed by owner"
        )
        .is_err());
        assert_eq!(
            registry.get(&entry.id).unwrap().is_some(),
            previous.is_some()
        );
        let value = service.vault_secret_for_dictation(&entry.id).unwrap();
        assert!(match value {
            Some(value) => previous.is_some() && value.as_str() == "old-value",
            None => previous.is_none(),
        });
        assert_eq!(
            service.vault_activity(&entry.id).unwrap().len(),
            usize::from(previous.is_some())
        );
        if root.exists() {
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn vault_activity_cannot_be_resolved_or_changed_as_a_credential() {
    let store = Arc::new(Store::default());
    let mut entry = entry();
    entry.source = UserCredentialSourceConfig::Vault {
        key: VAULT_ACTIVITY_KEY.into(),
    };
    let service =
        RuntimeSecretService::with_vault_store(vec![entry], "activity-reserved-test", store);
    service
        .record_vault_activity("entry", "Viewed by owner")
        .unwrap();
    assert!(service
        .vault_secret_for_dictation(VAULT_ACTIVITY_KEY)
        .is_err());
    assert!(service
        .set_vault_secret(VAULT_ACTIVITY_KEY, "replacement")
        .is_err());
    assert!(service.delete_vault_secret(VAULT_ACTIVITY_KEY).is_err());
    assert_eq!(service.vault_activity("entry").unwrap().len(), 1);
}

#[test]
fn vault_activity_rolls_back_a_write_that_failed_after_rename() {
    let root = std::env::temp_dir().join(format!(
        "activity-post-rename-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&path, "owner-passkey").unwrap();
    crate::secret::unlock_existing_chariox_encrypted_vault(
        &path,
        "owner-passkey",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let config = crate::config::UserCredentialVaultConfig {
        path: path.display().to_string(),
        ..Default::default()
    };
    let service = RuntimeSecretService::with_vault_config(vec![], &config).unwrap();
    service
        .record_vault_activity("entry", "Created by owner")
        .unwrap();
    crate::secret::fail_next_vault_dir_sync_for_test();
    assert!(service
        .record_vault_activity("entry", "Viewed by owner")
        .is_err());
    assert_eq!(service.vault_activity("entry").unwrap().len(), 1);
    crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn owner_factor_command_and_file_entries_use_the_real_injection_channels() {
    let root = std::env::temp_dir().join(format!("owner-channels-{:032x}", rand::random::<u128>()));
    let registry = CharioxCredentialRegistry::new(root.clone());
    let store = Arc::new(Store::default());
    let service = RuntimeSecretService::with_vault_store(vec![], "owner-channels", store.clone());
    let cases = [
        (
            "2FA code",
            VaultUseTarget::Totp {
                origin: "https://example.org".into(),
            },
            "otpauth://totp/Example?secret=JBSWY3DPEHPK3PXP&issuer=Example",
        ),
        (
            "Command secret",
            VaultUseTarget::CommandEnvironment {
                name: "API_KEY".into(),
                origin: "https://example.org".into(),
            },
            "command-value",
        ),
        (
            "File secret",
            VaultUseTarget::File {
                path: "/work/config.env".into(),
            },
            "file-value",
        ),
    ];
    let mut ids = Vec::new();
    for (kind, target, value) in cases {
        let entry = owner_credential(
            "local".into(),
            kind.into(),
            kind,
            VaultUseBinding {
                target: target.clone(),
                value_kind: Default::default(),
            },
        )
        .unwrap();
        ids.push((entry.id.clone(), target));
        commit_entry(
            &service,
            &registry,
            None,
            entry,
            Some(value),
            "Created by owner",
        )
        .unwrap();
    }
    let resolver =
        RuntimeSecretService::with_vault_store(registry.list().unwrap(), "owner-channels", store);
    let code = resolver
        .browser_secret_input_for_target_url(&ids[0].0, "https://example.org/login")
        .unwrap();
    assert!(code.len() == 6 && code.bytes().all(|c| c.is_ascii_digit()));
    assert!(resolver
        .browser_secret_input_for_target_url(&ids[0].0, "https://other.org")
        .is_err());
    assert_eq!(
        resolver
            .workspace_secret_input_for_target(&ids[1].0, &ids[1].1)
            .unwrap(),
        "command-value"
    );
    assert_eq!(
        resolver
            .workspace_secret_input_for_target(&ids[2].0, &ids[2].1)
            .unwrap(),
        "file-value"
    );
    for target in [
        VaultUseTarget::CommandEnvironment {
            name: "OTHER_KEY".into(),
            origin: "https://example.org".into(),
        },
        VaultUseTarget::CommandEnvironment {
            name: "API_KEY".into(),
            origin: "https://other.org".into(),
        },
        VaultUseTarget::CommandStdin {
            origin: "https://example.org".into(),
        },
        ids[2].1.clone(),
    ] {
        assert!(resolver
            .workspace_secret_input_for_target(&ids[1].0, &target)
            .is_err());
    }
    assert!(
        resolver.command_secret_input(&ids[1].0).is_err(),
        "a bound entry cannot bypass its destination"
    );
    assert!(resolver
        .workspace_secret_input_for_target(
            &ids[2].0,
            &VaultUseTarget::File {
                path: "/work/other.env".into()
            }
        )
        .is_err());
    std::fs::remove_dir_all(root).unwrap();
}
