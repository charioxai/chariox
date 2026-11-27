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
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.owner_user_id() == session.owner_user_id()
            && self.provider_account_authority_owner_user_id(session.owner_user_id())
                == crate::session::DEFAULT_LOCAL_USER_ID
        {
            return Ok(());
        }
        Err(crate::runtime::kernel_access::error(
            "only the owner's agents may create entries in this Vault",
        ))
    }

    pub(super) fn ensure_agent_can_manage_user_vault_for_agent(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        if agent.owner_user_id() == session.owner_user_id()
            && self.provider_account_authority_owner_user_id(session.owner_user_id())
                == crate::session::DEFAULT_LOCAL_USER_ID
        {
            return Ok(());
        }
        Err(crate::runtime::kernel_access::error(
            "only the owner's agents may create entries in this Vault",
        ))
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
        metadata: Some(crate::config::UserCredentialMetadataConfig {
            use_binding: Some(input.use_binding),
            ..Default::default()
        }),
    };
    crate::credential::validate_credential_registration(&credential)?;
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
        payment_card: None,
        use_binding: credential
            .metadata
            .as_ref()
            .and_then(|m| m.use_binding.clone()),
        created_by_kind: Some("agent".to_string()),
        created_by_id: agent_id.map(ToOwned::to_owned),
        session_id: Some(session_id.to_string()),
        provider: Some(provider.to_string()),
        provider_run_id: provider_run_id.map(ToOwned::to_owned),
        vault_key,
        created_at_ms: Some(now_ms),
        updated_at_ms: Some(now_ms),
        copied_from: None,
    });
    credential
}

impl KernelRuntimeState {
    /// Row 539: one card per secret. The card for one Vault entry asked by one agent has one id; a
    /// repeat ask while that card is open returns the same ids (`deduplicated`), and only a settled
    /// ask makes the next ask a new card.
    pub(super) async fn raise_secret_card(
        &self,
        task: &crate::durable_state::agent_lifecycle::AgentTaskExecution,
        session_id: &str,
        agent_id: &str,
        entry_id: &str,
        build_interaction: impl FnOnce(String) -> crate::session::RuntimeInteraction,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(
            serde_json::to_vec(&(session_id, agent_id, entry_id)).expect("identities serialize"),
        );
        let base: String = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let mut id = None;
        for generation in 0..1000u32 {
            let candidate = format!("credential-secret-{base}-{generation}");
            match self.owned.durable_state_store.agent_wake(&candidate)? {
                None => {
                    id = Some(candidate);
                    break;
                }
                Some(wake)
                    if wake.kind == crate::runtime::state::interaction_wake::KIND
                        && wake.room_id == session_id
                        && wake.agent_id == agent_id
                        && matches!(wake.state.as_str(), "starting" | "running") =>
                {
                    return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                        ok: true,
                        payload: serde_json::json!({
                            "interaction_id": candidate,
                            "registration_id": wake.registration_id,
                            "credential_id": entry_id,
                            "status": "pending",
                            "deduplicated": true,
                        }),
                    });
                }
                Some(_) => {}
            }
        }
        let id = id.ok_or_else(|| DaemonError::LocalTransport {
            operation: "runtime_tool_request_credential_secret",
            message: "too many settled requests for this credential".to_string(),
        })?;
        let interaction = build_interaction(id.clone());
        let result = self.arm_interaction_wake_for_task(task, interaction)?;
        self.announce_child_decision(session_id, &id).await?;
        if let Ok(session) = self.owned.session_store.get_session(session_id) {
            if let Some(ask) = session
                .active_interactions()
                .iter()
                .find(|ask| ask.id() == id)
            {
                self.owned.alert_owner_of_ask(session_id, ask);
            }
        }
        let mut payload = result.payload;
        payload["credential_id"] = entry_id.into();
        payload["status"] = "pending".into();
        Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
    }
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
