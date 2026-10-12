//! MP-08 / MP-10 / MP-11: private home bookkeeping for a recreated execution lease.
//! The marker is not a client/relay contract and survives home restart and failed submissions.

use super::OperationalHistoryStore;
use crate::{agent::RemoteAgentBinding, error::DaemonError};
use rusqlite::{params, OptionalExtension};

pub(super) fn migrate(connection: &rusqlite::Connection) -> Result<(), DaemonError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS remote_lease_context_pending (
        session_id TEXT NOT NULL, agent_id TEXT NOT NULL,
        worker_kernel_id TEXT NOT NULL, execution_lease_id TEXT NOT NULL,
        leased_agent_id TEXT NOT NULL, PRIMARY KEY (session_id, agent_id)
    )",
        )
        .map_err(|error| history_error("migrate remote lease context", error))
}

fn history_error(operation: &'static str, error: impl std::fmt::Display) -> DaemonError {
    DaemonError::SessionHistoryFailed {
        session_id: None,
        operation,
        message: error.to_string(),
    }
}

impl OperationalHistoryStore {
    pub(crate) fn mark_remote_lease_context_pending(
        &self,
        session_id: &str,
        agent_id: &str,
        binding: &RemoteAgentBinding,
    ) -> Result<(), DaemonError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| history_error("mark remote lease context", "history lock poisoned"))?;
        connection
            .execute(
                "INSERT INTO remote_lease_context_pending VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT (session_id, agent_id) DO UPDATE SET
            worker_kernel_id = excluded.worker_kernel_id,
            execution_lease_id = excluded.execution_lease_id,
            leased_agent_id = excluded.leased_agent_id",
                params![
                    session_id,
                    agent_id,
                    binding.worker_kernel_id,
                    binding.execution_lease_id,
                    binding.leased_agent_id
                ],
            )
            .map_err(|error| history_error("mark remote lease context", error))?;
        Ok(())
    }

    pub(crate) fn remote_lease_context_pending(
        &self,
        session_id: &str,
        agent_id: &str,
        binding: &RemoteAgentBinding,
    ) -> Result<bool, DaemonError> {
        let connection = self.lock_read_connection(Some(session_id))?;
        connection
            .query_row(
                "SELECT 1 FROM remote_lease_context_pending
            WHERE session_id = ?1 AND agent_id = ?2 AND worker_kernel_id = ?3
            AND execution_lease_id = ?4 AND leased_agent_id = ?5",
                params![
                    session_id,
                    agent_id,
                    binding.worker_kernel_id,
                    binding.execution_lease_id,
                    binding.leased_agent_id
                ],
                |_| Ok(true),
            )
            .optional()
            .map(|pending| pending.unwrap_or(false))
            .map_err(|error| history_error("read remote lease context", error))
    }

    pub(crate) fn clear_remote_lease_context_pending(
        &self,
        session_id: &str,
        agent_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
    ) -> Result<(), DaemonError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| history_error("clear remote lease context", "history lock poisoned"))?;
        connection.execute("DELETE FROM remote_lease_context_pending
            WHERE session_id = ?1 AND agent_id = ?2 AND worker_kernel_id = ?3 AND leased_agent_id = ?4",
            params![session_id, agent_id, worker_kernel_id, leased_agent_id])
            .map_err(|error| history_error("clear remote lease context", error))?;
        Ok(())
    }
}
