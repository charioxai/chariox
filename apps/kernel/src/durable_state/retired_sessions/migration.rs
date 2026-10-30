//! Reconcile historical Rooms before checkpoints become live-only state.

use std::collections::BTreeMap;

use super::*;
use crate::durable_state::{DurableStateSnapshot, DurableWriteOperation};

const RECEIPT: &str = "retired_sessions_backfill";

fn error(message: impl ToString) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "durable_state.backfill_retired_sessions",
        message: message.to_string(),
    }
}

impl DurableKernelStateStore {
    pub(crate) fn backfill_retired_sessions(
        &self,
        owner: &str,
        snapshot: Option<&DurableStateSnapshot>,
    ) -> Result<(), DaemonError> {
        let mut records = BTreeMap::new();
        let latest_sequence = self
            .latest_event_sequence()?
            .max(snapshot.map_or(0, |snapshot| snapshot.sequence));
        let verified_sequence;
        {
            let connection = self.lock_connection("durable_state.backfill_retired_sessions")?;
            let receipt: Option<String> = connection
                .query_row(
                    "SELECT metadata_value FROM durable_state_metadata
                     WHERE owner_id=?1 AND metadata_key=?2",
                    params![owner, RECEIPT],
                    |row| row.get(0),
                )
                .optional()
                .map_err(error)?;
            // A predecessor can append lifecycle events without maintaining
            // the index or receipt. Trust only the prefix we reconciled.
            verified_sequence = match receipt.as_deref() {
                None | Some("verified") => None,
                Some(value) => Some(value.parse::<u64>().map_err(error)?),
            }
            .filter(|sequence| *sequence <= latest_sequence);
            if verified_sequence == Some(latest_sequence) {
                return Ok(());
            }
            // Existing indexed records survive newer live-only checkpoints.
            let mut query = connection
                .prepare("SELECT payload FROM retired_sessions WHERE owner_id=?1")
                .map_err(error)?;
            let rows = query
                .query_map([owner], |row| row.get::<_, String>(0))
                .map_err(error)?;
            for row in rows {
                let session = decode(row.map_err(error)?)?;
                records.insert(session.id().to_string(), session);
            }
        }
        let mut cursor = verified_sequence.unwrap_or(0);
        let mut checkpoint_applied = snapshot.is_none_or(|snapshot| {
            verified_sequence.is_some_and(|sequence| snapshot.sequence <= sequence)
        });
        loop {
            let events = self.load_events_after_batch(cursor, 512)?;
            if events.is_empty() {
                break;
            }
            for event in events {
                if !checkpoint_applied
                    && snapshot.is_some_and(|snapshot| event.sequence > snapshot.sequence)
                {
                    apply_checkpoint(&mut records, owner, snapshot.expect("checkpoint exists"))?;
                    checkpoint_applied = true;
                }
                cursor = event.sequence;
                if matches!(
                    event.kind.as_str(),
                    "session.created" | "session.updated" | "session.ended" | "session.deleted"
                ) {
                    if let Some(payload) = event.payload.get("session") {
                        apply_session(
                            &mut records,
                            owner,
                            payload,
                            event.kind == "session.deleted",
                        )?;
                    }
                } else if event.kind == "sessions.updated" {
                    for payload in event
                        .payload
                        .get("sessions")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        apply_session(&mut records, owner, payload, false)?;
                    }
                }
            }
        }
        if !checkpoint_applied {
            apply_checkpoint(&mut records, owner, snapshot.expect("checkpoint exists"))?;
        }
        let records = records
            .into_values()
            .map(|session| {
                Ok((
                    session.id().to_string(),
                    session.project_id().to_string(),
                    serde_json::to_string(&session).map_err(error)?,
                ))
            })
            .collect::<Result<Vec<_>, DaemonError>>()?;
        // The rows and verified log position commit together. A failed startup
        // can retry; later boots reconcile only the suffix, so older checkpoints
        // cannot overwrite subsequent restores or deletes.
        self.writer
            .execute(DurableWriteOperation::RetiredSessionBackfill {
                owner_id: owner.to_string(),
                records,
                sequence: latest_sequence,
            })?;
        Ok(())
    }
}

fn apply_checkpoint(
    records: &mut BTreeMap<String, RuntimeSession>,
    owner: &str,
    snapshot: &DurableStateSnapshot,
) -> Result<(), DaemonError> {
    for payload in snapshot
        .payload
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        apply_session(records, owner, payload, false)?;
    }
    Ok(())
}

fn apply_session(
    records: &mut BTreeMap<String, RuntimeSession>,
    owner: &str,
    payload: &serde_json::Value,
    deleted: bool,
) -> Result<(), DaemonError> {
    if payload
        .get("host_daemon_id")
        .and_then(serde_json::Value::as_str)
        != Some(owner)
    {
        return Ok(());
    }
    let session: RuntimeSession = serde_json::from_value(payload.clone()).map_err(error)?;
    if !deleted && session.status() == SessionStatus::Ended {
        records.insert(session.id().to_string(), session);
    } else {
        records.remove(session.id());
    }
    Ok(())
}

pub(in crate::durable_state) fn commit_backfill(
    transaction: &Transaction<'_>,
    owner: &str,
    records: &[(String, String, String)],
    sequence: u64,
) -> rusqlite::Result<u64> {
    transaction.execute("DELETE FROM retired_sessions WHERE owner_id=?1", [owner])?;
    for (id, project, payload) in records {
        transaction.execute("INSERT INTO retired_sessions(owner_id,session_id,project_id,payload) VALUES (?1,?2,?3,?4)", params![owner,id,project,payload])?;
    }
    transaction.execute("INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms)
        VALUES (?1,?2,?3,?4) ON CONFLICT(owner_id,metadata_key) DO UPDATE SET metadata_value=excluded.metadata_value,updated_at_ms=excluded.updated_at_ms",
        params![owner,RECEIPT,sequence.to_string(),crate::session::unix_epoch_ms() as i64])?;
    Ok(records.len() as u64)
}
