//! Only normal workflow persistence calls this helper. App data cannot claim
//! delivery; the exact run/queue record must already exist in this transaction.
use super::super::{DurableWorkflowHotEntityWrite, DurableWorkflowRunWrite};
use crate::session::{
    WorkflowPublicationInvocationEnvelope, WorkflowQueuedPrompt, WorkflowQueuedPromptStatus,
    WorkflowRun,
};
use rusqlite::{params, OptionalExtension, Transaction};

pub(in crate::durable_state) fn record_workflow_transition_in(
    tx: &Transaction<'_>,
    durable_owner: &str,
    session_id: &str,
    hot: &[DurableWorkflowHotEntityWrite],
    runs: &[DurableWorkflowRunWrite],
) -> rusqlite::Result<()> {
    for encoded in runs {
        if encoded.session_id != session_id {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let run: WorkflowRun = serde_json::from_str(&encoded.payload_json)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let Some(envelope) = run
            .publication_invocation()
            .filter(|value| value.transport == "app_event")
        else {
            continue;
        };
        let Some(queue_item) = run.queue_item_id() else {
            continue;
        };
        let actual: Option<String> = tx.query_row(
            "SELECT payload_json FROM durable_workflow_runs WHERE owner_id=?1 AND session_id=?2 AND run_id=?3",
            params![durable_owner,session_id,encoded.run_id], |row|row.get(0),
        ).optional()?;
        if actual.as_deref() != Some(encoded.payload_json.as_str()) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        settle(tx, session_id, queue_item, envelope, "delivered")?;
    }
    for encoded in hot
        .iter()
        .filter(|value| value.entity_kind == "queued_prompt")
    {
        if encoded.session_id != session_id {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let queued: WorkflowQueuedPrompt = serde_json::from_str(&encoded.payload_json)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        if queued.status() != WorkflowQueuedPromptStatus::Cancelled {
            continue;
        }
        let Some(envelope) = queued
            .publication_invocation()
            .filter(|value| value.transport == "app_event")
        else {
            continue;
        };
        let actual:Option<String>=tx.query_row(
            "SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind='queued_prompt' AND entity_id=?3",
            params![durable_owner,session_id,encoded.entity_id], |row|row.get(0),
        ).optional()?;
        if actual.as_deref() != Some(encoded.payload_json.as_str()) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        settle(tx, session_id, queued.id(), envelope, "failed")?;
    }
    Ok(())
}
fn settle(
    tx: &Transaction<'_>,
    session: &str,
    queue: &str,
    envelope: &WorkflowPublicationInvocationEnvelope,
    state: &str,
) -> rusqlite::Result<()> {
    let Some(owner) = envelope.caller.get("owner_id").and_then(|v| v.as_str()) else {
        return Err(rusqlite::Error::InvalidQuery);
    };
    let Some(installation) = envelope
        .caller
        .get("installation_id")
        .and_then(|v| v.as_str())
    else {
        return Err(rusqlite::Error::InvalidQuery);
    };
    // An already terminal/pruned receipt is a normal replay of workflow history.
    // Exact original session+queue association prevents retargeting a binding
    // from acknowledging another run. Current App/publisher activation is not
    // required to record work which was already durably queued under authority.
    tx.execute(
        "UPDATE app_outbox SET state=?1,payload_json=NULL,invocation_json=NULL,
         revision=CASE WHEN revision<9223372036854775807 THEN revision+1 ELSE revision END
         WHERE owner_id=?2 AND installation_id=?3 AND receipt_id=?4 AND state='queued'
         AND queued_session_id=?5 AND queued_prompt_id=?6",
        params![
            state,
            owner,
            installation,
            envelope.invocation_id,
            session,
            queue
        ],
    )?;
    Ok(())
}

/// Recover the post-commit/pre-dispatch interval using actual hot queue records.
/// The durable cursor rotates even when old ambiguous/missing receipts remain.
pub(super) fn reconcile_queued_in(
    tx: &Transaction<'_>,
    durable_owner: &str,
    owner: &str,
    installation: &str,
) -> rusqlite::Result<Vec<String>> {
    let after:i64=tx.query_row("SELECT queued_after_sequence FROM app_outbox_replay_floors WHERE owner_id=?1 AND installation_id=?2",params![owner,installation],|r|r.get(0))?;
    let read = |after: i64| -> rusqlite::Result<Vec<(i64, String, String, Option<String>)>> {
        let mut statement=tx.prepare("SELECT sequence,receipt_id,queued_prompt_id,queued_session_id FROM app_outbox
         WHERE owner_id=?1 AND installation_id=?2 AND state='queued' AND sequence>?3 AND queued_prompt_id IS NOT NULL
         ORDER BY sequence LIMIT 1")?;
        let rows = statement.query_map(params![owner, installation, after], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        rows.collect()
    };
    let mut rows = read(after)?;
    if rows.is_empty() && after != 0 {
        rows = read(0)?;
    }
    let mut sessions = std::collections::BTreeSet::new();
    for (_, receipt, queue, session) in &rows {
        let mut statement = tx.prepare(
            "SELECT session_id,payload_json FROM durable_workflow_hot_entities
         WHERE owner_id=?1 AND entity_kind='queued_prompt' AND entity_id=?2
         AND (?3 IS NULL OR session_id=?3) LIMIT 2",
        )?;
        let actual = statement
            .query_map(params![durable_owner, queue, session], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if actual.len() != 1 {
            continue;
        }
        let (actual_session, encoded) = &actual[0];
        let queued: WorkflowQueuedPrompt =
            serde_json::from_str(encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let Some(envelope) = queued.publication_invocation() else {
            continue;
        };
        if !matches_envelope(envelope, owner, installation, receipt) || queued.id() != queue {
            continue;
        }
        if session.is_none() {
            tx.execute("UPDATE app_outbox SET queued_session_id=?1 WHERE owner_id=?2 AND installation_id=?3 AND receipt_id=?4 AND queued_session_id IS NULL AND state='queued' AND queued_prompt_id=?5",params![actual_session,owner,installation,receipt,queue])?;
        }
        if let Some(run_id) = queued.workflow_run_id() {
            let run:Option<String>=tx.query_row("SELECT payload_json FROM durable_workflow_runs WHERE owner_id=?1 AND session_id=?2 AND run_id=?3",params![durable_owner,actual_session,run_id],|r|r.get(0)).optional()?;
            if let Some(encoded) = run {
                let run: WorkflowRun =
                    serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
                if run.queue_item_id() == Some(queue.as_str())
                    && run
                        .publication_invocation()
                        .is_some_and(|value| matches_envelope(value, owner, installation, receipt))
                {
                    settle(tx, actual_session, queue, envelope, "delivered")?;
                    continue;
                }
            }
        }
        if queued.status() == WorkflowQueuedPromptStatus::Cancelled {
            settle(tx, actual_session, queue, envelope, "failed")?;
        } else if queued.status() == WorkflowQueuedPromptStatus::Queued {
            sessions.insert(actual_session.clone());
        }
    }
    tx.execute("UPDATE app_outbox_replay_floors SET queued_after_sequence=?1 WHERE owner_id=?2 AND installation_id=?3",params![rows.last().map(|r|r.0).unwrap_or(0),owner,installation])?;
    Ok(sessions.into_iter().collect())
}
fn matches_envelope(
    envelope: &WorkflowPublicationInvocationEnvelope,
    owner: &str,
    installation: &str,
    receipt: &str,
) -> bool {
    envelope.transport == "app_event"
        && envelope.invocation_id == receipt
        && envelope.caller.get("owner_id").and_then(|v| v.as_str()) == Some(owner)
        && envelope
            .caller
            .get("installation_id")
            .and_then(|v| v.as_str())
            == Some(installation)
}

/// An authoritative normalized replacement can explicitly remove a formerly
/// Queued item (the existing remove/clear APIs do so). Absence observed on restart
/// alone is never interpreted as cancellation.
pub(in crate::durable_state) fn record_queue_removals_in(
    tx: &Transaction<'_>,
    durable_owner: &str,
    session: &str,
    hot: &[DurableWorkflowHotEntityWrite],
    runs: &[DurableWorkflowRunWrite],
) -> rusqlite::Result<()> {
    let retained = hot
        .iter()
        .filter(|value| value.entity_kind == "queued_prompt")
        .map(|value| value.entity_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut run_queues = std::collections::BTreeSet::new();
    for encoded in runs {
        let run: WorkflowRun = serde_json::from_str(&encoded.payload_json)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        if let Some(queue) = run.queue_item_id() {
            run_queues.insert(queue.to_owned());
        }
    }
    let previous = {
        let mut statement=tx.prepare("SELECT DISTINCT e.entity_id,e.payload_json FROM durable_workflow_hot_entities e
          JOIN app_outbox r ON r.queued_prompt_id=e.entity_id AND r.queued_session_id=e.session_id AND r.state='queued'
          WHERE e.owner_id=?1 AND e.session_id=?2 AND e.entity_kind='queued_prompt'")?;
        let rows = statement.query_map(params![durable_owner, session], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, encoded) in previous {
        if retained.contains(id.as_str()) || run_queues.contains(&id) {
            continue;
        }
        let queued: WorkflowQueuedPrompt =
            serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
        if queued.status() != WorkflowQueuedPromptStatus::Queued {
            continue;
        }
        if let Some(envelope) = queued
            .publication_invocation()
            .filter(|value| value.transport == "app_event")
        {
            settle(tx, session, &id, envelope, "failed")?;
        }
    }
    Ok(())
}
