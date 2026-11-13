//! Durable owner-selected folder permissions. Publication and revocation are
//! ordered on the same writer as current installation/signer admission.
use super::{app_file_grants::PickState, DurableKernelStateStore, DurableWriterRequest};
use crate::runtime::app_operation_budget::AppOperationBudget;
use chariox_app_runtime::{
    app_outbox::EventCatalog,
    external_folder::{ExternalFolder, FolderAccess, FolderIdentity, FolderReplace},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{mpsc, Arc},
};

const PICK_MS: u64 = 10 * 60 * 1000;
const MAX_GRANTS: u64 = 32;
const CALLS_PER_MINUTE: u64 = 60;
const WRITE_QUOTA: u64 = 512 * 1024 * 1024;
type Result<T> = std::result::Result<T, &'static str>;

#[derive(Debug, Clone)]
pub(crate) struct FolderPick {
    pub operation_id: String,
    pub owner: String,
    pub installation: String,
    pub generation: u64,
    pub expires_ms: u64,
    pub state: PickState,
    pub grant_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FolderGrantSummary {
    pub grant_id: String,
    pub path: String,
    pub access: FolderAccess,
    pub written_bytes: u64,
}

pub(crate) enum FolderCommand {
    Pick {
        owner: String,
        catalog: Arc<EventCatalog>,
        now_ms: u64,
    },
    Grant {
        owner: String,
        catalog: Arc<EventCatalog>,
        operation_id: String,
        path: String,
        identity: FolderIdentity,
        access: FolderAccess,
        now_ms: u64,
    },
    Decline {
        owner: String,
        operation_id: String,
    },
    Revoke {
        owner: String,
        installation: String,
        grant_id: String,
    },
    Read {
        owner: String,
        catalog: Arc<EventCatalog>,
        grant_id: String,
        relative: String,
        list: bool,
        budget: AppOperationBudget,
        now_ms: u64,
    },
    Publish {
        prepared: PreparedFolderWrite,
    },
}
/// An admitted write spends its budget before staging. This proof cannot be
/// cloned, serialized or constructed outside this module; publication consumes it.
pub(crate) struct PreparedFolderWrite {
    reservation: FolderWriteReservation,
    staged: FolderReplace,
}
struct FolderWriteReservation {
    owner: String,
    catalog: Arc<EventCatalog>,
    grant_id: String,
    grant: Grant,
    bytes: u64,
    budget: AppOperationBudget,
}
pub(super) struct FolderReserveRequest {
    owner: String,
    catalog: Arc<EventCatalog>,
    grant_id: String,
    bytes: u64,
    budget: AppOperationBudget,
    response: mpsc::Sender<Result<FolderWriteReservation>>,
}
impl std::fmt::Debug for FolderReserveRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FolderReserveRequest(..)")
    }
}
pub(super) struct FolderRequest {
    command: FolderCommand,
    response: mpsc::Sender<Result<Value>>,
}
impl std::fmt::Debug for FolderRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FolderRequest(..)")
    }
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_folder_picks (
          operation_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, installation_id TEXT NOT NULL,
          generation INTEGER NOT NULL CHECK(generation>0), expires_ms INTEGER NOT NULL CHECK(expires_ms>=0),
          state TEXT NOT NULL CHECK(state IN ('pending','granted','declined','expired')),
          grant_id TEXT);
         CREATE INDEX IF NOT EXISTS app_folder_picks_pending ON app_folder_picks(state,expires_ms);
         CREATE TABLE IF NOT EXISTS app_folder_grants (
          grant_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, installation_id TEXT NOT NULL,
          path TEXT NOT NULL, identity_json TEXT NOT NULL, access TEXT NOT NULL CHECK(access IN ('read_only','read_write')),
          window_ms INTEGER NOT NULL CHECK(window_ms>=0), calls INTEGER NOT NULL CHECK(calls>=0), written_bytes INTEGER NOT NULL CHECK(written_bytes>=0));
         CREATE INDEX IF NOT EXISTS app_folder_grants_owner ON app_folder_grants(owner_id,installation_id);
         CREATE TRIGGER IF NOT EXISTS app_folder_permission_change
         AFTER UPDATE OF active_json ON app_installations
         WHEN OLD.active_json IS NOT NULL AND (NEW.active_json IS NULL OR
           json_extract(OLD.active_json,'$.release.capabilities_digest') IS NOT
           json_extract(NEW.active_json,'$.release.capabilities_digest'))
         BEGIN
           DELETE FROM app_folder_grants WHERE installation_id=NEW.installation_id;
           UPDATE app_folder_picks SET state='expired' WHERE installation_id=NEW.installation_id;
         END;",
    )
}

impl DurableKernelStateStore {
    pub(crate) fn app_folder_command(&self, command: FolderCommand) -> Result<Value> {
        let uncertain = matches!(&command, FolderCommand::Publish { .. });
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppFolder(Box::new(FolderRequest {
                command,
                response,
            })))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        crate::durable_state::await_writer_response(receiver).map_err(|_| {
            if uncertain {
                "APP_FOLDER_OUTCOME_UNCERTAIN"
            } else {
                "STORAGE_UNAVAILABLE"
            }
        })?
    }

    pub(crate) fn app_folder_pick(
        &self,
        owner: &str,
        installation: &str,
        operation: &str,
    ) -> Result<Option<FolderPick>> {
        Ok(self
            .owned_app_folder_pick(owner, operation)?
            .filter(|pick| pick.installation == installation))
    }

    pub(crate) fn owned_app_folder_pick(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<FolderPick>> {
        let connection = self
            .lock_connection("durable_state.app_folder_pick")
            .map_err(storage)?;
        Ok(load_pick(&connection, operation)?.filter(|pick| pick.owner == owner))
    }

    pub(crate) fn pending_app_folder_picks(&self, now_ms: u64) -> Result<Vec<FolderPick>> {
        let connection = self
            .lock_connection("durable_state.app_folder_picks")
            .map_err(storage)?;
        let mut statement = connection.prepare("SELECT operation_id FROM app_folder_picks WHERE state='pending' AND expires_ms>?1 ORDER BY expires_ms LIMIT 64").map_err(storage)?;
        let ids = statement
            .query_map([now_ms as i64], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage)?;
        ids.iter()
            .filter_map(|id| load_pick(&connection, id).transpose())
            .collect()
    }

    pub(crate) fn list_app_folder_grants(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Vec<FolderGrantSummary>> {
        let connection = self
            .lock_connection("durable_state.app_folder_list")
            .map_err(storage)?;
        let mut statement = connection.prepare("SELECT grant_id,path,access,written_bytes FROM app_folder_grants WHERE owner_id=?1 AND installation_id=?2 ORDER BY grant_id LIMIT 32").map_err(storage)?;
        let rows = statement
            .query_map(params![owner, installation], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    number(row, 3)?,
                ))
            })
            .map_err(storage)?;
        rows.map(|row| {
            let (grant_id, path, access, written_bytes) = row.map_err(storage)?;
            Ok(FolderGrantSummary {
                grant_id,
                path,
                access: access_from(&access)?,
                written_bytes,
            })
        })
        .collect()
    }

    /// Reserve the rate and write budget before touching disk. Cancellation,
    /// disk failure and uncertain publication never refund that accepted I/O.
    pub(crate) fn prepare_app_folder_write(
        &self,
        owner: String,
        catalog: Arc<EventCatalog>,
        grant_id: String,
        relative: &str,
        contents: &[u8],
        budget: AppOperationBudget,
    ) -> Result<Value> {
        let prepared =
            self.stage_app_folder_write(owner, catalog, grant_id, relative, contents, budget)?;
        self.app_folder_command(FolderCommand::Publish { prepared })
    }

    pub(crate) fn stage_app_folder_write(
        &self,
        owner: String,
        catalog: Arc<EventCatalog>,
        grant_id: String,
        relative: &str,
        contents: &[u8],
        budget: AppOperationBudget,
    ) -> Result<PreparedFolderWrite> {
        if contents.len() > chariox_app_runtime::external_folder::MAX_FILE_BYTES {
            return Err("LIMIT_EXCEEDED");
        }
        budget.check().map_err(|_| "CANCELLED")?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppFolderReserve(Box::new(
                FolderReserveRequest {
                    owner,
                    catalog,
                    grant_id,
                    bytes: contents.len() as u64,
                    budget,
                    response,
                },
            )))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let reservation = crate::durable_state::await_writer_response(receiver)
            .map_err(|_| "STORAGE_UNAVAILABLE")??;
        reservation.budget.check().map_err(|_| "CANCELLED")?;
        let folder = ExternalFolder::reopen(
            Path::new(&reservation.grant.path),
            reservation.grant.identity,
        )
        .map_err(folder_error)?;
        let staged = folder
            .prepare_replace(relative, contents)
            .map_err(folder_error)?;
        Ok(PreparedFolderWrite {
            reservation,
            staged,
        })
    }
}

struct Grant {
    path: String,
    identity: FolderIdentity,
    access: FolderAccess,
}
fn load_grant(connection: &Connection, owner: &str, installation: &str, id: &str) -> Result<Grant> {
    let row = connection.query_row("SELECT path,identity_json,access FROM app_folder_grants WHERE grant_id=?1 AND owner_id=?2 AND installation_id=?3",params![id,owner,installation],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).optional().map_err(storage)?.ok_or("NOT_FOUND")?;
    Ok(Grant {
        path: row.0,
        identity: serde_json::from_str(&row.1).map_err(storage)?,
        access: access_from(&row.2)?,
    })
}
fn load_pick(connection: &Connection, operation: &str) -> Result<Option<FolderPick>> {
    connection.query_row("SELECT operation_id,owner_id,installation_id,generation,expires_ms,state,grant_id FROM app_folder_picks WHERE operation_id=?1",[operation],|row|Ok(FolderPick{operation_id:row.get(0)?,owner:row.get(1)?,installation:row.get(2)?,generation:number(row,3)?,expires_ms:number(row,4)?,state:PickState::parse(&row.get::<_,String>(5)?).ok_or(rusqlite::Error::InvalidQuery)?,grant_id:row.get(6)?})).optional().map_err(storage)
}
fn number(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}
fn storage(_: impl std::fmt::Debug) -> &'static str {
    "STORAGE_UNAVAILABLE"
}
fn access_name(access: FolderAccess) -> &'static str {
    match access {
        FolderAccess::ReadOnly => "read_only",
        FolderAccess::ReadWrite => "read_write",
    }
}
fn access_from(access: &str) -> Result<FolderAccess> {
    match access {
        "read_only" => Ok(FolderAccess::ReadOnly),
        "read_write" => Ok(FolderAccess::ReadWrite),
        _ => Err("STORAGE_UNAVAILABLE"),
    }
}
fn folder_error(error: chariox_app_runtime::external_folder::FolderError) -> &'static str {
    use chariox_app_runtime::external_folder::FolderError::*;
    match error {
        Invalid => "INVALID_ARGUMENT",
        Identity => "FOLDER_CHANGED",
        Unsafe => "UNSAFE_ENTRY",
        Limit => "LIMIT_EXCEEDED",
        Io => "APP_FOLDER_UNAVAILABLE",
        OutcomeUncertain => "APP_FOLDER_OUTCOME_UNCERTAIN",
    }
}

pub(super) fn execute_reserve(connection: &mut Connection, request: FolderReserveRequest) {
    let FolderReserveRequest {
        owner,
        catalog,
        grant_id,
        bytes,
        budget,
        response,
    } = request;
    let result = (|| {
        budget.check().map_err(|_| "CANCELLED")?;
        admit(
            connection,
            &owner,
            &catalog,
            &grant_id,
            FolderIo::Write(bytes),
            crate::session::unix_epoch_ms(),
        )?;
        let grant = load_grant(connection, &owner, catalog.installation_id(), &grant_id)?;
        Ok(FolderWriteReservation {
            owner,
            catalog,
            grant_id,
            grant,
            bytes,
            budget,
        })
    })();
    let _ = response.send(result);
}

pub(super) fn execute(connection: &mut Connection, request: FolderRequest) {
    let result = apply(connection, request.command);
    let _ = request.response.send(result);
}

fn apply(connection: &mut Connection, command: FolderCommand) -> Result<Value> {
    match command {
        FolderCommand::Pick {
            owner,
            catalog,
            now_ms,
        } => {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            catalog
                .app_catalog()
                .require_current(&transaction, &owner)
                .map_err(|_| "APP_STALE")?;
            transaction
                .execute(
                    "UPDATE app_folder_picks SET state='expired' WHERE expires_ms<=?1 AND state='pending'",
                    [now_ms as i64],
                )
                .map_err(storage)?;
            transaction
                .execute(
                    "DELETE FROM app_folder_picks WHERE expires_ms<=?1",
                    [now_ms.saturating_sub(24 * 60 * 60 * 1000) as i64],
                )
                .map_err(storage)?;
            let pending:u64=transaction.query_row("SELECT count(*) FROM app_folder_picks WHERE owner_id=?1 AND installation_id=?2 AND state='pending'",params![owner,catalog.installation_id()],|row|number(row,0)).map_err(storage)?;
            if pending >= 4 {
                return Err("LIMIT_EXCEEDED");
            }
            let pick = FolderPick {
                operation_id: format!("folder-pick-{:032x}", rand::random::<u128>()),
                owner,
                installation: catalog.installation_id().into(),
                generation: catalog.generation(),
                expires_ms: now_ms + PICK_MS,
                state: PickState::Pending,
                grant_id: None,
            };
            transaction
                .execute(
                    "INSERT INTO app_folder_picks VALUES(?1,?2,?3,?4,?5,'pending',NULL)",
                    params![
                        pick.operation_id,
                        pick.owner,
                        pick.installation,
                        pick.generation as i64,
                        pick.expires_ms as i64
                    ],
                )
                .map_err(storage)?;
            transaction.commit().map_err(storage)?;
            Ok(json!({"operationId":pick.operation_id,"state":"pending"}))
        }
        FolderCommand::Grant {
            owner,
            catalog,
            operation_id,
            path,
            identity,
            access,
            now_ms,
        } => {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let pick = load_pick(&transaction, &operation_id)?
                .filter(|pick| pick.owner == owner)
                .ok_or("NOT_FOUND")?;
            if pick.state != PickState::Pending || pick.expires_ms <= now_ms {
                return Err("CONFLICT");
            }
            if catalog.installation_id() != pick.installation
                || catalog.generation() != pick.generation
            {
                return Err("APP_STALE");
            }
            catalog
                .app_catalog()
                .require_current(&transaction, &owner)
                .map_err(|_| "APP_STALE")?;
            let count:u64=transaction.query_row("SELECT count(*) FROM app_folder_grants WHERE owner_id=?1 AND installation_id=?2",params![owner,pick.installation],|row|number(row,0)).map_err(storage)?;
            if count >= MAX_GRANTS {
                return Err("LIMIT_EXCEEDED");
            }
            let id = format!("folder-{:032x}", rand::random::<u128>());
            transaction
                .execute(
                    "INSERT INTO app_folder_grants VALUES(?1,?2,?3,?4,?5,?6,?7,0,0)",
                    params![
                        id,
                        owner,
                        pick.installation,
                        path,
                        serde_json::to_string(&identity).map_err(storage)?,
                        access_name(access),
                        now_ms as i64
                    ],
                )
                .map_err(storage)?;
            transaction
                .execute(
                    "UPDATE app_folder_picks SET state='granted',grant_id=?2 WHERE operation_id=?1",
                    params![operation_id, id],
                )
                .map_err(storage)?;
            transaction.commit().map_err(storage)?;
            Ok(json!({"operationId":operation_id,"installationId":pick.installation,"grantId":id}))
        }
        FolderCommand::Decline {
            owner,
            operation_id,
        } => {
            connection.execute("UPDATE app_folder_picks SET state='declined' WHERE operation_id=?1 AND owner_id=?2 AND state='pending'",params![operation_id,owner]).map_err(storage)?;
            Ok(json!({}))
        }
        FolderCommand::Revoke {
            owner,
            installation,
            grant_id,
        } => {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            if transaction.execute("DELETE FROM app_folder_grants WHERE grant_id=?1 AND owner_id=?2 AND installation_id=?3",params![grant_id,owner,installation]).map_err(storage)?!=1 { return Err("NOT_FOUND"); }
            transaction
                .execute(
                    "UPDATE app_folder_picks SET state='expired' WHERE grant_id=?1",
                    [&grant_id],
                )
                .map_err(storage)?;
            transaction.commit().map_err(storage)?;
            Ok(json!({"grantId":grant_id}))
        }
        FolderCommand::Read {
            owner,
            catalog,
            grant_id,
            relative,
            list,
            budget,
            now_ms,
        } => {
            budget.check().map_err(|_| "CANCELLED")?;
            admit(
                connection,
                &owner,
                &catalog,
                &grant_id,
                FolderIo::Read,
                now_ms,
            )?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            catalog
                .app_catalog()
                .require_current(&transaction, &owner)
                .map_err(|_| "APP_STALE")?;
            let grant = load_grant(&transaction, &owner, catalog.installation_id(), &grant_id)?;
            let folder = ExternalFolder::reopen(Path::new(&grant.path), grant.identity)
                .map_err(folder_error)?;
            budget.check().map_err(|_| "CANCELLED")?;
            if list {
                serde_json::to_value(folder.list(&relative).map_err(folder_error)?).map_err(storage)
            } else {
                use base64::Engine;
                let bytes = folder.read(&relative).map_err(folder_error)?;
                budget.check().map_err(|_| "CANCELLED")?;
                Ok(
                    json!({"contentsBase64":base64::engine::general_purpose::STANDARD.encode(bytes)}),
                )
            }
        }
        FolderCommand::Publish { prepared } => {
            let PreparedFolderWrite {
                reservation,
                staged,
            } = prepared;
            let FolderWriteReservation {
                owner,
                catalog,
                grant_id,
                grant: admitted,
                bytes,
                budget,
            } = reservation;
            budget.check().map_err(|_| "CANCELLED")?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            catalog
                .app_catalog()
                .require_current(&transaction, &owner)
                .map_err(|_| "APP_STALE")?;
            let grant = load_grant(&transaction, &owner, catalog.installation_id(), &grant_id)?;
            if grant.access != FolderAccess::ReadWrite {
                return Err("READ_ONLY");
            }
            if grant.path != admitted.path || grant.identity != admitted.identity {
                return Err("FOLDER_CHANGED");
            }
            ExternalFolder::reopen(Path::new(&grant.path), grant.identity).map_err(folder_error)?;
            budget.check().map_err(|_| "CANCELLED")?;
            staged.publish().map_err(folder_error)?;
            Ok(json!({"bytesWritten":bytes}))
        }
    }
}

enum FolderIo {
    Read,
    Write(u64),
}

fn admit(
    connection: &mut Connection,
    owner: &str,
    catalog: &EventCatalog,
    grant_id: &str,
    operation: FolderIo,
    now_ms: u64,
) -> Result<()> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    catalog
        .app_catalog()
        .require_current(&transaction, owner)
        .map_err(|_| "APP_STALE")?;
    let grant = load_grant(&transaction, owner, catalog.installation_id(), grant_id)?;
    let bytes = match operation {
        FolderIo::Read => 0,
        FolderIo::Write(bytes) => {
            if grant.access != FolderAccess::ReadWrite {
                return Err("READ_ONLY");
            }
            bytes
        }
    };
    let (start, calls, written): (u64, u64, u64) = transaction
        .query_row(
            "SELECT window_ms,calls,written_bytes FROM app_folder_grants WHERE grant_id=?1",
            [grant_id],
            |row| Ok((number(row, 0)?, number(row, 1)?, number(row, 2)?)),
        )
        .map_err(storage)?;
    let (start, calls) = if now_ms.saturating_sub(start) >= 60_000 {
        (now_ms, 0)
    } else {
        (start, calls)
    };
    if calls >= CALLS_PER_MINUTE {
        return Err("RATE_LIMITED");
    }
    if written.saturating_add(bytes) > WRITE_QUOTA {
        return Err("QUOTA_EXCEEDED");
    }
    transaction
        .execute(
            "UPDATE app_folder_grants SET window_ms=?2,calls=?3,written_bytes=?4 WHERE grant_id=?1",
            params![
                grant_id,
                start as i64,
                (calls + 1) as i64,
                (written + bytes) as i64
            ],
        )
        .map_err(storage)?;
    transaction.commit().map_err(storage)
}
