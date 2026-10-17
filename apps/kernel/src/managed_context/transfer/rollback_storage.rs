//! MP-07 / MP-08 / MP-11: retain the protocol-491 index across upgrade/rollback.
//!
//! That reader predates owner destinations. Keep their live transfers beside
//! the permanent owner-contexts history, leaving only schema-5 disposable state
//! in state.json. The predecessor never reads or rewrites either owner store.
use std::path::Path;

use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerTransfers {
    schema_version: u32,
    entries: BTreeMap<String, PersistedTransfer>,
}

pub(super) fn read_state(root: &Path) -> Result<PersistedTransferState, DaemonError> {
    let mut state = match read_private_state_file(&root.join("state.json"))? {
        Some(bytes) => serde_json::from_slice::<PersistedTransferState>(&bytes)
            .map_err(|_| transfer_error("parse managed context transfer state"))?,
        None => PersistedTransferState::default(),
    };
    if !matches!(state.schema_version, 1..=6) {
        return Err(transfer_error(format!(
            "unsupported managed context transfer state version {}",
            state.schema_version
        )));
    }
    if let Some(bytes) = read_private_state_file(&root.join("owner-transfers.json"))? {
        let owners: OwnerTransfers = serde_json::from_slice(&bytes)
            .map_err(|_| transfer_error("parse owner context transfer state"))?;
        if owners.schema_version != 1
            || owners.entries.len() > MAX_TRANSFER_RECORDS
            || owners
                .entries
                .values()
                .any(|entry| entry.plan.destination.is_none())
        {
            return Err(transfer_error("invalid owner context transfer state"));
        }
        for (id, owner) in owners.entries {
            if let Some(legacy) = state.entries.get(&id) {
                // A migration can commit the sidecar and crash before replacing
                // the old index. Reconciliation may have settled an owner commit
                // in the sidecar; all other transfer fields must still agree.
                let mut legacy = legacy.clone();
                if legacy.phase == ManagedContextTransferPhase::Importing
                    && owner.phase == ManagedContextTransferPhase::Consumed
                {
                    legacy.phase = owner.phase;
                    legacy.import_receipt_sha256 = owner.import_receipt_sha256.clone();
                    legacy.import_receipt_json = owner.import_receipt_json.clone();
                    legacy.completed_at_ms = owner.completed_at_ms;
                }
                if legacy != owner {
                    return Err(transfer_error("owner transfer conflicts with legacy state"));
                }
            }
            state.entries.insert(id, owner);
        }
    }
    Ok(state)
}

pub(super) fn persist_state(
    root: &Path,
    state: &PersistedTransferState,
) -> Result<(), DaemonError> {
    let mut predecessor = state.clone();
    let mut owners = OwnerTransfers {
        schema_version: 1,
        entries: BTreeMap::new(),
    };
    predecessor.entries.retain(|id, entry| {
        if entry.plan.destination.is_some() {
            owners.entries.insert(id.clone(), entry.clone());
            false
        } else {
            true
        }
    });
    // Owner authority is externalized before persistence, including v5/v6
    // migration. Never emit owner launch targets that 491 cannot interpret.
    if !predecessor.owner_context_authorities.is_empty()
        || predecessor
            .applied_contexts
            .values()
            .any(|target| target.destination.is_some())
    {
        return Err(transfer_error(
            "owner history must be committed before index persistence",
        ));
    }
    let owner_bytes = serde_json::to_vec(&owners)
        .map_err(|_| transfer_error("serialize owner context transfer state"))?;
    let predecessor_bytes = serde_json::to_vec(&predecessor)
        .map_err(|_| transfer_error("serialize managed context transfer state"))?;
    // Commit owner state first: a failed main-index write must never erase an
    // import or its receipt. read_state handles interrupted legacy migration.
    write_private_state_file(&root.join("owner-transfers.json"), &owner_bytes)?;
    write_private_state_file(&root.join("state.json"), &predecessor_bytes)
}
