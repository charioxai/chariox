//! MP-08/MP-11: permanent owner replay/launch authority, separate from live receipts.
use super::*;

const OWNER_HISTORY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerContextRecord {
    schema_version: u32,
    pub(super) target: crate::local::ManagedContextLaunchTarget,
    pub(super) authority: OwnerContextAuthority,
    // Legacy receipts may already have expired. New completions retain enough
    // information to settle a crash between this commit and live-index persistence.
    completion: Option<OwnerImportCompletion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerImportCompletion {
    transfer_id: String,
    receipt_json: String,
    completed_at_ms: u64,
}

impl ManagedContextTransferStore {
    fn owner_history_path(&self, context_id: &str) -> PathBuf {
        self.root
            .join("owner-contexts")
            .join(format!("{}.json", sha256_bytes(context_id.as_bytes())))
    }

    pub(super) fn owner_history(
        &self,
        context_id: &str,
    ) -> Result<Option<OwnerContextRecord>, DaemonError> {
        let parent = self.root.join("owner-contexts");
        match fs::symlink_metadata(&parent) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(transfer_error(
                    "owner context history must be a real directory",
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(transfer_io_error("inspect owner context history", error)),
        }
        let Some(bytes) = storage::read_private_file_bounded(
            &self.owner_history_path(context_id),
            MAX_PERSISTED_IMPORT_BYTES as u64,
        )?
        else {
            return Ok(None);
        };
        let record: OwnerContextRecord = serde_json::from_slice(&bytes)
            .map_err(|_| transfer_error("invalid owner context history"))?;
        validate_owner_record(context_id, &record)?;
        Ok(Some(record))
    }

    fn persist_owner_history(&self, record: &OwnerContextRecord) -> Result<(), DaemonError> {
        let context_id = &record.target.context_id;
        validate_owner_record(context_id, record)?;
        if let Some(existing) = self.owner_history(context_id)? {
            if existing.target != record.target
                || existing.authority != record.authority
                || (record.completion.is_some() && existing.completion != record.completion)
            {
                return Err(transfer_error(
                    "owner context history conflicts with completion",
                ));
            }
            return Ok(());
        }
        let bytes = serde_json::to_vec(record)
            .map_err(|_| transfer_error("serialize owner context history"))?;
        if bytes.len() > MAX_PERSISTED_IMPORT_BYTES {
            return Err(transfer_error(
                "owner context history exceeds its record capacity",
            ));
        }
        ensure_private_directory(&self.root.join("owner-contexts"))?;
        // Make a newly created history directory durable before committing into it.
        #[cfg(unix)]
        fs::File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| transfer_io_error("sync owner context history root", error))?;
        write_private_state_file(&self.owner_history_path(context_id), &bytes)
    }

    // Called under the live-state lock. This per-context atomic write is the
    // owner completion commit point. A lost index write/reply cannot erase it.
    pub(super) fn commit_owner_import(
        &self,
        state: &mut PersistedTransferState,
        transfer_id: &str,
        target: crate::local::ManagedContextLaunchTarget,
        receipt_json: &str,
        now_ms: u64,
    ) -> Result<(), DaemonError> {
        let entry = state
            .entries
            .get(transfer_id)
            .expect("validated importing transfer");
        let record = OwnerContextRecord {
            schema_version: OWNER_HISTORY_SCHEMA_VERSION,
            target,
            authority: owner_authority(entry),
            completion: Some(OwnerImportCompletion {
                transfer_id: transfer_id.to_string(),
                receipt_json: receipt_json.to_string(),
                completed_at_ms: now_ms,
            }),
        };
        // On a retried commit, preserve the original durable completion time.
        let record = if let Some(existing) = self.owner_history(&record.target.context_id)? {
            if existing.target != record.target
                || existing.authority != record.authority
                || existing.completion.as_ref().is_none_or(|completion| {
                    completion.transfer_id != transfer_id || completion.receipt_json != receipt_json
                })
            {
                return Err(transfer_error(
                    "owner context history conflicts with completion",
                ));
            }
            existing
        } else {
            record
        };
        self.persist_owner_history(&record)?;
        settle_owner_completion(
            state.entries.get_mut(transfer_id).unwrap(),
            record.completion.as_ref().unwrap(),
        );
        let persisted = self.persist_locked(state);
        self.lock_active_imports().remove(transfer_id);
        persisted?;
        self.cleanup_transfer_artifacts(transfer_id)
    }

    pub(super) fn externalize_owner_history(
        &self,
        state: &mut PersistedTransferState,
    ) -> Result<bool, DaemonError> {
        let mut changed = false;
        // Migrate bounded v5 history before dropping a single replay/launch binding.
        // A crash leaves the old state and identical new record; retries are safe.
        for (context_id, authority) in state.owner_context_authorities.clone() {
            let target = state
                .applied_contexts
                .get(&context_id)
                .expect("validated owner target")
                .clone();
            let completion = state
                .entries
                .iter()
                .find(|(_, entry)| {
                    entry.plan.context_id == context_id
                        && entry.phase == ManagedContextTransferPhase::Consumed
                })
                .map(|(id, entry)| OwnerImportCompletion {
                    transfer_id: id.clone(),
                    receipt_json: entry
                        .import_receipt_json
                        .clone()
                        .expect("validated consumed receipt"),
                    completed_at_ms: entry.completed_at_ms.expect("validated completion time"),
                });
            self.persist_owner_history(&OwnerContextRecord {
                schema_version: OWNER_HISTORY_SCHEMA_VERSION,
                target,
                authority,
                completion,
            })?;
            state.owner_context_authorities.remove(&context_id);
            state.applied_contexts.remove(&context_id);
            state.consumed_context_ids.remove(&context_id);
            changed = true;
        }
        // Only inspect the bounded live index; never scan or load permanent
        // history wholesale. Reconcile the durable owner commit before cleanup.
        for (id, entry) in &mut state.entries {
            if entry.plan.destination.is_none() {
                continue;
            }
            let Some(record) = self.owner_history(&entry.plan.context_id)? else {
                if entry.phase == ManagedContextTransferPhase::Consumed {
                    return Err(transfer_error("consumed owner context history is missing"));
                }
                continue;
            };
            if record.authority != owner_authority(entry) {
                return Err(transfer_error("owner context history authority changed"));
            }
            if !matches!(
                entry.phase,
                ManagedContextTransferPhase::Importing | ManagedContextTransferPhase::Consumed
            ) {
                return Err(transfer_error(
                    "owner context history conflicts with live transfer",
                ));
            }
            if let Some(completion) = &record.completion {
                let receipt = serde_json::from_str(&completion.receipt_json)
                    .map_err(|_| transfer_error("invalid owner completion receipt"))?;
                if completion.transfer_id != *id
                    || launch_target_from_receipt(id, entry, &receipt)? != record.target
                    || (entry.phase == ManagedContextTransferPhase::Consumed
                        && entry.import_receipt_json.as_deref()
                            != Some(completion.receipt_json.as_str()))
                {
                    return Err(transfer_error(
                        "owner context history does not match live transfer",
                    ));
                }
                if entry.phase == ManagedContextTransferPhase::Importing {
                    settle_owner_completion(entry, completion);
                    changed = true;
                }
            } else if entry.phase != ManagedContextTransferPhase::Consumed {
                return Err(transfer_error(
                    "owner context history lacks a recoverable completion",
                ));
            }
        }
        Ok(changed)
    }
}

fn owner_authority(entry: &PersistedTransfer) -> OwnerContextAuthority {
    OwnerContextAuthority {
        owner_user_id: entry.owner_user_id.clone(),
        realm_id: entry.realm_id.clone(),
        target_key_thumbprint: entry.target_key_thumbprint.clone(),
    }
}

fn settle_owner_completion(entry: &mut PersistedTransfer, completion: &OwnerImportCompletion) {
    entry.phase = ManagedContextTransferPhase::Consumed;
    entry.import_receipt_sha256 = Some(sha256_bytes(completion.receipt_json.as_bytes()));
    entry.import_receipt_json = Some(completion.receipt_json.clone());
    entry.completed_at_ms = Some(completion.completed_at_ms);
}

fn validate_owner_record(context_id: &str, record: &OwnerContextRecord) -> Result<(), DaemonError> {
    if record.schema_version != OWNER_HISTORY_SCHEMA_VERSION
        || record.target.context_id != context_id
        || record.target.destination.is_none()
    {
        return Err(transfer_error("invalid owner context history binding"));
    }
    let mut state = PersistedTransferState::default();
    state.consumed_context_ids.insert(context_id.to_string());
    state
        .applied_contexts
        .insert(context_id.to_string(), record.target.clone());
    state
        .owner_context_authorities
        .insert(context_id.to_string(), record.authority.clone());
    validate_persisted_state(&state)?;
    if let Some(completion) = &record.completion {
        let receipt: crate::managed_context::package::ManagedContextPackageImportReceipt =
            serde_json::from_str(&completion.receipt_json)
                .map_err(|_| transfer_error("invalid owner completion receipt"))?;
        if !policy::valid_transfer_id(&completion.transfer_id)
            || completion.receipt_json.len() > MAX_IMPORT_RECEIPT_BYTES
            || completion.completed_at_ms == 0
            || receipt.transfer_id != completion.transfer_id
            || receipt.destination != record.target.destination
            || receipt.plan_digest != record.target.plan_digest
        {
            return Err(transfer_error("invalid owner completion bounds"));
        }
    }
    Ok(())
}
