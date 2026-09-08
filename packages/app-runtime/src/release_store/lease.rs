//! Read-only execution provenance from the existing archive/tree verifier.
//! This lease conveys immutable package bytes, not user authorization to run.
use super::unix::{digest_name, verify_tree, ExpectedTree, ReleaseStoreError, RootMode, MAGIC};
use crate::private_fs::{same_entry, Dir};
use chariox_app_package::{Declarations, Manifest, VerifiedPackage};
use sha2::{Digest, Sha256};
use std::{ffi::OsStr, fs::File, os::fd::AsRawFd};

type Result<T> = std::result::Result<T, ReleaseStoreError>;

/// Only ReleaseStore::lease_verified can create this value. The package root
/// retains a shared lease until the native worker and every broker call drain.
/// An App, path, or mutable StagedRelease value cannot manufacture this proof.
pub struct VerifiedReleaseLease {
    root: Dir,
    payload: Dir,
    package_digest: String,
    manifest: Manifest,
    declarations: Declarations,
}
impl VerifiedReleaseLease {
    pub fn package_digest(&self) -> &str {
        &self.package_digest
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn declarations(&self) -> &Declarations {
        &self.declarations
    }
    pub(crate) fn root(&self) -> &File {
        &self.root.0
    }
    pub(crate) fn payload(&self) -> &File {
        &self.payload.0
    }
}

pub(super) fn verified(
    store: &Dir,
    package: &VerifiedPackage<'_>,
    archive: &[u8],
) -> Result<VerifiedReleaseLease> {
    let digest = package.package_digest();
    if digest != format!("sha256:{:x}", Sha256::digest(archive)) {
        return Err(ReleaseStoreError::ArchiveMismatch);
    }
    let name = OsStr::new(digest_name(digest)?);
    let root = store.child(name)?;
    if unsafe { libc::flock(root.0.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
        return Err(ReleaseStoreError::Busy);
    }
    if !same_entry(store, name, &root)? {
        return Err(ReleaseStoreError::UnsafeEntry);
    }
    let marker = format!("{MAGIC}{digest}\n");
    let tree = ExpectedTree::new(package, archive, marker.as_bytes())?;
    verify_tree(&root, &tree, RootMode::Sealed)?;
    let payload = root.child(OsStr::new("payload"))?;
    if !same_entry(store, name, &root)? {
        return Err(ReleaseStoreError::UnsafeEntry);
    }
    Ok(VerifiedReleaseLease {
        root,
        payload,
        package_digest: digest.into(),
        manifest: package.manifest().clone(),
        declarations: package.declarations().clone(),
    })
}
