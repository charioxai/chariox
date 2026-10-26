//! MP-09 / MP-11 A03: verify an isolated owned session before pinned signals.
use std::io;

#[cfg(target_os = "linux")]
struct Member {
    pid: u32,
    group: u32,
    parent: u32,
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
        parent: u32::try_from(number(1)?).map_err(io::Error::other)?,
        session: u32::try_from(number(3)?).map_err(io::Error::other)?,
        start: number(19)?,
        exited: matches!(fields.first().copied(), Some("Z" | "X")),
    })
}

/// A process that exits between listing and reading /proc is simply gone.
#[cfg(target_os = "linux")]
fn vanished(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

#[cfg(target_os = "linux")]
fn process_ids() -> io::Result<impl Iterator<Item = u32>> {
    Ok(std::fs::read_dir("/proc")?.filter_map(|entry| {
        entry
            .ok()?
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
    }))
}

#[cfg(target_os = "linux")]
fn leader(pid: u32, birth: u64) -> io::Result<Member> {
    let leader = member(pid)?;
    if leader.group != pid || leader.session != pid || birth == 0 || leader.start != birth {
        return Err(io::Error::other("process has no isolated owned session"));
    }
    Ok(leader)
}

#[cfg(target_os = "linux")]
fn members(pid: u32, birth: u64) -> io::Result<Vec<Member>> {
    let leader = leader(pid, birth)?;
    let mut members = Vec::new();
    for id in process_ids()? {
        let current = match member(id) {
            Ok(current) => current,
            Err(e) if vanished(&e) => continue,
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
        Err(error) if vanished(&error) => return Ok(None),
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
        let mut complete = true;
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
                complete = false;
            }
        }
        complete
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
    // Only the leader is read while it runs; the session scan starts at its exit.
    if !leader(child.id(), birth)?.exited {
        return Ok(None);
    }
    let group = members(child.id(), birth)?;
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

/// Inherited by every watched process so descendants that outlive a crashed
/// kernel stay attributable to their wake after the session leader is gone.
pub(super) const WAKE_MARKER_ENV: &str = "CHARIOX_AGENT_WAKE_ID";

#[cfg(target_os = "linux")]
fn wake_marker(pid: u32) -> Option<String> {
    let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    let prefix = format!("{WAKE_MARKER_ENV}=");
    environ.split(|b| *b == 0).find_map(|entry| {
        entry
            .strip_prefix(prefix.as_bytes())
            .and_then(|id| String::from_utf8(id.to_vec()).ok())
    })
}

/// Approximate a boot-relative process birth in the current realtime epoch.
/// Fail closed when either clock is unavailable. One tick covers quantization.
#[cfg(target_os = "linux")]
pub(super) fn start_time_ms(start: u64) -> Option<u64> {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks <= 0 {
        return None;
    }
    let mut boot = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut boot) } != 0 || boot.tv_sec < 0 {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    let uptime = boot.tv_sec as u128 * 1000 + boot.tv_nsec as u128 / 1_000_000;
    u64::try_from(
        now.checked_sub(uptime)?
            .checked_add(start as u128 * 1000 / ticks as u128)?,
    )
    .ok()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn start_time_ms(_start: u64) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn born_after(current: &Member, launch_ms: u64) -> bool {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks <= 0 {
        return false;
    }
    let tolerance = 1000u64.div_ceil(ticks as u64);
    start_time_ms(current.start).is_some_and(|birth| birth.saturating_add(tolerance) >= launch_ms)
}

/// Stops actual orphans still carrying a wake marker, born after its launch.
/// A live unrelated parent's descendants are ambiguous and never signalled.
/// Pin targets, confirm exit, then rescan for newly re-parented descendants.
#[cfg(target_os = "linux")]
pub(super) fn reap_orphans(
    wakes: &std::collections::BTreeMap<String, u64>,
) -> io::Result<std::collections::BTreeMap<String, usize>> {
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};
    let mut stopped = std::collections::BTreeMap::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut pins: Vec<(String, std::os::fd::OwnedFd)> = Vec::new();
    let mut empty_scans = 0;
    loop {
        let mut remaining = Vec::new();
        for (wake, fd) in pins.drain(..) {
            let mut pfd = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, 0) };
            if ready < 0 {
                return Err(io::Error::last_os_error());
            }
            if ready > 0 && pfd.revents & libc::POLLIN != 0 {
                *stopped.entry(wake).or_insert(0) += 1;
            } else {
                remaining.push((wake, fd));
            }
        }
        pins = remaining;
        // Rescan only after every signalled parent has exited. Its children
        // are now re-parented, including descendants of a setsid child.
        if pins.is_empty() {
            for pid in process_ids()?.filter(|pid| *pid > 1 && *pid != std::process::id()) {
                let Some(wake) = wake_marker(pid).filter(|w| wakes.contains_key(w)) else {
                    continue;
                };
                let current = match member(pid) {
                    Ok(current) => current,
                    Err(error) if vanished(&error) => continue,
                    Err(error) => return Err(error),
                };
                if current.exited || current.parent != 1 || !born_after(&current, wakes[&wake]) {
                    continue;
                }
                let Some(fd) = pin_member(&current)? else {
                    continue;
                };
                if wake_marker(pid).as_ref() != Some(&wake)
                    || !member(pid).is_ok_and(|verified| {
                        verified.start == current.start
                            && verified.parent == 1
                            && born_after(&verified, wakes[&wake])
                    })
                {
                    return Err(io::Error::other(
                        "orphan identity changed; physical settlement unconfirmed",
                    ));
                }
                if unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        fd.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                } < 0
                    && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
                {
                    return Err(io::Error::last_os_error());
                }
                pins.push((wake, fd));
            }
        }
        if pins.is_empty() {
            empty_scans += 1;
            if empty_scans == 2 {
                return Ok(stopped);
            }
        } else {
            empty_scans = 0;
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other("escaped subtree exit unconfirmed at cleanup deadline; survivors may still be running"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn reap_orphans(
    _wakes: &std::collections::BTreeMap<String, u64>,
) -> io::Result<std::collections::BTreeMap<String, usize>> {
    Err(io::Error::other("orphan settlement requires Linux"))
}

#[cfg(all(test, target_os = "linux"))]
pub(super) fn marked_count_for_test(wake: &str) -> usize {
    process_ids()
        .unwrap()
        .filter(|pid| {
            wake_marker(*pid).as_deref() == Some(wake) && member(*pid).is_ok_and(|m| !m.exited)
        })
        .count()
}
