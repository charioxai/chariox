//! MP-08 / MP-10 / MP-11: bounded lookup of one persisted turn, off the stream replay path.
use super::OperationalHistoryStore;
use crate::error::DaemonError;
use rusqlite::OptionalExtension;

impl OperationalHistoryStore {
    pub(crate) fn latest_turn_usage(
        &self,
        session: &str,
        prompt: &str,
        run: &str,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        let connection = self.lock_read_connection(Some(session))?;
        let result: Option<String> = connection
            .query_row(
                "SELECT json_extract(event_json, '$.metadata.provider_usage_accounting_v1')
             FROM history_events WHERE session_id = ?1 AND prompt_id = ?2 AND provider_run_id = ?3
             AND json_extract(event_json, '$.metadata.provider_usage_accounting_v1') IS NOT NULL
             ORDER BY sequence DESC LIMIT 1",
                rusqlite::params![session, prompt, run],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.into()),
                operation: "read latest turn usage",
                message: error.to_string(),
            })?;
        result
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session.into()),
                    operation: "decode latest turn usage",
                    message: error.to_string(),
                })
            })
            .transpose()
    }
}
