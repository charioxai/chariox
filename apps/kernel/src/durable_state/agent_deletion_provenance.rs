//! MP-11: ownership of retired agents from kernel-owned deletion events.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use rusqlite::{params, OptionalExtension};

impl DurableKernelStateStore {
    pub(crate) fn deleted_agent_is_owned(
        &self,
        session_id: &str,
        agent_id: &str,
        user_id: &str,
    ) -> Result<bool, DaemonError> {
        let operation = "durable_state.deleted_agent_is_owned";
        let connection = self.lock_connection(operation)?;
        // Ordinary deletion uses the agent subject; workflow cleanup uses the
        // session subject. Filter by payload identity before limiting the rows.
        let provenance: Option<(Option<String>, Option<String>)> = connection
            .query_row(
                "SELECT json_extract(payload_json, '$.agent.session_id'),
                        json_extract(payload_json, '$.agent.owner_user_id')
                 FROM durable_state_events
                 WHERE kind = 'agent.deleted' AND subject_id IN (?1, ?2)
                   AND json_valid(payload_json)
                   AND json_extract(payload_json, '$.agent.id') = ?2
                 ORDER BY sequence DESC LIMIT 1",
                params![session_id, agent_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| DaemonError::LocalTransport {
                operation,
                message: error.to_string(),
            })?;
        Ok(provenance.is_some_and(|(session, owner)| {
            session.as_deref() == Some(session_id) && owner.as_deref() == Some(user_id)
        }))
    }
}
