//! Agents stored before every agent carried an explicit model and variant get them at start-up,
//! from the catalog of their own account, the same way a provider switch would.

use super::*;

impl KernelRuntimeState {
    /// Local agents whose provider has a catalog but who store no model or no variant.
    pub(crate) fn agents_missing_model_choice(&self) -> Vec<crate::agent::AgentInstance> {
        self.list_agents()
            .into_iter()
            .filter(|agent| {
                agent.remote_execution().is_none()
                    && crate::provider::canonical_provider_family(agent.provider()).is_some()
                    && (agent.model().is_none() || agent.effort().is_none())
            })
            .collect()
    }

    /// Gives each such agent the model and variant its account offers. An agent that is busy or
    /// whose account catalog cannot be read stays as it is and is logged; the next start retries.
    pub(crate) async fn resolve_stored_agent_models(&self) {
        for agent in self.agents_missing_model_choice() {
            let result = self
                .update_agent_profile(
                    agent.session_id(),
                    agent.id(),
                    agent.owner_user_id(),
                    None,
                    None,
                    None,
                    None,
                )
                .await;
            if let Err(error) = result {
                crate::logging::error_with_fields(
                    "agent.model",
                    "could not give a stored agent its model",
                    serde_json::json!({ "agent": agent.id(), "error": error.to_string() }),
                );
            }
        }
    }
}
