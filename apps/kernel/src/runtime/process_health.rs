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
            current_resident_set_bytes: current_resident_set_bytes(),
            peak_resident_set_bytes: peak_resident_set_bytes(),
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn current_resident_set_bytes() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let result = unsafe {
        libc::proc_pidinfo(
            std::process::id() as libc::c_int,
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
fn current_resident_set_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if resident_pages == 0 || page_size <= 0 {
        return None;
    }
    resident_pages.checked_mul(u64::try_from(page_size).ok()?)
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
fn current_resident_set_bytes() -> Option<u64> {
    None
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
    use super::KernelProcessHealthSnapshot;

    #[test]
    fn kernel_process_health_reports_current_pid() {
        let snapshot = KernelProcessHealthSnapshot::current();

        assert_eq!(snapshot.process_id, std::process::id());
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux"))]
        assert!(snapshot.current_resident_set_bytes.is_some());
    }
}
