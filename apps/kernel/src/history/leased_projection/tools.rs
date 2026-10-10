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
    #[serde(default)]
    delta_offsets: std::collections::BTreeMap<String, u64>,
    revision: u64,
}

impl ToolProjectionState {
    pub(crate) fn record(
        &mut self,
        snapshot_key: String,
        bytes: &[u8],
        stable_identity: bool,
    ) -> bool {
        let payload = serde_json::from_slice::<serde_json::Value>(bytes).ok();
        // The producer keeps a separate counter for each output field. A
        // delta payload with multiple fields does not identify which counter
        // it carries, so only use its offset when the field is unambiguous.
        let mut fields = ["output", "stdout", "stderr", "result", "content"]
            .into_iter()
            .filter(|field| {
                payload
                    .as_ref()
                    .is_some_and(|p| p.get(*field).is_some_and(|v| v.is_string()))
            });
        let field = fields.next().filter(|_| fields.next().is_none());
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
                && stable_identity
                && offset.zip(field).is_some_and(|(new, field)| {
                    self.delta_offsets.get(field).is_some_and(|old| new <= *old)
                }))
        {
            return false;
        }
        self.snapshot_key = snapshot_key;
        if replaced || offset.is_none() {
            if let Some(field) = field {
                self.delta_offsets.remove(field);
            } else {
                for field in ["output", "stdout", "stderr", "result", "content"] {
                    if payload
                        .as_ref()
                        .is_some_and(|p| p.get(field).is_some_and(|v| v.is_string()))
                    {
                        self.delta_offsets.remove(field);
                    }
                }
            }
        } else if let Some((offset, field)) = offset.zip(field).filter(|_| stable_identity) {
            self.delta_offsets.insert(field.into(), offset);
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
    pub(crate) identity: Option<String>,
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
         WHERE event_id = (SELECT event_id FROM history_events WHERE session_id = ?3 AND agent_id = ?4 AND provider_run_id = ?5 AND kind = 'provider_tool' AND leased_projection_tool_identity IS ?6 ORDER BY committed_sequence DESC LIMIT 1)",
        params![tool.stream_key, json, tool.session_id, tool.agent_id, tool.provider_run_id, tool.identity])
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

pub(crate) fn tool_identity(merge_key: &Option<String>, bytes: &[u8]) -> Option<String> {
    merge_key.clone().or_else(|| {
        let payload: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        ["id", "call_id"].into_iter().find_map(|field| {
            payload
                .get(field)?
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .map(str::to_string)
        })
    })
}

pub(super) fn tool_identity_sql(prefix: &str) -> String {
    format!("CASE WHEN {prefix}kind = 'provider_tool' THEN COALESCE({prefix}merge_key,
        CASE WHEN json_valid({prefix}content) THEN
            CASE WHEN json_type({prefix}content, '$.id') = 'text' AND trim(json_extract({prefix}content, '$.id')) != '' THEN json_extract({prefix}content, '$.id')
                 WHEN json_type({prefix}content, '$.call_id') = 'text' AND trim(json_extract({prefix}content, '$.call_id')) != '' THEN json_extract({prefix}content, '$.call_id') END
        END) END")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_snapshots_reset_offsets_after_provider_cache_restart_or_eviction() {
        let mut state = ToolProjectionState::default();
        assert!(state.record(
            "old delta".into(),
            br#"{"output":"old tail","chariox_delta_offset_bytes":4000}"#,
            true
        ));
        let json = serde_json::to_string(&state).unwrap();
        let mut state: ToolProjectionState = serde_json::from_str(&json).unwrap();
        assert!(state.record("new base".into(), br#"{"output":"short"}"#, true));
        assert!(state.record(
            "new delta".into(),
            br#"{"output":"tail","chariox_delta_offset_bytes":5}"#,
            true
        ));
        assert!(!state.record(
            "new delta".into(),
            br#"{"output":"tail","chariox_delta_offset_bytes":5}"#,
            true
        ));
        assert!(state.record(
            "unknown old".into(),
            br#"{"output":"tail","chariox_delta_offset_bytes":4000}"#,
            false
        ));
        assert!(state.record(
            "unknown new".into(),
            br#"{"output":"new tail","chariox_delta_offset_bytes":5}"#,
            false
        ));
    }

    #[test]
    fn different_tool_fields_do_not_share_delta_offsets() {
        let mut state = ToolProjectionState::default();
        assert!(state.record(
            "stdout".into(),
            br#"{"stdout":"line","chariox_delta_offset_bytes":1000}"#,
            true
        ));
        let multi =
            br#"{"stdout":"previous output","stderr":"new delta","chariox_delta_offset_bytes":10}"#;
        assert!(state.record("stderr".into(), multi, true));
        assert!(!state.record("stderr".into(), multi, true));
        let next = br#"{"stdout":"previous output","stderr":"next delta","chariox_delta_offset_bytes":20}"#;
        assert!(state.record("next stderr".into(), next, true));
        assert!(state.record(
            "single stderr".into(),
            br#"{"stderr":"new delta","chariox_delta_offset_bytes":30}"#,
            true
        ));
        // The fingerprints and offsets remain durable, even for ambiguous fields.
        let json = serde_json::to_string(&state).unwrap();
        let mut reopened: ToolProjectionState = serde_json::from_str(&json).unwrap();
        assert!(!reopened.record(
            "single stderr".into(),
            br#"{"stderr":"new delta","chariox_delta_offset_bytes":30}"#,
            true
        ));
        assert!(!reopened.record(
            "old stdout".into(),
            br#"{"stdout":"old delta","chariox_delta_offset_bytes":900}"#,
            true
        ));
    }
}
