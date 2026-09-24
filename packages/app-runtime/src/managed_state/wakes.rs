//! Kernel-owned App wakes. An App registers a durable due time; the kernel
//! starts the worker when it falls due and delivers the wake at least once.
//! Wakes are installation data, not a workflow trigger or a keep-alive grant.
use super::*;
use rusqlite::params;
use serde::{Deserialize, Serialize};

pub const MAX_WAKES: usize = 256;
pub const MAX_WAKE_CHANGES: usize = 16;
const MAX_ATTEMPTS: u32 = 8;
const RETRY_BASE_MS: u64 = 5_000;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueWake {
    pub owner_id: String,
    pub installation_id: String,
    pub wake: Wake,
    pub attempts: u32,
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
           PRIMARY KEY(installation_id, wake_id)
         );
         CREATE INDEX IF NOT EXISTS app_wakes_due ON app_wakes(next_attempt_at_ms);",
    )?;
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

pub(super) fn apply(
    transaction: &Connection,
    scope: StateScope<'_>,
    changes: &[WakeChange],
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
                    "INSERT INTO app_wakes(owner_id,installation_id,wake_id,due_at_ms,revision,attempts,next_attempt_at_ms)
                     VALUES(?1,?2,?3,?4,?5,0,?4)
                     ON CONFLICT(installation_id,wake_id) DO UPDATE SET
                       due_at_ms=excluded.due_at_ms, revision=excluded.revision,
                       attempts=0, next_attempt_at_ms=excluded.due_at_ms",
                    params![scope.owner, scope.installation, wake.id, wake.due_at_ms as i64, wake.revision],
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
pub fn due_wakes(connection: &Connection, now_ms: u64, limit: usize) -> Result<Vec<DueWake>> {
    let mut statement = connection.prepare(
        "SELECT owner_id,installation_id,wake_id,due_at_ms,revision,attempts FROM app_wakes
         WHERE next_attempt_at_ms<=?1 ORDER BY next_attempt_at_ms, installation_id, wake_id LIMIT ?2",
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
        })
    })?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

/// Remove a delivered wake. A wake replaced since delivery began keeps its
/// newer due time and revision.
pub fn complete_wake(connection: &Connection, delivered: &DueWake) -> Result<()> {
    connection.execute(
        "DELETE FROM app_wakes WHERE installation_id=?1 AND wake_id=?2 AND revision=?3 AND due_at_ms=?4",
        params![
            delivered.installation_id,
            delivered.wake.id,
            delivered.wake.revision,
            delivered.wake.due_at_ms as i64
        ],
    )?;
    Ok(())
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

/// Record a failed delivery with bounded exponential backoff. After the last
/// attempt the wake is dropped; the App reconstructs schedules from its state.
pub fn defer_wake(connection: &Connection, delivered: &DueWake, now_ms: u64) -> Result<bool> {
    let attempts = delivered.attempts + 1;
    if attempts >= MAX_ATTEMPTS {
        complete_wake(connection, delivered)?;
        return Ok(false);
    }
    let delay = RETRY_BASE_MS.saturating_mul(1 << attempts.min(10));
    connection.execute(
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
    Ok(true)
}
