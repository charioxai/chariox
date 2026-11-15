//! MP-08 / MP-09 / MP-10 / MP-11: uncertain remote sends bind lineage before transport.
use crate::{
    durable_prompt_state::{DurablePromptStateEventPayload, DURABLE_PROMPT_STATE_EVENT_KIND},
    session::PromptOrigin,
};
use rusqlite::{params, OptionalExtension, Transaction};

fn decode(payload: &str) -> rusqlite::Result<DurablePromptStateEventPayload> {
    let mut state: DurablePromptStateEventPayload =
        serde_json::from_str(payload).map_err(|_| rusqlite::Error::InvalidQuery)?;
    state.restore_private_states();
    Ok(state)
}

/// The send intent and its lineage commit together. An absent acknowledgement
/// must not let a workflow invoked by the worker lose its causal ancestry.
/// A send whose uncertainty ends while its prompt stays in the queue was rejected
/// by the worker: the commit that releases the hold also ends that lineage.
pub(in crate::durable_state) fn record_prompt_state_in(
    tx: &Transaction<'_>,
    state: &DurablePromptStateEventPayload,
) -> rusqlite::Result<()> {
    let notifications = || {
        state
            .active_prompt
            .iter()
            .chain(state.queued_prompts.iter())
            .filter(|prompt| prompt.prompt_origin() == PromptOrigin::Notification)
    };
    if notifications().next().is_none() {
        return Ok(());
    }
    for prompt in &state.queued_prompts {
        if prompt.prompt_origin() != PromptOrigin::Notification {
            continue;
        }
        let Some((target, ..)) = prompt.remote_steer_outcome_uncertainty() else {
            continue;
        };
        bind_in(tx, &state.session_id, prompt.id(), &target)?;
    }
    let previous: Option<String> = tx
        .query_row(
            "SELECT payload_json FROM durable_state_events WHERE kind=?3 AND subject_id=?1
             AND json_extract(payload_json,'$.agent_id')=?2 ORDER BY sequence DESC LIMIT 1 OFFSET 1",
            params![state.session_id, state.agent_id, DURABLE_PROMPT_STATE_EVENT_KIND],
            |row| row.get(0),
        )
        .optional()?;
    let Some(previous) = previous else {
        return Ok(());
    };
    let previous = decode(&previous)?;
    for prompt in notifications() {
        let Some((sent_to, ..)) = previous
            .queued_prompts
            .iter()
            .find(|earlier| earlier.id() == prompt.id())
            .and_then(|earlier| earlier.remote_steer_outcome_uncertainty())
        else {
            continue;
        };
        if prompt
            .remote_steer_outcome_uncertainty()
            .is_some_and(|(target, ..)| target == sent_to)
        {
            continue;
        }
        unbind_in(tx, &state.session_id, prompt.id(), &sent_to)?;
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
        "UPDATE app_outbox SET invocation_json=json_set(coalesce(invocation_json,'{}'),'$.steered_into_prompt_id',?3)
         WHERE source_kind IN ('workflow_completion','app_event') AND queued_session_id=?1 AND queued_prompt_id=?2",
        params![session, notification_prompt, target],
    )?;
    Ok(())
}

/// Removes only the association made for `target`; a later send to another turn keeps its own.
fn unbind_in(
    tx: &Transaction<'_>,
    session: &str,
    notification_prompt: &str,
    target: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE app_outbox SET invocation_json=json_remove(invocation_json,'$.steered_into_prompt_id')
         WHERE source_kind IN ('workflow_completion','app_event') AND queued_session_id=?1 AND queued_prompt_id=?2
           AND json_extract(invocation_json,'$.steered_into_prompt_id')=?3",
        params![session, notification_prompt, target],
    )?;
    Ok(())
}

/// The Steer association made before an App receipt settles. The App outbox clears the
/// invocation when it marks the receipt queued; the caller restores the association.
pub(in crate::durable_state) fn association_in(
    tx: &Transaction<'_>,
    receipt_id: &str,
) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT json_extract(invocation_json,'$.steered_into_prompt_id') FROM app_outbox WHERE receipt_id=?1",
        [receipt_id],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

pub(in crate::durable_state) fn restore_association_in(
    tx: &Transaction<'_>,
    receipt_id: &str,
    target: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE app_outbox SET invocation_json=json_set(coalesce(invocation_json,'{}'),'$.steered_into_prompt_id',?2)
         WHERE receipt_id=?1",
        params![receipt_id, target],
    )?;
    Ok(())
}
