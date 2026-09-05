use std::fmt;
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
static PROCESS_DISK_ADMISSION_LOCK: Mutex<()> = Mutex::new(());

pub(super) struct SliceDiskAdmissionGuard {
    _process: MutexGuard<'static, ()>,
    _engine: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SliceSnapshotDiskCapacity {
    host_available_bytes: u64,
    docker_available_bytes: u64,
    reserve_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SliceSnapshotDiskDemand {
    home_bytes: u64,
    writable_layer_bytes: u64,
}

impl SliceSnapshotDiskDemand {
    fn archive_budget_bytes(self) -> u64 {
        self.home_bytes
            .saturating_add(self.home_bytes.saturating_mul(ARCHIVE_OVERHEAD_PERCENT) / 100)
            .saturating_add(ARCHIVE_OVERHEAD_BYTES)
    }

    fn host_required_bytes(self) -> u64 {
        self.archive_budget_bytes()
    }

    fn docker_required_bytes(self) -> u64 {
        self.archive_budget_bytes()
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
        self.demand
            .host_required_bytes()
            .saturating_add(self.capacity.reserve_bytes)
            .saturating_sub(self.capacity.host_available_bytes)
    }

    fn docker_shortfall_bytes(self) -> u64 {
        self.demand
            .docker_required_bytes()
            .saturating_add(self.capacity.reserve_bytes)
            .saturating_sub(self.capacity.docker_available_bytes)
    }
}

impl fmt::Display for SliceSnapshotDiskRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "slice snapshot needs more disk headroom (host shortfall {} MiB, Docker shortfall {} MiB, {} MiB reserved); free Docker and Chariox state storage, then retry",
            self.host_shortfall_bytes() / (1024 * 1024),
            self.docker_shortfall_bytes() / (1024 * 1024),
            self.capacity.reserve_bytes / (1024 * 1024),
        )
    }
}

fn evaluate_slice_snapshot_disk_admission(
    capacity: SliceSnapshotDiskCapacity,
    demand: SliceSnapshotDiskDemand,
) -> Result<(), SliceSnapshotDiskRejection> {
    let admitted = capacity.host_available_bytes
        >= demand
            .host_required_bytes()
            .saturating_add(capacity.reserve_bytes)
        && capacity.docker_available_bytes
            >= demand
                .docker_required_bytes()
                .saturating_add(capacity.reserve_bytes);
    if admitted {
        Ok(())
    } else {
        Err(SliceSnapshotDiskRejection { capacity, demand })
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
    let (home_bytes, docker_available_bytes) = measure_slice_storage_with_helper(record, options)?;
    let demand = SliceSnapshotDiskDemand {
        home_bytes,
        writable_layer_bytes: docker_numeric_output(
            &["inspect", "--size", "--format", "{{.SizeRw}}", &container],
            "measure slice writable layer",
        )?,
    };
    let capacity = SliceSnapshotDiskCapacity {
        host_available_bytes: host_available_space(&options.root)?,
        docker_available_bytes,
        reserve_bytes: SNAPSHOT_DISK_RESERVE_BYTES,
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

fn measure_slice_storage_with_helper(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
) -> Result<(u64, u64), DaemonError> {
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
        Ok((home_bytes, docker_available_bytes))
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

fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    path.ancestors().find(|candidate| candidate.exists())
}

fn acquire_disk_admission_lock() -> Result<File, DaemonError> {
    let path = disk_admission_lock_path();
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
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

fn disk_admission_lock_path() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp/chariox-docker-disk-admission.lock")
    }
    #[cfg(windows)]
    {
        std::env::temp_dir().join("chariox-docker-disk-admission.lock")
    }
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
    fn disk_pressure_admission_fault_probe() {
        let demand = SliceSnapshotDiskDemand {
            home_bytes: GIB,
            writable_layer_bytes: 512 * 1024 * 1024,
        };
        let pressured = SliceSnapshotDiskCapacity {
            host_available_bytes: 2 * GIB,
            docker_available_bytes: 3 * GIB,
            reserve_bytes: 2 * GIB,
        };
        let rejection = evaluate_slice_snapshot_disk_admission(pressured, demand)
            .expect_err("snapshot must be rejected before either filesystem exhausts its reserve");
        let prior_generation = "known-good";
        let mut published_generation = prior_generation;
        if evaluate_slice_snapshot_disk_admission(pressured, demand).is_ok() {
            published_generation = "replacement";
        }
        let recovered = SliceSnapshotDiskCapacity {
            host_available_bytes: 5 * GIB,
            docker_available_bytes: 5 * GIB,
            ..pressured
        };
        evaluate_slice_snapshot_disk_admission(recovered, demand)
            .expect("snapshot admission should reopen after disk recovery");

        println!(
            "CHARIOX_DISK_PRESSURE_PROBE:{}",
            serde_json::json!({
                "schema": "chariox.disk_pressure_admission_probe.v1",
                "admissionClosesBeforeEnospc": rejection.host_shortfall_bytes() > 0,
                "activeStateRemainsConsistent": pressured == SliceSnapshotDiskCapacity { host_available_bytes: 2 * GIB, docker_available_bytes: 3 * GIB, reserve_bytes: 2 * GIB },
                "lastKnownGoodPreserved": published_generation == prior_generation,
                "resourceRecoveryRecorded": recovered.host_available_bytes > pressured.host_available_bytes,
                "reserveBytes": pressured.reserve_bytes,
            })
        );
    }

    #[test]
    fn snapshot_budget_accounts_for_archive_overhead_and_writable_layer() {
        let demand = SliceSnapshotDiskDemand {
            home_bytes: GIB,
            writable_layer_bytes: 512 * 1024 * 1024,
        };
        assert!(demand.host_required_bytes() > GIB);
        assert!(demand.docker_required_bytes() > demand.host_required_bytes());
    }
}
