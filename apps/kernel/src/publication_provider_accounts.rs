use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::account_profile::ProviderAccountProfileRegistry;
use crate::error::DaemonError;

const PUBLICATION_PROVIDER_ACCOUNT_BINDINGS: &str = "CHARIOX_PUBLICATION_PROVIDER_ACCOUNT_BINDINGS";
const CREDENTIAL_BINDINGS_ROOT: &str = "/home/chariox/.credential-bindings";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationProviderAccountBindings {
    schema_version: u32,
    #[serde(default)]
    defaults: Vec<PublicationProviderDefaultAccount>,
    accounts: Vec<PublicationProviderAccountBinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationProviderDefaultAccount {
    provider: String,
    account_profile: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationProviderAccountBinding {
    provider: String,
    account_profile: String,
    label: String,
    home: PathBuf,
}

pub(crate) fn materialize_publication_provider_accounts(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
) -> Result<(), DaemonError> {
    let Some(source) = std::env::var_os(PUBLICATION_PROVIDER_ACCOUNT_BINDINGS) else {
        return Ok(());
    };
    let bindings = validated_bindings(source.to_string_lossy().as_bytes())?;
    materialize_validated_bindings(registry, owner_user_id, &bindings)
}

fn materialize_validated_bindings(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    bindings: &PublicationProviderAccountBindings,
) -> Result<(), DaemonError> {
    for binding in &bindings.accounts {
        registry.materialize_deployment_profile(
            owner_user_id,
            &binding.provider,
            &binding.account_profile,
            &binding.label,
            &binding.home,
        )?;
    }
    for default in &bindings.defaults {
        if bindings.accounts.iter().any(|account| {
            account
                .provider
                .trim()
                .eq_ignore_ascii_case(default.provider.trim())
                && account.account_profile.trim() == default.account_profile.trim()
        }) {
            registry.set_default(owner_user_id, &default.provider, &default.account_profile)?;
        }
    }
    Ok(())
}

fn validated_bindings(source: &[u8]) -> Result<PublicationProviderAccountBindings, DaemonError> {
    let bindings: PublicationProviderAccountBindings =
        serde_json::from_slice(source).map_err(|_| invalid_bindings())?;
    if bindings.schema_version != 1
        || bindings.defaults.len() > 64
        || bindings.accounts.len() > 256
        || bindings.defaults.iter().any(|default| {
            default.provider.trim().is_empty() || default.account_profile.trim().is_empty()
        })
    {
        return Err(invalid_bindings());
    }
    let mut unique = BTreeMap::<(String, String), PublicationProviderAccountBinding>::new();
    for binding in bindings.accounts {
        validate_binding(&binding)?;
        let key = (
            binding.provider.trim().to_lowercase(),
            binding.account_profile.trim().to_string(),
        );
        if let Some(existing) = unique.get(&key) {
            if existing.home != binding.home || existing.label != binding.label {
                return Err(invalid_bindings());
            }
            continue;
        }
        unique.insert(key, binding);
    }
    Ok(PublicationProviderAccountBindings {
        schema_version: bindings.schema_version,
        defaults: bindings.defaults,
        accounts: unique.into_values().collect(),
    })
}

fn validate_binding(binding: &PublicationProviderAccountBinding) -> Result<(), DaemonError> {
    if binding.provider.trim().is_empty()
        || binding.account_profile.trim().is_empty()
        || binding.label.trim().is_empty()
        || !binding.home.is_absolute()
        || !binding
            .home
            .starts_with(Path::new(CREDENTIAL_BINDINGS_ROOT))
        || binding
            .home
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(invalid_bindings());
    }
    Ok(())
}

fn invalid_bindings() -> DaemonError {
    DaemonError::InvalidConfig {
        field: "publication_provider_account_bindings",
        message: "provider account bindings are invalid",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        materialize_validated_bindings, validated_bindings, PublicationProviderAccountBinding,
        PublicationProviderAccountBindings, PublicationProviderDefaultAccount,
    };
    use crate::account_profile::ProviderAccountProfileRegistry;

    #[test]
    fn provider_account_materialization_manifest_has_no_agent_scope() {
        let bindings = validated_bindings(
            br#"{
              "schema_version": 1,
              "defaults": [{"provider":"codex","account_profile":"main"}],
              "accounts": [{
                "provider":"codex",
                "account_profile":"secondary",
                "label":"Codex secondary",
                "home":"/home/chariox/.credential-bindings/001/home"
              }]
            }"#,
        )
        .expect("valid deployment account manifest");

        assert_eq!(bindings.accounts.len(), 1);
        assert_eq!(bindings.accounts[0].account_profile, "secondary");
    }

    #[test]
    fn provider_account_materialization_rejects_obsolete_agent_scope() {
        assert!(validated_bindings(
            br#"{
              "schema_version": 1,
              "defaults": [],
              "accounts": [{
                "agent_id":"agent-1",
                "provider":"codex",
                "account_profile":"secondary",
                "label":"Codex secondary",
                "home":"/home/chariox/.credential-bindings/001/home"
              }]
            }"#,
        )
        .is_err());
    }

    #[test]
    fn publication_default_account_is_registered_as_default() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "chariox-publication-provider-default-{}-{unique}",
            std::process::id()
        ));
        let source_home = root.join("source-home");
        fs::create_dir_all(source_home.join(".codex")).expect("create source profile");
        fs::write(source_home.join(".codex/auth.json"), "{}").expect("write source credential");
        let registry = ProviderAccountProfileRegistry::open(root.join("registry.json"))
            .expect("open registry");
        let bindings = PublicationProviderAccountBindings {
            schema_version: 1,
            defaults: vec![PublicationProviderDefaultAccount {
                provider: "codex".to_string(),
                account_profile: "profile-codex".to_string(),
            }],
            accounts: vec![PublicationProviderAccountBinding {
                provider: "codex".to_string(),
                account_profile: "profile-codex".to_string(),
                label: "Codex deployment".to_string(),
                home: source_home,
            }],
        };

        materialize_validated_bindings(&registry, "local", &bindings)
            .expect("materialize publication accounts");

        let profile = registry
            .get("local", "codex", "profile-codex")
            .expect("resolve publication account");
        assert!(profile.is_default);
        let _ = fs::remove_dir_all(root);
    }
}
