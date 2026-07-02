use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};

use crate::error::DaemonError;
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome, RuntimeSession};

pub(crate) const PROMPT_QUEUE_LIMIT: usize = 128;

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
    next_pending_prompt_number: u64,
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
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        let agent_id = prompt.target_agent_id().to_string();
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let should_start = {
            let state = owner.ensure_agent_state(session, &agent_id);
            !force_queue && state.active_prompt.is_none()
        };
        if should_start {
            let state = owner.ensure_agent_state(session, &agent_id);
            prompt.set_status(PromptStatus::Running);
            state.active_prompt = Some(prompt.clone());
            Ok(PromptSubmissionOutcome::Started { prompt })
        } else {
            let pending_prompt_id = owner.next_pending_prompt_id();
            let state = owner.ensure_agent_state(session, &agent_id);
            if state.queued_prompts.len() >= PROMPT_QUEUE_LIMIT {
                crate::logging::warn_with_fields(
                    "daemon.prompt_queue",
                    "agent prompt queue overloaded",
                    serde_json::json!({
                        "session_id": session.id(),
                        "agent_id": agent_id,
                        "prompt_id": prompt.id(),
                        "queued_prompts": state.queued_prompts.len(),
                        "queue_limit": PROMPT_QUEUE_LIMIT,
                    }),
                );
                return Err(DaemonError::LocalTransport {
                    operation: "queue prompt",
                    message: format!(
                        "agent prompt queue overloaded: queued prompt limit {PROMPT_QUEUE_LIMIT} reached"
                    ),
                });
            }
            prompt = prompt.into_pending_queue_item(pending_prompt_id);
            state.queued_prompts.push_back(prompt.clone());
            Ok(PromptSubmissionOutcome::Queued { prompt })
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

    pub(crate) fn mark_active_prompt_running(
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
        active.set_status(PromptStatus::Running);
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

    #[cfg(test)]
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
        if let Some(active_prompt) = state.active_prompt.as_ref() {
            return Err(DaemonError::LocalTransport {
                operation: "activate queued prompt",
                message: format!(
                    "cannot activate queued prompt for agent `{agent_id}` while active prompt `{}` is still running",
                    active_prompt.id()
                ),
            });
        }
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

    pub(crate) fn activate_next_queued_prompt_with_prompt_id(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
        prompt_id: String,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        if let Some(active_prompt) = state.active_prompt.as_ref() {
            return Err(DaemonError::LocalTransport {
                operation: "activate queued prompt",
                message: format!(
                    "cannot activate queued prompt for agent `{agent_id}` while active prompt `{}` is still running",
                    active_prompt.id()
                ),
            });
        }
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
            .expect("queue front checked above")
            .with_id(prompt_id);
        active.set_status(PromptStatus::Dispatching);
        state.active_prompt = Some(active.clone());
        Ok(Some(active))
    }

    #[cfg(test)]
    pub(crate) fn activate_prompt(
        &self,
        session: &RuntimeSession,
        mut prompt: PromptQueueItem,
    ) -> Result<PromptQueueItem, DaemonError> {
        let agent_id = prompt.target_agent_id().to_string();
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, &agent_id);
        if let Some(active_prompt) = state.active_prompt.as_ref() {
            if active_prompt.id() != prompt.id() {
                return Err(DaemonError::LocalTransport {
                    operation: "activate prompt",
                    message: format!(
                        "cannot activate prompt `{}` for agent `{agent_id}` while active prompt `{}` is still running",
                        prompt.id(),
                        active_prompt.id()
                    ),
                });
            }
        }
        state
            .queued_prompts
            .retain(|queued| queued.id() != prompt.id());
        prompt.set_status(PromptStatus::Running);
        state.active_prompt = Some(prompt.clone());
        Ok(prompt)
    }

    pub(crate) fn sync_external_active_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        match active_prompt {
            Some(mut prompt) => {
                if state
                    .active_prompt
                    .as_ref()
                    .is_some_and(|active| active.is_arroba_owned())
                {
                    return false;
                }
                prompt.set_status(PromptStatus::Running);
                if state.active_prompt.as_ref() == Some(&prompt) {
                    return false;
                }
                state.active_prompt = Some(prompt);
                true
            }
            None => {
                if state
                    .active_prompt
                    .as_ref()
                    .is_some_and(|active| active.is_external())
                {
                    state.active_prompt = None;
                    return true;
                }
                false
            }
        }
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

    pub(crate) fn remove_queued_prompts_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> usize {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let removed = state.queued_prompts.len();
        state.queued_prompts.clear();
        removed
    }

    pub(crate) fn remove_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let index = state
            .queued_prompts
            .iter()
            .position(|prompt| prompt.id() == prompt_id)?;
        state.queued_prompts.remove(index)
    }

    pub(crate) fn update_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        prompt: impl Into<String>,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let queued = state
            .queued_prompts
            .iter_mut()
            .find(|queued| queued.id() == prompt_id)?;
        queued.set_prompt(prompt);
        Some(queued.clone())
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
    fn next_pending_prompt_id(&mut self) -> String {
        self.next_pending_prompt_number = self.next_pending_prompt_number.wrapping_add(1);
        format!(
            "pending-prompt-{:016x}",
            crate::session::unix_epoch_ms() ^ self.next_pending_prompt_number.rotate_left(17)
        )
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_prepared_prompt_rejects_queue_overflow() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );

        for index in 0..PROMPT_QUEUE_LIMIT {
            let outcome = owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new(
                        format!("prompt-{index}"),
                        "attachment-1",
                        "agent-1",
                        "queued",
                        PromptStatus::Queued,
                    ),
                    true,
                )
                .expect("prompt should fit while under queue limit");
            assert!(matches!(outcome, PromptSubmissionOutcome::Queued { .. }));
        }

        let error = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-overflow",
                    "attachment-1",
                    "agent-1",
                    "overflow",
                    PromptStatus::Queued,
                ),
                true,
            )
            .expect_err("queue limit should reject overflow prompt");

        assert!(error.to_string().contains("agent prompt queue overloaded"));
        assert_eq!(
            owner.queued_prompt_count_for_agent(&session, "agent-1"),
            PROMPT_QUEUE_LIMIT
        );
    }

    #[test]
    fn activate_next_queued_prompt_rejects_when_active_prompt_exists() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let started = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("first prompt should start");
        assert!(matches!(started, PromptSubmissionOutcome::Started { .. }));
        let queued = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-queued",
                    "attachment-1",
                    "agent-1",
                    "queued",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("second prompt should queue");
        let queued_prompt_id = match queued {
            PromptSubmissionOutcome::Queued { prompt } => {
                assert!(prompt.id().starts_with("pending-prompt-"));
                assert_eq!(prompt.pending_prompt_id(), Some(prompt.id()));
                prompt.id().to_string()
            }
            PromptSubmissionOutcome::Started { .. } => panic!("second prompt should queue"),
        };

        let error = owner
            .activate_next_queued_prompt(&session, "agent-1", Some(&queued_prompt_id))
            .expect_err("queued prompt must not activate while active prompt is running");

        assert!(error.to_string().contains("cannot activate queued prompt"));
        assert_eq!(
            owner
                .active_prompt_for_agent_snapshot(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("prompt-active")
        );
        assert_eq!(
            owner
                .peek_next_queued_prompt(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some(queued_prompt_id.as_str())
        );
    }

    #[test]
    fn queued_prompt_promotes_with_new_real_prompt_id() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "draft-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("first prompt should start");
        let pending_prompt_id = match owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "draft-queued",
                    "attachment-1",
                    "agent-1",
                    "queued",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("second prompt should queue")
        {
            PromptSubmissionOutcome::Queued { prompt } => {
                assert!(prompt.id().starts_with("pending-prompt-"));
                assert_eq!(prompt.pending_prompt_id(), Some(prompt.id()));
                prompt.id().to_string()
            }
            PromptSubmissionOutcome::Started { .. } => panic!("second prompt should queue"),
        };

        let completed = owner
            .complete_active_prompt_only(&session, "agent-1")
            .expect("active prompt should complete");
        assert_eq!(completed.id(), "draft-active");

        let started = owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some(&pending_prompt_id),
                "prompt-real-2".to_string(),
            )
            .expect("queued prompt should activate")
            .expect("queued prompt should exist");

        assert_eq!(started.id(), "prompt-real-2");
        assert_eq!(started.pending_prompt_id(), None);
        assert_eq!(started.prompt(), "queued");
        assert_eq!(started.status(), PromptStatus::Dispatching);
        assert!(owner.peek_next_queued_prompt(&session, "agent-1").is_none());

        let running = owner
            .mark_active_prompt_running(&session, "agent-1")
            .expect("dispatching prompt should become running");
        assert_eq!(running.id(), "prompt-real-2");
        assert_eq!(running.status(), PromptStatus::Running);
    }

    #[test]
    fn activate_prompt_rejects_replacing_different_active_prompt() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let active = owner
            .activate_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
            )
            .expect("first prompt should activate");
        assert_eq!(active.status(), PromptStatus::Running);

        let error = owner
            .activate_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-replacement",
                    "attachment-1",
                    "agent-1",
                    "replacement",
                    PromptStatus::Queued,
                ),
            )
            .expect_err("different active prompt must not be replaced");

        assert!(error.to_string().contains("cannot activate prompt"));
        assert_eq!(
            owner
                .active_prompt_for_agent_snapshot(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("prompt-active")
        );
    }
}
