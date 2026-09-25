//! Kernel-owned human validation of an App's declared critical actions.
//! An operation binds owner, installation, generation, action and the exact
//! canonical parameters; a person approves or denies it through a trusted
//! kernel interaction, and an approval is consumed once at the effect boundary.
use super::{DurableKernelStateStore, DurableWriterRequest};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::sync::mpsc;

/// Undecided operations expire; an approval must be used within this window.
pub(crate) const PENDING_MS: u64 = 10 * 60 * 1000;
pub(crate) const APPROVAL_USE_MS: u64 = 10 * 60 * 1000;
/// Unfinished operations per installation.
const MAX_OPEN: i64 = 16;
/// Denied, expired and consumed operations are kept this long, then removed.
const FINISHED_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidationState {
    Pending,
    Approved,
    Denied,
    Expired,
    Consumed,
}
impl ValidationState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Consumed => "consumed",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "approved" => Self::Approved,
            "denied" => Self::Denied,
            "expired" => Self::Expired,
            "consumed" => Self::Consumed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidationOperation {
    pub(crate) operation_id: String,
    pub(crate) owner: String,
    pub(crate) installation: String,
    pub(crate) generation: u64,
    pub(crate) action: String,
    /// Canonical (sorted-key) JSON of the exact parameters.
    pub(crate) parameters: String,
    pub(crate) digest: String,
    pub(crate) state: ValidationState,
    pub(crate) expires_ms: u64,
}

/// Canonical JSON with sorted object keys, and its SHA-256 digest.
pub(crate) fn canonical(parameters: &serde_json::Value) -> (String, String) {
    fn sort(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                serde_json::Value::Object(
                    entries
                        .into_iter()
                        .map(|(k, v)| (k.clone(), sort(v)))
                        .collect(),
                )
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(sort).collect())
            }
            other => other.clone(),
        }
    }
    let text = sort(parameters).to_string();
    let digest = format!("sha256:{:x}", Sha256::digest(text.as_bytes()));
    (text, digest)
}

pub(crate) enum ValidationCommand {
    Create(ValidationOperation),
    Decide {
        operation_id: String,
        approved: bool,
        now_ms: u64,
    },
    Expire {
        now_ms: u64,
    },
    /// Single use: approved and unexpired for exactly this binding.
    Consume {
        owner: String,
        installation: String,
        generation: u64,
        action: String,
        operation_id: String,
        now_ms: u64,
    },
}

pub(super) struct ValidationRequest {
    command: ValidationCommand,
    response: mpsc::Sender<Result<Option<ValidationOperation>, &'static str>>,
}
impl std::fmt::Debug for ValidationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ValidationRequest(..)")
    }
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_validations (
            operation_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, installation_id TEXT NOT NULL,
            generation INTEGER NOT NULL, action TEXT NOT NULL, parameters_json TEXT NOT NULL,
            digest TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('pending','approved','denied','expired','consumed')),
            expires_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS app_validations_open ON app_validations(state, expires_ms);",
    )
}

const COLUMNS: &str =
    "operation_id, owner_id, installation_id, generation, action, parameters_json, digest, state, expires_ms";

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<ValidationOperation> {
    let state: String = row.get(7)?;
    Ok(ValidationOperation {
        operation_id: row.get(0)?,
        owner: row.get(1)?,
        installation: row.get(2)?,
        generation: row.get::<_, i64>(3)? as u64,
        action: row.get(4)?,
        parameters: row.get(5)?,
        digest: row.get(6)?,
        state: ValidationState::parse(&state).ok_or(rusqlite::Error::InvalidQuery)?,
        expires_ms: row.get::<_, i64>(8)? as u64,
    })
}

fn load(
    connection: &Connection,
    operation_id: &str,
) -> rusqlite::Result<Option<ValidationOperation>> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM app_validations WHERE operation_id=?1"),
            params![operation_id],
            decode,
        )
        .optional()
}

impl DurableKernelStateStore {
    pub(crate) fn app_validation(
        &self,
        command: ValidationCommand,
    ) -> Result<Option<ValidationOperation>, &'static str> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppValidation(Box::new(
                ValidationRequest { command, response },
            )))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        receiver.recv().map_err(|_| "STORAGE_UNAVAILABLE")?
    }

    /// Owner- and installation-scoped read for `validation.status`.
    pub(crate) fn app_validation_status(
        &self,
        owner: &str,
        installation: &str,
        operation_id: &str,
    ) -> Result<Option<ValidationOperation>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_validation_status")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        Ok(load(&connection, operation_id)
            .map_err(|_| "STORAGE_UNAVAILABLE")?
            .filter(|operation| operation.owner == owner && operation.installation == installation))
    }

    /// The oldest pending operation of each installation, oldest first: one
    /// App's backlog can never hide another App's (or owner's) request.
    pub(crate) fn pending_app_validations(
        &self,
        limit: usize,
    ) -> Result<Vec<ValidationOperation>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_validations_pending")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM (
                   SELECT *, row_number() OVER (
                     PARTITION BY owner_id, installation_id ORDER BY updated_ms, operation_id) AS rank
                   FROM app_validations WHERE state='pending')
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

pub(super) fn execute(connection: &mut Connection, request: ValidationRequest) {
    let result = apply(connection, request.command);
    let _ = request.response.send(result);
}

fn apply(
    connection: &mut Connection,
    command: ValidationCommand,
) -> Result<Option<ValidationOperation>, &'static str> {
    let storage = |_| "STORAGE_UNAVAILABLE";
    let transaction = connection.transaction().map_err(storage)?;
    let result = match command {
        ValidationCommand::Create(operation) => {
            let open: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM app_validations WHERE owner_id=?1 AND installation_id=?2
                     AND state IN ('pending','approved')",
                    params![operation.owner, operation.installation],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if open >= MAX_OPEN {
                return Err("LIMIT_EXCEEDED");
            }
            transaction
                .execute(
                    "INSERT INTO app_validations VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9)",
                    params![
                        operation.operation_id,
                        operation.owner,
                        operation.installation,
                        operation.generation as i64,
                        operation.action,
                        operation.parameters,
                        operation.digest,
                        operation.expires_ms as i64,
                        crate::session::unix_epoch_ms() as i64
                    ],
                )
                .map_err(storage)?;
            Some(operation)
        }
        ValidationCommand::Decide {
            operation_id,
            approved,
            now_ms,
        } => {
            let Some(operation) = load(&transaction, &operation_id).map_err(storage)? else {
                return Err("NOT_FOUND");
            };
            // Only a pending, unexpired operation takes a decision, once.
            if operation.state != ValidationState::Pending || operation.expires_ms <= now_ms {
                return Ok(Some(operation));
            }
            let (state, expires) = if approved {
                (ValidationState::Approved, now_ms + APPROVAL_USE_MS)
            } else {
                (ValidationState::Denied, operation.expires_ms)
            };
            transaction
                .execute(
                    "UPDATE app_validations SET state=?2, expires_ms=?3, updated_ms=?4 WHERE operation_id=?1",
                    params![operation_id, state.name(), expires as i64, now_ms as i64],
                )
                .map_err(storage)?;
            load(&transaction, &operation_id).map_err(storage)?
        }
        ValidationCommand::Expire { now_ms } => {
            transaction
                .execute(
                    "UPDATE app_validations SET state='expired', updated_ms=?1
                     WHERE state IN ('pending','approved') AND expires_ms<=?1",
                    params![now_ms as i64],
                )
                .map_err(storage)?;
            transaction
                .execute(
                    "DELETE FROM app_validations
                     WHERE state IN ('denied','expired','consumed') AND updated_ms<=?1",
                    params![now_ms.saturating_sub(FINISHED_RETENTION_MS) as i64],
                )
                .map_err(storage)?;
            None
        }
        ValidationCommand::Consume {
            owner,
            installation,
            generation,
            action,
            operation_id,
            now_ms,
        } => {
            let changed = transaction
                .execute(
                    "UPDATE app_validations SET state='consumed', updated_ms=?6
                     WHERE operation_id=?1 AND owner_id=?2 AND installation_id=?3 AND generation=?4
                       AND action=?5 AND state='approved' AND expires_ms>?6",
                    params![
                        operation_id,
                        owner,
                        installation,
                        generation as i64,
                        action,
                        now_ms as i64
                    ],
                )
                .map_err(storage)?;
            if changed != 1 {
                return Err("VALIDATION_REQUIRED");
            }
            load(&transaction, &operation_id).map_err(storage)?
        }
    };
    transaction.commit().map_err(storage)?;
    Ok(result)
}

#[cfg(test)]
mod tests;
