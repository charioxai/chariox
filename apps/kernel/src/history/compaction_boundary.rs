//! MP-08 / MP-10 / MP-11: durable recovery epochs for native session compaction.
use super::OperationalHistoryStore;
use crate::error::DaemonError;
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone)]
pub(crate) struct AgentCompactionBoundary {
    pub(crate) revision: u64,
    pub(crate) source_provider: String,
    pub(crate) source_model: String,
}

impl OperationalHistoryStore {
    pub(crate) fn record_agent_compaction_boundary(
        &self,
        session_id: &str,
        agent_id: &str,
        provider: &str,
        model: &str,
        account: &str,
        native_session: Option<&str>,
    ) -> Result<u64, DaemonError> {
        let family = crate::provider::canonical_provider_family(provider).unwrap_or(provider);
        let connection = self
            .connection
            .lock()
            .map_err(|e| compaction_error(session_id, "lock compaction boundary", e))?;
        connection.execute(
            "INSERT INTO history_agent_compaction_boundaries
                 (session_id, agent_id, revision, delivered_revision, provider_family, source_provider, source_model, account_profile, provider_session_id)
             VALUES (?1, ?2, 1, 0, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (session_id, agent_id) DO UPDATE SET
                 revision = revision + 1, provider_family = excluded.provider_family,
                 source_provider = excluded.source_provider, source_model = excluded.source_model,
                 account_profile = excluded.account_profile, provider_session_id = excluded.provider_session_id",
            params![session_id, agent_id, family, provider, model, account, native_session],
        ).map_err(|e| compaction_error(session_id, "record compaction boundary", e))?;
        connection.query_row("SELECT revision FROM history_agent_compaction_boundaries WHERE session_id = ?1 AND agent_id = ?2",
            params![session_id, agent_id], |row| row.get::<_, i64>(0))
            .map(|revision| revision.max(0) as u64).map_err(|e| compaction_error(session_id, "read compaction epoch", e))
    }

    pub(crate) fn load_agent_compaction_boundary(
        &self,
        session_id: &str,
        agent_id: &str,
        provider: &str,
        account: &str,
        native_session: Option<&str>,
    ) -> Result<Option<AgentCompactionBoundary>, DaemonError> {
        let family = crate::provider::canonical_provider_family(provider).unwrap_or(provider);
        let connection = self.lock_read_connection(Some(session_id))?;
        connection.query_row(
            "SELECT revision, source_provider, source_model FROM history_agent_compaction_boundaries
             WHERE session_id = ?1 AND agent_id = ?2 AND revision > delivered_revision
               AND provider_family = ?3 AND account_profile = ?4
               AND (provider_session_id IS NULL OR provider_session_id = ?5)",
            params![session_id, agent_id, family, account, native_session],
            |row| Ok(AgentCompactionBoundary { revision: row.get::<_, i64>(0)?.max(0) as u64,
                source_provider: row.get(1)?, source_model: row.get(2)? }),
        ).optional().map_err(|e| compaction_error(session_id, "load compaction boundary", e))
    }

    /// A late acknowledgement must never erase a newer compaction epoch.
    pub(crate) fn deliver_agent_compaction_boundary(
        &self,
        session_id: &str,
        agent_id: &str,
        revision: u64,
    ) -> Result<(), DaemonError> {
        let connection = self
            .connection
            .lock()
            .map_err(|e| compaction_error(session_id, "lock compaction acknowledgement", e))?;
        connection
            .execute(
                "UPDATE history_agent_compaction_boundaries SET delivered_revision = ?3
             WHERE session_id = ?1 AND agent_id = ?2 AND revision = ?3",
                params![session_id, agent_id, revision as i64],
            )
            .map(|_| ())
            .map_err(|e| compaction_error(session_id, "acknowledge compaction boundary", e))
    }
}

fn compaction_error(
    session_id: &str,
    operation: &'static str,
    error: impl std::fmt::Display,
) -> DaemonError {
    DaemonError::SessionHistoryFailed {
        session_id: Some(session_id.into()),
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compaction_epoch_survives_reopen_and_ignores_stale_acknowledgement() {
        let root = crate::test_support::TestWorktree::new("durable-compaction-epoch");
        let path = root.path().join("history.db");
        let store = OperationalHistoryStore::open(path.clone()).unwrap();
        let first = store
            .record_agent_compaction_boundary(
                "s",
                "a",
                "claude-p",
                "sonnet",
                "account",
                Some("native"),
            )
            .unwrap();
        drop(store);
        let store = OperationalHistoryStore::open(path).unwrap();
        let load = |provider, account, native| {
            store
                .load_agent_compaction_boundary("s", "a", provider, account, native)
                .unwrap()
        };
        assert_eq!(
            load("claude", "account", Some("native")).unwrap().revision,
            first
        );
        assert!(load("claude", "other-account", Some("native")).is_none());
        assert!(load("claude", "account", Some("other-native")).is_none());
        assert!(load("codex", "account", Some("native")).is_none());
        let second = store
            .record_agent_compaction_boundary(
                "s",
                "a",
                "claude",
                "sonnet",
                "account",
                Some("native"),
            )
            .unwrap();
        assert!(second > first);
        store
            .deliver_agent_compaction_boundary("s", "a", first)
            .unwrap();
        assert_eq!(
            load("claude-headless", "account", Some("native"))
                .unwrap()
                .revision,
            second
        );
        store
            .deliver_agent_compaction_boundary("s", "a", second)
            .unwrap();
        assert!(load("claude", "account", Some("native")).is_none());
        let third = store
            .record_agent_compaction_boundary(
                "s",
                "a",
                "claude",
                "sonnet",
                "account",
                Some("native"),
            )
            .unwrap();
        assert!(third > second);
        store
            .deliver_agent_compaction_boundary("s", "a", second)
            .unwrap();
        assert_eq!(
            load("claude", "account", Some("native")).unwrap().revision,
            third
        );
    }
}
