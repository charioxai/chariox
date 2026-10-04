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
        if registry.values.len() > 256
            || registry.targets.len() > 256
            || registry.vault_keys.len() > 256
            || registry.values.iter().any(String::is_empty)
        {
            return Err(protection_error());
        }
        let history_before_ms = if !registry.values.is_empty() || registry.unknown {
            registry.history_before_ms.max(self.epoch)
        } else {
            registry.history_before_ms
        };
        Ok(Some(Protection {
            values: std::mem::take(&mut registry.values)
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
        }))
    }
}

impl RoomSecretObservations {
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
            self.persist_marker(
                &room,
                &Protection {
                    unknown: true,
                    ..Default::default()
                },
            )?;
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

    // Caller holds the Room's exclusive capture/input barrier. Revocation keeps
    // unknown observations closed; explicit operator clearance restores fresh
    // observations but never releases historical artifacts or cached results.
    pub(in crate::runtime::state) fn forget(
        &self,
        room: &str,
        clear_unknown: bool,
    ) -> Result<(), DaemonError> {
        let room = self.room_key(room);
        self.blocked(room, false)?;
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = rooms.get_mut(room).ok_or_else(protection_error)?;
        let replacement = Protection {
            unknown: !clear_unknown,
            recovered_artifacts: true,
            history_before_ms: crate::session::unix_epoch_ms(),
            revision: protection.revision.saturating_add(1),
            ..Default::default()
        };
        // Memory stays closed on every deletion/write failure, even when this
        // was an operator clearance rather than an ordinary revocation.
        *protection = Protection {
            unknown: true,
            ..Default::default()
        };
        self.remove_registry(room)?;
        self.persist_marker(room, &replacement)?;
        *protection = replacement;
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
        self.forget(room, false)?;
        self.remove_registry(room)?;
        std::fs::remove_file(self.marker(room)).map_err(|_| protection_error())?;
        std::fs::File::open(&self.root)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| protection_error())
    }
}
