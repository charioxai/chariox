//! Saved state replacement on the existing FULL-synchronous writer. Only
//! private values and pending wakes roll back; authority and delivery tables
//! are never decoded from a snapshot or mutated by this operation.
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::{local::AppRequestErrorCode as Error, runtime::app_snapshot_restore::Result};
use chariox_app_runtime::{app_outbox::EventCatalog, managed_state::*};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{mpsc, Arc},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    values: Vec<Entry>,
    head: Option<Head>,
    migration: Option<Value>,
    wakes: Vec<SavedWake>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    key: String,
    version: u64,
    value_json: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedWake {
    id: String,
    due_at_ms: u64,
    revision: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Head {
    revision: u64,
    key_count: usize,
    payload_bytes: usize,
}
pub(super) struct RestoreRequest {
    owner: String,
    catalog: Arc<EventCatalog>,
    restore_id: String,
    state: Value,
    response: mpsc::Sender<Result<()>>,
    #[cfg(test)]
    fault: Option<RestoreFault>,
}
impl std::fmt::Debug for RestoreRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestoreRequest")
            .field("installation", &self.catalog.installation_id())
            .finish_non_exhaustive()
    }
}
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum RestoreFault {
    BeforeCommit,
    AfterCommit,
}
fn decode(state: &Value) -> Result<State> {
    let state: State = serde_json::from_value(state.clone()).map_err(|_| Error::Conflict)?;
    if state.migration.is_some() || state.values.len() > MAX_KEYS || state.wakes.len() > MAX_WAKES {
        return Err(Error::Conflict);
    }
    let mut keys = BTreeSet::new();
    let mut bytes = 0;
    for entry in &state.values {
        if entry.key.is_empty()
            || entry.key.len() > MAX_KEY_BYTES
            || entry.key.chars().any(char::is_control)
            || !keys.insert(&entry.key)
            || entry.version == 0
            || entry.version > MAX_REVISION
            || entry.value_json.len() > MAX_VALUE_BYTES
            || serde_json::from_str::<Value>(&entry.value_json).is_err()
        {
            return Err(Error::Conflict);
        }
        bytes += entry.key.len() + entry.value_json.len();
    }
    if bytes > MAX_STATE_BYTES {
        return Err(Error::LimitExceeded);
    }
    match &state.head {
        Some(head)
            if head.revision <= MAX_REVISION
                && head.key_count == state.values.len()
                && head.payload_bytes == bytes
                && state
                    .values
                    .iter()
                    .all(|entry| entry.version <= head.revision) => {}
        None if state.values.is_empty() => (),
        _ => return Err(Error::Conflict),
    }
    let mut wakes = BTreeSet::new();
    for wake in &state.wakes {
        if wake.id.is_empty()
            || wake.id.len() > MAX_KEY_BYTES
            || wake.id.chars().any(char::is_control)
            || wake.revision.is_empty()
            || wake.revision.len() > MAX_KEY_BYTES
            || wake.revision.chars().any(char::is_control)
            || wake.due_at_ms > MAX_REVISION
            || !wakes.insert(&wake.id)
        {
            return Err(Error::Conflict);
        }
    }
    Ok(state)
}
pub(crate) fn validate_state(state: &Value) -> Result<()> {
    decode(state).map(|_| ())
}

impl DurableKernelStateStore {
    pub(crate) fn commit_app_snapshot_restore(
        &self,
        owner: &str,
        catalog: Arc<EventCatalog>,
        restore_id: String,
        state: Value,
        #[cfg(test)] fault: Option<RestoreFault>,
    ) -> Result<()> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppRestore(Box::new(RestoreRequest {
                owner: owner.into(),
                catalog,
                restore_id,
                state,
                response,
                #[cfg(test)]
                fault,
            })))
            .map_err(|_| Error::StorageUnavailable)?;
        receiver.recv().map_err(|_| Error::StorageUnavailable)?
    }
    pub(crate) fn app_snapshot_restore_committed(
        &self,
        owner: &str,
        installation: &str,
        restore_id: &str,
    ) -> Result<bool> {
        self.require_writer_healthy()
            .map_err(|_| Error::StorageUnavailable)?;
        let connection = self
            .lock_connection("durable_state.restore_receipt")
            .map_err(|_| Error::StorageUnavailable)?;
        connection.query_row("SELECT EXISTS(SELECT 1 FROM app_restore_receipts WHERE owner_id=?1 AND installation_id=?2 AND restore_id=?3)",
            params![owner, installation, restore_id], |row| row.get(0)).map_err(|_| Error::StorageUnavailable)
    }
}
pub(super) fn execute(
    connection: &mut Connection,
    request: RestoreRequest,
    fatal: &std::sync::atomic::AtomicBool,
) -> bool {
    let (result, uncertain) = apply(connection, &request);
    // The writer must be fenced before publishing an uncertain reply.
    if uncertain {
        fatal.store(true, std::sync::atomic::Ordering::Release);
    }
    let _ = request.response.send(result);
    uncertain
}
fn apply(connection: &mut Connection, request: &RestoreRequest) -> (Result<()>, bool) {
    match apply_effect(connection, request) {
        Err(error) => (Err(error), false),
        Ok(tx) => {
            #[cfg(test)]
            if matches!(request.fault, Some(RestoreFault::BeforeCommit)) {
                drop(tx);
                return (Err(Error::StorageUnavailable), true);
            }
            match tx.commit() {
                Ok(()) => {
                    #[cfg(test)]
                    if matches!(request.fault, Some(RestoreFault::AfterCommit)) {
                        return (Err(Error::StorageUnavailable), true);
                    }
                    (Ok(()), false)
                }
                // SQLite's atomic restart recovery decides the journal. No read of
                // this connection can resolve durability after an uncertain COMMIT.
                Err(_) => (Err(Error::StorageUnavailable), true),
            }
        }
    }
}

fn apply_effect<'a>(
    connection: &'a mut Connection,
    request: &RestoreRequest,
) -> Result<rusqlite::Transaction<'a>> {
    let state = decode(&request.state)?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::StorageUnavailable)?;
    request
        .catalog
        .app_catalog()
        .require_current(&tx, &request.owner)
        .map_err(|_| Error::Conflict)?;
    let installation = request.catalog.installation_id();
    // Keep versions monotonic even though values are restored from the past.
    let current: Option<i64> = tx
        .query_row(
            "SELECT revision FROM app_state_heads WHERE installation_id=?1",
            [installation],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::StorageUnavailable)?;
    let current = current.unwrap_or(0);
    if current < 0 || current > MAX_REVISION as i64 {
        return Err(Error::Conflict);
    }
    let revision = current
        .max(state.head.as_ref().map_or(0, |h| h.revision as i64))
        .checked_add(1)
        .filter(|r| *r <= MAX_REVISION as i64)
        .ok_or(Error::LimitExceeded)?;
    tx.execute(
        "DELETE FROM app_state_values WHERE installation_id=?1",
        [installation],
    )
    .map_err(|_| Error::StorageUnavailable)?;
    for entry in &state.values {
        tx.execute("INSERT INTO app_state_values(installation_id,key,version,value_json) VALUES(?1,?2,?3,?4)",
                params![installation, entry.key, revision, entry.value_json]).map_err(|_| Error::StorageUnavailable)?;
    }
    let bytes: usize = state
        .values
        .iter()
        .map(|e| e.key.len() + e.value_json.len())
        .sum();
    tx.execute("INSERT INTO app_state_heads(installation_id,revision,key_count,payload_bytes) VALUES(?1,?2,?3,?4) ON CONFLICT(installation_id) DO UPDATE SET revision=excluded.revision,key_count=excluded.key_count,payload_bytes=excluded.payload_bytes",
            params![installation, revision, state.values.len() as i64, bytes as i64]).map_err(|_| Error::StorageUnavailable)?;
    tx.execute(
        "DELETE FROM app_wakes WHERE installation_id=?1 AND owner_id=?2",
        params![installation, request.owner],
    )
    .map_err(|_| Error::StorageUnavailable)?;
    for wake in &state.wakes {
        tx.execute("INSERT INTO app_wakes(owner_id,installation_id,wake_id,due_at_ms,revision,attempts,next_attempt_at_ms) VALUES(?1,?2,?3,?4,?5,0,?4)",
                params![request.owner, installation, wake.id, wake.due_at_ms as i64, wake.revision]).map_err(|_| Error::StorageUnavailable)?;
    }
    tx.execute("INSERT INTO app_restore_receipts(owner_id,installation_id,restore_id) VALUES(?1,?2,?3) ON CONFLICT(installation_id) DO UPDATE SET owner_id=excluded.owner_id,restore_id=excluded.restore_id",
            params![request.owner, installation, request.restore_id]).map_err(|_| Error::StorageUnavailable)?;
    Ok(tx)
}
