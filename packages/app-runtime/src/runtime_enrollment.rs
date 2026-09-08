//! Read-only admission of an installed, signed App runtime. Trust comes from a
//! separately installed root-owned enrollment, never a key in an App or bundle.
//! This verifies bytes and retains installer leases; it does not establish the
//! process sandbox or replace macOS code-signature/notarization verification.
mod filesystem;
mod manifest;
#[cfg(test)]
mod tests;

use ed25519_dalek::{Signature, VerifyingKey};
use filesystem::Directory;
use manifest::Inventory;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
};

const MAX_MANIFEST: u64 = 262_144;
const MAX_BUNDLE: u64 = 536_870_912;
const INVENTORY: &str = "runtime-inventory.json";
const SIGNATURE: &str = "runtime-inventory.sig";
const LEASE: &str = ".runtime-lease";

#[derive(Debug, thiserror::Error)]
pub enum EnrollmentError {
    #[error("app_runtime_enrollment_io")]
    Io(#[from] std::io::Error),
    #[error("app_runtime_enrollment_identity")]
    Identity,
    #[error("app_runtime_enrollment_signature")]
    Signature,
    #[error("app_runtime_enrollment_contract")]
    Contract,
    #[error("app_runtime_enrollment_busy")]
    Busy,
    #[error("app_runtime_enrollment_limit")]
    Limit,
}
type Result<T> = std::result::Result<T, EnrollmentError>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Enrollment {
    schema: String,
    revision: u64,
    target: String,
    runtime_root: PathBuf,
    inventory_sha256: String,
    public_key_hex: String,
}

/// Not Clone: the concrete platform domain must own this through native reap
/// and outstanding broker drain. Cloning an FD is not an independent enrollment.
pub struct EnrolledRuntime {
    root: Directory,
    _lease: File,
    files: BTreeMap<String, File>,
    _enrollment_file: File,
    _inventory_file: File,
    _signature_file: File,
    inventory_digest: String,
    revision: u64,
    target: String,
}

impl EnrolledRuntime {
    /// Only the system installer's fixed trust location is accepted. Neither an
    /// App request nor an environment variable can supply an alternate key/path.
    pub fn open_installed() -> Result<Self> {
        #[cfg(target_os = "linux")]
        let path = Path::new("/etc/chariox/apps/runtime-enrollment.json");
        #[cfg(target_os = "macos")]
        let path =
            Path::new("/Library/Application Support/Chariox/AppRuntime/runtime-enrollment.json");
        Self::open(path, 0)
    }

    fn open(path: &Path, trusted_uid: u32) -> Result<Self> {
        let parent = Directory::open(path.parent().ok_or(EnrollmentError::Identity)?, trusted_uid)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(EnrollmentError::Identity)?;
        let mut enrollment_file = parent.file(name, None)?;
        let bytes = filesystem::small(&mut enrollment_file, 4096)?;
        let enrollment: Enrollment =
            serde_json::from_slice(&bytes).map_err(|_| EnrollmentError::Contract)?;
        if enrollment.schema != "chariox.app-runtime-enrollment.v1"
            || enrollment.revision == 0
            || enrollment.revision > i64::MAX as u64
            || enrollment.target != manifest::target()?
            || !manifest::hex(&enrollment.inventory_sha256, 64)
        {
            return Err(EnrollmentError::Contract);
        }
        let key = VerifyingKey::from_bytes(&decode_hex::<32>(&enrollment.public_key_hex)?)
            .map_err(|_| EnrollmentError::Signature)?;
        let root = Directory::open(&enrollment.runtime_root, trusted_uid)?;
        let lease = root.file(LEASE, Some(0o444))?;
        filesystem::shared_lease(&lease)?;
        let mut inventory_file = root.file(INVENTORY, Some(0o444))?;
        let mut signature_file = root.file(SIGNATURE, Some(0o444))?;
        let inventory_bytes = filesystem::small(&mut inventory_file, MAX_MANIFEST)?;
        if format!("{:x}", Sha256::digest(&inventory_bytes)) != enrollment.inventory_sha256 {
            return Err(EnrollmentError::Identity);
        }
        let signature_bytes = filesystem::small(&mut signature_file, 128)?;
        let signature = Signature::from_bytes(&decode_hex::<64>(
            std::str::from_utf8(&signature_bytes).map_err(|_| EnrollmentError::Signature)?,
        )?);
        key.verify_strict(&inventory_bytes, &signature)
            .map_err(|_| EnrollmentError::Signature)?;
        let inventory: Inventory =
            serde_json::from_slice(&inventory_bytes).map_err(|_| EnrollmentError::Contract)?;
        inventory.validate(&enrollment.target)?;
        let expected: Vec<_> = inventory
            .files
            .iter()
            .map(|entry| entry.path.clone())
            .chain([INVENTORY.into(), SIGNATURE.into(), LEASE.into()])
            .collect();
        root.require_inventory(&expected)?;
        let mut files = BTreeMap::new();
        let mut total = inventory_bytes.len() as u64;
        for entry in inventory.files {
            total = total
                .checked_add(entry.size)
                .ok_or(EnrollmentError::Limit)?;
            if total > MAX_BUNDLE {
                return Err(EnrollmentError::Limit);
            }
            let mut file = root.file(
                &entry.path,
                Some(if entry.executable { 0o555 } else { 0o444 }),
            )?;
            filesystem::verify_file(&mut file, entry.size, &entry.sha256)?;
            files.insert(entry.path, file);
        }
        // Re-read held trust input after bounded hashing. The installer's change
        // protocol retains exclusive ownership of this generation's lease; it
        // publishes a new generation/enrollment instead of mutating old bytes.
        if filesystem::small(&mut enrollment_file, 4096)? != bytes {
            return Err(EnrollmentError::Identity);
        }
        Ok(Self {
            root,
            _lease: lease,
            files,
            _enrollment_file: enrollment_file,
            _inventory_file: inventory_file,
            _signature_file: signature_file,
            inventory_digest: enrollment.inventory_sha256,
            revision: enrollment.revision,
            target: enrollment.target,
        })
    }

    pub fn inventory_digest(&self) -> &str {
        &self.inventory_digest
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn target(&self) -> &str {
        &self.target
    }
    /// Descriptor-only access for the platform provisioner. Executing a path
    /// from this lease still requires that provisioner's observed sandbox.
    pub(crate) fn file(&self, name: &str) -> Option<&File> {
        self.files.get(name)
    }
    pub(crate) fn root(&self) -> &File {
        &self.root.file
    }
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N]> {
    if !manifest::hex(value, N * 2) {
        return Err(EnrollmentError::Signature);
    }
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| EnrollmentError::Signature)?;
    }
    Ok(bytes)
}
