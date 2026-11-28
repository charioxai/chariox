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
    /// that names it. Deletion events are never removed, so the answer comes
    /// from the agents named so far and each call reads only the deletion
    /// events appended since the last one, never the whole deletion history.
    pub(crate) fn agent_deletion_recorded(&self, agent_id: &str) -> Result<bool, DaemonError> {
        let operation = "durable_state.agent_deletion_recorded";
        let connection = self.lock_connection(operation)?;
        let mut deleted = self
            .deleted_agents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let through = deleted.through_sequence;
        // Ordinary deletion uses the agent subject; workflow cleanup uses the
        // session subject and names the agent in the payload.
        let mut newest = through;
        let mut named = Vec::new();
        let mut statement = connection
            .prepare(
                "SELECT sequence, subject_id, json_extract(payload_json, '$.agent.id')
                 FROM durable_state_events
                 WHERE kind = 'agent.deleted' AND sequence > ?1",
            )
            .map_err(|error| storage_error(operation, error))?;
        let rows = statement
            .query_map(params![through], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| storage_error(operation, error))?;
        for row in rows {
            let (sequence, subject, payload_agent) =
                row.map_err(|error| storage_error(operation, error))?;
            newest = newest.max(sequence);
            named.extend(subject.into_iter().chain(payload_agent));
        }
        let mut statement = connection
            .prepare(
                "SELECT event.sequence, agent.value
                 FROM durable_state_events AS event, json_each(event.payload_json, '$.agent_ids') AS agent
                 WHERE event.kind = ?1 AND event.sequence > ?2",
            )
            .map_err(|error| storage_error(operation, error))?;
        let rows = statement
            .query_map(params![AGENT_SUBTREE_DELETING, through], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| storage_error(operation, error))?;
        for row in rows {
            let (sequence, agent) = row.map_err(|error| storage_error(operation, error))?;
            newest = newest.max(sequence);
            named.push(agent);
        }
        deleted.through_sequence = newest;
        deleted.agent_ids.extend(named);
        Ok(deleted.agent_ids.contains(agent_id))
    }
}

/// The agents whose deletion the durable events name, read up to
/// `through_sequence`.
#[derive(Debug, Default)]
pub(crate) struct DeletedAgents {
    through_sequence: i64,
    agent_ids: std::collections::HashSet<String>,
}

fn storage_error(operation: &'static str, error: rusqlite::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    fn scratch_store(name: &str) -> (std::path::PathBuf, DurableKernelStateStore) {
        let root = std::env::temp_dir().join(format!(
            "chariox-deletion-{name}-{}-{}",
            std::process::id(),
            super::super::rand_suffix()
        ));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        (root, store)
    }

    #[test]
    fn a_deletion_is_recorded_by_its_subject_its_payload_or_a_cascade_and_only_then() {
        let (root, store) = scratch_store("kinds");
        assert!(!store.agent_deletion_recorded("agent-own").unwrap());
        store
            .append_event("agent.deleted", Some("agent-own".into()), json!({}))
            .unwrap();
        store
            .append_event(
                "agent.deleted",
                Some("session-1".into()),
                json!({"agent": {"id": "agent-named"}}),
            )
            .unwrap();
        assert!(store.agent_deletion_recorded("agent-own").unwrap());
        assert!(store.agent_deletion_recorded("agent-named").unwrap());
        assert!(!store.agent_deletion_recorded("agent-cascade").unwrap());
        // An event appended after an answer is seen by the next call.
        store
            .append_event(
                AGENT_SUBTREE_DELETING,
                Some("agent-root".into()),
                json!({"agent_ids": ["agent-root", "agent-cascade"]}),
            )
            .unwrap();
        assert!(store.agent_deletion_recorded("agent-cascade").unwrap());
        assert!(!store.agent_deletion_recorded("agent-alive").unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A kernel that has deleted thousands of agents asks about each blocked
    /// task on every maintenance pass: the answer must not re-read that history.
    #[test]
    fn asking_again_does_not_reread_the_deletion_history() {
        let (root, store) = scratch_store("history");
        let filler = "x".repeat(2_000);
        for index in 0..3_000 {
            store
                .append_event(
                    "agent.deleted",
                    Some(format!("session-{index}")),
                    json!({"agent": {"id": format!("agent-gone-{index}")}, "filler": filler}),
                )
                .unwrap();
        }
        let first = Instant::now();
        assert!(!store.agent_deletion_recorded("agent-alive").unwrap());
        let first = first.elapsed();
        let again = Instant::now();
        for _ in 0..50 {
            assert!(!store.agent_deletion_recorded("agent-alive").unwrap());
        }
        let again = again.elapsed() / 50;
        assert!(
            again * 20 < first,
            "a repeated answer took {again:?} against {first:?} for the first"
        );
        assert!(store.agent_deletion_recorded("agent-gone-2999").unwrap());
        let _ = std::fs::remove_dir_all(root);
    }
}
