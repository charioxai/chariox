//! Durable webhook inbox: every verified webhook is stored here before the provider is told it
//! was received. Its trigger interests are chosen once, at the first attempt, and each one then
//! has its own delivery row that is published, retried, replayed and fenced on its own. The
//! status of the stored event is only a summary of those rows, so one owner's replay or
//! revocation can never change what another owner's interest still has to receive.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::AegsStore;
use crate::{NormalizedEvent, SubscriptionClaim};

/// A trigger interest that was eligible for an event, with the connection generation that
/// authorized it.
pub type SelectedInterest = (SubscriptionClaim, Option<i64>);

/// Publication attempts one interest gets before it needs an explicit replay.
const MAX_PUBLICATION_ATTEMPTS: u32 = 8;
const FIRST_RETRY_DELAY_MS: u64 = 5_000;
const MAX_RETRY_DELAY_MS: u64 = 10 * 60_000;
/// How long a claimed attempt keeps other workers away; a crashed attempt is retried afterwards.
/// Unit tests shorten it so a publication can outlast a lease in real time.
const ATTEMPT_LEASE_MS: u64 = if cfg!(test) { 300 } else { 60_000 };
/// Published events are kept as long as AEDS keeps its own deduplication window.
const RETENTION_MS: u64 = 30 * 24 * 60 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxStatus {
    /// Stored and waiting for (another) publication attempt.
    Pending,
    /// Published to AEDS for every matching trigger.
    Published,
    /// Stored, but no active trigger was left to receive it.
    Unmatched,
    /// Attempts ran out for at least one trigger; a replay is needed.
    Failed,
}

/// Where one interest stands for one stored event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    Published,
    /// Attempts ran out; only a replay by the interest's owner revives it.
    Failed,
    /// The interest lost its authority (revoked, reauthorized or changed) before it was
    /// published, so the event is never published for it.
    Fenced,
}

impl DeliveryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Published => "published",
            Self::Failed => "failed",
            Self::Fenced => "fenced",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Self::Pending),
            "published" => Ok(Self::Published),
            "failed" => Ok(Self::Failed),
            "fenced" => Ok(Self::Fenced),
            other => Err(format!("unknown inbox delivery status `{other}`")),
        }
    }
}

/// The delivery of one stored event to one trigger interest.
#[derive(Debug, Clone, PartialEq)]
pub struct InboxDelivery {
    pub connection_id: String,
    pub owner_id: String,
    /// The interest and authorization generation chosen when the event was selected.
    pub interest: SelectedInterest,
    pub status: DeliveryStatus,
    pub attempts: u32,
    pub next_due_at_ms: u64,
    pub published_at_ms: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InboxEntry {
    pub delivery_id: String,
    /// The connection named by the webhook URL, when the provider routes per connection.
    pub route_connection_id: Option<String>,
    pub event: NormalizedEvent,
    pub received_at_ms: u64,
    /// Derived from the delivery rows: published only when none is pending or failed.
    pub status: InboxStatus,
    /// The most attempts any interest has used.
    pub attempts: u32,
    /// When the earliest pending interest is due.
    pub next_attempt_at_ms: Option<u64>,
    pub published_at_ms: Option<u64>,
    pub publication_count: u32,
    pub last_error: Option<String>,
}

/// A publication attempt that owns one delivery row until it finishes, fails or its lease ends.
/// Its token is what lets only the current attempt change that row afterwards.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedInboxDelivery {
    pub delivery_id: String,
    pub interest: SelectedInterest,
    pub attempts: u32,
    token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requeue {
    NotFound,
    /// An attempt for one of the owner's interests is awaiting AEDS right now; replaying would
    /// race it.
    Busy,
    Queued,
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
                  event_json, received_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
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

    /// Whether the interests of `delivery_id` were chosen already.
    pub fn inbox_event_selected(&self, delivery_id: &str) -> Result<bool, String> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT selected FROM webhook_inbox WHERE delivery_id = ?1",
                params![delivery_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map(|selected| selected.unwrap_or(false))
            .map_err(|error| error.to_string())
    }

    /// Chooses the interests of a stored event, once. Returns `false` when another caller chose
    /// them first or the event is gone; the stored choice then stands.
    pub fn select_inbox_interests(
        &self,
        delivery_id: &str,
        selected: &[SelectedInterest],
        now_ms: u64,
    ) -> Result<bool, String> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let marked = transaction
            .execute(
                "UPDATE webhook_inbox SET selected = 1 WHERE delivery_id = ?1 AND selected = 0",
                params![delivery_id],
            )
            .map_err(|error| error.to_string())?;
        if marked == 0 {
            return Ok(false);
        }
        insert_deliveries(&transaction, delivery_id, selected, now_ms)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(true)
    }

    /// The bindings of `delivery_id` that are due for a publication attempt (limited to
    /// `owner_id`'s when given), in publication order. An attempt whose lease ran out with the
    /// budget spent is failed here instead of being offered again.
    pub fn due_inbox_bindings(
        &self,
        delivery_id: &str,
        owner_id: Option<&str>,
        now_ms: u64,
    ) -> Result<Vec<String>, String> {
        let connection = self.lock()?;
        fail_exhausted_leases(&connection, now_ms)?;
        let mut statement = connection
            .prepare(
                "SELECT binding_id FROM webhook_inbox_delivery
                 WHERE delivery_id = ?1 AND status = 'pending' AND next_due_at_ms <= ?2
                   AND (?3 IS NULL OR owner_id = ?3)
                 ORDER BY connection_id, event_interest_key, binding_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![delivery_id, now_ms as i64, owner_id], |row| {
                row.get(0)
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    /// Takes the next publication attempt of one due, pending interest, at the moment its
    /// publication starts, so its lease covers that publication and not the ones before it.
    /// `None` when another caller holds the row, it is not due, or its attempt budget is spent.
    /// Only one caller gets a row until its lease expires, so the inline attempt and the retry
    /// worker never publish the same interest at once.
    pub fn claim_inbox_delivery(
        &self,
        delivery_id: &str,
        binding_id: &str,
        now_ms: u64,
    ) -> Result<Option<ClaimedInboxDelivery>, String> {
        let token = crate::random_opaque("attempt");
        let connection = self.lock()?;
        let claimed = connection
            .execute(
                "UPDATE webhook_inbox_delivery
                 SET attempts = attempts + 1, next_due_at_ms = ?4, attempt_token = ?5
                 WHERE delivery_id = ?1 AND binding_id = ?2 AND status = 'pending'
                   AND next_due_at_ms <= ?3 AND attempts < ?6",
                params![
                    delivery_id,
                    binding_id,
                    now_ms as i64,
                    (now_ms + ATTEMPT_LEASE_MS) as i64,
                    token,
                    MAX_PUBLICATION_ATTEMPTS,
                ],
            )
            .map_err(|error| error.to_string())?;
        if claimed == 0 {
            return Ok(None);
        }
        let delivery = connection
            .query_row(
                &format!(
                    "SELECT {DELIVERY_COLUMNS} FROM webhook_inbox_delivery
                     WHERE delivery_id = ?1 AND binding_id = ?2"
                ),
                params![delivery_id, binding_id],
                read_delivery,
            )
            .map_err(|error| error.to_string())??;
        Ok(Some(ClaimedInboxDelivery {
            delivery_id: delivery_id.to_string(),
            attempts: delivery.attempts,
            interest: delivery.interest,
            token,
        }))
    }

    /// Claims every due interest of `delivery_id` at one instant, as a test stands in for the
    /// attempts that publish them one after another.
    #[cfg(test)]
    pub(crate) fn claim_all_inbox_deliveries(
        &self,
        delivery_id: &str,
        owner_id: Option<&str>,
        now_ms: u64,
    ) -> Result<Vec<ClaimedInboxDelivery>, String> {
        let mut claimed = Vec::new();
        for binding_id in self.due_inbox_bindings(delivery_id, owner_id, now_ms)? {
            claimed.extend(self.claim_inbox_delivery(delivery_id, &binding_id, now_ms)?);
        }
        Ok(claimed)
    }

    /// Stored events that need an attempt: those whose interests were never chosen and those
    /// with a pending interest that is due.
    pub fn due_inbox_deliveries(&self, now_ms: u64, limit: usize) -> Result<Vec<String>, String> {
        let connection = self.lock()?;
        fail_exhausted_leases(&connection, now_ms)?;
        let mut statement = connection
            .prepare(
                "SELECT delivery_id FROM (
                     SELECT delivery_id, received_at_ms AS due FROM webhook_inbox WHERE selected = 0
                     UNION
                     SELECT delivery_id, MIN(next_due_at_ms) FROM webhook_inbox_delivery
                     WHERE status = 'pending' AND next_due_at_ms <= ?1 GROUP BY delivery_id)
                 ORDER BY due LIMIT ?2",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![now_ms as i64, limit as i64], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    /// Records that `claimed` was published. Returns `false` when the attempt was superseded by
    /// a replay or a later claim, in which case the row is left untouched.
    pub fn finish_inbox_delivery(
        &self,
        claimed: &ClaimedInboxDelivery,
        now_ms: u64,
    ) -> Result<bool, String> {
        self.close_inbox_delivery(claimed, DeliveryStatus::Published, now_ms)
    }

    /// Records that `claimed` lost its authority and must never be published. Returns `false`
    /// when the attempt was superseded.
    pub fn fence_inbox_delivery(
        &self,
        claimed: &ClaimedInboxDelivery,
        now_ms: u64,
    ) -> Result<bool, String> {
        self.close_inbox_delivery(claimed, DeliveryStatus::Fenced, now_ms)
    }

    fn close_inbox_delivery(
        &self,
        claimed: &ClaimedInboxDelivery,
        status: DeliveryStatus,
        now_ms: u64,
    ) -> Result<bool, String> {
        let changed = self
            .lock()?
            .execute(
                "UPDATE webhook_inbox_delivery
                 SET status = ?4, published_at_ms = ?5, last_error = NULL, attempt_token = NULL
                 WHERE delivery_id = ?1 AND binding_id = ?2 AND attempt_token = ?3",
                params![
                    claimed.delivery_id,
                    claimed.interest.0.binding_id,
                    claimed.token,
                    status.as_str(),
                    (status == DeliveryStatus::Published).then_some(now_ms as i64),
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    /// Records a failed attempt and schedules the next one, or marks the interest `failed` once
    /// its attempt budget is spent. `None` means the attempt was superseded and nothing changed.
    pub fn fail_inbox_delivery(
        &self,
        claimed: &ClaimedInboxDelivery,
        error: &str,
        now_ms: u64,
    ) -> Result<Option<DeliveryStatus>, String> {
        let status = if claimed.attempts >= MAX_PUBLICATION_ATTEMPTS {
            DeliveryStatus::Failed
        } else {
            DeliveryStatus::Pending
        };
        let changed = self
            .lock()?
            .execute(
                "UPDATE webhook_inbox_delivery
                 SET status = ?4, next_due_at_ms = ?5, last_error = ?6, attempt_token = NULL
                 WHERE delivery_id = ?1 AND binding_id = ?2 AND attempt_token = ?3",
                params![
                    claimed.delivery_id,
                    claimed.interest.0.binding_id,
                    claimed.token,
                    status.as_str(),
                    (now_ms + retry_delay_ms(claimed.attempts)) as i64,
                    error,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok((changed == 1).then_some(status))
    }

    /// Makes a stored event publishable again for one owner, with a fresh attempt budget and no
    /// backoff. `selected` holds that owner's interests chosen again under the current
    /// authorization; they replace the owner's delivery rows and nobody else's. An attempt of
    /// the owner whose lease is still running is never reset; a crashed one is, and its token
    /// stops being valid. An event whose interests were never chosen needs no reset: its first
    /// attempt chooses them for every owner.
    pub fn requeue_inbox_event(
        &self,
        delivery_id: &str,
        owner_id: &str,
        selected: &[SelectedInterest],
        now_ms: u64,
    ) -> Result<Requeue, String> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let Some(already_selected) = transaction
            .query_row(
                "SELECT selected FROM webhook_inbox WHERE delivery_id = ?1",
                params![delivery_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
        else {
            return Ok(Requeue::NotFound);
        };
        if !already_selected {
            return Ok(Requeue::Queued);
        }
        let in_flight: bool = transaction
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM webhook_inbox_delivery
                 WHERE delivery_id = ?1 AND owner_id = ?2
                   AND attempt_token IS NOT NULL AND next_due_at_ms > ?3)",
                params![delivery_id, owner_id, now_ms as i64],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if in_flight {
            return Ok(Requeue::Busy);
        }
        transaction
            .execute(
                "DELETE FROM webhook_inbox_delivery WHERE delivery_id = ?1 AND owner_id = ?2",
                params![delivery_id, owner_id],
            )
            .map_err(|error| error.to_string())?;
        insert_deliveries(&transaction, delivery_id, selected, now_ms)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(Requeue::Queued)
    }

    /// Who owns an interest: its connection's owner, or for a legacy subscription without a
    /// connection row the principal that reconciled it. `None` when the binding is unknown.
    pub fn interest_owner(
        &self,
        connection_id: &str,
        binding_id: &str,
    ) -> Result<Option<String>, String> {
        self.lock()?
            .query_row(
                "SELECT COALESCE(c.owner_id, s.owner_id) FROM subscriptions s
                 LEFT JOIN connections c ON c.connection_id = ?1
                 WHERE s.binding_id = ?2",
                params![connection_id, binding_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// One stored event summarized over every owner's delivery rows, for the publication worker.
    pub fn inbox_event(&self, delivery_id: &str) -> Result<Option<InboxEntry>, String> {
        let connection = self.lock()?;
        let stored = connection
            .query_row(
                &format!("SELECT {INBOX_COLUMNS} FROM webhook_inbox WHERE delivery_id = ?1"),
                params![delivery_id],
                read_entry,
            )
            .optional()
            .map_err(|error| error.to_string())?
            .transpose()?;
        stored
            .map(|stored| summarize(&connection, stored, None))
            .transpose()
    }

    /// The per-interest delivery rows of one stored event.
    pub fn inbox_deliveries(&self, delivery_id: &str) -> Result<Vec<InboxDelivery>, String> {
        delivery_rows(&*self.lock()?, delivery_id, None)
    }

    /// The newest stored events of one owner, newest first, summarized over that owner's
    /// delivery rows only.
    pub fn inbox_events_for_owner(
        &self,
        owner_id: &str,
        limit: usize,
    ) -> Result<Vec<InboxEntry>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {INBOX_COLUMNS} FROM webhook_inbox WHERE {OWNED_BY}
                 ORDER BY received_at_ms DESC, delivery_id LIMIT ?2"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![owner_id, limit as i64], read_entry)
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string())?)
            .map(|entry| summarize(&connection, entry?, Some(owner_id)))
            .collect()
    }

    /// One stored event, only if it belongs to `owner_id`, summarized over that owner's delivery
    /// rows only.
    pub fn inbox_event_for_owner(
        &self,
        owner_id: &str,
        delivery_id: &str,
    ) -> Result<Option<InboxEntry>, String> {
        let connection = self.lock()?;
        let entry = connection
            .query_row(
                &format!(
                    "SELECT {INBOX_COLUMNS} FROM webhook_inbox WHERE {OWNED_BY} AND delivery_id = ?2"
                ),
                params![owner_id, delivery_id],
                read_entry,
            )
            .optional()
            .map_err(|error| error.to_string())?
            .transpose()?;
        entry
            .map(|entry| summarize(&connection, entry, Some(owner_id)))
            .transpose()
    }

    /// Drops events older than the retention window. Events with a pending interest, or whose
    /// interests were never chosen, are never dropped.
    pub fn prune_inbox(&self, now_ms: u64) -> Result<usize, String> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let expired = "FROM webhook_inbox WHERE selected = 1 AND received_at_ms < ?1
             AND NOT EXISTS (SELECT 1 FROM webhook_inbox_delivery d
                 WHERE d.delivery_id = webhook_inbox.delivery_id AND d.status = 'pending')";
        let cutoff = now_ms.saturating_sub(RETENTION_MS) as i64;
        transaction
            .execute(
                &format!(
                    "DELETE FROM webhook_inbox_delivery WHERE delivery_id IN (SELECT delivery_id {expired})"
                ),
                params![cutoff],
            )
            .map_err(|error| error.to_string())?;
        let removed = transaction
            .execute(&format!("DELETE {expired}"), params![cutoff])
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(removed)
    }
}

/// Adds one pending row per selected binding, owned by the connection's owner. A legacy
/// subscription without a connection row is owned by the principal that reconciled it, so
/// every selected claim keeps a row; a claim with no owner at all fails loudly.
fn insert_deliveries(
    connection: &rusqlite::Connection,
    delivery_id: &str,
    selected: &[SelectedInterest],
    now_ms: u64,
) -> Result<(), String> {
    for (claim, generation) in selected {
        let interest_json = serde_json::to_string(claim).map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO webhook_inbox_delivery
                 (delivery_id, binding_id, connection_id, event_interest_key, owner_id,
                  interest_json, authority_generation, status, next_due_at_ms)
                 SELECT ?1, ?7, ?2, ?3, COALESCE(c.owner_id, s.owner_id), ?4, ?5, 'pending', ?6
                 FROM (SELECT 1)
                 LEFT JOIN connections c ON c.connection_id = ?2
                 LEFT JOIN subscriptions s ON s.binding_id = ?7
                 ON CONFLICT(delivery_id, binding_id) DO NOTHING",
                params![
                    delivery_id,
                    claim.connection_id,
                    claim.event_interest_key,
                    interest_json,
                    generation,
                    now_ms as i64,
                    claim.binding_id,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Fails the interests whose last allowed attempt never reported back: the process stopped or
/// stalled, its lease ended, and no attempt is left to claim. Without this a row that keeps
/// crashing would stay pending for ever.
fn fail_exhausted_leases(connection: &rusqlite::Connection, now_ms: u64) -> Result<(), String> {
    connection
        .execute(
            "UPDATE webhook_inbox_delivery
             SET status = 'failed', attempt_token = NULL,
                 last_error = 'attempt ' || attempts || ' did not finish before its lease expired'
             WHERE status = 'pending' AND attempt_token IS NOT NULL AND next_due_at_ms <= ?1
               AND attempts >= ?2",
            params![now_ms as i64, MAX_PUBLICATION_ATTEMPTS],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn retry_delay_ms(attempts: u32) -> u64 {
    FIRST_RETRY_DELAY_MS
        .saturating_mul(1 << attempts.saturating_sub(1).min(20))
        .min(MAX_RETRY_DELAY_MS)
}

/// An event belongs to the owner of the connection that received it: the connection named by
/// the webhook URL, or else the connections whose live trigger subscriptions watch the event's
/// scope (`?1` is the owner id). Subscriptions are reconciled under the kernel's id, so their
/// own `owner_id` says nothing about who owns the event, except for a legacy subscription
/// that has no connection row at all: the principal that reconciled it owns that interest, as
/// `insert_deliveries` records. A revoked connection, an inactive subscription or one from an
/// older authorization generation grants nothing.
const OWNED_BY: &str = "(CASE WHEN route_connection_id IS NOT NULL
     THEN route_connection_id IN (SELECT connection_id FROM connections WHERE owner_id = ?1)
     ELSE (connection_scope IN
         (SELECT s.connection_scope FROM subscriptions s
          JOIN connections c ON c.connection_id = s.connection_id
          JOIN connection_generations g ON g.connection_id = c.connection_id
          JOIN subscription_authorities a ON a.binding_id = s.binding_id
          WHERE c.owner_id = ?1 AND c.status = 'ready' AND s.active = 1
            AND a.connection_owner_id = c.owner_id AND a.provider = c.provider
            AND a.generation = g.generation)
         OR connection_scope IN
         (SELECT s.connection_scope FROM subscriptions s
          WHERE s.owner_id = ?1 AND s.active = 1
            AND NOT EXISTS (SELECT 1 FROM connections c WHERE c.connection_id = s.connection_id)))
     END)";

const INBOX_COLUMNS: &str =
    "delivery_id, route_connection_id, event_json, received_at_ms, selected";

const DELIVERY_COLUMNS: &str = "connection_id, owner_id, interest_json, authority_generation, \
     status, attempts, next_due_at_ms, published_at_ms, last_error";

/// A stored event before its delivery rows are folded into the summary fields.
struct StoredEvent {
    entry: InboxEntry,
    selected: bool,
}

/// The delivery rows of one event, `owner_id`'s only when given.
fn delivery_rows(
    connection: &rusqlite::Connection,
    delivery_id: &str,
    owner_id: Option<&str>,
) -> Result<Vec<InboxDelivery>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM webhook_inbox_delivery WHERE delivery_id = ?1
               AND (?2 IS NULL OR owner_id = ?2)
             ORDER BY connection_id, event_interest_key, binding_id"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![delivery_id, owner_id], read_delivery)
        .map_err(|error| error.to_string())?;
    rows.map(|row| row.map_err(|error| error.to_string())?)
        .collect()
}

/// Folds the delivery rows into the entry's status: pending while any interest still has
/// attempts to make, failed if one ran out, published once every interest is settled. With an
/// `owner_id` only that owner's rows count, so one owner never sees another's outcome.
fn summarize(
    connection: &rusqlite::Connection,
    stored: StoredEvent,
    owner_id: Option<&str>,
) -> Result<InboxEntry, String> {
    let StoredEvent {
        mut entry,
        selected,
    } = stored;
    let rows = delivery_rows(connection, &entry.delivery_id, owner_id)?;
    let any = |status| rows.iter().any(|row| row.status == status);
    entry.status = if !selected || any(DeliveryStatus::Pending) {
        InboxStatus::Pending
    } else if any(DeliveryStatus::Failed) {
        InboxStatus::Failed
    } else if any(DeliveryStatus::Published) {
        InboxStatus::Published
    } else {
        InboxStatus::Unmatched
    };
    entry.attempts = rows.iter().map(|row| row.attempts).max().unwrap_or(0);
    entry.next_attempt_at_ms = if selected {
        rows.iter()
            .filter(|row| row.status == DeliveryStatus::Pending)
            .map(|row| row.next_due_at_ms)
            .min()
    } else {
        Some(entry.received_at_ms)
    };
    entry.publication_count = rows
        .iter()
        .filter(|row| row.status == DeliveryStatus::Published)
        .count() as u32;
    entry.published_at_ms = if entry.status == InboxStatus::Published {
        rows.iter().filter_map(|row| row.published_at_ms).max()
    } else {
        None
    };
    entry.last_error = rows
        .iter()
        .filter(|row| matches!(row.status, DeliveryStatus::Pending | DeliveryStatus::Failed))
        .find_map(|row| row.last_error.clone());
    Ok(entry)
}

fn read_entry(row: &Row<'_>) -> rusqlite::Result<Result<StoredEvent, String>> {
    let event_json: String = row.get(2)?;
    let event = match serde_json::from_str(&event_json) {
        Ok(event) => event,
        Err(error) => return Ok(Err(format!("stored webhook event is unreadable: {error}"))),
    };
    Ok(Ok(StoredEvent {
        entry: InboxEntry {
            delivery_id: row.get(0)?,
            route_connection_id: row.get(1)?,
            event,
            received_at_ms: row.get::<_, i64>(3)? as u64,
            status: InboxStatus::Pending,
            attempts: 0,
            next_attempt_at_ms: None,
            published_at_ms: None,
            publication_count: 0,
            last_error: None,
        },
        selected: row.get(4)?,
    }))
}

fn read_delivery(row: &Row<'_>) -> rusqlite::Result<Result<InboxDelivery, String>> {
    let interest_json: String = row.get(2)?;
    let claim = match serde_json::from_str(&interest_json) {
        Ok(claim) => claim,
        Err(error) => {
            return Ok(Err(format!(
                "stored webhook selection is unreadable: {error}"
            )))
        }
    };
    let status: String = row.get(4)?;
    let status = match DeliveryStatus::parse(&status) {
        Ok(status) => status,
        Err(error) => return Ok(Err(error)),
    };
    Ok(Ok(InboxDelivery {
        connection_id: row.get(0)?,
        owner_id: row.get(1)?,
        interest: (claim, row.get(3)?),
        status,
        attempts: row.get(5)?,
        next_due_at_ms: row.get::<_, i64>(6)? as u64,
        published_at_ms: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
        last_error: row.get(8)?,
    }))
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

    fn claim(connection: &str) -> SubscriptionClaim {
        SubscriptionClaim {
            binding_id: format!("binding-{connection}"),
            generator_id: "dev.chariox.github".into(),
            connection_id: connection.into(),
            connection_scope: "scope".into(),
            event_interest_key: format!("interest-{connection}"),
            event_type: "pull_request.synchronize".into(),
            event_type_version: 1,
            filter: serde_json::Value::Null,
            revision: 1,
            active: true,
        }
    }

    /// One ready connection per `(connection, owner)` and a stored, selected event for all.
    fn selected_store(owners: &[(&str, &str)]) -> (AegsStore, Vec<SelectedInterest>) {
        let store = AegsStore::open(":memory:").unwrap();
        let mut interests = Vec::new();
        for (connection, owner) in owners {
            store
                .upsert_ready_connection(connection, owner, "github", &serde_json::Value::Null, 1)
                .unwrap();
            interests.push((claim(connection), Some(1)));
        }
        store.record_inbox_event("d1", None, &event(), 0).unwrap();
        assert!(store.select_inbox_interests("d1", &interests, 0).unwrap());
        (store, interests)
    }

    #[test]
    fn claimed_delivery_is_leased_and_retries_back_off_up_to_the_budget() {
        let (store, _) = selected_store(&[("c1", "owner")]);
        assert_eq!(
            store
                .claim_all_inbox_deliveries("d1", None, 0)
                .unwrap()
                .len(),
            1
        );
        assert!(store
            .claim_all_inbox_deliveries("d1", None, 1)
            .unwrap()
            .is_empty());
        // A crashed attempt becomes claimable again once its lease ends.
        let mut now = ATTEMPT_LEASE_MS;
        let mut claimed = store.claim_all_inbox_deliveries("d1", None, now).unwrap();
        let mut delays = Vec::new();
        loop {
            let status = store
                .fail_inbox_delivery(&claimed[0], "refused", now)
                .unwrap()
                .unwrap();
            let row = store.inbox_deliveries("d1").unwrap().remove(0);
            delays.push(row.next_due_at_ms - now);
            if status == DeliveryStatus::Failed {
                break;
            }
            now = row.next_due_at_ms;
            claimed = store.claim_all_inbox_deliveries("d1", None, now).unwrap();
        }
        assert_eq!(delays[..3], [10_000, 20_000, 40_000]);
        assert!(delays.iter().all(|delay| *delay <= MAX_RETRY_DELAY_MS));
        assert_eq!(delays.len() as u32, MAX_PUBLICATION_ATTEMPTS - 1);
        assert_eq!(
            store.inbox_event("d1").unwrap().unwrap().status,
            InboxStatus::Failed
        );
    }

    #[test]
    fn crashed_attempts_spend_the_budget_and_the_last_one_fails_with_a_lease_reason() {
        let (store, _) = selected_store(&[("c1", "owner")]);
        let mut now = 0;
        for attempt in 1..=MAX_PUBLICATION_ATTEMPTS {
            let claimed = store.claim_all_inbox_deliveries("d1", None, now).unwrap();
            assert_eq!(claimed.len(), 1, "attempt {attempt}");
            assert_eq!(claimed[0].attempts, attempt);
            // Still leased: it is neither claimable nor failed.
            assert!(store
                .claim_all_inbox_deliveries("d1", None, now + ATTEMPT_LEASE_MS - 1)
                .unwrap()
                .is_empty());
            now += ATTEMPT_LEASE_MS;
        }
        assert_eq!(
            store.inbox_event("d1").unwrap().unwrap().status,
            InboxStatus::Pending,
            "the last attempt may still be running until its lease ends"
        );
        assert_eq!(
            store.due_inbox_deliveries(now, 8).unwrap(),
            Vec::<String>::new()
        );
        let entry = store.inbox_event("d1").unwrap().unwrap();
        assert_eq!(entry.status, InboxStatus::Failed);
        assert_eq!(entry.attempts, MAX_PUBLICATION_ATTEMPTS);
        assert_eq!(
            entry.last_error.as_deref(),
            Some("attempt 8 did not finish before its lease expired")
        );
        assert!(store
            .claim_all_inbox_deliveries("d1", None, now + RETENTION_MS)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn legacy_event_is_owned_by_the_reconciling_principal_without_exposing_connection_owned_events()
    {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .upsert_ready_connection("c-b", "owner-b", "github", &serde_json::Value::Null, 1)
            .unwrap();
        let connection_owned = SubscriptionClaim {
            connection_scope: "other".into(),
            ..claim("c-b")
        };
        store
            .reconcile(
                "kernel-a",
                "dev.chariox.github",
                &[claim("legacy"), connection_owned],
            )
            .unwrap();
        assert_eq!(
            store.interest_owner("legacy", "binding-legacy").unwrap(),
            Some("kernel-a".into())
        );
        assert_eq!(
            store.interest_owner("c-b", "binding-c-b").unwrap(),
            Some("owner-b".into())
        );
        let mut other_scope = event();
        other_scope.connection_scope = "other".into();
        store
            .record_inbox_event("legacy-1", None, &event(), 1)
            .unwrap();
        store
            .record_inbox_event("owned-1", None, &other_scope, 2)
            .unwrap();
        let ids = |owner| {
            store
                .inbox_events_for_owner(owner, 10)
                .unwrap()
                .into_iter()
                .map(|entry| entry.delivery_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("kernel-a"), vec!["legacy-1"]);
        assert_eq!(ids("owner-b"), vec!["owned-1"]);
        assert!(ids("stranger").is_empty());
        assert!(store
            .inbox_event_for_owner("kernel-a", "owned-1")
            .unwrap()
            .is_none());
        // An interest that was deactivated grants nothing.
        store
            .reconcile("kernel-a", "dev.chariox.github", &[])
            .unwrap();
        assert!(ids("kernel-a").is_empty());
    }

    #[test]
    fn superseded_attempt_cannot_change_the_delivery() {
        let (store, interests) = selected_store(&[("c1", "owner")]);
        let first = store
            .claim_all_inbox_deliveries("d1", None, 0)
            .unwrap()
            .remove(0);
        assert_eq!(
            store
                .requeue_inbox_event("d1", "owner", &interests, 1)
                .unwrap(),
            Requeue::Busy
        );
        // The first attempt crashed or stalled past its lease; a replay takes over.
        assert_eq!(
            store
                .requeue_inbox_event("d1", "owner", &interests, ATTEMPT_LEASE_MS)
                .unwrap(),
            Requeue::Queued
        );
        let second = store
            .claim_all_inbox_deliveries("d1", None, ATTEMPT_LEASE_MS)
            .unwrap()
            .remove(0);
        assert!(!store.finish_inbox_delivery(&first, 5).unwrap());
        assert!(!store.fence_inbox_delivery(&first, 5).unwrap());
        assert_eq!(
            store
                .fail_inbox_delivery(&first, "late failure", 5)
                .unwrap(),
            None
        );
        let entry = store.inbox_event("d1").unwrap().unwrap();
        assert_eq!(entry.status, InboxStatus::Pending);
        assert_eq!(entry.last_error, None);
        assert!(store.finish_inbox_delivery(&second, 6).unwrap());
        assert_eq!(
            store.inbox_event("d1").unwrap().unwrap().status,
            InboxStatus::Published
        );
        // A failure that arrives after success does not reopen the delivery.
        assert_eq!(store.fail_inbox_delivery(&second, "late", 7).unwrap(), None);
        assert_eq!(
            store
                .requeue_inbox_event("missing", "owner", &[], 8)
                .unwrap(),
            Requeue::NotFound
        );
    }

    #[test]
    fn legacy_subscription_without_a_connection_row_still_gets_a_delivery_row() {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .reconcile("kernel-a", "dev.chariox.github", &[claim("legacy")])
            .unwrap();
        store.record_inbox_event("d1", None, &event(), 0).unwrap();
        let selected = [(claim("legacy"), None)];
        assert!(store.select_inbox_interests("d1", &selected, 0).unwrap());
        let rows = store.inbox_deliveries("d1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].owner_id, "kernel-a");
        assert_eq!(rows[0].status, DeliveryStatus::Pending);
        let claimed = store.claim_all_inbox_deliveries("d1", None, 0).unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(store.finish_inbox_delivery(&claimed[0], 1).unwrap());
        assert_eq!(
            store.inbox_event("d1").unwrap().unwrap().status,
            InboxStatus::Published
        );
    }

    #[test]
    fn entry_is_published_only_when_every_interest_is_settled() {
        let (store, _) = selected_store(&[("c-a", "owner-a"), ("c-b", "owner-b")]);
        let claimed = store.claim_all_inbox_deliveries("d1", None, 0).unwrap();
        assert!(store.finish_inbox_delivery(&claimed[0], 1).unwrap());
        let entry = store.inbox_event("d1").unwrap().unwrap();
        assert_eq!(entry.status, InboxStatus::Pending);
        assert_eq!(entry.publication_count, 1);
        assert_eq!(entry.published_at_ms, None);
        assert!(store.fence_inbox_delivery(&claimed[1], 2).unwrap());
        let entry = store.inbox_event("d1").unwrap().unwrap();
        assert_eq!(entry.status, InboxStatus::Published);
        assert_eq!(entry.published_at_ms, Some(1));
    }

    #[test]
    fn pruning_keeps_pending_events_and_recent_ones() {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .upsert_ready_connection("c1", "owner", "github", &serde_json::Value::Null, 1)
            .unwrap();
        let interests = [(claim("c1"), Some(1))];
        for (id, at) in [("pending", 0), ("old", 0), ("recent", RETENTION_MS)] {
            store.record_inbox_event(id, None, &event(), at).unwrap();
            store.select_inbox_interests(id, &interests, at).unwrap();
        }
        for (id, at) in [("old", 0), ("recent", RETENTION_MS)] {
            let claimed = store.claim_all_inbox_deliveries(id, None, at).unwrap();
            store.finish_inbox_delivery(&claimed[0], at + 1).unwrap();
        }
        assert_eq!(store.prune_inbox(RETENTION_MS + 1).unwrap(), 1);
        assert!(store.has_inbox_event("pending").unwrap());
        assert!(store.has_inbox_event("recent").unwrap());
        assert!(!store.has_inbox_event("old").unwrap());
        assert!(store.inbox_deliveries("old").unwrap().is_empty());
    }

    #[test]
    fn replay_replaces_only_the_requesting_owners_rows() {
        let (store, interests) = selected_store(&[("c-a", "owner-a"), ("c-b", "owner-b")]);
        // B's attempt failed and waits in backoff; A's published.
        let claimed = store.claim_all_inbox_deliveries("d1", None, 0).unwrap();
        assert!(store.finish_inbox_delivery(&claimed[0], 1).unwrap());
        store
            .fail_inbox_delivery(&claimed[1], "refused", 1)
            .unwrap();
        let before = store.inbox_deliveries("d1").unwrap();

        assert_eq!(
            store
                .requeue_inbox_event("d1", "owner-a", &interests[..1], 2)
                .unwrap(),
            Requeue::Queued
        );
        let after = store.inbox_deliveries("d1").unwrap();
        assert_eq!(after[1], before[1], "B keeps its attempts, timer and error");
        assert_eq!(after[0].status, DeliveryStatus::Pending);
        assert_eq!(after[0].attempts, 0);
        // A's replay claims A's row only; B's backoff still holds.
        assert_eq!(
            store
                .claim_all_inbox_deliveries("d1", Some("owner-a"), 2)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(store.inbox_deliveries("d1").unwrap()[1], before[1]);
        assert_eq!(
            store.inbox_event("d1").unwrap().unwrap().status,
            InboxStatus::Pending
        );
    }

    #[test]
    fn owner_summaries_fold_only_the_requesting_owners_rows() {
        let (store, _) = selected_store(&[("c-a", "owner-a"), ("c-b", "owner-b")]);
        store
            .reconcile(
                "kernel",
                "dev.chariox.github",
                &[claim("c-a"), claim("c-b")],
            )
            .unwrap();
        // A published; B ran out of attempts.
        let claimed = store.claim_all_inbox_deliveries("d1", None, 0).unwrap();
        assert!(store.finish_inbox_delivery(&claimed[0], 1).unwrap());
        let mut token = claimed[1].clone();
        let mut now = 1;
        loop {
            let status = store
                .fail_inbox_delivery(&token, "refused", now)
                .unwrap()
                .unwrap();
            if status == DeliveryStatus::Failed {
                break;
            }
            now += MAX_RETRY_DELAY_MS;
            token = store
                .claim_all_inbox_deliveries("d1", Some("owner-b"), now)
                .unwrap()
                .remove(0);
        }

        let for_a = store
            .inbox_event_for_owner("owner-a", "d1")
            .unwrap()
            .unwrap();
        assert_eq!(for_a.status, InboxStatus::Published);
        assert_eq!(
            (for_a.attempts, for_a.last_error),
            (1, None),
            "not B's attempts"
        );
        let listed = store.inbox_events_for_owner("owner-a", 10).unwrap();
        assert_eq!(listed[0].status, InboxStatus::Published);
        assert_eq!(listed[0].last_error, None);

        let for_b = store
            .inbox_event_for_owner("owner-b", "d1")
            .unwrap()
            .unwrap();
        assert_eq!(for_b.status, InboxStatus::Failed);
        assert_eq!(for_b.last_error.as_deref(), Some("refused"));
        assert_eq!(for_b.publication_count, 0);

        assert_eq!(
            store.inbox_event("d1").unwrap().unwrap().status,
            InboxStatus::Failed,
            "the worker's summary stays aggregate"
        );
    }

    #[test]
    fn routed_event_belongs_to_the_routed_connections_owner_even_when_scopes_are_shared() {
        let store = AegsStore::open(":memory:").unwrap();
        let mut claims = Vec::new();
        for (connection, owner) in [("c-a", "owner-a"), ("c-b", "owner-b")] {
            store
                .upsert_ready_connection(connection, owner, "github", &serde_json::Value::Null, 1)
                .unwrap();
            claims.push(SubscriptionClaim {
                binding_id: format!("binding-{connection}"),
                generator_id: "dev.chariox.github".into(),
                connection_id: connection.into(),
                connection_scope: "scope".into(),
                event_interest_key: format!("interest-{connection}"),
                event_type: "pull_request.synchronize".into(),
                event_type_version: 1,
                filter: serde_json::Value::Null,
                revision: 1,
                active: true,
            });
        }
        // One reconciliation carries both owners' triggers, as the kernel sends them.
        store
            .reconcile("kernel", "dev.chariox.github", &claims)
            .unwrap();
        store
            .record_inbox_event("routed", Some("c-a"), &event(), 1)
            .unwrap();
        store
            .record_inbox_event("unrouted", None, &event(), 2)
            .unwrap();
        let ids = |owner| {
            store
                .inbox_events_for_owner(owner, 10)
                .unwrap()
                .into_iter()
                .map(|entry| entry.delivery_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("owner-a"), vec!["unrouted", "routed"]);
        assert_eq!(ids("owner-b"), vec!["unrouted"]);
        assert!(ids("kernel").is_empty());
    }

    #[test]
    fn revoked_connection_no_longer_grants_access_to_events_in_its_scope() {
        let store = AegsStore::open(":memory:").unwrap();
        let mut claims = Vec::new();
        for (connection, owner) in [("c-a", "owner-a"), ("c-b", "owner-b")] {
            store
                .upsert_ready_connection(connection, owner, "github", &serde_json::Value::Null, 1)
                .unwrap();
            claims.push(SubscriptionClaim {
                binding_id: format!("binding-{connection}"),
                generator_id: "dev.chariox.github".into(),
                connection_id: connection.into(),
                connection_scope: "scope".into(),
                event_interest_key: format!("interest-{connection}"),
                event_type: "pull_request.synchronize".into(),
                event_type_version: 1,
                filter: serde_json::Value::Null,
                revision: 1,
                active: true,
            });
        }
        // One reconciliation carries both owners' triggers, as the kernel sends them.
        store
            .reconcile("kernel", "dev.chariox.github", &claims)
            .unwrap();
        store.record_inbox_event("late", None, &event(), 5).unwrap();
        assert!(store
            .inbox_event_for_owner("owner-a", "late")
            .unwrap()
            .is_some());

        assert!(store.revoke_connection("c-a", "owner-a", 3).unwrap());
        assert!(store
            .inbox_events_for_owner("owner-a", 10)
            .unwrap()
            .is_empty());
        assert!(store
            .inbox_event_for_owner("owner-a", "late")
            .unwrap()
            .is_none());
        assert!(store
            .inbox_event_for_owner("owner-b", "late")
            .unwrap()
            .is_some());
    }
}
