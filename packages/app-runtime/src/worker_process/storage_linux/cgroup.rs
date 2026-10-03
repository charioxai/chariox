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
    pub fn require_single_process(&self, pid: i32) -> Result<()> {
        if files::identity(&self.directory.0)? != self.identity
            || files::identity(&self._root.0)? != self.root_identity
        {
            return Err(Error::Identity);
        }
        let mut bytes = Vec::new();
        use std::io::Read;
        open(&self.directory, "cgroup.procs", libc::O_RDONLY)?
            .take(1025)
            .read_to_end(&mut bytes)?;
        if bytes != format!("{pid}\n").as_bytes() {
            return Err(Error::Identity);
        }
        Ok(())
    }
    pub fn require_empty(&self) -> Result<()> {
        match self.members()? {
            Members::None => Ok(()),
            Members::Some => Err(Error::Busy),
            // Acquisition and an explicit release still need the live leaf.
            Members::Removed => Err(Error::Io),
        }
    }
    /// Disconnect and recovery release: kill whatever is left and wait until
    /// the leaf is empty. A leaf removed meanwhile is empty for good (see
    /// `Members::Removed`): after a kernel crash, systemd trims the stopped
    /// unit's delegated subtree, often before this loop observes it empty.
    pub fn quiesce(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.members()? {
                Members::None | Members::Removed => return Ok(()),
                Members::Some => {}
            }
            match self.kill.write_at(b"1", 0) {
                Ok(1) => {}
                // Removed after the observation above: the next observation
                // reports it (still within the deadline).
                Err(error) if error.raw_os_error() == Some(libc::ENODEV) => {}
                _ => return Err(Error::Io),
            }
            if Instant::now() >= deadline {
                return Err(Error::RecoveryRequired);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn members(&self) -> Result<Members> {
        if files::identity(&self.directory.0)? != self.identity {
            return Err(Error::Identity);
        }
        let mut bytes = [0u8; 1025];
        members(
            self.events
                .read_at(&mut bytes, 0)
                .map(|count| &bytes[..count]),
        )
    }
}
/// What the bound leaf's `cgroup.events` says about its processes.
#[derive(Debug, PartialEq)]
enum Members {
    None,
    Some,
    /// The held directory was removed (reading it fails with ENODEV). cgroup
    /// v2 removes only a cgroup with no processes or children, and no process
    /// can join a removed one, so it stays empty.
    Removed,
}
fn members(read: std::io::Result<&[u8]>) -> Result<Members> {
    let bytes = match read {
        Ok(bytes) => bytes,
        Err(error) if error.raw_os_error() == Some(libc::ENODEV) => return Ok(Members::Removed),
        Err(error) => return Err(error.into()),
    };
    if bytes.len() > 1024 {
        return Err(Error::Identity);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Identity)?;
    Ok(if populated(text)? {
        Members::Some
    } else {
        Members::None
    })
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
    #[test]
    fn only_a_removed_leaf_reads_as_removed() {
        let removed = std::io::Error::from_raw_os_error(libc::ENODEV);
        assert_eq!(members(Err(removed)), Ok(Members::Removed));
        for other in [libc::EIO, libc::EACCES, libc::EBADF] {
            assert_eq!(
                members(Err(std::io::Error::from_raw_os_error(other))),
                Err(Error::Io)
            );
        }
        assert_eq!(members(Ok(b"populated 0\nfrozen 0\n")), Ok(Members::None));
        assert_eq!(members(Ok(b"populated 1\nfrozen 0\n")), Ok(Members::Some));
        assert_eq!(members(Ok(&[b'x'; 1025])), Err(Error::Identity));
    }
    /// The kernel-crash case on a real cgroup: the leaf's last process dies
    /// and the leaf is removed (as systemd trims a stopped unit) before the
    /// helper quiesces it on disconnect. Run as root on a cgroup v2 host:
    /// `cargo test -p chariox-app-runtime --lib -- --ignored removed_after`.
    #[test]
    #[ignore = "needs root and a writable cgroup v2 root at /sys/fs/cgroup"]
    fn quiesce_releases_a_leaf_removed_after_its_members_died() {
        use std::{fs, io::Write, os::unix::fs::DirBuilderExt, process::Command};
        /// Removes the scratch cgroups (and kills a leftover member) even when
        /// an assertion fails midway.
        struct Scratch(std::path::PathBuf, std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::write(self.1.join("cgroup.kill"), "1");
                for _ in 0..100 {
                    if fs::remove_dir(&self.1).is_ok() || !self.1.exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                let _ = fs::remove_dir(&self.0);
            }
        }
        let unique = format!(
            "{:08x}{:024x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::path::PathBuf::from(format!("/sys/fs/cgroup/chariox-quiesce-{unique}"));
        let leaf = format!("app-{unique}");
        let path = root.join(&leaf);
        fs::DirBuilder::new().mode(0o755).create(&root).unwrap();
        let _scratch = Scratch(root.clone(), path.clone());
        fs::DirBuilder::new().mode(0o755).create(&path).unwrap();
        let owner = Owner {
            uid: 0,
            gid: 0,
            cgroup_root: root.to_str().unwrap().into(),
            kernel_database_paths: Vec::new(),
        };
        let bound = Bound::acquire(&owner, &leaf).unwrap();
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(path.join("cgroup.procs"))
            .unwrap()
            .write_all(child.id().to_string().as_bytes())
            .unwrap();
        assert_eq!(bound.require_empty(), Err(Error::Busy));
        fs::write(path.join("cgroup.kill"), "1").unwrap();
        child.wait().unwrap();
        while fs::read_to_string(path.join("cgroup.events"))
            .unwrap()
            .contains("populated 1")
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        fs::remove_dir(&path).unwrap();
        // Acquisition and explicit release keep refusing a removed leaf...
        assert_eq!(bound.require_empty(), Err(Error::Io));
        // ...but the disconnect release it blocked forever now completes.
        assert_eq!(bound.quiesce(), Ok(()));
    }
}
