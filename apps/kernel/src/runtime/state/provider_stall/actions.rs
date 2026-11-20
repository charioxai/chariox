//! Applies a decision only to the provider turn that raised it.
use super::*;
use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::session::{PromptQueueItem, PromptStatus, RuntimeInteractionChoice};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RecoveryAction {
    Resume,
    Cancel,
    StartFresh,
}

impl RecoveryAction {
    const CHOICES: [(Self, &'static str, &'static str); 3] = [
        (Self::Resume, "resume", "Resume"),
        (Self::Cancel, "cancel", "Cancel"),
        (Self::StartFresh, "start_fresh", "Start fresh from notes"),
    ];

    pub(super) fn choices() -> Vec<RuntimeInteractionChoice> {
        Self::CHOICES
            .into_iter()
            .map(|(_, key, label)| RuntimeInteractionChoice::new(key, label, key, None))
            .collect()
    }

    pub(super) fn from_choice(choice: &str) -> Option<Self> {
        Self::CHOICES
            .into_iter()
            .find_map(|(action, key, _)| (key == choice).then_some(action))
    }
}

impl KernelRuntimeState {
    pub(super) async fn apply_provider_stall_action(
        &self,
        notice: &StallNotice,
        action: RecoveryAction,
    ) -> Result<(), DaemonError> {
        let room = self.stall_room(&notice.room)?;
        let activity = self.agent_activity_for_session(&room);
        if activity
            .get(&notice.agent)
            .and_then(|activity| stall_notice(&room, &notice.agent, activity))
            .as_ref()
            != Some(notice)
        {
            // A late answer must never stop a turn that made progress or a succeeding turn.
            return Ok(());
        }
        let agent = self.owned.agent_store.get_agent(&notice.agent)?;
        if agent.archived() {
            return Ok(());
        }
        let prompt = room
            .active_prompt_for_agent(agent.id())
            .expect("notice matched the running prompt");
        let attachment = self.ensure_agent_message_attachment(room.id(), &agent)?;
        let cancellation = self
            .cancel_agent_prompt_if_matches_with_external_authority(
                room.id(),
                agent.id(),
                &attachment,
                Some(prompt.id()),
                None,
            )
            .await?;
        if let Some(dispatch) = cancellation.dispatch {
            self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
        }
        if action == RecoveryAction::Cancel {
            return Ok(());
        }
        self.wait_for_stall_cancellation(notice).await?;
        if action == RecoveryAction::StartFresh {
            let notes = self
                .owned
                .stall_recovery_notes(room.id(), agent.id(), prompt.id());
            let fresh = self.spawn_stall_replacement(&agent, &notice.id()).await?;
            self.retry_stalled_prompt(&fresh, prompt, notes.as_deref())
                .await
        } else {
            self.retry_stalled_prompt(&agent, prompt, None).await
        }
    }

    async fn wait_for_stall_cancellation(&self, notice: &StallNotice) -> Result<(), DaemonError> {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let sequence = self.session_projection_session_change_sequence(&notice.room);
                let room = self.owned.session_store.get_session(&notice.room)?;
                if self.owned.prompt_state_owner.active_prompt_for_agent(&room, &notice.agent)
                    .is_none_or(|prompt| prompt.id() != notice.prompt) {
                    return Ok(());
                }
                self.wait_for_session_projection_session_change_after(&notice.room, sequence).await;
            }
        })
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "recover stalled turn",
            message: "The provider has not confirmed the stop. Wait for the turn to end before trying again.".into(),
        })?
    }

    async fn spawn_stall_replacement(
        &self,
        agent: &AgentInstance,
        id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let mut request = CreateAgentRequest::new(agent.session_id(), agent.provider())
            .with_owner_user_id(agent.owner_user_id())
            .with_account_profile(agent.provider_account_profile())
            .with_spawn_key(id);
        request.stall_recovery = true;
        request.model = agent.model().map(str::to_owned);
        request.effort = agent.effort().map(str::to_owned);
        request.worktree_id = agent.worktree_id().map(str::to_owned);
        request.max_context_tokens = agent.max_context_tokens();
        request.execution_mode_override = agent.execution_mode_override();
        request.permission_level_override = agent.permission_level_override();
        if let Some(parent) = agent.parent_agent_id() {
            request = request.with_parent(&self.owned.agent_store.get_agent(parent)?);
        }
        if let Some(remote) = agent.remote_execution() {
            request.kernel_ref = Some(remote.worker_kernel_id.clone());
        }
        self.spawn_agent(request).await
    }

    async fn retry_stalled_prompt(
        &self,
        agent: &AgentInstance,
        original: &PromptQueueItem,
        notes: Option<&str>,
    ) -> Result<(), DaemonError> {
        let room = agent.session_id();
        let attachment = self.ensure_agent_message_attachment(room, agent)?;
        let prompt = PromptQueueItem::new(
            self.owned.session_store.reserve_prompt_id(),
            attachment,
            agent.id(),
            original.prompt(),
            PromptStatus::Queued,
        )
        .with_attachments(original.attachments().to_vec());
        let mut context = "Continue the interrupted request. Check the current state and pick up from the progress already made.".to_owned();
        if let Some(notes) = notes {
            context.push_str("\n\nNotes from the previous agent:\n");
            context.push_str(notes);
        }
        let prompt = prompt.with_hidden_system_context(context);
        let mut prepared = crate::app::KernelPreparedPromptSubmission {
            session_id: room.into(),
            prompt,
            force_queue: false,
            refresh_projection: true,
        };
        prepared.prepare_task_prompt_identity(|| self.owned.session_store.reserve_prompt_id())?;
        let submission = self.submit_prepared_prompt(prepared).await?;
        if let Some(dispatch) = submission.dispatch {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(dispatch) = submission.remote_dispatch {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(())
    }
}
