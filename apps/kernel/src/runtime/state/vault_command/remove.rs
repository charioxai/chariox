//! `chariox.vault.remove_capture` undoes a capture the same agent made by mistake (a label
//! instead of the value): the Vault entry and the protection of its value go together, so
//! the page reads normally again. No person approves it; the agent chose the capture itself.
use super::capture::CAPTURED_KIND;
use super::*;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

pub(crate) const VAULT_REMOVE_CAPTURE: &str = "chariox.vault.remove_capture";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveArgs {
    credential_id: String,
}

pub(crate) fn remove_capture_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: VAULT_REMOVE_CAPTURE.into(),
        description: "remove a Vault entry that this agent captured in this session (capture_browser, capture_command or capture_file), for example after capturing a label instead of the value. The entry's value stops being masked in what you observe. The result names what was removed (credential_id, label, length); the value is never returned. Entries made by the owner, by another agent or by generate cannot be removed with this tool.".into(),
        input_schema: serde_json::json!({"type":"object","required":["credential_id"],"properties":{
            "credential_id":{"type":"string","minLength":1,"maxLength":128}},
            "additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn vault_remove_capture(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let context = self.vault_agent_context(token)?;
        self.recheck_vault_command_context(&context)?;
        let args: RemoveArgs = serde_json::from_value(arguments)
            .map_err(|e| error(format!("invalid Vault remove arguments: {e}")))?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        let entry = registry.get(&args.credential_id)?.ok_or_else(|| {
            error("no Vault entry has this credential_id; it may already be removed")
        })?;
        let own = entry.metadata.as_ref().is_some_and(|metadata| {
            metadata.created_by_kind.as_deref() == Some(CAPTURED_KIND)
                && metadata.created_by_id.as_deref() == Some(context.agent_id.as_str())
                && metadata.session_id.as_deref() == Some(context.session_id.as_str())
        });
        if !own {
            return Err(error(
                "only a capture that this agent made in this session can be removed with this tool",
            ));
        }
        let crate::config::UserCredentialSourceConfig::Vault { key } = &entry.source else {
            return Err(error("this entry does not store its value in the Vault"));
        };
        let _unlock = self
            .ensure_vault_unlocked_for_agent(
                &context.session_id,
                &context.agent_id,
                "vault_remove_capture",
            )
            .await?;
        self.recheck_vault_command_context(&context)?;
        let _mutation = self.vault_observation_mutation_guard().await;
        if registry
            .other_handle_using_vault_key(key, &entry.id)?
            .is_some()
        {
            return Err(error(
                "Another entry uses this value. Remove that entry first.",
            ));
        }
        let service = self.home_runtime_secret_service()?;
        let value = service
            .vault_secret_for_dictation(key)?
            .ok_or_else(|| error("this entry holds no value"))?;
        let length = value.chars().count();
        super::super::owner_vault::commit_entry(
            &service,
            &registry,
            Some(entry.clone()),
            entry.clone(),
            None,
            "Removed by the agent that captured it",
        )?;
        let browser_user = self.provider_account_authority_owner_user_id(&context.owner_user_id);
        self.drop_protected_value(&browser_user, key, &value)
            .await?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"removed": [{"credential_id": entry.id, "label": entry.description, "length": length}],
                "protection_dropped": true}),
        })
    }
}
