//! Transfer size limits come from the machine that must hold the data, not from product constants.
//! Per-file and parser bounds stay with their parsers; totals are admitted against real free
//! disk and memory so a legitimate larger context only fails when the destination lacks room.

use std::path::Path;

/// Kept free beyond a transfer's needs so admission never fills the volume.
pub(crate) const DISK_RESERVE_BYTES: u64 = 256 * 1024 * 1024;

/// Bytes a transfer may still write below `dir` (or its nearest existing ancestor); unlimited when
/// the volume cannot be measured.
pub(crate) fn disk_budget(dir: &Path) -> u64 {
    #[cfg(test)]
    if let Some(free) = simulated_free(dir) {
        return free.saturating_sub(DISK_RESERVE_BYTES);
    }
    dir.ancestors()
        .find_map(|candidate| fs2::available_space(candidate).ok())
        .map(|free| free.saturating_sub(DISK_RESERVE_BYTES))
        .unwrap_or(u64::MAX)
}

/// Budget used when the machine cannot report its available memory (probe failed or platform
/// unsupported); deliberately small so a blind kernel is never the one to run out.
const UNMEASURED_MEMORY_BYTES: u64 = 512 * 1024 * 1024;

/// Bytes an in-memory snapshot may occupy: half of what the host reports available, capped by the
/// headroom of the process's memory cgroup (container limit minus current usage).
pub(crate) fn memory_budget() -> u64 {
    memory_budget_from(host_available_memory_bytes(), cgroup_headroom_bytes())
}

fn memory_budget_from(host: Option<u64>, cgroup: Option<u64>) -> u64 {
    host.into_iter()
        .chain(cgroup)
        .min()
        .map_or(UNMEASURED_MEMORY_BYTES, |available| available / 2)
}

#[cfg(target_os = "linux")]
fn host_available_memory_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    kib.checked_mul(1024)
}

/// Free, speculative, purgeable and file-backed pages are reclaimable without swapping.
#[cfg(target_os = "macos")]
fn host_available_memory_bytes() -> Option<u64> {
    let page_size = sysctl_u64(c"hw.pagesize")?;
    let free = sysctl_u64(c"vm.page_free_count")?;
    let reclaimable = [
        c"vm.page_speculative_count",
        c"vm.page_purgeable_count",
        c"vm.page_pageable_external_count",
    ]
    .into_iter()
    .map(|name| sysctl_u64(name).unwrap_or(0))
    .sum::<u64>();
    free.saturating_add(reclaimable).checked_mul(page_size)
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &std::ffi::CStr) -> Option<u64> {
    let mut buffer = [0_u8; 8];
    let mut length = buffer.len();
    // SAFETY: `name` is NUL-terminated and `buffer`/`length` describe writable memory of that size.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    match (status, length) {
        (0, 4) => Some(u64::from(u32::from_ne_bytes(buffer[..4].try_into().ok()?))),
        (0, 8) => Some(u64::from_ne_bytes(buffer)),
        _ => None,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn host_available_memory_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn cgroup_headroom_bytes() -> Option<u64> {
    let membership = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    cgroup_headroom_in(Path::new("/sys/fs/cgroup"), &membership)
}

#[cfg(not(target_os = "linux"))]
fn cgroup_headroom_bytes() -> Option<u64> {
    None
}

/// Smallest `limit - usage` over this process's memory cgroup and every ancestor, for cgroup v2
/// (`0::/path`) and the v1 memory controller; `None` when no level sets a limit.
#[cfg(target_os = "linux")]
fn cgroup_headroom_in(mount: &Path, membership: &str) -> Option<u64> {
    let mut headroom: Option<u64> = None;
    for line in membership.lines() {
        let [_, controllers, path] = line.splitn(3, ':').collect::<Vec<_>>()[..] else {
            continue;
        };
        let (root, files) = if controllers.is_empty() {
            (
                mount.to_path_buf(),
                ("memory.max", "memory.current", "inactive_file"),
            )
        } else if controllers.split(',').any(|name| name == "memory") {
            (
                mount.join("memory"),
                (
                    "memory.limit_in_bytes",
                    "memory.usage_in_bytes",
                    "total_inactive_file",
                ),
            )
        } else {
            continue;
        };
        let mut directory = root.join(path.trim_start_matches('/'));
        loop {
            if let Some(room) = cgroup_level_headroom(&directory, files) {
                headroom = Some(headroom.map_or(room, |smallest| smallest.min(room)));
            }
            if directory == root || !directory.pop() {
                break;
            }
        }
    }
    headroom
}

/// Headroom of one cgroup level; reclaimable page cache does not count as used.
#[cfg(target_os = "linux")]
fn cgroup_level_headroom(
    directory: &Path,
    (limit, usage, inactive): (&str, &str, &str),
) -> Option<u64> {
    const UNLIMITED: u64 = 1 << 62;
    let read = |name: &str| std::fs::read_to_string(directory.join(name)).ok();
    let limit = read(limit)?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|limit| *limit < UNLIMITED)?;
    let used = read(usage)
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(limit);
    let reclaimable = read("memory.stat")
        .and_then(|stat| {
            stat.lines().find_map(|line| {
                line.strip_prefix(inactive)?
                    .strip_prefix(' ')?
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
        })
        .unwrap_or(0);
    Some(limit.saturating_sub(used.saturating_sub(reclaimable)))
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
struct SimulatedVolume {
    prefix: std::path::PathBuf,
    capacity: u64,
    measured: bool,
}

#[cfg(test)]
thread_local! {
    static SIMULATED: std::cell::RefCell<Vec<SimulatedVolume>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn simulated_free(dir: &Path) -> Option<u64> {
    SIMULATED.with(|volumes| {
        let volumes = volumes.borrow();
        let volume = volumes
            .iter()
            .filter(|volume| dir.starts_with(&volume.prefix))
            .max_by_key(|volume| volume.prefix.components().count())?;
        let used = if volume.measured {
            tree_size(&volume.prefix)
        } else {
            0
        };
        Some(volume.capacity.saturating_sub(used))
    })
}

#[cfg(test)]
pub(crate) fn tree_size(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| tree_size(&entry.path()))
        .sum()
}

#[cfg(test)]
fn simulate<T>(volume: SimulatedVolume, body: impl FnOnce() -> T) -> T {
    SIMULATED.with(|volumes| volumes.borrow_mut().push(volume));
    let result = body();
    SIMULATED.with(|volumes| volumes.borrow_mut().pop());
    result
}

/// Runs `body` as if every volume had only `free` bytes available.
#[cfg(test)]
pub(crate) fn with_free_disk<T>(free: u64, body: impl FnOnce() -> T) -> T {
    simulate(
        SimulatedVolume {
            prefix: "/".into(),
            capacity: free,
            measured: false,
        },
        body,
    )
}

/// Runs `body` with a volume of `capacity` bytes holding everything below `root`, so free space
/// shrinks as the transfer writes there.
#[cfg(test)]
pub(crate) fn with_volume<T>(root: &Path, capacity: u64, body: impl FnOnce() -> T) -> T {
    simulate(
        SimulatedVolume {
            prefix: root.to_path_buf(),
            capacity,
            measured: true,
        },
        body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmeasured_machines_get_a_conservative_budget_instead_of_unlimited() {
        assert_eq!(memory_budget_from(None, None), UNMEASURED_MEMORY_BYTES);
        assert_eq!(memory_budget_from(Some(32 << 30), None), 16 << 30);
        assert_eq!(memory_budget_from(Some(32 << 30), Some(1 << 30)), 512 << 20);
        assert!(memory_budget() < u64::MAX);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_memory_limited_container_caps_the_budget_below_host_memory() {
        let mount = std::env::temp_dir().join(format!("chariox-cgroup-{}", std::process::id()));
        let level = |relative: &str, files: &[(&str, &str)]| {
            let directory = mount.join(relative);
            std::fs::create_dir_all(&directory).unwrap();
            for (name, value) in files {
                std::fs::write(directory.join(name), value).unwrap();
            }
        };
        // Leaf: 1 GiB limit with 600 MiB used, 100 MiB of it reclaimable cache; parent: 2 GiB limit
        // with 1.9 GiB used, so the ancestor is the tighter bound.
        level(
            "pod/leaf",
            &[
                ("memory.max", "1073741824\n"),
                ("memory.current", "629145600\n"),
                ("memory.stat", "anon 1\ninactive_file 104857600\n"),
            ],
        );
        level(
            "pod",
            &[
                ("memory.max", "2147483648\n"),
                ("memory.current", "2040109465\n"),
            ],
        );
        let headroom = cgroup_headroom_in(&mount, "0::/pod/leaf\n").unwrap();
        assert_eq!(headroom, 2_147_483_648 - 2_040_109_465);
        level(
            "pod",
            &[("memory.max", "max\n"), ("memory.current", "2040109465\n")],
        );
        assert_eq!(
            cgroup_headroom_in(&mount, "0::/pod/leaf\n"),
            Some(1_073_741_824 - 629_145_600 + 104_857_600)
        );
        level(
            "memory/old",
            &[
                ("memory.limit_in_bytes", "536870912\n"),
                ("memory.usage_in_bytes", "536870912\n"),
            ],
        );
        assert_eq!(cgroup_headroom_in(&mount, "4:memory:/old\n"), Some(0));
        assert_eq!(cgroup_headroom_in(&mount, "0::/unlimited\n"), None);
        std::fs::remove_dir_all(&mount).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_reports_reclaimable_memory_below_installed_memory() {
        let available = host_available_memory_bytes().expect("sysctl probes succeed on macOS");
        assert!(available > 0 && available <= sysctl_u64(c"hw.memsize").unwrap());
    }

    #[test]
    fn real_volumes_are_measured_and_refusals_say_how_to_recover() {
        assert!(disk_budget(&std::env::temp_dir()) > 0);
        assert_eq!(with_free_disk(0, || disk_budget(Path::new("/"))), 0);
        let message = disk_shortage("transfer", Path::new("/data"), 5 << 20, 1 << 20);
        assert!(message.contains("free space on that volume"));
        assert!(memory_shortage("snapshot", 5 << 20, 1 << 20).contains("remove unused Extensions"));
    }
}
