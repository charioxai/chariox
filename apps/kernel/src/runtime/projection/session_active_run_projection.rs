use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::session::RuntimeSession;

pub(crate) fn projected_active_provider_run_id(
    session: &RuntimeSession,
    mut provider_run_by_id: impl FnMut(&str) -> Option<RuntimeProviderRun>,
    mut provider_run_for_agent: impl FnMut(&str) -> Option<RuntimeProviderRun>,
    mut active_prompt_for_agent: impl FnMut(&str) -> bool,
    active_prompt_agent_id: Option<String>,
) -> Option<String> {
    if let Some(active_provider_run_id) = session.active_provider_run_id() {
        if let Some(active_run) = provider_run_by_id(active_provider_run_id) {
            let active_run_agent_id = active_run.agent_instance_id();
            let active_prompt_is_running = active_run_agent_id
                .map(|agent_id| active_prompt_for_agent(agent_id))
                .unwrap_or(false);
            if active_run.state() == ProviderRunState::Running && active_prompt_is_running {
                return Some(active_provider_run_id.to_string());
            }
        }
    }

    let projected_agent_id =
        active_prompt_agent_id.or_else(|| session.focused_agent_id().map(str::to_string));
    projected_agent_id.as_deref().and_then(|agent_id| {
        provider_run_for_agent(agent_id).and_then(|run| match run.state() {
            ProviderRunState::Running | ProviderRunState::Starting => Some(run.id().to_string()),
            ProviderRunState::Parked | ProviderRunState::Ended => None,
        })
    })
}
