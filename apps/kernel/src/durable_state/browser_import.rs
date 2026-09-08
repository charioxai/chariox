//! Current import recovery metadata. Cookie values never belong in this table.
use rusqlite::{params, OptionalExtension, Transaction};

use super::{DurableKernelStateStore, DurableWriteOperation};
use crate::error::DaemonError;

pub(crate) enum ImportStateWrite {
    Begin {
        environment_id: String,
        request_id: String,
        user_id: String,
        room_id: String,
    },
    Recovered {
        environment_id: String,
        request_id: String,
    },
    Cleared {
        environment_id: String,
        request_id: String,
    },
}

impl std::fmt::Debug for ImportStateWrite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[browser import state transition]")
    }
}

impl DurableKernelStateStore {
    /// A recovered row still blocks admission until journal cleanup is complete.
    pub(crate) fn browser_import_pending(&self, environment_id: &str) -> Result<bool, DaemonError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| DaemonError::LocalTransport {
                operation: "browser_import.read",
                message: "state lock unavailable".into(),
            })?;
        connection
            .query_row(
                "SELECT 1 FROM durable_browser_import WHERE environment_id = ?1",
                params![environment_id],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(|_| DaemonError::LocalTransport {
                operation: "browser_import.read",
                message: "state read failed".into(),
            })
    }

    /// Call only after matching journal cleanup is acknowledged.
    pub(crate) fn clear_recovered_browser_import(
        &self,
        environment_id: &str,
        request_id: &str,
    ) -> Result<(), DaemonError> {
        self.writer
            .execute(DurableWriteOperation::BrowserImport(
                ImportStateWrite::Cleared {
                    environment_id: environment_id.into(),
                    request_id: request_id.into(),
                },
            ))
            .map(|_| ())
    }
    pub(crate) fn begin_browser_import_recovery(
        &self,
        environment_id: &str,
        request_id: &str,
        user_id: &str,
        room_id: &str,
    ) -> Result<(), DaemonError> {
        for value in [environment_id, request_id, user_id, room_id] {
            if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
                return Err(DaemonError::LocalTransport {
                    operation: "browser_import.begin",
                    message: "invalid import identity".into(),
                });
            }
        }
        self.writer
            .execute(DurableWriteOperation::BrowserImport(
                ImportStateWrite::Begin {
                    environment_id: environment_id.into(),
                    request_id: request_id.into(),
                    user_id: user_id.into(),
                    room_id: room_id.into(),
                },
            ))
            .map(|_| ())
    }

    /// Trusted kernel completion only, after verified recovery under exclusion.
    /// Retain the row until journal cleanup has been acknowledged separately.
    pub(crate) fn mark_browser_import_recovered(
        &self,
        environment_id: &str,
        request_id: &str,
    ) -> Result<(), DaemonError> {
        self.writer
            .execute(DurableWriteOperation::BrowserImport(
                ImportStateWrite::Recovered {
                    environment_id: environment_id.into(),
                    request_id: request_id.into(),
                },
            ))
            .map(|_| ())
    }
}

pub(super) fn apply(
    transaction: &Transaction<'_>,
    write: &ImportStateWrite,
) -> rusqlite::Result<u64> {
    let changed = match write {
        ImportStateWrite::Begin {
            environment_id,
            request_id,
            user_id,
            room_id,
        } => transaction.execute(
            "INSERT INTO durable_browser_import VALUES (?1, ?2, ?3, ?4, 1)",
            params![environment_id, request_id, user_id, room_id],
        )?,
        ImportStateWrite::Recovered {
            environment_id,
            request_id,
        } => transaction.execute(
            "UPDATE durable_browser_import SET recovery_required = 0
             WHERE environment_id = ?1 AND request_id = ?2 AND recovery_required = 1",
            params![environment_id, request_id],
        )?,
        ImportStateWrite::Cleared {
            environment_id,
            request_id,
        } => transaction.execute(
            "DELETE FROM durable_browser_import
             WHERE environment_id = ?1 AND request_id = ?2 AND recovery_required = 0",
            params![environment_id, request_id],
        )?,
    };
    if changed != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(changed as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledged_import_survives_reopen_and_rejects_stale_completion() {
        let directory = std::env::temp_dir().join(format!(
            "chariox-import-state-{}-{}",
            std::process::id(),
            super::super::rand_suffix()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("kernel.db");
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            assert!(!store.browser_import_pending("env").unwrap());
            store
                .begin_browser_import_recovery("env", "request", "user", "room")
                .unwrap();
            assert!(store
                .begin_browser_import_recovery("env", "other", "user", "room")
                .is_err());
        }
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            assert!(store.browser_import_pending("env").unwrap());
            assert!(store
                .clear_recovered_browser_import("env", "request")
                .is_err());
            assert!(store.mark_browser_import_recovered("env", "other").is_err());
            let read = rusqlite::Connection::open(&path).unwrap();
            let required: i64 = read.query_row(
                "SELECT recovery_required FROM durable_browser_import WHERE environment_id = 'env'",
                [], |row| row.get(0),
            ).unwrap();
            assert_eq!(required, 1);
            store
                .mark_browser_import_recovered("env", "request")
                .unwrap();
            assert!(store
                .begin_browser_import_recovery("env", "new", "user", "room")
                .is_err());
            assert!(store.browser_import_pending("env").unwrap());
            assert!(store
                .clear_recovered_browser_import("env", "other")
                .is_err());
            store
                .clear_recovered_browser_import("env", "request")
                .unwrap();
            assert!(!store.browser_import_pending("env").unwrap());
            store
                .begin_browser_import_recovery("env", "new", "user", "room")
                .unwrap();
            assert!(store
                .mark_browser_import_recovered("env", "request")
                .is_err());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
