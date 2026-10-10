//! MP-08/MP-11: bounded terminal receipts, without retiring recovery authority.
use super::*;
use std::io;

// Keep up to 128 recent terminal receipts for at most the archive retention
// window (24 hours), reserving capacity for a new status plus two owner files.
const MAX_RETAINED_TERMINAL_OPERATIONS: usize = MAX_OUTBOUND_OPERATIONS / 2;
const MAX_OPERATION_METADATA_SCAN_ENTRIES: usize = MAX_OUTBOUND_OPERATIONS * 3 + 3;

fn terminal(status: &ManagedContextOutboundOperationStatus) -> bool {
    status.phase == ManagedContextOutboundOperationPhase::Completed
        || (status.phase == ManagedContextOutboundOperationPhase::Failed && !status.retryable)
}

// MP-08 / MP-11: an inactive operation past the recovery window may retire
// only when no matching, unexpired package can still resume it. Never interpret
// unreadable or unsafe recovery state as permission to remove owner authority.
fn retirable(
    status: &ManagedContextOutboundOperationStatus,
    artifact_parent: &Path,
    now: u64,
) -> Result<bool, DaemonError> {
    if terminal(status) {
        return Ok(true);
    }
    if now.saturating_sub(status.updated_at_ms) < OUTBOUND_ARTIFACT_RETENTION_MS {
        return Ok(false);
    }
    let root = artifact_parent.join(&status.context_id);
    if !path_entry_exists(&root)? {
        return Ok(true);
    }
    validate_artifact_root(&root)?;
    if path_entry_exists(&root.join("retired"))? || !path_entry_exists(&root.join("state.json"))? {
        return Ok(true);
    }
    let bytes =
        read_bounded_regular_file(&root.join("state.json"), MAX_OUTBOUND_ARTIFACT_STATE_BYTES)?;
    let persisted: PersistedOutboundArtifact = serde_json::from_slice(&bytes)
        .map_err(|_| outbound_service_error("invalid operation recovery state", false))?;
    if persisted.schema_version != OUTBOUND_ARTIFACT_SCHEMA_VERSION
        || persisted.plan_digest != status.plan_digest
    {
        return Ok(false);
    }
    if persisted.created_at_ms == 0
        || now.saturating_sub(persisted.created_at_ms) >= OUTBOUND_ARTIFACT_RETENTION_MS
    {
        return Ok(true);
    }
    let package = root.join("managed-context.pkg");
    let metadata = match fs::symlink_metadata(&package) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => {
            return Err(outbound_service_io_error(
                "inspect operation recovery package",
                error,
            ))
        }
    };
    Ok(metadata.is_file() && metadata.len() != persisted.package_size_bytes)
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
        self.reclaim_operation_metadata_inner(protected_context, false, None)
    }

    pub(crate) fn reclaim_operation_metadata_at_startup(&self) -> Result<(), DaemonError> {
        self.reclaim_operation_metadata_inner(None, true, None)
    }

    pub(crate) fn reclaim_rejected_preparation(&self, context_id: &str) -> Result<(), DaemonError> {
        self.reclaim_operation_metadata_inner(None, false, Some(context_id))
    }

    fn reclaim_operation_metadata_inner(
        &self,
        protected_context: Option<&str>,
        startup: bool,
        rejected_context: Option<&str>,
    ) -> Result<(), DaemonError> {
        let Some(parent) = self.status_parent() else {
            return Ok(());
        };
        let _guard = self
            .artifact_lock
            .lock()
            .expect("operation retirement lock");
        // Reopening an unused store must not materialize metadata. Persistence
        // creates the directory only when admitting the first real record.
        if !path_entry_exists(&parent)? {
            return Ok(());
        }
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
        let now = crate::session::unix_epoch_ms();
        let mut entry_count = entries.len();
        let mut statuses = Vec::new();
        let mut disk_unfinished = BTreeSet::new();
        let mut disk_counts = BTreeMap::<String, usize>::new();
        let mut bindings: BTreeMap<String, Vec<(PathBuf, String, bool)>> = BTreeMap::new();
        let mut indexed = BTreeSet::new();
        let mut unconsumed = BTreeSet::new();
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
                let eligible = retirable(
                    &status,
                    self.artifact_parent.as_deref().expect("artifact parent"),
                    now,
                )?;
                if name == format!("{}.json", status.context_id) || (retiring && eligible) {
                    if !eligible {
                        disk_unfinished.insert(status.context_id.clone());
                    }
                    *disk_counts.entry(status.context_id.clone()).or_default() += 1;
                    statuses.push((status, path, retiring, eligible));
                }
            } else if let Ok(saved) = serde_json::from_slice::<PersistedOwnerTicket>(&bytes) {
                let plan = saved.ticket.context_plan.package_binding();
                if !valid_artifact_name(&plan.context_id) {
                    continue;
                }
                let index = name
                    == format!(
                        "owner-{}.json",
                        plan.plan_digest.trim_start_matches("sha256:")
                    );
                if index {
                    indexed.insert(plan.context_id.clone());
                }
                if index || name == format!("{}-owner.json", plan.context_id) {
                    if saved.consumption_attempted == Some(false) {
                        unconsumed.insert(plan.context_id.clone());
                    }
                    bindings.entry(plan.context_id).or_default().push((
                        path,
                        plan.plan_digest,
                        saved.consumption_attempted == Some(false),
                    ));
                }
            }
        }
        // Lock order matches owner preparation: artifact -> status -> active.
        // A live terminal update or a memory-only retryable failure cannot be
        // erased based on an older durable status.
        let mut state = self.state.lock().expect("operation retirement status lock");
        let active = self.active_context_ids();
        // MP-08/MP-11: no status means no operation was admitted. Retire an
        // explicitly unconsumed preparation when its start was rejected, its
        // index moved on, or startup has no in-flight preparation to preserve.
        // Legacy/attempted bindings and admitted/recoverable statuses stay intact.
        for context_id in unconsumed {
            if (!indexed.contains(&context_id)
                || startup
                || rejected_context == Some(context_id.as_str()))
                && !disk_counts.contains_key(&context_id)
                && !path_entry_exists(&parent.join(format!("{context_id}.json")))?
                && !path_entry_exists(&parent.join(format!(".retired-{context_id}.json")))?
                && !path_entry_exists(
                    &self
                        .artifact_parent
                        .as_deref()
                        .expect("artifact parent")
                        .join(&context_id),
                )?
                && !state.contains_key(&context_id)
                && !active.contains(&context_id)
                && protected_context != Some(context_id.as_str())
                && bindings
                    .get(&context_id)
                    .is_some_and(|files| files.iter().all(|(_, _, unconsumed)| *unconsumed))
            {
                let mut files = bindings.remove(&context_id).expect("unconsumed binding");
                // Remove the index first; a crash leaves a reclaimable context
                // binding. Also handle an index-only interrupted preparation.
                files.sort_by_key(|(path, _, _)| {
                    !path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("owner-")
                });
                for (path, _, _) in files {
                    fs::remove_file(path).map_err(|error| {
                        outbound_service_io_error("retire unadmitted owner binding", error)
                    })?;
                    sync_metadata_directory(&parent)?;
                    entry_count = entry_count.saturating_sub(1);
                }
            }
        }
        statuses.retain(|(status, _, retiring, eligible)| {
            *eligible
                && !active.contains(&status.context_id)
                && !disk_unfinished.contains(&status.context_id)
                && disk_counts.get(&status.context_id) == Some(&1)
                && (*retiring || protected_context != Some(status.context_id.as_str()))
                && state.get(&status.context_id).is_none_or(|memory| {
                    memory.phase == status.phase
                        && memory.retryable == status.retryable
                        && memory.plan_digest == status.plan_digest
                        && memory.updated_at_ms == status.updated_at_ms
                })
                && bindings.get(&status.context_id).is_none_or(|files| {
                    files
                        .iter()
                        .all(|(_, digest, _)| digest == &status.plan_digest)
                })
        });
        statuses.sort_by(|(a, _, ar, _), (b, _, br, _)| {
            (!*ar, a.updated_at_ms, &a.context_id).cmp(&(!*br, b.updated_at_ms, &b.context_id))
        });
        let mut remaining = statuses.len();
        for (status, path, retiring, _) in statuses {
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
                for (file, _, _) in files {
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
