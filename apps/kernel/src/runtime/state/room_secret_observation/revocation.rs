//! MP-08/MP-10/MP-11: value-free obligations, indexed and delivered per physical slice.
use super::*;
use serde::Deserialize;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime::state) struct PendingRevocation {
    room: String,
    slice: String,
    worker_ref: String,
    created_at_ms: u64,
    #[serde(default, skip_serializing_if = "is_retire")]
    pub(super) disposition: Disposition,
}

fn is_retire(disposition: &Disposition) -> bool {
    *disposition == Disposition::Retire
}

fn slice_binding(id: &str, worker: &str, created: u64) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(id, worker, created)).expect("binding serializes"))
    )
}

impl RoomSecretObservations {
    fn revocation_root(&self) -> PathBuf {
        self.root.join("revocations")
    }

    fn slice_revocation_dir(&self, slice: &crate::slice::SliceRecord) -> PathBuf {
        self.revocation_root().join(slice_binding(
            &slice.id,
            &slice.worker_kernel_ref,
            slice.created_at_ms,
        ))
    }

    fn revocation_path(&self, pending: &PendingRevocation) -> PathBuf {
        self.revocation_root()
            .join(slice_binding(
                &pending.slice,
                &pending.worker_ref,
                pending.created_at_ms,
            ))
            .join(format!(
                "{:x}.pending-revocation",
                Sha256::digest(
                    serde_json::to_vec(&(&pending.room, pending.disposition))
                        .expect("room disposition serializes")
                )
            ))
    }

    fn delivery_lock(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<Arc<tokio::sync::Mutex<()>>, DaemonError> {
        Ok(self
            .revocation_delivery
            .lock()
            .map_err(|_| protection_error())?
            .entry(self.slice_revocation_dir(slice))
            .or_default()
            .clone())
    }

    pub(in crate::runtime::state) fn defer_revocation(
        &self,
        room: &str,
        slice: &crate::slice::SliceRecord,
        disposition: Disposition,
    ) -> Result<(), DaemonError> {
        let pending = PendingRevocation {
            room: room.into(),
            slice: slice.id.clone(),
            worker_ref: slice.worker_kernel_ref.clone(),
            created_at_ms: slice.created_at_ms,
            disposition,
        };
        // Dispositions have separate receipts: an in-flight retirement acknowledgement
        // cannot erase a newer Room deletion's wipe obligation.
        self.write_revocation(&pending)
    }

    fn write_revocation(&self, pending: &PendingRevocation) -> Result<(), DaemonError> {
        let path = self.revocation_path(pending);
        let directory = path.parent().ok_or_else(protection_error)?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .map_err(|_| protection_error())?;
        let temporary = path.with_extension(format!("revocation-write-{}", std::process::id()));
        let result = (|| {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temporary)
                .map_err(|_| protection_error())?;
            file.write_all(&serde_json::to_vec(pending).map_err(|_| protection_error())?)
                .map_err(|_| protection_error())?;
            file.sync_all().map_err(|_| protection_error())?;
            std::fs::rename(&temporary, &path).map_err(|_| protection_error())?;
            self.sync_revocations(directory)?;
            self.sync_revocations(&self.revocation_root())?;
            self.sync_revocations(&self.root)
        })();
        let _ = std::fs::remove_file(temporary);
        result
    }

    fn read_revocation(&self, path: &std::path::Path) -> Result<PendingRevocation, DaemonError> {
        use std::io::Read;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| protection_error())?;
        let metadata = file.metadata().map_err(|_| protection_error())?;
        if !metadata.is_file() || metadata.len() > 65_536 {
            return Err(protection_error());
        }
        let mut bytes = Vec::new();
        file.take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|_| protection_error())?;
        serde_json::from_slice(&bytes).map_err(|_| protection_error())
    }

    // One-time upgrade of the unshipped predecessor's flat records. New admissions
    // never scan or parse another slice's records. Invalid legacy bindings fail closed.
    pub(super) fn migrate_flat_revocations(&self) -> Result<(), DaemonError> {
        for entry in std::fs::read_dir(&self.root).map_err(|_| protection_error())? {
            let path = entry.map_err(|_| protection_error())?.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "pending-revocation")
            {
                continue;
            }
            let record = self.read_revocation(&path)?;
            let legacy = self.root.join(format!(
                "{:x}.pending-revocation",
                Sha256::digest(serde_json::to_vec(&record).map_err(|_| protection_error())?)
            ));
            if path != legacy {
                return Err(protection_error());
            }
            self.write_revocation(&record)?;
            std::fs::remove_file(path).map_err(|_| protection_error())?;
        }
        self.sync_revocations(&self.root)
    }

    pub(in crate::runtime::state) fn pending_revocations(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<Vec<PendingRevocation>, DaemonError> {
        let directory = self.slice_revocation_dir(slice);
        match std::fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.is_dir() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            _ => return Err(protection_error()),
        }
        let mut pending = Vec::new();
        for entry in std::fs::read_dir(&directory).map_err(|_| protection_error())? {
            let path = entry.map_err(|_| protection_error())?.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "pending-revocation")
            {
                continue;
            }
            let record = match self.read_revocation(&path) {
                Ok(record) => record,
                Err(_) if !path.exists() => continue, // An acknowledged record can disappear concurrently.
                Err(error) => return Err(error),
            };
            if self.revocation_path(&record) != path
                || record.slice != slice.id
                || record.worker_ref != slice.worker_kernel_ref
                || record.created_at_ms != slice.created_at_ms
            {
                return Err(protection_error());
            }
            pending.push(record);
        }
        pending.sort_by_key(|record| match record.disposition {
            Disposition::Retire => 0,
            Disposition::ResetEnvironment => 1,
            Disposition::DeleteRoom => 2,
        });
        Ok(pending)
    }

    fn acknowledge_revocation(&self, pending: &PendingRevocation) -> Result<(), DaemonError> {
        let path = self.revocation_path(pending);
        std::fs::remove_file(&path).map_err(|_| protection_error())?;
        self.sync_revocations(path.parent().ok_or_else(protection_error)?)
    }

    pub(in crate::runtime::state) fn remove_slice_revocations(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<(), DaemonError> {
        self.remove_revocation_directory(&self.slice_revocation_dir(slice))
    }

    fn remove_revocation_directory(&self, directory: &std::path::Path) -> Result<(), DaemonError> {
        match std::fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.is_dir() => {
                std::fs::remove_dir_all(directory).map_err(|_| protection_error())?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            _ => return Err(protection_error()),
        }
        self.revocation_delivery
            .lock()
            .map_err(|_| protection_error())?
            .remove(directory);
        self.sync_revocations(&self.revocation_root())
    }

    fn collect_deleted_slice_revocations(
        &self,
        slices: &[crate::slice::SliceRecord],
    ) -> Result<(), DaemonError> {
        let retained: BTreeSet<_> = slices
            .iter()
            .map(|slice| self.slice_revocation_dir(slice))
            .collect();
        let entries = match std::fs::read_dir(self.revocation_root()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(protection_error()),
        };
        for entry in entries {
            let entry = entry.map_err(|_| protection_error())?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.len() == 64
                && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                && !retained.contains(&entry.path())
            {
                self.remove_revocation_directory(&entry.path())?;
            }
        }
        Ok(())
    }

    fn sync_revocations(&self, directory: &std::path::Path) -> Result<(), DaemonError> {
        std::fs::File::open(directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| protection_error())
    }
}

impl KernelRuntimeState {
    // Caller already owns slice admission. Do not acquire a second route slot.
    pub(crate) async fn settle_slice_observation_revocations(
        &self,
        slice_id: &str,
    ) -> Result<(), DaemonError> {
        let slice = self.owned.slice_store.resolve(slice_id)?;
        let _guard = self
            .owned
            .room_secret_observations
            .delivery_lock(&slice)?
            .lock_owned()
            .await;
        self.deliver_slice_observation_revocations(slice_id).await
    }

    async fn deliver_slice_observation_revocations(
        &self,
        slice_id: &str,
    ) -> Result<(), DaemonError> {
        let store = &self.owned.room_secret_observations;
        let slice = self.owned.slice_store.resolve(slice_id)?;
        // Teardown may have crashed after durable Room deletion but before queuing
        // its wipe. Reconstruct that obligation from the orphaned slice binding.
        if let Some(room) = slice
            .environment_session_id
            .as_deref()
            .filter(|room| self.owned.session_store.get_session(room).is_err())
        {
            store.defer_revocation(room, &slice, Disposition::DeleteRoom)?;
        }
        let pending = store.pending_revocations(&slice)?;
        if pending.is_empty() {
            return Ok(());
        }
        if slice.status != crate::slice::SliceStatus::Running {
            return Err(pending_revocation_error());
        }
        let config = self.owned.config_projection.snapshot();
        let config = config.slice_relay_override(&slice).unwrap_or(config);
        let target = chariox_relay::protocol::ClientTarget {
            daemon_id: slice.worker_kernel_id.clone(),
            daemon_alias: slice
                .worker_kernel_id
                .is_none()
                .then(|| slice.worker_kernel_ref.clone()),
        };
        for record in pending {
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                self.send_room_slice_peer_request_unchecked(
                    &config,
                    target.clone(),
                    crate::transport::relay_peer::RelayPeerRequest::RoomBrowserController {
                        session_id: record.room.clone(),
                        slice_id: slice.id.clone(),
                        command: Command::ClearSecretObservation {
                            disposition: record.disposition,
                        },
                    },
                    std::time::Duration::from_secs(1),
                ),
            )
            .await
            .map_err(|_| pending_revocation_error())?
            .map_err(|_| pending_revocation_error())?;
            if !matches!(response, crate::transport::relay_peer::RelayPeerResponse::RoomBrowserController {
                session_id, slice_id, result: Response::SecretObservationCleared,
            } if session_id == record.room && slice_id == record.slice)
            {
                return Err(pending_revocation_error());
            }
            store.acknowledge_revocation(&record)?;
        }
        Ok(())
    }

    pub(crate) fn retry_slice_observation_revocations(&self, slice_id: &str) {
        let Ok(slice) = self.owned.slice_store.resolve(slice_id) else {
            return;
        };
        if slice.status != crate::slice::SliceStatus::Running {
            return;
        }
        let store = &self.owned.room_secret_observations;
        let orphan = slice
            .environment_session_id
            .as_deref()
            .is_some_and(|room| self.owned.session_store.get_session(room).is_err());
        if !orphan
            && store
                .pending_revocations(&slice)
                .is_ok_and(|records| records.is_empty())
        {
            return;
        }
        let Ok(lock) = store.delivery_lock(&slice) else {
            return;
        };
        let Ok(guard) = lock.try_lock_owned() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let runtime = self.clone();
        handle.spawn(async move {
            let _guard = guard;
            if runtime
                .deliver_slice_observation_revocations(&slice.id)
                .await
                .is_err()
            {
                tracing::debug!("MP-08/MP-10/MP-11: slice observation obligation remains pending");
            }
        });
    }

    pub(crate) fn retry_pending_slice_observation_revocations(&self) {
        let slices = self.list_slices();
        if self
            .owned
            .room_secret_observations
            .collect_deleted_slice_revocations(&slices)
            .is_err()
        {
            tracing::warn!(
                "MP-08/MP-10/MP-11: deleted-slice observation obligation cleanup remains pending"
            );
        }
        for slice in slices {
            self.retry_slice_observation_revocations(&slice.id);
        }
    }
}

fn pending_revocation_error() -> DaemonError {
    DaemonError::LocalTransport { operation: "room.secret_observation.revocation_pending",
        message: "Room observation revocation is pending: the bound slice must acknowledge it before new work is admitted. Delivery resumes automatically when the worker is reachable.".into() }
}
