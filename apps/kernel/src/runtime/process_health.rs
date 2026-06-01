use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelProcessHealthSnapshot {
    pub process_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_resident_set_bytes: Option<u64>,
}

impl KernelProcessHealthSnapshot {
    pub(crate) fn current() -> Self {
        Self {
            process_id: std::process::id(),
            peak_resident_set_bytes: peak_resident_set_bytes(),
        }
    }
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
    }
}
