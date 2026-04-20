use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::types::PromptSubmissionOutcome;
use super::types::{AgentPromptState, PromptQueueItem, SchedulerState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::session) struct PromptRuntimeState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    prompt_states: BTreeMap<String, AgentPromptState>,
    active_prompt: Option<PromptQueueItem>,
    queued_prompts: VecDeque<PromptQueueItem>,
    scheduler_state: SchedulerState,
}

impl Default for PromptRuntimeState {
    fn default() -> Self {
        Self {
            prompt_states: BTreeMap::new(),
            active_prompt: None,
            queued_prompts: VecDeque::new(),
            scheduler_state: SchedulerState::Idle,
        }
    }
}

impl PromptRuntimeState {
    pub(in crate::session) fn prompt_states(&self) -> &BTreeMap<String, AgentPromptState> {
        &self.prompt_states
    }

    pub(in crate::session) fn active_prompt(&self) -> Option<&PromptQueueItem> {
        self.active_prompt.as_ref()
    }

    pub(in crate::session) fn queued_prompts(&self) -> &VecDeque<PromptQueueItem> {
        &self.queued_prompts
    }

    pub(in crate::session) fn active_prompt_for_agent(
        &self,
        agent_id: &str,
    ) -> Option<&PromptQueueItem> {
        self.prompt_states
            .get(agent_id)
            .and_then(AgentPromptState::active_prompt)
    }

    pub(in crate::session) fn queued_prompts_for_agent(
        &self,
        agent_id: &str,
    ) -> Option<&VecDeque<PromptQueueItem>> {
        self.prompt_states
            .get(agent_id)
            .map(AgentPromptState::queued_prompts)
    }

    pub(in crate::session) fn mirror_agent_prompt_state(
        &mut self,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
        queued_prompts: VecDeque<PromptQueueItem>,
        focused_agent_id: Option<&str>,
    ) {
        if active_prompt.is_none() && queued_prompts.is_empty() {
            self.prompt_states.remove(agent_id);
        } else {
            self.prompt_states.insert(
                agent_id.to_string(),
                AgentPromptState::from_parts(active_prompt, queued_prompts),
            );
        }
        self.refresh_after_mutation(focused_agent_id);
    }

    pub(in crate::session) fn has_any_active_prompt(&self) -> bool {
        self.prompt_states
            .values()
            .any(|state| state.active_prompt().is_some())
    }

    pub(in crate::session) fn has_any_prompt_work(&self) -> bool {
        self.prompt_states
            .values()
            .any(|state| state.active_prompt().is_some() || !state.queued_prompts().is_empty())
    }

    pub(in crate::session) fn scheduler_state(&self) -> SchedulerState {
        self.scheduler_state
    }

    #[cfg(test)]
    pub(in crate::session) fn submit_prompt(
        &mut self,
        prompt: PromptQueueItem,
        focused_agent_id: Option<&str>,
    ) -> PromptSubmissionOutcome {
        let agent_id = prompt.target_agent_id().to_string();
        let prompt_state = self.prompt_states.entry(agent_id).or_default();
        let outcome = if prompt_state.active_prompt.is_none() {
            let mut running = prompt;
            running.set_status(super::types::PromptStatus::Running);
            prompt_state.active_prompt = Some(running.clone());
            PromptSubmissionOutcome::Started { prompt: running }
        } else {
            let mut queued = prompt;
            queued.set_status(super::types::PromptStatus::Queued);
            prompt_state.queued_prompts.push_back(queued.clone());
            PromptSubmissionOutcome::Queued { prompt: queued }
        };
        self.refresh_after_mutation(focused_agent_id);
        outcome
    }

    #[cfg(test)]
    pub(in crate::session) fn complete_active_prompt_only(
        &mut self,
        agent_id: &str,
        focused_agent_id: Option<&str>,
    ) -> Option<PromptQueueItem> {
        let prompt_state = self.prompt_states.get_mut(agent_id)?;
        let mut completed = prompt_state.active_prompt.take()?;
        completed.set_status(super::types::PromptStatus::Completed);
        self.drop_empty_prompt_state(agent_id);
        self.refresh_after_mutation(focused_agent_id);
        Some(completed)
    }

    #[cfg(test)]
    pub(in crate::session) fn cancel_active_prompt_only(
        &mut self,
        agent_id: &str,
        focused_agent_id: Option<&str>,
    ) -> Option<PromptQueueItem> {
        let prompt_state = self.prompt_states.get_mut(agent_id)?;
        let mut cancelled = prompt_state.active_prompt.take()?;
        cancelled.set_status(super::types::PromptStatus::Cancelled);
        self.drop_empty_prompt_state(agent_id);
        self.refresh_after_mutation(focused_agent_id);
        Some(cancelled)
    }

    pub(in crate::session) fn remove_queued_prompts_by_attachment(
        &mut self,
        attachment_id: &str,
        focused_agent_id: Option<&str>,
    ) -> usize {
        let mut removed = 0;
        let agent_ids: Vec<String> = self.prompt_states.keys().cloned().collect();
        for agent_id in agent_ids {
            if let Some(prompt_state) = self.prompt_states.get_mut(&agent_id) {
                let original_len = prompt_state.queued_prompts.len();
                prompt_state
                    .queued_prompts
                    .retain(|prompt| prompt.source_attachment_id() != attachment_id);
                removed += original_len - prompt_state.queued_prompts.len();
            }
            self.drop_empty_prompt_state(&agent_id);
        }
        self.refresh_after_mutation(focused_agent_id);
        removed
    }

    pub(in crate::session) fn remove_queued_prompts_by_workflow_run(
        &mut self,
        workflow_run_id: &str,
        focused_agent_id: Option<&str>,
    ) -> usize {
        let mut removed = 0;
        let agent_ids: Vec<String> = self.prompt_states.keys().cloned().collect();
        for agent_id in agent_ids {
            if let Some(prompt_state) = self.prompt_states.get_mut(&agent_id) {
                let original_len = prompt_state.queued_prompts.len();
                prompt_state
                    .queued_prompts
                    .retain(|prompt| prompt.workflow_run_id() != Some(workflow_run_id));
                removed += original_len - prompt_state.queued_prompts.len();
            }
            self.drop_empty_prompt_state(&agent_id);
        }
        self.refresh_after_mutation(focused_agent_id);
        removed
    }

    #[cfg(test)]
    pub(in crate::session) fn peek_next_queued_prompt(
        &self,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.prompt_states
            .get(agent_id)
            .and_then(|state| state.queued_prompts.front().cloned())
    }

    #[cfg(test)]
    pub(in crate::session) fn pop_next_queued_prompt(
        &mut self,
        agent_id: &str,
        focused_agent_id: Option<&str>,
    ) -> Option<PromptQueueItem> {
        let next = self
            .prompt_states
            .get_mut(agent_id)?
            .queued_prompts
            .pop_front();
        self.drop_empty_prompt_state(agent_id);
        self.refresh_after_mutation(focused_agent_id);
        next
    }

    #[cfg(test)]
    pub(in crate::session) fn activate_prompt(
        &mut self,
        mut prompt: PromptQueueItem,
        focused_agent_id: Option<&str>,
    ) -> PromptQueueItem {
        let agent_id = prompt.target_agent_id().to_string();
        prompt.set_status(super::types::PromptStatus::Running);
        let prompt_state = self.prompt_states.entry(agent_id).or_default();
        prompt_state.active_prompt = Some(prompt.clone());
        self.refresh_after_mutation(focused_agent_id);
        prompt
    }

    pub(in crate::session) fn clear(&mut self) {
        self.prompt_states.clear();
        self.active_prompt = None;
        self.queued_prompts.clear();
        self.scheduler_state = SchedulerState::Idle;
    }

    pub(in crate::session) fn retain_agent_ids(
        &mut self,
        agent_ids: &std::collections::BTreeSet<String>,
        focused_agent_id: Option<&str>,
    ) {
        self.prompt_states
            .retain(|agent_id, _| agent_ids.contains(agent_id));
        self.refresh_after_mutation(focused_agent_id);
    }

    pub(in crate::session) fn interrupt_active_prompts(
        &mut self,
        focused_agent_id: Option<&str>,
    ) -> Vec<PromptQueueItem> {
        let agent_ids = self.prompt_states.keys().cloned().collect::<Vec<_>>();
        let mut interrupted = Vec::new();
        for agent_id in agent_ids {
            if let Some(prompt_state) = self.prompt_states.get_mut(&agent_id) {
                if let Some(mut active_prompt) = prompt_state.active_prompt.take() {
                    active_prompt.set_status(super::types::PromptStatus::Cancelled);
                    interrupted.push(active_prompt);
                }
            }
            self.drop_empty_prompt_state(&agent_id);
        }
        self.refresh_after_mutation(focused_agent_id);
        interrupted
    }

    pub(in crate::session) fn refresh_after_focus_change(
        &mut self,
        focused_agent_id: Option<&str>,
    ) {
        self.refresh_prompt_projection(focused_agent_id);
    }

    fn refresh_after_mutation(&mut self, focused_agent_id: Option<&str>) {
        self.refresh_prompt_projection(focused_agent_id);
        self.refresh_scheduler_state();
    }

    fn refresh_scheduler_state(&mut self) {
        self.scheduler_state = if self
            .prompt_states
            .values()
            .any(|state| state.active_prompt.is_some())
        {
            if self
                .prompt_states
                .values()
                .all(|state| state.queued_prompts.is_empty())
            {
                SchedulerState::Running
            } else {
                SchedulerState::Waiting
            }
        } else if self
            .prompt_states
            .values()
            .all(|state| state.queued_prompts.is_empty())
        {
            SchedulerState::Idle
        } else {
            SchedulerState::Runnable
        };
    }

    fn refresh_prompt_projection(&mut self, focused_agent_id: Option<&str>) {
        let projected_agent_id = focused_agent_id
            .map(str::to_string)
            .filter(|agent_id| self.prompt_states.contains_key(agent_id))
            .or_else(|| {
                self.prompt_states
                    .iter()
                    .find(|(_, state)| state.active_prompt.is_some())
                    .map(|(agent_id, _)| agent_id.clone())
            })
            .or_else(|| self.prompt_states.keys().next().cloned());
        if let Some(agent_id) = projected_agent_id {
            if let Some(state) = self.prompt_states.get(&agent_id) {
                self.active_prompt = state.active_prompt.clone();
                self.queued_prompts = state.queued_prompts.clone();
                return;
            }
        }
        self.active_prompt = None;
        self.queued_prompts.clear();
    }

    fn drop_empty_prompt_state(&mut self, agent_id: &str) {
        let should_remove = self
            .prompt_states
            .get(agent_id)
            .map(|state| state.active_prompt.is_none() && state.queued_prompts.is_empty())
            .unwrap_or(false);
        if should_remove {
            self.prompt_states.remove(agent_id);
        }
    }
}
