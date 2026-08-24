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
    for binding in bindings {
        registry.materialize_deployment_profile(
            owner_user_id,
            &binding.provider,
            &binding.account_profile,
            &binding.label,
            &binding.home,
        )?;
    }
    Ok(())
}

fn validated_bindings(
    source: &[u8],
) -> Result<Vec<PublicationProviderAccountBinding>, DaemonError> {
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
    Ok(unique.into_values().collect())
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
    use super::validated_bindings;

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

        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].account_profile, "secondary");
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
}
