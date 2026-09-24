use super::*;
use crate::installation::{load_installation, ActiveGeneration, InstallationError};
use rusqlite::{params, OptionalExtension};

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_state_heads (
           installation_id TEXT PRIMARY KEY,
           revision INTEGER NOT NULL CHECK(revision >= 0),
           key_count INTEGER NOT NULL CHECK(key_count >= 0),
           payload_bytes INTEGER NOT NULL CHECK(payload_bytes >= 0)
         );
         CREATE TABLE IF NOT EXISTS app_state_values (
           installation_id TEXT NOT NULL,
           key TEXT NOT NULL,
           version INTEGER NOT NULL CHECK(version > 0),
           value_json TEXT NOT NULL,
           PRIMARY KEY(installation_id,key)
         );",
    )?;
    Ok(())
}

pub(super) fn active(transaction: &Connection, scope: StateScope<'_>) -> Result<ActiveGeneration> {
    let installation = load_installation(transaction, scope.installation)?;
    if installation.owner_id != scope.owner {
        return Err(InstallationError::NotFound.into());
    }
    if installation.generation != scope.generation {
        return Err(InstallationError::Conflict.into());
    }
    if installation.admission_paused {
        return Err(InstallationError::AdmissionPaused.into());
    }
    installation
        .active
        .ok_or_else(|| InstallationError::Inactive.into())
}

pub(super) fn read(
    transaction: &Connection,
    installation: &str,
    key: &str,
) -> Result<Option<StateRecord>> {
    let entry: Option<(i64, String)> = transaction
        .query_row(
            "SELECT version,value_json FROM app_state_values WHERE installation_id=?1 AND key=?2",
            params![installation, key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    entry
        .map(|(version, text)| {
            if !(1..=MAX_REVISION as i64).contains(&version) || text.len() > MAX_VALUE_BYTES {
                return Err(StateError::Corrupt);
            }
            let value = serde_json::from_str(&text).map_err(|_| StateError::Corrupt)?;
            Ok(StateRecord {
                value,
                version: version as u64,
            })
        })
        .transpose()
}

pub(super) fn apply(
    transaction: &Connection,
    scope: StateScope<'_>,
    changes: &StateChanges,
) -> Result<u64> {
    let current = active(transaction, scope)?;
    if current.release.schema_version != changes.schema_version {
        return Err(StateError::SchemaMismatch);
    }
    let head: Option<(i64, i64, i64)> = transaction
        .query_row(
            "SELECT revision,key_count,payload_bytes FROM app_state_heads WHERE installation_id=?1",
            [scope.installation],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (revision, mut keys, mut bytes) = head.unwrap_or((0, 0, 0));
    if !(0..=MAX_REVISION as i64).contains(&revision)
        || !(0..=MAX_KEYS as i64).contains(&keys)
        || !(0..=MAX_STATE_BYTES as i64).contains(&bytes)
    {
        return Err(StateError::Corrupt);
    }
    for check in &changes.checks {
        if read(transaction, scope.installation, &check.key)?.map(|record| record.version)
            != check.version
        {
            return Err(StateError::Conflict);
        }
    }
    if changes.writes.is_empty() {
        return Ok(revision as u64);
    }
    if revision == MAX_REVISION as i64 {
        return Err(StateError::Limit);
    }
    let next = revision + 1;
    // Calculate the final footprint first. A replacing transaction at the limit
    // may delete one key and add another in either request order.
    for (key, value) in &changes.writes {
        let old: Option<i64> = transaction.query_row(
            "SELECT length(CAST(value_json AS BLOB)) FROM app_state_values WHERE installation_id=?1 AND key=?2",
            params![scope.installation,key], |row| row.get(0),
        ).optional()?;
        if let Some(old_bytes) = old {
            keys -= 1;
            bytes -= key.len() as i64 + old_bytes;
        }
        if let Some(text) = value {
            keys += 1;
            bytes += (key.len() + text.len()) as i64;
        }
    }
    if keys < 0 || bytes < 0 {
        return Err(StateError::Corrupt);
    }
    if keys > MAX_KEYS as i64 || bytes > MAX_STATE_BYTES as i64 {
        return Err(StateError::Limit);
    }
    for (key, value) in &changes.writes {
        match value {
            Some(text) => {
                transaction.execute(
                "INSERT INTO app_state_values(installation_id,key,version,value_json) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(installation_id,key) DO UPDATE SET version=excluded.version,value_json=excluded.value_json",
                params![scope.installation,key,next,text],
            )?;
            }
            None => {
                transaction.execute(
                    "DELETE FROM app_state_values WHERE installation_id=?1 AND key=?2",
                    params![scope.installation, key],
                )?;
            }
        }
    }
    transaction.execute(
        "INSERT INTO app_state_heads(installation_id,revision,key_count,payload_bytes) VALUES(?1,?2,?3,?4)
         ON CONFLICT(installation_id) DO UPDATE SET revision=excluded.revision,key_count=excluded.key_count,payload_bytes=excluded.payload_bytes",
        params![scope.installation,next,keys,bytes],
    )?;
    Ok(next as u64)
}
