//! MP-08 / MP-09 / MP-10 / MP-11: uncertain remote sends bind lineage before transport.
use crate::{durable_prompt_state::DurablePromptStateEventPayload, session::PromptOrigin};
use rusqlite::{params, Transaction};

/// The send intent and its lineage commit together. An absent acknowledgement
/// must not let a workflow invoked by the worker lose its causal ancestry.
pub(in crate::durable_state) fn record_prompt_state_in(
    tx: &Transaction<'_>,
    payload: &str,
) -> rusqlite::Result<()> {
    let mut state: DurablePromptStateEventPayload =
        serde_json::from_str(payload).map_err(|_| rusqlite::Error::InvalidQuery)?;
    state.restore_private_states();
    for prompt in &state.queued_prompts {
        if prompt.prompt_origin() != PromptOrigin::Notification {
            continue;
        }
        let Some((target, ..)) = prompt.remote_steer_outcome_uncertainty() else {
            continue;
        };
        bind_in(tx, &state.session_id, prompt.id(), &target)?;
    }
    Ok(())
}

pub(super) fn bind_in(
    tx: &Transaction<'_>,
    session: &str,
    notification_prompt: &str,
    target: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE app_outbox SET invocation_json=json_set(invocation_json,'$.steered_into_prompt_id',?3)
         WHERE source_kind='workflow_completion' AND queued_session_id=?1 AND queued_prompt_id=?2",
        params![session, notification_prompt, target],
    )?;
    Ok(())
}
