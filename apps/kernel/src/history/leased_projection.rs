//! Worker-local projection bookkeeping. No fields here are part of the relay protocol.
mod tools;
pub(crate) use tools::ProjectedToolState;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

use super::{HistoryEvent, OperationalHistoryStore};
use crate::error::DaemonError;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LeasedProjectionCursor {
    pub(crate) committed_sequence: u64,
    pub(crate) latest_home_prompt_key: Option<String>,
    #[serde(default)]
    pub(crate) latest_home_prompt_sequence: u64,
    pub(crate) output_prompt_key: Option<String>,
}

pub(crate) struct ProjectedHistoryKeys {
    pub(crate) event_id: String,
    pub(crate) stream_key: Option<String>,
    pub(crate) snapshot_key: String,
}

const LEASED_PROJECTION_HISTORY_SQL: &str = "SELECT committed_sequence, event_json FROM history_events
             WHERE session_id = ?1 AND agent_id = ?2 AND provider_run_id = ?3 AND committed_sequence > ?4
             UNION ALL
             SELECT committed_sequence, event_json FROM history_events
             WHERE session_id = ?1 AND agent_id = ?2 AND kind = 'user_prompt' AND provider_run_id IS NOT ?3 AND committed_sequence > ?4
             ORDER BY committed_sequence";

fn operational_history_error(
    operation: &'static str,
    error: impl std::fmt::Display,
) -> DaemonError {
    DaemonError::SessionHistoryFailed {
        session_id: None,
        operation,
        message: error.to_string(),
    }
}

pub(super) fn migrate(connection: &mut rusqlite::Connection) -> Result<(), DaemonError> {
    let transaction = connection
        .transaction()
        .map_err(|e| operational_history_error("begin projection migration", e))?;
    let connection = &transaction;
    let mut statement = connection
        .prepare("PRAGMA table_info(history_events)")
        .map_err(|e| operational_history_error("inspect projection schema", e))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| operational_history_error("read projection schema", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| operational_history_error("decode projection schema", e))?;
    drop(statement);
    let backfill = !columns.iter().any(|column| column == "committed_sequence");
    for (column, sql_type) in [
        ("committed_sequence", "INTEGER"),
        ("leased_projection_stream_key", "TEXT"),
        ("leased_projection_snapshot_key", "TEXT"),
    ] {
        if !columns.iter().any(|existing| existing == column) {
            connection
                .execute(
                    &format!("ALTER TABLE history_events ADD COLUMN {column} {sql_type}"),
                    [],
                )
                .map_err(|e| operational_history_error("add projection column", e))?;
        }
    }
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS history_commit_counter (id INTEGER PRIMARY KEY, value INTEGER NOT NULL);
         INSERT OR IGNORE INTO history_commit_counter VALUES (1, 0);

         CREATE TRIGGER IF NOT EXISTS history_commit_insert AFTER INSERT ON history_events BEGIN
             UPDATE history_commit_counter SET value = value + 1 WHERE id = 1;
             UPDATE history_events SET committed_sequence = (SELECT value FROM history_commit_counter WHERE id = 1) WHERE event_id = NEW.event_id;
         END;
         CREATE TRIGGER IF NOT EXISTS history_commit_update AFTER UPDATE OF event_json ON history_events BEGIN
             UPDATE history_commit_counter SET value = value + 1 WHERE id = 1;
             UPDATE history_events SET committed_sequence = (SELECT value FROM history_commit_counter WHERE id = 1) WHERE event_id = NEW.event_id;
         END;
         CREATE INDEX IF NOT EXISTS idx_history_projection_commit ON history_events(session_id, agent_id, provider_run_id, committed_sequence);
         CREATE INDEX IF NOT EXISTS idx_history_projection_prompt_commit ON history_events(session_id, agent_id, kind, committed_sequence);
         CREATE INDEX IF NOT EXISTS idx_history_projection_stream ON history_events(leased_projection_stream_key);
         CREATE INDEX IF NOT EXISTS idx_history_projection_snapshot ON history_events(leased_projection_snapshot_key);
         CREATE TABLE IF NOT EXISTS leased_projection_cursors (projection_id TEXT PRIMARY KEY, cursor_json TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS leased_projection_pending_keys (key TEXT PRIMARY KEY, projection_id TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS leased_projection_tools (stream_key TEXT PRIMARY KEY, projection_id TEXT NOT NULL, state_json TEXT NOT NULL);
         DROP TRIGGER IF EXISTS history_projection_prune;
         CREATE TRIGGER history_projection_prune BEFORE DELETE ON history_events BEGIN
             INSERT OR IGNORE INTO leased_projection_pending_keys
                 SELECT OLD.leased_projection_stream_key, OLD.session_id || ':' || OLD.agent_id || ':' || OLD.provider_run_id
                 WHERE OLD.kind != 'provider_tool' AND OLD.leased_projection_stream_key IS NOT NULL
                   AND EXISTS (SELECT 1 FROM leased_projection_cursors WHERE projection_id = OLD.session_id || ':' || OLD.agent_id || ':' || OLD.provider_run_id);
             INSERT OR IGNORE INTO leased_projection_pending_keys
                 SELECT OLD.leased_projection_snapshot_key, OLD.session_id || ':' || OLD.agent_id || ':' || OLD.provider_run_id
                 WHERE OLD.kind != 'provider_tool' AND OLD.leased_projection_snapshot_key IS NOT NULL
                   AND EXISTS (SELECT 1 FROM leased_projection_cursors WHERE projection_id = OLD.session_id || ':' || OLD.agent_id || ':' || OLD.provider_run_id);
             INSERT INTO leased_projection_tools
                 SELECT OLD.leased_projection_stream_key, OLD.session_id || ':' || OLD.agent_id || ':' || OLD.provider_run_id, OLD.leased_projection_snapshot_key
                 WHERE OLD.kind = 'provider_tool' AND OLD.leased_projection_stream_key IS NOT NULL AND json_valid(OLD.leased_projection_snapshot_key)
                   AND EXISTS (SELECT 1 FROM leased_projection_cursors WHERE projection_id = OLD.session_id || ':' || OLD.agent_id || ':' || OLD.provider_run_id)
                 ON CONFLICT(stream_key) DO UPDATE SET state_json = excluded.state_json
                 WHERE json_extract(excluded.state_json, '$.revision') > json_extract(state_json, '$.revision');
         END;
         DROP TRIGGER IF EXISTS history_projection_replace;
         CREATE TRIGGER history_projection_replace BEFORE UPDATE OF leased_projection_stream_key, leased_projection_snapshot_key ON history_events BEGIN
             INSERT OR IGNORE INTO leased_projection_pending_keys SELECT OLD.leased_projection_stream_key, OLD.session_id || ':' || OLD.agent_id || ':' || OLD.provider_run_id WHERE OLD.kind != 'provider_tool' AND OLD.leased_projection_stream_key IS NOT NULL AND OLD.leased_projection_stream_key IS NOT NEW.leased_projection_stream_key;
             INSERT OR IGNORE INTO leased_projection_pending_keys SELECT OLD.leased_projection_snapshot_key, OLD.session_id || ':' || OLD.agent_id || ':' || OLD.provider_run_id WHERE OLD.kind != 'provider_tool' AND OLD.leased_projection_snapshot_key IS NOT NULL AND OLD.leased_projection_snapshot_key IS NOT NEW.leased_projection_snapshot_key;
         END;")
        .map_err(|e| operational_history_error("migrate leased projection", e))?;
    if backfill {
        connection.execute_batch(
            "UPDATE history_events SET committed_sequence = sequence WHERE committed_sequence IS NULL;
             UPDATE history_commit_counter SET value = (SELECT COALESCE(MAX(committed_sequence), 0) FROM history_events) WHERE id = 1;"
        ).map_err(|e| operational_history_error("backfill projection commit order", e))?;
    }
    transaction
        .commit()
        .map_err(|e| operational_history_error("commit projection migration", e))
}

impl OperationalHistoryStore {
    fn ensure_leased_projection_schema(&self) -> Result<(), DaemonError> {
        if !self.projection_schema_initialized.load(Ordering::Acquire) {
            let mut connection = self
                .connection
                .lock()
                .map_err(|e| operational_history_error("lock projection migration", e))?;
            if !self.projection_schema_initialized.load(Ordering::Acquire) {
                migrate(&mut connection)?;
                self.projection_schema_initialized
                    .store(true, Ordering::Release);
            }
        }
        Ok(())
    }

    pub(crate) fn load_leased_projection_cursor(
        &self,
        projection_id: &str,
    ) -> Result<LeasedProjectionCursor, DaemonError> {
        self.ensure_leased_projection_schema()?;
        let connection = self.lock_read_connection(None)?;
        let json: Option<String> = connection
            .query_row(
                "SELECT cursor_json FROM leased_projection_cursors WHERE projection_id = ?1",
                [projection_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| operational_history_error("load leased projection cursor", e))?;
        json.map(|json| {
            serde_json::from_str(&json)
                .map_err(|e| operational_history_error("decode leased projection cursor", e))
        })
        .transpose()
        .map(|cursor| cursor.unwrap_or_default())
    }

    pub(crate) fn load_leased_projection_history(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        after: u64,
    ) -> Result<Vec<(u64, HistoryEvent)>, DaemonError> {
        self.ensure_leased_projection_schema()?;
        self.delay_read_if_configured();
        let connection = self.lock_read_connection(Some(session_id))?;
        let mut statement = connection
            .prepare(LEASED_PROJECTION_HISTORY_SQL)
            .map_err(|e| operational_history_error("prepare incremental projection history", e))?;
        let rows = statement
            .query_map(
                params![session_id, agent_id, provider_run_id, after as i64],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|e| operational_history_error("load incremental projection history", e))?;
        rows.map(|row| {
            let (sequence, json) = row
                .map_err(|e| operational_history_error("read incremental projection history", e))?;
            let event = serde_json::from_str(&json).map_err(|e| {
                operational_history_error("decode incremental projection history", e)
            })?;
            Ok((sequence as u64, event))
        })
        .collect()
    }

    pub(crate) fn leased_projection_key_exists(&self, key: &str) -> Result<bool, DaemonError> {
        self.ensure_leased_projection_schema()?;
        let connection = self.lock_read_connection(None)?;
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM leased_projection_pending_keys WHERE key = ?1)
                 OR EXISTS(SELECT 1 FROM history_events WHERE leased_projection_stream_key = ?1)
                 OR EXISTS(SELECT 1 FROM history_events WHERE leased_projection_snapshot_key = ?1)",
                [key],
                |row| row.get(0),
            )
            .map_err(|e| operational_history_error("lookup leased projection key", e))
    }

    pub(crate) fn delete_leased_projection_state(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        self.ensure_leased_projection_schema()?;
        let prefix = format!("{session_id}:{agent_id}:");
        let mut connection = self
            .connection
            .lock()
            .map_err(|e| operational_history_error("lock projection cleanup", e))?;
        let transaction = connection
            .transaction()
            .map_err(|e| operational_history_error("begin projection cleanup", e))?;
        for table in [
            "leased_projection_tools",
            "leased_projection_pending_keys",
            "leased_projection_cursors",
        ] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE substr(projection_id, 1, length(?1)) = ?1"),
                    [&prefix],
                )
                .map_err(|e| operational_history_error("delete leased projection state", e))?;
        }
        transaction
            .commit()
            .map_err(|e| operational_history_error("commit projection cleanup", e))
    }

    pub(crate) fn commit_leased_projection_cursor(
        &self,
        projection_id: &str,
        cursor: &LeasedProjectionCursor,
        pending_keys: &[String],
        history_keys: &[ProjectedHistoryKeys],
        tool_states: &[ProjectedToolState],
    ) -> Result<(), DaemonError> {
        self.ensure_leased_projection_schema()?;
        let json = serde_json::to_string(cursor)
            .map_err(|e| operational_history_error("encode leased projection cursor", e))?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|e| operational_history_error("lock leased projection state", e))?;
        let transaction = connection
            .transaction()
            .map_err(|e| operational_history_error("begin leased projection commit", e))?;
        for key in pending_keys {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO leased_projection_pending_keys VALUES (?1, ?2)",
                    params![key, projection_id],
                )
                .map_err(|e| operational_history_error("save pending projection key", e))?;
        }
        for keys in history_keys {
            // Finalized transcript rows retain the dedupe identity. Only streams whose
            // history has not committed yet need separate pending bookkeeping.
            transaction.execute(
                "UPDATE history_events SET leased_projection_stream_key = ?2, leased_projection_snapshot_key = ?3 WHERE event_id = ?1",
                params![keys.event_id, keys.stream_key, keys.snapshot_key])
                .map_err(|e| operational_history_error("compact projected history keys", e))?;
            transaction
                .execute(
                    "DELETE FROM leased_projection_pending_keys WHERE key IN (?1, ?2)",
                    params![keys.stream_key, keys.snapshot_key],
                )
                .map_err(|e| operational_history_error("retire pending projection keys", e))?;
        }
        for state in tool_states {
            tools::compact_tool_state(&transaction, projection_id, state)?;
        }
        transaction.execute("INSERT INTO leased_projection_cursors VALUES (?1, ?2) ON CONFLICT(projection_id) DO UPDATE SET cursor_json = excluded.cursor_json", params![projection_id, json])
            .map_err(|e| operational_history_error("save leased projection cursor", e))?;
        transaction
            .commit()
            .map_err(|e| operational_history_error("commit leased projection cursor", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
    use crate::terminal::TerminalOutputKind;

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "chariox-projection-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn open(&self) -> OperationalHistoryStore {
            OperationalHistoryStore::open(self.0.join("history.sqlite")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn entry(index: usize) -> SessionHistoryEntry {
        SessionHistoryEntry::provider_output(
            "session",
            "run",
            Some("agent"),
            TerminalOutputKind::ProviderOutput,
            Some(format!("message-{index}")),
            "x".repeat(1024),
        )
    }

    #[test]
    fn ordinary_history_store_does_not_install_projection_schema() {
        let fixture = Fixture::new();
        let store = fixture.open();
        for index in 0..100 {
            store
                .append_transcript(&entry(index), HistoryEventTurnContext::default())
                .unwrap();
        }
        drop(store);
        let store = fixture.open();
        let connection = store.lock_read_connection(None).unwrap();
        let count: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name LIKE 'history_commit_%' OR name LIKE '%projection%'", [], |row| row.get(0)).unwrap();
        assert_eq!(
            count, 0,
            "ordinary histories must pay no projection migration/write cost"
        );
        let columns: i64 = connection.query_row("SELECT count(*) FROM pragma_table_info('history_events') WHERE name = 'committed_sequence'", [], |row| row.get(0)).unwrap();
        assert_eq!(columns, 0);
    }

    #[test]
    fn tool_watermark_stays_bounded_without_history_and_survives_pruning() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let cursor = LeasedProjectionCursor::default();
        let stream_key = "session:agent:run:ProviderTool:tool";
        let mut tool = ProjectedToolState {
            stream_key: stream_key.into(),
            session_id: "session".into(),
            agent_id: "agent".into(),
            provider_run_id: "run".into(),
            merge_key: Some("tool".into()),
            state: store.load_leased_tool_state(stream_key).unwrap(),
        };
        for index in 0..1000 {
            let bytes = serde_json::to_vec(
                &serde_json::json!({"output": "same delta", "chariox_delta_offset_bytes": index}),
            )
            .unwrap();
            assert!(tool.state.record(format!("snapshot-{index}"), &bytes));
            store
                .commit_leased_projection_cursor(
                    "session:agent:run",
                    &cursor,
                    &[],
                    &[],
                    std::slice::from_ref(&tool),
                )
                .unwrap();
            assert_eq!(store.pending_leased_projection_counts(), (0, 1));
        }
        let mut transcript = entry(1);
        transcript.kind = crate::history::SessionHistoryEntryKind::ProviderTool;
        transcript.merge_key = Some("tool".into());
        transcript.text = "full transcript, unlike terminal delta".into();
        let event = store
            .append_transcript(&transcript, HistoryEventTurnContext::default())
            .unwrap();
        store
            .commit_leased_projection_cursor("session:agent:run", &cursor, &[], &[], &[tool])
            .unwrap();
        assert_eq!(store.pending_leased_projection_counts(), (0, 0));
        store
            .prune_events_before(event.timestamp_ms + 1, true)
            .unwrap();
        assert_eq!(store.pending_leased_projection_counts(), (0, 1));
        drop(store);
        let store = fixture.open();
        let mut state = store.load_leased_tool_state(stream_key).unwrap();
        assert!(!state.record(
            "old snapshot".into(),
            br#"{"chariox_delta_offset_bytes":0}"#
        ));
        assert!(!state.record("snapshot-999".into(), b"last snapshot"));
        assert!(state.record(
            "next snapshot".into(),
            br#"{"chariox_delta_offset_bytes":1000}"#
        ));
        store
            .delete_leased_projection_state("session", "agent")
            .unwrap();
        assert_eq!(store.pending_leased_projection_counts(), (0, 0));
    }

    #[test]
    fn committed_cursor_recovers_late_lower_sequence_and_replaced_history_after_reopen() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let late_sequence = store.reserve_sequence();
        let late =
            HistoryEvent::transcript(late_sequence, &entry(1), HistoryEventTurnContext::default());
        store
            .append_transcript(&entry(2), HistoryEventTurnContext::default())
            .unwrap();
        let first = store
            .load_leased_projection_history("session", "agent", "run", 0)
            .unwrap();
        assert_eq!(first.len(), 1);
        let cursor = LeasedProjectionCursor {
            committed_sequence: first[0].0,
            ..Default::default()
        };
        store
            .commit_leased_projection_cursor(
                "projection",
                &cursor,
                &["pending-stream".into()],
                &[],
                &[],
            )
            .unwrap();
        drop(store);
        let store = fixture.open();
        assert!(store
            .leased_projection_key_exists("pending-stream")
            .unwrap());
        let cursor = store.load_leased_projection_cursor("projection").unwrap();
        store.append(&late).unwrap();
        let next = store
            .load_leased_projection_history("session", "agent", "run", cursor.committed_sequence)
            .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].1.sequence, late_sequence);
        let mut replacement = entry(2);
        replacement.text = "replacement".into();
        store
            .replace_transcript_by_merge_key(
                "session",
                Some("agent"),
                "message-2",
                &replacement,
                HistoryEventTurnContext::default(),
            )
            .unwrap();
        let updates = store
            .load_leased_projection_history("session", "agent", "run", next[0].0)
            .unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1.content.as_deref(), Some("replacement"));
    }

    #[test]
    fn history_pruning_preserves_dedupe_until_lease_cleanup() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let event = store
            .append_transcript(&entry(1), HistoryEventTurnContext::default())
            .unwrap();
        let events = store
            .load_leased_projection_history("session", "agent", "run", 0)
            .unwrap();
        let cursor = LeasedProjectionCursor {
            committed_sequence: events[0].0,
            ..Default::default()
        };
        let keys = ProjectedHistoryKeys {
            event_id: event.event_id.clone(),
            stream_key: Some("stream".into()),
            snapshot_key: "snapshot".into(),
        };
        store
            .commit_leased_projection_cursor("session:agent:run", &cursor, &[], &[keys], &[])
            .unwrap();
        assert_eq!(
            store
                .prune_events_before(event.timestamp_ms + 1, true)
                .unwrap(),
            1
        );
        drop(store);
        let store = fixture.open();
        assert!(store.leased_projection_key_exists("stream").unwrap());
        assert!(store.leased_projection_key_exists("snapshot").unwrap());
        store
            .delete_leased_projection_state("session", "agent")
            .unwrap();
        assert!(!store.leased_projection_key_exists("stream").unwrap());
        assert!(!store.leased_projection_key_exists("snapshot").unwrap());
        // Pruning old rows after cleanup must not resurrect deleted lease state.
        let event = store
            .append_transcript(&entry(2), HistoryEventTurnContext::default())
            .unwrap();
        let keys = ProjectedHistoryKeys {
            event_id: event.event_id,
            stream_key: Some("retired-stream".into()),
            snapshot_key: "retired-snapshot".into(),
        };
        store
            .commit_leased_projection_cursor("session:agent:run", &cursor, &[], &[keys], &[])
            .unwrap();
        store
            .delete_leased_projection_state("session", "agent")
            .unwrap();
        store
            .prune_events_before(event.timestamp_ms + 1, true)
            .unwrap();
        assert!(!store
            .leased_projection_key_exists("retired-stream")
            .unwrap());
        assert!(!store
            .leased_projection_key_exists("retired-snapshot")
            .unwrap());
    }

    #[test]
    fn cursor_and_pending_state_stay_bounded_and_incremental_read_bytes_stay_flat() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let mut cursor = LeasedProjectionCursor::default();
        let mut drain_bytes = Vec::new();
        let mut drain_steps = Vec::new();
        for batch in 0..10 {
            let entries = (batch * 1000..(batch + 1) * 1000)
                .map(entry)
                .collect::<Vec<_>>();
            store
                .append_transcripts(
                    entries
                        .iter()
                        .map(|entry| (entry, HistoryEventTurnContext::default()))
                        .collect(),
                )
                .unwrap();
            let events = store
                .load_leased_projection_history(
                    "session",
                    "agent",
                    "run",
                    cursor.committed_sequence,
                )
                .unwrap();
            assert_eq!(events.len(), 1000);
            let keys = events
                .iter()
                .map(|(_, event)| ProjectedHistoryKeys {
                    event_id: event.event_id.clone(),
                    stream_key: Some(format!("stream-{}", event.sequence)),
                    snapshot_key: format!("snapshot-{}", event.sequence),
                })
                .collect::<Vec<_>>();
            cursor.committed_sequence = events.last().unwrap().0;
            let pending = keys
                .iter()
                .flat_map(|keys| [keys.stream_key.clone(), Some(keys.snapshot_key.clone())])
                .flatten()
                .collect::<Vec<_>>();
            store
                .commit_leased_projection_cursor("projection", &cursor, &pending, &keys, &[])
                .unwrap();
            let connection = store.lock_read_connection(None).unwrap();
            let pending_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM leased_projection_pending_keys",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let cursor_bytes: i64 = connection
                .query_row(
                    "SELECT length(cursor_json) FROM leased_projection_cursors",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                pending_count, 0,
                "finalized keys must compact into existing history rows"
            );
            assert!(
                cursor_bytes < 220,
                "cursor size must be independent of projected outputs"
            );
            drop(connection);
            assert!(store
                .load_leased_projection_history(
                    "session",
                    "agent",
                    "run",
                    cursor.committed_sequence
                )
                .unwrap()
                .is_empty());
            store
                .append_transcript(&entry(10_000 + batch), HistoryEventTurnContext::default())
                .unwrap();
            let next = store
                .load_leased_projection_history(
                    "session",
                    "agent",
                    "run",
                    cursor.committed_sequence,
                )
                .unwrap();
            assert_eq!(next.len(), 1);
            let connection = store.lock_read_connection(None).unwrap();
            let mut statement = connection.prepare(LEASED_PROJECTION_HISTORY_SQL).unwrap();
            let rows = statement
                .query_map(
                    params!["session", "agent", "run", cursor.committed_sequence as i64],
                    |row| row.get::<_, String>(1),
                )
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(rows.len(), 1);
            drain_steps.push(statement.get_status(rusqlite::StatementStatus::VmStep));
            drop(statement);
            drop(connection);
            drain_bytes.push(next[0].1.content.as_ref().unwrap().len());
            cursor.committed_sequence = next[0].0;
            store
                .commit_leased_projection_cursor("projection", &cursor, &[], &[], &[])
                .unwrap();
        }
        assert_eq!(drain_bytes, vec![1024; 10]);
        assert!(drain_steps.iter().all(|steps| *steps < 250));
        assert_eq!(
            drain_steps.iter().min(),
            drain_steps.iter().max(),
            "SQL work per new output must stay flat across growing history"
        );
        assert!(store.leased_projection_key_exists("stream-1").unwrap());
        drop(store);
        let store = fixture.open();
        let cursor = store.load_leased_projection_cursor("projection").unwrap();
        assert!(store
            .load_leased_projection_history("session", "agent", "run", cursor.committed_sequence)
            .unwrap()
            .is_empty());
        assert!(store.leased_projection_key_exists("stream-1").unwrap());
    }
}
