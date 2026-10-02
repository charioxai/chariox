//! Kernel-owned App wakes. An App registers a durable due time; the kernel
//! starts the worker when it falls due and delivers the wake at least once.
//! Wakes are installation data, not a workflow trigger or a keep-alive grant:
//! only a wake armed while the App served a tool call or an inbound event
//! counts as use when it is delivered (owner decision 6).
use super::*;
use rusqlite::params;
use serde::{Deserialize, Serialize};

pub const MAX_WAKES: usize = 256;
pub const MAX_WAKE_CHANGES: usize = 16;
const MAX_ATTEMPTS: u32 = 8;
const RETRY_BASE_MS: u64 = 5_000;
/// Longer than any delay a delivery outcome sets (the last backoff, or the
/// kernel's short postponements). A next attempt further ahead was set by a
/// wall clock that has since moved back, so it is not left waiting that long.
const MAX_DELAY_MS: u64 = RETRY_BASE_MS << MAX_ATTEMPTS;

/// A wake set by the App. `revision` is App-defined (for example a schedule
/// revision) and is delivered back unchanged so stale wakes can be ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Wake {
    pub id: String,
    pub due_at_ms: u64,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum WakeChange {
    Set(Wake),
    Cancel { id: String },
}

/// A due wake claimed for delivery. `attempts` counts earlier failed deliveries.
/// `counts_as_use` records whether it was armed during a tool call or an
/// inbound event; a wake the App armed from its own wake handler or timer
/// does not keep it running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueWake {
    pub owner_id: String,
    pub installation_id: String,
    pub wake: Wake,
    pub attempts: u32,
    pub counts_as_use: bool,
}

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_wakes (
           owner_id TEXT NOT NULL,
           installation_id TEXT NOT NULL,
           wake_id TEXT NOT NULL,
           due_at_ms INTEGER NOT NULL CHECK(due_at_ms >= 0),
           revision TEXT NOT NULL,
           attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
           next_attempt_at_ms INTEGER NOT NULL CHECK(next_attempt_at_ms >= 0),
           counts_as_use INTEGER NOT NULL DEFAULT 0 CHECK(counts_as_use IN (0,1)),
           PRIMARY KEY(installation_id, wake_id)
         );
         CREATE INDEX IF NOT EXISTS app_wakes_due ON app_wakes(next_attempt_at_ms);
         CREATE TABLE IF NOT EXISTS app_wake_clock (
           singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
           last_poll_at_ms INTEGER NOT NULL CHECK(last_poll_at_ms >= 0)
         );
         INSERT OR IGNORE INTO app_wake_clock VALUES (1, 0);",
    )?;
    // Wakes armed before their origin was recorded do not count as use.
    let recorded: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('app_wakes') WHERE name='counts_as_use')",
        [],
        |row| row.get(0),
    )?;
    if !recorded {
        connection.execute_batch(
            "ALTER TABLE app_wakes ADD COLUMN counts_as_use INTEGER NOT NULL DEFAULT 0 CHECK(counts_as_use IN (0,1));",
        )?;
    }
    Ok(())
}

fn identity(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_KEY_BYTES
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(StateError::Invalid);
    }
    Ok(())
}

fn validate(wake: &Wake) -> Result<()> {
    identity(&wake.id)?;
    if wake.revision.len() > MAX_KEY_BYTES
        || wake.revision.chars().any(char::is_control)
        || wake.due_at_ms > MAX_REVISION
    {
        return Err(StateError::Invalid);
    }
    Ok(())
}

/// `counts_as_use` is the arming context: a set replaces the wake's origin
/// along with its due time.
pub(super) fn apply(
    transaction: &Connection,
    scope: StateScope<'_>,
    changes: &[WakeChange],
    counts_as_use: bool,
) -> Result<()> {
    if changes.len() > MAX_WAKE_CHANGES {
        return Err(StateError::Limit);
    }
    if changes.is_empty() {
        return Ok(());
    }
    store::active(transaction, scope)?;
    for change in changes {
        match change {
            WakeChange::Set(wake) => {
                validate(wake)?;
                transaction.execute(
                    "INSERT INTO app_wakes(owner_id,installation_id,wake_id,due_at_ms,revision,attempts,next_attempt_at_ms,counts_as_use)
                     VALUES(?1,?2,?3,?4,?5,0,?4,?6)
                     ON CONFLICT(installation_id,wake_id) DO UPDATE SET
                       due_at_ms=excluded.due_at_ms, revision=excluded.revision,
                       attempts=0, next_attempt_at_ms=excluded.due_at_ms,
                       counts_as_use=excluded.counts_as_use",
                    params![scope.owner, scope.installation, wake.id, wake.due_at_ms as i64, wake.revision, counts_as_use],
                )?;
            }
            WakeChange::Cancel { id } => {
                identity(id)?;
                transaction.execute(
                    "DELETE FROM app_wakes WHERE installation_id=?1 AND wake_id=?2",
                    params![scope.installation, id],
                )?;
            }
        }
    }
    let count: i64 = transaction.query_row(
        "SELECT count(*) FROM app_wakes WHERE installation_id=?1",
        params![scope.installation],
        |row| row.get(0),
    )?;
    if count as usize > MAX_WAKES {
        return Err(StateError::Limit);
    }
    Ok(())
}

pub(super) fn list(transaction: &Connection, scope: StateScope<'_>) -> Result<Vec<Wake>> {
    store::active(transaction, scope)?;
    let mut statement = transaction.prepare(
        "SELECT wake_id,due_at_ms,revision FROM app_wakes WHERE installation_id=?1
         ORDER BY due_at_ms, wake_id LIMIT ?2",
    )?;
    let rows = statement.query_map(params![scope.installation, MAX_WAKES as i64], row_wake)?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

fn row_wake(row: &rusqlite::Row<'_>) -> rusqlite::Result<Wake> {
    Ok(Wake {
        id: row.get(0)?,
        due_at_ms: row.get::<_, i64>(1)?.max(0) as u64,
        revision: row.get(2)?,
    })
}

/// Due wakes across installations, oldest first. Reading claims nothing: the
/// kernel's single wake pump delivers them and then completes or defers each.
/// Due times are absolute epoch milliseconds, so time zones and DST never move
/// them; after the wall clock moves back, a due wake whose retry was set by the
/// earlier clock is due again at once.
pub fn due_wakes(connection: &Connection, now_ms: u64, limit: usize) -> Result<Vec<DueWake>> {
    // Persist the observed clock so even a small correction across a restart
    // releases old retry deadlines. Future wakes keep their original due time.
    // Use the caller's transaction when present (for durable checkpoint tests).
    let transaction = connection
        .is_autocommit()
        .then(|| connection.unchecked_transaction())
        .transpose()?;
    let reader = transaction.as_deref().unwrap_or(connection);
    reader.execute(
        "UPDATE app_wakes SET next_attempt_at_ms=MAX(due_at_ms, ?1)
         WHERE next_attempt_at_ms>MAX(due_at_ms, ?1)
           AND EXISTS(SELECT 1 FROM app_wake_clock WHERE last_poll_at_ms>?1)",
        [now_ms as i64],
    )?;
    reader.execute(
        "UPDATE app_wake_clock SET last_poll_at_ms=?1 WHERE singleton=1",
        [now_ms as i64],
    )?;
    let mut statement = reader.prepare(
        "SELECT owner_id,installation_id,wake_id,due_at_ms,revision,attempts,counts_as_use FROM app_wakes
         WHERE next_attempt_at_ms<=?1 ORDER BY due_at_ms, installation_id, wake_id LIMIT ?2",
    )?;
    let rows = statement.query_map(params![now_ms as i64, limit as i64], |row| {
        Ok(DueWake {
            owner_id: row.get(0)?,
            installation_id: row.get(1)?,
            wake: Wake {
                id: row.get(2)?,
                due_at_ms: row.get::<_, i64>(3)?.max(0) as u64,
                revision: row.get(4)?,
            },
            attempts: row.get::<_, i64>(5)?.max(0) as u32,
            counts_as_use: row.get(6)?,
        })
    })?;
    let due = rows.collect::<std::result::Result<_, _>>()?;
    drop(statement);
    if let Some(transaction) = transaction {
        transaction.commit()?;
    }
    Ok(due)
}

/// Remove a delivered wake. A wake replaced since delivery began keeps its
/// newer due time and revision.
pub fn complete_wake(connection: &Connection, delivered: &DueWake) -> Result<()> {
    remove_delivered_wake(connection, delivered).map(|_| ())
}

fn remove_delivered_wake(connection: &Connection, delivered: &DueWake) -> Result<usize> {
    Ok(connection.execute(
        "DELETE FROM app_wakes WHERE installation_id=?1 AND wake_id=?2 AND revision=?3 AND due_at_ms=?4",
        params![
            delivered.installation_id,
            delivered.wake.id,
            delivered.wake.revision,
            delivered.wake.due_at_ms as i64
        ],
    )?)
}

/// Wait for an on-demand start without consuming a delivery attempt.
pub fn postpone_wake(connection: &Connection, waiting: &DueWake, until_ms: u64) -> Result<()> {
    connection.execute(
        "UPDATE app_wakes SET next_attempt_at_ms=?5
         WHERE installation_id=?1 AND wake_id=?2 AND revision=?3 AND due_at_ms=?4",
        params![
            waiting.installation_id,
            waiting.wake.id,
            waiting.wake.revision,
            waiting.wake.due_at_ms as i64,
            until_ms.min(MAX_REVISION) as i64
        ],
    )?;
    Ok(())
}

/// What changed when a failed delivery was settled against its original wake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeFailureOutcome {
    Retried,
    Dropped,
    /// The App cancelled or replaced this delivery's wake while it ran.
    Obsolete,
}

/// Record a failed delivery with bounded exponential backoff. After the last
/// attempt the wake is dropped; a cancelled or replaced wake is left untouched.
pub fn defer_wake(
    connection: &Connection,
    delivered: &DueWake,
    now_ms: u64,
) -> Result<WakeFailureOutcome> {
    let attempts = delivered.attempts + 1;
    if attempts >= MAX_ATTEMPTS {
        return Ok(if remove_delivered_wake(connection, delivered)? == 0 {
            WakeFailureOutcome::Obsolete
        } else {
            WakeFailureOutcome::Dropped
        });
    }
    let delay = RETRY_BASE_MS.saturating_mul(1 << attempts.min(10));
    let changed = connection.execute(
        "UPDATE app_wakes SET attempts=?5, next_attempt_at_ms=?6
         WHERE installation_id=?1 AND wake_id=?2 AND revision=?3 AND due_at_ms=?4",
        params![
            delivered.installation_id,
            delivered.wake.id,
            delivered.wake.revision,
            delivered.wake.due_at_ms as i64,
            attempts,
            now_ms.saturating_add(delay).min(MAX_REVISION) as i64
        ],
    )?;
    Ok(if changed == 0 {
        WakeFailureOutcome::Obsolete
    } else {
        WakeFailureOutcome::Retried
    })
}
