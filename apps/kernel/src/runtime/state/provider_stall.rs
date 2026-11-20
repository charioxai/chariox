//! The home kernel owns the decision for a stalled turn, including turns that run on a lease.
use super::*;
use crate::runtime::projection::{AgentPromptRuntimeStatus, AgentWaitKind, STALLED_AFTER_MS};
use crate::session::{RuntimeInteraction, RuntimeSession};

mod actions;

const PREFIX: &str = "provider-stall-";

pub(super) fn is_provider_stall_decision(interaction: &RuntimeInteraction) -> bool {
    interaction.id().starts_with(PREFIX)
        && interaction.kernel_operation_id() == Some(interaction.id())
}

#[derive(Clone)]
pub(super) struct PublishedStall {
    notice: StallNotice,
    announced: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct StallNotice {
    room: String,
    agent: String,
    prompt: String,
    run: Option<String>,
    quiet_since_ms: u64,
}

impl StallNotice {
    fn id(&self) -> String {
        use sha2::{Digest, Sha256};
        let identity = (
            &self.room,
            &self.agent,
            &self.prompt,
            &self.run,
            self.quiet_since_ms,
        );
        let key = Sha256::digest(serde_json::to_vec(&identity).expect("stall identity serializes"));
        format!("{PREFIX}{key:x}")
    }
}

impl KernelRuntimeState {
    pub(super) async fn reconcile_provider_stalls(&self, now: u64) -> Result<(), DaemonError> {
        let Some(cutoff) = now.checked_sub(STALLED_AFTER_MS) else {
            return Ok(());
        };
        let previous = self.owned.provider_stalls.lock().unwrap().clone();
        let rooms = self
            .owned
            .prompt_state_owner
            .rooms_with_running_prompts_before(cutoff);
        let mut stalled = BTreeMap::new();
        for room in rooms {
            let Ok(room) = self.stall_room(&room) else {
                continue;
            };
            let activity = self.agent_activity_for_session_at(&room, None, now);
            for agent in room.agents().iter().filter(|agent| !agent.archived()) {
                let Some(current) = activity.get(agent.id()).filter(|a| a.liveness.stalled) else {
                    continue;
                };
                if let Some(notice) = stall_notice(&room, agent.id(), current) {
                    stalled.insert(agent.id().to_owned(), notice);
                }
            }
        }

        for (agent, published) in &previous {
            let notice = &published.notice;
            if stalled.get(agent) != Some(notice) {
                if self.owned.session_store.get_session(&notice.room).is_ok() {
                    self.owned
                        .timeout_runtime_interaction(&notice.room, &notice.id())?;
                }
                self.owned.provider_stalls.lock().unwrap().remove(agent);
            }
        }
        let mut recipients = BTreeSet::new();
        for (agent, notice) in stalled {
            match self
                .publish_provider_stall(
                    &notice,
                    previous.get(&agent).filter(|entry| entry.notice == notice),
                    now,
                )
                .await
            {
                Ok(Some(recipient)) => {
                    recipients.insert(recipient);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, agent_id = %agent, "provider stall decision could not be published");
                }
            }
        }
        // Queue every sibling's event before the parent claims a wake. Delivery may contact
        // another kernel, so it must leave the sweep free to clear cards on progress.
        for (room, holder) in recipients {
            let state = self.as_kernel();
            tokio::spawn(async move {
                if let Err(error) = state.deliver_agent_inbox(&room, &holder).await {
                    tracing::warn!(%error, "provider stall decision delivery deferred");
                }
            });
        }
        Ok(())
    }

    async fn publish_provider_stall(
        &self,
        notice: &StallNotice,
        previous: Option<&PublishedStall>,
        now: u64,
    ) -> Result<Option<(String, String)>, DaemonError> {
        if previous.is_some_and(|entry| entry.announced) {
            return Ok(None);
        }
        if previous.is_none() {
            self.raise_provider_stall(notice, now).await?;
            self.owned.provider_stalls.lock().unwrap().insert(
                notice.agent.clone(),
                PublishedStall {
                    notice: notice.clone(),
                    announced: false,
                },
            );
        }
        let recipient = self
            .owned
            .record_child_decision_event(&notice.room, &notice.id())?;
        if let Some(entry) = self
            .owned
            .provider_stalls
            .lock()
            .unwrap()
            .get_mut(&notice.agent)
        {
            entry.announced = true;
        }
        Ok(recipient)
    }

    fn stall_room(&self, id: &str) -> Result<RuntimeSession, DaemonError> {
        let mut room = self.owned.session_store.get_session(id)?;
        // A worker's backing Room is hidden. The parent's Room on the home kernel has the
        // authoritative prompt and the leased provider's projected activity.
        if !room.is_hidden() && room.status() != crate::session::SessionStatus::Ended {
            room.set_agents(self.owned.agent_store.get_session_agents(id));
            self.owned
                .prompt_state_owner
                .project_into_session(&mut room);
        }
        Ok(room)
    }

    async fn raise_provider_stall(
        &self,
        notice: &StallNotice,
        now: u64,
    ) -> Result<(), DaemonError> {
        let id = notice.id();
        let room = self.owned.session_store.get_session(&notice.room)?;
        let agent = self.owned.agent_store.get_agent(&notice.agent)?;
        let name = agent.alias().unwrap_or(agent.agent_ref());
        let minutes = now.saturating_sub(notice.quiet_since_ms) / 60_000;
        let provider = super::provider_auth_recovery::provider_label(agent.provider());
        let message = format!(
            "{name} has been waiting for its provider ({provider}) for {minutes} minutes without progress."
        );
        let choices = actions::RecoveryAction::choices();
        let interaction =
            RuntimeInteraction::for_kernel_operation(&id, &id, "Agent stalled", &message, choices)
                .without_timeout()
                .with_related_agent_id(agent.id().to_owned());
        let rx = self
            .create_kernel_operation_interaction(room.id(), agent.owner_user_id(), interaction)
            .await?;
        let state = self.as_kernel();
        let action_notice = notice.clone();
        tokio::spawn(async move {
            let Ok(answer) = rx.await else { return };
            let Some(action) = answer
                .choice_id
                .as_deref()
                .and_then(actions::RecoveryAction::from_choice)
            else {
                return;
            };
            if let Err(error) = state
                .apply_provider_stall_action(&action_notice, action)
                .await
            {
                state.owned.record_notice_for_agent(
                    &action_notice.room,
                    None,
                    Some(&action_notice.agent),
                    state
                        .owned
                        .attachment_store
                        .list_session_attachment_ids(&action_notice.room),
                    format!(
                        "Could not recover the stalled turn. {}",
                        crate::secret_redaction::redact_secrets(&error.to_string())
                    ),
                );
            }
        });

        Ok(())
    }
}

fn stall_notice(
    room: &RuntimeSession,
    agent: &str,
    activity: &crate::runtime::projection::AgentRuntimeActivity,
) -> Option<StallNotice> {
    let turn = activity.active_turn.as_ref()?;
    let wait = activity.liveness.waiting_on.as_ref()?;
    let prompt = room.active_prompt_for_agent(agent)?;
    if room.is_hidden()
        || room.status() == crate::session::SessionStatus::Ended
        || wait.kind != AgentWaitKind::Provider
        || turn.status != AgentPromptRuntimeStatus::Running
        || (turn.prompt_id != prompt.id()
            && prompt.pending_prompt_id() != Some(turn.prompt_id.as_str()))
        || room.active_interaction_for_agent(agent).is_some()
    {
        return None;
    }
    Some(StallNotice {
        room: room.id().into(),
        agent: agent.into(),
        prompt: prompt.id().to_owned(),
        run: turn.provider_run_id.clone(),
        quiet_since_ms: wait.since_ms,
    })
}
