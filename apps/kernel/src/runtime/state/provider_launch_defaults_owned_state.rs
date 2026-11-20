use super::*;

use crate::session::SessionAgentDefaults;

impl KernelRuntimeOwnedState {
    /// Reads the persisted `providers.default/model/effort` config.toml values.
    /// Returns `None` when no provider default has ever been configured.
    pub(super) fn configured_session_agent_defaults(&self) -> Option<SessionAgentDefaults> {
        let providers = self.config_projection.snapshot().user_config.providers;
        let provider = providers.default.filter(|value| !value.trim().is_empty())?;
        Some(SessionAgentDefaults {
            provider,
            model: providers.model,
            effort: providers.effort,
            // Account defaults are kernel-registry state keyed by owner and provider.
            // They are resolved at launch time rather than duplicated in config.toml.
            account_profile: None,
            execution_mode: None,
            permission_level: None,
        })
    }
}

impl KernelRuntimeState {
    /// The provider/model/effort selection a new session starts from: what the caller sent, or the
    /// persisted `config.toml` default when it sent none. Reads only; see
    /// [`Self::persist_provider_launch_defaults`] for the write.
    pub(crate) fn seed_session_agent_defaults(
        &self,
        agent_defaults: Option<SessionAgentDefaults>,
    ) -> Option<SessionAgentDefaults> {
        agent_defaults.or_else(|| self.owned.configured_session_agent_defaults())
    }

    /// Saves an explicit selection as the new `providers.default/model/effort`, overwriting what
    /// was there. Callers save only after the selection has been accepted, so a refused request
    /// leaves the saved defaults as they were.
    pub(crate) async fn persist_provider_launch_defaults(
        &self,
        defaults: &SessionAgentDefaults,
    ) -> Result<(), DaemonError> {
        let provider = defaults.provider.trim();
        if !provider.is_empty() && provider != "default" {
            self.set_user_config_value("providers.default".to_string(), provider.to_string())
                .await?;
        }
        if let Some(model) = non_empty(defaults.model.as_deref()) {
            self.set_user_config_value("providers.model".to_string(), model.to_string())
                .await?;
        }
        if let Some(effort) = non_empty(defaults.effort.as_deref()) {
            self.set_user_config_value("providers.effort".to_string(), effort.to_string())
                .await?;
        }
        Ok(())
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
