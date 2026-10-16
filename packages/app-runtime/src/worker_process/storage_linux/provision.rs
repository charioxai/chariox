//! Fixed managed-kernel systemd delegation setup. Only the unit's root
//! ExecStartPost runs this mode (the main process is already in its
//! `supervisor` subgroup); no socket operation accepts a path, service name or
//! controller.
use super::{files, model::Enrollment, Error, Result};
use crate::private_fs::Dir;
use std::{
    ffi::OsStr,
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::FileExt,
    },
    path::Path,
};
pub(super) const APPS: &str = "/sys/fs/cgroup/system.slice/chariox-managed-bootstrap.service/apps";
pub(super) const PATH1_APPS: &str =
    "/sys/fs/cgroup/system.slice/chariox-path1-managed-bootstrap.service/apps";

// The caller cannot select a unit. Match only systemd's actual root post-start
// control process, then require the enrollment for that exact topology.
fn managed_service(cgroup: &[u8]) -> Result<(&'static str, &'static str)> {
    match cgroup {
        b"0::/system.slice/chariox-managed-bootstrap.service/.control\n" => {
            Ok(("chariox-managed-bootstrap.service", APPS))
        }
        b"0::/system.slice/chariox-path1-managed-bootstrap.service/.control\n" => {
            Ok(("chariox-path1-managed-bootstrap.service", PATH1_APPS))
        }
        _ => Err(Error::Identity),
    }
}

pub(super) fn managed(config: Enrollment) -> Result<()> {
    config.validate()?;
    let mut bytes = [0u8; 1024];
    let count = File::open("/proc/self/cgroup")?.read_at(&mut bytes, 0)?;
    let (service_name, root) = managed_service(&bytes[..count])?;
    let mut matches = config
        .owners
        .iter()
        .filter(|owner| owner.cgroup_root == root);
    let owner = matches.next().ok_or(Error::Identity)?;
    if matches.next().is_some() {
        return Err(Error::Identity);
    }
    // The systemd unit root can be delegated to the enrolled UID. Ancestors
    // remain root-owned and the final component is opened without symlinks.
    let parent = files::root_directory(Path::new("/sys/fs/cgroup/system.slice"))?;
    let service = parent.child(OsStr::new(service_name))?;
    let mut fs = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(service.0.as_raw_fd(), fs.as_mut_ptr()) } != 0
        || unsafe { fs.assume_init() }.f_type != libc::CGROUP2_SUPER_MAGIC
    {
        return Err(Error::Identity);
    }
    if !read(&service, "cgroup.procs")?.is_empty() {
        return Err(Error::Busy);
    }
    write(&service, "cgroup.subtree_control", "+cpu +memory +pids")?;
    let apps = match service.create_child(OsStr::new("apps")) {
        Ok(apps) => apps,
        Err(crate::private_fs::FsError::Io(error))
            if error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            service.child(OsStr::new("apps"))?
        }
        Err(error) => return Err(error.into()),
    };
    if !read(&apps, "cgroup.events")?
        .lines()
        .any(|line| line == "populated 0")
    {
        return Err(Error::Busy);
    }
    write(&apps, "cgroup.subtree_control", "+cpu +memory +pids")?;
    for file in [
        &apps.0,
        &open(&apps, "cgroup.procs", libc::O_WRONLY)?,
        &open(&apps, "cgroup.subtree_control", libc::O_WRONLY)?,
        &open(&service, "cgroup.procs", libc::O_WRONLY)?,
    ] {
        if unsafe { libc::fchown(file.as_raw_fd(), owner.uid, owner.gid) } != 0 {
            return Err(Error::Io);
        }
    }
    if unsafe { libc::fchmod(apps.0.as_raw_fd(), 0o700) } != 0 {
        return Err(Error::Io);
    }
    let controls = read(&apps, "cgroup.subtree_control")?;
    if ["cpu", "memory", "pids"]
        .iter()
        .any(|required| !controls.split_whitespace().any(|value| value == *required))
    {
        return Err(Error::Identity);
    }
    Ok(())
}
fn open(directory: &Dir, name: &str, flags: i32) -> Result<File> {
    let name = files::component(name)?;
    let fd = unsafe {
        libc::openat(
            directory.0.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(Error::Io);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn read(directory: &Dir, name: &str) -> Result<String> {
    let mut bytes = [0u8; 1025];
    let count = open(directory, name, libc::O_RDONLY)?.read_at(&mut bytes, 0)?;
    if count > 1024 {
        return Err(Error::Identity);
    }
    String::from_utf8(bytes[..count].into()).map_err(|_| Error::Identity)
}
fn write(directory: &Dir, name: &str, value: &str) -> Result<()> {
    if open(directory, name, libc::O_WRONLY)?.write_at(value.as_bytes(), 0)? != value.len() {
        return Err(Error::Io);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_domain_is_bound_to_the_actual_systemd_topology() {
        for (unit, root) in [
            ("chariox-managed-bootstrap.service", APPS),
            ("chariox-path1-managed-bootstrap.service", PATH1_APPS),
        ] {
            let cgroup = format!("0::/system.slice/{unit}/.control\n");
            assert_eq!(managed_service(cgroup.as_bytes()), Ok((unit, root)));
            for suffix in ["", "/supervisor", "/apps", "/.control/child"] {
                let cgroup = format!("0::/system.slice/{unit}{suffix}\n");
                assert_eq!(managed_service(cgroup.as_bytes()), Err(Error::Identity));
            }
        }
        for cgroup in [
            b"0::/system.slice/other.service/.control\n".as_slice(),
            b"0::/system.slice/chariox-disposable-worker-bootstrap.service/.control\n",
            b"0::/system.slice/chariox-path1-managed-bootstrap.service/.control\n1:cpu:/\n",
        ] {
            assert_eq!(managed_service(cgroup), Err(Error::Identity));
        }
    }
}
