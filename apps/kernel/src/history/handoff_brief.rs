//! Each agent's provider-switch handoff brief and the history it covers.

use rusqlite::{params, OptionalExtension};

use crate::error::DaemonError;

use super::{unix_epoch_ms, OperationalHistoryStore};

/// A model-written summary of an agent's conversation through
/// `covered_through_sequence`; later history updates it incrementally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentHandoffBrief {
    pub(crate) brief: String,
    pub(crate) covered_through_sequence: u64,
}

impl OperationalHistoryStore {
    /// Invalidating a cached observation also removes its incremental watermark.
    pub(crate) fn delete_agent_handoff_brief(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        connection
            .execute(
                "DELETE FROM history_agent_handoff_briefs WHERE session_id = ?1 AND agent_id = ?2",
                params![session_id, agent_id],
            )
            .map(|_| ())
            .map_err(|error| handoff_brief_error(session_id, "invalidate handoff brief", error))
    }

    pub(crate) fn load_agent_handoff_brief(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<AgentHandoffBrief>, DaemonError> {
        let connection = self.lock_read_connection(Some(session_id))?;
        connection
            .query_row(
                "SELECT brief, covered_through_sequence FROM history_agent_handoff_briefs
                 WHERE session_id = ?1 AND agent_id = ?2",
                params![session_id, agent_id],
                |row| {
                    Ok(AgentHandoffBrief {
                        brief: row.get(0)?,
                        covered_through_sequence: row.get::<_, i64>(1)?.max(0) as u64,
                    })
                },
            )
            .optional()
            .map_err(|error| handoff_brief_error(session_id, "load handoff brief", error))
    }

    /// Stores `brief` unless a brief covering more history is already stored.
    pub(crate) fn save_agent_handoff_brief(
        &self,
        session_id: &str,
        agent_id: &str,
        brief: &AgentHandoffBrief,
    ) -> Result<(), DaemonError> {
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        connection
            .execute(
                "INSERT INTO history_agent_handoff_briefs
                     (session_id, agent_id, covered_through_sequence, brief, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (session_id, agent_id) DO UPDATE SET
                     covered_through_sequence = excluded.covered_through_sequence,
                     brief = excluded.brief,
                     updated_at_ms = excluded.updated_at_ms
                 WHERE excluded.covered_through_sequence >= covered_through_sequence",
                params![
                    session_id,
                    agent_id,
                    brief.covered_through_sequence as i64,
                    brief.brief,
                    unix_epoch_ms() as i64
                ],
            )
            .map(|_| ())
            .map_err(|error| handoff_brief_error(session_id, "save handoff brief", error))
    }
}

fn handoff_brief_error(
    session_id: &str,
    operation: &'static str,
    error: rusqlite::Error,
) -> DaemonError {
    DaemonError::SessionHistoryFailed {
        session_id: Some(session_id.to_string()),
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_brief_never_moves_its_watermark_back() {
        let path = std::env::temp_dir().join(format!(
            "chariox-handoff-brief-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store =
            OperationalHistoryStore::open(path.clone()).expect("operational history should open");
        let brief = |text: &str, sequence| AgentHandoffBrief {
            brief: text.to_string(),
            covered_through_sequence: sequence,
        };

        assert_eq!(
            store.load_agent_handoff_brief("session", "agent").unwrap(),
            None
        );
        store
            .save_agent_handoff_brief("session", "agent", &brief("first", 10))
            .unwrap();
        store
            .save_agent_handoff_brief("session", "agent", &brief("stale", 4))
            .unwrap();
        store
            .save_agent_handoff_brief("session", "other", &brief("other", 2))
            .unwrap();
        assert_eq!(
            store.load_agent_handoff_brief("session", "agent").unwrap(),
            Some(brief("first", 10))
        );
        store
            .save_agent_handoff_brief("session", "agent", &brief("later", 20))
            .unwrap();
        assert_eq!(
            store.load_agent_handoff_brief("session", "agent").unwrap(),
            Some(brief("later", 20))
        );
        drop(store);
        for path in [
            path.clone(),
            path.with_extension("db-wal"),
            path.with_extension("db-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}
