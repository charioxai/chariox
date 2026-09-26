//! User-selected external file grants. An App asks to pick a file; only its
//! owner answers, through a trusted kernel prompt, by handing over the file's
//! bytes (or declining). Each grant is a copy the App may import once into its
//! private data before it expires. No host path ever reaches the App.
use super::{DurableKernelStateStore, DurableWriterRequest};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::sync::mpsc;

/// An unanswered pick expires; a granted file must be imported within this window.
pub(crate) const PICK_MS: u64 = 10 * 60 * 1000;
pub(crate) const GRANT_MS: u64 = 30 * 60 * 1000;
/// The same bound as one private-file replacement.
pub(crate) const MAX_FILE_BYTES: usize = 512 * 1024;
pub(crate) const MAX_FILES: usize = 8;
/// All files of one answer together. The answer is sent as base64 (a third
/// larger) and must fit the smallest transport: a relayed request's 768 KiB of
/// ciphertext, not only the 1 MiB local frame.
pub(crate) const MAX_TOTAL_BYTES: usize = 512 * 1024;
const MAX_NAME_BYTES: usize = 255;
/// Unfinished picks per installation.
const MAX_OPEN: i64 = 4;
const FINISHED_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickState {
    Pending,
    Granted,
    Declined,
    Expired,
}
impl PickState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Granted => "granted",
            Self::Declined => "declined",
            Self::Expired => "expired",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "granted" => Self::Granted,
            "declined" => Self::Declined,
            "expired" => Self::Expired,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilePick {
    pub(crate) operation_id: String,
    pub(crate) owner: String,
    pub(crate) installation: String,
    pub(crate) generation: u64,
    /// Accepted file name suffixes (for example `.md`); empty accepts any.
    pub(crate) accept: Vec<String>,
    pub(crate) multiple: bool,
    pub(crate) state: PickState,
    pub(crate) expires_ms: u64,
    /// Granted files, in the order the owner chose them.
    pub(crate) grants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GrantedFile {
    pub(crate) name: String,
    pub(crate) contents: Vec<u8>,
}

pub(crate) enum FileGrantCommand {
    Create(FilePick),
    /// The owner's answer: the chosen files' bytes.
    Grant {
        owner: String,
        operation_id: String,
        files: Vec<GrantedFile>,
        now_ms: u64,
    },
    Decline {
        operation_id: String,
        now_ms: u64,
    },
    Expire {
        now_ms: u64,
    },
    /// Takes an unexpired grant of this installation's current generation for
    /// one import: no concurrent import can take it too.
    Claim {
        owner: String,
        installation: String,
        generation: u64,
        grant_id: String,
        now_ms: u64,
    },
    /// The claimed import was published: the bytes are dropped.
    Imported {
        owner: String,
        installation: String,
        grant_id: String,
    },
    /// The claimed import failed before publishing: the grant can be used again.
    Release {
        owner: String,
        installation: String,
        grant_id: String,
    },
}

enum FileGrantReply {
    Pick(Option<FilePick>),
    Claimed(GrantedFile),
}

pub(super) struct FileGrantRequest {
    command: FileGrantCommand,
    response: mpsc::Sender<Result<FileGrantReply, &'static str>>,
}
impl std::fmt::Debug for FileGrantRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileGrantRequest(..)")
    }
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_file_picks (
            operation_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, installation_id TEXT NOT NULL,
            generation INTEGER NOT NULL, accept_json TEXT NOT NULL,
            multiple INTEGER NOT NULL CHECK(multiple IN (0,1)),
            state TEXT NOT NULL CHECK(state IN ('pending','granted','declined','expired')),
            expires_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS app_file_picks_open ON app_file_picks(state, expires_ms);
         CREATE TABLE IF NOT EXISTS app_file_grants (
            grant_id TEXT PRIMARY KEY, operation_id TEXT NOT NULL, position INTEGER NOT NULL,
            owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, name TEXT NOT NULL,
            size INTEGER NOT NULL, digest TEXT NOT NULL, contents BLOB,
            expires_ms INTEGER NOT NULL, imported INTEGER NOT NULL CHECK(imported IN (0,1)));
         CREATE INDEX IF NOT EXISTS app_file_grants_pick ON app_file_grants(operation_id, position);",
    )
}

const COLUMNS: &str =
    "operation_id, owner_id, installation_id, generation, accept_json, multiple, state, expires_ms";

fn load(connection: &Connection, operation_id: &str) -> rusqlite::Result<Option<FilePick>> {
    let pick = connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM app_file_picks WHERE operation_id=?1"),
            params![operation_id],
            |row| {
                let accept: String = row.get(4)?;
                let state: String = row.get(6)?;
                Ok(FilePick {
                    operation_id: row.get(0)?,
                    owner: row.get(1)?,
                    installation: row.get(2)?,
                    generation: row.get::<_, i64>(3)? as u64,
                    accept: serde_json::from_str(&accept)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    multiple: row.get(5)?,
                    state: PickState::parse(&state).ok_or(rusqlite::Error::InvalidQuery)?,
                    expires_ms: row.get::<_, i64>(7)? as u64,
                    grants: Vec::new(),
                })
            },
        )
        .optional()?;
    let Some(mut pick) = pick else {
        return Ok(None);
    };
    let mut statement = connection
        .prepare("SELECT grant_id FROM app_file_grants WHERE operation_id=?1 ORDER BY position")?;
    pick.grants = statement
        .query_map(params![operation_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(pick))
}

/// A file name the owner chose: its final path component only, so it shows
/// the choice without revealing where the file lived.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

fn accepted(pick: &FilePick, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    pick.accept.is_empty()
        || pick
            .accept
            .iter()
            .any(|suffix| lower.ends_with(&suffix.to_ascii_lowercase()))
}

impl DurableKernelStateStore {
    fn send_app_file_grant(
        &self,
        command: FileGrantCommand,
    ) -> Result<FileGrantReply, &'static str> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppFileGrant(Box::new(
                FileGrantRequest { command, response },
            )))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        receiver.recv().map_err(|_| "STORAGE_UNAVAILABLE")?
    }

    pub(crate) fn app_file_grant(
        &self,
        command: FileGrantCommand,
    ) -> Result<Option<FilePick>, &'static str> {
        match self.send_app_file_grant(command)? {
            FileGrantReply::Pick(pick) => Ok(pick),
            FileGrantReply::Claimed(_) => Err("STORAGE_UNAVAILABLE"),
        }
    }

    /// `FileGrantCommand::Claim`: the grant's bytes, now reserved for one import.
    pub(crate) fn claim_app_file_grant(
        &self,
        command: FileGrantCommand,
    ) -> Result<GrantedFile, &'static str> {
        match self.send_app_file_grant(command)? {
            FileGrantReply::Claimed(file) => Ok(file),
            FileGrantReply::Pick(_) => Err("STORAGE_UNAVAILABLE"),
        }
    }

    /// Owner- and installation-scoped read for `host.pick_file_status`.
    pub(crate) fn app_file_pick(
        &self,
        owner: &str,
        installation: &str,
        operation_id: &str,
    ) -> Result<Option<FilePick>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_file_pick")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        Ok(load(&connection, operation_id)
            .map_err(|_| "STORAGE_UNAVAILABLE")?
            .filter(|pick| pick.owner == owner && pick.installation == installation))
    }

    /// The oldest pending pick of each installation, oldest first.
    pub(crate) fn pending_app_file_picks(
        &self,
        limit: usize,
    ) -> Result<Vec<FilePick>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_file_picks_pending")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut statement = connection
            .prepare(
                "SELECT operation_id FROM (
                   SELECT operation_id, updated_ms, row_number() OVER (
                     PARTITION BY owner_id, installation_id ORDER BY updated_ms, operation_id) AS rank
                   FROM app_file_picks WHERE state='pending')
                 WHERE rank=1 ORDER BY updated_ms LIMIT ?1",
            )
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let ids = statement
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))
            .map_err(|_| "STORAGE_UNAVAILABLE")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        ids.iter()
            .filter_map(|id| load(&connection, id).transpose())
            .collect::<rusqlite::Result<_>>()
            .map_err(|_| "STORAGE_UNAVAILABLE")
    }
}

pub(super) fn execute(connection: &mut Connection, request: FileGrantRequest) {
    let result = apply(connection, request.command);
    let _ = request.response.send(result);
}

fn apply(
    connection: &mut Connection,
    command: FileGrantCommand,
) -> Result<FileGrantReply, &'static str> {
    let storage = |_| "STORAGE_UNAVAILABLE";
    let transaction = connection.transaction().map_err(storage)?;
    let result = match command {
        FileGrantCommand::Create(pick) => {
            let open: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM app_file_picks WHERE owner_id=?1 AND installation_id=?2
                     AND state='pending'",
                    params![pick.owner, pick.installation],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if open >= MAX_OPEN {
                return Err("LIMIT_EXCEEDED");
            }
            transaction
                .execute(
                    "INSERT INTO app_file_picks VALUES (?1,?2,?3,?4,?5,?6,'pending',?7,?8)",
                    params![
                        pick.operation_id,
                        pick.owner,
                        pick.installation,
                        pick.generation as i64,
                        serde_json::to_string(&pick.accept).map_err(|_| "INVALID_ARGUMENT")?,
                        pick.multiple,
                        pick.expires_ms as i64,
                        crate::session::unix_epoch_ms() as i64
                    ],
                )
                .map_err(storage)?;
            Some(pick)
        }
        FileGrantCommand::Grant {
            owner,
            operation_id,
            files,
            now_ms,
        } => {
            let Some(pick) = load(&transaction, &operation_id)
                .map_err(storage)?
                .filter(|pick| pick.owner == owner)
            else {
                return Err("NOT_FOUND");
            };
            if pick.state != PickState::Pending || pick.expires_ms <= now_ms {
                return Err("CONFLICT");
            }
            if files.is_empty()
                || files.len() > MAX_FILES
                || files.iter().map(|file| file.contents.len()).sum::<usize>() > MAX_TOTAL_BYTES
                || (!pick.multiple && files.len() > 1)
                || files.iter().any(|file| {
                    !valid_name(&file.name)
                        || !accepted(&pick, &file.name)
                        || file.contents.len() > MAX_FILE_BYTES
                })
            {
                return Err("INVALID_ARGUMENT");
            }
            for (position, file) in files.iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO app_file_grants VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0)",
                        params![
                            format!("grant-{:032x}", rand::random::<u128>()),
                            operation_id,
                            position as i64,
                            pick.owner,
                            pick.installation,
                            file.name,
                            file.contents.len() as i64,
                            format!("sha256:{:x}", Sha256::digest(&file.contents)),
                            file.contents,
                            (now_ms + GRANT_MS) as i64
                        ],
                    )
                    .map_err(storage)?;
            }
            transaction
                .execute(
                    "UPDATE app_file_picks SET state='granted', expires_ms=?2, updated_ms=?3
                     WHERE operation_id=?1",
                    params![operation_id, (now_ms + GRANT_MS) as i64, now_ms as i64],
                )
                .map_err(storage)?;
            load(&transaction, &operation_id).map_err(storage)?
        }
        FileGrantCommand::Decline {
            operation_id,
            now_ms,
        } => {
            transaction
                .execute(
                    "UPDATE app_file_picks SET state='declined', updated_ms=?2
                     WHERE operation_id=?1 AND state='pending'",
                    params![operation_id, now_ms as i64],
                )
                .map_err(storage)?;
            load(&transaction, &operation_id).map_err(storage)?
        }
        FileGrantCommand::Expire { now_ms } => {
            transaction
                .execute(
                    "UPDATE app_file_picks SET state='expired', updated_ms=?1
                     WHERE state IN ('pending','granted') AND expires_ms<=?1",
                    params![now_ms as i64],
                )
                .map_err(storage)?;
            // An uninstalled or updated App cannot receive or import files.
            transaction
                .execute(
                    "UPDATE app_file_picks SET state='expired', updated_ms=?1
                     WHERE state IN ('pending','granted') AND NOT EXISTS (
                       SELECT 1 FROM app_installations i
                       WHERE i.installation_id=app_file_picks.installation_id
                         AND i.owner_id=app_file_picks.owner_id
                         AND i.generation=app_file_picks.generation
                         AND i.active_json IS NOT NULL)",
                    params![now_ms as i64],
                )
                .map_err(storage)?;
            // Expired or unusable bytes are dropped at once.
            transaction
                .execute(
                    "UPDATE app_file_grants SET contents=NULL WHERE contents IS NOT NULL AND
                     (expires_ms<=?1 OR operation_id IN
                       (SELECT operation_id FROM app_file_picks WHERE state='expired'))",
                    params![now_ms as i64],
                )
                .map_err(storage)?;
            let before = now_ms.saturating_sub(FINISHED_RETENTION_MS) as i64;
            transaction
                .execute(
                    "DELETE FROM app_file_grants WHERE operation_id IN (SELECT operation_id
                     FROM app_file_picks WHERE state IN ('declined','expired') AND updated_ms<=?1)",
                    params![before],
                )
                .map_err(storage)?;
            transaction
                .execute(
                    "DELETE FROM app_file_picks WHERE state IN ('declined','expired') AND updated_ms<=?1",
                    params![before],
                )
                .map_err(storage)?;
            None
        }
        FileGrantCommand::Claim {
            owner,
            installation,
            generation,
            grant_id,
            now_ms,
        } => {
            let file = transaction
                .query_row(
                    "SELECT g.name, g.contents FROM app_file_grants g
                     JOIN app_file_picks p ON p.operation_id=g.operation_id
                     WHERE g.grant_id=?1 AND g.owner_id=?2 AND g.installation_id=?3
                       AND g.imported=0 AND g.contents IS NOT NULL AND g.expires_ms>?4
                       AND p.state='granted' AND p.generation=?5",
                    params![
                        grant_id,
                        owner,
                        installation,
                        now_ms as i64,
                        generation as i64
                    ],
                    |row| {
                        Ok(GrantedFile {
                            name: row.get(0)?,
                            contents: row.get(1)?,
                        })
                    },
                )
                .optional()
                .map_err(storage)?
                .ok_or("NOT_FOUND")?;
            transaction
                .execute(
                    "UPDATE app_file_grants SET imported=1 WHERE grant_id=?1",
                    params![grant_id],
                )
                .map_err(storage)?;
            transaction.commit().map_err(storage)?;
            return Ok(FileGrantReply::Claimed(file));
        }
        FileGrantCommand::Imported {
            owner,
            installation,
            grant_id,
        } => {
            transaction
                .execute(
                    "UPDATE app_file_grants SET contents=NULL WHERE grant_id=?1
                     AND owner_id=?2 AND installation_id=?3 AND imported=1",
                    params![grant_id, owner, installation],
                )
                .map_err(storage)?;
            None
        }
        FileGrantCommand::Release {
            owner,
            installation,
            grant_id,
        } => {
            transaction
                .execute(
                    "UPDATE app_file_grants SET imported=0 WHERE grant_id=?1
                     AND owner_id=?2 AND installation_id=?3 AND imported=1 AND contents IS NOT NULL",
                    params![grant_id, owner, installation],
                )
                .map_err(storage)?;
            None
        }
    };
    transaction.commit().map_err(storage)?;
    Ok(FileGrantReply::Pick(result))
}

#[cfg(test)]
mod tests;
