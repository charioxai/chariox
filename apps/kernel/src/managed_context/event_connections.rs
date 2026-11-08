//! Event connection records that move to another kernel of the same owner.
//!
//! A record names a connection at the event services; it holds no provider
//! credential, which stays with those services. The target does not call them
//! while importing: a connection they no longer know is marked unavailable the
//! first time the target refreshes or attaches it.
//!
//! The event owner id inside a record is a bearer capability for the
//! connection, so a record only travels inside the encrypted package and never
//! appears in `Debug` output, receipts or errors.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::package::ManagedContextEventConnectionSelection;
use crate::error::DaemonError;
use crate::event_connection::EventConnectionRegistry;
use crate::local::EventConnectionStatus;

pub(crate) const MAX_EVENT_CONNECTIONS: usize = 16;
pub(crate) const MAX_EVENT_CONNECTION_COMPONENT_BYTES: u64 = 256 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventConnectionRecord {
    generator_id: String,
    connection_id: String,
    event_owner_id: String,
    status: EventConnectionStatus,
}

impl std::fmt::Debug for EventConnectionRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventConnectionRecord")
            .field("generator_id", &self.generator_id)
            .field("connection_id", &self.connection_id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// The ids of the records an import wrote. `created` is false for a record the
/// target already held; it says which records to read from the event service
/// after the import, and a rollback decides from the import ledger instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedContextEventConnectionReceipt {
    pub connection_id: String,
    pub created: bool,
}

#[derive(Clone)]
pub(crate) struct ManagedContextEventConnectionImportTarget {
    pub registry: EventConnectionRegistry,
    pub owner_user_id: String,
}

impl ManagedContextEventConnectionImportTarget {
    pub(crate) fn new(
        registry: EventConnectionRegistry,
        config: &crate::config::DaemonConfig,
        owner_user_id: &str,
    ) -> Self {
        Self {
            registry,
            owner_user_id: crate::event_connection::event_connection_user_id(config, owner_user_id),
        }
    }
}

impl std::fmt::Debug for ManagedContextEventConnectionImportTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedContextEventConnectionImportTarget")
            .field("owner_user_id", &self.owner_user_id)
            .finish_non_exhaustive()
    }
}

/// Reads the selected connections from the source registry, refusing any that
/// is missing, not ready or has no event owner id. The transfer calls this
/// before it spends Cloud authority, and again to build the package.
pub(crate) fn export_selected(
    selection: &ManagedContextEventConnectionSelection,
    registry: &EventConnectionRegistry,
    owner_user_id: &str,
) -> Result<Vec<EventConnectionRecord>, DaemonError> {
    let ManagedContextEventConnectionSelection::Selected { connection_ids } = selection else {
        return Ok(Vec::new());
    };
    connection_ids
        .iter()
        .map(|connection_id| {
            let connection = registry.get(owner_user_id, connection_id)?.ok_or_else(|| {
                refusal(
                    "event_connection_not_found",
                    "a selected event connection does not exist on this kernel",
                )
            })?;
            let event_owner_id = registry
                .connection_event_owner_id(owner_user_id, connection_id)?
                .filter(|event_owner_id| !event_owner_id.is_empty());
            match event_owner_id {
                Some(event_owner_id) if connection.status == EventConnectionStatus::Ready => {
                    Ok(EventConnectionRecord {
                        generator_id: connection.generator_id,
                        connection_id: connection.connection_id,
                        event_owner_id,
                        status: connection.status,
                    })
                }
                _ => Err(refusal(
                    "event_connection_not_ready",
                    "a selected event connection is not ready to copy",
                )),
            }
        })
        .collect()
}

pub(crate) fn validate_selection(
    selection: &ManagedContextEventConnectionSelection,
) -> Result<(), DaemonError> {
    let ManagedContextEventConnectionSelection::Selected { connection_ids } = selection else {
        return Ok(());
    };
    let mut previous: Option<&str> = None;
    for connection_id in connection_ids {
        if connection_id.is_empty()
            || connection_id.trim() != connection_id
            || connection_id.len() > 256
            || connection_ids.len() > MAX_EVENT_CONNECTIONS
            || previous.is_some_and(|value| value >= connection_id.as_str())
        {
            return Err(refusal(
                "event_connection_selection_invalid",
                "the event connection selection is invalid",
            ));
        }
        previous = Some(connection_id);
    }
    if connection_ids.is_empty() {
        return Err(refusal(
            "event_connection_selection_invalid",
            "the event connection selection is invalid",
        ));
    }
    Ok(())
}

/// The package carries exactly the selected connections, in selection order.
pub(crate) fn validate_records(
    selection: &ManagedContextEventConnectionSelection,
    records: &[EventConnectionRecord],
) -> Result<(), DaemonError> {
    let ManagedContextEventConnectionSelection::Selected { connection_ids } = selection else {
        return Err(refusal(
            "event_connection_payload_unselected",
            "the package carries event connections that were not selected",
        ));
    };
    if records.len() != connection_ids.len()
        || records.iter().zip(connection_ids).any(|(record, id)| {
            record.connection_id != *id
                || record.generator_id.is_empty()
                || record.event_owner_id.is_empty()
        })
    {
        return Err(refusal(
            "event_connection_payload_mismatch",
            "the package event connections do not match the selection",
        ));
    }
    Ok(())
}

pub(crate) fn receipts_for(
    records: &[EventConnectionRecord],
) -> Vec<ManagedContextEventConnectionReceipt> {
    records
        .iter()
        .map(|record| ManagedContextEventConnectionReceipt {
            connection_id: record.connection_id.clone(),
            created: true,
        })
        .collect()
}

/// The transfers that rely on each record an import wrote, keyed by owner and
/// connection id. A record absent from the ledger belongs to the owner (or to
/// an import that is no longer running) and a rollback leaves it alone. The
/// ledger lives in the registry because overlapping transfers share it; it
/// holds nothing across a restart, which is when a recovered import leaves its
/// records anyway.
#[derive(Default)]
pub(crate) struct ImportLedger {
    entries: BTreeMap<(String, String), LedgerEntry>,
}

struct LedgerEntry {
    /// An import wrote the record, so it goes when the last transfer that
    /// relies on it fails.
    created_by_import: bool,
    transfers: BTreeSet<String>,
}

/// Writes the records into the target registry. A record the target already
/// holds is kept when it is the same connection and refused when it is not;
/// nothing changes unless every record is acceptable. The check and the write
/// run under the ledger lock, so exactly one of two overlapping transfers
/// creates a shared record and the other reuses it.
pub(crate) fn import(
    target: &ManagedContextEventConnectionImportTarget,
    transfer_id: &str,
    records: &[EventConnectionRecord],
) -> Result<Vec<ManagedContextEventConnectionReceipt>, DaemonError> {
    let mut ledger = target.registry.lock_import_ledger()?;
    let mut missing = Vec::with_capacity(records.len());
    for record in records {
        let existing = target
            .registry
            .get(&target.owner_user_id, &record.connection_id)?;
        let existing_owner = target
            .registry
            .connection_event_owner_id(&target.owner_user_id, &record.connection_id)?;
        match (existing, existing_owner) {
            (None, _) => missing.push(true),
            (Some(connection), Some(event_owner_id))
                if connection.generator_id == record.generator_id
                    && event_owner_id == record.event_owner_id =>
            {
                missing.push(false)
            }
            _ => {
                return Err(refusal(
                    "event_connection_conflict",
                    "an event connection with the same id already exists on this kernel",
                ))
            }
        }
    }
    let mut receipts = Vec::with_capacity(records.len());
    for (record, created) in records.iter().zip(missing) {
        let key = (target.owner_user_id.clone(), record.connection_id.clone());
        if created {
            if let Err(error) = target.registry.upsert(
                &target.owner_user_id,
                &record.event_owner_id,
                chariox_event_protocol::AegsConnectionSummary {
                    generator_id: record.generator_id.clone(),
                    connection_id: record.connection_id.clone(),
                    status: record.status,
                    metadata: serde_json::Value::Null,
                    expires_at_ms: None,
                    updated_at_ms: crate::session::unix_epoch_ms(),
                },
            ) {
                if let Err(rollback_error) =
                    rollback_locked(&mut ledger, target, transfer_id, &receipts)
                {
                    return Err(DaemonError::ManagedContext {
                        code: "managed_context_unavailable",
                        operation: "import event connections",
                        message: format!("{error}; {rollback_error}"),
                        retryable: true,
                    });
                }
                return Err(error);
            }
            ledger.entries.insert(
                key,
                LedgerEntry {
                    created_by_import: true,
                    transfers: BTreeSet::from([transfer_id.to_string()]),
                },
            );
        } else if let Some(entry) = ledger.entries.get_mut(&key) {
            entry.transfers.insert(transfer_id.to_string());
        }
        receipts.push(ManagedContextEventConnectionReceipt {
            connection_id: record.connection_id.clone(),
            created,
        });
    }
    Ok(receipts)
}

/// Gives up this transfer's claim on its records and removes a record only
/// when an import wrote it and no other transfer relies on it any more.
pub(crate) fn rollback(
    target: &ManagedContextEventConnectionImportTarget,
    transfer_id: &str,
    receipts: &[ManagedContextEventConnectionReceipt],
) -> Result<(), DaemonError> {
    let mut ledger = target.registry.lock_import_ledger()?;
    rollback_locked(&mut ledger, target, transfer_id, receipts)
}

fn rollback_locked(
    ledger: &mut ImportLedger,
    target: &ManagedContextEventConnectionImportTarget,
    transfer_id: &str,
    receipts: &[ManagedContextEventConnectionReceipt],
) -> Result<(), DaemonError> {
    let mut failures = 0;
    for receipt in receipts {
        let key = (target.owner_user_id.clone(), receipt.connection_id.clone());
        let Some(entry) = ledger.entries.get_mut(&key) else {
            continue;
        };
        entry.transfers.remove(transfer_id);
        if !entry.transfers.is_empty() {
            continue;
        }
        if entry.created_by_import
            && target
                .registry
                .remove(&target.owner_user_id, &receipt.connection_id)
                .is_err()
        {
            entry.transfers.insert(transfer_id.to_string());
            failures += 1;
            continue;
        }
        ledger.entries.remove(&key);
    }
    if failures > 0 {
        return Err(DaemonError::ManagedContext {
            code: "managed_context_unavailable",
            operation: "roll back event connections",
            message: format!("{failures} event connection record(s) could not be removed"),
            retryable: true,
        });
    }
    Ok(())
}

fn refusal(code: &'static str, message: &'static str) -> DaemonError {
    DaemonError::ManagedContext {
        code,
        operation: "managed context event connections",
        message: message.to_string(),
        retryable: false,
    }
}
