//! App file exports: an App offers one of its private files; only its owner
//! can take it, through a trusted kernel prompt, by saving it from a terminal.
//! The offered bytes are a copy made when the App asked. The owner may take it
//! again until the offer expires (a failed or cancelled local save is not the
//! end of it); then the bytes are dropped.
use super::{DurableKernelStateStore, DurableWriterRequest};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::mpsc;

/// An unanswered offer expires and its bytes are dropped.
pub(crate) const OFFER_MS: u64 = 10 * 60 * 1000;
/// Unanswered offers per installation.
const MAX_OPEN: i64 = 4;
const FINISHED_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileExport {
    pub(crate) operation_id: String,
    pub(crate) owner: String,
    pub(crate) installation: String,
    pub(crate) generation: u64,
    /// The file's final name component, as offered to the owner.
    pub(crate) name: String,
    pub(crate) size: u64,
    /// `pending`, `saved`, `declined` or `expired`.
    pub(crate) state: String,
    pub(crate) expires_ms: u64,
}

pub(crate) enum FileExportCommand {
    Create {
        export: FileExport,
        contents: Vec<u8>,
    },
    /// The owner takes the bytes (again, until the offer expires).
    Save {
        owner: String,
        operation_id: String,
        now_ms: u64,
    },
    Decline {
        operation_id: String,
        now_ms: u64,
    },
    Expire {
        now_ms: u64,
    },
}

pub(crate) enum FileExportReply {
    Export(FileExport),
    Saved { name: String, contents: Vec<u8> },
    Done,
}

pub(super) struct FileExportRequest {
    command: FileExportCommand,
    response: mpsc::Sender<Result<FileExportReply, &'static str>>,
}
impl std::fmt::Debug for FileExportRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileExportRequest(..)")
    }
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_file_exports (
            operation_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, installation_id TEXT NOT NULL,
            generation INTEGER NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL, contents BLOB,
            state TEXT NOT NULL CHECK(state IN ('pending','saved','declined','expired')),
            expires_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS app_file_exports_open ON app_file_exports(state, expires_ms);",
    )
}

const COLUMNS: &str =
    "operation_id, owner_id, installation_id, generation, name, size, state, expires_ms";

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileExport> {
    Ok(FileExport {
        operation_id: row.get(0)?,
        owner: row.get(1)?,
        installation: row.get(2)?,
        generation: row.get::<_, i64>(3)? as u64,
        name: row.get(4)?,
        size: row.get::<_, i64>(5)? as u64,
        state: row.get(6)?,
        expires_ms: row.get::<_, i64>(7)? as u64,
    })
}

/// The offer's generation is still the installation's active one.
const ACTIVE: &str = "SELECT 1 FROM app_installations i
    WHERE i.installation_id=app_file_exports.installation_id
      AND i.owner_id=app_file_exports.owner_id
      AND i.generation=app_file_exports.generation
      AND i.active_json IS NOT NULL";

fn offering_generation_active(
    connection: &Connection,
    export: &FileExport,
) -> rusqlite::Result<bool> {
    connection.query_row(
        &format!("SELECT EXISTS ({ACTIVE}) FROM app_file_exports WHERE operation_id=?1"),
        params![export.operation_id],
        |row| row.get(0),
    )
}

fn load(connection: &Connection, operation_id: &str) -> rusqlite::Result<Option<FileExport>> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM app_file_exports WHERE operation_id=?1"),
            params![operation_id],
            decode,
        )
        .optional()
}

impl DurableKernelStateStore {
    pub(crate) fn app_file_export(
        &self,
        command: FileExportCommand,
    ) -> Result<FileExportReply, &'static str> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppFileExport(Box::new(
                FileExportRequest { command, response },
            )))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        receiver.recv().map_err(|_| "STORAGE_UNAVAILABLE")?
    }

    /// The oldest pending offer of each installation, oldest first.
    pub(crate) fn pending_app_file_exports(
        &self,
        limit: usize,
    ) -> Result<Vec<FileExport>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_file_exports_pending")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM (
                   SELECT *, row_number() OVER (
                     PARTITION BY owner_id, installation_id ORDER BY updated_ms, operation_id) AS rank
                   FROM app_file_exports WHERE state='pending')
                 WHERE rank=1 ORDER BY updated_ms LIMIT ?1"
            ))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let rows = statement
            .query_map(params![limit as i64], decode)
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        rows.collect::<Result<_, _>>()
            .map_err(|_| "STORAGE_UNAVAILABLE")
    }
}

pub(super) fn execute(connection: &mut Connection, request: FileExportRequest) {
    let result = apply(connection, request.command);
    let _ = request.response.send(result);
}

fn apply(
    connection: &mut Connection,
    command: FileExportCommand,
) -> Result<FileExportReply, &'static str> {
    let storage = |_| "STORAGE_UNAVAILABLE";
    let transaction = connection.transaction().map_err(storage)?;
    let reply = match command {
        FileExportCommand::Create { export, contents } => {
            let open: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM app_file_exports WHERE owner_id=?1 AND installation_id=?2
                     AND state='pending'",
                    params![export.owner, export.installation],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if open >= MAX_OPEN {
                return Err("LIMIT_EXCEEDED");
            }
            transaction
                .execute(
                    "INSERT INTO app_file_exports VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9)",
                    params![
                        export.operation_id,
                        export.owner,
                        export.installation,
                        export.generation as i64,
                        export.name,
                        contents.len() as i64,
                        contents,
                        export.expires_ms as i64,
                        crate::session::unix_epoch_ms() as i64
                    ],
                )
                .map_err(storage)?;
            FileExportReply::Export(
                load(&transaction, &export.operation_id)
                    .map_err(storage)?
                    .ok_or("STORAGE_UNAVAILABLE")?,
            )
        }
        FileExportCommand::Save {
            owner,
            operation_id,
            now_ms,
        } => {
            let Some(export) = load(&transaction, &operation_id)
                .map_err(storage)?
                .filter(|export| export.owner == owner)
            else {
                return Err("NOT_FOUND");
            };
            if !matches!(export.state.as_str(), "pending" | "saved")
                || export.expires_ms <= now_ms
                || !offering_generation_active(&transaction, &export).map_err(storage)?
            {
                return Err("CONFLICT");
            }
            let contents: Vec<u8> = transaction
                .query_row(
                    "SELECT contents FROM app_file_exports WHERE operation_id=?1",
                    params![operation_id],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            transaction
                .execute(
                    "UPDATE app_file_exports SET state='saved', updated_ms=?2
                     WHERE operation_id=?1",
                    params![operation_id, now_ms as i64],
                )
                .map_err(storage)?;
            FileExportReply::Saved {
                name: export.name,
                contents,
            }
        }
        FileExportCommand::Decline {
            operation_id,
            now_ms,
        } => {
            transaction
                .execute(
                    "UPDATE app_file_exports SET state='declined', contents=NULL, updated_ms=?2
                     WHERE operation_id=?1 AND state='pending'",
                    params![operation_id, now_ms as i64],
                )
                .map_err(storage)?;
            FileExportReply::Done
        }
        FileExportCommand::Expire { now_ms } => {
            transaction
                .execute(
                    &format!(
                        "UPDATE app_file_exports SET state='expired', contents=NULL, updated_ms=?1
                     WHERE state='pending' AND (expires_ms<=?1 OR NOT EXISTS ({ACTIVE}))"
                    ),
                    params![now_ms as i64],
                )
                .map_err(storage)?;
            // A saved offer ends the same way: at expiry, or once the App
            // that offered it is updated or uninstalled.
            transaction
                .execute(
                    &format!(
                        "UPDATE app_file_exports SET state='expired', contents=NULL, updated_ms=?1
                         WHERE state='saved' AND (expires_ms<=?1 OR NOT EXISTS ({ACTIVE}))"
                    ),
                    params![now_ms as i64],
                )
                .map_err(storage)?;
            transaction
                .execute(
                    "DELETE FROM app_file_exports WHERE state IN ('saved','declined','expired')
                     AND updated_ms<=?1",
                    params![now_ms.saturating_sub(FINISHED_RETENTION_MS) as i64],
                )
                .map_err(storage)?;
            FileExportReply::Done
        }
    };
    transaction.commit().map_err(storage)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (std::path::PathBuf, DurableKernelStateStore) {
        let root = std::env::temp_dir().join(format!(
            "chariox-file-exports-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        rusqlite::Connection::open(store.path())
            .unwrap()
            .execute_batch(
                "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                 VALUES('docs','com.example.docs','alice',3,3,'{}')",
            )
            .unwrap();
        (root, store)
    }

    fn offer(store: &DurableKernelStateStore, id: &str, expires_ms: u64) {
        store
            .app_file_export(FileExportCommand::Create {
                export: FileExport {
                    operation_id: id.into(),
                    owner: "alice".into(),
                    installation: "docs".into(),
                    generation: 3,
                    name: "plan.md".into(),
                    size: 0,
                    state: "pending".into(),
                    expires_ms,
                },
                contents: b"# Plan".to_vec(),
            })
            .unwrap();
    }

    #[test]
    fn only_the_owner_saves_an_offer_until_it_ends_and_declined_or_expired_offers_release_nothing()
    {
        let (root, store) = store();
        offer(&store, "offer", 1_000);
        let save = |owner: &str, now_ms| {
            store.app_file_export(FileExportCommand::Save {
                owner: owner.into(),
                operation_id: "offer".into(),
                now_ms,
            })
        };
        assert!(matches!(save("mallory", 10), Err("NOT_FOUND")));
        let Ok(FileExportReply::Saved { name, contents }) = save("alice", 10) else {
            panic!("the owner saves the offer");
        };
        assert_eq!(
            (name.as_str(), contents.as_slice()),
            ("plan.md", b"# Plan".as_slice())
        );
        // A failed or cancelled local save can take it again until it expires.
        assert!(matches!(
            save("alice", 11),
            Ok(FileExportReply::Saved { .. })
        ));
        assert!(matches!(save("alice", 1_000), Err("CONFLICT")));

        offer(&store, "declined", 1_000);
        store
            .app_file_export(FileExportCommand::Decline {
                operation_id: "declined".into(),
                now_ms: 5,
            })
            .unwrap();
        offer(&store, "late", 100);
        store
            .app_file_export(FileExportCommand::Expire { now_ms: 100 })
            .unwrap();
        for id in ["declined", "late"] {
            assert!(matches!(
                store.app_file_export(FileExportCommand::Save {
                    owner: "alice".into(),
                    operation_id: id.into(),
                    now_ms: 50,
                }),
                Err("CONFLICT")
            ));
        }
        assert!(store.pending_app_file_exports(8).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_saved_offer_ends_when_the_app_updates() {
        let (root, store) = store();
        offer(&store, "offer", 1_000);
        let save = || {
            store.app_file_export(FileExportCommand::Save {
                owner: "alice".into(),
                operation_id: "offer".into(),
                now_ms: 10,
            })
        };
        assert!(matches!(save(), Ok(FileExportReply::Saved { .. })));
        let db = rusqlite::Connection::open(store.path()).unwrap();
        db.execute(
            "UPDATE app_installations SET generation=4, allocated_generation=4 WHERE installation_id='docs'",
            [],
        )
        .unwrap();
        assert!(matches!(save(), Err("CONFLICT")));
        store
            .app_file_export(FileExportCommand::Expire { now_ms: 20 })
            .unwrap();
        let (state, dropped): (String, bool) = db
            .query_row(
                "SELECT state, contents IS NULL FROM app_file_exports WHERE operation_id='offer'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((state.as_str(), dropped), ("expired", true));
        let _ = std::fs::remove_dir_all(root);
    }
}
