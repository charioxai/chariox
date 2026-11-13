//! MP-08 / MP-09 / MP-10 / MP-11: durable ordinary prompt transitions settle App receipts.
use crate::{durable_prompt_state::DurablePromptStateEventPayload, session::PromptQueueItem};
use rusqlite::{params, OptionalExtension, Transaction};
fn decode(payload: &str) -> rusqlite::Result<DurablePromptStateEventPayload> {
    serde_json::from_str(payload).map_err(|_| rusqlite::Error::InvalidQuery)
}
fn contains(state: &DurablePromptStateEventPayload, id: &str) -> bool {
    state
        .active_prompt
        .iter()
        .chain(state.queued_prompts.iter())
        .any(|p| p.id() == id)
}
// An active or steered prompt consumed the notification. A queued cancellation did not.
fn terminal_state(
    tx: &Transaction<'_>,
    session: &str,
    agent: &str,
    prompt: &PromptQueueItem,
    active: bool,
) -> rusqlite::Result<&'static str> {
    let steered: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM durable_trigger_firings WHERE session_id=?1 AND prompt_id=?2)
         OR EXISTS(SELECT 1 FROM durable_state_events WHERE kind='session.prompt.steered'
           AND subject_id=?1 AND json_extract(payload_json,'$.agent_id')=?3
           AND json_extract(payload_json,'$.prompt_id')=?2)",
        params![session, prompt.id(), agent],
        |row| row.get(0),
    )?;
    Ok(if steered || active {
        "delivered"
    } else {
        "failed"
    })
}
fn settle(
    tx: &Transaction<'_>,
    session: &str,
    agent: &str,
    prompt: &str,
    state: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE app_outbox SET state=?1,revision=revision+1
        WHERE state='queued' AND source_kind='app_event' AND queued_session_id=?2
          AND queued_agent_id=?3 AND queued_prompt_id=?4",
        params![state, session, agent, prompt],
    )?;
    Ok(())
}
/// Called after inserting the prompt-state event, in the same writer transaction.
/// Promotion keeps the prompt id; only removal from both live queues settles it.
pub(in crate::durable_state) fn record_prompt_state_in(
    tx: &Transaction<'_>,
    current: &DurablePromptStateEventPayload,
) -> rusqlite::Result<()> {
    let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM app_outbox WHERE state='queued' AND queued_session_id=?1 AND queued_agent_id=?2)", params![current.session_id,current.agent_id], |row| row.get(0))?;
    if !pending {
        return Ok(());
    }
    let previous: Option<String> = tx.query_row(
        "SELECT payload_json FROM durable_state_events WHERE kind='session.prompt_state.updated'
         AND subject_id=?1 AND json_extract(payload_json,'$.agent_id')=?2 ORDER BY sequence DESC LIMIT 1 OFFSET 1",
        params![current.session_id,current.agent_id], |row| row.get(0),
    ).optional()?;
    let Some(previous) = previous else {
        return Ok(());
    };
    let previous = decode(&previous)?;
    for (prompt, active) in previous
        .active_prompt
        .iter()
        .map(|p| (p, true))
        .chain(previous.queued_prompts.iter().map(|p| (p, false)))
    {
        if !contains(current, prompt.id()) {
            settle(
                tx,
                &current.session_id,
                &current.agent_id,
                prompt.id(),
                terminal_state(tx, &current.session_id, &current.agent_id, prompt, active)?,
            )?;
        }
    }
    Ok(())
}
/// A fast provider may finish between durable queue admission and receipt association.
/// Use committed prompt history so that race cannot strand a queued receipt.
pub(in crate::durable_state) fn reconcile_prompt_in(
    tx: &Transaction<'_>,
    receipt: &str,
) -> rusqlite::Result<()> {
    let (session, agent, prompt): (String,String,String) = tx.query_row(
        "SELECT queued_session_id,queued_agent_id,queued_prompt_id FROM app_outbox WHERE receipt_id=?1 AND state='queued'",
        [receipt], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    )?;
    let latest: Option<(i64,String)> = tx.query_row(
        "SELECT sequence,payload_json FROM durable_state_events WHERE kind='session.prompt_state.updated'
         AND subject_id=?1 AND json_extract(payload_json,'$.agent_id')=?2 ORDER BY sequence DESC LIMIT 1",
        params![session,agent], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    let (sequence, latest) = latest.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    if contains(&decode(&latest)?, &prompt) {
        return Ok(());
    }
    let last_live: Option<String> = tx.query_row(
        "SELECT payload_json FROM durable_state_events WHERE kind='session.prompt_state.updated'
         AND subject_id=?1 AND json_extract(payload_json,'$.agent_id')=?2 AND sequence<?3
         AND (json_extract(payload_json,'$.active_prompt.id')=?4 OR EXISTS
              (SELECT 1 FROM json_each(payload_json,'$.queued_prompts') WHERE json_extract(value,'$.id')=?4))
         ORDER BY sequence DESC LIMIT 1",
        params![session,agent,sequence,prompt], |row| row.get(0),
    ).optional()?;
    {
        let last_live = last_live.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        let state = decode(&last_live)?;
        if let Some((item, active)) = state
            .active_prompt
            .iter()
            .map(|p| (p, true))
            .chain(state.queued_prompts.iter().map(|p| (p, false)))
            .find(|(p, _)| p.id() == prompt)
        {
            settle(
                tx,
                &session,
                &agent,
                &prompt,
                terminal_state(tx, &session, &agent, item, active)?,
            )?;
        }
    }
    Ok(())
}
