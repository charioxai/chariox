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
    ) -> Result<Option<u64>, DaemonError> {
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
                return Ok(None);
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
        let had_retired_sessions = !records.is_empty();
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
        if !had_retired_sessions && records.is_empty() {
            // The index already answers correctly. Only the replay cursor is
            // missing, so startup can publish readiness while it commits.
            return Ok(Some(latest_sequence));
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
                records: Some(records),
                sequence: latest_sequence,
            })?;
        Ok(None)
    }

    pub(crate) fn record_retired_session_backfill(
        &self,
        owner: String,
        sequence: u64,
    ) -> Result<(), DaemonError> {
        self.writer
            .execute(DurableWriteOperation::RetiredSessionBackfill {
                owner_id: owner,
                records: None,
                sequence,
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
    records: Option<&[(String, String, String)]>,
    sequence: u64,
) -> rusqlite::Result<u64> {
    if let Some(records) = records {
        transaction.execute("DELETE FROM retired_sessions WHERE owner_id=?1", [owner])?;
        for (id, project, payload) in records {
            transaction.execute("INSERT INTO retired_sessions(owner_id,session_id,project_id,payload) VALUES (?1,?2,?3,?4)", params![owner,id,project,payload])?;
        }
    }
    transaction.execute("INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms)
        VALUES (?1,?2,?3,?4) ON CONFLICT(owner_id,metadata_key) DO UPDATE SET metadata_value=excluded.metadata_value,updated_at_ms=excluded.updated_at_ms",
        params![owner,RECEIPT,sequence.to_string(),crate::session::unix_epoch_ms() as i64])?;
    Ok(records.map_or(0, |records| records.len() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutover_backfill_reads_room_with_inherited_queued_message_projection() {
        let root = crate::test_support::TestWorktree::new("pre-l12-room-checkpoint");
        let path = root.path().join("state.sqlite");
        let room = RuntimeSession::new(
            "room",
            crate::session::ProjectId::test(),
            None,
            "workspace",
            "tree",
            "machine",
            "home",
        );
        let mut stored = serde_json::to_value(&room).unwrap();
        stored["status"] = serde_json::to_value(SessionStatus::Ended).unwrap();
        stored["queued_agent_messages"] = serde_json::json!([{
            "id": "agent-message-1", "agent_id": "agent", "sender": "peer",
            "prompt": "fixture message",
        }]);
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            store
                .save_entity_checkpoint(
                    "home",
                    1,
                    vec![crate::durable_state::DurableCheckpointEntity {
                        kind: "sessions".into(),
                        id: "room".into(),
                        payload_json: stored.to_string(),
                    }],
                )
                .unwrap();
        }

        let store = DurableKernelStateStore::open_owned(path).unwrap();
        let snapshot = store.latest_snapshot_for_owner("home").unwrap().unwrap();
        assert_eq!(
            store
                .backfill_retired_sessions("home", Some(&snapshot))
                .unwrap(),
            None
        );
        let stored_room = &snapshot.payload["sessions"][0];
        assert_eq!(stored_room["id"], "room");
        assert!(stored_room.get("queued_agent_messages").is_none());
        let retired = store.retired_session("home", "room").unwrap().unwrap();
        assert_eq!(retired.status(), SessionStatus::Ended);
    }

    #[test]
    fn deferred_backfill_receipt_preserves_a_room_ended_after_restore() {
        let root = crate::test_support::TestWorktree::new("deferred-retired-room-backfill");
        let path = root.path().join("state.sqlite");
        let store = DurableKernelStateStore::open_owned(path.clone()).unwrap();
        let sequence = store
            .backfill_retired_sessions("home", None)
            .unwrap()
            .unwrap();
        let room = RuntimeSession::new(
            "room",
            crate::session::ProjectId::test(),
            None,
            "workspace",
            "tree",
            "machine",
            "home",
        );
        let mut room = serde_json::to_value(room).unwrap();
        room["status"] = serde_json::to_value(SessionStatus::Ended).unwrap();
        store
            .append_event(
                "session.ended",
                Some("room".into()),
                serde_json::json!({"session": room}),
            )
            .unwrap();

        store
            .record_retired_session_backfill("home".into(), sequence)
            .unwrap();
        assert!(store.retired_session("home", "room").unwrap().is_some());
        drop(store);
        let reopened = DurableKernelStateStore::open_owned(path).unwrap();
        assert_eq!(
            reopened.backfill_retired_sessions("home", None).unwrap(),
            None
        );
        assert!(reopened.retired_session("home", "room").unwrap().is_some());
    }
}
