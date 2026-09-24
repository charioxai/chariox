//! Only the root-enrolled cgroup subtree can authorize a storage lifetime.
//! cgroup-v2 forbids rename; a missing exact child was removed while empty.
use super::{
    files,
    model::{self, Identity, Journal, Owner},
    Error, Result,
};
use crate::private_fs::Dir;
use std::{
    ffi::OsStr,
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{FileExt, MetadataExt},
    },
    path::{Component, Path},
    time::{Duration, Instant},
};

pub(super) struct Bound {
    _root: Dir,
    pub root_identity: Identity,
    directory: Dir,
    pub identity: Identity,
    events: File,
    kill: File,
}
impl Bound {
    pub fn acquire(owner: &Owner, leaf: &str) -> Result<Self> {
        model::hex(leaf.strip_prefix("app-").ok_or(Error::Invalid)?, 32)?;
        let root = open_root(owner)?.ok_or(Error::Identity)?;
        let directory = root.child(OsStr::new(leaf))?;
        let bound = Self::from_directory(root, directory, owner)?;
        bound.require_empty()?;
        Ok(bound)
    }
    pub fn recover(owner: &Owner, journal: &Journal) -> Result<Option<Self>> {
        if boot_id()? != journal.boot_id {
            return Ok(None);
        }
        let Some(root) = open_root(owner)? else {
            return Ok(None);
        };
        if files::identity(&root.0)? != journal.cgroup_root_identity {
            // v2 forbids renaming cgroups, including ancestors. A replacement
            // at the same enrolled path required removal of the empty original.
            return Ok(None);
        }
        let directory = match root.child(OsStr::new(&journal.cgroup_leaf)) {
            Ok(directory) => directory,
            Err(crate::private_fs::FsError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error.into()),
        };
        if files::identity(&directory.0)? != journal.cgroup_identity {
            // The original immutable cgroup-v2 name can only be replaced after
            // its empty directory was removed. Do not signal the replacement.
            return Ok(None);
        }
        Self::from_directory(root, directory, owner).map(Some)
    }
    fn from_directory(root: Dir, directory: Dir, owner: &Owner) -> Result<Self> {
        for dir in [&root, &directory] {
            let metadata = dir.0.metadata()?;
            if metadata.uid() != owner.uid || metadata.mode() & 0o022 != 0 {
                return Err(Error::Identity);
            }
            filesystem(&dir.0)?;
        }
        let identity = files::identity(&directory.0)?;
        Ok(Self {
            root_identity: files::identity(&root.0)?,
            _root: root,
            events: open(&directory, "cgroup.events", libc::O_RDONLY)?,
            kill: open(&directory, "cgroup.kill", libc::O_WRONLY)?,
            directory,
            identity,
        })
    }
    pub fn require_empty(&self) -> Result<()> {
        if files::identity(&self.directory.0)? != self.identity {
            return Err(Error::Identity);
        }
        let mut bytes = [0u8; 1025];
        let count = self.events.read_at(&mut bytes, 0)?;
        if count > 1024 {
            return Err(Error::Identity);
        }
        let text = std::str::from_utf8(&bytes[..count]).map_err(|_| Error::Identity)?;
        match populated(text)? {
            false => Ok(()),
            true => Err(Error::Busy),
        }
    }
    pub fn quiesce(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.require_empty() {
                Ok(()) => return Ok(()),
                Err(Error::Busy) => {}
                Err(error) => return Err(error),
            }
            if self.kill.write_at(b"1", 0)? != 1 {
                return Err(Error::Io);
            }
            if Instant::now() >= deadline {
                return Err(Error::RecoveryRequired);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
fn open_root(owner: &Owner) -> Result<Option<Dir>> {
    let mut dir = Dir::absolute_root()?;
    for part in Path::new(&owner.cgroup_root).components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => match dir.child(name) {
                Ok(child) => dir = child,
                Err(crate::private_fs::FsError::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    filesystem(&dir.0)?;
                    return Ok(None);
                }
                Err(error) => return Err(error.into()),
            },
            _ => return Err(Error::Identity),
        }
        let metadata = dir.0.metadata()?;
        if ![0, owner.uid].contains(&metadata.uid()) || metadata.mode() & 0o022 != 0 {
            return Err(Error::Identity);
        }
    }
    filesystem(&dir.0)?;
    if dir.0.metadata()?.uid() != owner.uid {
        return Err(Error::Identity);
    }
    Ok(Some(dir))
}
fn filesystem(file: &File) -> Result<()> {
    let mut value = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(file.as_raw_fd(), value.as_mut_ptr()) } != 0
        || unsafe { value.assume_init() }.f_type != libc::CGROUP2_SUPER_MAGIC
    {
        return Err(Error::Identity);
    }
    Ok(())
}
fn open(parent: &Dir, name: &str, flags: i32) -> Result<File> {
    let name = files::component(name)?;
    let fd = unsafe {
        libc::openat(
            parent.0.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(Error::Io);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub(super) fn boot_id() -> Result<String> {
    // Kernel-owned proc value; bounded read, never caller supplied.
    let file = File::open("/proc/sys/kernel/random/boot_id")?;
    let mut bytes = [0u8; 64];
    let count = file.read_at(&mut bytes, 0)?;
    let id = std::str::from_utf8(&bytes[..count])
        .map_err(|_| Error::Identity)?
        .trim_end_matches('\n');
    model::uuid(id)?;
    Ok(id.into())
}
fn populated(text: &str) -> Result<bool> {
    let mut found = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            return Err(Error::Identity);
        };
        if key == "populated" {
            if found.is_some() {
                return Err(Error::Identity);
            }
            found = Some(match value {
                "0" => false,
                "1" => true,
                _ => return Err(Error::Identity),
            });
        }
    }
    found.ok_or(Error::Identity)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_empty_observation_permits_release() {
        assert_eq!(populated("populated 0\nfrozen 0\n"), Ok(false));
        assert_eq!(populated("populated 1\nfrozen 0\n"), Ok(true));
        for bad in [
            "",
            "populated 00\n",
            "populated 0\npopulated 1\n",
            "populated true",
        ] {
            assert!(populated(bad).is_err());
        }
    }
}
