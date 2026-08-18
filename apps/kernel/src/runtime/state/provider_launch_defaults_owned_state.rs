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
            account_profile: providers.account_profile,
            execution_mode: None,
            permission_level: None,
        })
    }
}

impl KernelRuntimeState {
    /// Resolves a session's launch provider/model/effort selection.
    ///
    /// When the caller supplies an explicit selection, it is persisted to
    /// `config.toml` as the new `providers.default/model/effort`, overwriting
    /// whatever was there. When the caller supplies none, the persisted
    /// config.toml default is used to seed the session (if one is set).
    pub(crate) async fn resolve_session_agent_defaults(
        &self,
        agent_defaults: Option<SessionAgentDefaults>,
    ) -> Result<Option<SessionAgentDefaults>, DaemonError> {
        match agent_defaults {
            Some(defaults) => {
                self.persist_provider_launch_defaults(&defaults).await?;
                Ok(Some(defaults))
            }
            None => Ok(self.owned.configured_session_agent_defaults()),
        }
    }

    async fn persist_provider_launch_defaults(
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
