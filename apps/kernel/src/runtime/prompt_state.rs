use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};

use crate::error::DaemonError;
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome, RuntimeSession};

#[derive(Debug, Clone, Default)]
struct OwnedAgentPromptState {
    active_prompt: Option<PromptQueueItem>,
    queued_prompts: VecDeque<PromptQueueItem>,
}

impl OwnedAgentPromptState {
    fn from_session(session: &RuntimeSession, agent_id: &str) -> Self {
        session
            .prompt_states()
            .get(agent_id)
            .map(|state| Self {
                active_prompt: state.active_prompt().cloned(),
                queued_prompts: state.queued_prompts().clone(),
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PromptStateKey {
    session_id: String,
    agent_id: String,
}

impl PromptStateKey {
    fn new(session_id: &str, agent_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptStateOwner {
    state: Arc<StdMutex<PromptStateOwnerState>>,
}

#[derive(Debug, Default)]
struct PromptStateOwnerState {
    states: BTreeMap<PromptStateKey, OwnedAgentPromptState>,
}

impl PromptStateOwner {
    pub(crate) fn active_prompt_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .clone()
    }

    pub(crate) fn active_prompt_for_agent_snapshot(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let key = PromptStateKey::new(session.id(), agent_id);
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .states
            .get(&key)
            .map(|state| state.active_prompt.clone())
            .unwrap_or_else(|| {
                session
                    .prompt_states()
                    .get(agent_id)
                    .and_then(|state| state.active_prompt().cloned())
            })
    }

    pub(crate) fn active_prompt_agent_id(&self, session: &RuntimeSession) -> Option<String> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        if let Some(focused_agent_id) = session.focused_agent_id() {
            if owner
                .ensure_agent_state(session, focused_agent_id)
                .active_prompt
                .is_some()
            {
                return Some(focused_agent_id.to_string());
            }
        }

        let mut active_agents = session
            .agents()
            .iter()
            .filter_map(|agent| {
                owner
                    .ensure_agent_state(session, agent.id())
                    .active_prompt
                    .as_ref()
                    .map(|_| agent.id().to_string())
            })
            .collect::<Vec<_>>();
        for agent_id in session.prompt_states().keys() {
            if active_agents.iter().any(|active| active == agent_id) {
                continue;
            }
            if owner
                .ensure_agent_state(session, agent_id)
                .active_prompt
                .is_some()
            {
                active_agents.push(agent_id.clone());
            }
        }
        if active_agents.len() == 1 {
            active_agents.into_iter().next()
        } else {
            None
        }
    }

    pub(crate) fn has_any_active_prompt(&self, session: &RuntimeSession) -> bool {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        if session.agents().iter().any(|agent| {
            owner
                .ensure_agent_state(session, agent.id())
                .active_prompt
                .is_some()
        }) {
            return true;
        }
        session.prompt_states().keys().any(|agent_id| {
            owner
                .ensure_agent_state(session, agent_id)
                .active_prompt
                .is_some()
        })
    }

    pub(crate) fn queued_prompt_count_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> usize {
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .ensure_agent_state(session, agent_id)
            .queued_prompts
            .len()
    }

    pub(crate) fn submit_prepared_prompt(
        &self,
        session: &RuntimeSession,
        mut prompt: PromptQueueItem,
        force_queue: bool,
    ) -> PromptSubmissionOutcome {
        let agent_id = prompt.target_agent_id().to_string();
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, &agent_id);
        if !force_queue && state.active_prompt.is_none() {
            prompt.set_status(PromptStatus::Running);
            state.active_prompt = Some(prompt.clone());
            PromptSubmissionOutcome::Started { prompt }
        } else {
            prompt.set_status(PromptStatus::Queued);
            state.queued_prompts.push_back(prompt.clone());
            PromptSubmissionOutcome::Queued { prompt }
        }
    }

    pub(crate) fn complete_active_prompt_only(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let mut completed = state.active_prompt.take()?;
        completed.set_status(PromptStatus::Completed);
        Some(completed)
    }

    pub(crate) fn cancel_active_prompt_only(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let mut cancelled = state.active_prompt.take()?;
        cancelled.set_status(PromptStatus::Cancelled);
        Some(cancelled)
    }

    pub(crate) fn begin_cancelling_active_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()?;
        active.set_status(PromptStatus::Cancelling);
        Some(active.clone())
    }

    pub(crate) fn finalize_active_prompt_cancellation(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let active_status = state.active_prompt.as_ref()?.status();
        if active_status != PromptStatus::Cancelling {
            return None;
        }
        let mut cancelled = state.active_prompt.take()?;
        cancelled.set_status(PromptStatus::Cancelled);
        Some(cancelled)
    }

    pub(crate) fn peek_next_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .ensure_agent_state(session, agent_id)
            .queued_prompts
            .front()
            .cloned()
    }

    pub(crate) fn activate_next_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let Some(front) = state.queued_prompts.front() else {
            return Ok(None);
        };
        if let Some(expected_prompt_id) = expected_prompt_id {
            if front.id() != expected_prompt_id {
                return Err(DaemonError::LocalTransport {
                    operation: "activate expected queued prompt",
                    message: format!(
                        "expected queued prompt `{}` but prompt owner queue front was `{}`",
                        expected_prompt_id,
                        front.id()
                    ),
                });
            }
        }
        let mut active = state
            .queued_prompts
            .pop_front()
            .expect("queue front checked above");
        active.set_status(PromptStatus::Running);
        state.active_prompt = Some(active.clone());
        Ok(Some(active))
    }

    pub(crate) fn activate_prompt(
        &self,
        session: &RuntimeSession,
        mut prompt: PromptQueueItem,
    ) -> PromptQueueItem {
        let agent_id = prompt.target_agent_id().to_string();
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, &agent_id);
        state
            .queued_prompts
            .retain(|queued| queued.id() != prompt.id());
        prompt.set_status(PromptStatus::Running);
        state.active_prompt = Some(prompt.clone());
        prompt
    }

    pub(crate) fn remove_queued_prompts_by_attachment(
        &self,
        session: &RuntimeSession,
        attachment_id: &str,
    ) -> usize {
        self.remove_queued_prompts_matching(session, |prompt| {
            prompt.source_attachment_id() == attachment_id
        })
    }

    pub(crate) fn remove_queued_prompts_by_workflow_run(
        &self,
        session: &RuntimeSession,
        workflow_run_id: &str,
    ) -> usize {
        self.remove_queued_prompts_matching(session, |prompt| {
            prompt.workflow_run_id() == Some(workflow_run_id)
        })
    }

    pub(crate) fn state_parts(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> (Option<PromptQueueItem>, VecDeque<PromptQueueItem>) {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        (state.active_prompt.clone(), state.queued_prompts.clone())
    }

    pub(crate) fn remove_session(&self, session_id: &str) {
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .states
            .retain(|key, _| key.session_id.as_str() != session_id);
    }

    pub(crate) fn remove_agent(&self, session_id: &str, agent_id: &str) {
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .states
            .remove(&PromptStateKey::new(session_id, agent_id));
    }

    fn remove_queued_prompts_matching(
        &self,
        session: &RuntimeSession,
        mut should_remove: impl FnMut(&PromptQueueItem) -> bool,
    ) -> usize {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let mut agent_ids = session
            .agents()
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.sort();
        agent_ids.dedup();

        let mut removed = 0;
        for agent_id in agent_ids {
            let state = owner.ensure_agent_state(session, &agent_id);
            let original_len = state.queued_prompts.len();
            state.queued_prompts.retain(|prompt| !should_remove(prompt));
            removed += original_len - state.queued_prompts.len();
        }
        removed
    }
}

impl PromptStateOwnerState {
    fn ensure_agent_state(
        &mut self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> &mut OwnedAgentPromptState {
        let key = PromptStateKey::new(session.id(), agent_id);
        self.states
            .entry(key)
            .or_insert_with(|| OwnedAgentPromptState::from_session(session, agent_id))
    }
}
