//! MP-08 / MP-09 / MP-10 / MP-11: a notification prompt carries its lineage from admission.
//! The receipt names the prompt's durable operation before the prompt exists, so the
//! prompt-state commit that admits the prompt binds the receipt in the same transaction.
use crate::{
    durable_prompt_state::DurablePromptStateEventPayload, error::DaemonError, session::PromptOrigin,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

const OPERATION_PREFIX: &str = "agent-notification:";

pub(super) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_notification_operations (
          operation_id TEXT PRIMARY KEY, receipt_id TEXT NOT NULL UNIQUE);",
    )
}

/// The durable operation of the prompt that delivers one occurrence to one trigger.
pub(crate) fn operation_id(
    trigger_id: &str,
    payload: &serde_json::Value,
) -> Result<String, DaemonError> {
    use sha2::{Digest, Sha256};
    let occurrence = payload
        .get("occurrence_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| super::error("notification occurrence id missing"))?;
    let identity = serde_json::to_vec(&(
        trigger_id,
        payload.get("source_id"),
        payload.get("installation_id"),
        payload.get("event_name"),
        occurrence,
    ))
    .map_err(|_| super::error("notification identity encoding failed"))?;
    Ok(format!("{OPERATION_PREFIX}{:x}", Sha256::digest(identity)))
}

/// Names the operation before admission. Reservations of receipts that are no longer
/// waiting for a prompt are dropped, so the table only holds pending deliveries.
pub(super) fn reserve_in(
    tx: &Transaction<'_>,
    receipt_id: &str,
    operation_id: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM agent_notification_operations WHERE receipt_id NOT IN
           (SELECT receipt_id FROM app_outbox WHERE state IN ('accepted','retryable'))",
        [],
    )?;
    tx.execute(
        "INSERT INTO agent_notification_operations
         SELECT ?2, receipt_id FROM app_outbox WHERE receipt_id=?1
           AND source_kind IN ('workflow_completion','app_event')
           AND state IN ('accepted','retryable')
         ON CONFLICT DO NOTHING",
        params![receipt_id, operation_id],
    )?;
    Ok(())
}

/// Called after inserting the prompt-state event, in the same writer transaction: the
/// admitted prompt and its receipt's lineage become durable together. Until the receipt
/// is settled, the latest prompt admitted for its operation holds the lineage; the
/// settlement writes the same prompt id.
pub(in crate::durable_state) fn record_prompt_state_in(
    tx: &Transaction<'_>,
    state: &DurablePromptStateEventPayload,
) -> rusqlite::Result<()> {
    for prompt in state
        .active_prompt
        .iter()
        .chain(state.queued_prompts.iter())
    {
        if prompt.prompt_origin() != PromptOrigin::Notification {
            continue;
        }
        let Some(operation) = prompt
            .durable_operation_id()
            .filter(|id| id.starts_with(OPERATION_PREFIX))
        else {
            continue;
        };
        let Some(receipt) = tx
            .query_row(
                "SELECT receipt_id FROM agent_notification_operations WHERE operation_id=?1",
                [operation],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            continue;
        };
        tx.execute(
            "DELETE FROM agent_notification_operations WHERE operation_id=?1",
            [operation],
        )?;
        tx.execute(
            "UPDATE app_outbox SET queued_prompt_id=?2,queued_session_id=?3,
               invocation_json=CASE source_kind WHEN 'workflow_completion'
                 THEN json_set(coalesce(invocation_json,'{}'),'$.ancestry',json_extract(payload_json,'$.ancestry')) ELSE invocation_json END
             WHERE receipt_id=?1 AND state IN ('accepted','retryable')",
            params![receipt, prompt.id(), state.session_id],
        )?;
    }
    Ok(())
}
