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
        let native_session = origin.native_session.unwrap_or("");
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
               AND provider_session_id IN ('', ?5)
             ORDER BY revision DESC LIMIT 1",
            params![session_id, agent_id, family, account, native_session.unwrap_or("")],
            |row| Ok(AgentCompactionBoundary { revision: row.get::<_, i64>(0)?.max(0) as u64,
                source_provider: row.get(1)?, source_model: row.get(2)? }),
        ).optional().map_err(|e| compaction_error(session_id, "load compaction boundary", e))
    }

    /// The compactions already observed in one native session, by native record ID.
    pub(crate) fn observed_agent_compaction_ids(
        &self,
        session_id: &str,
        agent_id: &str,
        provider: &str,
        account: &str,
        native_session: &str,
    ) -> Result<Vec<String>, DaemonError> {
        let family = crate::provider::canonical_provider_family(provider).unwrap_or(provider);
        let connection = self.lock_read_connection(Some(session_id))?;
        let mut statement = connection
            .prepare(
                "SELECT provider_turn_id FROM history_agent_compaction_observations
                 WHERE session_id = ?1 AND agent_id = ?2 AND provider_family = ?3
                   AND account_profile = ?4 AND provider_session_id = ?5
                 ORDER BY provider_turn_id",
            )
            .map_err(|e| compaction_error(session_id, "list observed compactions", e))?;
        let ids = statement
            .query_map(
                params![session_id, agent_id, family, account, native_session],
                |row| row.get(0),
            )
            .map_err(|e| compaction_error(session_id, "list observed compactions", e))?
            .collect::<Result<_, _>>();
        ids.map_err(|e| compaction_error(session_id, "read observed compactions", e))
    }

    /// A reload acknowledged in one turn of a recovery epoch still counts in the next.
    pub(crate) fn record_agent_compaction_reload(
        &self,
        session_id: &str,
        agent_id: &str,
        revision: u64,
        skill_id: &str,
        digest: &str,
    ) -> Result<(), DaemonError> {
        let connection = self
            .connection
            .lock()
            .map_err(|e| compaction_error(session_id, "lock compaction reload", e))?;
        connection
            .execute(
                "INSERT OR IGNORE INTO history_agent_compaction_reloads
                 (session_id, agent_id, revision, skill_id, digest) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, agent_id, revision as i64, skill_id, digest],
            )
            .map(|_| ())
            .map_err(|e| compaction_error(session_id, "record compaction reload", e))
    }

    /// The `(skill ID, digest)` pairs the harness has taken back in one recovery epoch.
    pub(crate) fn agent_compaction_reloads(
        &self,
        session_id: &str,
        agent_id: &str,
        revision: u64,
    ) -> Result<std::collections::BTreeSet<(String, String)>, DaemonError> {
        let connection = self.lock_read_connection(Some(session_id))?;
        let mut statement = connection
            .prepare(
                "SELECT skill_id, digest FROM history_agent_compaction_reloads
                 WHERE session_id = ?1 AND agent_id = ?2 AND revision = ?3",
            )
            .map_err(|e| compaction_error(session_id, "list compaction reloads", e))?;
        let reloads = statement
            .query_map(params![session_id, agent_id, revision as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|e| compaction_error(session_id, "list compaction reloads", e))?
            .collect::<Result<_, _>>();
        reloads.map_err(|e| compaction_error(session_id, "read compaction reloads", e))
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
            .map_err(|e| compaction_error(session_id, "acknowledge compaction boundary", e))?;
        connection
            .execute(
                "DELETE FROM history_agent_compaction_reloads
                 WHERE session_id = ?1 AND agent_id = ?2 AND revision = ?3",
                params![session_id, agent_id, revision as i64],
            )
            .map(|_| ())
            .map_err(|e| compaction_error(session_id, "clear compaction reloads", e))
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
    let native_session = origin.native_session.unwrap_or("");
    // Revisions count per agent, so an acknowledgement names one native session's recovery.
    connection.execute(
        "INSERT INTO history_agent_compaction_boundaries
             (session_id, agent_id, provider_family, account_profile, provider_session_id, revision, delivered_revision, source_provider, source_model)
         VALUES (?1, ?2, ?3, ?4, ?5,
             (SELECT COALESCE(MAX(revision), 0) + 1 FROM history_agent_compaction_boundaries WHERE session_id = ?1 AND agent_id = ?2),
             0, ?6, ?7)
         ON CONFLICT (session_id, agent_id, provider_family, account_profile, provider_session_id) DO UPDATE SET
             revision = excluded.revision, source_provider = excluded.source_provider, source_model = excluded.source_model",
        params![session_id, agent_id, family, origin.account, native_session, origin.provider, origin.model],
    ).map_err(|e| compaction_error(session_id, "record compaction boundary", e))?;
    connection.query_row(
        "SELECT revision FROM history_agent_compaction_boundaries
         WHERE session_id = ?1 AND agent_id = ?2 AND provider_family = ?3 AND account_profile = ?4 AND provider_session_id = ?5",
        params![session_id, agent_id, family, origin.account, native_session], |row| row.get::<_, i64>(0))
        .map(|revision| revision.max(0) as u64).map_err(|e| compaction_error(session_id, "read compaction epoch", e))
}

const BOUNDARIES_TABLE: &str = "CREATE TABLE IF NOT EXISTS history_agent_compaction_boundaries (
    session_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    provider_family TEXT NOT NULL,
    account_profile TEXT NOT NULL,
    provider_session_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    delivered_revision INTEGER NOT NULL,
    source_provider TEXT NOT NULL,
    source_model TEXT NOT NULL,
    PRIMARY KEY (session_id, agent_id, provider_family, account_profile, provider_session_id)
) WITHOUT ROWID;";

/// Pending recovery is keyed per native session. Databases that kept one boundary
/// per agent are rebuilt in place; an unscoped boundary keeps the empty session ID.
pub(super) fn migrate(connection: &rusqlite::Connection) -> Result<(), DaemonError> {
    let migrate_error =
        |e: rusqlite::Error| compaction_error("", "migrate compaction boundaries", e);
    let key_columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('history_agent_compaction_boundaries') WHERE pk > 0",
            [],
            |row| row.get(0),
        )
        .map_err(migrate_error)?;
    if key_columns != 2 {
        return connection
            .execute_batch(BOUNDARIES_TABLE)
            .map_err(migrate_error);
    }
    connection
        .execute_batch(&format!(
            "BEGIN;
             ALTER TABLE history_agent_compaction_boundaries RENAME TO history_agent_compaction_boundaries_v1;
             {BOUNDARIES_TABLE}
             INSERT INTO history_agent_compaction_boundaries
             SELECT session_id, agent_id, provider_family, account_profile, COALESCE(provider_session_id, ''),
                    revision, delivered_revision, source_provider, source_model
             FROM history_agent_compaction_boundaries_v1;
             DROP TABLE history_agent_compaction_boundaries_v1;
             COMMIT;"
        ))
        .map_err(migrate_error)
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
    fn acknowledging_one_native_session_keeps_the_others_partial_reloads() {
        for first_to_finish in [0, 1] {
            let root = crate::test_support::TestWorktree::new("partial-native-reloads");
            let path = root.path().join("history.db");
            let store = OperationalHistoryStore::open(path.clone()).unwrap();
            let revisions = ["native-a", "native-b"].map(|native| {
                store
                    .record_agent_compaction_boundary(
                        "s",
                        "a",
                        "claude",
                        "haiku",
                        "account",
                        Some(native),
                    )
                    .unwrap()
            });
            for revision in revisions {
                store
                    .record_agent_compaction_reload("s", "a", revision, "review", "digest-1")
                    .unwrap();
            }
            store
                .deliver_agent_compaction_boundary("s", "a", revisions[first_to_finish])
                .unwrap();
            assert!(store
                .agent_compaction_reloads("s", "a", revisions[first_to_finish])
                .unwrap()
                .is_empty());
            drop(store);
            let store = OperationalHistoryStore::open(path).unwrap();
            let pending = revisions[1 - first_to_finish];
            store
                .record_agent_compaction_reload("s", "a", pending, "audit", "digest-2")
                .unwrap();
            assert_eq!(
                store.agent_compaction_reloads("s", "a", pending).unwrap(),
                [
                    ("review".into(), "digest-1".into()),
                    ("audit".into(), "digest-2".into())
                ]
                .into_iter()
                .collect(),
                "finishing native session {first_to_finish} erased another session's progress"
            );
            store
                .deliver_agent_compaction_boundary("s", "a", pending)
                .unwrap();
        }
    }
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
    fn compacted_native_sessions_of_one_agent_recover_independently() {
        let root = crate::test_support::TestWorktree::new("two-native-compactions");
        let store = OperationalHistoryStore::open(root.path().join("history.db")).unwrap();
        let origin = |native| AgentCompactionOrigin {
            provider: "claude",
            model: "haiku",
            account: "account",
            native_session: Some(native),
        };
        let load = |native| {
            store
                .load_agent_compaction_boundary("s", "a", "claude", "account", Some(native))
                .unwrap()
        };
        for (native, turn) in [("native-a", "compact-a"), ("native-b", "compact-b")] {
            assert!(store
                .record_observed_agent_compaction_boundary("s", "a", origin(native), turn)
                .unwrap());
        }
        let ids = |native| {
            store
                .observed_agent_compaction_ids("s", "a", "claude-headless", "account", native)
                .unwrap()
        };
        assert_eq!(ids("native-a"), ["compact-a"]);
        assert_eq!(ids("native-b"), ["compact-b"]);
        assert!(ids("native-c").is_empty());
        let (a, b) = (load("native-a").unwrap(), load("native-b").unwrap());
        assert_ne!(a.revision, b.revision);
        store
            .deliver_agent_compaction_boundary("s", "a", a.revision)
            .unwrap();
        assert!(load("native-a").is_none());
        assert_eq!(load("native-b").unwrap().revision, b.revision);
        assert!(!store
            .record_observed_agent_compaction_boundary("s", "a", origin("native-a"), "compact-a")
            .unwrap());
        assert!(load("native-a").is_none());
        store
            .deliver_agent_compaction_boundary("s", "a", b.revision)
            .unwrap();
        assert!(load("native-b").is_none());
    }
    #[test]
    fn a_boundary_per_agent_database_keeps_its_pending_recovery() {
        let root = crate::test_support::TestWorktree::new("compaction-boundary-migration");
        let path = root.path().join("history.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE history_agent_compaction_boundaries (
                    session_id TEXT NOT NULL, agent_id TEXT NOT NULL, revision INTEGER NOT NULL,
                    delivered_revision INTEGER NOT NULL, provider_family TEXT NOT NULL,
                    source_provider TEXT NOT NULL, source_model TEXT NOT NULL,
                    account_profile TEXT NOT NULL, provider_session_id TEXT,
                    PRIMARY KEY (session_id, agent_id)) WITHOUT ROWID;
                 INSERT INTO history_agent_compaction_boundaries
                    VALUES ('s', 'a', 3, 2, 'claude', 'claude', 'haiku', 'account', 'native'),
                           ('s', 'unscoped', 1, 0, 'claude', 'claude', 'haiku', 'account', NULL);",
            )
            .unwrap();
        let store = OperationalHistoryStore::open(path).unwrap();
        let load = |agent, native| {
            store
                .load_agent_compaction_boundary("s", agent, "claude", "account", native)
                .unwrap()
        };
        assert_eq!(load("a", Some("native")).unwrap().revision, 3);
        assert!(load("a", Some("other")).is_none());
        assert_eq!(load("unscoped", Some("any")).unwrap().revision, 1);
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
