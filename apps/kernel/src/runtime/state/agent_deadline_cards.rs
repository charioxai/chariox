//! One recovery decision per expired agent limit, independent of the delivery it refused.
use super::*;
use crate::local::{AgentDeadline, AgentDeadlineState, SetAgentDeadlineRequest};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};

const PREFIX: &str = "agent-deadline-ended-";
const EXTEND_MS: u64 = 3_600_000;

pub(super) fn is_deadline_card(id: &str) -> bool {
    id.starts_with(PREFIX)
}

fn card_id(deadline: &AgentDeadline) -> String {
    format!("{PREFIX}{}-{}", deadline.agent_id, deadline.deadline_ms)
}

fn reached(deadline: &AgentDeadline) -> bool {
    deadline.state == AgentDeadlineState::Reached
        || deadline.deadline_ms <= crate::session::unix_epoch_ms()
}

impl KernelRuntimeState {
    pub(super) fn has_pending_agent_message(&self, room: &str, agent: &str) -> bool {
        let mut after = 0;
        loop {
            let Ok(events) = self
                .owned
                .durable_state_store
                .agent_work_inbox(room, agent, after, None)
            else {
                return false;
            };
            let Some(last) = events.last().map(|event| event.sequence) else {
                return false;
            };
            if events
                .iter()
                .any(|event| event.kind == "message" && event.state == "pending")
            {
                return true;
            }
            after = last;
        }
    }

    pub(super) async fn show_deadline_refusal(&self, room: &str, agent: &str) {
        if let Err(error) = self.raise_deadline_card(room, agent).await {
            tracing::warn!(%error, room, agent, "could not show the agent's ended time limit");
        }
    }

    async fn raise_deadline_card(&self, room: &str, agent: &str) -> Result<(), DaemonError> {
        let Some(deadline) = self
            .owned
            .durable_state_store
            .agent_deadline(room, agent)?
            .filter(reached)
        else {
            return Ok(());
        };
        let target = self.owned.agent_store.get_agent(agent)?;
        let session = self.owned.session_store.get_session(room)?;
        let id = card_id(&deadline);
        if session
            .active_interactions()
            .iter()
            .any(|card| card.id() == id)
            && self.owned.pending_interactions.write().contains_key(&id)
        {
            return Ok(());
        }
        let time = i64::try_from(deadline.deadline_ms)
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|time| format!(" at {}", time.format("%Y-%m-%d %H:%M:%S UTC")))
            .unwrap_or_default();
        let name = target.alias().unwrap_or(target.agent_ref());
        let card = RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "Agent time limit ended",
            format!("{name}’s time limit ended{time}. Extend it to let the agent take new work."),
            vec![
                RuntimeInteractionChoice::new(
                    "extend_limit",
                    "Extend limit by 1 hour",
                    "extend_limit",
                    None,
                ),
                RuntimeInteractionChoice::new("dismiss", "Dismiss", "dismiss", None),
            ],
        )
        .without_timeout()
        .with_related_agent_id(agent.to_owned());
        let rx = self
            .create_kernel_operation_interaction(
                super::CardKind::OwnerControl,
                session.id(),
                session.owner_user_id(),
                card,
            )
            .await?;

        // This card and an independent extension can cross. The setter closes cards it sees,
        // and this recheck handles an extension that committed just before registration.
        if !self
            .owned
            .durable_state_store
            .agent_deadline(room, agent)?
            .is_some_and(|current| current.deadline_ms == deadline.deadline_ms && reached(&current))
        {
            return self.owned.close_deadline_card(&deadline);
        }
        let state = self.as_kernel();
        let owner = session.owner_user_id().to_owned();
        tokio::spawn(async move {
            if rx
                .await
                .is_ok_and(|answer| answer.choice_id.as_deref() == Some("extend_limit"))
            {
                let request = SetAgentDeadlineRequest {
                    session_id: deadline.session_id.clone(),
                    agent_id: deadline.agent_id.clone(),
                    deadline_ms: Some(crate::session::unix_epoch_ms().saturating_add(EXTEND_MS)),
                };
                if let Err(error) = state
                    .set_agent_deadline_inner(&owner, request, Some(deadline.deadline_ms))
                    .await
                {
                    state.owned.record_notice_for_agent(
                        &deadline.session_id,
                        None,
                        Some(&deadline.agent_id),
                        state
                            .owned
                            .attachment_store
                            .list_session_attachment_ids(&deadline.session_id),
                        error.to_string(),
                    );
                }
            }
        });
        Ok(())
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn close_deadline_card(&self, deadline: &AgentDeadline) -> Result<(), DaemonError> {
        self.settle_card_condition_met(
            &deadline.session_id,
            &card_id(deadline),
            "The agent’s time limit was updated. It can take new work.".into(),
        )
    }
}
