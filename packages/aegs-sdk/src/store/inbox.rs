//! Durable webhook inbox: every verified webhook is stored here before the provider is told it
//! was received, then published to AEDS from the stored copy until it succeeds or the attempt
//! budget runs out.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::AegsStore;
use crate::NormalizedEvent;

/// Publication attempts one stored event gets before it needs an explicit replay.
const MAX_PUBLICATION_ATTEMPTS: u32 = 8;
const FIRST_RETRY_DELAY_MS: u64 = 5_000;
const MAX_RETRY_DELAY_MS: u64 = 10 * 60_000;
/// How long a claimed attempt keeps other workers away; a crashed attempt is retried afterwards.
const ATTEMPT_LEASE_MS: u64 = 60_000;
/// Published events are kept as long as AEDS keeps its own deduplication window.
const RETENTION_MS: u64 = 30 * 24 * 60 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxStatus {
    /// Stored and waiting for (another) publication attempt.
    Pending,
    /// Published to AEDS for every matching trigger.
    Published,
    /// Stored, but no active trigger matched when it was last processed.
    Unmatched,
    /// Attempts ran out; a replay is needed.
    Failed,
}

impl InboxStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Published => "published",
            Self::Unmatched => "unmatched",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Self::Pending),
            "published" => Ok(Self::Published),
            "unmatched" => Ok(Self::Unmatched),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown inbox status `{other}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InboxEntry {
    pub delivery_id: String,
    /// The connection named by the webhook URL, when the provider routes per connection.
    pub route_connection_id: Option<String>,
    pub event: NormalizedEvent,
    pub received_at_ms: u64,
    pub status: InboxStatus,
    pub attempts: u32,
    pub next_attempt_at_ms: u64,
    pub published_at_ms: Option<u64>,
    pub publication_count: u32,
    pub last_error: Option<String>,
}

impl AegsStore {
    /// Stores a verified webhook. Returns `false` when `delivery_id` was already stored, in which
    /// case the existing entry is left untouched.
    pub fn record_inbox_event(
        &self,
        delivery_id: &str,
        route_connection_id: Option<&str>,
        event: &NormalizedEvent,
        now_ms: u64,
    ) -> Result<bool, String> {
        let event_json = serde_json::to_string(event).map_err(|error| error.to_string())?;
        let connection = self.lock()?;
        let inserted = connection
            .execute(
                "INSERT OR IGNORE INTO webhook_inbox
                 (delivery_id, route_connection_id, connection_scope, event_type, occurrence_id,
                  event_json, received_at_ms, status, next_attempt_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?7)",
                params![
                    delivery_id,
                    route_connection_id,
                    event.connection_scope,
                    event.event_type,
                    event.occurrence_id,
                    event_json,
                    now_ms as i64,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(inserted == 1)
    }

    pub fn has_inbox_event(&self, delivery_id: &str) -> Result<bool, String> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT 1 FROM webhook_inbox WHERE delivery_id = ?1",
                params![delivery_id],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(|error| error.to_string())
    }

    /// Takes the next publication attempt of a pending entry. Only one caller gets the entry
    /// until its lease expires, so the inline attempt and the retry worker never overlap.
    pub fn claim_inbox_attempt(
        &self,
        delivery_id: &str,
        now_ms: u64,
    ) -> Result<Option<InboxEntry>, String> {
        let connection = self.lock()?;
        let claimed = connection
            .execute(
                "UPDATE webhook_inbox
                 SET attempts = attempts + 1, next_attempt_at_ms = ?2
                 WHERE delivery_id = ?1 AND status = 'pending' AND next_attempt_at_ms <= ?3",
                params![
                    delivery_id,
                    (now_ms + ATTEMPT_LEASE_MS) as i64,
                    now_ms as i64
                ],
            )
            .map_err(|error| error.to_string())?;
        if claimed == 0 {
            return Ok(None);
        }
        inbox_entry(&connection, delivery_id)
    }

    pub fn due_inbox_deliveries(&self, now_ms: u64, limit: usize) -> Result<Vec<String>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT delivery_id FROM webhook_inbox
                 WHERE status = 'pending' AND next_attempt_at_ms <= ?1
                 ORDER BY next_attempt_at_ms LIMIT ?2",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![now_ms as i64, limit as i64], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn finish_inbox_publication(
        &self,
        delivery_id: &str,
        publication_count: usize,
        now_ms: u64,
    ) -> Result<(), String> {
        let status = if publication_count == 0 {
            InboxStatus::Unmatched
        } else {
            InboxStatus::Published
        };
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE webhook_inbox
                 SET status = ?2, published_at_ms = ?3, publication_count = ?4, last_error = NULL
                 WHERE delivery_id = ?1",
                params![
                    delivery_id,
                    status.as_str(),
                    now_ms as i64,
                    publication_count as i64
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Records a failed attempt and schedules the next one, or marks the entry `failed` once the
    /// attempt budget is spent.
    pub fn fail_inbox_publication(
        &self,
        delivery_id: &str,
        error: &str,
        now_ms: u64,
    ) -> Result<InboxStatus, String> {
        let connection = self.lock()?;
        let attempts: u32 = connection
            .query_row(
                "SELECT attempts FROM webhook_inbox WHERE delivery_id = ?1",
                params![delivery_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let status = if attempts >= MAX_PUBLICATION_ATTEMPTS {
            InboxStatus::Failed
        } else {
            InboxStatus::Pending
        };
        connection
            .execute(
                "UPDATE webhook_inbox
                 SET status = ?2, next_attempt_at_ms = ?3, last_error = ?4
                 WHERE delivery_id = ?1",
                params![
                    delivery_id,
                    status.as_str(),
                    (now_ms + retry_delay_ms(attempts)) as i64,
                    error
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(status)
    }

    /// Makes a stored event publishable again with a fresh attempt budget. Returns `false` when
    /// no such entry exists or it is already waiting for an attempt.
    pub fn requeue_inbox_event(&self, delivery_id: &str, now_ms: u64) -> Result<bool, String> {
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE webhook_inbox
                 SET status = 'pending', attempts = 0, next_attempt_at_ms = ?2, last_error = NULL
                 WHERE delivery_id = ?1 AND status != 'pending'",
                params![delivery_id, now_ms as i64],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    pub fn inbox_event(&self, delivery_id: &str) -> Result<Option<InboxEntry>, String> {
        inbox_entry(&*self.lock()?, delivery_id)
    }

    /// The newest stored events of one owner, newest first. An event belongs to an owner when
    /// its webhook route or its connection scope matches one of the owner's connections.
    pub fn inbox_events_for_owner(
        &self,
        owner_id: &str,
        limit: usize,
    ) -> Result<Vec<InboxEntry>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {INBOX_COLUMNS} FROM webhook_inbox
                 WHERE route_connection_id IN
                         (SELECT connection_id FROM connections WHERE owner_id = ?1)
                    OR connection_scope IN
                         (SELECT connection_scope FROM subscriptions WHERE owner_id = ?1)
                 ORDER BY received_at_ms DESC, delivery_id LIMIT ?2"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![owner_id, limit as i64], read_entry)
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string())?)
            .collect()
    }

    /// Drops events older than the retention window. Pending events are never dropped.
    pub fn prune_inbox(&self, now_ms: u64) -> Result<usize, String> {
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM webhook_inbox WHERE status != 'pending' AND received_at_ms < ?1",
                params![now_ms.saturating_sub(RETENTION_MS) as i64],
            )
            .map_err(|error| error.to_string())
    }
}

fn retry_delay_ms(attempts: u32) -> u64 {
    FIRST_RETRY_DELAY_MS
        .saturating_mul(1 << attempts.saturating_sub(1).min(20))
        .min(MAX_RETRY_DELAY_MS)
}

const INBOX_COLUMNS: &str =
    "delivery_id, route_connection_id, event_json, received_at_ms, status, \
     attempts, next_attempt_at_ms, published_at_ms, publication_count, last_error";

fn inbox_entry(
    connection: &rusqlite::Connection,
    delivery_id: &str,
) -> Result<Option<InboxEntry>, String> {
    connection
        .query_row(
            &format!("SELECT {INBOX_COLUMNS} FROM webhook_inbox WHERE delivery_id = ?1"),
            params![delivery_id],
            read_entry,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .transpose()
}

fn read_entry(row: &Row<'_>) -> rusqlite::Result<Result<InboxEntry, String>> {
    let event_json: String = row.get(2)?;
    let status: String = row.get(4)?;
    let entry = InboxEntry {
        delivery_id: row.get(0)?,
        route_connection_id: row.get(1)?,
        event: match serde_json::from_str(&event_json) {
            Ok(event) => event,
            Err(error) => return Ok(Err(format!("stored webhook event is unreadable: {error}"))),
        },
        received_at_ms: row.get::<_, i64>(3)? as u64,
        status: match InboxStatus::parse(&status) {
            Ok(status) => status,
            Err(error) => return Ok(Err(error)),
        },
        attempts: row.get(5)?,
        next_attempt_at_ms: row.get::<_, i64>(6)? as u64,
        published_at_ms: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
        publication_count: row.get(8)?,
        last_error: row.get(9)?,
    };
    Ok(Ok(entry))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> NormalizedEvent {
        NormalizedEvent {
            occurrence_id: "occurrence".into(),
            event_type: "pull_request.synchronize".into(),
            occurred_at: "2026-10-17T10:48:33Z".into(),
            connection_scope: "scope".into(),
            prompt: "prompt".into(),
            metadata: serde_json::json!({"repository": "charioxai/chariox", "number": 1026}),
            reply_context: None,
        }
    }

    #[test]
    fn stored_event_survives_a_restart_and_a_second_delivery_id_is_ignored() {
        let directory = std::env::var_os("TMPDIR").unwrap_or_else(|| "/var/tmp".into());
        let path = std::path::Path::new(&directory)
            .join(format!("aegs-inbox-{}.db", crate::random_opaque("test")));
        {
            let store = AegsStore::open(&path).unwrap();
            assert!(store
                .record_inbox_event("d1", Some("c1"), &event(), 10)
                .unwrap());
            assert!(!store
                .record_inbox_event("d1", Some("c1"), &event(), 20)
                .unwrap());
        }
        let store = AegsStore::open(&path).unwrap();
        let entry = store.inbox_event("d1").unwrap().unwrap();
        assert_eq!(entry.event, event());
        assert_eq!(entry.received_at_ms, 10);
        assert_eq!(store.due_inbox_deliveries(10, 8).unwrap(), vec!["d1"]);
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn claimed_attempt_is_leased_and_retries_back_off_up_to_the_budget() {
        let store = AegsStore::open(":memory:").unwrap();
        store.record_inbox_event("d1", None, &event(), 0).unwrap();
        assert!(store.claim_inbox_attempt("d1", 0).unwrap().is_some());
        assert!(store.claim_inbox_attempt("d1", 1).unwrap().is_none());
        // A crashed attempt becomes claimable again once its lease ends.
        assert!(store
            .claim_inbox_attempt("d1", ATTEMPT_LEASE_MS)
            .unwrap()
            .is_some());
        let mut now = ATTEMPT_LEASE_MS;
        let mut delays = Vec::new();
        loop {
            let status = store.fail_inbox_publication("d1", "refused", now).unwrap();
            let entry = store.inbox_event("d1").unwrap().unwrap();
            delays.push(entry.next_attempt_at_ms - now);
            if status == InboxStatus::Failed {
                break;
            }
            now = entry.next_attempt_at_ms;
            store.claim_inbox_attempt("d1", now).unwrap().unwrap();
        }
        assert_eq!(delays[..3], [10_000, 20_000, 40_000]);
        assert!(delays.iter().all(|delay| *delay <= MAX_RETRY_DELAY_MS));
        assert_eq!(delays.len() as u32, MAX_PUBLICATION_ATTEMPTS - 1);
    }

    #[test]
    fn pruning_keeps_pending_events_and_recent_ones() {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .record_inbox_event("pending", None, &event(), 0)
            .unwrap();
        store.record_inbox_event("old", None, &event(), 0).unwrap();
        store.finish_inbox_publication("old", 1, 1).unwrap();
        store
            .record_inbox_event("recent", None, &event(), RETENTION_MS)
            .unwrap();
        store
            .finish_inbox_publication("recent", 1, RETENTION_MS)
            .unwrap();
        assert_eq!(store.prune_inbox(RETENTION_MS + 1).unwrap(), 1);
        assert!(store.has_inbox_event("pending").unwrap());
        assert!(store.has_inbox_event("recent").unwrap());
        assert!(!store.has_inbox_event("old").unwrap());
    }
}
