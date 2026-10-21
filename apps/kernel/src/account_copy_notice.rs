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
}
