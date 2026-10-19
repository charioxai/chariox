//! MP-08/MP-10/MP-11: ephemeral authority follows only a kernel catalog handoff.
use super::*;

impl PromptStateOwner {
    pub(crate) fn reserve_sudo_catalog_prompt(
        &self,
        session: &RuntimeSession,
        agent: &str,
        source: &str,
        next: &str,
    ) {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent);
        let Some(entry) = state.sudo_entry_id.clone().filter(|_| {
            state
                .active_prompt
                .as_ref()
                .is_some_and(|p| p.id() == source && p.status() == PromptStatus::Running)
        }) else {
            return;
        };
        let hold = state.sudo_work.get_or_insert_with(|| SudoWorkHold {
            entry_id: entry.clone(),
            prompts: BTreeSet::from([source.to_owned()]),
            catalog_prompt: None,
        });
        if hold.entry_id == entry {
            hold.prompts.insert(next.to_owned());
            hold.catalog_prompt = Some(next.to_owned());
        }
    }

    pub(crate) fn sudo_catalog_work_open(
        &self,
        session: &RuntimeSession,
        agent: &str,
        entry: &str,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner
            .ensure_agent_state(session, agent)
            .sudo_work
            .as_ref()
            .is_some_and(|h| h.entry_id == entry && h.catalog_prompt.is_some())
    }

    pub(crate) fn sudo_work_prompt_admitted(
        &self,
        session: &RuntimeSession,
        agent: &str,
        prompt: &str,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner
            .ensure_agent_state(session, agent)
            .sudo_work
            .as_ref()
            .is_some_and(|h| h.prompts.contains(prompt))
    }

    pub(crate) fn finish_sudo_catalog_prompt(
        &self,
        session: &RuntimeSession,
        agent: &str,
        prompt: &str,
    ) {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(hold) = owner.ensure_agent_state(session, agent).sudo_work.as_mut() {
            if hold.catalog_prompt.as_deref() == Some(prompt) {
                hold.catalog_prompt = None;
            }
        }
    }
}
