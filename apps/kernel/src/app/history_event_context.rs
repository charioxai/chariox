use crate::app::{ActiveTurnState, ActiveTurnStore};
use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
use crate::provider::ProviderProcessServiceStore;
use crate::runtime::prompt_state::PromptStateOwner;
use crate::session::SessionStateStore;

#[derive(Clone)]
pub(crate) struct HistoryEventContextResolver {
    providers: ProviderProcessServiceStore,
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
    ) -> Self {
        Self {
            providers,
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
        let provider_run = entry
            .provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.providers.get_run(provider_run_id).ok());
        let agent_id = entry.agent_id.clone().or_else(|| {
            provider_run
                .as_ref()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        let session = self.sessions.get_session(&entry.session_id).ok();
        let active_prompt = session.as_ref().and_then(|session| {
            agent_id.as_deref().and_then(|agent_id| {
                self.prompt_state_owner
                    .active_prompt_for_agent_or_restore(session, agent_id)
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
        HistoryEventTurnContext {
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
            workflow_run_id: overrides.workflow_run_id.map(str::to_string).or_else(|| {
                active_prompt
                    .as_ref()
                    .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
            }),
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
}

#[derive(Default)]
pub(crate) struct HistoryEventContextOverrides<'a> {
    pub(crate) prompt_id: Option<&'a str>,
    pub(crate) workflow_run_id: Option<&'a str>,
    pub(crate) workflow_node_run_id: Option<&'a str>,
}
