use std::collections::BTreeSet;

use rusqlite::params;

use super::{
    history_event_kind_key, DaemonError, HistoryEvent, HistoryEventKind, HistoryEventTurnContext,
    OperationalHistoryStore, SessionHistoryEntry,
};

impl OperationalHistoryStore {
    pub fn append_missing_legacy_transcripts(
        &self,
        entries: &[SessionHistoryEntry],
    ) -> Result<Vec<HistoryEvent>, DaemonError> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let session_id = entries[0].session_id.as_str();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let mut statement = connection
            .prepare(
                "SELECT agent_id, kind, timestamp_ms, content, event_json
             FROM history_events
             WHERE session_id = ?1",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "prepare operational legacy import lookup",
                message: error.to_string(),
            })?;
        let mut rows = statement.query(params![session_id]).map_err(|error| {
            DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "query operational legacy import lookup",
                message: error.to_string(),
            }
        })?;
        let mut existing_merge_keys = BTreeSet::new();
        let mut existing_exact_entries = BTreeSet::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "read operational legacy import lookup",
                message: error.to_string(),
            })?
        {
            let agent_id = row.get::<_, Option<String>>(0).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "decode operational legacy import agent",
                    message: error.to_string(),
                }
            })?;
            let kind =
                row.get::<_, String>(1)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.to_string()),
                        operation: "decode operational legacy import kind",
                        message: error.to_string(),
                    })?;
            let timestamp_ms =
                row.get::<_, i64>(2)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.to_string()),
                        operation: "decode operational legacy import timestamp",
                        message: error.to_string(),
                    })?;
            let content = row.get::<_, Option<String>>(3).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "decode operational legacy import content",
                    message: error.to_string(),
                }
            })?;
            let event_json =
                row.get::<_, String>(4)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.to_string()),
                        operation: "decode operational legacy import event json",
                        message: error.to_string(),
                    })?;
            if let Ok(event) = serde_json::from_str::<HistoryEvent>(&event_json) {
                if let Some(merge_key) = event
                    .metadata
                    .get("merge_key")
                    .and_then(|value| value.as_str())
                {
                    existing_merge_keys.insert(merge_key.to_string());
                }
            }
            existing_exact_entries.insert(legacy_transcript_identity(
                agent_id.as_deref(),
                &kind,
                timestamp_ms.max(0) as u64,
                content.as_deref().unwrap_or_default(),
            ));
        }
        drop(rows);
        drop(statement);
        drop(connection);

        let missing = entries
            .iter()
            .filter(|entry| {
                entry
                    .merge_key
                    .as_deref()
                    .is_none_or(|merge_key| !existing_merge_keys.contains(merge_key))
                    && !existing_exact_entries.contains(&legacy_transcript_identity(
                        entry.agent_id.as_deref(),
                        history_event_kind_key(HistoryEventKind::from(entry.kind)),
                        entry.timestamp_ms,
                        &entry.text,
                    ))
            })
            .map(|entry| (entry, HistoryEventTurnContext::default()))
            .collect::<Vec<_>>();
        self.append_transcripts(missing)
    }
}

fn legacy_transcript_identity(
    agent_id: Option<&str>,
    kind: &str,
    timestamp_ms: u64,
    content: &str,
) -> (Option<String>, String, u64, String) {
    (
        agent_id.map(str::to_string),
        kind.to_string(),
        timestamp_ms,
        content.to_string(),
    )
}
