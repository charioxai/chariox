use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelProcessHealthSnapshot {
    pub process_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_resident_set_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_resident_set_bytes: Option<u64>,
}

impl KernelProcessHealthSnapshot {
    pub(crate) fn current() -> Self {
        Self {
            process_id: std::process::id(),
            current_resident_set_bytes: resident_set_bytes_for_pid(std::process::id()),
            peak_resident_set_bytes: peak_resident_set_bytes(),
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn resident_set_bytes_for_pid(pid: u32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let result = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if result != size {
        return None;
    }
    let resident = unsafe { info.assume_init() }.pti_resident_size;
    (resident > 0).then_some(resident)
}

#[cfg(target_os = "linux")]
pub(crate) fn resident_set_bytes_for_pid(pid: u32) -> Option<u64> {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let resident_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if resident_pages == 0 || page_size <= 0 {
        return None;
    }
    resident_pages.checked_mul(u64::try_from(page_size).ok()?)
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
pub(crate) fn resident_set_bytes_for_pid(_pid: u32) -> Option<u64> {
    None
}

#[cfg(unix)]
fn signal_pid(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok().filter(|pid| *pid > 1)
}

#[cfg(unix)]
pub fn process_running(pid: u32) -> bool {
    let Some(pid) = signal_pid(pid) else {
        return false;
    };
    let result = unsafe { libc::kill(pid, 0) };
    result == 0
}

#[cfg(not(unix))]
pub fn process_running(pid: u32) -> bool {
    resident_set_bytes_for_pid(pid).is_some()
}

/// Kills `pid` and the descendants found in one snapshot, each by its start
/// identity, so a PID reused in the meantime is never signalled. There is no
/// grace period in which a descendant's PID could be recycled.
#[cfg(unix)]
pub(crate) fn terminate_process_tree(pid: u32) -> bool {
    super::owned_process_signals::OwnedProcessSignals::for_pid(pid)
        .and_then(|mut owned| owned.kill_owned_processes())
        .is_ok()
}

#[cfg(not(unix))]
pub(crate) fn terminate_process_tree(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn peak_resident_set_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    let max_rss = u64::try_from(usage.ru_maxrss).ok()?;
    if max_rss == 0 {
        return None;
    }
    Some(rusage_maxrss_to_bytes(max_rss))
}

#[cfg(not(unix))]
fn peak_resident_set_bytes() -> Option<u64> {
    None
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn rusage_maxrss_to_bytes(max_rss: u64) -> u64 {
    max_rss
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios"))))]
fn rusage_maxrss_to_bytes(max_rss: u64) -> u64 {
    max_rss.saturating_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::{resident_set_bytes_for_pid, KernelProcessHealthSnapshot};

    #[cfg(unix)]
    #[test]
    fn process_signal_targets_reject_special_and_overflow_pids() {
        for pid in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
            assert_eq!(super::signal_pid(pid), None, "unsafe signal target {pid}");
            assert!(!super::process_running(pid));
            assert!(!super::terminate_process_tree(pid));
        }
        assert_eq!(super::signal_pid(2), Some(2));
        assert_eq!(super::signal_pid(i32::MAX as u32), Some(i32::MAX));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn terminating_a_process_tree_ends_its_descendants_at_once() {
        use std::io::{BufRead, BufReader};
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 60 & echo $!; wait"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let descendant: u32 = line.trim().parse().unwrap();
        let alive = |pid: u32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|stat| !stat.split_once(") ").unwrap().1.starts_with('Z'))
        };
        assert!(alive(descendant));
        let started = std::time::Instant::now();
        assert!(super::terminate_process_tree(child.id()));
        let _ = child.wait();
        assert!(!alive(descendant), "the descendant survived the tree kill");
        assert!(started.elapsed() < std::time::Duration::from_millis(900));
    }

    #[test]
    fn kernel_process_health_reports_current_pid() {
        let snapshot = KernelProcessHealthSnapshot::current();

        assert_eq!(snapshot.process_id, std::process::id());
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux"))]
        assert!(snapshot.current_resident_set_bytes.is_some());
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux"))]
        assert!(resident_set_bytes_for_pid(std::process::id()).is_some());
    }
}
