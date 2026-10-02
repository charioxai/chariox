//! Restore uses the same foreground installation gate as start/update/stop.
//! Native storage preparation is worker-free when the installation is stopped.
use super::*;
use crate::local::{AppRequestErrorCode as Error, RestoreAppDataSnapshotRequest};
use crate::runtime::app_snapshot_restore as snapshot_restore;
impl AppLifecycleService {
    pub(crate) fn restore_snapshot_blocking(
        &self,
        owner: &str,
        request: &RestoreAppDataSnapshotRequest,
    ) -> snapshot_restore::Result<()> {
        if self.0.stopped.load(Ordering::Acquire) {
            return Err(Error::Busy);
        }
        let generation: u64 = request
            .expected_generation
            .parse()
            .map_err(|_| Error::InvalidRequest)?;
        if generation == 0 || generation.to_string() != request.expected_generation {
            return Err(Error::InvalidRequest);
        }
        let key = (owner.to_owned(), request.installation_id.clone());
        let _gate = self.0.operation(key.clone()).map_err(code)?;
        let _permit = self
            .0
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let _preparation = self
            .0
            .preparation
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        // Snapshot identity/integrity and host-space preflight precede stop;
        // authority and free space are checked again after draining.
        snapshot_restore::validate(
            &self.0.store,
            owner,
            &request.installation_id,
            generation,
            &request.snapshot_id,
        )?;
        let data = self
            .0
            .entries
            .lock()
            .map_err(|_| Error::StorageUnavailable)?
            .get(&key)
            .and_then(|entry| {
                entry
                    .control
                    .restore_data
                    .lock()
                    .ok()
                    .and_then(|data| data.clone())
            });
        self.stop_under_gate(owner, &request.installation_id, key)
            .map_err(code)?;
        let active = self
            .0
            .store
            .active_app_release(owner, &request.installation_id)
            .map_err(Error::from)?;
        let verified = active.verify().map_err(Error::from)?;
        let catalog = active.event_catalog(&verified).map_err(Error::from)?;
        if catalog.generation() != generation {
            return Err(Error::Conflict);
        }
        if let Some(data) = data {
            return snapshot_restore::restore(
                &self.0.store,
                owner,
                catalog,
                &data,
                &request.snapshot_id,
            );
        }
        #[cfg(target_os = "linux")]
        {
            use chariox_app_runtime::{
                release_store::ReleaseStore, runtime_enrollment::EnrolledRuntime,
                worker_process::PreparedWorker,
            };
            snapshot_restore::require_recovery_generation(
                &self.0.store,
                owner,
                &request.installation_id,
                generation,
            )?;
            let release = ReleaseStore::open_or_create(self.0.store.path())
                .and_then(|store| store.lease_verified(&verified, active.bytes()))
                .map_err(|_| Error::StorageUnavailable)?;
            let prepared = PreparedWorker::prepare_linux(
                EnrolledRuntime::open_installed().map_err(|_| Error::StorageUnavailable)?,
                release,
                active.binding(),
                None,
                generation,
            )
            .map_err(|_| Error::StorageUnavailable)?;
            let (_, outcome) = prepared
                .visit_private_data(|data| {
                    snapshot_restore::restore(
                        &self.0.store,
                        owner,
                        catalog,
                        data,
                        &request.snapshot_id,
                    )
                })
                .map_err(|_| Error::StorageUnavailable)?;
            outcome
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(Error::StorageUnavailable)
        }
    }
}
pub(super) fn code(error: LifecycleError) -> Error {
    match error {
        LifecycleError::Busy | LifecycleError::LiveLimit => Error::Busy,
        LifecycleError::Authority | LifecycleError::Stopped => Error::Conflict,
        _ => Error::StorageUnavailable,
    }
}
