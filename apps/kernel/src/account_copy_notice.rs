//! MP-08/MP-10/MP-11: local metadata only; provider CLIs own the credential state.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CredentialCopyNotice {
    owner_user_id: String,
    provider: String,
    profile_id: String,
    source_machine: String,
    #[serde(default)]
    notified_machines: Vec<String>,
}

/// Source side of a copy: this profile's login was handed to another machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct IssuedCredentialCopy {
    owner_user_id: String,
    provider: String,
    profile_id: String,
}

fn nonempty(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(serde_json::Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

pub(crate) fn renewable_login(provider: &str, value: &serde_json::Value) -> bool {
    match provider {
        "codex" => {
            value.get("auth_mode").and_then(serde_json::Value::as_str) != Some("apikey")
                && !nonempty(value.get("OPENAI_API_KEY"))
                && nonempty(value.pointer("/tokens/refresh_token"))
        }
        "claude" => nonempty(value.pointer("/claudeAiOauth/refreshToken")),
        "opencode" => value.get("openai").is_some_and(|auth| {
            auth.get("type").and_then(serde_json::Value::as_str) == Some("oauth")
                && nonempty(auth.get("refresh"))
        }),
        _ => false,
    }
}

impl ProviderAccountProfileRegistry {
    pub(crate) fn has_renewable_login(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        // A provider can remove an invalidated OAuth file before reporting the failure.
        // Retain the provenance of a copied renewable login for that recovery case.
        if let Some(stored) = self.stored_login_is_renewable(owner_user_id, provider, profile_id) {
            return Ok(stored);
        }
        let profile = self.get(owner_user_id, provider, profile_id)?;
        Ok(self
            .read_document()?
            .credential_copy_notices
            .iter()
            .any(|notice| {
                notice.owner_user_id == owner_user_id
                    && notice.provider == profile.provider
                    && notice.profile_id == profile.profile_id
            }))
    }

    /// `Some(true)` for a stored refresh-token login, `Some(false)` for a stored API key,
    /// `None` when no stored credential says either.
    pub(crate) fn stored_login_is_renewable(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Option<bool> {
        let materialization = self
            .export_materialization(owner_user_id, provider, profile_id)
            .ok()?;
        for file in &materialization.files {
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&file.contents_base64)
            else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if renewable_login(provider, &value) {
                return Some(true);
            }
            if nonempty(value.get("OPENAI_API_KEY"))
                || value
                    .pointer("/openai/type")
                    .and_then(serde_json::Value::as_str)
                    == Some("api")
            {
                return Some(false);
            }
        }
        None
    }

    /// Whether another machine may hold this login's refresh-token chain: this
    /// machine received it as a copy, or handed copies out. Sticky for the life
    /// of the profile; provider logout or login would revoke the shared chain.
    pub(crate) fn credential_is_shared(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        let profile = self.get(owner_user_id, provider, profile_id)?;
        let document = self.read_document()?;
        let stored =
            resolve_stored_profile(&document, owner_user_id, &profile.provider, profile_id)?;
        Ok(stored.materialized_replica
            || stored.managed_context_replica.is_some()
            || !profile.materializations.is_empty()
            || document.credential_copy_notices.iter().any(|notice| {
                notice.owner_user_id == owner_user_id
                    && notice.provider == profile.provider
                    && notice.profile_id == profile.profile_id
            })
            || document.credential_copies_issued.iter().any(|issued| {
                issued.owner_user_id == owner_user_id
                    && issued.provider == profile.provider
                    && issued.profile_id == profile.profile_id
            }))
    }

    pub(crate) fn record_credential_issued(
        &self,
        owner_user_id: &str,
        materialization: &ProviderAccountMaterialization,
    ) -> Result<(), DaemonError> {
        let provider = normalize_provider(&materialization.profile.provider)?;
        if !materialization_is_renewable(provider, materialization) {
            return Ok(());
        }
        let mut document = self.write_document()?;
        let profile_id = &materialization.profile.profile_id;
        if document.credential_copies_issued.iter().any(|issued| {
            issued.owner_user_id == owner_user_id
                && issued.provider == provider
                && issued.profile_id == *profile_id
        }) {
            return Ok(());
        }
        let original = document.clone();
        document
            .credential_copies_issued
            .push(IssuedCredentialCopy {
                owner_user_id: owner_user_id.into(),
                provider: provider.into(),
                profile_id: profile_id.clone(),
            });
        if let Err(error) = self.persist_locked(&document) {
            *document = original;
            return Err(error);
        }
        Ok(())
    }

    /// Removes this machine's stored login without contacting the provider, so
    /// no other holder of the same refresh-token chain is signed out.
    pub(crate) fn discard_local_credential(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<(), DaemonError> {
        let provider = normalize_provider(provider)?;
        let document = self.read_document()?;
        let stored = resolve_stored_profile(&document, owner_user_id, provider, profile_id)?;
        match &stored.locator {
            ProviderAccountLocator::Codex { codex_home } => {
                remove_credential_file(codex_home, "auth.json")
            }
            ProviderAccountLocator::Claude {
                claude_config_dir,
                ambient_default,
            } => {
                remove_credential_file(claude_config_dir, ".credentials.json")?;
                remove_claude_keychain_credentials(
                    claude_config_dir,
                    *ambient_default == Some(true),
                )
            }
            ProviderAccountLocator::Opencode { .. } => Ok(()),
        }
    }

    pub(crate) fn record_credential_copy(
        &self,
        owner_user_id: &str,
        materialization: &ProviderAccountMaterialization,
        source_machine: &str,
    ) -> Result<(), DaemonError> {
        let provider = normalize_provider(&materialization.profile.provider)?;
        if !materialization_is_renewable(provider, materialization) {
            return Ok(());
        }
        let mut document = self.write_document()?;
        if document.credential_copy_notices.iter().any(|notice| {
            notice.owner_user_id == owner_user_id
                && notice.provider == provider
                && notice.profile_id == materialization.profile.profile_id
        }) {
            return Ok(());
        }
        let original = document.clone();
        document.credential_copy_notices.push(CredentialCopyNotice {
            owner_user_id: owner_user_id.into(),
            provider: provider.into(),
            profile_id: materialization.profile.profile_id.clone(),
            source_machine: source_machine.into(),
            notified_machines: Vec::new(),
        });
        if let Err(error) = self.persist_locked(&document) {
            *document = original;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn take_credential_copy_notice(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        machine: &str,
    ) -> Result<Option<String>, DaemonError> {
        let profile = self.get(owner_user_id, provider, profile_id)?;
        let mut document = self.write_document()?;
        let original = document.clone();
        let Some(notice) = document.credential_copy_notices.iter_mut().find(|notice| {
            notice.owner_user_id == owner_user_id
                && notice.provider == profile.provider
                && notice.profile_id == profile.profile_id
                && !notice.notified_machines.iter().any(|id| id == machine)
        }) else {
            return Ok(None);
        };
        let provider = match profile.provider.as_str() {
            "codex" => "Codex",
            "claude" => "Claude",
            "opencode" => "OpenCode",
            other => other,
        };
        let message = format!("{provider} credentials on {machine} were copied from {}. The provider may invalidate one copy when another refreshes; you may need to log in on this machine later.", notice.source_machine);
        notice.notified_machines.push(machine.into());
        if let Err(error) = self.persist_locked(&document) {
            *document = original;
            return Err(error);
        }
        Ok(Some(message))
    }
}

fn materialization_is_renewable(
    provider: &str,
    materialization: &ProviderAccountMaterialization,
) -> bool {
    materialization.files.iter().any(|file| {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&file.contents_base64)
        else {
            return false;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return false;
        };
        renewable_login(provider, &value)
    })
}

fn remove_credential_file(root: &Path, name: &str) -> Result<(), DaemonError> {
    validate_materialization_root(root)?;
    match fs::remove_file(root.join(name)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(registry_error(
            "discard local provider credential",
            error.to_string(),
        )),
        _ => Ok(()),
    }
}

#[cfg(target_os = "macos")]
fn remove_claude_keychain_credentials(
    claude_config_dir: &Path,
    ambient_default: bool,
) -> Result<(), DaemonError> {
    for service in claude_keychain_service_names(claude_config_dir, ambient_default) {
        let output = std::process::Command::new("security")
            .args(["delete-generic-password", "-s", &service])
            .output()
            .map_err(registry_io("discard local provider credential"))?;
        keychain_delete_outcome(output.status.code(), &output.stderr)?;
    }
    Ok(())
}

/// `security` exits 44 when the item is absent, which is a finished deletion; any other failure
/// (locked or inaccessible Keychain, denied access) leaves the usable login in place.
#[cfg(any(target_os = "macos", test))]
fn keychain_delete_outcome(code: Option<i32>, stderr: &[u8]) -> Result<(), DaemonError> {
    const ERR_SEC_ITEM_NOT_FOUND: i32 = 44;
    if code == Some(0) || code == Some(ERR_SEC_ITEM_NOT_FOUND) {
        return Ok(());
    }
    Err(registry_error(
        "discard local provider credential",
        format!(
            "Keychain deletion failed ({}): {}",
            code.map_or_else(|| "signal".to_string(), |code| code.to_string()),
            String::from_utf8_lossy(stderr).trim()
        ),
    ))
}

#[cfg(not(target_os = "macos"))]
fn remove_claude_keychain_credentials(_: &Path, _: bool) -> Result<(), DaemonError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keychain_deletion_failures_other_than_an_absent_item_are_errors() {
        assert!(keychain_delete_outcome(Some(0), b"").is_ok());
        assert!(
            keychain_delete_outcome(Some(44), b"The specified item could not be found").is_ok()
        );
        let denied = keychain_delete_outcome(Some(36), b"User interaction is not allowed.")
            .expect_err("a locked Keychain must not count as a deleted login");
        assert!(denied.to_string().contains("User interaction"), "{denied}");
        assert!(keychain_delete_outcome(None, b"").is_err());
    }

    #[test]
    fn mp08_mp10_mp11_only_renewable_logins_get_copy_notices() {
        assert!(renewable_login(
            "codex",
            &serde_json::json!({"tokens":{"refresh_token":"synthetic"}})
        ));
        assert!(!renewable_login(
            "codex",
            &serde_json::json!({"OPENAI_API_KEY":"synthetic","tokens":{"refresh_token":"synthetic"}})
        ));
        assert!(renewable_login(
            "claude",
            &serde_json::json!({"claudeAiOauth":{"refreshToken":"synthetic"}})
        ));
        assert!(!renewable_login(
            "claude",
            &serde_json::json!({"setupToken":"synthetic"})
        ));
        assert!(renewable_login(
            "opencode",
            &serde_json::json!({"openai":{"type":"oauth","refresh":"synthetic"}})
        ));
        assert!(!renewable_login(
            "opencode",
            &serde_json::json!({"openai":{"type":"api","key":"synthetic"}})
        ));
    }

    #[test]
    fn mp08_mp10_mp11_notice_is_once_per_account_machine_across_restart() {
        let root =
            std::env::temp_dir().join(format!("chariox-copy-notice-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("registry.json");
        let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
        let profile = registry
            .create_managed("owner", "codex", "Synthetic")
            .unwrap();
        let materialization = ProviderAccountMaterialization {
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: "owner".into(),
                provider: "codex".into(),
                profile_id: profile.profile_id.clone(),
                label: "Synthetic".into(),
                origin: profile.origin,
                is_default: false,
            },
            files: vec![ProviderAccountMaterializationFile {
                relative_path: "auth.json".into(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(br#"{"tokens":{"refresh_token":"synthetic"}}"#),
            }],
            generated_at_ms: 1,
        };
        registry
            .record_credential_copy("owner", &materialization, "source-machine")
            .unwrap();
        let message = registry
            .take_credential_copy_notice("owner", "codex", &profile.profile_id, "worker-machine")
            .unwrap()
            .unwrap();
        assert_eq!(message, "Codex credentials on worker-machine were copied from source-machine. The provider may invalidate one copy when another refreshes; you may need to log in on this machine later.");
        assert!(registry
            .take_credential_copy_notice("owner", "codex", &profile.profile_id, "worker-machine")
            .unwrap()
            .is_none());
        drop(registry);
        let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
        registry
            .record_credential_copy("owner", &materialization, "different-source")
            .unwrap();
        assert!(registry
            .take_credential_copy_notice("owner", "codex", &profile.profile_id, "worker-machine")
            .unwrap()
            .is_none());
        assert!(registry
            .take_credential_copy_notice("owner", "codex", &profile.profile_id, "another-machine")
            .unwrap()
            .is_some());
        assert!(!String::from_utf8(fs::read(&path).unwrap())
            .unwrap()
            .contains("refresh_token"));
        fs::remove_dir_all(root).unwrap();
    }

    fn synthetic_materialization(
        profile: &ProviderAccountProfile,
        auth: &[u8],
    ) -> ProviderAccountMaterialization {
        ProviderAccountMaterialization {
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: "owner".into(),
                provider: profile.provider.clone(),
                profile_id: profile.profile_id.clone(),
                label: "Synthetic".into(),
                origin: profile.origin,
                is_default: false,
            },
            files: vec![ProviderAccountMaterializationFile {
                relative_path: "auth.json".into(),
                contents_base64: base64::engine::general_purpose::STANDARD.encode(auth),
            }],
            generated_at_ms: 1,
        }
    }

    fn codex_home(registry: &ProviderAccountProfileRegistry, profile_id: &str) -> PathBuf {
        PathBuf::from(
            &registry
                .resolve_environment("owner", "codex", profile_id)
                .unwrap()["CODEX_HOME"],
        )
    }

    #[test]
    fn credfix_login_with_copies_elsewhere_is_shared_and_discard_stays_local() {
        let root = std::env::temp_dir().join(format!("chariox-credfix-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("registry.json");
        let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
        let profile = registry.create_managed("owner", "codex", "Source").unwrap();
        let home = codex_home(&registry, &profile.profile_id);
        fs::create_dir_all(&home).unwrap();
        let login = br#"{"tokens":{"refresh_token":"synthetic"}}"#;
        fs::write(home.join("auth.json"), login).unwrap();
        fs::write(home.join("config.toml"), "keep").unwrap();
        let shared = |registry: &ProviderAccountProfileRegistry| {
            registry
                .credential_is_shared("owner", "codex", &profile.profile_id)
                .unwrap()
        };
        assert!(!shared(&registry), "a sole holder may sign out normally");

        // Handing out an API key is not a shared refresh-token chain.
        registry
            .record_credential_issued(
                "owner",
                &synthetic_materialization(&profile, br#"{"OPENAI_API_KEY":"synthetic"}"#),
            )
            .unwrap();
        assert!(!shared(&registry));

        registry
            .record_credential_issued("owner", &synthetic_materialization(&profile, login))
            .unwrap();
        assert!(shared(&registry));
        drop(registry);
        let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
        assert!(shared(&registry), "the copy ledger survives a restart");

        registry
            .discard_local_credential("owner", "codex", &profile.profile_id)
            .unwrap();
        assert!(!home.join("auth.json").exists());
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            "keep"
        );
        registry
            .discard_local_credential("owner", "codex", &profile.profile_id)
            .expect("an already removed credential is not an error");
        assert!(!String::from_utf8(fs::read(&path).unwrap())
            .unwrap()
            .contains("refresh_token"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn credfix_a_received_copy_is_shared() {
        let root = std::env::temp_dir().join(format!("chariox-credfix-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let registry = ProviderAccountProfileRegistry::open(root.join("registry.json")).unwrap();
        let profile = registry.create_managed("owner", "codex", "Copy").unwrap();
        assert!(!registry
            .credential_is_shared("owner", "codex", &profile.profile_id)
            .unwrap());
        registry
            .record_credential_copy(
                "owner",
                &synthetic_materialization(&profile, br#"{"tokens":{"refresh_token":"x"}}"#),
                "source-machine",
            )
            .unwrap();
        assert!(registry
            .credential_is_shared("owner", "codex", &profile.profile_id)
            .unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn credfix_stored_login_kind_distinguishes_refresh_logins_from_keys_and_nothing() {
        let root = std::env::temp_dir().join(format!("chariox-credfix-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let registry = ProviderAccountProfileRegistry::open(root.join("registry.json")).unwrap();
        let profile = registry.create_managed("owner", "codex", "Kind").unwrap();
        let home = codex_home(&registry, &profile.profile_id);
        fs::create_dir_all(&home).unwrap();
        let kind = || registry.stored_login_is_renewable("owner", "codex", &profile.profile_id);
        assert_eq!(kind(), None);
        fs::write(home.join("auth.json"), br#"{"OPENAI_API_KEY":"synthetic"}"#).unwrap();
        assert_eq!(kind(), Some(false));
        fs::write(
            home.join("auth.json"),
            br#"{"tokens":{"refresh_token":"synthetic"}}"#,
        )
        .unwrap();
        assert_eq!(kind(), Some(true));
        fs::remove_dir_all(root).unwrap();
    }
}
