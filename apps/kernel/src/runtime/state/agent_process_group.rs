//! MP-09 / MP-11 A03: verify an isolated owned session before group signals.
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
        if current.group != pid {
            continue;
        }
        // Only descendants can join this new session. The unreaped leader
        // pins its identity while the caller holds its Child mutex.
        if current.session != pid || current.start < leader.start {
            return Err(io::Error::other("process group contains an unowned member"));
        }
        members.push(current);
    }
    if !members.iter().any(|m| m.pid == pid) {
        return Err(io::Error::other("owned leader disappeared"));
    }
    Ok(members)
}

/// Caller holds the unreaped Child handle; no arbitrary PID is admitted.
pub(super) fn signal_group(pid: u32, birth: u64, signal: libc::c_int) -> bool {
    let Ok(group) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if group <= 1 {
        return false;
    }
    #[cfg(target_os = "linux")]
    if members(pid, birth).is_ok() {
        return unsafe { libc::kill(-group, signal) == 0 };
    }
    false
}

/// Keep the leader unreaped until every executing group member settles.
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
        if !signal_group(child.id(), birth, libc::SIGKILL) {
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
