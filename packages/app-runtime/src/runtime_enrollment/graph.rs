//! Exact signed graph validation shared by enrollment reads and the installer.
use super::{
    decode_hex, filesystem, manifest::Inventory, EnrollmentError, Result, INVENTORY, LEASE,
    MAX_BUNDLE, MAX_MANIFEST, SIGNATURE,
};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::File};

pub(super) enum Modes {
    Installed,
    Source,
}
pub(super) struct VerifiedGraph {
    pub root: filesystem::Directory,
    pub lease: File,
    pub files: BTreeMap<String, File>,
    pub inventory: Inventory,
    pub inventory_file: File,
    pub signature_file: File,
    pub inventory_bytes: Vec<u8>,
    pub signature_bytes: Vec<u8>,
}
impl VerifiedGraph {
    pub fn open(
        root: filesystem::Directory,
        key: &VerifyingKey,
        digest: &str,
        target: &str,
        modes: Modes,
    ) -> Result<Self> {
        let exact = matches!(modes, Modes::Installed);
        let lease = root.file(LEASE, if exact { Some(0o444) } else { None })?;
        filesystem::shared_lease(&lease)?;
        let mut inventory_file = root.file(INVENTORY, if exact { Some(0o444) } else { None })?;
        let mut signature_file = root.file(SIGNATURE, if exact { Some(0o444) } else { None })?;
        let inventory_bytes = filesystem::small(&mut inventory_file, MAX_MANIFEST)?;
        if format!("{:x}", Sha256::digest(&inventory_bytes)) != digest {
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
        inventory.validate(target)?;
        let expected: Vec<_> = inventory
            .files
            .iter()
            .map(|entry| entry.path.clone())
            .chain([INVENTORY.into(), SIGNATURE.into(), LEASE.into()])
            .collect();
        root.require_inventory(&expected)?;
        let mut files = BTreeMap::new();
        let mut total = (inventory_bytes.len() + signature_bytes.len()) as u64;
        for entry in &inventory.files {
            total = total
                .checked_add(entry.size)
                .ok_or(EnrollmentError::Limit)?;
            if total > MAX_BUNDLE {
                return Err(EnrollmentError::Limit);
            }
            let mode = if exact {
                Some(if entry.executable { 0o555 } else { 0o444 })
            } else {
                None
            };
            let mut file = root.file(&entry.path, mode)?;
            filesystem::verify_file(&mut file, entry.size, &entry.sha256)?;
            files.insert(entry.path.clone(), file);
        }
        Ok(Self {
            root,
            lease,
            files,
            inventory,
            inventory_file,
            signature_file,
            inventory_bytes,
            signature_bytes,
        })
    }
}
