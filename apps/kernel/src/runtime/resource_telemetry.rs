#[cfg(target_os = "linux")]
use std::path::Path;

use crate::config::DaemonConfig;
use crate::error::DaemonError;
#[cfg(target_os = "linux")]
use crate::local::{
    KernelResourceTelemetryDisk, KernelResourceTelemetryLogs, KernelResourceTelemetryMemory,
    KernelResourceTelemetryMetadata, KernelResourceTelemetryProcess,
    KERNEL_RESOURCE_TELEMETRY_SCHEMA,
};
use crate::local::{KernelResourceTelemetrySnapshot, LocalDaemonResponse};

pub(crate) fn execute_kernel_resource_telemetry_request(
    config: DaemonConfig,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::KernelResourceTelemetry {
        snapshot: collect_kernel_resource_telemetry(&config)?,
    })
}

#[cfg(target_os = "linux")]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::mem::MaybeUninit;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
static MONOTONIC_START: OnceLock<Instant> = OnceLock::new();
#[cfg(target_os = "linux")]
static LAST_CAPTURED_AT_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
static LAST_WALL_CAPTURED_AT_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
const CPU_SAMPLE_WINDOW_MS: u64 = 100;

#[cfg(target_os = "linux")]
pub(crate) fn collect_kernel_resource_telemetry(
    config: &DaemonConfig,
) -> Result<KernelResourceTelemetrySnapshot, DaemonError> {
    let target_id = config.host_machine_id.trim();
    if target_id.is_empty() {
        return Err(telemetry_error("kernel machine identity is missing"));
    }

    let (cpu_percent, cpu_sample_window_ms) = read_cpu_metrics()?;
    let memory = read_memory_metrics()?;
    let log_root = crate::logging::default_log_root();
    let disk = read_disk_metrics(&log_root)?;
    let process = read_process_metrics()?;
    let logs = KernelResourceTelemetryLogs {
        bytes: read_log_bytes(&log_root)?,
    };
    let (captured_at, captured_at_monotonic_ms) = capture_timestamp()?;

    Ok(KernelResourceTelemetrySnapshot {
        schema: KERNEL_RESOURCE_TELEMETRY_SCHEMA.to_string(),
        captured_at,
        captured_at_monotonic_ms,
        telemetry: KernelResourceTelemetryMetadata {
            scope: "managed-target".to_string(),
            authoritative: true,
            target_id: target_id.to_string(),
            source: "kernel".to_string(),
        },
        cpu_percent,
        cpu_sample_window_ms,
        memory,
        disk,
        process,
        logs,
    })
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuCounters {
    total: u64,
    idle: u64,
}

#[cfg(target_os = "linux")]
fn read_cpu_metrics() -> Result<(u32, u64), DaemonError> {
    let before = read_cpu_counters()?;
    std::thread::sleep(Duration::from_millis(CPU_SAMPLE_WINDOW_MS));
    let after = read_cpu_counters()?;
    Ok((cpu_percent_between(before, after)?, CPU_SAMPLE_WINDOW_MS))
}

#[cfg(target_os = "linux")]
fn read_cpu_counters() -> Result<CpuCounters, DaemonError> {
    let contents = fs::read_to_string("/proc/stat")
        .map_err(|error| telemetry_error(format!("read /proc/stat: {error}")))?;
    parse_cpu_counters(&contents)
}

#[cfg(target_os = "linux")]
fn parse_cpu_counters(contents: &str) -> Result<CpuCounters, DaemonError> {
    let line = contents
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or_else(|| telemetry_error("/proc/stat is missing aggregate CPU counters"))?;
    let fields = line
        .split_whitespace()
        .skip(1)
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|error| telemetry_error(format!("parse /proc/stat CPU counter: {error}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if fields.len() < 4 {
        return Err(telemetry_error(
            "/proc/stat aggregate CPU counters are incomplete",
        ));
    }
    let total = fields.iter().take(8).try_fold(0_u64, |total, value| {
        total
            .checked_add(*value)
            .ok_or_else(|| telemetry_error("/proc/stat aggregate CPU counters overflow"))
    })?;
    let idle = fields[3]
        .checked_add(fields.get(4).copied().unwrap_or(0))
        .ok_or_else(|| telemetry_error("/proc/stat idle CPU counters overflow"))?;
    Ok(CpuCounters { total, idle })
}

#[cfg(target_os = "linux")]
fn cpu_percent_between(before: CpuCounters, after: CpuCounters) -> Result<u32, DaemonError> {
    let total = after
        .total
        .checked_sub(before.total)
        .ok_or_else(|| telemetry_error("/proc/stat aggregate CPU counters moved backwards"))?;
    let idle = after
        .idle
        .checked_sub(before.idle)
        .ok_or_else(|| telemetry_error("/proc/stat idle CPU counters moved backwards"))?;
    if total == 0 || idle > total {
        return Err(telemetry_error(
            "/proc/stat CPU sample has no valid elapsed capacity",
        ));
    }
    let busy = total - idle;
    let rounded_up = busy
        .checked_mul(100)
        .and_then(|value| value.checked_add(total - 1))
        .ok_or_else(|| telemetry_error("/proc/stat CPU percentage overflows"))?
        / total;
    u32::try_from(rounded_up.min(100))
        .map_err(|_| telemetry_error("/proc/stat CPU percentage is invalid"))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn collect_kernel_resource_telemetry(
    _config: &DaemonConfig,
) -> Result<KernelResourceTelemetrySnapshot, DaemonError> {
    Err(telemetry_error(
        "kernel resource telemetry is unsupported on this platform",
    ))
}

#[cfg(target_os = "linux")]
fn read_memory_metrics() -> Result<KernelResourceTelemetryMemory, DaemonError> {
    let contents = fs::read_to_string("/proc/meminfo")
        .map_err(|error| telemetry_error(format!("read /proc/meminfo: {error}")))?;
    let total_bytes = read_meminfo_kib(&contents, "MemTotal")?;
    let available_bytes = read_meminfo_kib(&contents, "MemAvailable")?;
    let used_bytes = checked_used_bytes(total_bytes, available_bytes)?;
    if total_bytes == 0 {
        return Err(telemetry_error("/proc/meminfo reported zero total memory"));
    }
    Ok(KernelResourceTelemetryMemory {
        used_bytes,
        total_bytes,
        available_bytes,
    })
}

#[cfg(target_os = "linux")]
fn read_meminfo_kib(contents: &str, key: &str) -> Result<u64, DaemonError> {
    let line = contents
        .lines()
        .find(|line| {
            line.split_once(':')
                .is_some_and(|(name, _)| name.trim() == key)
        })
        .ok_or_else(|| telemetry_error(format!("/proc/meminfo is missing {key}")))?;
    let (_, value) = line
        .split_once(':')
        .ok_or_else(|| telemetry_error(format!("/proc/meminfo has an invalid {key} line")))?;
    let mut fields = value.split_whitespace();
    let value = fields
        .next()
        .ok_or_else(|| telemetry_error(format!("/proc/meminfo has no value for {key}")))?
        .parse::<u64>()
        .map_err(|error| telemetry_error(format!("parse /proc/meminfo {key}: {error}")))?;
    if fields.next() != Some("kB") {
        return Err(telemetry_error(format!(
            "/proc/meminfo {key} is not expressed in kB"
        )));
    }
    value
        .checked_mul(1024)
        .ok_or_else(|| telemetry_error(format!("/proc/meminfo {key} overflows bytes")))
}

#[cfg(target_os = "linux")]
fn read_disk_metrics(path: &Path) -> Result<KernelResourceTelemetryDisk, DaemonError> {
    if !path.is_dir() {
        return Err(telemetry_error(format!(
            "kernel log root is missing: {}",
            path.display()
        )));
    }
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| telemetry_error("kernel log root contains an invalid path"))?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    let status = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if status != 0 {
        return Err(telemetry_error(format!(
            "statvfs kernel log root: {}",
            std::io::Error::last_os_error()
        )));
    }
    let stats = unsafe { stats.assume_init() };
    let block_size = u64::try_from(stats.f_frsize)
        .map_err(|_| telemetry_error("statvfs reported an invalid fragment size"))?;
    let total_blocks = u64::try_from(stats.f_blocks)
        .map_err(|_| telemetry_error("statvfs reported an invalid block count"))?;
    let available_blocks = u64::try_from(stats.f_bavail)
        .map_err(|_| telemetry_error("statvfs reported an invalid available block count"))?;
    if block_size == 0 {
        return Err(telemetry_error("statvfs reported zero fragment size"));
    }
    let total_bytes = total_blocks
        .checked_mul(block_size)
        .ok_or_else(|| telemetry_error("statvfs total bytes overflow"))?;
    let available_bytes = available_blocks
        .checked_mul(block_size)
        .ok_or_else(|| telemetry_error("statvfs available bytes overflow"))?;
    let used_bytes = checked_used_bytes(total_bytes, available_bytes)?;
    if total_bytes == 0 {
        return Err(telemetry_error("statvfs reported zero total disk bytes"));
    }
    Ok(KernelResourceTelemetryDisk {
        used_bytes,
        total_bytes,
        available_bytes,
    })
}

#[cfg(target_os = "linux")]
fn read_process_metrics() -> Result<KernelResourceTelemetryProcess, DaemonError> {
    let entries = fs::read_dir("/proc")
        .map_err(|error| telemetry_error(format!("read /proc process entries: {error}")))?;
    let mut count = 0_u64;
    for entry in entries {
        let entry =
            entry.map_err(|error| telemetry_error(format!("read /proc process entry: {error}")))?;
        if entry.file_name().to_string_lossy().parse::<u32>().is_ok() {
            count = count
                .checked_add(1)
                .ok_or_else(|| telemetry_error("process count overflow"))?;
        }
    }
    if count == 0 {
        return Err(telemetry_error("/proc reported no processes"));
    }

    let statm = fs::read_to_string("/proc/self/statm")
        .map_err(|error| telemetry_error(format!("read /proc/self/statm: {error}")))?;
    let resident_pages = statm
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| telemetry_error("/proc/self/statm is missing resident pages"))?
        .parse::<u64>()
        .map_err(|error| telemetry_error(format!("parse /proc/self/statm: {error}")))?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let page_size = u64::try_from(page_size)
        .ok()
        .filter(|size| *size > 0)
        .ok_or_else(|| telemetry_error("kernel page size is unavailable"))?;
    let rss_bytes = resident_pages
        .checked_mul(page_size)
        .ok_or_else(|| telemetry_error("process RSS bytes overflow"))?;

    Ok(KernelResourceTelemetryProcess { count, rss_bytes })
}

#[cfg(target_os = "linux")]
fn read_log_bytes(path: &Path) -> Result<u64, DaemonError> {
    let entries = fs::read_dir(path)
        .map_err(|error| telemetry_error(format!("read kernel log root: {error}")))?;
    let mut bytes = 0_u64;
    for entry in entries {
        let entry =
            entry.map_err(|error| telemetry_error(format!("read kernel log entry: {error}")))?;
        if !entry
            .file_type()
            .map_err(|error| telemetry_error(format!("inspect kernel log entry: {error}")))?
            .is_file()
        {
            continue;
        }
        if entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("ndjson")
        {
            continue;
        }
        bytes = bytes
            .checked_add(
                entry
                    .metadata()
                    .map_err(|error| telemetry_error(format!("stat kernel log entry: {error}")))?
                    .len(),
            )
            .ok_or_else(|| telemetry_error("kernel log byte count overflow"))?;
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn capture_timestamp() -> Result<(String, u64), DaemonError> {
    let monotonic_start = MONOTONIC_START.get_or_init(Instant::now);
    let monotonic_ms = monotonic_start
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX);
    let captured_at_monotonic_ms = monotonic_max(&LAST_CAPTURED_AT_MS, monotonic_ms);
    let wall_ms = crate::session::unix_epoch_ms();
    let wall_ms = monotonic_max(&LAST_WALL_CAPTURED_AT_MS, wall_ms);
    let wall_ms = i64::try_from(wall_ms)
        .map_err(|_| telemetry_error("kernel wall-clock timestamp overflows i64"))?;
    let captured_at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(wall_ms)
        .ok_or_else(|| telemetry_error("kernel wall-clock timestamp is invalid"))?
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok((captured_at, captured_at_monotonic_ms))
}

#[cfg(target_os = "linux")]
fn monotonic_max(slot: &AtomicU64, candidate: u64) -> u64 {
    let mut current = slot.load(Ordering::Relaxed);
    loop {
        if candidate <= current {
            return current;
        }
        match slot.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return candidate,
            Err(observed) => current = observed,
        }
    }
}

#[cfg(target_os = "linux")]
fn checked_used_bytes(total_bytes: u64, available_bytes: u64) -> Result<u64, DaemonError> {
    total_bytes
        .checked_sub(available_bytes)
        .ok_or_else(|| telemetry_error("available bytes exceed total bytes"))
}

fn telemetry_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "collect kernel resource telemetry",
        message: message.into(),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn kernel_resource_snapshot_is_complete_and_monotonic() {
        let log_root = crate::logging::default_log_root();
        std::fs::create_dir_all(&log_root).expect("kernel log root should be available");
        let first = collect_kernel_resource_telemetry(&DaemonConfig::for_tests())
            .expect("kernel telemetry should be available");
        let second = collect_kernel_resource_telemetry(&DaemonConfig::for_tests())
            .expect("kernel telemetry should remain available");

        assert_eq!(first.schema, KERNEL_RESOURCE_TELEMETRY_SCHEMA);
        assert_eq!(first.telemetry.scope, "managed-target");
        assert!(first.telemetry.authoritative);
        assert_eq!(first.telemetry.source, "kernel");
        assert!(first.cpu_percent <= 100);
        assert_eq!(first.cpu_sample_window_ms, CPU_SAMPLE_WINDOW_MS);
        assert!(first.memory.used_bytes <= first.memory.total_bytes);
        assert!(first.memory.available_bytes <= first.memory.total_bytes);
        assert!(first.disk.used_bytes <= first.disk.total_bytes);
        assert!(first.disk.available_bytes <= first.disk.total_bytes);
        assert!(first.process.count > 0);
        assert!(second.captured_at_monotonic_ms >= first.captured_at_monotonic_ms);
        assert!(second.captured_at >= first.captured_at);
    }

    #[test]
    fn kernel_resource_snapshot_fails_closed_when_availability_exceeds_capacity() {
        assert!(checked_used_bytes(10, 11).is_err());
    }

    #[test]
    fn cpu_metrics_parse_aggregate_counters_and_round_busy_percent_up() {
        let before =
            parse_cpu_counters("cpu  100 10 20 800 40 5 5 20 0 0\ncpu0 1 1 1 1 1 1 1 1 0 0\n")
                .expect("aggregate counters should parse");
        let after =
            parse_cpu_counters("cpu  130 10 30 850 50 5 5 20 0 0\ncpu0 1 1 1 1 1 1 1 1 0 0\n")
                .expect("later aggregate counters should parse");

        assert_eq!(
            before,
            CpuCounters {
                total: 1_000,
                idle: 840
            }
        );
        assert_eq!(
            after,
            CpuCounters {
                total: 1_100,
                idle: 900
            }
        );
        assert_eq!(cpu_percent_between(before, after).unwrap(), 40);
        assert_eq!(
            cpu_percent_between(
                CpuCounters {
                    total: 100,
                    idle: 90
                },
                CpuCounters {
                    total: 201,
                    idle: 190
                },
            )
            .unwrap(),
            1,
        );
    }

    #[test]
    fn cpu_metrics_fail_closed_for_missing_or_invalid_elapsed_counters() {
        assert!(parse_cpu_counters("intr 1\n").is_err());
        assert!(parse_cpu_counters("cpu 1 2 3\n").is_err());
        assert!(cpu_percent_between(
            CpuCounters {
                total: 100,
                idle: 50
            },
            CpuCounters {
                total: 100,
                idle: 50
            },
        )
        .is_err());
        assert!(cpu_percent_between(
            CpuCounters {
                total: 100,
                idle: 50
            },
            CpuCounters {
                total: 200,
                idle: 160
            },
        )
        .is_err());
    }
}
