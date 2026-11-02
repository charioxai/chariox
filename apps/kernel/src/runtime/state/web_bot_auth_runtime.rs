//! MP-08 / MP-09 / MP-10 / MP-11: project-scoped operator identity.
use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use super::KernelRuntimeState;
use crate::{error::DaemonError, web_bot_auth::WebBotAuthSigner};

#[derive(Default)]
pub(super) struct WebBotAuthState {
    cached: std::sync::Mutex<Option<CachedSigner>>,
    preparing: tokio::sync::Mutex<()>,
}

pub(super) struct CachedSigner {
    binding: String,
    pub(super) signer: Arc<WebBotAuthSigner>,
    predecessor: Option<crate::web_bot_auth::PublicJwk>,
}

// This record is serialized only into the kernel Vault, never a response or trace.
#[derive(Serialize, Deserialize)]
struct StoredKey {
    binding: String,
    private_key: String,
    signature_agent: String,
    not_before: u64,
    expires_at: u64,
    #[serde(default)]
    predecessor: Option<crate::web_bot_auth::PublicJwk>,
}

#[derive(Deserialize)]
struct IssuedKey {
    kid: String,
    private_key: String,
    algorithm: String,
    signature_agent: String,
    directory_url: String,
    not_before: String,
    expires_at: String,
}

impl KernelRuntimeState {
    pub(super) async fn web_bot_auth_browser_material(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<serde_json::Value, DaemonError> {
        // Capture before Vault preparation so a policy change during the wait
        // cannot install stale signing material in a newly started browser.
        let revision = self.owned.kernel_browser_host.web_bot_auth_revision();
        Ok(
            match self.web_bot_auth_signer(session_id, agent_id).await? {
                Some(signer) => serde_json::json!({
                    "policy_revision": revision,
                "project_id": self.owned.session_store.get_session(session_id)?.project_id(),
                    "private_key": signer.private_key_base64url().as_str(),
                    "signature_agent": signer.signature_agent_origin(),
                    "not_before": signer.public_jwk().nbf,
                    "expires_at": signer.public_jwk().exp,
                }),
                None => serde_json::json!({"policy_revision": revision, "disabled": true}),
            },
        )
    }

    pub(crate) async fn apply_web_bot_auth_browser_policy(
        &self,
        reset: bool,
    ) -> Result<(), DaemonError> {
        let projects = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .web_bot_auth
            .projects;
        let host = self.owned.kernel_browser_host.clone();
        tokio::task::spawn_blocking(move || host.update_web_bot_auth_policy(&projects, reset))
            .await
            .map_err(|_| {
                error("browser signing policy update task failed; retry the configuration change")
            })?
            .map_err(|_| {
                error("browser signing policy could not be updated; retry the configuration change")
            })
    }
    pub(super) fn web_bot_auth_enabled(&self, session_id: &str) -> Result<bool, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .web_bot_auth
            .projects
            .iter()
            .any(|project| project == session.project_id()))
    }

    pub(super) async fn web_bot_auth_signer(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<Arc<WebBotAuthSigner>>, DaemonError> {
        if !self.web_bot_auth_enabled(session_id)? {
            return Ok(None);
        }
        let config = self.owned.config_projection.snapshot();
        let binding = signing_binding(&config)?;
        let _preparing = self.owned.web_bot_auth.preparing.lock().await;
        let now = crate::session::unix_epoch_ms() / 1000;
        let ready = {
            let cached = self
                .owned
                .web_bot_auth
                .cached
                .lock()
                .map_err(|_| error("signer state lock failed"))?;
            cached
                .as_ref()
                .filter(|key| key.binding == binding && key.signer.public_jwk().exp > now + 300)
                .map(|key| key.signer.clone())
        };
        if ready.is_some() {
            return Ok(ready);
        }
        // Admission already established the requesting agent. The private key is
        // durable only in the Vault, with the same owner unlock as credential use.
        let _unlock = self
            .ensure_vault_unlocked_for_agent(session_id, agent_id, "web_bot_auth")
            .await?;
        let signer = tokio::task::spawn_blocking(move || {
            load_or_issue_key(&config, &binding, now).map(|(signer, predecessor)| CachedSigner {
                binding,
                signer: Arc::new(signer),
                predecessor,
            })
        })
        .await
        .map_err(|_| error("signing-key preparation failed"))??;
        let result = signer.signer.clone();
        *self
            .owned
            .web_bot_auth
            .cached
            .lock()
            .map_err(|_| error("signer state lock failed"))? = Some(signer);
        Ok(Some(result))
    }

    /// The public route cannot unlock the Vault or issue keys on behalf of callers.
    pub(crate) fn web_bot_auth_directory(
        &self,
        authority: &str,
    ) -> Result<(Vec<u8>, std::collections::BTreeMap<String, String>), DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let guard = self
            .owned
            .web_bot_auth
            .cached
            .lock()
            .map_err(|_| error("signer state lock failed"))?;
        let now = crate::session::unix_epoch_ms() / 1000;
        match guard.as_ref().filter(|key| {
            config.cloud_relay.is_none()
                && !config.user_config.web_bot_auth.projects.is_empty()
                && config
                    .user_config
                    .web_bot_auth
                    .signature_agent
                    .as_deref()
                    .and_then(|origin| crate::web_bot_auth::validate_origin(origin).ok())
                    .is_some_and(|origin| key.binding == format!("self:{origin}"))
                && key.signer.public_jwk().nbf <= now
                && key.signer.public_jwk().exp > now
        }) {
            Some(key) => {
                let mut keys = vec![key.signer.public_jwk()];
                keys.extend(
                    key.predecessor
                        .as_ref()
                        .filter(|old| old.nbf <= now && old.exp > now),
                );
                let body = serde_json::to_vec(&serde_json::json!({"keys": keys}))
                    .map_err(|_| error("public signing-key directory encoding failed"))?;
                let headers = key.signer.signed_directory_headers(authority, &body, now)?;
                Ok((body, headers))
            }
            None => Ok((b"{\"keys\":[]}".to_vec(), Default::default())),
        }
    }
}

fn load_or_issue_key(
    config: &crate::config::DaemonConfig,
    binding: &str,
    now: u64,
) -> Result<(WebBotAuthSigner, Option<crate::web_bot_auth::PublicJwk>), DaemonError> {
    let vault_config = &config.user_config.credential_vault;
    let vault = crate::secret::web_bot_auth_vault_store(vault_config)?;
    let key_name = format!("web-bot-auth:{}", config.daemon_id);
    let mut predecessor = None;
    if let Some(raw) = vault.find_secret(&vault_config.service, &key_name)? {
        let raw = Zeroizing::new(raw);
        let stored: StoredKey = serde_json::from_str(&raw).map_err(|_| {
            error("the Vault signing-key record is invalid; rotate the Web Bot Auth key")
        })?;
        if stored.binding == binding && stored.not_before <= now && stored.expires_at > now + 300 {
            let signer = signer_from_record(&stored)?;
            return Ok((
                signer,
                stored
                    .predecessor
                    .as_ref()
                    .filter(|old| old.exp > now)
                    .cloned(),
            ));
        }
        if config.cloud_relay.is_none() && stored.binding == binding && stored.expires_at > now {
            predecessor = Some(signer_from_record(&stored)?.public_jwk().clone());
        }
    }
    let stored = if let Some(profile) = &config.cloud_relay {
        let credential = profile.kernel_credential.as_deref().ok_or_else(|| {
            error("re-enroll this kernel with Cloud before enabling Web Bot Auth")
        })?;
        let endpoint = format!(
            "{}/v1/web-bot-auth/keys",
            profile.api_url.trim_end_matches('/')
        );
        crate::web_bot_auth::validate_origin(&profile.api_url)?;
        let response = crate::outbound_dial::agent_builder()
            .redirects(0)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .post(&endpoint)
            .set("x-chariox-kernel-credential", credential)
            .set("content-type", "application/json")
            .send_string("{}")
            .map_err(|_| {
                error("Cloud could not issue a signing key; check kernel enrollment and retry")
            })?;
        let mut raw = Zeroizing::new(String::new());
        use std::io::Read;
        response
            .into_reader()
            .take(16 * 1024)
            .read_to_string(&mut raw)
            .map_err(|_| error("Cloud signing-key response could not be read"))?;
        let issued: IssuedKey = serde_json::from_str(&raw)
            .map_err(|_| error("Cloud signing-key response is invalid"))?;
        let parse_time = |value: &str| -> Result<u64, DaemonError> {
            chrono::DateTime::parse_from_rfc3339(value)
                .ok()
                .and_then(|time| u64::try_from(time.timestamp()).ok())
                .ok_or_else(|| error("Cloud signing-key validity is invalid"))
        };
        let record = StoredKey {
            binding: binding.to_string(),
            private_key: issued.private_key,
            signature_agent: crate::web_bot_auth::validate_origin(&issued.signature_agent)?,
            not_before: parse_time(&issued.not_before)?,
            expires_at: parse_time(&issued.expires_at)?,
            predecessor: None,
        };
        let signer = signer_from_record(&record)?;
        if issued.algorithm != "Ed25519"
            || issued.kid != signer.kid()
            || issued.directory_url
                != format!(
                    "{}/.well-known/http-message-signatures-directory",
                    record.signature_agent
                )
            || record.not_before > now
            || record.expires_at <= now + 300
        {
            return Err(error("Cloud signing-key metadata does not match its key"));
        }
        record
    } else {
        let mut seed = Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng.fill_bytes(&mut *seed);
        StoredKey {
            binding: binding.to_string(),
            private_key: URL_SAFE_NO_PAD.encode(seed.as_slice()),
            signature_agent: crate::web_bot_auth::validate_origin(
                config
                    .user_config
                    .web_bot_auth
                    .signature_agent
                    .as_deref()
                    .unwrap_or(""),
            )?,
            not_before: now,
            expires_at: now + 86_400,
            predecessor,
        }
    };
    let signer = signer_from_record(&stored)?;
    let serialized = Zeroizing::new(
        serde_json::to_string(&stored).map_err(|_| error("signing-key storage failed"))?,
    );
    vault.set_secret(&vault_config.service, &key_name, &serialized)?;
    Ok((signer, stored.predecessor.clone()))
}

impl Drop for StoredKey {
    fn drop(&mut self) {
        self.private_key.zeroize();
    }
}

fn signer_from_record(record: &StoredKey) -> Result<WebBotAuthSigner, DaemonError> {
    let decoded = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(&record.private_key)
            .map_err(|_| error("signing-key encoding is invalid"))?,
    );
    let seed: [u8; 32] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| error("signing-key size is invalid"))?;
    WebBotAuthSigner::from_seed(
        seed,
        &record.signature_agent,
        record.not_before,
        record.expires_at,
    )
}

fn error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "web_bot_auth",
        message: message.to_string(),
    }
}

fn signing_binding(config: &crate::config::DaemonConfig) -> Result<String, DaemonError> {
    let binding = if let Some(profile) = &config.cloud_relay {
        // A replacement enrollment grant revokes its predecessor's keys even
        // when the kernel id is unchanged. Never reuse that grant's Vault key.
        let credential = profile.kernel_credential.as_deref().ok_or_else(|| {
            error("re-enroll this kernel with Cloud before enabling Web Bot Auth")
        })?;
        format!(
            "cloud:{}:{}:{:x}",
            profile.api_url,
            profile.kernel_id.as_deref().unwrap_or(""),
            Sha256::digest(credential.as_bytes())
        )
    } else {
        let origin = config.user_config.web_bot_auth.signature_agent.as_deref()
                .ok_or_else(|| error("configure web_bot_auth.signature_agent to publish this self-hosted kernel's key directory"))?;
        format!("self:{}", crate::web_bot_auth::validate_origin(origin)?)
    };
    Ok(binding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_bot_auth_vault_reuses_and_rotates_self_hosted_key() {
        let private = crate::test_support::TestWorktree::new("web-bot-auth-vault");
        let path = private.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "fixture passphrase")
            .unwrap();
        crate::secret::unlock_chariox_encrypted_vault(
            &path,
            "fixture passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        config.user_config.credential_vault.service = format!("wba-{}", rand::random::<u64>());
        config.user_config.web_bot_auth.signature_agent = Some("https://operator.example".into());
        let binding = signing_binding(&config).unwrap();
        let (initial, prior) = load_or_issue_key(&config, &binding, 1000).unwrap();
        assert!(prior.is_none());
        let (retained, _) = load_or_issue_key(&config, &binding, 1100).unwrap();
        assert_eq!(initial.kid(), retained.kid());
        let (rotated, prior) = load_or_issue_key(&config, &binding, 1000 + 86_400 - 200).unwrap();
        assert_eq!(
            prior.as_ref().map(|key| key.kid.as_str()),
            Some(initial.kid())
        );
        let (_, expired_prior) = load_or_issue_key(&config, &binding, 1000 + 86_400).unwrap();
        assert!(expired_prior.is_none());
        assert_ne!(initial.kid(), rotated.kid());
        assert_eq!(rotated.public_jwk().exp - rotated.public_jwk().nbf, 86_400);
        let vault =
            crate::secret::web_bot_auth_vault_store(&config.user_config.credential_vault).unwrap();
        vault
            .delete_secret(
                &config.user_config.credential_vault.service,
                &format!("web-bot-auth:{}", config.daemon_id),
            )
            .unwrap();
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
    }

    #[test]
    fn web_bot_auth_reenrollment_replaces_the_key_binding() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url: "https://cloud.example".into(),
            kernel_id: Some("kernel-a".into()),
            kernel_credential: Some("fixture-grant-a".into()),
            ..Default::default()
        });
        let first = signing_binding(&config).unwrap();
        config.cloud_relay.as_mut().unwrap().kernel_credential = Some("fixture-grant-b".into());
        let second = signing_binding(&config).unwrap();
        assert_ne!(first, second);
        assert!(!first.contains("fixture-grant-a"));
        assert!(!second.contains("fixture-grant-b"));
    }
}
