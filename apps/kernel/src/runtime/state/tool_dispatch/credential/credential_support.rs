use rand::Rng;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

impl KernelRuntimeState {
    pub(super) fn ensure_agent_can_manage_user_vault(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Err(DaemonError::LocalTransport {
                operation: "agent_vault_management_policy",
                message: "credential creation requires an agent-scoped provider run".to_string(),
            });
        };
        let config = self.owned.config_projection.snapshot().user_config;
        match config.credential_vault.agent_management {
            crate::config::CredentialVaultAgentManagementPolicy::Allow => {}
            crate::config::CredentialVaultAgentManagementPolicy::Deny => {
                return Err(DaemonError::LocalTransport {
                    operation: "agent_vault_management_policy",
                    message: "agent vault credential creation is disabled by user config"
                        .to_string(),
                });
            }
        }
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let authority = crate::session::effective_agent_user_authority(&session, Some(&agent));
        if authority.is_full() {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation: "agent_vault_management_policy",
            message: "agent vault credential creation requires full agent user authority"
                .to_string(),
        })
    }

    pub(super) fn ensure_agent_can_manage_user_vault_for_agent(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let config = self.owned.config_projection.snapshot().user_config;
        match config.credential_vault.agent_management {
            crate::config::CredentialVaultAgentManagementPolicy::Allow => {}
            crate::config::CredentialVaultAgentManagementPolicy::Deny => {
                return Err(DaemonError::LocalTransport {
                    operation: "agent_vault_management_policy",
                    message: "agent vault credential creation is disabled by user config"
                        .to_string(),
                });
            }
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let authority = crate::session::effective_agent_user_authority(&session, Some(agent));
        if authority.is_full() {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation: "agent_vault_management_policy",
            message: "agent vault credential creation requires full agent user authority"
                .to_string(),
        })
    }
}

pub(super) fn credential_from_runtime_input(
    input: crate::transport::runtime_tools::RuntimeCredentialConfigInput,
) -> Result<crate::config::UserCredentialConfig, DaemonError> {
    let source = input
        .source
        .unwrap_or_else(|| crate::config::UserCredentialSourceConfig::Vault {
            key: input.id.clone(),
        });
    if !matches!(
        source,
        crate::config::UserCredentialSourceConfig::Vault { .. }
    ) {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_credential_input",
            message: "runtime-created credentials must use a vault source".to_string(),
        });
    }
    let credential = crate::config::UserCredentialConfig {
        id: input.id,
        description: input.description,
        source,
        allowed_hosts: input.allowed_hosts,
        allowed_uses: input.allowed_uses,
        injection: input.injection,
        metadata: None,
    };
    crate::config::validate_credentials(std::slice::from_ref(&credential))?;
    Ok(credential)
}

pub(super) fn stamp_runtime_credential_metadata(
    credential: crate::config::UserCredentialConfig,
    provider_run: &crate::provider::RuntimeProviderRun,
) -> crate::config::UserCredentialConfig {
    stamp_runtime_credential_metadata_for_agent(
        credential,
        provider_run.agent_instance_id(),
        provider_run.session_id(),
        provider_run.provider(),
        Some(provider_run.id()),
    )
}

pub(super) fn stamp_runtime_credential_metadata_for_agent(
    mut credential: crate::config::UserCredentialConfig,
    agent_id: Option<&str>,
    session_id: &str,
    provider: &str,
    provider_run_id: Option<&str>,
) -> crate::config::UserCredentialConfig {
    let now_ms = crate::session::unix_epoch_ms();
    let vault_key = match &credential.source {
        crate::config::UserCredentialSourceConfig::Vault { key } => Some(key.clone()),
        crate::config::UserCredentialSourceConfig::Env { .. }
        | crate::config::UserCredentialSourceConfig::File { .. } => None,
    };
    credential.metadata = Some(crate::config::UserCredentialMetadataConfig {
        created_by_kind: Some("agent".to_string()),
        created_by_id: agent_id.map(ToOwned::to_owned),
        session_id: Some(session_id.to_string()),
        provider: Some(provider.to_string()),
        provider_run_id: provider_run_id.map(ToOwned::to_owned),
        vault_key,
        created_at_ms: Some(now_ms),
        updated_at_ms: Some(now_ms),
    });
    credential
}

pub(super) fn generate_credential_secret(
    generator: &crate::transport::runtime_tools::GeneratedCredentialSecretGeneratorArgs,
) -> Result<String, DaemonError> {
    if generator.kind != "password" {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_create_generated_credential",
            message: format!("unsupported generator kind `{}`", generator.kind),
        });
    }
    if !(12..=256).contains(&generator.length) {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_create_generated_credential",
            message: "generated password length must be between 12 and 256".to_string(),
        });
    }
    let letters = "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let ambiguous = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let symbols = "!#$%&*+-=?@^_";
    let mut alphabet = if generator.avoid_ambiguous {
        letters.to_string()
    } else {
        ambiguous.to_string()
    };
    if generator.symbols {
        alphabet.push_str(symbols);
    }
    let chars = alphabet.chars().collect::<Vec<_>>();
    let mut rng = rand::rngs::OsRng;
    Ok((0..generator.length)
        .map(|_| {
            let index = rng.gen_range(0..chars.len());
            chars[index]
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_credential_metadata_records_agent_provenance_without_secret() {
        let credential = crate::config::UserCredentialConfig {
            id: "gmail-login".to_string(),
            description: Some("Gmail credential".to_string()),
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: "gmail-login".to_string(),
            },
            allowed_hosts: vec!["accounts.google.com".to_string()],
            allowed_uses: vec![crate::config::UserCredentialUse::Browser],
            injection: crate::config::UserCredentialInjectionConfig::Browser,
            metadata: None,
        };
        let provider_run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
            "run-123",
            "session-123".to_string(),
            Some("agent-123".to_string()),
            "codex".to_string(),
        );

        let credential = stamp_runtime_credential_metadata(credential, &provider_run);
        let metadata = credential.metadata.expect("metadata");

        assert_eq!(metadata.created_by_kind.as_deref(), Some("agent"));
        assert_eq!(metadata.created_by_id.as_deref(), Some("agent-123"));
        assert_eq!(metadata.session_id.as_deref(), Some("session-123"));
        assert_eq!(metadata.provider.as_deref(), Some("codex"));
        assert_eq!(metadata.provider_run_id.as_deref(), Some("run-123"));
        assert_eq!(metadata.vault_key.as_deref(), Some("gmail-login"));
        assert!(metadata.created_at_ms.is_some());
        assert_eq!(metadata.updated_at_ms, metadata.created_at_ms);
    }
}
