use super::*;
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptState {
    Accepted,
    Queued,
    Delivered,
    Retryable,
    Failed,
    Expired,
}
impl ReceiptState {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "queued" => Ok(Self::Queued),
            "delivered" => Ok(Self::Delivered),
            "retryable" => Ok(Self::Retryable),
            "failed" => Ok(Self::Failed),
            "expired" => Ok(Self::Expired),
            _ => Err(OutboxError::Corrupt),
        }
    }
}

/// Provisional until the transaction producing it has committed. A queued
/// receipt proves workflow queue insertion only when the kernel composes that
/// insertion and mark_queued_in inside the same committing writer transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub receipt_id: String,
    pub automation_id: String,
    pub occurrence_id: String,
    pub event_version: u32,
    pub occurred_at_ms: u64,
    pub schedule_revision: Option<String>,
    pub state: ReceiptState,
    pub revision: u64,
    pub attempts: u32,
    pub accepted_at_ms: u64,
    pub expires_at_ms: u64,
    pub next_attempt_at_ms: u64,
    pub queued_prompt_id: Option<String>,
    pub queued_session_id: Option<String>,
    pub payload: Option<String>,
    pub invocation: Option<String>,
    pub content_digest: String,
    pub automation_revision: u64,
}

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_automations (
            owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, automation_id TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK(revision>0), event_name TEXT NOT NULL,
            event_version INTEGER NOT NULL CHECK(event_version>0 AND event_version<=4294967295),
            schema_digest TEXT NOT NULL, session_id TEXT NOT NULL, publication_id TEXT NOT NULL,
            endpoint_id TEXT NOT NULL, queue_id TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('active','paused','broken','disabled')),
            scheduled INTEGER NOT NULL DEFAULT 0 CHECK(scheduled IN (0,1)),
            PRIMARY KEY(owner_id,installation_id,automation_id)
         );
         CREATE TABLE IF NOT EXISTS app_outbox (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, receipt_id TEXT NOT NULL UNIQUE,
            automation_id TEXT NOT NULL, event_version INTEGER NOT NULL, occurrence_id TEXT NOT NULL,
            occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms>=0 AND occurred_at_ms<=9007199254740991),
            schedule_revision TEXT NOT NULL DEFAULT '',
            event_name TEXT NOT NULL, schema_digest TEXT NOT NULL, content_digest TEXT NOT NULL,
            automation_revision INTEGER NOT NULL, accepted_generation INTEGER NOT NULL,
            payload_json TEXT, invocation_json TEXT, accepted_at_ms INTEGER NOT NULL, expires_at_ms INTEGER NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('accepted','queued','delivered','retryable','failed','expired')),
            revision INTEGER NOT NULL CHECK(revision>0), attempts INTEGER NOT NULL CHECK(attempts>=0 AND attempts<=8),
            next_attempt_at_ms INTEGER NOT NULL, queued_prompt_id TEXT, queued_session_id TEXT,
            UNIQUE(owner_id,installation_id,automation_id,event_version,occurrence_id,schedule_revision)
         );
         CREATE INDEX IF NOT EXISTS app_outbox_pending
            ON app_outbox(owner_id,installation_id,state,next_attempt_at_ms,sequence);
         CREATE TRIGGER IF NOT EXISTS app_automation_limit BEFORE INSERT ON app_automations
            WHEN (SELECT count(*) FROM app_automations WHERE owner_id=NEW.owner_id AND installation_id=NEW.installation_id)>=256
            BEGIN SELECT RAISE(ABORT,'app_automation_limit'); END;"
    )?;
    // Pre-release protocol 290 rows have no canonical invocation. Preserve
    // their receipts/digests; they cannot be delivered by the 291 handoff. Never
    // infer a prompt from an event's domain payload during migration.
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('app_outbox') WHERE name='invocation_json')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        connection.execute_batch("ALTER TABLE app_outbox ADD COLUMN invocation_json TEXT;")?;
    }
    let has_session: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('app_outbox') WHERE name='queued_session_id')",
        [], |row| row.get(0),
    )?;
    if !has_session {
        connection.execute_batch("ALTER TABLE app_outbox ADD COLUMN queued_session_id TEXT;")?;
    }
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_outbox_replay_floors (
        owner_id TEXT NOT NULL, installation_id TEXT NOT NULL,
        minimum_occurred_at_ms INTEGER NOT NULL CHECK(minimum_occurred_at_ms>=0),
        queued_after_sequence INTEGER NOT NULL DEFAULT 0 CHECK(queued_after_sequence>=0),
        PRIMARY KEY(owner_id,installation_id)
    );",
    )?;
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS app_outbox_queued_workflow
        ON app_outbox(queued_prompt_id,queued_session_id) WHERE state='queued';",
    )?;
    // Old pending occurrences cannot acquire an invented invocation. Retain
    // their immutable receipt identity/digest, but release queue/payload capacity
    // so a page of unsupported rows cannot starve current-contract deliveries.
    connection.execute_batch(
        "UPDATE app_outbox SET state='failed',payload_json=NULL,invocation_json=NULL,
        revision=CASE WHEN revision<9223372036854775807 THEN revision+1 ELSE revision END
        WHERE state IN ('accepted','retryable') AND invocation_json IS NULL;",
    )?;
    Ok(())
}

pub(super) fn accept(
    tx: &Connection,
    automation: &VerifiedAutomation,
    occurrence: &Occurrence,
    payload: &str,
    invocation: &str,
    now: u64,
) -> Result<Receipt> {
    // Stable schema/envelope version plus canonical payload. Worker generation
    // and mutable automation revision are deliberately outside occurrence identity.
    let canonical = serde_json_canonicalizer::to_vec(&serde_json::json!({
        "contract":"chariox.app-occurrence.v2", "event":automation.binding.event_name,
        "eventVersion":occurrence.event_version,"schema":automation.binding.schema_digest,
        "occurredAtMs":occurrence.occurred_at_ms,"scheduleRevision":occurrence.schedule_revision,
        "payload":serde_json::from_str::<Value>(payload).map_err(|_|OutboxError::Invalid)?,
        "invocation":serde_json::from_str::<Value>(invocation).map_err(|_|OutboxError::Invalid)?,
    }))
    .map_err(|_| OutboxError::Invalid)?;
    let content_digest = digest(&canonical);
    let existing:Option<(String,String)> = tx.query_row(
        "SELECT receipt_id,content_digest FROM app_outbox WHERE owner_id=?1 AND installation_id=?2
         AND automation_id=?3 AND event_version=?4 AND occurrence_id=?5 AND schedule_revision=?6",
        params![automation.owner,automation.installation_id(),automation.id(),occurrence.event_version,occurrence.occurrence_id,occurrence.schedule_revision.as_deref().unwrap_or("")],
        |row|Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    if let Some((id, previous)) = existing {
        if previous != content_digest {
            return Err(OutboxError::Conflict);
        }
        return receipt(tx, &automation.owner, automation.installation_id(), &id);
    }
    // Existing duplicates precede the durable replay floor. A pruned identity
    // embeds its immutable original time and can never become new after the floor.
    let floor =
        maintenance::advance_floor(tx, &automation.owner, automation.installation_id(), now)?;
    if occurrence.occurred_at_ms > now.saturating_add(MAX_FUTURE_SKEW_MS) {
        return Err(OutboxError::Invalid);
    }
    if occurrence.occurred_at_ms < floor {
        return Err(OutboxError::TooOld);
    }
    let (count,pending,bytes):(i64,i64,i64)=tx.query_row(
        "SELECT count(*),coalesce(sum(state IN ('accepted','retryable')),0),coalesce(sum(coalesce(length(CAST(payload_json AS BLOB)),0)+coalesce(length(CAST(invocation_json AS BLOB)),0)),0)
         FROM app_outbox WHERE owner_id=?1 AND installation_id=?2",
        params![automation.owner,automation.installation_id()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    )?;
    if count >= MAX_RECEIPTS as i64
        || pending >= MAX_PENDING as i64
        || bytes < 0
        || bytes
            .checked_add((payload.len() + invocation.len()) as i64)
            .ok_or(OutboxError::Limit)?
            > MAX_RETAINED_PAYLOAD_BYTES as i64
    {
        return Err(OutboxError::Limit);
    }
    let id: String = tx.query_row(
        "SELECT 'app_receipt_' || lower(hex(randomblob(16)))",
        [],
        |row| row.get(0),
    )?;
    let expires = now
        .checked_add(PENDING_LIFETIME_MS)
        .ok_or(OutboxError::Invalid)?;
    tx.execute(
        "INSERT INTO app_outbox (owner_id,installation_id,receipt_id,automation_id,event_version,occurrence_id,event_name,
            schema_digest,content_digest,automation_revision,accepted_generation,payload_json,accepted_at_ms,expires_at_ms,
            state,revision,attempts,next_attempt_at_ms,occurred_at_ms,schedule_revision,invocation_json)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,'accepted',1,0,?13,?15,?16,?17)",
        params![automation.owner,automation.installation_id(),id,automation.id(),occurrence.event_version,
            occurrence.occurrence_id,automation.binding.event_name,automation.binding.schema_digest,content_digest,
            time(automation.revision())?,time(automation.catalog.generation())?,payload,time(now)?,time(expires)?,
            time(occurrence.occurred_at_ms)?,occurrence.schedule_revision.as_deref().unwrap_or(""),invocation],
    )?;
    receipt(tx, &automation.owner, automation.installation_id(), &id)
}

const COLUMNS:&str="receipt_id,automation_id,occurrence_id,event_version,state,revision,attempts,accepted_at_ms,expires_at_ms,next_attempt_at_ms,queued_prompt_id,payload_json,content_digest,automation_revision,occurred_at_ms,schedule_revision,invocation_json,queued_session_id";
fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Receipt> {
    let positive = |index: usize| -> rusqlite::Result<u64> {
        u64::try_from(row.get::<_, i64>(index)?).map_err(|_| rusqlite::Error::InvalidQuery)
    };
    Ok(Receipt {
        receipt_id: row.get(0)?,
        automation_id: row.get(1)?,
        occurrence_id: row.get(2)?,
        event_version: u32::try_from(positive(3)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        state: ReceiptState::parse(&row.get::<_, String>(4)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        revision: positive(5)?,
        attempts: u32::try_from(positive(6)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        accepted_at_ms: positive(7)?,
        expires_at_ms: positive(8)?,
        next_attempt_at_ms: positive(9)?,
        queued_prompt_id: row.get(10)?,
        queued_session_id: row.get(17)?,
        payload: row.get(11)?,
        invocation: row.get(16)?,
        content_digest: row.get(12)?,
        automation_revision: positive(13)?,
        occurred_at_ms: positive(14)?,
        schedule_revision: match row.get::<_, String>(15)? {
            value if value.is_empty() => None,
            value => Some(value),
        },
    })
}
pub(super) fn receipt(
    tx: &Connection,
    owner: &str,
    installation: &str,
    id: &str,
) -> Result<Receipt> {
    tx.query_row(&format!("SELECT {COLUMNS} FROM app_outbox WHERE owner_id=?1 AND installation_id=?2 AND receipt_id=?3"),
        params![owner,installation,id],decode).optional()?.ok_or(OutboxError::NotFound)
}
pub(super) fn pending(
    tx: &Connection,
    owner: &str,
    installation: &str,
    now: u64,
    limit: usize,
) -> Result<Vec<Receipt>> {
    let mut statement=tx.prepare(&format!("SELECT {COLUMNS} FROM app_outbox
        WHERE owner_id=?1 AND installation_id=?2 AND state IN ('accepted','retryable') AND next_attempt_at_ms<=?3
        AND NOT EXISTS (SELECT 1 FROM app_automations a WHERE a.owner_id=app_outbox.owner_id
          AND a.installation_id=app_outbox.installation_id AND a.automation_id=app_outbox.automation_id AND a.status='paused')
        ORDER BY sequence LIMIT ?4"))?;
    let rows = statement.query_map(
        params![owner, installation, time(now)?, limit as i64],
        decode,
    )?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

impl AppOutbox {
    /// Call only in the writer transaction that inserts the matching existing
    /// workflow queue item and workflow receipt. This method never dispatches.
    pub fn mark_queued_in(
        tx: &Transaction<'_>,
        automation: &VerifiedAutomation,
        id: &str,
        expected_revision: u64,
        queued_prompt_id: &str,
        now: u64,
    ) -> Result<Receipt> {
        identifier(queued_prompt_id)?;
        automation.require_current(tx)?;
        let previous = transition_input(tx, automation, id, expected_revision, now)?;
        if previous.accepted_at_ms > now
            || previous.expires_at_ms <= now
            || previous.next_attempt_at_ms > now
        {
            return Err(OutboxError::Conflict);
        }
        let changed=tx.execute("UPDATE app_outbox SET state='queued',revision=revision+1,attempts=attempts+1,
            queued_prompt_id=?1,queued_session_id=?6,payload_json=NULL,invocation_json=NULL WHERE owner_id=?2 AND installation_id=?3 AND receipt_id=?4 AND revision=?5",
            params![queued_prompt_id,automation.owner,automation.installation_id(),id,time(expected_revision)?,automation.binding.session_id])?;
        if changed != 1 {
            return Err(OutboxError::Conflict);
        }
        receipt(tx, &automation.owner, automation.installation_id(), id)
    }

    /// Records a kernel-classified retryable queue failure. It cannot be called
    /// by an App to retry a failed/expired/queued occurrence or an external effect.
    pub fn mark_retryable_in(
        tx: &Transaction<'_>,
        automation: &VerifiedAutomation,
        id: &str,
        expected_revision: u64,
        retry_at_ms: u64,
        now: u64,
    ) -> Result<Receipt> {
        automation.require_current(tx)?;
        let previous = transition_input(tx, automation, id, expected_revision, now)?;
        if retry_at_ms <= now
            || retry_at_ms >= previous.expires_at_ms
            || previous.attempts + 1 >= MAX_ATTEMPTS
        {
            return Err(OutboxError::Limit);
        }
        let changed=tx.execute("UPDATE app_outbox SET state='retryable',revision=revision+1,attempts=attempts+1,
            next_attempt_at_ms=?1 WHERE owner_id=?2 AND installation_id=?3 AND receipt_id=?4 AND revision=?5",
            params![time(retry_at_ms)?,automation.owner,automation.installation_id(),id,time(expected_revision)?])?;
        if changed != 1 {
            return Err(OutboxError::Conflict);
        }
        receipt(tx, &automation.owner, automation.installation_id(), id)
    }

    /// A real kernel queue attempt failed permanently (including the eighth
    /// attempt or insufficient lifetime for another backoff). Count that final
    /// attempt without allowing an App to reset or resurrect the receipt.
    pub fn mark_failed_attempt_in(
        tx: &Transaction<'_>,
        automation: &VerifiedAutomation,
        id: &str,
        expected_revision: u64,
        now: u64,
    ) -> Result<Receipt> {
        automation.require_current(tx)?;
        transition_input(tx, automation, id, expected_revision, now)?;
        let changed=tx.execute("UPDATE app_outbox SET state='failed',revision=revision+1,attempts=attempts+1,
            payload_json=NULL,invocation_json=NULL WHERE owner_id=?1 AND installation_id=?2 AND receipt_id=?3 AND revision=?4",
            params![automation.owner,automation.installation_id(),id,time(expected_revision)?])?;
        if changed != 1 {
            return Err(OutboxError::Conflict);
        }
        receipt(tx, &automation.owner, automation.installation_id(), id)
    }

    /// Terminal classification retains identity, immutable content digest, and
    /// receipt until the bounded terminal retention policy can reclaim it.
    /// Capacity exhaustion applies backpressure within the rolling retention window.
    pub fn settle_in(
        tx: &Transaction<'_>,
        catalog: &EventCatalog,
        owner: &str,
        id: &str,
        expected_revision: u64,
        state: ReceiptState,
        now: u64,
    ) -> Result<Receipt> {
        catalog.require_current(tx, owner)?;
        time(now)?;
        identifier(id)?;
        let previous = receipt(tx, owner, catalog.installation_id(), id)?;
        if previous.revision != expected_revision
            || expected_revision >= i64::MAX as u64
            || now < previous.accepted_at_ms
        {
            return Err(OutboxError::Conflict);
        }
        let target = match (previous.state, state) {
            (ReceiptState::Queued, ReceiptState::Delivered) => "delivered",
            (ReceiptState::Accepted | ReceiptState::Retryable, ReceiptState::Failed) => "failed",
            (ReceiptState::Accepted | ReceiptState::Retryable, ReceiptState::Expired)
                if now >= previous.expires_at_ms =>
            {
                "expired"
            }
            _ => return Err(OutboxError::Conflict),
        };
        let changed=tx.execute("UPDATE app_outbox SET state=?1,revision=revision+1,payload_json=NULL,invocation_json=NULL WHERE owner_id=?2 AND installation_id=?3 AND receipt_id=?4 AND revision=?5",
            params![target,owner,catalog.installation_id(),id,time(expected_revision)?])?;
        if changed != 1 {
            return Err(OutboxError::Conflict);
        }
        receipt(tx, owner, catalog.installation_id(), id)
    }
}

pub(super) fn transition_input(
    tx: &Transaction<'_>,
    automation: &VerifiedAutomation,
    id: &str,
    revision: u64,
    now: u64,
) -> Result<Receipt> {
    identifier(id)?;
    time(now)?;
    let previous = receipt(tx, &automation.owner, automation.installation_id(), id)?;
    if previous.revision != revision
        || revision >= i64::MAX as u64
        || previous.automation_id != automation.id()
        || previous.automation_revision != automation.revision()
        || previous.attempts >= MAX_ATTEMPTS
        || previous.payload.is_none()
        || previous.invocation.is_none()
        || !matches!(
            previous.state,
            ReceiptState::Accepted | ReceiptState::Retryable
        )
        || now < previous.accepted_at_ms
        || now >= previous.expires_at_ms
        || now < previous.next_attempt_at_ms
    {
        return Err(OutboxError::Conflict);
    }
    Ok(previous)
}
