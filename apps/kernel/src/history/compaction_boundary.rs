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

pub(crate) struct AgentCompactionOrigin<'a> {
    pub(crate) provider: &'a str,
    pub(crate) model: &'a str,
    pub(crate) account: &'a str,
    pub(crate) native_session: Option<&'a str>,
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
        let connection = self
            .connection
            .lock()
            .map_err(|e| compaction_error(session_id, "lock compaction boundary", e))?;
        record_boundary(
            &connection,
            session_id,
            agent_id,
            &AgentCompactionOrigin {
                provider,
                model,
                account,
                native_session,
            },
        )
    }

    /// Transcript snapshots are replayed on reconnect and restart. Their stable
    /// native record IDs must advance the recovery epoch exactly once.
    pub(crate) fn record_observed_agent_compaction_boundary(
        &self,
        session_id: &str,
        agent_id: &str,
        origin: AgentCompactionOrigin<'_>,
        provider_turn_id: &str,
    ) -> Result<bool, DaemonError> {
        let native_session = origin.native_session.ok_or_else(|| {
            compaction_error(
                session_id,
                "record observed compaction",
                "native session ID is required",
            )
        })?;
        let family =
            crate::provider::canonical_provider_family(origin.provider).unwrap_or(origin.provider);
        let mut connection = self
            .connection
            .lock()
            .map_err(|e| compaction_error(session_id, "lock observed compaction", e))?;
        let transaction = connection
            .transaction()
            .map_err(|e| compaction_error(session_id, "begin observed compaction", e))?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO history_agent_compaction_observations
             (session_id, agent_id, provider_family, account_profile, provider_session_id, provider_turn_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, agent_id, family, origin.account, native_session, provider_turn_id],
        ).map_err(|e| compaction_error(session_id, "record observed compaction", e))? > 0;
        if inserted {
            record_boundary(&transaction, session_id, agent_id, &origin)?;
        }
        transaction
            .commit()
            .map_err(|e| compaction_error(session_id, "commit observed compaction", e))?;
        Ok(inserted)
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

fn record_boundary(
    connection: &rusqlite::Connection,
    session_id: &str,
    agent_id: &str,
    origin: &AgentCompactionOrigin<'_>,
) -> Result<u64, DaemonError> {
    let family =
        crate::provider::canonical_provider_family(origin.provider).unwrap_or(origin.provider);
    connection.execute(
        "INSERT INTO history_agent_compaction_boundaries
             (session_id, agent_id, revision, delivered_revision, provider_family, source_provider, source_model, account_profile, provider_session_id)
         VALUES (?1, ?2, 1, 0, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (session_id, agent_id) DO UPDATE SET
             revision = revision + 1, provider_family = excluded.provider_family,
             source_provider = excluded.source_provider, source_model = excluded.source_model,
             account_profile = excluded.account_profile, provider_session_id = excluded.provider_session_id",
        params![session_id, agent_id, family, origin.provider, origin.model, origin.account, origin.native_session],
    ).map_err(|e| compaction_error(session_id, "record compaction boundary", e))?;
    connection.query_row("SELECT revision FROM history_agent_compaction_boundaries WHERE session_id = ?1 AND agent_id = ?2",
        params![session_id, agent_id], |row| row.get::<_, i64>(0))
        .map(|revision| revision.max(0) as u64).map_err(|e| compaction_error(session_id, "read compaction epoch", e))
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
    fn observed_compaction_replay_after_acknowledgement_and_restart_is_idempotent() {
        let root = crate::test_support::TestWorktree::new("observed-compaction-replay");
        let path = root.path().join("history.db");
        let origin = || AgentCompactionOrigin {
            provider: "claude",
            model: "haiku",
            account: "account",
            native_session: Some("native"),
        };
        let store = OperationalHistoryStore::open(path.clone()).unwrap();
        assert!(store
            .record_observed_agent_compaction_boundary("s", "a", origin(), "compact-1")
            .unwrap());
        store
            .deliver_agent_compaction_boundary("s", "a", 1)
            .unwrap();
        drop(store);
        let store = OperationalHistoryStore::open(path).unwrap();
        assert!(!store
            .record_observed_agent_compaction_boundary("s", "a", origin(), "compact-1")
            .unwrap());
        assert!(store
            .load_agent_compaction_boundary("s", "a", "claude", "account", Some("native"))
            .unwrap()
            .is_none());
        assert!(store
            .record_observed_agent_compaction_boundary("s", "a", origin(), "compact-2")
            .unwrap());
        assert!(!store
            .record_observed_agent_compaction_boundary("s", "a", origin(), "compact-1")
            .unwrap());
        assert_eq!(
            store
                .load_agent_compaction_boundary("s", "a", "claude", "account", Some("native"))
                .unwrap()
                .unwrap()
                .revision,
            2
        );
    }
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
