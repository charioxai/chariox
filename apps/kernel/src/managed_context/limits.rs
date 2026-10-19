//! Transfer size limits come from the machine that must hold the data, not from product constants.
//! Per-file and parser bounds stay with their parsers; totals are admitted against real free
//! disk and memory so a legitimate larger context only fails when the destination lacks room.

use std::path::Path;

/// Kept free beyond a transfer's needs so admission never fills the volume.
const DISK_RESERVE_BYTES: u64 = 256 * 1024 * 1024;

/// Bytes a transfer may still write below `dir` (or its nearest existing ancestor); unlimited when
/// the volume cannot be measured.
pub(crate) fn disk_budget(dir: &Path) -> u64 {
    #[cfg(test)]
    if let Some(free) = FORCED_FREE_BYTES.with(std::cell::Cell::get) {
        return free.saturating_sub(DISK_RESERVE_BYTES);
    }
    dir.ancestors()
        .find_map(|candidate| fs2::available_space(candidate).ok())
        .map(|free| free.saturating_sub(DISK_RESERVE_BYTES))
        .unwrap_or(u64::MAX)
}

/// Bytes an in-memory snapshot may occupy: half of what the machine reports available.
pub(crate) fn memory_budget() -> u64 {
    available_memory_bytes().map_or(u64::MAX, |available| available / 2)
}

#[cfg(target_os = "linux")]
fn available_memory_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    kib.checked_mul(1024)
}

#[cfg(not(target_os = "linux"))]
fn available_memory_bytes() -> Option<u64> {
    None
}

/// Message for a refusal because the real destination lacks room, with a way to recover.
pub(crate) fn disk_shortage(what: &str, dir: &Path, written: u64, budget: u64) -> String {
    format!(
        "{what} needs more than the {} MiB that can be written below {} ({} MiB written so far); \
         free space on that volume or point CHARIOX_HOME and TMPDIR at a larger one, then retry",
        budget / (1024 * 1024),
        dir.display(),
        written / (1024 * 1024)
    )
}

pub(crate) fn memory_shortage(what: &str, used: u64, budget: u64) -> String {
    format!(
        "{what} needs more than the {} MiB of memory this machine can spare ({} MiB used so far); \
         close other programs or remove unused Extensions from the context, then retry",
        budget / (1024 * 1024),
        used / (1024 * 1024)
    )
}

#[cfg(test)]
thread_local! {
    static FORCED_FREE_BYTES: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Runs `body` as if the destination volume had only `free` bytes available.
#[cfg(test)]
pub(crate) fn with_free_disk<T>(free: u64, body: impl FnOnce() -> T) -> T {
    let previous = FORCED_FREE_BYTES.with(|cell| cell.replace(Some(free)));
    let result = body();
    FORCED_FREE_BYTES.with(|cell| cell.set(previous));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_volumes_are_measured_and_refusals_say_how_to_recover() {
        assert!(disk_budget(&std::env::temp_dir()) > 0);
        assert_eq!(with_free_disk(0, || disk_budget(Path::new("/"))), 0);
        let message = disk_shortage("transfer", Path::new("/data"), 5 << 20, 1 << 20);
        assert!(message.contains("free space on that volume"));
        assert!(memory_shortage("snapshot", 5 << 20, 1 << 20).contains("remove unused Extensions"));
    }
}
