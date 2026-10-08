//! MP-08/MP-11: bounded terminal receipts, without retiring recovery authority.
use super::*;

// Keep up to 128 recent terminal receipts for at most the archive retention
// window (24 hours), reserving capacity for a new status plus two owner files.
const MAX_RETAINED_TERMINAL_OPERATIONS: usize = MAX_OUTBOUND_OPERATIONS / 2;
const MAX_OPERATION_METADATA_SCAN_ENTRIES: usize = MAX_OUTBOUND_OPERATIONS * 3 + 3;

fn terminal(status: &ManagedContextOutboundOperationStatus) -> bool {
    status.phase == ManagedContextOutboundOperationPhase::Completed
        || (status.phase == ManagedContextOutboundOperationPhase::Failed && !status.retryable)
}

fn sync_metadata_directory(parent: &Path) -> Result<(), DaemonError> {
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|error| outbound_service_io_error("sync operation retirement", error))?;
    Ok(())
}

impl ManagedContextOutboundOperationStore {
    pub(crate) fn reclaim_operation_metadata(
        &self,
        protected_context: Option<&str>,
    ) -> Result<(), DaemonError> {
        let Some(parent) = self.status_parent() else {
            return Ok(());
        };
        let _guard = self
            .artifact_lock
            .lock()
            .expect("operation retirement lock");
        create_private_directory(&parent)?;
        let entries = fs::read_dir(&parent)
            .map_err(|error| outbound_service_io_error("list operation metadata", error))?
            .take(MAX_OPERATION_METADATA_SCAN_ENTRIES + 1)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| outbound_service_io_error("list operation metadata", error))?;
        if entries.len() > MAX_OPERATION_METADATA_SCAN_ENTRIES {
            return Err(outbound_service_error(
                "operation metadata exceeds its scan limit",
                false,
            ));
        }
        let mut entry_count = entries.len();
        let mut statuses = Vec::new();
        let mut disk_unfinished = BTreeSet::new();
        let mut disk_counts = BTreeMap::<String, usize>::new();
        let mut bindings: BTreeMap<String, Vec<(PathBuf, String)>> = BTreeMap::new();
        for entry in entries {
            let path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            // Unknown, malformed or unsafe entries are retained and still consume
            // capacity. Never follow or remove them to make admission succeed.
            let Ok(bytes) = read_bounded_regular_file(&path, 256 * 1024) else {
                continue;
            };
            if let Ok(status) =
                serde_json::from_slice::<ManagedContextOutboundOperationStatus>(&bytes)
            {
                if !valid_artifact_name(&status.context_id) {
                    continue;
                }
                let retiring = name == format!(".retired-{}.json", status.context_id);
                if name == format!("{}.json", status.context_id) || (retiring && terminal(&status))
                {
                    if !terminal(&status) {
                        disk_unfinished.insert(status.context_id.clone());
                    }
                    *disk_counts.entry(status.context_id.clone()).or_default() += 1;
                    statuses.push((status, path, retiring));
                }
            } else if let Ok(saved) = serde_json::from_slice::<PersistedOwnerTicket>(&bytes) {
                let plan = saved.ticket.context_plan.package_binding();
                if !valid_artifact_name(&plan.context_id) {
                    continue;
                }
                if name == format!("{}-owner.json", plan.context_id)
                    || name
                        == format!(
                            "owner-{}.json",
                            plan.plan_digest.trim_start_matches("sha256:")
                        )
                {
                    bindings
                        .entry(plan.context_id)
                        .or_default()
                        .push((path, plan.plan_digest));
                }
            }
        }
        // Lock order matches owner preparation: artifact -> status -> active.
        // A live terminal update or a memory-only retryable failure cannot be
        // erased based on an older durable status.
        let mut state = self.state.lock().expect("operation retirement status lock");
        let active = self.active_context_ids();
        statuses.retain(|(status, _, retiring)| {
            terminal(status)
                && !active.contains(&status.context_id)
                && !disk_unfinished.contains(&status.context_id)
                && disk_counts.get(&status.context_id) == Some(&1)
                && (*retiring || protected_context != Some(status.context_id.as_str()))
                && state.get(&status.context_id).is_none_or(|memory| {
                    terminal(memory) && memory.plan_digest == status.plan_digest
                        && memory.updated_at_ms == status.updated_at_ms
                })
                && bindings.get(&status.context_id).is_none_or(|files| {
                    files
                        .iter()
                        .all(|(_, digest)| digest == &status.plan_digest)
                })
        });
        statuses.sort_by(|(a, _, ar), (b, _, br)| {
            (!*ar, a.updated_at_ms, &a.context_id).cmp(&(!*br, b.updated_at_ms, &b.context_id))
        });
        let mut remaining = statuses.len();
        let now = crate::session::unix_epoch_ms();
        for (status, path, retiring) in statuses {
            let expired =
                now.saturating_sub(status.updated_at_ms) >= OUTBOUND_ARTIFACT_RETENTION_MS;
            if !retiring
                && !expired
                && remaining <= MAX_RETAINED_TERMINAL_OPERATIONS
                && entry_count <= MAX_OUTBOUND_OPERATIONS * 3 - 5
            {
                continue;
            }
            let tombstone = parent.join(format!(".retired-{}.json", status.context_id));
            if !retiring {
                // Rename removes the queryable status before removing its owner
                // authority. A crash leaves a terminal tombstone to finish on
                // reopen, not an unauthenticated receipt or immortal orphan.
                if path_entry_exists(&tombstone)? {
                    return Err(outbound_service_error(
                        "operation retirement is already pending",
                        false,
                    ));
                }
                fs::rename(&path, &tombstone)
                    .map_err(|error| outbound_service_io_error("retire terminal status", error))?;
                sync_metadata_directory(&parent)?;
            }
            state.remove(&status.context_id);
            if let Some(files) = bindings.remove(&status.context_id) {
                for (file, _) in files {
                    fs::remove_file(file).map_err(|error| {
                        outbound_service_io_error("retire owner operation metadata", error)
                    })?;
                    entry_count = entry_count.saturating_sub(1);
                }
            }
            // Indexes are selected by their current context binding. A plan
            // index already rebound to another context remains untouched.
            fs::remove_file(tombstone)
                .map_err(|error| outbound_service_io_error("settle operation retirement", error))?;
            sync_metadata_directory(&parent)?;
            entry_count = entry_count.saturating_sub(1);
            remaining -= 1;
        }
        Ok(())
    }
}
