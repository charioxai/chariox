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
        leased_agent_id TEXT NOT NULL, replay_prompt_id TEXT, PRIMARY KEY (session_id, agent_id)
    );
    CREATE TABLE IF NOT EXISTS remote_lease_recreated (
        session_id TEXT NOT NULL, agent_id TEXT NOT NULL,
        worker_kernel_id TEXT NOT NULL, execution_lease_id TEXT NOT NULL,
        leased_agent_id TEXT NOT NULL, PRIMARY KEY (session_id, agent_id)
    )",
        )
        .map_err(|error| history_error("migrate remote lease context", error))?;
    Ok(())
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
                "INSERT INTO remote_lease_context_pending (session_id, agent_id, worker_kernel_id, execution_lease_id, leased_agent_id) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT (session_id, agent_id) DO UPDATE SET
            worker_kernel_id = excluded.worker_kernel_id,
            execution_lease_id = excluded.execution_lease_id,
            leased_agent_id = excluded.leased_agent_id, replay_prompt_id = NULL",
                params![
                    session_id,
                    agent_id,
                    binding.worker_kernel_id,
                    binding.execution_lease_id,
                    binding.leased_agent_id
                ],
            )
            .map_err(|error| history_error("mark remote lease context", error))?;
        connection
            .execute(
                "INSERT INTO remote_lease_recreated (session_id, agent_id, worker_kernel_id, execution_lease_id, leased_agent_id) VALUES (?1, ?2, ?3, ?4, ?5)
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

    /// A recreated lease's worker history starts at its recreation, so every
    /// new provider session on it needs the home conversation again.
    pub(crate) fn rearm_recreated_remote_lease_context(
        &self,
        session_id: &str,
        agent_id: &str,
        binding: &RemoteAgentBinding,
    ) -> Result<(), DaemonError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| history_error("rearm remote lease context", "history lock poisoned"))?;
        connection
            .execute(
                "INSERT INTO remote_lease_context_pending (session_id, agent_id, worker_kernel_id, execution_lease_id, leased_agent_id)
            SELECT session_id, agent_id, worker_kernel_id, execution_lease_id, leased_agent_id FROM remote_lease_recreated
            WHERE session_id = ?1 AND agent_id = ?2 AND worker_kernel_id = ?3
            AND execution_lease_id = ?4 AND leased_agent_id = ?5
            ON CONFLICT (session_id, agent_id) DO UPDATE SET replay_prompt_id = NULL",
                params![
                    session_id,
                    agent_id,
                    binding.worker_kernel_id,
                    binding.execution_lease_id,
                    binding.leased_agent_id
                ],
            )
            .map_err(|error| history_error("rearm remote lease context", error))?;
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

    pub(crate) fn record_remote_lease_context_replay(
        &self,
        session_id: &str,
        agent_id: &str,
        binding: &RemoteAgentBinding,
        prompt_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| history_error("record remote lease replay", "history lock poisoned"))?;
        connection.execute("UPDATE remote_lease_context_pending SET replay_prompt_id = ?6
            WHERE session_id = ?1 AND agent_id = ?2 AND worker_kernel_id = ?3 AND execution_lease_id = ?4 AND leased_agent_id = ?5",
            params![session_id, agent_id, binding.worker_kernel_id, binding.execution_lease_id, binding.leased_agent_id, prompt_id])
            .map_err(|error| history_error("record remote lease replay", error))?;
        Ok(())
    }

    pub(crate) fn clear_remote_lease_context_if_replayed(
        &self,
        session_id: &str,
        agent_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError> {
        let connection = self.connection.lock().map_err(|_| {
            history_error(
                "clear accepted remote lease replay",
                "history lock poisoned",
            )
        })?;
        connection.execute("DELETE FROM remote_lease_context_pending
            WHERE session_id = ?1 AND agent_id = ?2 AND worker_kernel_id = ?3 AND leased_agent_id = ?4 AND replay_prompt_id = ?5",
            params![session_id, agent_id, worker_kernel_id, leased_agent_id, prompt_id])
            .map_err(|error| history_error("clear accepted remote lease replay", error))?;
        Ok(())
    }

    #[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review976r2_replay_ack_survives_store_reopen_and_matches_exact_prompt() {
        let root = std::env::temp_dir().join(format!(
            "lease-replay-ack-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("history.db");
        let store = OperationalHistoryStore::open(path.clone()).unwrap();
        let binding = RemoteAgentBinding {
            worker_kernel_id: "worker".into(),
            worker_machine_id: "machine".into(),
            execution_lease_id: "execution".into(),
            leased_agent_id: "leased-agent".into(),
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: None,
        };
        store
            .mark_remote_lease_context_pending("session", "agent", &binding)
            .unwrap();
        store
            .record_remote_lease_context_replay("session", "agent", &binding, Some("replay-prompt"))
            .unwrap();
        drop(store);
        let store = OperationalHistoryStore::open(path).unwrap();
        store
            .clear_remote_lease_context_if_replayed(
                "session",
                "agent",
                "worker",
                "leased-agent",
                "fresh-workflow",
            )
            .unwrap();
        assert!(store
            .remote_lease_context_pending("session", "agent", &binding)
            .unwrap());
        // A retry that withholds replay supersedes the pre-send preparation.
        store
            .record_remote_lease_context_replay("session", "agent", &binding, None)
            .unwrap();
        store
            .clear_remote_lease_context_if_replayed(
                "session",
                "agent",
                "worker",
                "leased-agent",
                "replay-prompt",
            )
            .unwrap();
        assert!(store
            .remote_lease_context_pending("session", "agent", &binding)
            .unwrap());
        store
            .record_remote_lease_context_replay("session", "agent", &binding, Some("retry-prompt"))
            .unwrap();
        store
            .clear_remote_lease_context_if_replayed(
                "session",
                "agent",
                "worker",
                "leased-agent",
                "retry-prompt",
            )
            .unwrap();
        assert!(!store
            .remote_lease_context_pending("session", "agent", &binding)
            .unwrap());
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
