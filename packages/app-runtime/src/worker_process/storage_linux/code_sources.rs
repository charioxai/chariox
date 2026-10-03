//! Mechanical root-helper admission of installer-enrolled code sources. User
//! publisher/permission policy remains in the kernel's VerifiedReleaseLease.
use super::{code_model, model::Owner, Error, Result};
use crate::{
    private_fs::{self, Dir},
    runtime_enrollment::EnrolledRuntime,
};
use sha2::{Digest, Sha256};
use std::{
    ffi::{CString, OsStr},
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Component, Path},
};

// Phase 1 kernel verification currently uses Limits::default(). The regression
// below fences these mechanical bounds against that authoritative contract.
const ARCHIVE_BYTES: usize = 128 * 1024 * 1024;
const FILE_BYTES: u64 = 64 * 1024 * 1024;
const TREE_ENTRIES: usize = 131_072;
const TREE_DEPTH: usize = 24;

pub(super) struct Sources {
    pub runtime: EnrolledRuntime,
    pub payload: Dir,
    _release: Dir,
}
impl Sources {
    pub fn open(owner: &Owner, package: &str, runtime: &str, revision: u64) -> Result<Self> {
        code_model::validate_pins(package, runtime, revision)?;
        let runtime_lease = EnrolledRuntime::open_installed().map_err(|_| Error::Identity)?;
        if runtime_lease.inventory_digest() != runtime || runtime_lease.revision() != revision {
            return Err(Error::Identity);
        }
        let (release, payload) = release(owner, package)?;
        Ok(Self {
            runtime: runtime_lease,
            payload,
            _release: release,
        })
    }
    pub fn file(&self, index: usize) -> &File {
        if index == 0 {
            &self.payload.0
        } else {
            self.runtime.root()
        }
    }
}

fn release(owner: &Owner, package: &str) -> Result<(Dir, Dir)> {
    model_package(package)?;
    let mut selected = None;
    for database in &owner.kernel_database_paths {
        let root =
            crate::release_store::root_for_database(database).map_err(|_| Error::Identity)?;
        let Some(store) = kernel_directory(&root, owner.uid)? else {
            continue;
        };
        if store.0.metadata()?.mode() & 0o7777 != 0o700 {
            return Err(Error::Identity);
        }
        let name = OsStr::new(&package[7..]);
        let release = match store.child(name) {
            Ok(release) => release,
            Err(private_fs::FsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                continue
            }
            Err(_) => return Err(Error::Identity),
        };
        if selected.is_some() {
            return Err(Error::Identity);
        }
        readonly_directory(&release, owner.uid, None)?;
        if unsafe { libc::flock(release.0.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
            return Err(Error::Busy);
        }
        let mut archive = file(&release, OsStr::new("envelope.cxapp"), owner.uid)?;
        if archive.metadata()?.len() > ARCHIVE_BYTES as u64 {
            return Err(Error::Capacity);
        }
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut total = 0usize;
        loop {
            let count = archive.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total = total.checked_add(count).ok_or(Error::Capacity)?;
            if total > ARCHIVE_BYTES {
                return Err(Error::Capacity);
            }
            digest.update(&buffer[..count]);
        }
        if format!("sha256:{:x}", digest.finalize()) != package {
            return Err(Error::Identity);
        }
        let payload = release.child(OsStr::new("payload"))?;
        let device = release.0.metadata()?.dev();
        let mut entries = 0;
        let mut bytes = 0;
        payload_tree(&payload, owner.uid, device, 0, &mut entries, &mut bytes)?;
        if !private_fs::same_entry(&store, name, &release)? {
            return Err(Error::Identity);
        }
        selected = Some((release, payload));
    }
    selected.ok_or(Error::Identity)
}
fn model_package(package: &str) -> Result<()> {
    super::model::hex(package.strip_prefix("sha256:").ok_or(Error::Invalid)?, 64)
}

fn kernel_directory(path: &Path, uid: u32) -> Result<Option<Dir>> {
    let mut root = Dir::absolute_root()?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                root = match root.child(name) {
                    Ok(dir) => dir,
                    Err(private_fs::FsError::Io(error))
                        if error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        return Ok(None)
                    }
                    Err(_) => return Err(Error::Identity),
                };
            }
            _ => return Err(Error::Identity),
        }
        let meta = root.0.metadata()?;
        if (meta.uid() != 0 && meta.uid() != uid) || meta.mode() & 0o022 != 0 {
            return Err(Error::Identity);
        }
    }
    if root.0.metadata()?.uid() != uid {
        return Err(Error::Identity);
    }
    Ok(Some(root))
}
fn readonly_directory(dir: &Dir, uid: u32, device: Option<u64>) -> Result<()> {
    let meta = dir.0.metadata()?;
    if meta.uid() != uid
        || meta.mode() & 0o7777 != 0o500
        || device.is_some_and(|dev| meta.dev() != dev)
    {
        return Err(Error::Identity);
    }
    Ok(())
}
fn file(parent: &Dir, name: &OsStr, uid: u32) -> Result<File> {
    let name = CString::new(name.as_encoded_bytes()).map_err(|_| Error::Identity)?;
    let fd = unsafe {
        libc::openat(
            parent.0.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(Error::Io);
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.nlink() != 1 || meta.uid() != uid || meta.mode() & 0o7777 != 0o400 {
        return Err(Error::Identity);
    }
    Ok(file)
}
fn payload_tree(
    dir: &Dir,
    uid: u32,
    device: u64,
    depth: usize,
    entries: &mut usize,
    bytes: &mut u64,
) -> Result<()> {
    if depth > TREE_DEPTH {
        return Err(Error::Capacity);
    }
    readonly_directory(dir, uid, Some(device))?;
    for name in dir.entries(TREE_ENTRIES.saturating_sub(*entries))? {
        *entries += 1;
        if *entries > TREE_ENTRIES {
            return Err(Error::Capacity);
        }
        let stat = private_fs::entry_metadata(dir, &name)?;
        match stat.st_mode & libc::S_IFMT {
            libc::S_IFDIR => {
                payload_tree(&dir.child(&name)?, uid, device, depth + 1, entries, bytes)?
            }
            libc::S_IFREG => {
                let file = file(dir, &name, uid)?;
                let meta = file.metadata()?;
                if meta.dev() != device || meta.len() > FILE_BYTES {
                    return Err(Error::Identity);
                }
                *bytes = bytes.checked_add(meta.len()).ok_or(Error::Capacity)?;
                if *bytes > ARCHIVE_BYTES as u64 {
                    return Err(Error::Capacity);
                }
            }
            _ => return Err(Error::Identity),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
