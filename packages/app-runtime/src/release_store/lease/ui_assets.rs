//! Signed UI files only. The eventual App-origin service supplies a once-decoded
//! relative path; this API does not parse URLs or authorize a viewer/installation.
use super::{Result, VerifiedReleaseLease};
use crate::release_store::ReleaseStoreError;
use chariox_app_package::VerifiedPackage;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, ffi::OsStr, io::Read};

pub(super) type Inventory = BTreeMap<String, (usize, [u8; 32])>;

pub(super) fn inventory(package: &VerifiedPackage<'_>) -> Inventory {
    package
        .files()
        .filter_map(|(path, bytes)| {
            path.strip_prefix("ui/")
                .map(|path| (path.into(), (bytes.len(), Sha256::digest(bytes).into())))
        })
        .collect()
}

impl VerifiedReleaseLease {
    /// Reads one exact signed path relative to `ui/`, with a caller-supplied
    /// memory bound (never more than the package's 64 MiB file ceiling).
    ///
    /// No fallback serves runtime code, schemas, an archive, a host path or an
    /// unlisted file. The held release lease prevents normal cleanup throughout
    /// the read. Descriptor-relative opens reject substituted symlinks; size and
    /// digest checks reject post-verification changes before returning bytes.
    ///
    /// This is synchronous I/O. Callers retain their bounded blocking admission
    /// until completion and must independently check current installation trust.
    pub fn read_ui_asset(&self, relative_path: &str, maximum_bytes: usize) -> Result<Vec<u8>> {
        let (size, digest) = self
            .ui_assets
            .get(relative_path)
            .ok_or(ReleaseStoreError::UnsafeEntry)?;
        if *size > maximum_bytes || *size > 64 * 1024 * 1024 {
            return Err(ReleaseStoreError::ReservationExceeded);
        }
        // Inventory keys originate in the verified archive's canonical paths,
        // not in a caller-selected filesystem traversal.
        let mut directory = self.payload.child(OsStr::new("ui"))?;
        let mut parts = relative_path.split('/').peekable();
        let name = loop {
            let part = parts.next().ok_or(ReleaseStoreError::UnsafeEntry)?;
            if parts.peek().is_none() {
                break part;
            }
            directory = directory.child(OsStr::new(part))?;
        };
        let mut file = directory.read_file(OsStr::new(name), true)?;
        if file.metadata()?.len() != *size as u64 {
            return Err(ReleaseStoreError::InvalidExisting);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(*size)
            .map_err(|_| ReleaseStoreError::ReservationExceeded)?;
        bytes.resize(*size, 0);
        file.read_exact(&mut bytes)?;
        let actual: [u8; 32] = Sha256::digest(&bytes).into();
        if file.read(&mut [0_u8; 1])? != 0 || actual != *digest {
            return Err(ReleaseStoreError::InvalidExisting);
        }
        Ok(bytes)
    }
}
