//! MP-08 / MP-10 / MP-11: trusted local and admitted worker history provenance.
use crate::agent::AgentServiceStore;
use crate::app::{ActiveTurnState, ActiveTurnStore};
use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
use crate::provider::{ProviderProcessServiceStore, RuntimeProviderRun};
use crate::runtime::projection::ProviderRunProjectionStore;
use crate::runtime::prompt_state::PromptStateOwner;
use crate::session::SessionStateStore;

#[derive(Clone)]
pub(crate) struct HistoryEventContextResolver {
    providers: ProviderProcessServiceStore,
    agents: AgentServiceStore,
    projections: ProviderRunProjectionStore,
    sessions: SessionStateStore,
    prompt_state_owner: PromptStateOwner,
    active_turns: ActiveTurnStore,
}

impl HistoryEventContextResolver {
    pub(crate) fn new(
        providers: ProviderProcessServiceStore,
        sessions: SessionStateStore,
        prompt_state_owner: PromptStateOwner,
        active_turns: ActiveTurnStore,
        agents: AgentServiceStore,
        projections: ProviderRunProjectionStore,
    ) -> Self {
        Self {
            providers,
            agents,
            projections,
            sessions,
            prompt_state_owner,
            active_turns,
        }
    }

    pub(crate) fn resolve(&self, entry: &SessionHistoryEntry) -> HistoryEventTurnContext {
        let active_turn = entry
            .provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.active_turns.get(provider_run_id));
        self.resolve_with_overrides(
            entry,
            HistoryEventContextOverrides::default(),
            active_turn.as_ref(),
        )
    }

    pub(crate) fn resolve_with_overrides(
        &self,
        entry: &SessionHistoryEntry,
        overrides: HistoryEventContextOverrides<'_>,
        active_turn: Option<&ActiveTurnState>,
    ) -> HistoryEventTurnContext {
        let provider_run = self.provider_run(entry);
        let agent_id = entry.agent_id.clone().or_else(|| {
            provider_run
                .as_ref()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        // Worker counters may collide with local counters. Never inherit another
        // agent's active turn merely because its raw provider-run ID matches.
        let active_turn = active_turn.filter(|turn| {
            turn.session_id == entry.session_id
                && agent_id.as_deref() == Some(turn.agent_id.as_str())
                && entry.provider_run_id.as_deref() == Some(turn.provider_run_id.as_str())
        });
        let session = self.sessions.get_session(&entry.session_id).ok();
        let active_prompt = session.as_ref().and_then(|session| {
            agent_id.as_deref().and_then(|agent_id| {
                self.prompt_state_owner
                    .active_prompt_for_agent(session, agent_id)
            })
        });
        let prompt_id = overrides
            .prompt_id
            .map(str::to_string)
            .or_else(|| active_turn.map(|turn| turn.prompt_id.clone()))
            .or_else(|| active_prompt.as_ref().map(|prompt| prompt.id().to_string()));
        let external_turn_id = entry
            .external_provider_observed_turn_id()
            .map(str::to_string);
        let turn_id = external_turn_id
            .or_else(|| active_turn.map(|turn| turn.trace_id.clone()))
            .or_else(|| prompt_id.clone());
        let workflow_run_id = overrides.workflow_run_id.map(str::to_string).or_else(|| {
            active_prompt
                .as_ref()
                .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
        });
        let workflow_id = workflow_run_id.as_deref().and_then(|workflow_run_id| {
            session
                .as_ref()
                .and_then(|session| session.workflow_run(workflow_run_id))
                .map(|workflow_run| workflow_run.workflow_id().to_string())
        });
        HistoryEventTurnContext {
            public_history_owner_user_id: session.as_ref().map(|s| s.owner_user_id().to_owned()),
            session_id: Some(entry.session_id.clone()),
            agent_id,
            provider: provider_run.as_ref().map(|run| run.provider().to_string()),
            model: provider_run.as_ref().map(|run| run.model().to_string()),
            turn_id,
            prompt_id,
            provider_run_id: entry.provider_run_id.clone(),
            provider_session_id: provider_run
                .as_ref()
                .and_then(|run| run.provider_session_id().map(str::to_string)),
            workflow_id,
            workflow_run_id,
            workflow_node_id: overrides
                .workflow_node_run_id
                .map(str::to_string)
                .or_else(|| {
                    active_prompt
                        .as_ref()
                        .and_then(|prompt| prompt.workflow_node_run_id().map(str::to_string))
                }),
            worktree_path: provider_run.as_ref().and_then(|run| {
                run.working_directory()
                    .map(|path| path.display().to_string())
            }),
            ..HistoryEventTurnContext::default()
        }
    }

    fn provider_run(&self, entry: &SessionHistoryEntry) -> Option<RuntimeProviderRun> {
        let run_id = entry.provider_run_id.as_deref()?;
        let agent = match entry.agent_id.as_deref() {
            Some(id) => Some(self.agents.get_agent(id).ok()?),
            None => None,
        };
        if agent
            .as_ref()
            .is_some_and(|agent| agent.session_id() != entry.session_id)
        {
            return None;
        }
        let run = match agent.as_ref().and_then(|agent| agent.remote_execution()) {
            Some(remote) => {
                if remote.execution_lease_id.is_empty() || remote.leased_agent_id.is_empty() {
                    return None;
                }
                let worker_run_id = remote.active_worker_provider_run_id.as_deref()?;
                let projected_id = crate::provider::projected_leased_provider_run_id(
                    &remote.leased_agent_id,
                    worker_run_id,
                );
                if run_id != worker_run_id && run_id != projected_id {
                    return None;
                }
                // This store is populated only after authenticated peer/lease
                // admission. Unknown or stale projections never fall back local.
                self.projections.get(&projected_id)?
            }
            None => self.providers.get_run(run_id).ok()?,
        };
        (run.session_id() == entry.session_id
            && entry
                .agent_id
                .as_deref()
                .is_none_or(|id| run.agent_instance_id() == Some(id)))
        .then_some(run)
    }
}

#[derive(Default)]
pub(crate) struct HistoryEventContextOverrides<'a> {
    pub(crate) prompt_id: Option<&'a str>,
    pub(crate) workflow_run_id: Option<&'a str>,
    pub(crate) workflow_node_run_id: Option<&'a str>,
}

#[cfg(test)]
mod tests;
