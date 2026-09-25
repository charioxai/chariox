//! Restricted migrations of structured state during a local update. Quiesce
//! snapshots the installation's values in the transaction that pauses its
//! admission, so no writer can interleave. The staged worker then migrates
//! them one step at a time; commit requires the release's target version and
//! drops the snapshot, and an abort (also on crash recovery) restores it.
use super::*;
use crate::installation::{load_installation, InstallationError};
use rusqlite::{params, OptionalExtension};

/// Part of the installation schema: the update journal owns these rows.
pub(crate) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_state_migrations (
           installation_id TEXT PRIMARY KEY,
           generation INTEGER NOT NULL CHECK(generation > 0),
           head_json TEXT,
           target INTEGER NOT NULL CHECK(target >= 0),
           migrated INTEGER NOT NULL CHECK(migrated >= 0 AND migrated <= target)
         );
         CREATE TABLE IF NOT EXISTS app_state_snapshot_values (
           installation_id TEXT NOT NULL,
           key TEXT NOT NULL,
           version INTEGER NOT NULL CHECK(version > 0),
           value_json TEXT NOT NULL,
           PRIMARY KEY(installation_id,key)
         );",
    )?;
    Ok(())
}

/// Called by quiesce for a staged generation whose schema differs from the
/// data's (`from`). A repeated quiesce keeps the first snapshot; any leftover
/// of an earlier stage is replaced.
pub(crate) fn begin_in(
    tx: &Connection,
    installation: &str,
    generation: u64,
    from: u32,
    target: u32,
) -> rusqlite::Result<()> {
    let started: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_state_migrations WHERE installation_id=?1 AND generation=?2)",
        params![installation, generation as i64],
        |row| row.get(0),
    )?;
    if started {
        return Ok(());
    }
    discard(tx, installation)?;
    let head: Option<(i64, i64, i64)> = tx
        .query_row(
            "SELECT revision,key_count,payload_bytes FROM app_state_heads WHERE installation_id=?1",
            [installation],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let head = head.map(|head| serde_json::json!([head.0, head.1, head.2]).to_string());
    tx.execute(
        "INSERT INTO app_state_migrations(installation_id,generation,head_json,target,migrated)
         VALUES(?1,?2,?3,?4,?5)",
        params![installation, generation as i64, head, target, from],
    )?;
    tx.execute(
        "INSERT INTO app_state_snapshot_values(installation_id,key,version,value_json)
         SELECT installation_id,key,version,value_json FROM app_state_values WHERE installation_id=?1",
        [installation],
    )?;
    Ok(())
}

/// The schema a migrating worker of this pending generation writes next, or
/// None when the installation has no migration for it.
pub(super) fn next_schema(
    tx: &Connection,
    installation: &str,
    generation: u64,
) -> Result<Option<u32>> {
    let row: Option<(u32, u32)> = tx
        .query_row(
            "SELECT target,migrated FROM app_state_migrations
             WHERE installation_id=?1 AND generation=?2",
            params![installation, generation as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(target, migrated)| (migrated < target).then_some(migrated + 1)))
}

/// A migrating worker reports a finished step. Steps are strictly in order.
pub(super) fn step(tx: &Connection, scope: StateScope<'_>, to: u32) -> Result<()> {
    if admit(tx, scope)? != Some(to) {
        return Err(StateError::SchemaMismatch);
    }
    tx.execute(
        "UPDATE app_state_migrations SET migrated=?3 WHERE installation_id=?1 AND generation=?2",
        params![scope.installation, scope.generation as i64, to],
    )?;
    Ok(())
}

/// Commit side: the data must have reached the release's schema.
pub(crate) fn finish_in(
    tx: &Connection,
    installation: &str,
    generation: u64,
) -> std::result::Result<(), InstallationError> {
    let row: Option<(u32, u32)> = tx
        .query_row(
            "SELECT target,migrated FROM app_state_migrations
             WHERE installation_id=?1 AND generation=?2",
            params![installation, generation as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((target, migrated)) if migrated != target => {
            Err(InstallationError::Invalid("data migration incomplete"))
        }
        Some(_) => Ok(discard(tx, installation)?),
        None => Ok(()),
    }
}

/// Abort side: put the snapshot back. The head revision only moves forward,
/// so no reader mistakes restored values for an older state.
pub(crate) fn restore_in(
    tx: &Connection,
    installation: &str,
    generation: u64,
) -> std::result::Result<(), InstallationError> {
    let head: Option<Option<String>> = tx
        .query_row(
            "SELECT head_json FROM app_state_migrations WHERE installation_id=?1 AND generation=?2",
            params![installation, generation as i64],
            |row| row.get(0),
        )
        .optional()?;
    let Some(head) = head else { return Ok(()) };
    let saved: (i64, i64, i64) = match head {
        Some(text) => serde_json::from_str(&text)?,
        None => (0, 0, 0),
    };
    let current: i64 = tx
        .query_row(
            "SELECT revision FROM app_state_heads WHERE installation_id=?1",
            [installation],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    tx.execute(
        "DELETE FROM app_state_values WHERE installation_id=?1",
        [installation],
    )?;
    tx.execute(
        "INSERT INTO app_state_values(installation_id,key,version,value_json)
         SELECT installation_id,key,version,value_json FROM app_state_snapshot_values
         WHERE installation_id=?1",
        [installation],
    )?;
    let revision = if current > saved.0 {
        current + 1
    } else {
        saved.0
    };
    tx.execute(
        "INSERT INTO app_state_heads(installation_id,revision,key_count,payload_bytes)
         VALUES(?1,?2,?3,?4)
         ON CONFLICT(installation_id) DO UPDATE SET
           revision=excluded.revision,key_count=excluded.key_count,payload_bytes=excluded.payload_bytes",
        params![installation, revision, saved.1, saved.2],
    )?;
    Ok(discard(tx, installation)?)
}

fn discard(tx: &Connection, installation: &str) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM app_state_migrations WHERE installation_id=?1",
        [installation],
    )?;
    tx.execute(
        "DELETE FROM app_state_snapshot_values WHERE installation_id=?1",
        [installation],
    )?;
    Ok(())
}

/// Admission of a migrating worker: its generation is the installation's
/// pending one, admission is paused for the update, and a migration exists.
pub(super) fn admit(tx: &Connection, scope: StateScope<'_>) -> Result<Option<u32>> {
    let installation = load_installation(tx, scope.installation)?;
    if installation.owner_id != scope.owner {
        return Err(InstallationError::NotFound.into());
    }
    if installation.pending_generation != Some(scope.generation) || !installation.admission_paused {
        return Ok(None);
    }
    next_schema(tx, scope.installation, scope.generation)
}
