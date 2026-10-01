//! Protocol 394: every passkey prompt is a kernel-owned pending interaction,
//! projected as a popup to every terminal connected as its owner, attached to
//! the session or not (kernel access plan, section 5.2, D1). Today that is
//! each critical approval. It joins its owner's set when it is registered and
//! leaves it when a terminal answers it (approve with the verified passkey, or
//! refuse) or when it expires; the change wakes every terminal subscription,
//! which sends the new set, so every popup closes. A later answer is told the
//! prompt was already answered. A wrong passkey answers nothing, so the
//! prompt stays.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::{KernelRuntimeOwnedState, KernelRuntimeState, RuntimeChangeSignal};
use crate::error::DaemonError;
use crate::local::{PasskeyPrompt, PasskeyPromptKind};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice, RuntimeSession};

pub(crate) const PASSKEY_ALREADY_ANSWERED: &str = "PASSKEY_ALREADY_ANSWERED";
/// Answered prompts remembered, so a late answer is told so.
const ANSWERED_MEMORY: usize = 64;

#[derive(Debug, Default)]
pub(super) struct PasskeyPromptBoard {
    changes: RuntimeChangeSignal,
    answered: Mutex<VecDeque<(String, String)>>,
}

impl PasskeyPromptBoard {
    /// A prompt joined or left the set.
    pub(super) fn record_change(&self) {
        self.changes.record_change();
    }

    pub(super) fn record_answered(&self, session_id: &str, interaction_id: &str) {
        let mut answered = self.answered.lock().expect("passkey prompts poisoned");
        if answered.len() == ANSWERED_MEMORY {
            answered.pop_front();
        }
        answered.push_back((session_id.to_owned(), interaction_id.to_owned()));
        drop(answered);
        self.record_change();
    }

    /// A prompt raised again under the same id is open again.
    pub(super) fn forget_answered(&self, session_id: &str, interaction_id: &str) {
        self.answered
            .lock()
            .expect("passkey prompts poisoned")
            .retain(|(session, id)| session != session_id || id != interaction_id);
    }

    fn was_answered(&self, session_id: &str, interaction_id: &str) -> bool {
        self.answered
            .lock()
            .expect("passkey prompts poisoned")
            .iter()
            .any(|(session, id)| session == session_id && id == interaction_id)
    }
}

/// The popup for a kernel decision that needs the passkey, built from what
/// the kernel registered: `None` for any other interaction. Such a decision
/// has exactly two choices, approve (marked `requires_passkey`) and refuse.
pub(super) fn passkey_prompt(
    session: &RuntimeSession,
    interaction: &RuntimeInteraction,
    registered_at_ms: u64,
) -> Result<Option<Arc<PasskeyPrompt>>, DaemonError> {
    let choices = interaction.choices();
    if !choices
        .iter()
        .any(RuntimeInteractionChoice::requires_passkey)
    {
        return Ok(None);
    }
    let (approve, refuse) = match choices {
        [first, second] if first.requires_passkey() != second.requires_passkey() => {
            if first.requires_passkey() {
                (first, second)
            } else {
                (second, first)
            }
        }
        _ => {
            return Err(DaemonError::LocalTransport {
                operation: "runtime interaction",
                message: "A passkey prompt has one approve and one refuse choice".into(),
            })
        }
    };
    let timeout_ms = interaction.timeout_sec().unwrap_or_default() * 1000;
    Ok(Some(Arc::new(PasskeyPrompt {
        kind: PasskeyPromptKind::CriticalApproval,
        session_id: session.id().to_owned(),
        session_alias: session.alias().map(str::to_owned),
        interaction_id: interaction.id().to_owned(),
        title: interaction
            .title()
            .unwrap_or("Critical approval")
            .to_owned(),
        message: interaction.message().to_owned(),
        approve_choice_id: approve.id().to_owned(),
        refuse_choice_id: refuse.id().to_owned(),
        requested_at_ms: interaction.requested_at_ms(),
        expires_at_ms: registered_at_ms.saturating_add(timeout_ms),
    })))
}

impl KernelRuntimeOwnedState {
    fn passkey_prompts_for(&self, user_id: &str) -> Vec<PasskeyPrompt> {
        let now = std::time::Instant::now();
        let mut prompts = self
            .pending_interactions
            .write()
            .values()
            .filter(|pending| pending.belongs_to(&self.session_store))
            .filter(|pending| pending.kernel_operation_owner.as_deref() == Some(user_id))
            .filter(|pending| {
                pending
                    .kernel_operation_deadline
                    .is_some_and(|deadline| now < deadline)
            })
            .filter_map(|pending| pending.passkey_prompt.as_deref().cloned())
            .collect::<Vec<_>>();
        prompts.sort_by(|a, b| {
            (a.requested_at_ms, &a.interaction_id).cmp(&(b.requested_at_ms, &b.interaction_id))
        });
        prompts
    }

    /// The refusal for an answer to a decision that is no longer pending:
    /// "already answered" when a terminal answered it.
    pub(super) fn closed_interaction_error(
        &self,
        session_id: &str,
        interaction_id: &str,
        error: DaemonError,
    ) -> DaemonError {
        let pending = self
            .pending_interactions
            .write()
            .get(interaction_id)
            .is_some_and(|pending| pending.session_id == session_id);
        if !pending
            && self
                .passkey_prompts
                .was_answered(session_id, interaction_id)
        {
            return super::critical_approval_passkey::passkey_error(
                PASSKEY_ALREADY_ANSWERED,
                "already answered",
            );
        }
        error
    }
}

impl KernelRuntimeState {
    /// The passkey prompts pending for `user_id`, oldest first.
    pub(crate) fn passkey_prompts_for(&self, user_id: &str) -> Vec<PasskeyPrompt> {
        self.owned.passkey_prompts_for(user_id)
    }

    pub(crate) fn passkey_prompt_change_sequence(&self) -> u64 {
        self.owned.passkey_prompts.changes.sequence()
    }

    pub(crate) async fn wait_for_passkey_prompt_change_after(&self, sequence: u64) {
        self.owned
            .passkey_prompts
            .changes
            .wait_for_change_after(sequence)
            .await;
    }
}
