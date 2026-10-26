//! Read-only execution provenance from the existing archive/tree verifier.
//! This lease conveys immutable package bytes, not user authorization to run.
use super::unix::{digest_name, verify_tree, ExpectedTree, ReleaseStoreError, RootMode, MAGIC};
use crate::private_fs::{same_entry, Dir};
use chariox_app_package::{Declarations, ReleaseManifest, VerifiedApp};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
};

type Result<T> = std::result::Result<T, ReleaseStoreError>;

mod ui_assets;

/// Held stored bytes only. The caller must verify the signature using current
/// enrolled trust, then obtain VerifiedReleaseLease from the complete tree.
pub struct StoredReleaseArchive {
    _root: Dir,
    file: File,
    digest: String,
}
impl StoredReleaseArchive {
    pub fn read_bytes(&mut self) -> Result<Vec<u8>> {
        const LIMIT: u64 = 128 * 1024 * 1024;
        let size = self.file.metadata()?.len();
        if size == 0 || size > LIMIT {
            return Err(ReleaseStoreError::ReservationExceeded);
        }
        let capacity = usize::try_from(size).map_err(|_| ReleaseStoreError::ReservationExceeded)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| ReleaseStoreError::ReservationExceeded)?;
        bytes.resize(capacity, 0);
        self.file.seek(SeekFrom::Start(0))?;
        self.file.read_exact(&mut bytes)?;
        if self.file.read(&mut [0_u8; 1])? != 0
            || format!("sha256:{:x}", Sha256::digest(&bytes)) != self.digest
        {
            return Err(ReleaseStoreError::ArchiveMismatch);
        }
        Ok(bytes)
    }
}
pub(super) fn stored_archive(store: &Dir, digest: &str) -> Result<StoredReleaseArchive> {
    let name = OsStr::new(digest_name(digest)?);
    let root = store.child(name)?;
    if unsafe { libc::flock(root.0.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
        return Err(ReleaseStoreError::Busy);
    }
    let metadata = root.0.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != 0o500
        || !same_entry(store, name, &root)?
    {
        return Err(ReleaseStoreError::UnsafeEntry);
    }
    let file = root.read_file(OsStr::new("envelope.cxapp"), true)?;
    if file.metadata()?.mode() & 0o7777 != 0o400 || !same_entry(store, name, &root)? {
        return Err(ReleaseStoreError::UnsafeEntry);
    }
    Ok(StoredReleaseArchive {
        _root: root,
        file,
        digest: digest.into(),
    })
}

/// Only ReleaseStore::lease_verified can create this value. The package root
/// retains a shared lease until the native worker and every broker call drain.
/// An App, path, or mutable StagedRelease value cannot manufacture this proof.
pub struct VerifiedReleaseLease {
    _root: Dir,
    payload: Dir,
    package_digest: String,
    manifest: ReleaseManifest,
    declarations: Declarations,
    ui_assets: ui_assets::Inventory,
    component: Option<(String, usize, String)>,
}
impl VerifiedReleaseLease {
    pub fn package_digest(&self) -> &str {
        &self.package_digest
    }
    pub fn manifest(&self) -> &ReleaseManifest {
        &self.manifest
    }
    pub fn declarations(&self) -> &Declarations {
        &self.declarations
    }
    pub(crate) fn payload(&self) -> &File {
        &self.payload.0
    }

    /// Re-read through the held directory and compare the exact signed bytes.
    /// A lease never turns a pathname into continuing executable provenance.
    pub fn verified_component(&self) -> Result<super::aot_cache::VerifiedComponent> {
        let (path, size, digest) = self
            .component
            .as_ref()
            .ok_or(ReleaseStoreError::InvalidExisting)?;
        let manifest = self
            .manifest
            .wasm()
            .ok_or(ReleaseStoreError::InvalidExisting)?;
        let mut directory = self.payload.child(OsStr::new("runtime"))?;
        let mut parts = path
            .strip_prefix("runtime/")
            .ok_or(ReleaseStoreError::UnsafeEntry)?
            .split('/')
            .peekable();
        let name = loop {
            let part = parts.next().ok_or(ReleaseStoreError::UnsafeEntry)?;
            if parts.peek().is_none() {
                break part;
            }
            directory = directory.child(OsStr::new(part))?;
        };
        let file = directory.read_file(OsStr::new(name), true)?;
        if file.metadata()?.len() != *size as u64 {
            return Err(ReleaseStoreError::InvalidExisting);
        }
        let mut bytes = Vec::new();
        file.take(*size as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() != *size || super::aot_cache::digest(&bytes) != *digest {
            return Err(ReleaseStoreError::InvalidExisting);
        }
        Ok(super::aot_cache::VerifiedComponent::new(
            bytes,
            digest.clone(),
            self.package_digest.clone(),
            manifest.world.clone(),
        ))
    }
}

pub(super) fn verified(
    store: &Dir,
    package: &impl VerifiedApp,
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
    let manifest = package.release_manifest();
    let component = manifest
        .wasm()
        .map(|manifest| {
            let (path, bytes) = package
                .files()
                .find(|(path, _)| *path == manifest.runtime.component)
                .ok_or(ReleaseStoreError::InvalidExisting)?;
            Ok::<_, ReleaseStoreError>((
                path.to_owned(),
                bytes.len(),
                super::aot_cache::digest(bytes),
            ))
        })
        .transpose()?;
    Ok(VerifiedReleaseLease {
        _root: root,
        payload,
        package_digest: digest.into(),
        manifest,
        declarations: package.declarations().clone(),
        ui_assets: ui_assets::inventory(package),
        component,
    })
}
