//! A4: the durable record of a sub-agent cascade. One event names the whole
//! subtree before any of it is torn down, and the deletion pass finishes it.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use rusqlite::params;

pub(crate) const AGENT_SUBTREE_DELETING: &str = "agent.subtree_deleting";

impl DurableKernelStateStore {
    /// Every agent a cascade marked as deleting, each subtree root first.
    pub(crate) fn agents_marked_deleting(&self) -> Result<Vec<String>, DaemonError> {
        let operation = "durable_state.agents_marked_deleting";
        let connection = self.lock_connection(operation)?;
        let mut statement = connection
            .prepare(
                "SELECT agent.value
                 FROM durable_state_events, json_each(payload_json, '$.agent_ids') AS agent
                 WHERE kind = ?1
                 ORDER BY sequence, agent.key",
            )
            .map_err(|error| storage_error(operation, error))?;
        let rows = statement
            .query_map(params![AGENT_SUBTREE_DELETING], |row| row.get(0))
            .map_err(|error| storage_error(operation, error))?;
        rows.collect::<Result<Vec<String>, _>>()
            .map_err(|error| storage_error(operation, error))
    }

    /// Whether an agent's deletion is durable: its own record, or a cascade
    /// that names it.
    pub(crate) fn agent_deletion_recorded(&self, agent_id: &str) -> Result<bool, DaemonError> {
        let operation = "durable_state.agent_deletion_recorded";
        let connection = self.lock_connection(operation)?;
        // Ordinary deletion uses the agent subject; workflow cleanup uses the
        // session subject and names the agent in the payload.
        connection
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM durable_state_events
                     WHERE kind = 'agent.deleted'
                       AND (subject_id = ?1 OR json_extract(payload_json, '$.agent.id') = ?1)
                 ) OR EXISTS(
                     SELECT 1
                     FROM durable_state_events, json_each(payload_json, '$.agent_ids') AS agent
                     WHERE kind = ?2 AND agent.value = ?1
                 )",
                params![agent_id, AGENT_SUBTREE_DELETING],
                |row| row.get(0),
            )
            .map_err(|error| storage_error(operation, error))
    }
}

fn storage_error(operation: &'static str, error: rusqlite::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}
