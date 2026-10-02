//! Tool terminal deltas and transcript snapshots have different bytes. Keep the
//! latest fingerprint and byte-offset watermark per tool, attached to its history
//! row, rather than a durable hash for every emitted delta.
use super::{operational_history_error, OperationalHistoryStore};
use crate::error::DaemonError;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct ToolProjectionState {
    snapshot_key: String,
    delta_offset: Option<u64>,
    revision: u64,
}

impl ToolProjectionState {
    pub(crate) fn record(&mut self, snapshot_key: String, bytes: &[u8]) -> bool {
        let payload = serde_json::from_slice::<serde_json::Value>(bytes).ok();
        let offset = payload
            .as_ref()
            .and_then(|p| p.get("chariox_delta_offset_bytes"))
            .and_then(|v| v.as_u64());
        let replaced = payload
            .as_ref()
            .and_then(|p| p.get("chariox_output_replaced"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if self.snapshot_key == snapshot_key
            || (!replaced
                && offset
                    .zip(self.delta_offset)
                    .is_some_and(|(new, old)| new <= old))
        {
            return false;
        }
        self.snapshot_key = snapshot_key;
        if replaced {
            self.delta_offset = None;
        } else if offset.is_some() {
            self.delta_offset = offset;
        }
        self.revision += 1;
        true
    }
}

pub(crate) struct ProjectedToolState {
    pub(crate) stream_key: String,
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) merge_key: Option<String>,
    pub(crate) state: ToolProjectionState,
}

impl OperationalHistoryStore {
    pub(crate) fn load_leased_tool_state(
        &self,
        stream_key: &str,
    ) -> Result<ToolProjectionState, DaemonError> {
        self.ensure_leased_projection_schema()?;
        let connection = self.lock_read_connection(None)?;
        let history: Option<String> = connection.query_row(
            "SELECT leased_projection_snapshot_key FROM history_events WHERE leased_projection_stream_key = ?1 AND kind = 'provider_tool' ORDER BY committed_sequence DESC LIMIT 1",
            [stream_key], |row| row.get(0)).optional()
            .map_err(|e| operational_history_error("load tool projection history", e))?;
        let pending: Option<String> = connection
            .query_row(
                "SELECT state_json FROM leased_projection_tools WHERE stream_key = ?1",
                [stream_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| operational_history_error("load pending tool projection", e))?;
        [history, pending]
            .into_iter()
            .flatten()
            .map(|json| {
                serde_json::from_str::<ToolProjectionState>(&json)
                    .map_err(|e| operational_history_error("decode tool projection", e))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|states| {
                states
                    .into_iter()
                    .max_by_key(|s| s.revision)
                    .unwrap_or_default()
            })
    }

    #[cfg(test)]
    pub(crate) fn pending_leased_projection_counts(&self) -> (i64, i64) {
        self.ensure_leased_projection_schema().unwrap();
        self.lock_read_connection(None).unwrap().query_row(
            "SELECT (SELECT count(*) FROM leased_projection_pending_keys), (SELECT count(*) FROM leased_projection_tools)", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
    }
}

pub(super) fn compact_tool_state(
    connection: &rusqlite::Connection,
    projection_id: &str,
    tool: &ProjectedToolState,
) -> Result<(), DaemonError> {
    let json = serde_json::to_string(&tool.state)
        .map_err(|e| operational_history_error("encode tool projection", e))?;
    let updated = connection.execute(
        "UPDATE history_events SET leased_projection_stream_key = ?1, leased_projection_snapshot_key = ?2
         WHERE event_id = (SELECT event_id FROM history_events WHERE session_id = ?3 AND agent_id = ?4 AND provider_run_id = ?5 AND kind = 'provider_tool' AND merge_key IS ?6 ORDER BY committed_sequence DESC LIMIT 1)",
        params![tool.stream_key, json, tool.session_id, tool.agent_id, tool.provider_run_id, tool.merge_key])
        .map_err(|e| operational_history_error("compact tool projection", e))?;
    if updated == 0 {
        connection.execute("INSERT INTO leased_projection_tools VALUES (?1, ?2, ?3) ON CONFLICT(stream_key) DO UPDATE SET state_json = excluded.state_json", params![tool.stream_key, projection_id, json])
            .map_err(|e| operational_history_error("save pending tool projection", e))?;
    } else {
        connection
            .execute(
                "DELETE FROM leased_projection_tools WHERE stream_key = ?1",
                [&tool.stream_key],
            )
            .map_err(|e| operational_history_error("retire pending tool projection", e))?;
    }
    Ok(())
}
