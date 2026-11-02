//! MP-08 / MP-10 / MP-11: red-capable failure injection at the real storage seam.
use super::*;
use crate::config::{
    UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialSourceConfig,
};
use crate::credential::CharioxCredentialRegistry;
use crate::secret::{CredentialVaultStore, RuntimeSecretService, OWNER_VAULT_CATALOGUE_KEY};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Default)]
struct Store {
    values: Mutex<BTreeMap<String, String>>,
    fail_catalogue: AtomicBool,
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
        if key == OWNER_VAULT_CATALOGUE_KEY && self.fail_catalogue.swap(false, Ordering::SeqCst) {
            return Err(failure("Injected catalogue write failure"));
        }
        self.values.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete_secret(&self, _: &str, key: &str) -> Result<(), DaemonError> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[test]
fn owner_vault_registered_changes_roll_back_when_activity_cannot_be_saved() {
    for replacement in [Some("new-value"), None] {
        let root = std::env::temp_dir().join(format!(
            "owner-vault-rollback-{:032x}",
            rand::random::<u128>()
        ));
        let registry = CharioxCredentialRegistry::new(root.clone());
        let entry = UserCredentialConfig {
            id: "entry".into(),
            description: Some("Name".into()),
            source: UserCredentialSourceConfig::Vault {
                key: "entry".into(),
            },
            allowed_hosts: vec!["example.org".into()],
            allowed_uses: vec![crate::config::UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
            metadata: None,
        };
        registry.upsert(entry.clone()).unwrap();
        let store = Arc::new(Store::default());
        let service = RuntimeSecretService::with_vault_store(
            vec![entry.clone()],
            format!("owner-vault-test-{:032x}", rand::random::<u128>()),
            store.clone(),
        );
        service.set_vault_secret("entry", "old-value").unwrap();
        service
            .save_owner_vault_catalogue(Some("previous-catalogue"))
            .unwrap();
        let mut catalogue = Catalogue::default();
        catalogue.record("entry", "Replaced by owner");
        store.fail_catalogue.store(true, Ordering::SeqCst);
        assert!(commit_registered(
            &service,
            &registry,
            entry,
            &catalogue,
            Some("previous-catalogue"),
            replacement
        )
        .is_err());
        assert!(
            service
                .vault_secret_for_dictation("entry")
                .unwrap()
                .unwrap()
                .as_str()
                == "old-value"
        );
        assert!(service.owner_vault_catalogue().unwrap().unwrap().as_str() == "previous-catalogue");
        assert!(registry.get("entry").unwrap().is_some());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn owner_vault_catalogue_cannot_be_used_or_changed_as_an_ordinary_credential() {
    let store = Arc::new(Store::default());
    let entry = UserCredentialConfig {
        id: "forged".into(),
        description: None,
        source: UserCredentialSourceConfig::Vault {
            key: OWNER_VAULT_CATALOGUE_KEY.into(),
        },
        allowed_hosts: vec![],
        allowed_uses: vec![crate::config::UserCredentialUse::Provider],
        injection: UserCredentialInjectionConfig::Provider,
        metadata: None,
    };
    let service = RuntimeSecretService::with_vault_store(
        vec![entry],
        format!("owner-vault-test-{:032x}", rand::random::<u128>()),
        store,
    );
    service
        .save_owner_vault_catalogue(Some("private-catalogue"))
        .unwrap();
    assert!(service.provider_secret_input("forged").is_err());
    assert!(service
        .vault_secret_for_dictation(OWNER_VAULT_CATALOGUE_KEY)
        .is_err());
    assert!(service
        .set_vault_secret(OWNER_VAULT_CATALOGUE_KEY, "replacement")
        .is_err());
    assert!(service
        .delete_vault_secret(OWNER_VAULT_CATALOGUE_KEY)
        .is_err());
    assert!(service.owner_vault_catalogue().unwrap().unwrap().as_str() == "private-catalogue");
}

#[test]
fn owner_vault_catalogue_rolls_back_a_write_that_failed_after_rename() {
    let root = std::env::temp_dir().join(format!(
        "owner-vault-post-rename-{:032x}",
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
        .save_owner_vault_catalogue(Some("original-catalogue"))
        .unwrap();
    crate::secret::fail_next_vault_dir_sync_for_test();
    assert!(service
        .save_owner_vault_catalogue(Some("changed-catalogue"))
        .is_err());
    assert!(service.owner_vault_catalogue().unwrap().unwrap().as_str() == "original-catalogue");
    crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
