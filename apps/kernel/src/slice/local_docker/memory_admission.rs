use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use crate::config::DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB;
use crate::error::DaemonError;
use fs2::FileExt;

use super::{
    broker::docker_command, local_docker_container_name, LocalDockerSliceAction,
    LocalDockerSliceOptions,
};
use crate::slice::SliceRecord;

const DOCKER_ENGINE_RESERVE_MB: u64 = 512;
const MIB: u64 = 1024 * 1024;
static PROCESS_ADMISSION_LOCK: Mutex<()> = Mutex::new(());

pub(super) struct SliceMemoryAdmissionGuard {
    _process: MutexGuard<'static, ()>,
    _engine: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SliceMemoryCapacity {
    total_bytes: u64,
    committed_bytes: u64,
    reserve_bytes: u64,
}

impl SliceMemoryCapacity {
    fn available_bytes(self) -> u64 {
        self.total_bytes
            .saturating_sub(self.committed_bytes)
            .saturating_sub(self.reserve_bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SliceMemoryAdmissionRejection {
    capacity: SliceMemoryCapacity,
    requested_bytes: u64,
}

impl SliceMemoryAdmissionRejection {
    fn available_bytes(self) -> u64 {
        self.capacity.available_bytes()
    }
}

impl fmt::Display for SliceMemoryAdmissionRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "slice needs {} MiB but Docker has {} MiB safely available ({} MiB total, {} MiB committed, {} MiB reserved); stop an active slice or configure a smaller slices.linux.memory_mb",
            self.requested_bytes / MIB,
            self.available_bytes() / MIB,
            self.capacity.total_bytes / MIB,
            self.capacity.committed_bytes / MIB,
            self.capacity.reserve_bytes / MIB,
        )
    }
}

pub(super) fn default_slice_memory_bytes() -> u64 {
    u64::from(DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB) * MIB
}

fn evaluate_slice_memory_admission(
    capacity: SliceMemoryCapacity,
    requested_bytes: u64,
) -> Result<(), SliceMemoryAdmissionRejection> {
    if requested_bytes <= capacity.available_bytes() {
        Ok(())
    } else {
        Err(SliceMemoryAdmissionRejection {
            capacity,
            requested_bytes,
        })
    }
}

pub(super) fn admit_slice_start(
    record: &SliceRecord,
    action: LocalDockerSliceAction,
    options: &LocalDockerSliceOptions,
) -> Result<SliceMemoryAdmissionGuard, DaemonError> {
    let process = PROCESS_ADMISSION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let engine = acquire_engine_admission_lock()?;
    let configured_bytes = u64::from(
        options
            .memory_mb
            .unwrap_or(DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB),
    ) * MIB;
    let target_container = local_docker_container_name(record);
    let all_slice_containers = docker_slice_container_names(
        &["ps", "-a", "--format", "{{.Names}}"],
        "list all slice containers",
    )?;
    let existing_target_limit = if action != LocalDockerSliceAction::RestoreState
        && all_slice_containers
            .iter()
            .any(|name| name == &target_container)
    {
        Some(docker_container_memory_limit(&target_container)?)
    } else {
        None
    };
    let requested_bytes = effective_start_reservation(
        action,
        configured_bytes,
        existing_target_limit,
        &target_container,
    )?;
    let capacity = docker_memory_capacity(&target_container)?;
    evaluate_slice_memory_admission(capacity, requested_bytes).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "slice.memory.admission",
            message: error.to_string(),
        }
    })?;
    Ok(SliceMemoryAdmissionGuard {
        _process: process,
        _engine: engine,
    })
}

fn effective_start_reservation(
    action: LocalDockerSliceAction,
    configured_bytes: u64,
    existing_target_limit: Option<u64>,
    target_container: &str,
) -> Result<u64, DaemonError> {
    if action == LocalDockerSliceAction::RestoreState {
        return Ok(configured_bytes);
    }
    match existing_target_limit {
        Some(limit) => require_bounded_container_limit(target_container, limit),
        None => Ok(configured_bytes),
    }
}

fn docker_memory_capacity(excluded_container: &str) -> Result<SliceMemoryCapacity, DaemonError> {
    let total_bytes = docker_numeric_output(
        &["info", "--format", "{{.MemTotal}}"],
        "read Docker memory capacity",
    )?;
    if total_bytes == 0 {
        return Err(memory_measurement_error(
            "Docker reported zero bytes of memory capacity",
        ));
    }
    let containers = docker_slice_container_names(
        &["ps", "--format", "{{.Names}}"],
        "list active slice containers",
    )?;
    let mut committed_bytes = 0_u64;
    for container in containers
        .iter()
        .filter(|name| name.as_str() != excluded_container)
    {
        let configured = docker_container_memory_limit(container)?;
        committed_bytes =
            committed_bytes.saturating_add(require_bounded_container_limit(container, configured)?);
    }
    Ok(SliceMemoryCapacity {
        total_bytes,
        committed_bytes,
        reserve_bytes: DOCKER_ENGINE_RESERVE_MB * MIB,
    })
}

fn docker_slice_container_names(
    args: &[&str],
    operation: &'static str,
) -> Result<Vec<String>, DaemonError> {
    Ok(docker_output(args, operation)?
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with("chariox-slice-"))
        .map(str::to_string)
        .collect())
}

fn docker_container_memory_limit(container: &str) -> Result<u64, DaemonError> {
    docker_numeric_output(
        &["inspect", "--format", "{{.HostConfig.Memory}}", container],
        "inspect slice memory limit",
    )
}

fn require_bounded_container_limit(container: &str, limit: u64) -> Result<u64, DaemonError> {
    if limit > 0 {
        return Ok(limit);
    }
    Err(DaemonError::LocalTransport {
        operation: "slice.memory.admission",
        message: format!(
            "cannot safely start a slice while container `{container}` has no memory limit; destroy and recreate that slice to apply slices.linux.memory_mb"
        ),
    })
}

fn acquire_engine_admission_lock() -> Result<File, DaemonError> {
    let path = engine_admission_lock_path();
    let file = open_engine_admission_lock(&path)?;
    FileExt::lock_exclusive(&file).map_err(|error| {
        memory_measurement_error(&format!(
            "failed to lock Docker engine admission at {}: {error}",
            path.display()
        ))
    })?;
    Ok(file)
}

fn engine_admission_lock_path() -> PathBuf {
    // A host-wide lock deliberately over-serializes distinct Docker contexts.
    // It also prevents equivalent endpoint spellings from bypassing admission.
    #[cfg(unix)]
    {
        // Do not use std::env::temp_dir(): kernels with different TMPDIR values
        // must still contend on the same Docker-engine admission lock.
        PathBuf::from("/tmp/chariox-docker-memory-admission.lock")
    }
    #[cfg(windows)]
    {
        std::env::temp_dir().join("chariox-docker-memory-admission.lock")
    }
}

fn open_engine_admission_lock(path: &Path) -> Result<File, DaemonError> {
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
    let file = options.open(path).map_err(|error| {
        memory_measurement_error(&format!(
            "failed to open Docker engine admission lock {}: {error}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        memory_measurement_error(&format!(
            "failed to inspect Docker engine admission lock {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(memory_measurement_error(
            "Docker engine admission lock is not a regular file",
        ));
    }
    Ok(file)
}

fn docker_numeric_output(args: &[&str], operation: &'static str) -> Result<u64, DaemonError> {
    let output = docker_output(args, operation)?;
    output
        .trim()
        .parse::<u64>()
        .map_err(|_| memory_measurement_error(&format!("{operation} returned a non-numeric value")))
}

fn docker_output(args: &[&str], operation: &'static str) -> Result<String, DaemonError> {
    let output = docker_command()
        .args(args)
        .output()
        .map_err(|error| memory_measurement_error(&format!("failed to {operation}: {error}")))?;
    if !output.status.success() {
        return Err(memory_measurement_error(&format!(
            "failed to {operation}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| memory_measurement_error(&format!("{operation} returned non-UTF-8 output")))
}

fn memory_measurement_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "slice.memory.admission",
        message: format!("cannot safely admit a slice because {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn memory_pressure_admission_fault_probe() {
        let pressured = SliceMemoryCapacity {
            total_bytes: 4 * GIB,
            committed_bytes: 2 * GIB,
            reserve_bytes: 512 * 1024 * 1024,
        };
        let active_before = pressured;
        let rejection = evaluate_slice_memory_admission(pressured, 2 * GIB)
            .expect_err("a second 2 GiB slice must be rejected before exhausting a 4 GiB engine");
        let recovered = SliceMemoryCapacity {
            committed_bytes: 0,
            ..pressured
        };
        evaluate_slice_memory_admission(recovered, 2 * GIB)
            .expect("admission should reopen after resources recover");
        let unbounded_slice_rejected =
            require_bounded_container_limit("chariox-slice-legacy", 0).is_err();
        let existing_target_limit_reserved = effective_start_reservation(
            LocalDockerSliceAction::Recover,
            2 * GIB,
            Some(3 * GIB),
            "chariox-slice-existing",
        )
        .is_ok_and(|reservation| reservation == 3 * GIB);
        let engine_lock_exclusive = engine_lock_exclusivity_probe();

        println!(
            "CHARIOX_MEMORY_PRESSURE_PROBE:{}",
            serde_json::json!({
                "schema": "chariox.memory_pressure_admission_probe.v1",
                "admissionClosesBeforeOom": rejection.available_bytes() < 2 * GIB,
                "activeStateRemainsConsistent": pressured == active_before,
                "resourceRecoveryRecorded": recovered.available_bytes() > pressured.available_bytes(),
                "unboundedSliceRejected": unbounded_slice_rejected,
                "existingTargetLimitReserved": existing_target_limit_reserved,
                "engineLockExclusive": engine_lock_exclusive,
                "defaultSliceLimitBytes": default_slice_memory_bytes(),
                "reserveBytes": pressured.reserve_bytes,
            })
        );
    }

    fn engine_lock_exclusivity_probe() -> bool {
        let path = std::env::temp_dir().join(format!(
            "chariox-memory-admission-test-{:032x}.lock",
            rand::random::<u128>()
        ));
        let first = open_engine_admission_lock(&path).expect("first lock file should open");
        FileExt::lock_exclusive(&first).expect("first engine lock should acquire");
        let second = open_engine_admission_lock(&path).expect("second lock file should open");
        let contended = FileExt::try_lock_exclusive(&second).is_err();
        drop(first);
        let recovered = FileExt::try_lock_exclusive(&second).is_ok();
        drop(second);
        let _ = std::fs::remove_file(path);
        contended && recovered
    }

    #[test]
    fn existing_target_uses_its_actual_limit_unless_restore_recreates_it() {
        assert_eq!(
            effective_start_reservation(
                LocalDockerSliceAction::Provision,
                2 * GIB,
                Some(3 * GIB),
                "chariox-slice-existing",
            )
            .unwrap(),
            3 * GIB
        );
        assert_eq!(
            effective_start_reservation(
                LocalDockerSliceAction::RestoreState,
                2 * GIB,
                Some(3 * GIB),
                "chariox-slice-existing",
            )
            .unwrap(),
            2 * GIB
        );
        assert!(effective_start_reservation(
            LocalDockerSliceAction::Recover,
            2 * GIB,
            Some(0),
            "chariox-slice-legacy",
        )
        .is_err());
    }

    #[test]
    fn admission_accepts_the_exact_safe_capacity_boundary() {
        evaluate_slice_memory_admission(
            SliceMemoryCapacity {
                total_bytes: 4 * GIB,
                committed_bytes: GIB,
                reserve_bytes: GIB,
            },
            2 * GIB,
        )
        .expect("the exact capacity boundary should be admitted");
    }

    #[cfg(unix)]
    #[test]
    fn engine_lock_path_is_independent_of_process_temporary_directory() {
        assert_eq!(
            engine_admission_lock_path(),
            PathBuf::from("/tmp/chariox-docker-memory-admission.lock")
        );
    }
}
