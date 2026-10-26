//! Kernel-private derived native code. A cache entry is neither an installed
//! release nor publisher trust. Only a reverified release lease supplies input.
use super::{
    aot_key::{AotCacheKey, AotProvenance},
    ReleaseStoreError,
};
use crate::private_fs::{Dir, FsError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const MAGIC: &[u8; 8] = b"CXAOT001";
const MAX_PROVENANCE: usize = 16 * 1024;
pub const MAX_ARTIFACT_BYTES: usize = 256 * 1024 * 1024;
type Result<T> = std::result::Result<T, AotError>;

#[derive(Debug, thiserror::Error)]
pub enum AotError {
    #[error("app_aot_store: {0}")]
    Store(#[from] ReleaseStoreError),
    #[error("app_aot_io: {0}")]
    Io(#[from] std::io::Error),
    #[error("app_aot_integrity: evict the damaged cache entry and retry")]
    Integrity,
    #[error("app_aot_incompatible: rebuild from the retained component")]
    Incompatible,
    #[error("app_aot_limit")]
    Limit,
    #[error("app_aot_busy: retry activation")]
    Busy,
    #[error("app_aot_compile: {0}")]
    Compile(String),
}
impl From<FsError> for AotError {
    fn from(error: FsError) -> Self {
        Self::Store(error.into())
    }
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Constructed only after reading the signed bytes through a release lease.
pub struct VerifiedComponent {
    bytes: Vec<u8>,
    digest: String,
    release: String,
    world: String,
}
impl VerifiedComponent {
    pub(super) fn new(bytes: Vec<u8>, digest: String, release: String, world: String) -> Self {
        Self {
            bytes,
            digest,
            release,
            world,
        }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn world(&self) -> &str {
        &self.world
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AotRuntimeIdentity {
    pub wasmtime_version: String,
    pub worker_build: String,
    pub engine_config_digest: String,
    pub target_triple: String,
    pub cpu_feature_policy: String,
}
impl AotRuntimeIdentity {
    pub fn key(&self, component_digest: &str) -> AotCacheKey {
        AotCacheKey {
            component_digest: component_digest.into(),
            wasmtime_version: self.wasmtime_version.clone(),
            worker_build: self.worker_build.clone(),
            engine_config_digest: self.engine_config_digest.clone(),
            target_triple: self.target_triple.clone(),
            cpu_feature_policy: self.cpu_feature_policy.clone(),
        }
    }
}

/// Only validated bytes may cross the worker's unsafe deserialization boundary.
pub struct VerifiedAot {
    provenance: AotProvenance,
    bytes: Vec<u8>,
}
impl VerifiedAot {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn provenance(&self) -> &AotProvenance {
        &self.provenance
    }

    /// The worker calls this only on the supervisor's inherited cache descriptor,
    /// never on an uploaded path. Rechecks bytes in memory before deserialization.
    pub fn from_supervisor_descriptor(mut file: File, expected: &AotCacheKey) -> Result<Self> {
        file.seek(SeekFrom::Start(0))?;
        let size = file.metadata()?.len();
        if size > (MAX_ARTIFACT_BYTES + MAX_PROVENANCE + 12) as u64 {
            return Err(AotError::Limit);
        }
        let mut header = [0; 12];
        file.read_exact(&mut header)?;
        if &header[..8] != MAGIC {
            return Err(AotError::Integrity);
        }
        let length = u32::from_be_bytes(header[8..].try_into().unwrap()) as usize;
        if length == 0 || length > MAX_PROVENANCE {
            return Err(AotError::Integrity);
        }
        let mut encoded = vec![0; length];
        file.read_exact(&mut encoded)?;
        let provenance: AotProvenance =
            serde_json::from_slice(&encoded).map_err(|_| AotError::Integrity)?;
        if !provenance.usable_by(expected) {
            return Err(AotError::Incompatible);
        }
        if !valid_digest(&provenance.source_release_digest) {
            return Err(AotError::Integrity);
        }
        let mut bytes = Vec::new();
        file.take(MAX_ARTIFACT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty()
            || bytes.len() > MAX_ARTIFACT_BYTES
            || digest(&bytes) != provenance.artifact_digest
        {
            return Err(AotError::Integrity);
        }
        Ok(Self { provenance, bytes })
    }
}

pub struct PreparedAot {
    pub file: File,
    pub key: AotCacheKey,
    pub cache_hit: bool,
}

pub struct AotCache {
    root: Dir,
}
impl AotCache {
    fn read_entry(&self, name: &OsStr) -> std::result::Result<File, FsError> {
        use std::os::unix::fs::MetadataExt;
        let file = self.root.read_file(name, false)?;
        if file.metadata()?.mode() & 0o7777 != 0o600 {
            return Err(FsError::UnsafeEntry);
        }
        Ok(file)
    }
    pub fn open_or_create(database: &Path) -> Result<Self> {
        let release_root = super::root_for_database(database)?;
        let name = release_root
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or(AotError::Integrity)?;
        let root = Dir::open_or_create_private_child(
            release_root.parent().ok_or(AotError::Integrity)?,
            OsStr::new(&name.replace("app-releases-", "app-aot-")),
        )?;
        Ok(Self { root })
    }
    pub fn prepare(
        &self,
        component: &VerifiedComponent,
        identity: &AotRuntimeIdentity,
        compile: impl FnOnce(&VerifiedComponent) -> Result<Vec<u8>>,
    ) -> Result<PreparedAot> {
        if digest(component.bytes()) != component.digest {
            return Err(AotError::Integrity);
        }
        let lock = match self.root.create_private_file(OsStr::new(".lock")) {
            Ok(file) => file,
            Err(FsError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.root.open_private_file(OsStr::new(".lock"))?
            }
            Err(error) => return Err(error.into()),
        };
        if !crate::private_fs::try_lock_file(&lock)? {
            return Err(AotError::Busy);
        }
        self.prepare_locked(component, identity, compile)
    }
    fn prepare_locked(
        &self,
        component: &VerifiedComponent,
        identity: &AotRuntimeIdentity,
        compile: impl FnOnce(&VerifiedComponent) -> Result<Vec<u8>>,
    ) -> Result<PreparedAot> {
        let key = identity.key(component.digest());
        let id = key.id();
        let name = OsStr::new(&id[7..]);
        match self.read_entry(name) {
            Ok(file) => {
                VerifiedAot::from_supervisor_descriptor(file.try_clone()?, &key)?;
                self.root.sync()?;
                return Ok(PreparedAot {
                    file,
                    key,
                    cache_hit: true,
                });
            }
            Err(FsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let bytes = compile(component)?;
        if bytes.is_empty() || bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(AotError::Limit);
        }
        let provenance = AotProvenance {
            key: key.clone(),
            source_release_digest: component.release.clone(),
            artifact_digest: digest(&bytes),
        };
        let metadata =
            serde_json_canonicalizer::to_vec(&provenance).map_err(|_| AotError::Integrity)?;
        if metadata.len() > MAX_PROVENANCE {
            return Err(AotError::Limit);
        }
        let mut entry = Vec::with_capacity(12 + metadata.len() + bytes.len());
        entry.extend_from_slice(MAGIC);
        entry.extend_from_slice(&(metadata.len() as u32).to_be_bytes());
        entry.extend_from_slice(&metadata);
        entry.extend_from_slice(&bytes);
        self.root.atomic_replace(name, &entry)?;
        let file = self.read_entry(name)?;
        VerifiedAot::from_supervisor_descriptor(file.try_clone()?, &key)?;
        Ok(PreparedAot {
            file,
            key,
            cache_hit: false,
        })
    }
    /// Removing derived code cannot remove the separately leased release or data.
    pub fn evict(&self, key: &AotCacheKey) -> Result<()> {
        self.root.remove_file(OsStr::new(&key.id()[7..]))?;
        self.root.sync()?;
        Ok(())
    }
}
fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests;
