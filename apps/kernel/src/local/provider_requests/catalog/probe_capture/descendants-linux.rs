//! MP-08 / MP-10 / MP-11: parent ownership plus pidfds pins redirected probe descendants.
use std::collections::BTreeSet;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
pub(super) struct Descendant {
    pid: u32,
    fd: OwnedFd,
}
impl Descendant {
    pub(super) fn alive(&self) -> bool {
        let mut p = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut p, 1, 0) == 0 }
    }
    pub(super) fn signal(&self, signal: i32) -> io::Result<()> {
        super::signal_pid(self.pid.into())?;
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::ESRCH) {
                return Err(e);
            }
        }
        Ok(())
    }
}
fn children(pid: u32) -> io::Result<Vec<u32>> {
    let mut result = Vec::new();
    let tasks = match std::fs::read_dir(format!("/proc/{pid}/task")) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(result),
        Err(e) => return Err(e),
    };
    for task in tasks.flatten() {
        if let Ok(text) = std::fs::read_to_string(task.path().join("children")) {
            result.extend(
                text.split_whitespace()
                    .filter_map(|p| p.parse::<u32>().ok()),
            );
            if result.len() > 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "probe descendant limit",
                ));
            }
        }
    }
    Ok(result)
}
fn parent(pid: u32) -> Option<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()?
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
fn group_members(root: u32) -> io::Result<Vec<u32>> {
    let mut members = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if pid != root && group(pid) == Some(root) {
            members.push(pid);
        }
        if members.len() >= 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "probe descendant limit",
            ));
        }
    }
    Ok(members)
}
fn group(pid: u32) -> Option<u32> {
    // MP-08 / MP-10 / MP-11: process-group lookup needs no unrelated process
    // stat files. getpgid is a read-only kernel lookup; pidfd pinning and the
    // ancestry/group checks below still establish ownership before signaling.
    let pid = super::signal_pid(pid.into()).ok()?;
    let group = unsafe { libc::getpgid(pid) };
    (group > 1).then_some(group as u32)
}
pub(super) fn observe(
    root: Option<u32>,
    owned: &mut Vec<Descendant>,
    include_group: bool,
) -> io::Result<()> {
    let mut parents = root
        .into_iter()
        .chain(owned.iter().filter(|p| p.alive()).map(|p| p.pid))
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    while let Some(pid) = parents.pop() {
        if !visited.insert(pid) {
            continue;
        }
        let mut candidates = children(pid)?;
        if include_group && root == Some(pid) {
            candidates.extend(group_members(pid)?);
        }
        for child in candidates {
            if super::signal_pid(child.into()).is_err()
                || owned.iter().any(|p| p.pid == child && p.alive())
            {
                continue;
            }
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child, 0) };
            if fd < 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() == Some(libc::ESRCH) {
                    continue;
                }
                return Err(e);
            }
            let pinned = Descendant {
                pid: child,
                fd: unsafe { OwnedFd::from_raw_fd(fd as i32) },
            };
            let parent_owned = root == Some(pid) || owned.iter().any(|p| p.pid == pid && p.alive());
            if pinned.alive()
                && parent_owned
                && (parent(child) == Some(pid) || (root == Some(pid) && group(child) == Some(pid)))
            {
                if owned.len() >= 1024 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "probe descendant limit",
                    ));
                }
                owned.push(pinned);
                parents.push(child);
            }
        }
    }
    Ok(())
}
