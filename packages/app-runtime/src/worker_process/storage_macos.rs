//! Kernel-private APFS storage preparation. All operations and Drop are blocking.
//! The caller must quiesce/reap the previous worker before prepare/release. This
//! component provides fixed private volume capacity, not signed runtime trust,
//! App authorization, snapshot/rollback semantics, or aggregate runtime admission.

mod commands;
mod identity;
mod journal;
#[cfg(test)]
mod tests;
mod volume;

use crate::private_fs::{Dir, FsError};
use identity::FileIdentity;
use journal::{Image, Journal};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::File,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum Error {
    #[error("app_storage_io")]
    Io,
    #[error("app_storage_identity")]
    Identity,
    #[error("app_storage_busy")]
    Busy,
    #[error("app_storage_capacity")]
    Capacity,
    #[error("app_storage_metadata")]
    Metadata,
    #[error("app_storage_command")]
    Command,
    #[error("app_storage_command_timeout")]
    CommandTimeout,
    #[error("app_storage_command_output")]
    CommandOutput,
    #[error("app_storage_recovery_required")]
    RecoveryRequired,
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}
impl From<FsError> for Error {
    fn from(_: FsError) -> Self {
        Self::Identity
    }
}
type Result<T> = std::result::Result<T, Error>;

const MAX_INSTALLATIONS: usize = 64;
const MAX_RESERVED_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const HOST_RESERVE: u64 = 8 * 1024 * 1024 * 1024;
const CAPACITIES: [u64; 2] = [512 * 1024 * 1024, 64 * 1024 * 1024];

pub(super) struct StorageRoot {
    dir: Dir,
    path: PathBuf,
}
impl StorageRoot {
    /// `path` is the installer/kernel-owned storage root, never an App parameter.
    pub fn open(path: &Path) -> Result<Self> {
        let dir = Dir::open_private(path)?;
        if path.as_os_str().as_encoded_bytes().len() > 700 {
            return Err(Error::Identity);
        }
        Ok(Self {
            dir,
            path: path.to_owned(),
        })
    }

    pub fn prepare(
        &self,
        owner: &str,
        installation: &str,
        generation: u64,
    ) -> Result<MountedStorage> {
        self.prepare_with_capacities(owner, installation, generation, CAPACITIES)
    }

    /// Kernel startup/failed-preparation recovery after all prior workers are
    /// quiescent. Never deletes an image or trusts a remembered device number.
    pub fn recover_all_blocking(&self) -> Result<()> {
        let global = Dir::open_private(&self.path)?;
        if FileIdentity::of(&global.0)? != FileIdentity::of(&self.dir.0)? {
            return Err(Error::Identity);
        }
        if !global.try_lock()? {
            return Err(Error::Busy);
        }
        for name in self.dir.entries(MAX_INSTALLATIONS)? {
            installation_name(&name)?;
            let dir = self.dir.child(&name)?;
            if !dir.try_lock()? {
                return Err(Error::Busy);
            }
            journal::recover_temporaries(&dir)?;
            if let Some(journal) = journal::load(&dir)? {
                let mut storage = MountedStorage {
                    root: dir,
                    path: self.path.join(&name),
                    journal,
                    images: [None, None],
                    mounted: [None, None],
                    released: false,
                    cleanup_attempted: false,
                    deadline: std::time::Instant::now(),
                };
                storage.release_blocking()?;
            } else {
                // An interrupted first mkdir has no image creation authority yet.
                // Remove only empty, same-filesystem private mount directories.
                for child in dir.entries(2)? {
                    if ![OsStr::new("data"), OsStr::new("tmp")].contains(&child.as_os_str()) {
                        return Err(Error::RecoveryRequired);
                    }
                    let mount = private_mount(&dir, child.to_str().ok_or(Error::Identity)?)?;
                    if mount.0.metadata()?.dev() != dir.0.metadata()?.dev() {
                        return Err(Error::Identity);
                    }
                    FileIdentity::of(&mount.0)?.require(&dir, &child, &mount.0)?;
                    dir.remove_directory(&child)?;
                }
                dir.sync()?;
                FileIdentity::of(&dir.0)?.require(&self.dir, &name, &dir.0)?;
                self.dir.remove_directory(&name)?;
                self.dir.sync()?;
            }
        }
        Ok(())
    }

    fn prepare_with_capacities(
        &self,
        owner: &str,
        installation: &str,
        generation: u64,
        capacities: [u64; 2],
    ) -> Result<MountedStorage> {
        journal::identifier(owner)?;
        journal::identifier(installation)?;
        if generation == 0 || generation > i64::MAX as u64 {
            return Err(Error::Identity);
        }
        let global = Dir::open_private(&self.path)?;
        if FileIdentity::of(&global.0)? != FileIdentity::of(&self.dir.0)? {
            return Err(Error::Identity);
        }
        if !global.try_lock()? {
            return Err(Error::Busy);
        }
        let digest = Sha256::digest(format!("{owner}\0{installation}").as_bytes());
        let name = format!("installation-{digest:x}");
        self.check_capacity(&name, capacities)?;
        let dir = Dir::open_or_create_private_child(&self.path, OsStr::new(&name))?;
        if !dir.try_lock()? {
            return Err(Error::Busy);
        }
        journal::recover_temporaries(&dir)?;
        let path = self.path.join(name);
        let prior = journal::load(&dir)?;
        let journal = if let Some(journal) = prior {
            if journal.owner != owner
                || journal.installation != installation
                || journal.generation > generation
                || journal.images.each_ref().map(|image| image.capacity) != capacities
            {
                return Err(Error::Identity);
            }
            journal
        } else {
            // A previously interrupted mkdir is harmless; only these empty,
            // private mount directories are admitted before the first journal.
            for name in dir.entries(4)? {
                if ![OsStr::new("data"), OsStr::new("tmp")].contains(&name.as_os_str()) {
                    return Err(Error::Identity);
                }
            }
            let mounts = ["data", "tmp"].map(|role| private_mount(&dir, role));
            let [data, tmp] = mounts;
            Journal {
                schema: "chariox.app-storage.v1".into(),
                owner: owner.into(),
                installation: installation.into(),
                generation,
                pending_recovery: true,
                images: [
                    journal::image("data", capacities[0], FileIdentity::of(&data?.0)?),
                    journal::image("tmp", capacities[1], FileIdentity::of(&tmp?.0)?),
                ],
            }
        };
        journal.save(&dir)?;
        global.sync()?;
        let mut storage = MountedStorage {
            root: dir,
            path,
            journal,
            images: [None, None],
            mounted: [None, None],
            released: false,
            cleanup_attempted: false,
            deadline: std::time::Instant::now(),
        };
        // Recovery always detaches any prior mapping first. Reusing a remembered
        // /dev identifier or an existing mount without rediscovery is forbidden.
        storage.release_blocking()?;
        storage.journal.generation = generation;
        storage.journal.pending_recovery = true;
        storage.journal.save(&storage.root)?;
        storage.released = false;
        storage.cleanup_attempted = false;
        storage.prepare_volumes()?;
        Ok(storage)
    }

    fn check_capacity(&self, selected: &str, capacities: [u64; 2]) -> Result<()> {
        let mut total = 0_u64;
        let mut unallocated = 0_u64;
        let entries = self.dir.entries(MAX_INSTALLATIONS)?;
        let mut existing = false;
        let mut reserved = false;
        for entry in &entries {
            let value = installation_name(entry)?;
            if value == selected {
                existing = true;
            }
            let dir = self.dir.child(entry)?;
            if let Some(journal) = journal::load(&dir)? {
                if value == selected {
                    reserved = true;
                }
                for image in &journal.images {
                    total = total.checked_add(image.reserved()).ok_or(Error::Capacity)?;
                    let allocated = match dir.read_file(OsStr::new(&image.image), false) {
                        Ok(file) => {
                            let metadata = file.metadata()?;
                            if metadata.len() > image.reserved() {
                                return Err(Error::Identity);
                            }
                            metadata.blocks().saturating_mul(512)
                        }
                        Err(FsError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => 0,
                        Err(e) => return Err(e.into()),
                    };
                    unallocated = unallocated
                        .checked_add(image.reserved().saturating_sub(allocated))
                        .ok_or(Error::Capacity)?;
                }
            } else if value != selected {
                return Err(Error::RecoveryRequired);
            }
        }
        if !reserved {
            if !existing && entries.len() == MAX_INSTALLATIONS {
                return Err(Error::Capacity);
            }
            let additional: u64 = capacities
                .iter()
                .map(|v| v + journal::METADATA_ALLOWANCE)
                .sum();
            total = total.checked_add(additional).ok_or(Error::Capacity)?;
            unallocated = unallocated.checked_add(additional).ok_or(Error::Capacity)?;
        }
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
        if unsafe { libc::fstatfs(self.dir.0.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
            return Err(Error::Io);
        }
        let stat = unsafe { stat.assume_init() };
        let available = stat.f_bavail.saturating_mul(stat.f_bsize as u64);
        require_capacity(total, unallocated, available)
    }
}

fn require_capacity(total: u64, unallocated: u64, available: u64) -> Result<()> {
    if total > MAX_RESERVED_BYTES || available.saturating_sub(unallocated) < HOST_RESERVE {
        return Err(Error::Capacity);
    }
    Ok(())
}

fn installation_name(name: &OsStr) -> Result<&str> {
    let value = name.to_str().ok_or(Error::Identity)?;
    if value.len() != 77
        || !value.starts_with("installation-")
        || !value[13..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::Identity);
    }
    Ok(value)
}

/// Held root, image and mounted-directory identities survive through worker
/// execution. Release is called only after actual worker/domain reap. If detach
/// fails, the durable journal retains recovery and capacity; no data is deleted
/// or reservation reported reclaimed. A subsequent owner must recover it first.
pub(super) struct MountedStorage {
    root: Dir,
    path: PathBuf,
    journal: Journal,
    images: [Option<File>; 2],
    mounted: [Option<Dir>; 2],
    released: bool,
    cleanup_attempted: bool,
    deadline: std::time::Instant,
}
impl MountedStorage {
    fn command_context(&self) -> commands::Context<'_> {
        commands::Context {
            lease: &self.root.0,
            deadline: self.deadline,
        }
    }
    pub fn paths(&self) -> [PathBuf; 2] {
        [self.path.join("data"), self.path.join("tmp")]
    }
    pub fn directories(&self) -> Result<[&File; 2]> {
        Ok([
            &self.mounted[0].as_ref().ok_or(Error::RecoveryRequired)?.0,
            &self.mounted[1].as_ref().ok_or(Error::RecoveryRequired)?.0,
        ])
    }
    pub fn reserved_bytes(&self) -> u64 {
        self.journal.images.iter().map(Image::reserved).sum()
    }
}
impl Drop for MountedStorage {
    fn drop(&mut self) {
        if !self.released && !self.cleanup_attempted {
            let _ = self.release_blocking();
        }
    }
}

fn private_mount(root: &Dir, role: &str) -> Result<Dir> {
    let name = OsStr::new(role);
    let child = match root.create_child(name) {
        Ok(child) => {
            if unsafe { libc::fchmod(child.0.as_raw_fd(), 0o700) } != 0 {
                return Err(Error::Io);
            }
            child
        }
        Err(FsError::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists => root.child(name)?,
        Err(e) => return Err(e.into()),
    };
    let metadata = child.0.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != 0o700
        || !child.entries(1)?.is_empty()
    {
        return Err(Error::Identity);
    }
    child.sync()?;
    root.sync()?;
    Ok(child)
}
