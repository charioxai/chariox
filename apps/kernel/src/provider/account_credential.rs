use sha2::{Digest, Sha256};

use crate::config::DaemonConfig;
use crate::error::DaemonError;

use super::ProviderCredentialEnvironment;

pub(crate) const CLAUDE_OAUTH_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";

#[derive(Debug)]
pub(crate) struct StoredProviderAccountCredential {
    pub(crate) credential_id: String,
    pub(crate) replaced: bool,
}

// This existing free-form provenance value records verification of this
// registration. Native profile observations never grant it. Replacing the
// stored credential resets it unless that replacement was itself verified.
const VERIFIED_ACCOUNT_CREDENTIAL: &str = "verified_provider_account_profile";

#[derive(Clone, Copy)]
pub(crate) struct ProviderAccountCredentialVerification {
    pub(crate) registered: bool,
    pub(crate) verified: bool,
    pub(crate) revision: Option<u64>,
}

pub(crate) fn provider_account_credential_verification(
    owner: &str,
    provider: &str,
    profile: &str,
) -> Result<ProviderAccountCredentialVerification, DaemonError> {
    let id = provider_account_credential_id(owner, provider, profile);
    let credential = crate::credential::CharioxCredentialRegistry::user()?.get(&id)?;
    let metadata = credential
        .as_ref()
        .and_then(|entry| entry.metadata.as_ref());
    Ok(ProviderAccountCredentialVerification {
        registered: credential.is_some(),
        verified: metadata.is_some_and(|value| {
            value.created_by_kind.as_deref() == Some(VERIFIED_ACCOUNT_CREDENTIAL)
        }),
        revision: metadata.and_then(|value| value.updated_at_ms),
    })
}

/// Account observations from a pre-upgrade native login do not describe an
/// unchecked Vault credential. Clear them before status or usage admission.
pub(crate) fn reconcile_claude_vault_observation(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner: &str,
    profile: crate::account_profile::ProviderAccountProfile,
) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError> {
    if profile.provider != "claude" {
        return Ok(profile);
    }
    let verification =
        provider_account_credential_verification(owner, "claude", &profile.profile_id)?;
    if !verification.registered || verification.verified {
        return Ok(profile);
    }
    registry.update_observation(
        owner,
        "claude",
        &profile.profile_id,
        crate::account_profile::ProviderAccountAuthState::Unknown,
        None,
        None,
        profile.detected_provider_version.clone(),
        Some(
            crate::account_profile::ProviderAccountUsageSnapshot::unavailable(
                &profile.profile_id,
                "claude",
            ),
        ),
    )
}

/// The caller holds the account login lane. A completed old first-use check
/// must not verify a replacement registration. This writes no secret.
pub(crate) fn mark_provider_account_credential_verified(
    owner: &str,
    profile: &str,
    revision: Option<u64>,
) -> Result<bool, DaemonError> {
    let id = provider_account_credential_id(owner, "claude", profile);
    let registry = crate::credential::CharioxCredentialRegistry::user()?;
    let Some(mut credential) = registry.get(&id)? else {
        return Ok(false);
    };
    if credential
        .metadata
        .as_ref()
        .and_then(|value| value.updated_at_ms)
        != revision
    {
        return Ok(false);
    }
    let metadata = credential.metadata.get_or_insert_with(Default::default);
    metadata.created_by_kind = Some(VERIFIED_ACCOUNT_CREDENTIAL.into());
    registry.upsert(credential)?;
    Ok(true)
}

/// Stable handle for the Chariox-vault credential assigned to one provider
/// account. The handle contains no provider secret or host-local path.
pub(crate) fn provider_account_credential_id(
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> String {
    let provider = provider.trim().to_ascii_lowercase();
    let identity = format!(
        "{}\0{}\0{}",
        owner_user_id.trim(),
        crate::provider::canonical_provider_family(&provider).unwrap_or(provider.as_str()),
        profile_id.trim()
    );
    let digest = Sha256::digest(identity.as_bytes());
    format!("provider-account-{}-{digest:x}", canonical_label(&provider))[..64].to_string()
}

pub(crate) fn resolve_provider_account_credentials(
    config: &DaemonConfig,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<ProviderCredentialEnvironment, DaemonError> {
    let credential_id = provider_account_credential_id(owner_user_id, provider, profile_id);
    let credentials = crate::credential::load_user_credentials()?;
    if !credentials
        .iter()
        .any(|credential| credential.id == credential_id)
    {
        return Ok(ProviderCredentialEnvironment::default());
    }

    let env_name = match crate::provider::canonical_provider_family(provider) {
        Some("claude") => CLAUDE_OAUTH_TOKEN_ENV,
        _ => {
            return Err(DaemonError::InvalidConfig {
                field: "provider account credential",
                message: "vault-backed provider launch credentials are currently supported only for Claude",
            });
        }
    };
    let service_revision = credentials
        .iter()
        .find(|entry| entry.id == credential_id)
        .and_then(|entry| entry.metadata.as_ref())
        .and_then(|metadata| metadata.updated_at_ms);
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        credentials,
        &config.user_config.credential_vault,
    )?;
    let mut environment = ProviderCredentialEnvironment::default();
    environment.insert(env_name, service.provider_secret_input(&credential_id)?);
    // Bind the in-memory launch to the registration read for this secret.
    environment.registration_revision = service_revision;
    Ok(environment)
}

/// One sign-in per account: the account's own Claude login runs every agent of
/// that account, interactive or not, on Linux (its credential file) and macOS
/// (its login Keychain) alike. A registered account credential takes precedence:
/// setup-token repair does not remove a rejected native credential, and file or
/// Keychain availability does not establish that it still authenticates.
pub(crate) fn resolve_provider_account_credentials_for_launch(
    config: &DaemonConfig,
    profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    client_interface: crate::provider::ProviderClientInterface,
) -> Result<ProviderCredentialEnvironment, DaemonError> {
    if crate::provider::canonical_provider_family(provider) != Some("claude")
        || client_interface == crate::provider::ProviderClientInterface::NativeTui
    {
        return resolve_provider_account_credentials(config, owner_user_id, provider, profile_id);
    }
    if claude_login_runs_agents(profiles, owner_user_id, profile_id)? {
        return Ok(ProviderCredentialEnvironment::default());
    }
    let environment =
        resolve_provider_account_credentials(config, owner_user_id, provider, profile_id)?;
    if !environment.is_empty() {
        return Ok(environment);
    }
    Err(DaemonError::InvalidConfig {
        field: "provider account credential",
        message:
            "this Claude account is not signed in; open Provider Accounts and choose Log in on it",
    })
}

/// Fall back to native Claude credentials only without a registered account
/// credential. A successful setup-token repair must remain selected even when
/// an older native file or Keychain item is still present.
fn claude_login_runs_agents(
    profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    profile_id: &str,
) -> Result<bool, DaemonError> {
    if provider_account_credential_registered(owner_user_id, "claude", profile_id)? {
        return Ok(false);
    }
    match std::env::consts::OS {
        "linux" => profiles.has_portable_claude_credentials(owner_user_id, profile_id),
        "macos" => {
            let authenticated = profiles
                .get(owner_user_id, "claude", profile_id)?
                .auth_state
                == crate::account_profile::ProviderAccountAuthState::Authenticated;
            Ok(claude_macos_login_runs_agents(
                authenticated,
                authenticated
                    && profiles.has_native_claude_credentials(owner_user_id, profile_id)?,
            ))
        }
        _ => Ok(false),
    }
}

// A Vault verification authenticates an account without proving native login.
fn claude_macos_login_runs_agents(authenticated: bool, native_credentials_available: bool) -> bool {
    authenticated && native_credentials_available
}

/// Whether a launch reads the account's Chariox-vault credential, by the same
/// rule as `resolve_provider_account_credentials_for_launch`: never when the
/// account's own Claude login runs the agent, so it needs no vault unlock.
pub(crate) fn launch_uses_vault_credential(
    profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    client_interface: crate::provider::ProviderClientInterface,
) -> Result<bool, DaemonError> {
    if crate::provider::canonical_provider_family(provider) == Some("claude")
        && client_interface != crate::provider::ProviderClientInterface::NativeTui
        && claude_login_runs_agents(profiles, owner_user_id, profile_id)?
    {
        return Ok(false);
    }
    provider_account_credential_uses_vault(owner_user_id, provider, profile_id)
}

/// Whether a Chariox-held launch credential is registered for the account.
/// Reads only the registry; no secret is resolved.
pub(crate) fn provider_account_credential_registered(
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<bool, DaemonError> {
    let credential_id = provider_account_credential_id(owner_user_id, provider, profile_id);
    Ok(crate::credential::load_user_credentials()?
        .iter()
        .any(|credential| credential.id == credential_id))
}

pub(crate) fn provider_account_credential_uses_vault(
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<bool, DaemonError> {
    let credential_id = provider_account_credential_id(owner_user_id, provider, profile_id);
    Ok(crate::credential::load_user_credentials()?
        .iter()
        .find(|credential| credential.id == credential_id)
        .is_some_and(|credential| {
            matches!(
                credential.source,
                crate::config::UserCredentialSourceConfig::Vault { .. }
            )
        }))
}

#[cfg(test)]
pub(crate) fn store_provider_account_credential(
    config: &DaemonConfig,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    secret: &str,
    overwrite: bool,
) -> Result<StoredProviderAccountCredential, DaemonError> {
    store_account_credential(
        config,
        owner_user_id,
        provider,
        profile_id,
        secret,
        overwrite,
        false,
    )
}

/// Only called after an official CLI verification of this exact replacement.
pub(crate) fn store_verified_provider_account_credential(
    config: &DaemonConfig,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    secret: &str,
    overwrite: bool,
) -> Result<StoredProviderAccountCredential, DaemonError> {
    store_account_credential(
        config,
        owner_user_id,
        provider,
        profile_id,
        secret,
        overwrite,
        true,
    )
}

fn store_account_credential(
    config: &DaemonConfig,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    secret: &str,
    overwrite: bool,
    verified: bool,
) -> Result<StoredProviderAccountCredential, DaemonError> {
    let provider = validate_provider_account_credential_input(provider, secret)?;
    let secret = secret.trim();
    let credential_id = provider_account_credential_id(owner_user_id, provider, profile_id);
    let registry = crate::credential::CharioxCredentialRegistry::user()?;
    let existing = registry.get(&credential_id)?;
    let replaced = existing.is_some();
    // Monotonic per registration even for replacements within one millisecond.
    let now_ms = crate::session::unix_epoch_ms().max(
        existing
            .as_ref()
            .and_then(|entry| entry.metadata.as_ref())
            .and_then(|metadata| metadata.updated_at_ms)
            .map_or(0, |previous| previous.saturating_add(1)),
    );
    let created_at_ms = existing
        .as_ref()
        .and_then(|credential| credential.metadata.as_ref())
        .and_then(|metadata| metadata.created_at_ms)
        .unwrap_or(now_ms);
    let credential = crate::config::UserCredentialConfig {
        id: credential_id.clone(),
        description: Some(format!("{provider} account profile {profile_id}")),
        source: crate::config::UserCredentialSourceConfig::Vault {
            key: credential_id.clone(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![crate::config::UserCredentialUse::Provider],
        injection: crate::config::UserCredentialInjectionConfig::Provider,
        metadata: Some(crate::config::UserCredentialMetadataConfig {
            created_by_kind: Some(
                if verified {
                    VERIFIED_ACCOUNT_CREDENTIAL
                } else {
                    "provider_account_profile"
                }
                .to_string(),
            ),
            created_by_id: Some(profile_id.to_string()),
            session_id: None,
            provider: Some(provider.to_string()),
            provider_run_id: None,
            vault_key: Some(credential_id.clone()),
            created_at_ms: Some(created_at_ms),
            updated_at_ms: Some(now_ms),
        }),
    };
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        Vec::new(),
        &config.user_config.credential_vault,
    )?;
    service.upsert_vault_backed_credential_with_secret(&registry, credential, secret, overwrite)?;
    Ok(StoredProviderAccountCredential {
        credential_id,
        replaced,
    })
}

pub(crate) fn validate_provider_account_credential_input(
    provider: &str,
    secret: &str,
) -> Result<&'static str, DaemonError> {
    let provider =
        crate::provider::canonical_provider_family(provider).ok_or(DaemonError::InvalidConfig {
            field: "provider account credential",
            message: "unsupported provider",
        })?;
    if provider != "claude" {
        return Err(DaemonError::InvalidConfig {
            field: "provider account credential",
            message: "vault-backed account credentials are currently supported only for Claude",
        });
    }
    if secret.trim().is_empty() {
        return Err(DaemonError::InvalidConfig {
            field: "provider account credential",
            message: "Claude setup token must not be empty",
        });
    }
    Ok(provider)
}

fn canonical_label(provider: &str) -> &'static str {
    crate::provider::canonical_provider_family(provider).unwrap_or("provider")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp08_mp10_mp11_vault_authentication_does_not_prove_a_macos_native_login() {
        assert!(!claude_macos_login_runs_agents(true, false));
        assert!(claude_macos_login_runs_agents(true, true));
        assert!(!claude_macos_login_runs_agents(false, true));
    }

    #[test]
    fn mp08_mp10_mp11_repaired_vault_token_overrides_rejected_native_credentials() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-repaired-claude-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::env::set_var("CHARIOX_HOME", &root);
        std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::ProcessMemory;
        let profiles = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .expect("profile registry");
        let profile = profiles
            .create_managed("local", "claude", "Repaired account")
            .expect("profile");
        let env = profiles
            .resolve_environment("local", "claude", &profile.profile_id)
            .expect("profile environment");
        let native_path = std::path::Path::new(&env["CLAUDE_CONFIG_DIR"]).join(".credentials.json");
        // A revoked token still passes the file-presence/refresh-token check.
        // Repair must not select it again just because it remains on disk.
        std::fs::write(&native_path,
            br#"{"claudeAiOauth":{"accessToken":"rejected-native","refreshToken":"revoked-refresh"}}"#)
            .expect("rejected native fixture");
        assert!(profiles
            .has_native_claude_credentials("local", &profile.profile_id)
            .unwrap());
        store_provider_account_credential(
            &config,
            "local",
            "claude",
            &profile.profile_id,
            "verified-replacement",
            true,
        )
        .expect("verified setup-token repair storage");
        profiles
            .update_observation(
                "local",
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .expect("verified repair observation");
        let mut outcomes = Vec::new();
        for provider in ["claude-p", "claude-headless"] {
            let uses_vault = launch_uses_vault_credential(
                &profiles,
                "local",
                provider,
                &profile.profile_id,
                crate::provider::ProviderClientInterface::Chariox,
            )
            .expect("repaired launch Vault requirement");
            let launch = resolve_provider_account_credentials_for_launch(
                &config,
                &profiles,
                "local",
                provider,
                &profile.profile_id,
                crate::provider::ProviderClientInterface::Chariox,
            )
            .expect("repaired launch resolves");
            let repaired = launch.iter().any(|(name, value)| {
                name == CLAUDE_OAUTH_TOKEN_ENV && value == "verified-replacement"
            });
            outcomes.push((provider, uses_vault, repaired));
        }
        assert!(
            native_path.exists(),
            "repair preserves provider-owned native state"
        );
        std::env::remove_var("CHARIOX_HOME");
        std::env::remove_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT");
        std::fs::remove_dir_all(root).expect("fixture cleanup");
        assert_eq!(
            outcomes,
            vec![("claude-p", true, true), ("claude-headless", true, true)]
        );
    }

    #[test]
    fn credential_handle_is_stable_and_registry_safe() {
        let first = provider_account_credential_id("local", "Claude", "Work account");
        let second = provider_account_credential_id("local", "claude", "Work account");

        assert_eq!(first, second);
        assert!(first.starts_with("provider-account-claude-"));
        assert!(first
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')));
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn every_claude_mode_resolves_the_same_account_credential() {
        // A setup token stored for the account must reach `claude-p` and
        // `claude-headless` agents of that account too.
        let stored = provider_account_credential_id("local", "claude", "work");
        assert_eq!(
            provider_account_credential_id("local", "claude-p", "work"),
            stored
        );
        assert_eq!(
            provider_account_credential_id("local", "claude-headless", "work"),
            stored
        );
    }

    #[test]
    fn distinct_account_profiles_have_distinct_credential_handles() {
        assert_ne!(
            provider_account_credential_id("local", "claude", "personal"),
            provider_account_credential_id("local", "claude", "work")
        );
    }

    #[test]
    fn credential_input_validation_rejects_invalid_requests_before_storage() {
        let unsupported = validate_provider_account_credential_input("codex", "token")
            .expect_err("non-Claude credentials should be rejected");
        assert!(unsupported.to_string().contains("only for Claude"));
        let empty = validate_provider_account_credential_input("claude", "  ")
            .expect_err("empty Claude credentials should be rejected");
        assert!(empty.to_string().contains("must not be empty"));
        assert_eq!(
            validate_provider_account_credential_input("Claude", " token ")
                .expect("valid Claude credential should pass"),
            "claude"
        );
    }

    #[test]
    fn vault_requirement_is_derived_from_the_registered_credential_source() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-account-vault-requirement-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("CHARIOX_HOME", &root);

        let credential_id = provider_account_credential_id("local", "claude", "work");
        assert!(
            !provider_account_credential_uses_vault("local", "claude", "work")
                .expect("missing credential should not require vault access")
        );
        crate::credential::CharioxCredentialRegistry::user()
            .expect("credential registry should resolve")
            .upsert(crate::config::UserCredentialConfig {
                id: credential_id.clone(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault { key: credential_id },
                allowed_hosts: Vec::new(),
                allowed_uses: vec![crate::config::UserCredentialUse::Provider],
                injection: crate::config::UserCredentialInjectionConfig::Provider,
                metadata: None,
            })
            .expect("provider credential should register");
        assert!(
            provider_account_credential_uses_vault("local", "claude", "work")
                .expect("vault-backed credential should require vault access")
        );

        std::env::remove_var("CHARIOX_HOME");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn configured_provider_credential_resolves_only_into_secret_environment() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-account-credential-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("CHARIOX_HOME", &root);
        std::env::set_var("CHARIOX_TEST_CLAUDE_SETUP_TOKEN", "setup-token-secret");

        let credential_id = provider_account_credential_id("local", "claude", "work");
        crate::credential::CharioxCredentialRegistry::user()
            .expect("credential registry should resolve")
            .upsert(crate::config::UserCredentialConfig {
                id: credential_id,
                description: None,
                source: crate::config::UserCredentialSourceConfig::Env {
                    name: "CHARIOX_TEST_CLAUDE_SETUP_TOKEN".to_string(),
                },
                allowed_hosts: Vec::new(),
                allowed_uses: vec![crate::config::UserCredentialUse::Provider],
                injection: crate::config::UserCredentialInjectionConfig::Provider,
                metadata: None,
            })
            .expect("provider credential should register");

        let environment = resolve_provider_account_credentials(
            &crate::config::DaemonConfig::for_tests(),
            "local",
            "claude",
            "work",
        )
        .expect("provider credential should resolve");
        let values = environment.iter().collect::<Vec<_>>();
        assert_eq!(values, vec![(CLAUDE_OAUTH_TOKEN_ENV, "setup-token-secret")]);
        assert!(!format!("{environment:?}").contains("setup-token-secret"));

        std::env::remove_var("CHARIOX_TEST_CLAUDE_SETUP_TOKEN");
        std::env::remove_var("CHARIOX_HOME");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn store_provider_credential_uses_vault_source_and_provider_policy() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-account-store-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("CHARIOX_HOME", &root);
        std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::ProcessMemory;

        let stored = store_provider_account_credential(
            &config,
            "local",
            "claude",
            "work",
            "  setup-token-secret  ",
            false,
        )
        .expect("provider credential should store");
        assert!(!stored.replaced);
        let credential = crate::credential::CharioxCredentialRegistry::user()
            .expect("registry should open")
            .get(&stored.credential_id)
            .expect("credential should read")
            .expect("credential should exist");
        assert_eq!(
            credential.source,
            crate::config::UserCredentialSourceConfig::Vault {
                key: stored.credential_id.clone()
            }
        );
        assert_eq!(
            credential.allowed_uses,
            vec![crate::config::UserCredentialUse::Provider]
        );
        assert_eq!(
            credential.injection,
            crate::config::UserCredentialInjectionConfig::Provider
        );
        let resolved = resolve_provider_account_credentials(&config, "local", "claude", "work")
            .expect("stored credential should resolve");
        assert_eq!(
            resolved.iter().collect::<Vec<_>>(),
            vec![(CLAUDE_OAUTH_TOKEN_ENV, "setup-token-secret")]
        );
        let first = provider_account_credential_verification("local", "claude", "work").unwrap();
        assert!(!first.verified);
        assert!(
            mark_provider_account_credential_verified("local", "work", first.revision).unwrap()
        );
        assert!(
            provider_account_credential_verification("local", "claude", "work")
                .unwrap()
                .verified
        );
        let duplicate = store_provider_account_credential(
            &config,
            "local",
            "claude",
            "work",
            "replacement-token",
            false,
        )
        .expect_err("replacement should require explicit overwrite");
        assert!(duplicate.to_string().contains("overwrite=true"));
        let replacement = store_provider_account_credential(
            &config,
            "local",
            "claude",
            "work",
            "replacement-token",
            true,
        )
        .expect("explicit replacement should succeed");
        assert!(replacement.replaced);
        let current = provider_account_credential_verification("local", "claude", "work").unwrap();
        assert!(!current.verified);
        assert!(current.revision > first.revision);
        assert!(
            !mark_provider_account_credential_verified("local", "work", first.revision).unwrap()
        );
        assert!(
            !provider_account_credential_verification("local", "claude", "work")
                .unwrap()
                .verified
        );
        let resolved = resolve_provider_account_credentials(&config, "local", "claude", "work")
            .expect("replacement should resolve");
        assert_eq!(
            resolved.iter().collect::<Vec<_>>(),
            vec![(CLAUDE_OAUTH_TOKEN_ENV, "replacement-token")]
        );

        std::env::remove_var("CHARIOX_HOME");
        std::env::remove_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT");
        let _ = std::fs::remove_dir_all(root);
    }
}
