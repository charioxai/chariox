//! Worker-side bound on concurrently running leased turns.
//!
//! `remote_lease_capacity` limits leased agents with a provider turn in flight,
//! not how many leases or idle agents this worker holds. A turn that arrives at
//! capacity is admitted into its backing session, so the home sees it running,
//! but its provider dispatch is held in arrival order until a running turn
//! settles.

use crate::app::{KernelAgentService, KernelPromptDispatch};
use crate::error::DaemonError;
use crate::session::{PromptAttachment, PromptCancellation, PromptStatus};
use crate::transport::relay_peer::RemoteGitTurnContext;

use super::prompt_lifecycle::join_hidden_context;
use super::RemoteLeaseRuntime;

pub(crate) struct LeasedTurnWaitingForCapacity {
    leased_agent_id: String,
    dispatch: KernelPromptDispatch,
    /// Observed when the turn starts, so the git baseline excludes edits
    /// made by other turns while this one waited.
    git_context: Option<RemoteGitTurnContext>,
}

impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn leased_turn_is_waiting_for_capacity(&self, leased_agent_id: &str) -> bool {
        self.app
            .leased_turns_waiting_for_capacity
            .iter()
            .any(|waiting| waiting.leased_agent_id == leased_agent_id)
    }

    /// Whether a new leased turn must queue behind capacity or earlier waiters.
    pub(super) fn leased_turn_must_wait_for_capacity(&mut self) -> bool {
        self.admit_leased_turns_waiting_for_capacity();
        !self.app.leased_turns_waiting_for_capacity.is_empty() || self.running_turns_at_capacity()
    }

    pub(super) fn hold_leased_turn_for_capacity(
        &mut self,
        leased_agent_id: &str,
        dispatch: KernelPromptDispatch,
        git_context: Option<RemoteGitTurnContext>,
    ) {
        let message = format!(
            "Waiting for worker capacity: `{}` runs at most {} leased turns at once. This turn starts automatically when a running turn finishes.",
            self.app.config.host_machine_id,
            self.app.config.remote_lease_capacity.unwrap_or_default(),
        );
        let recipients = self
            .app
            .attachments
            .list_session_attachment_ids(&dispatch.session_id);
        self.app.record_notice(
            &dispatch.session_id,
            Some(&dispatch.provider_run_id),
            recipients,
            message,
        );
        self.app
            .leased_turns_waiting_for_capacity
            .push_back(LeasedTurnWaitingForCapacity {
                leased_agent_id: leased_agent_id.to_string(),
                dispatch,
                git_context,
            });
    }

    /// A steer for a turn that has not reached its provider yet is delivered
    /// with that turn.
    pub(super) fn add_steer_to_held_turn(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        hidden_system_context: &str,
        attachments: Vec<PromptAttachment>,
    ) {
        let Some(waiting) = self
            .app
            .leased_turns_waiting_for_capacity
            .iter_mut()
            .find(|waiting| waiting.leased_agent_id == leased_agent_id)
        else {
            return;
        };
        let dispatch = &mut waiting.dispatch;
        dispatch.prompt = format!("{}\n\n{prompt}", dispatch.prompt.trim_end());
        dispatch.hidden_system_context =
            join_hidden_context(&dispatch.hidden_system_context, hidden_system_context);
        dispatch.attachments.extend(attachments);
    }

    #[cfg(test)]
    pub(crate) fn held_turn_prompt(&self, leased_agent_id: &str) -> Option<String> {
        self.app
            .leased_turns_waiting_for_capacity
            .iter()
            .find(|waiting| waiting.leased_agent_id == leased_agent_id)
            .map(|waiting| waiting.dispatch.prompt.clone())
    }

    /// Starts held turns, oldest first, while running turns are below capacity.
    /// Turns that settled while waiting, or whose agent is gone, are dropped.
    pub(crate) fn admit_leased_turns_waiting_for_capacity(&mut self) {
        for waiting in std::mem::take(&mut self.app.leased_turns_waiting_for_capacity) {
            if self.turn_is_still_held(&waiting) {
                self.app
                    .leased_turns_waiting_for_capacity
                    .push_back(waiting);
            }
        }
        while !self.app.leased_turns_waiting_for_capacity.is_empty()
            && !self.running_turns_at_capacity()
        {
            let Some(LeasedTurnWaitingForCapacity {
                leased_agent_id,
                dispatch,
                git_context,
            }) = self.app.leased_turns_waiting_for_capacity.pop_front()
            else {
                return;
            };
            if let (Some(git_context), Some(leased_agent)) = (
                git_context,
                self.app.leased_agents.get(&leased_agent_id).cloned(),
            ) {
                self.observe_leased_git_before(
                    &leased_agent,
                    &dispatch.provider_run_id,
                    git_context,
                );
            }
            let (session_id, provider_run_id, prompt_id) = (
                dispatch.session_id.clone(),
                dispatch.provider_run_id.clone(),
                dispatch.prompt_id.clone(),
            );
            if let Err(error) =
                KernelAgentService::new(self.app).finish_compat_prompt_dispatch(Some(dispatch))
            {
                crate::logging::warn_with_fields(
                    "daemon.remote_lease_capacity",
                    "held leased turn failed to start",
                    serde_json::json!({
                        "leased_agent_id": leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
                // The dispatch failure cancelled the backing prompt, but the
                // home was already told the turn is running.
                self.record_held_turn_settlement(&session_id, &provider_run_id, &prompt_id);
            }
        }
    }

    /// Cancels a turn that never reached its provider.
    pub(super) fn cancel_leased_turn_waiting_for_capacity(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<Option<PromptCancellation>, DaemonError> {
        let Some(index) = self
            .app
            .leased_turns_waiting_for_capacity
            .iter()
            .position(|waiting| waiting.leased_agent_id == leased_agent_id)
        else {
            return Ok(None);
        };
        let Some(waiting) = self.app.leased_turns_waiting_for_capacity.remove(index) else {
            return Ok(None);
        };
        if !self.turn_is_still_held(&waiting) {
            return Ok(None);
        }
        let dispatch = waiting.dispatch;
        self.app.prompt_owner_begin_cancelling_active_prompt(
            &dispatch.session_id,
            &dispatch.agent_id,
        )?;
        let cancellation = self.app.finalize_active_prompt_cancellation(
            &dispatch.session_id,
            &dispatch.agent_id,
            Some(&dispatch.provider_run_id),
        )?;
        self.record_held_turn_settlement(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.prompt_id,
        );
        Ok(Some(cancellation))
    }

    /// A held turn that ends without reaching its provider gets no provider
    /// settlement, so record the completion the home drain projects.
    fn record_held_turn_settlement(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        prompt_id: &str,
    ) {
        let recipients = self.app.attachments.list_session_attachment_ids(session_id);
        self.app.record_assistant_message_completion(
            session_id,
            provider_run_id,
            recipients,
            &format!("prompt-complete:{prompt_id}"),
            crate::session::unix_epoch_ms(),
        );
    }

    fn turn_is_still_held(&mut self, waiting: &LeasedTurnWaitingForCapacity) -> bool {
        let dispatch = &waiting.dispatch;
        self.app
            .leased_agents
            .contains_key(&waiting.leased_agent_id)
            && self
                .app
                .prompt_owner_active_prompt_for_agent(&dispatch.session_id, &dispatch.agent_id)
                .ok()
                .flatten()
                .is_some_and(|active| {
                    active.id() == dispatch.prompt_id && active.status() != PromptStatus::Cancelling
                })
    }

    fn running_turns_at_capacity(&mut self) -> bool {
        self.app
            .config
            .remote_lease_capacity
            .is_some_and(|capacity| self.running_leased_turn_count() >= capacity)
    }

    /// Leased agents with an active backing prompt that is not held here.
    fn running_leased_turn_count(&mut self) -> usize {
        let candidates = self
            .app
            .leased_agents
            .values()
            .filter(|agent| !self.leased_turn_is_waiting_for_capacity(&agent.id))
            .map(|agent| {
                (
                    agent.backing_session_id.clone(),
                    agent.backing_agent_id.clone(),
                )
            })
            .collect::<Vec<_>>();
        candidates
            .into_iter()
            .filter(|(session_id, agent_id)| {
                self.app
                    .prompt_owner_active_prompt_for_agent(session_id, agent_id)
                    .ok()
                    .flatten()
                    .is_some()
            })
            .count()
    }
}
