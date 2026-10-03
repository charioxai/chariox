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
    targets: Vec<serde_json::Value>,
    unknown: bool,
    revision: u64,
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
            targets: protection.targets.clone(),
            unknown: protection.unknown,
            revision: protection.revision,
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
            || registry.values.iter().any(String::is_empty)
        {
            return Err(protection_error());
        }
        Ok(Some(Protection {
            values: std::mem::take(&mut registry.values)
                .into_iter()
                .map(Zeroizing::new)
                .collect(),
            targets: std::mem::take(&mut registry.targets),
            unknown: registry.unknown,
            revision: registry.revision,
            recovered_artifacts: true,
        }))
    }
}
