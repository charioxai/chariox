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
pub fn process_running(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0
}

#[cfg(not(unix))]
pub fn process_running(pid: u32) -> bool {
    resident_set_bytes_for_pid(pid).is_some()
}

#[cfg(unix)]
pub(crate) fn terminate_process_tree(pid: u32) -> bool {
    let mut pids = descendant_process_ids(pid);
    pids.push(pid);
    pids.dedup();

    for child_pid in pids.iter().rev() {
        let _ = unsafe { libc::kill(*child_pid as libc::pid_t, libc::SIGTERM) };
    }
    wait_for_processes_to_exit(&pids, std::time::Duration::from_millis(1_000));
    let mut killed = false;
    for child_pid in pids.iter().rev() {
        if process_running(*child_pid) {
            let result = unsafe { libc::kill(*child_pid as libc::pid_t, libc::SIGKILL) };
            killed |= result == 0;
        } else {
            killed = true;
        }
    }
    killed
}

#[cfg(not(unix))]
pub(crate) fn terminate_process_tree(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn wait_for_processes_to_exit(pids: &[u32], timeout: std::time::Duration) {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout && pids.iter().any(|pid| process_running(*pid)) {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn descendant_process_ids(root_pid: u32) -> Vec<u32> {
    let output = match std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .output()
    {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let rows = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse::<u32>().ok()?;
            let ppid = parts.next()?.parse::<u32>().ok()?;
            Some((pid, ppid))
        })
        .collect::<Vec<_>>();
    let mut descendants = Vec::new();
    let mut frontier = vec![root_pid];
    while let Some(parent_pid) = frontier.pop() {
        for (pid, ppid) in &rows {
            if *ppid == parent_pid && !descendants.contains(pid) {
                descendants.push(*pid);
                frontier.push(*pid);
            }
        }
    }
    descendants
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
