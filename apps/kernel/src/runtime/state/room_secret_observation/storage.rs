//! MP-08/MP-10/MP-11: recover masking and scrubbing without a human clearance.
//! Sealed to the existing runtime identity, Room and purpose; no new keys.
use super::*;
use serde::Deserialize;
use std::os::unix::fs::OpenOptionsExt;

const PURPOSE: &[u8] = b"room-vault-observation-registry-v1";
const MAX_REGISTRY_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Registry {
    values: Vec<String>,
    #[serde(default)]
    retired_values: Vec<String>,
    #[serde(default)]
    stored_values: Vec<String>,
    #[serde(default)]
    vault_keys: BTreeSet<String>,
    #[serde(default)]
    provenance_known: bool,
    targets: Vec<serde_json::Value>,
    unknown: bool,
    revision: u64,
    #[serde(default)]
    history_before_ms: u64,
}
impl Drop for Registry {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.values.zeroize();
        self.retired_values.zeroize();
        self.stored_values.zeroize();
    }
}

impl RoomSecretObservations {
    pub(super) fn registry_path(&self, room: &str) -> PathBuf {
        self.marker(room).with_extension("sealed")
    }

    pub(super) fn persist_registry(
        &self,
        room: &str,
        protection: &Protection,
    ) -> Result<(), DaemonError> {
        let Some(key) = &self.identity else {
            return Ok(());
        };
        let public = crate::transport::relay_crypto::public_key_from_private_key_base64(key)
            .map_err(|_| protection_error())?;
        let registry = Registry {
            values: protection.values.iter().map(|v| v.to_string()).collect(),
            retired_values: protection
                .retired_values
                .iter()
                .map(|v| v.to_string())
                .collect(),
            stored_values: protection
                .stored_values
                .iter()
                .map(|v| v.to_string())
                .collect(),
            vault_keys: protection.vault_keys.clone(),
            provenance_known: protection.provenance_known,
            targets: protection.targets.clone(),
            unknown: protection.unknown,
            revision: protection.revision,
            history_before_ms: protection.history_before_ms,
        };
        let plaintext =
            Zeroizing::new(serde_json::to_vec(&registry).map_err(|_| protection_error())?);
        let sealed = crate::transport::relay_crypto::encrypt_payload_for_peer_bound(
            key,
            &public,
            PURPOSE,
            room.as_bytes(),
            &plaintext,
        )
        .map_err(|_| protection_error())?;
        let path = self.registry_path(room);
        let temporary = path.with_extension(format!("pending-{}", std::process::id()));
        let result = (|| {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temporary)
                .map_err(|_| protection_error())?;
            let bytes = serde_json::to_vec(&sealed).map_err(|_| protection_error())?;
            if bytes.len() as u64 > MAX_REGISTRY_BYTES {
                return Err(protection_error());
            }
            file.write_all(&bytes).map_err(|_| protection_error())?;
            file.sync_all().map_err(|_| protection_error())?;
            std::fs::rename(&temporary, &path).map_err(|_| protection_error())?;
            std::fs::File::open(&self.root)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| protection_error())
        })();
        let _ = std::fs::remove_file(temporary);
        result
    }

    // Lost values cannot scrub prior live echoes. Retire only this Room's
    // unavailable provenance; fresh provisioning clears the observation fence.
    // Replacing the damaged seal lets lifecycle operations recover without a
    // human and preserves the historical boundary across restarts.
    pub(super) fn retire_unreadable_registry(&self, room: &str) -> Protection {
        let protection = Protection {
            unknown: true,
            provenance_known: true,
            recovered_artifacts: true,
            history_before_ms: self.epoch,
            revision: self.epoch,
            ..Default::default()
        };
        tracing::warn!(room, "MP-08/MP-10/MP-11: unreadable Room observation registry retired; prior observations withheld");
        if self.persist_marker(room, &protection).is_err() {
            // The original marker/corrupt seal still fences observations after
            // restart. A storage fault must not veto another Room's Vault write.
            tracing::warn!(
                room,
                "MP-08/MP-10/MP-11: retired-unknown observation registry could not be persisted"
            );
        }
        protection
    }

    pub(super) fn restore_registry(&self, room: &str) -> Result<Option<Protection>, DaemonError> {
        let Some(key) = &self.identity else {
            return Ok(None);
        };
        use std::io::Read;
        let mut file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.registry_path(room))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(protection_error()),
        };
        let metadata = file.metadata().map_err(|_| protection_error())?;
        if !metadata.is_file() || metadata.len() > MAX_REGISTRY_BYTES {
            return Err(protection_error());
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| protection_error())?;
        let sealed = serde_json::from_slice(&bytes).map_err(|_| protection_error())?;
        let plaintext = crate::transport::relay_crypto::decrypt_payload_for_private_key_bound(
            key,
            &sealed,
            PURPOSE,
            room.as_bytes(),
        )
        .map_err(|_| protection_error())?;
        let mut registry: Registry =
            serde_json::from_slice(&plaintext.plaintext).map_err(|_| protection_error())?;
        if registry.values.len() + registry.retired_values.len() + registry.stored_values.len()
            > 256
            || registry.targets.len() > 256
            || registry.vault_keys.len() > 256
            || registry
                .values
                .iter()
                .chain(&registry.retired_values)
                .chain(&registry.stored_values)
                .any(String::is_empty)
        {
            return Err(protection_error());
        }
        let history_before_ms = if !registry.values.is_empty()
            || !registry.retired_values.is_empty()
            || registry.unknown
        {
            registry.history_before_ms.max(self.epoch)
        } else {
            registry.history_before_ms
        };
        Ok(Some(Protection {
            values: std::mem::take(&mut registry.values)
                .into_iter()
                .map(Zeroizing::new)
                .collect(),
            retired_values: std::mem::take(&mut registry.retired_values)
                .into_iter()
                .map(Zeroizing::new)
                .collect(),
            stored_values: std::mem::take(&mut registry.stored_values)
                .into_iter()
                .map(Zeroizing::new)
                .collect(),
            vault_keys: std::mem::take(&mut registry.vault_keys),
            provenance_known: registry.provenance_known,
            targets: std::mem::take(&mut registry.targets),
            unknown: registry.unknown,
            revision: registry.revision,
            recovered_artifacts: true,
            history_before_ms,
            history_boundary_bytes: None,
        }))
    }
}

impl RoomSecretObservations {
    fn history_marker(&self, room: &str) -> PathBuf {
        self.marker(room).with_extension("history")
    }

    pub(super) fn legacy_history_cutoff(&self, room: &str) -> Result<Option<u64>, DaemonError> {
        use std::io::Read;
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.history_marker(room))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(protection_error()),
        };
        let metadata = file.metadata().map_err(|_| protection_error())?;
        if !metadata.is_file() || metadata.len() > 32 {
            return Err(protection_error());
        }
        let mut text = String::new();
        file.take(33)
            .read_to_string(&mut text)
            .map_err(|_| protection_error())?;
        text.parse().map(Some).map_err(|_| protection_error())
    }

    // MP-08/MP-10/MP-11: fence only Rooms present at the first upgraded boot.
    pub(super) fn migrate_legacy_rooms(
        &self,
        recovered: BTreeSet<String>,
    ) -> Result<(), DaemonError> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.root)
            .map_err(|_| protection_error())?;
        let complete = self.root.join("migration-v1");
        match std::fs::symlink_metadata(&complete) {
            Ok(metadata) if metadata.is_file() => return Ok(()),
            Ok(_) => return Err(protection_error()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(protection_error()),
        }
        for room in recovered {
            // Separate from the protected-value marker: a lost secret registry
            // must never fall back to this clean historical state.
            use std::io::Write;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(self.history_marker(&room))
            {
                Ok(mut file) => {
                    file.write_all(self.epoch.to_string().as_bytes())
                        .map_err(|_| protection_error())?;
                    file.sync_all().map_err(|_| protection_error())?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.legacy_history_cutoff(&room)?;
                }
                Err(_) => return Err(protection_error()),
            }
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(complete)
            .map_err(|_| protection_error())?;
        file.sync_all().map_err(|_| protection_error())?;
        std::fs::File::open(&self.root)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| protection_error())
    }

    pub(in crate::runtime::state) fn apply_disposition(
        &self,
        room: &str,
        disposition: Disposition,
    ) -> Result<(), DaemonError> {
        match disposition {
            Disposition::Retire => self.forget(room),
            Disposition::ResetEnvironment => self.reset_environment(room),
            Disposition::DeleteRoom => self.delete_room(room),
        }
    }

    // Caller holds the exclusive capture/input barrier. Retired values cannot
    // resolve credentials or match Vault keys, but keep prior echoes scrubbed.
    pub(in crate::runtime::state) fn forget(&self, room: &str) -> Result<(), DaemonError> {
        let room = self.room_key(room);
        let _history_guard = self.invalidate_public_history(room)?;
        self.blocked(room, false)?;
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = rooms.get_mut(room).ok_or_else(protection_error)?;
        if protection.unknown
            && protection.values.is_empty()
            && protection.retired_values.is_empty()
            && protection.vault_keys.is_empty()
        {
            // Nothing remains to retire; retain the existing missing-value fence.
            // This also keeps public deletion available during storage faults.
            return Ok(());
        }
        protection.retired_values.append(&mut protection.values);
        protection.vault_keys.clear();
        protection.provenance_known = true;
        protection.recovered_artifacts = true;
        protection.history_before_ms = protection
            .history_before_ms
            .max(crate::session::unix_epoch_ms());
        protection.revision = protection.revision.saturating_add(1);
        if let Err(error) = self.persist_marker(room, protection) {
            protection.unknown = true;
            return Err(error);
        }
        Ok(())
    }

    // Only a verified fresh physical environment may clear a missing-value fence.
    // Retain the historical boundary even though it has no remaining live echoes.
    pub(in crate::runtime::state) fn reset_environment(
        &self,
        room: &str,
    ) -> Result<(), DaemonError> {
        let room = self.room_key(room);
        let _history_guard = self.invalidate_public_history(room)?;
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = rooms.entry(room.into()).or_insert_with(|| {
            self.initial(room).unwrap_or_else(|_| Protection {
                unknown: true,
                revision: self.epoch,
                ..Default::default()
            })
        });
        // Vault-stored values are not environment state; keep scrubbing them.
        *protection = Protection {
            stored_values: std::mem::take(&mut protection.stored_values),
            recovered_artifacts: true,
            history_before_ms: protection
                .history_before_ms
                .max(crate::session::unix_epoch_ms()),
            revision: protection.revision.saturating_add(1),
            ..Default::default()
        };
        if let Err(error) = self.persist_marker(room, protection) {
            protection.unknown = true;
            return Err(error);
        }
        Ok(())
    }

    fn remove_registry(&self, room: &str) -> Result<(), DaemonError> {
        match std::fs::remove_file(self.registry_path(room)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(protection_error()),
        }
        std::fs::File::open(&self.root)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| protection_error())
    }

    pub(in crate::runtime::state) fn delete_room(&self, room: &str) -> Result<(), DaemonError> {
        // Keep an in-memory tombstone for already admitted late callbacks.
        let room = self.room_key(room);
        let _history_guard = self.invalidate_public_history(room)?;
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        rooms.insert(
            room.into(),
            Protection {
                unknown: true,
                ..Default::default()
            },
        );
        self.remove_registry(room)?;
        match std::fs::remove_file(self.marker(room)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(protection_error()),
        }
        match std::fs::remove_file(self.history_marker(room)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(protection_error()),
        }
        std::fs::File::open(&self.root)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| protection_error())
    }
}
