//! Runs inside the existing slice-start operation and blocking supervisor task.
//! Saved-state records are the durable authority across retries and restarts.
use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;
use crate::slice::{
    chromium_migration as docker, LocalDockerSliceOptions, LocalDockerSliceRelay,
    SliceOperationStatus, SliceRecord, SliceSavedStateRecord,
};

pub(super) fn provision(
    runtime: &KernelRuntimeState,
    record: &SliceRecord,
    relay: Option<LocalDockerSliceRelay>,
    options: &LocalDockerSliceOptions,
) -> Result<(), DaemonError> {
    if options.allow_unconfined_seccomp {
        return docker::provision(record, relay, options, None);
    }
    docker::prepare_host(record)?;
    let current = docker::inspect(&docker::canonical_name(record))?;
    let saved = runtime.active_saved_state_for_slice(&record.id)?;
    let migration_state = saved.filter(|state| {
        state.source_slice_id == record.id
            && state.last_operation.as_deref() == Some(docker::MIGRATION_OPERATION)
    });
    let pending = if let Some(state) = migration_state {
        let rollback_exists = docker::inspect(&docker::rollback_name(record, &state))?.is_some();
        (state.last_operation_status != Some(SliceOperationStatus::Completed) || rollback_exists)
            .then_some(state)
    } else {
        None
    };
    let mut checkpoint = if let Some(state) = pending {
        let source = docker::source_id(&state)?;
        // If the original was independently restarted after a failed migration,
        // capture its latest state instead of applying an older checkpoint.
        if current
            .as_ref()
            .is_some_and(|container| container.id == source && container.running)
        {
            prepare(
                runtime,
                record,
                options,
                current.as_ref().expect("source was found"),
            )?
        } else {
            state
        }
    } else {
        match current {
            Some(ref container)
                if container.policy.as_deref() != Some(docker::policy_digest()?.as_str()) =>
            {
                prepare(runtime, record, options, container)?
            }
            _ => return docker::provision(record, relay, options, None),
        }
    };
    let mut driver = Driver {
        runtime,
        record,
        relay,
        options,
    };
    execute(&mut driver, &mut checkpoint)
}

fn prepare(
    runtime: &KernelRuntimeState,
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    source: &docker::Container,
) -> Result<SliceSavedStateRecord, DaemonError> {
    runtime.record_slice_audit_event(
        record,
        docker::MIGRATION_OPERATION,
        "checkpointing",
        None,
        None,
    )?;
    let checkpoint = docker::checkpoint(record, options, source)?;
    // A failed durable write never permits the source container to be renamed.
    runtime.save_slice_state_record(&record.id, checkpoint.clone())?;
    runtime.record_slice_audit_event(
        record,
        docker::MIGRATION_OPERATION,
        "checkpointed",
        None,
        None,
    )?;
    Ok(checkpoint)
}

trait MigrationDriver {
    fn stage(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError>;
    fn provision(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError>;
    fn verify(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError>;
    fn persist(
        &mut self,
        checkpoint: &mut SliceSavedStateRecord,
        status: SliceOperationStatus,
    ) -> Result<(), DaemonError>;
    fn rollback(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError>;
    fn cleanup(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError>;
    fn audit(&mut self, status: SliceOperationStatus) -> Result<(), DaemonError>;
}

fn execute(
    driver: &mut impl MigrationDriver,
    checkpoint: &mut SliceSavedStateRecord,
) -> Result<(), DaemonError> {
    if checkpoint.last_operation_status == Some(SliceOperationStatus::Completed) {
        // A cleanup retry can happen after the user has worked in the verified
        // replacement. Start/check it normally, but never roll it back to the
        // old migration checkpoint if this later start or verification fails.
        driver.provision(checkpoint)?;
        driver.verify(checkpoint)?;
        cleanup_verified(driver, checkpoint);
        return Ok(());
    }
    // Stage performs all identity checks before any mutation. An unrelated
    // container or unavailable identity must not enter rollback cleanup either.
    driver.stage(checkpoint)?;
    let result = driver
        .provision(checkpoint)
        .and_then(|()| driver.verify(checkpoint))
        .and_then(|()| driver.persist(checkpoint, SliceOperationStatus::Completed));
    match result {
        Ok(()) => {
            audit_published(driver, SliceOperationStatus::Completed);
            // The successful browser is authoritative now. A cleanup failure
            // keeps a stopped rollback container and the retained checkpoint.
            cleanup_verified(driver, checkpoint);
            Ok(())
        }
        Err(cause) => {
            let rollback = driver.rollback(checkpoint);
            let persisted = driver.persist(checkpoint, SliceOperationStatus::Failed);
            if persisted.is_ok() {
                audit_published(driver, SliceOperationStatus::Failed);
            }
            Err(docker::error(format!("Chromium migration failed: {cause}; checkpoint {} retained; rollback {}; recovery record {}",
                checkpoint.id, if rollback.is_ok() { "completed; checkpoint retained" } else { "requires retry" },
                if persisted.is_ok() { "saved" } else { "requires retry" })))
        }
    }
}

fn audit_published(driver: &mut impl MigrationDriver, status: SliceOperationStatus) {
    if let Err(error) = driver.audit(status) {
        // Terminal audit is diagnostic. The checkpoint and its active
        // reference have already committed; this cannot authorize rollback.
        tracing::warn!(%error, "Chromium migration terminal audit could not be recorded");
    }
}

fn cleanup_verified(driver: &mut impl MigrationDriver, checkpoint: &SliceSavedStateRecord) {
    if let Err(error) = driver.cleanup(checkpoint) {
        tracing::warn!(%error, "verified Chromium migration retained its old container");
    }
}

struct Driver<'a> {
    runtime: &'a KernelRuntimeState,
    record: &'a SliceRecord,
    relay: Option<LocalDockerSliceRelay>,
    options: &'a LocalDockerSliceOptions,
}

struct Layout {
    source_id: String,
    canonical: Option<docker::Container>,
    rollback: Option<docker::Container>,
}

fn validate_layout_identity(
    source_id: &str,
    checkpoint_id: &str,
    canonical: Option<&docker::Container>,
    rollback: Option<&docker::Container>,
) -> Result<(), DaemonError> {
    if let Some(original) = rollback {
        if original.id != source_id || original.running {
            return Err(docker::error(
                "rollback name refers to an unexpected or running container; left untouched",
            ));
        }
    }
    if let Some(candidate) = canonical {
        if candidate.id == source_id {
            if candidate.running || rollback.is_some() {
                return Err(docker::error(
                    "original container changed during migration; left untouched",
                ));
            }
        } else if candidate.migration.as_deref() != Some(checkpoint_id) {
            return Err(docker::error(
                "canonical name refers to an unrelated container; left untouched",
            ));
        }
    }
    Ok(())
}

impl Driver<'_> {
    fn layout(&self, checkpoint: &SliceSavedStateRecord) -> Result<Layout, DaemonError> {
        if checkpoint.source_slice_id != self.record.id
            || checkpoint.last_operation.as_deref() != Some(docker::MIGRATION_OPERATION)
        {
            return Err(docker::error(
                "checkpoint does not belong to this slice migration",
            ));
        }
        let source_id = docker::source_id(checkpoint)?;
        let canonical = docker::inspect(&docker::canonical_name(self.record))?;
        let rollback = docker::inspect(&docker::rollback_name(self.record, checkpoint))?;
        for container in canonical.iter().chain(rollback.iter()) {
            docker::require_home(container, self.record)?;
        }
        validate_layout_identity(
            &source_id,
            &checkpoint.id,
            canonical.as_ref(),
            rollback.as_ref(),
        )?;
        Ok(Layout {
            source_id,
            canonical,
            rollback,
        })
    }
}

impl MigrationDriver for Driver<'_> {
    fn stage(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        let layout = self.layout(checkpoint)?;
        if let Some(original) = layout
            .canonical
            .filter(|container| container.id == layout.source_id)
        {
            docker::rename(
                &original.id,
                &docker::rollback_name(self.record, checkpoint),
            )?;
        }
        self.runtime.record_slice_audit_event(
            self.record,
            docker::MIGRATION_OPERATION,
            "replacing",
            None,
            None,
        )?;
        Ok(())
    }

    fn provision(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        self.layout(checkpoint)?;
        docker::provision(
            self.record,
            self.relay.clone(),
            self.options,
            Some(checkpoint),
        )
    }

    fn verify(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        let layout = self.layout(checkpoint)?;
        let candidate = layout
            .canonical
            .ok_or_else(|| docker::error("replacement container disappeared"))?;
        if !candidate.running || candidate.migration.as_deref() != Some(checkpoint.id.as_str()) {
            return Err(docker::error(
                "replacement container was not created by this migration",
            ));
        }
        self.runtime.record_slice_audit_event(
            self.record,
            docker::MIGRATION_OPERATION,
            "verifying",
            None,
            None,
        )?;
        docker::verify(self.record, self.options)
    }

    fn persist(
        &mut self,
        checkpoint: &mut SliceSavedStateRecord,
        status: SliceOperationStatus,
    ) -> Result<(), DaemonError> {
        docker::mark_checkpoint(checkpoint, status.clone())?;
        self.runtime
            .save_slice_state_record(&self.record.id, checkpoint.clone())?;
        Ok(())
    }

    fn audit(&mut self, status: SliceOperationStatus) -> Result<(), DaemonError> {
        self.runtime.record_slice_audit_event(
            self.record,
            docker::MIGRATION_OPERATION,
            if status == SliceOperationStatus::Completed {
                "completed"
            } else {
                "failed"
            },
            None,
            None,
        )?;
        Ok(())
    }

    fn rollback(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        let layout = self.layout(checkpoint)?;
        if let Some(container) = layout.canonical {
            if container.id == layout.source_id {
                return Ok(());
            }
            docker::remove_candidate(&container, self.record, checkpoint)?;
        }
        // Recheck after candidate removal, before restoring a shared named volume.
        let layout = self.layout(checkpoint)?;
        if layout.canonical.is_some() {
            return Err(docker::error("canonical name changed before home rollback"));
        }
        docker::restore_home(self.record, self.options, checkpoint)?;
        if let Some(original) = layout.rollback {
            docker::rename(&original.id, &docker::canonical_name(self.record))?;
        }
        Ok(())
    }

    fn cleanup(&mut self, checkpoint: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        let layout = self.layout(checkpoint)?;
        if let Some(original) = layout.rollback {
            docker::remove_original(&original, &layout.source_id)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
