//! MP-08/MP-10/MP-11: local metadata only; provider CLIs own the credential state.
use super::*;
#[cfg(target_os = "macos")]
use crate::runtime::bounded_process::BoundedCommandExt as _;

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
        if let Ok(materialization) =
            self.export_materialization(owner_user_id, provider, profile_id)
        {
            for file in &materialization.files {
                let Ok(bytes) =
                    base64::engine::general_purpose::STANDARD.decode(&file.contents_base64)
                else {
                    continue;
                };
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    continue;
                };
                if renewable_login(provider, &value) {
                    return Ok(true);
                }
                if nonempty(value.get("OPENAI_API_KEY"))
                    || value
                        .pointer("/openai/type")
                        .and_then(serde_json::Value::as_str)
                        == Some("api")
                {
                    return Ok(false);
                }
            }
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

    pub(crate) fn record_credential_copy(
        &self,
        owner_user_id: &str,
        materialization: &ProviderAccountMaterialization,
        source_machine: &str,
    ) -> Result<(), DaemonError> {
        let provider = normalize_provider(&materialization.profile.provider)?;
        let renewable = materialization.files.iter().any(|file| {
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&file.contents_base64)
            else {
                return false;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                return false;
            };
            renewable_login(provider, &value)
        });
        if !renewable {
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

    /// A shared refresh-token chain may have been received here or issued from here.
    pub(crate) fn credential_is_shared(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        let profile = self.get(owner_user_id, provider, profile_id)?;
        let document = self.read_document()?;
        let stored = resolve_stored_profile(
            &document,
            owner_user_id,
            &profile.provider,
            &profile.profile_id,
        )?;
        Ok(stored.materialized_replica
            || stored.managed_context_replica.is_some()
            || !profile.materializations.is_empty()
            || stored.transferred_from.is_some()
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
        let renewable = materialization.files.iter().any(|file| {
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&file.contents_base64)
            else {
                return false;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                return false;
            };
            renewable_login(provider, &value)
        });
        if !renewable {
            return Ok(());
        }
        let profile = self.get(owner_user_id, provider, &materialization.profile.profile_id)?;
        let mut document = self.write_document()?;
        if document.credential_copies_issued.iter().any(|issued| {
            issued.owner_user_id == owner_user_id
                && issued.provider == provider
                && issued.profile_id == profile.profile_id
        }) {
            return Ok(());
        }
        let original = document.clone();
        document.credential_copies_issued.push(IssuedCredentialCopy {
            owner_user_id: owner_user_id.into(),
            provider: provider.into(),
            profile_id: profile.profile_id,
        });
        if let Err(error) = self.persist_locked(&document) {
            *document = original;
            return Err(error);
        }
        Ok(())
    }

    /// Disconnect this holder without revoking any other holder of a shared login.
    pub(crate) fn discard_shared_login(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        let provider = normalize_provider(provider)?;
        if !matches!(provider, "codex" | "claude") {
            return Ok(false);
        }
        // Resolve aliases before looking up provenance, and retain a stable root
        // until its local credential has been removed.
        let profile = self.get(owner_user_id, provider, profile_id)?;
        if !self.credential_is_shared(owner_user_id, provider, &profile.profile_id)? {
            return Ok(false);
        }
        let document = self.read_document()?;
        let stored =
            resolve_stored_profile(&document, owner_user_id, provider, &profile.profile_id)?;

        match &stored.locator {
            ProviderAccountLocator::Codex { codex_home } => {
                remove_local_login_file(codex_home, "auth.json")?;
            }
            ProviderAccountLocator::Claude {
                claude_config_dir,
                ambient_default,
            } => {
                #[cfg(target_os = "macos")]
                for service in
                    claude_keychain_service_names(claude_config_dir, *ambient_default == Some(true))
                {
                    let status = crate::process_spawn::command("security")
                        .args(["delete-generic-password", "-s", &service])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .bounded_status()
                        .map_err(registry_io("discard copied Claude login"))?;
                    // security returns 44 when the item is already absent.
                    if !status.success() && status.code() != Some(44) {
                        return Err(registry_error(
                            "discard copied Claude login",
                            format!("Keychain deletion failed ({status}). Unlock the Keychain and retry."),
                        ));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                let _ = ambient_default;
                remove_local_login_file(claude_config_dir, ".credentials.json")?;
            }
            ProviderAccountLocator::Opencode { .. } => return Ok(false),
        }
        Ok(true)
    }

    /// Whether any copy notice still waits to be shown on `machine`. Every provider
    /// output pass asks before it walks the Room's agents.
    pub(crate) fn credential_copy_notice_waiting(&self, machine: &str) -> bool {
        self.read_document().is_ok_and(|document| {
            document
                .credential_copy_notices
                .iter()
                .any(|notice| !notice.notified_machines.iter().any(|id| id == machine))
        })
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
            if let Some(notice) = document.credential_copy_notices.iter_mut().find(|notice| {
                notice.owner_user_id == owner_user_id
                    && notice.provider == profile.provider
                    && notice.profile_id == profile.profile_id
            }) {
                notice.notified_machines.retain(|id| id != machine);
            }
            return Err(error);
        }
        Ok(Some(message))
    }
}

fn remove_local_login_file(root: &Path, name: &str) -> Result<(), DaemonError> {
    validate_materialization_root(root)?;
    match fs::remove_file(root.join(name)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(registry_error(
            "discard copied provider login",
            format!("{error}. Check access to the provider profile and retry."),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                provider_account_id: None,
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
    #[test]
    fn issued_and_received_login_provenance_remain_separate_and_owner_bound() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("issued-copy-provenance");
        std::env::set_var("CHARIOX_HOME", root.path().join("state"));
        let path = root.path().join("accounts.json");
        let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
        let source = registry.create_managed("owner", "codex", "Source").unwrap();
        let other = registry.create_managed("owner", "codex", "Other").unwrap();
        let foreign = registry.create_managed("other-owner", "codex", "Foreign").unwrap();
        let environment = registry.resolve_environment("owner", "codex", &source.profile_id).unwrap();
        let home = PathBuf::from(&environment["CODEX_HOME"]);
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("auth.json"), br#"{"OPENAI_API_KEY":"synthetic"}"#).unwrap();
        let api_key = registry.export_materialization("owner", "codex", &source.profile_id).unwrap();
        registry.record_credential_issued("owner", &api_key).unwrap();
        assert!(!registry.credential_is_shared("owner", "codex", &source.profile_id).unwrap());

        fs::write(home.join("auth.json"), br#"{"tokens":{"refresh_token":"synthetic"}}"#).unwrap();
        fs::write(home.join("config.toml"), "keep").unwrap();
        let issued = registry.export_materialization("owner", "codex", &source.profile_id).unwrap();
        registry.record_credential_issued("owner", &issued).unwrap();
        registry.record_credential_issued("owner", &issued).unwrap();
        assert!(registry.credential_is_shared("owner", "codex", &source.profile_id).unwrap());
        assert!(!registry.credential_is_shared("owner", "codex", &other.profile_id).unwrap());
        assert!(!registry.credential_is_shared("other-owner", "codex", &foreign.profile_id).unwrap());
        assert!(registry.credential_is_shared("other-owner", "codex", &source.profile_id).is_err());
        assert!(registry.record_credential_issued("other-owner", &issued).is_err());
        {
            let document = registry.read_document().unwrap();
            assert_eq!(document.credential_copies_issued.len(), 1);
            assert!(document.credential_copy_notices.is_empty(), "issuing is not receiving");
        }
        assert!(registry.take_credential_copy_notice("owner", "codex", &source.profile_id, "source-machine").unwrap().is_none());
        let mut received = issued.clone();
        received.profile.profile_id = other.profile_id.clone();
        registry.record_credential_copy("owner", &received, "source-machine").unwrap();
        assert!(registry.credential_is_shared("owner", "codex", &other.profile_id).unwrap());
        assert_eq!(registry.read_document().unwrap().credential_copies_issued.len(), 1);
        assert!(registry.take_credential_copy_notice("owner", "codex", &other.profile_id, "receiver-machine").unwrap().is_some());
        drop(registry);
        let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
        assert!(registry.credential_is_shared("owner", "codex", &source.profile_id).unwrap());
        assert!(registry.discard_shared_login("owner", "codex", &source.profile_id).unwrap());
        assert!(!home.join("auth.json").exists());
        assert_eq!(fs::read_to_string(home.join("config.toml")).unwrap(), "keep");
        assert!(registry.discard_shared_login("owner", "codex", &source.profile_id).unwrap());
        assert!(!fs::read_to_string(&path).unwrap().contains("refresh_token"));
    }

}
