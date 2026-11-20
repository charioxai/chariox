//! The owner hand-off's authority lives in its domain events.
use super::*;
use crate::session::RuntimeHandoff;

pub(crate) mod event_kind {
    pub(crate) const OPENED: &str = "handoff.opened";
    pub(crate) const CLAIMED: &str = "handoff.claimed";
    pub(crate) const OUTCOME: &str = "handoff.outcome";
}

pub(crate) struct OpenOwnerHandoff {
    pub(crate) room: String,
    pub(crate) handoff: RuntimeHandoff,
}

fn decode(payload: serde_json::Value) -> Result<OpenOwnerHandoff, DaemonError> {
    let room = payload["room"].as_str().map(str::to_owned);
    let handoff = serde_json::from_value(payload["handoff"].clone()).ok();
    match (room, handoff) {
        (Some(room), Some(handoff)) => Ok(OpenOwnerHandoff { room, handoff }),
        _ => Err(DaemonError::LocalTransport {
            operation: "durable_state.owner_handoff",
            message: "The saved owner hand-off is unreadable".into(),
        }),
    }
}

impl DurableKernelStateStore {
    pub(crate) fn record_open_owner_handoff(
        &self,
        room: &str,
        handoff: &RuntimeHandoff,
    ) -> Result<(), DaemonError> {
        self.append_event(
            event_kind::OPENED,
            Some(RuntimeHandoff::interaction_id(&handoff.obligation_id)),
            serde_json::json!({"room": room, "handoff": handoff}),
        )?;
        Ok(())
    }

    pub(crate) fn owner_handoff(&self, id: &str) -> Result<Option<OpenOwnerHandoff>, DaemonError> {
        self.load_subject_events_by_kind(id, event_kind::OPENED, 1)?
            .pop()
            .map(|event| decode(event.payload))
            .transpose()
    }

    pub(crate) fn pending_owner_handoffs(
        &self,
        now: u64,
    ) -> Result<Vec<OpenOwnerHandoff>, DaemonError> {
        let sql_error = |error: rusqlite::Error| DaemonError::LocalTransport {
            operation: "durable_state.pending_owner_handoffs",
            message: error.to_string(),
        };
        let connection = self.lock_connection("durable_state.pending_owner_handoffs")?;
        let mut query = connection
            .prepare(
                "SELECT opened.payload_json FROM durable_state_events opened
             WHERE opened.kind=?1 AND opened.timestamp_ms>=?2
               AND json_extract(opened.payload_json,'$.handoff.expires_at_ms')>?3
               AND NOT EXISTS (SELECT 1 FROM durable_state_events ended
                 WHERE ended.kind IN (?4,?5) AND ended.subject_id=opened.subject_id)",
            )
            .map_err(sql_error)?;
        let now = i64::try_from(now).map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.pending_owner_handoffs",
            message: error.to_string(),
        })?;
        let earliest = now.saturating_sub((crate::session::HANDOFF_MAX_TIMEOUT_SEC * 1_000) as i64);
        let rows = query
            .query_map(
                params![
                    event_kind::OPENED,
                    earliest,
                    now,
                    event_kind::CLAIMED,
                    event_kind::OUTCOME
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(sql_error)?;
        rows.map(|row| {
            let payload = serde_json::from_str(&row.map_err(sql_error)?).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "durable_state.pending_owner_handoffs",
                    message: error.to_string(),
                }
            })?;
            decode(payload)
        })
        .collect()
    }
}
