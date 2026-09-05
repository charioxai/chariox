use std::fmt;
#[cfg(unix)]
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, MutexGuard};

use fs2::FileExt;

use crate::error::DaemonError;
use crate::slice::SliceRecord;

use super::{broker::docker_command, local_docker_container_name, LocalDockerSliceOptions};

const SNAPSHOT_DISK_RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const ARCHIVE_OVERHEAD_BYTES: u64 = 16 * 1024 * 1024;
const ARCHIVE_OVERHEAD_PERCENT: u64 = 5;
const ARCHIVE_ENTRY_OVERHEAD_BYTES: u64 = 8 * 1024;
#[cfg(unix)]
const UNIX_DISK_ADMISSION_LOCK_PATH: &str = "/tmp/chariox-docker-disk-admission.lock";
const WINDOWS_DISK_ADMISSION_LOCK_NAME: &str = r"Global\CharioxDockerDiskAdmission";
static PROCESS_DISK_ADMISSION_LOCK: Mutex<()> = Mutex::new(());

pub(super) struct SliceDiskAdmissionGuard {
    _process: MutexGuard<'static, ()>,
    _engine: DiskAdmissionLock,
}

#[cfg(unix)]
type DiskAdmissionLock = File;

#[cfg(windows)]
struct DiskAdmissionLock(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for DiskAdmissionLock {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::ReleaseMutex;

        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SliceSnapshotDiskCapacity {
    host_available_bytes: u64,
    docker_available_bytes: u64,
    reserve_bytes: u64,
    shared_storage_pool: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SliceSnapshotDiskDemand {
    home_bytes: u64,
    entry_count: u64,
    writable_layer_bytes: u64,
}

impl SliceSnapshotDiskDemand {
    fn archive_budget_bytes(self) -> u64 {
        self.home_bytes
            .saturating_add(self.home_bytes.saturating_mul(ARCHIVE_OVERHEAD_PERCENT) / 100)
            .saturating_add(
                self.entry_count
                    .saturating_mul(ARCHIVE_ENTRY_OVERHEAD_BYTES),
            )
            .saturating_add(ARCHIVE_OVERHEAD_BYTES)
    }

    fn host_required_bytes(self) -> u64 {
        self.archive_budget_bytes()
    }

    fn docker_required_bytes(self) -> u64 {
        self.archive_budget_bytes()
            .saturating_add(self.writable_layer_bytes)
    }

    fn shared_pool_required_bytes(self) -> u64 {
        self.archive_budget_bytes()
            .saturating_mul(2)
            .saturating_add(self.writable_layer_bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SliceSnapshotDiskRejection {
    capacity: SliceSnapshotDiskCapacity,
    demand: SliceSnapshotDiskDemand,
}

impl SliceSnapshotDiskRejection {
    fn host_shortfall_bytes(self) -> u64 {
        self.host_required_bytes()
            .saturating_add(self.capacity.reserve_bytes)
            .saturating_sub(self.capacity.host_available_bytes)
    }

    fn docker_shortfall_bytes(self) -> u64 {
        self.docker_required_bytes()
            .saturating_add(self.capacity.reserve_bytes)
            .saturating_sub(self.capacity.docker_available_bytes)
    }

    fn host_required_bytes(self) -> u64 {
        if self.capacity.shared_storage_pool {
            self.demand.shared_pool_required_bytes()
        } else {
            self.demand.host_required_bytes()
        }
    }

    fn docker_required_bytes(self) -> u64 {
        if self.capacity.shared_storage_pool {
            self.demand.shared_pool_required_bytes()
        } else {
            self.demand.docker_required_bytes()
        }
    }
}

impl fmt::Display for SliceSnapshotDiskRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "slice snapshot needs more disk headroom (host shortfall {} MiB, Docker shortfall {} MiB, {} MiB reserved, shared storage pool: {}); free Docker and Chariox state storage, then retry",
            self.host_shortfall_bytes() / (1024 * 1024),
            self.docker_shortfall_bytes() / (1024 * 1024),
            self.capacity.reserve_bytes / (1024 * 1024),
            self.capacity.shared_storage_pool,
        )
    }
}

fn evaluate_slice_snapshot_disk_admission(
    capacity: SliceSnapshotDiskCapacity,
    demand: SliceSnapshotDiskDemand,
) -> Result<(), SliceSnapshotDiskRejection> {
    let rejection = SliceSnapshotDiskRejection { capacity, demand };
    let admitted = capacity.host_available_bytes
        >= rejection
            .host_required_bytes()
            .saturating_add(capacity.reserve_bytes)
        && capacity.docker_available_bytes
            >= rejection
                .docker_required_bytes()
                .saturating_add(capacity.reserve_bytes);
    if admitted {
        Ok(())
    } else {
        Err(rejection)
    }
}

pub(super) fn admit_slice_snapshot(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
) -> Result<SliceDiskAdmissionGuard, DaemonError> {
    let process = PROCESS_DISK_ADMISSION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let engine = acquire_disk_admission_lock()?;
    let container = local_docker_container_name(record);
    let measurement = measure_slice_storage_with_helper(record, options)?;
    let demand = SliceSnapshotDiskDemand {
        home_bytes: measurement.home_bytes,
        entry_count: measurement.entry_count,
        writable_layer_bytes: docker_numeric_output(
            &["inspect", "--size", "--format", "{{.SizeRw}}", &container],
            "measure slice writable layer",
        )?,
    };
    let capacity = SliceSnapshotDiskCapacity {
        host_available_bytes: host_available_space(&options.root)?,
        docker_available_bytes: measurement.docker_available_bytes,
        reserve_bytes: SNAPSHOT_DISK_RESERVE_BYTES,
        shared_storage_pool: docker_and_state_share_filesystem(&options.root),
    };
    evaluate_slice_snapshot_disk_admission(capacity, demand).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "slice.disk.admission",
            message: error.to_string(),
        }
    })?;
    Ok(SliceDiskAdmissionGuard {
        _process: process,
        _engine: engine,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SliceStorageMeasurement {
    home_bytes: u64,
    entry_count: u64,
    docker_available_bytes: u64,
}

fn measure_slice_storage_with_helper(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
) -> Result<SliceStorageMeasurement, DaemonError> {
    let container = local_docker_container_name(record);
    let volume = format!("{container}-home");
    let helper = format!("{container}-disk-admission-{:016x}", rand::random::<u64>());
    remove_helper_best_effort(&helper);
    let mut created = false;
    let result = (|| {
        docker_success(
            &[
                "create",
                "--name",
                &helper,
                "--user",
                "root",
                "-v",
                &format!("{volume}:/home-src:ro"),
                &options.docker_image,
                "sleep",
                "infinity",
            ],
            "create slice disk measurement helper",
        )?;
        created = true;
        docker_success(&["start", &helper], "start slice disk measurement helper")?;
        let home_bytes = docker_numeric_field(
            &["exec", "-u", "root", &helper, "du", "-sb", "/home-src"],
            "measure slice home storage",
        )?;
        let entry_count = docker_numeric_field(
            &[
                "exec",
                "-u",
                "root",
                &helper,
                "bash",
                "-lc",
                "set -euo pipefail; find /home-src -printf . | wc -c",
            ],
            "count slice home entries",
        )?;
        let docker_available_bytes = docker_numeric_field(
            &[
                "exec",
                "-u",
                "root",
                &helper,
                "df",
                "-B1",
                "--output=avail",
                "/tmp",
            ],
            "measure Docker snapshot storage",
        )?;
        Ok(SliceStorageMeasurement {
            home_bytes,
            entry_count,
            docker_available_bytes,
        })
    })();
    let cleanup = if created {
        docker_success(
            &["rm", "-f", &helper],
            "remove slice disk measurement helper",
        )
    } else {
        Ok(())
    };
    match (result, cleanup) {
        (Ok(measurement), Ok(())) => Ok(measurement),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(error), Err(cleanup_error)) => Err(DaemonError::LocalTransport {
            operation: "slice.disk.admission",
            message: format!(
                "{error}; disk measurement helper cleanup also failed: {cleanup_error}"
            ),
        }),
    }
}

fn docker_success(args: &[&str], operation: &'static str) -> Result<(), DaemonError> {
    let output = docker_command()
        .args(args)
        .output()
        .map_err(|error| disk_measurement_error(&format!("failed to {operation}: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(disk_measurement_error(&format!(
            "failed to {operation}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn remove_helper_best_effort(helper: &str) {
    let _ = docker_command()
        .args(["rm", "-f", helper])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn host_available_space(path: &Path) -> Result<u64, DaemonError> {
    let existing = nearest_existing_ancestor(path).ok_or_else(|| {
        disk_measurement_error("Chariox state storage has no existing filesystem ancestor")
    })?;
    fs2::available_space(existing).map_err(|error| {
        disk_measurement_error(&format!(
            "failed to measure Chariox state storage at {}: {error}",
            existing.display()
        ))
    })
}

fn docker_and_state_share_filesystem(state_root: &Path) -> bool {
    let docker_root = match docker_output(
        &["info", "--format", "{{.DockerRootDir}}"],
        "locate Docker storage",
    ) {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value.trim()),
        _ => return true,
    };
    paths_share_filesystem(state_root, &docker_root).unwrap_or(true)
}

#[cfg(unix)]
fn paths_share_filesystem(left: &Path, right: &Path) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;

    let left = nearest_existing_ancestor(left)?;
    let right = nearest_existing_ancestor(right)?;
    let left_device = left.metadata().ok()?.dev();
    let right_device = right.metadata().ok()?.dev();
    Some(left_device == right_device)
}

#[cfg(not(unix))]
fn paths_share_filesystem(_left: &Path, _right: &Path) -> Option<bool> {
    None
}

fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    path.ancestors().find(|candidate| candidate.exists())
}

#[cfg(unix)]
fn acquire_disk_admission_lock() -> Result<DiskAdmissionLock, DaemonError> {
    let path = disk_admission_lock_path();
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let file = options.open(&path).map_err(|error| {
        disk_measurement_error(&format!(
            "failed to open Docker disk admission lock {}: {error}",
            path.display()
        ))
    })?;
    if !file
        .metadata()
        .map_err(|error| {
            disk_measurement_error(&format!(
                "failed to inspect Docker disk admission lock {}: {error}",
                path.display()
            ))
        })?
        .is_file()
    {
        return Err(disk_measurement_error(
            "Docker disk admission lock is not a regular file",
        ));
    }
    FileExt::lock_exclusive(&file).map_err(|error| {
        disk_measurement_error(&format!(
            "failed to lock Docker disk admission at {}: {error}",
            path.display()
        ))
    })?;
    Ok(file)
}

#[cfg(unix)]
fn disk_admission_lock_path() -> PathBuf {
    PathBuf::from(UNIX_DISK_ADMISSION_LOCK_PATH)
}

#[cfg(windows)]
fn acquire_disk_admission_lock() -> Result<DiskAdmissionLock, DaemonError> {
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject, INFINITE};

    let name = windows_disk_admission_lock_name_wide();
    let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(disk_measurement_error(&format!(
            "failed to create the global Docker disk admission mutex: {}",
            std::io::Error::last_os_error()
        )));
    }
    let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
    if matches!(wait, WAIT_OBJECT_0 | WAIT_ABANDONED) {
        Ok(DiskAdmissionLock(handle))
    } else {
        unsafe {
            CloseHandle(handle);
        }
        Err(disk_measurement_error(&format!(
            "failed to acquire the global Docker disk admission mutex: {} (wait status {wait:#x})",
            std::io::Error::last_os_error()
        )))
    }
}

fn windows_disk_admission_lock_name() -> &'static str {
    WINDOWS_DISK_ADMISSION_LOCK_NAME
}

fn windows_disk_admission_lock_name_wide() -> Vec<u16> {
    windows_disk_admission_lock_name()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

fn docker_numeric_output(args: &[&str], operation: &'static str) -> Result<u64, DaemonError> {
    let output = docker_output(args, operation)?;
    output
        .trim()
        .parse::<u64>()
        .map_err(|_| disk_measurement_error(&format!("{operation} returned a non-numeric value")))
}

fn docker_numeric_field(args: &[&str], operation: &'static str) -> Result<u64, DaemonError> {
    let output = docker_output(args, operation)?;
    output
        .split_whitespace()
        .find_map(|value| value.parse::<u64>().ok())
        .ok_or_else(|| disk_measurement_error(&format!("{operation} returned no numeric value")))
}

fn docker_output(args: &[&str], operation: &'static str) -> Result<String, DaemonError> {
    let output = docker_command()
        .args(args)
        .output()
        .map_err(|error| disk_measurement_error(&format!("failed to {operation}: {error}")))?;
    if !output.status.success() {
        return Err(disk_measurement_error(&format!(
            "failed to {operation}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| disk_measurement_error(&format!("{operation} returned non-UTF-8 output")))
}

fn disk_measurement_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "slice.disk.admission",
        message: format!("cannot safely save slice state because {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn snapshot_budget_accounts_for_archive_overhead_and_writable_layer() {
        let demand = SliceSnapshotDiskDemand {
            home_bytes: GIB,
            entry_count: 1,
            writable_layer_bytes: 512 * 1024 * 1024,
        };
        assert!(demand.host_required_bytes() > GIB);
        assert!(demand.docker_required_bytes() > demand.host_required_bytes());
    }

    #[test]
    fn shared_pool_reserves_both_archive_copies_and_the_committed_layer() {
        let demand = SliceSnapshotDiskDemand {
            home_bytes: GIB,
            entry_count: 1,
            writable_layer_bytes: 512 * 1024 * 1024,
        };
        let separate = SliceSnapshotDiskCapacity {
            host_available_bytes: 3 * GIB,
            docker_available_bytes: 3 * GIB,
            reserve_bytes: GIB,
            shared_storage_pool: false,
        };
        evaluate_slice_snapshot_disk_admission(separate, demand)
            .expect("independent storage pools have enough capacity");
        assert!(evaluate_slice_snapshot_disk_admission(
            SliceSnapshotDiskCapacity {
                shared_storage_pool: true,
                ..separate
            },
            demand,
        )
        .is_err());
        assert_eq!(
            demand.shared_pool_required_bytes(),
            demand
                .archive_budget_bytes()
                .saturating_mul(2)
                .saturating_add(demand.writable_layer_bytes)
        );
    }

    #[test]
    fn archive_budget_scales_with_many_small_files() {
        let demand = SliceSnapshotDiskDemand {
            home_bytes: 0,
            entry_count: 1_000_000,
            writable_layer_bytes: 0,
        };
        assert!(demand.archive_budget_bytes() >= 8_000_000_000);
    }

    #[test]
    fn windows_disk_lock_name_is_machine_wide() {
        assert_eq!(
            windows_disk_admission_lock_name(),
            r"Global\CharioxDockerDiskAdmission"
        );
        assert!(!windows_disk_admission_lock_name().contains(':'));
        assert!(!windows_disk_admission_lock_name().contains('%'));
        let wide = windows_disk_admission_lock_name_wide();
        assert_eq!(wide.last(), Some(&0));
        assert!(!wide[..wide.len() - 1].contains(&0));
    }
}
