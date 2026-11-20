//! MP-08 / MP-10 / MP-11: bounded chronological journal reads for durable skill loads.
use super::{DurableKernelStateStore, DurableStateEvent};
use crate::error::DaemonError;
use rusqlite::params;
impl DurableKernelStateStore {
    pub(crate) fn load_subject_events_by_kind_after(
        &self,
        subject_id: &str,
        kind: &str,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Vec<DurableStateEvent>, DaemonError> {
        let limit = limit.clamp(1, 200);
        let after_sequence =
            i64::try_from(after_sequence).map_err(|_| DaemonError::LocalTransport {
                operation: "durable_state.load_subject_events_by_kind_after",
                message: "event cursor exceeds durable sequence range".into(),
            })?;
        let connection = self.lock_connection("durable_state.load_subject_events_by_kind_after")?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, event_id, kind, subject_id, timestamp_ms, payload_json
                 FROM durable_state_events
                 WHERE subject_id = ?1 AND kind = ?2 AND sequence > ?3
                 ORDER BY sequence ASC
                 LIMIT ?4",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_subject_events_by_kind_after",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![subject_id, kind, after_sequence, limit as i64])
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_subject_events_by_kind_after",
                message: error.to_string(),
            })?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.load_subject_events_by_kind_after",
            message: error.to_string(),
        })? {
            let payload_json =
                row.get::<_, String>(5)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "durable_state.load_subject_events_by_kind_after",
                        message: error.to_string(),
                    })?;
            events.push(DurableStateEvent {
                sequence: row.get::<_, i64>(0).unwrap_or_default().max(0) as u64,
                event_id: row.get::<_, String>(1).unwrap_or_default(),
                kind: row.get::<_, String>(2).unwrap_or_default(),
                subject_id: row.get::<_, Option<String>>(3).unwrap_or_default(),
                timestamp_ms: row.get::<_, i64>(4).unwrap_or_default().max(0) as u64,
                payload: serde_json::from_str(&payload_json).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "durable_state.decode_event",
                        message: error.to_string(),
                    }
                })?,
            });
        }
        Ok(events)
    }
}
