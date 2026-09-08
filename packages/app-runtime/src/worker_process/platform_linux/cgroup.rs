//! Descriptor-owned cgroup-v2 leaf. The installer delegates only this subtree;
//! creation never walks or mutates an unrelated systemd/Docker cgroup.
use super::{policy, Result, WorkerError, MEMORY_BYTES, TASKS};
use std::{
    collections::BTreeMap,
    ffi::{CString, OsStr},
    fs::{File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) struct Delegation {
    directory: File,
    path: PathBuf,
}
impl Delegation {
    /// Private installer input, never a daemon/App request field. The manager
    /// already occupies its own sibling leaf; this delegated inner node is empty.
    pub fn open(path: &Path) -> Result<Self> {
        if !path.is_absolute()
            || path.as_os_str().len() > 1024
            || std::fs::canonicalize(path).map_err(|_| WorkerError::Preparation)? != path
        {
            return Err(WorkerError::Preparation);
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| WorkerError::Preparation)?;
        filesystem(&directory)?;
        let metadata = directory.metadata().map_err(|_| WorkerError::Preparation)?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
            return Err(WorkerError::Preparation);
        }
        if read_at(&directory, "cgroup.type")? != "domain\n"
            || !policy::pids(&read_at(&directory, "cgroup.procs")?)?.is_empty()
        {
            return Err(WorkerError::Preparation);
        }
        let control = read_at(&directory, "cgroup.subtree_control")?;
        for required in ["cpu", "memory", "pids"] {
            if !control.split_whitespace().any(|value| value == required) {
                return Err(WorkerError::Preparation);
            }
        }
        Ok(Self {
            directory,
            path: path.to_owned(),
        })
    }

    pub fn create(&self) -> Result<Lease> {
        let mut random = [0u8; 16];
        let mut offset = 0;
        while offset < random.len() {
            let result = unsafe {
                libc::getrandom(
                    random[offset..].as_mut_ptr().cast(),
                    random.len() - offset,
                    0,
                )
            };
            if result < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            if result <= 0 {
                return Err(WorkerError::ResourceDomain);
            }
            offset += result as usize;
        }
        let name = CString::new(format!(
            "app-{}",
            random
                .iter()
                .map(|value| format!("{value:02x}"))
                .collect::<String>()
        ))
        .unwrap();
        let parent = self
            .directory
            .try_clone()
            .map_err(|_| WorkerError::ResourceDomain)?;
        if unsafe { libc::mkdirat(self.directory.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(WorkerError::ResourceDomain);
        }
        let created = (|| {
            let directory = open(
                &parent,
                name.to_str().unwrap(),
                libc::O_RDONLY | libc::O_DIRECTORY,
            )?;
            filesystem(&directory)?;
            Ok(Lease {
                parent,
                path: self.path.join(OsStr::from_bytes(name.as_bytes())),
                name: name.clone(),
                processes: open(&directory, "cgroup.procs", libc::O_WRONLY)?,
                kill: open(&directory, "cgroup.kill", libc::O_WRONLY)?,
                events: open(&directory, "cgroup.events", libc::O_RDONLY)?,
                directory,
                empty: false,
            })
        })();
        let mut lease = match created {
            Ok(lease) => lease,
            Err(error) => {
                // No child can be launched before this private preparation returns.
                unsafe {
                    libc::unlinkat(
                        self.directory.as_raw_fd(),
                        name.as_ptr(),
                        libc::AT_REMOVEDIR,
                    );
                }
                return Err(error);
            }
        };
        for (name, value) in [
            ("memory.max", MEMORY_BYTES.to_string()),
            ("memory.swap.max", "0".into()),
            ("memory.oom.group", "1".into()),
            ("pids.max", TASKS.to_string()),
            ("cpu.max", "100000 100000".into()),
        ] {
            write(&open(&lease.directory, name, libc::O_WRONLY)?, &value)?;
        }
        lease.verify_limits()?;
        lease.empty = false;
        Ok(lease)
    }
}

pub(super) struct Lease {
    parent: File,
    name: CString,
    pub path: PathBuf,
    directory: File,
    pub processes: File,
    kill: File,
    events: File,
    empty: bool,
}
impl Lease {
    pub fn verify_limits(&self) -> Result<()> {
        let values = [
            "memory.max",
            "memory.swap.max",
            "memory.oom.group",
            "pids.max",
            "cpu.max",
        ]
        .into_iter()
        .map(|name| Ok((name, read_at(&self.directory, name)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
        policy::limits(&values)
    }
    pub fn members(&self) -> Result<Vec<i32>> {
        policy::pids(&read_at(&self.directory, "cgroup.procs")?)
    }
    pub fn check_running(&self) -> Result<()> {
        self.verify_limits()?;
        let memory = read_at(&self.directory, "memory.events")?;
        if policy::number(
            policy::fields(&memory, ' ')?
                .get("oom_kill")
                .ok_or(WorkerError::ResourceDomain)?,
        )? != 0
        {
            return Err(WorkerError::MemoryLimit);
        }
        let tasks = read_at(&self.directory, "pids.events")?;
        if policy::number(
            policy::fields(&tasks, ' ')?
                .get("max")
                .ok_or(WorkerError::ResourceDomain)?,
        )? != 0
        {
            return Err(WorkerError::ThreadLimit);
        }
        Ok(())
    }
    pub fn terminate(&self) {
        let _ = write(&self.kill, "1");
    }
    pub fn reap_blocking(&mut self) {
        // No unowned background cleanup or reservation release on an I/O error.
        // An uninterruptible task deliberately retains this ownership thread.
        while !self.empty {
            self.terminate();
            let value = read(&self.events);
            self.empty = value
                .as_ref()
                .ok()
                .and_then(|v| policy::fields(v, ' ').ok())
                .and_then(|fields| fields.get("populated").map(|value| *value == "0"))
                .unwrap_or(false);
            if !self.empty {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.reap_blocking();
        unsafe {
            libc::unlinkat(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                libc::AT_REMOVEDIR,
            );
        }
    }
}

fn filesystem(file: &File) -> Result<()> {
    let mut data = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(file.as_raw_fd(), data.as_mut_ptr()) } != 0
        || unsafe { data.assume_init() }.f_type != libc::CGROUP2_SUPER_MAGIC
    {
        return Err(WorkerError::Preparation);
    }
    Ok(())
}
fn open(parent: &File, name: &str, flags: i32) -> Result<File> {
    if name.is_empty() || name.contains('/') || name.contains('\0') || matches!(name, "." | "..") {
        return Err(WorkerError::ResourceDomain);
    }
    let name = CString::new(name).unwrap();
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(WorkerError::ResourceDomain);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn read_at(parent: &File, name: &str) -> Result<String> {
    read(&open(parent, name, libc::O_RDONLY)?)
}
fn read(file: &File) -> Result<String> {
    let mut bytes = [0u8; 32769];
    let size = file
        .read_at(&mut bytes, 0)
        .map_err(|_| WorkerError::ResourceDomain)?;
    if size > 32768 {
        return Err(WorkerError::ResourceDomain);
    }
    String::from_utf8(bytes[..size].to_vec()).map_err(|_| WorkerError::ResourceDomain)
}
fn write(file: &File, value: &str) -> Result<()> {
    // cgroup control writes are single records; a partial write is a failure.
    let size = file
        .write_at(value.as_bytes(), 0)
        .map_err(|_| WorkerError::ResourceDomain)?;
    if size != value.len() {
        return Err(WorkerError::ResourceDomain);
    }
    Ok(())
}
