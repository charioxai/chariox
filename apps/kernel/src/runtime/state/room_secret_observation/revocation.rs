//! MP-08/MP-10/MP-11: durable value-free worker revocations, settled at admission.
use super::*;
use serde::Deserialize;
use std::os::unix::fs::OpenOptionsExt;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime::state) struct PendingRevocation {
    room: String,
    slice: String,
    worker_ref: String,
    created_at_ms: u64,
}

impl RoomSecretObservations {
    fn revocation_path(&self, pending: &PendingRevocation) -> PathBuf {
        let binding = serde_json::to_vec(pending).expect("string binding serializes");
        self.root
            .join(format!("{:x}.pending-revocation", Sha256::digest(binding)))
    }

    pub(in crate::runtime::state) fn defer_revocation(
        &self,
        room: &str,
        slice: &crate::slice::SliceRecord,
    ) -> Result<(), DaemonError> {
        let pending = PendingRevocation {
            room: room.into(),
            slice: slice.id.clone(),
            worker_ref: slice.worker_kernel_ref.clone(),
            created_at_ms: slice.created_at_ms,
        };
        let path = self.revocation_path(&pending);
        use std::io::Write;
        let temporary = path.with_extension(format!("revocation-write-{}", std::process::id()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temporary)
                .map_err(|_| protection_error())?;
            file.write_all(&serde_json::to_vec(&pending).map_err(|_| protection_error())?)
                .map_err(|_| protection_error())?;
            file.sync_all().map_err(|_| protection_error())?;
            std::fs::rename(&temporary, &path).map_err(|_| protection_error())
        })();
        let _ = std::fs::remove_file(temporary);
        result?;
        self.sync_revocations()
    }

    pub(in crate::runtime::state) fn pending_revocations(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<Vec<PendingRevocation>, DaemonError> {
        use std::io::Read;
        let mut pending = Vec::new();
        for entry in std::fs::read_dir(&self.root).map_err(|_| protection_error())? {
            let path = entry.map_err(|_| protection_error())?.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "pending-revocation")
            {
                continue;
            }
            let file = match std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(protection_error()),
            };
            let metadata = file.metadata().map_err(|_| protection_error())?;
            if !metadata.is_file() || metadata.len() > 65_536 {
                return Err(protection_error());
            }
            let mut bytes = Vec::new();
            file.take(65_537)
                .read_to_end(&mut bytes)
                .map_err(|_| protection_error())?;
            let record: PendingRevocation =
                serde_json::from_slice(&bytes).map_err(|_| protection_error())?;
            if self.revocation_path(&record) != path {
                return Err(protection_error());
            }
            if record.slice == slice.id
                && record.worker_ref == slice.worker_kernel_ref
                && record.created_at_ms == slice.created_at_ms
            {
                pending.push(record);
            }
        }
        Ok(pending)
    }

    fn acknowledge_revocation(&self, pending: &PendingRevocation) -> Result<(), DaemonError> {
        std::fs::remove_file(self.revocation_path(pending)).map_err(|_| protection_error())?;
        self.sync_revocations()
    }

    fn sync_revocations(&self) -> Result<(), DaemonError> {
        std::fs::File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| protection_error())
    }
}

impl KernelRuntimeState {
    // Caller already owns slice admission. Do not acquire a second route slot.
    pub(in crate::runtime::state) async fn settle_slice_observation_revocations(
        &self,
        slice_id: &str,
    ) -> Result<(), DaemonError> {
        let store = &self.owned.room_secret_observations;
        let slice = self.owned.slice_store.resolve(slice_id)?;
        if store.pending_revocations(&slice)?.is_empty() {
            return Ok(());
        }
        let _guard = store.revocation_delivery.lock().await;
        let slice = self.owned.slice_store.resolve(slice_id)?;
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
                        command: Command::ClearSecretObservation,
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
}

fn pending_revocation_error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "room.secret_observation.revocation_pending",
        message: "Room observation revocation is pending: the bound slice must acknowledge it before new work is admitted. Delivery resumes automatically when the worker is reachable.".into(),
    }
}
