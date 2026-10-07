//! MP-09 / MP-11 A03: verify an isolated owned session before pinned signals.
use std::io;

#[cfg(target_os = "linux")]
struct Member {
    pid: u32,
    group: u32,
    session: u32,
    start: u64,
    exited: bool,
}

#[cfg(target_os = "linux")]
fn member(pid: u32) -> io::Result<Member> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields: Vec<_> = stat[stat
        .rfind(')')
        .ok_or_else(|| io::Error::other("invalid process identity"))?
        + 1..]
        .split_whitespace()
        .collect();
    let number = |index: usize| {
        fields
            .get(index)
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| io::Error::other("invalid process identity"))
    };
    Ok(Member {
        pid,
        group: u32::try_from(number(2)?).map_err(io::Error::other)?,
        session: u32::try_from(number(3)?).map_err(io::Error::other)?,
        start: number(19)?,
        exited: matches!(fields.first().copied(), Some("Z" | "X")),
    })
}

#[cfg(target_os = "linux")]
fn members(pid: u32, birth: u64) -> io::Result<Vec<Member>> {
    let leader = member(pid)?;
    if leader.group != pid || leader.session != pid || birth == 0 || leader.start != birth {
        return Err(io::Error::other("process has no isolated owned session"));
    }
    let mut members = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let current = match member(id) {
            Ok(current) => current,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if current.session != pid {
            if current.group == pid {
                return Err(io::Error::other("process group contains an unowned member"));
            }
            continue;
        }
        // A child may change groups without leaving this owned session.
        // The unreaped leader pins the session identity under its Child mutex.
        if current.start < leader.start {
            return Err(io::Error::other("process group contains an unowned member"));
        }
        members.push(current);
    }
    if !members.iter().any(|m| m.pid == pid) {
        return Err(io::Error::other("owned leader disappeared"));
    }
    Ok(members)
}

#[cfg(target_os = "linux")]
fn pin_member(current: &Member) -> io::Result<Option<std::os::fd::OwnedFd>> {
    use std::os::fd::FromRawFd;
    let pid = libc::pid_t::try_from(current.pid).map_err(io::Error::other)?;
    if pid <= 1 {
        return Err(io::Error::other("reserved process target"));
    }
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if raw < 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw as libc::c_int) };
    let verified = match member(current.pid) {
        Ok(verified) => verified,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if verified.start != current.start || verified.session != current.session {
        return Err(io::Error::other("owned process identity changed"));
    }
    Ok(Some(fd))
}

/// Caller holds the unreaped Child; only verified session members are pinned.
pub(super) fn signal_session(pid: u32, birth: u64, signal: libc::c_int) -> bool {
    if libc::pid_t::try_from(pid).is_err() || pid <= 1 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let Ok(owned) = members(pid, birth) else {
            return false;
        };
        // Pin and revalidate every executing member before signaling any.
        let mut pins = Vec::new();
        for current in owned.iter().filter(|member| !member.exited) {
            match pin_member(current) {
                Ok(Some(fd)) => pins.push(fd),
                Ok(None) => {}
                Err(_) => return false,
            }
        }
        for fd in pins {
            if unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd.as_raw_fd(),
                    signal,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            } < 0
                && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            {
                return false;
            }
        }
        return true;
    }
    #[cfg(not(target_os = "linux"))]
    false
}

/// Keep the leader unreaped until every executing session member settles.
#[cfg(target_os = "linux")]
pub(super) fn poll_exit(
    child: &mut std::process::Child,
    birth: u64,
) -> io::Result<Option<std::process::ExitStatus>> {
    let group = members(child.id(), birth)?;
    if !group.iter().any(|m| m.pid == child.id() && m.exited) {
        return Ok(None);
    }
    if group.iter().any(|m| !m.exited) {
        if !signal_session(child.id(), birth, libc::SIGKILL) {
            return Err(io::Error::other("owned descendants could not be settled"));
        }
        return Ok(None);
    }
    child.wait().map(Some)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn poll_exit(
    _child: &mut std::process::Child,
    _birth: u64,
) -> io::Result<Option<std::process::ExitStatus>> {
    Err(io::Error::other(
        "process ownership verification requires Linux",
    ))
}

#[cfg(target_os = "linux")]
pub(super) fn birth(pid: u32) -> io::Result<u64> {
    member(pid).map(|m| m.start)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn birth(_pid: u32) -> io::Result<u64> {
    Err(io::Error::other(
        "process ownership verification requires Linux",
    ))
}
