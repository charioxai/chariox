//! MP-08 / MP-09 / MP-10 / MP-11 PR5-C3: which transfers rely on an event connection record an
//! import wrote. Every row commits in the same writer transaction as the event that changes it, so
//! the ledger survives a restart exactly as far as the records do.
use std::collections::BTreeSet;

use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};

use super::DurableKernelStateStore;
use crate::error::DaemonError;

/// The field an import adds to the `event_connection.upserted` event of a record it creates.
pub(crate) const IMPORTED_BY_TRANSFER_ID: &str = "imported_by_transfer_id";
const UPSERTED: &str = "event_connection.upserted";
const REMOVED: &str = "event_connection.removed";
const CHANGED: &str = "event_connection.import.changed";

/// The transfers that rely on one record. `created_by_import` holds until a transfer that relies
/// on the record completes; from then on the record is the owner's and no rollback removes it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EventConnectionImportClaims {
    pub(crate) created_by_import: bool,
    pub(crate) transfers: BTreeSet<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub(crate) enum EventConnectionImportChange {
    Joined {
        owner_user_id: String,
        connection_id: String,
        transfer_id: String,
    },
    Released {
        owner_user_id: String,
        connection_id: String,
        transfer_id: String,
    },
    Completed {
        owner_user_id: String,
        transfer_id: String,
    },
}

#[derive(Deserialize)]
struct ImportedRecord {
    owner_user_id: String,
    connection: ImportedConnection,
    imported_by_transfer_id: Option<String>,
}

#[derive(Deserialize)]
struct ImportedConnection {
    connection_id: String,
}

#[derive(Deserialize)]
struct RemovedRecord {
    owner_user_id: String,
    id: String,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS event_connection_imports (
            owner_user_id TEXT NOT NULL, connection_id TEXT NOT NULL, transfer_id TEXT NOT NULL,
            created_by_import INTEGER NOT NULL,
            PRIMARY KEY(owner_user_id, connection_id, transfer_id)
        );",
    )
}

pub(super) fn apply_event(tx: &Transaction<'_>, kind: &str, payload: &str) -> rusqlite::Result<()> {
    let decode_error = |_| rusqlite::Error::InvalidQuery;
    match kind {
        UPSERTED if payload.contains(IMPORTED_BY_TRANSFER_ID) => {
            let record: ImportedRecord = serde_json::from_str(payload).map_err(decode_error)?;
            if let Some(transfer_id) = record.imported_by_transfer_id {
                tx.execute(
                    "INSERT OR IGNORE INTO event_connection_imports
                    (owner_user_id, connection_id, transfer_id, created_by_import)
                    VALUES (?1, ?2, ?3, 1)",
                    params![
                        record.owner_user_id,
                        record.connection.connection_id,
                        transfer_id
                    ],
                )?;
            }
        }
        REMOVED => {
            let removed: RemovedRecord = serde_json::from_str(payload).map_err(decode_error)?;
            tx.execute(
                "DELETE FROM event_connection_imports WHERE owner_user_id=?1 AND connection_id=?2",
                params![removed.owner_user_id, removed.id],
            )?;
        }
        CHANGED => match serde_json::from_str(payload).map_err(decode_error)? {
            EventConnectionImportChange::Joined {
                owner_user_id,
                connection_id,
                transfer_id,
            } => {
                tx.execute(
                    "INSERT OR IGNORE INTO event_connection_imports
                    (owner_user_id, connection_id, transfer_id, created_by_import)
                    SELECT owner_user_id, connection_id, ?3, MAX(created_by_import)
                    FROM event_connection_imports WHERE owner_user_id=?1 AND connection_id=?2
                    GROUP BY owner_user_id, connection_id",
                    params![owner_user_id, connection_id, transfer_id],
                )?;
            }
            EventConnectionImportChange::Released {
                owner_user_id,
                connection_id,
                transfer_id,
            } => {
                tx.execute(
                    "DELETE FROM event_connection_imports
                    WHERE owner_user_id=?1 AND connection_id=?2 AND transfer_id=?3",
                    params![owner_user_id, connection_id, transfer_id],
                )?;
            }
            EventConnectionImportChange::Completed {
                owner_user_id,
                transfer_id,
            } => {
                tx.execute(
                    "UPDATE event_connection_imports SET created_by_import=0
                    WHERE owner_user_id=?1 AND connection_id IN (
                        SELECT connection_id FROM event_connection_imports
                        WHERE owner_user_id=?1 AND transfer_id=?2
                    )",
                    params![owner_user_id, transfer_id],
                )?;
                tx.execute(
                    "DELETE FROM event_connection_imports WHERE owner_user_id=?1 AND transfer_id=?2",
                    params![owner_user_id, transfer_id],
                )?;
            }
        },
        _ => {}
    }
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn append_event_connection_import_change(
        &self,
        change: EventConnectionImportChange,
    ) -> Result<(), DaemonError> {
        let payload = serde_json::to_value(change).map_err(|error| error_for(error.to_string()))?;
        self.append_event(CHANGED, None, payload)?;
        Ok(())
    }

    pub(crate) fn event_connection_import_claims(
        &self,
        owner_user_id: &str,
        connection_id: &str,
    ) -> Result<Option<EventConnectionImportClaims>, DaemonError> {
        let connection = self.lock_connection("event_connection_imports.read")?;
        let mut query = connection
            .prepare(
                "SELECT transfer_id, created_by_import FROM event_connection_imports
                WHERE owner_user_id=?1 AND connection_id=?2",
            )
            .map_err(|error| error_for(error.to_string()))?;
        let rows = query
            .query_map(params![owner_user_id, connection_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            })
            .map_err(|error| error_for(error.to_string()))?;
        let mut claims: Option<EventConnectionImportClaims> = None;
        for row in rows {
            let (transfer_id, created_by_import) =
                row.map_err(|error| error_for(error.to_string()))?;
            let entry = claims.get_or_insert_with(|| EventConnectionImportClaims {
                created_by_import: false,
                transfers: BTreeSet::new(),
            });
            entry.created_by_import |= created_by_import;
            entry.transfers.insert(transfer_id);
        }
        Ok(claims)
    }
}

fn error_for(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "event_connection_imports",
        message,
    }
}
