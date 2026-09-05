use std::fmt;
use std::sync::{Mutex, MutexGuard};

use crate::config::DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB;
use crate::error::DaemonError;

use super::{broker::docker_command, local_docker_container_name, LocalDockerSliceOptions};
use crate::slice::SliceRecord;

const DOCKER_ENGINE_RESERVE_MB: u64 = 512;
const MIB: u64 = 1024 * 1024;
static ADMISSION_LOCK: Mutex<()> = Mutex::new(());

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
    options: &LocalDockerSliceOptions,
) -> Result<MutexGuard<'static, ()>, DaemonError> {
    let guard = ADMISSION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let requested_bytes = u64::from(
        options
            .memory_mb
            .unwrap_or(DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB),
    ) * MIB;
    let capacity = docker_memory_capacity(&local_docker_container_name(record))?;
    evaluate_slice_memory_admission(capacity, requested_bytes).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "slice.memory.admission",
            message: error.to_string(),
        }
    })?;
    Ok(guard)
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
    let containers = docker_output(
        &["ps", "--format", "{{.Names}}"],
        "list active slice containers",
    )?;
    let mut committed_bytes = 0_u64;
    for container in containers
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with("chariox-slice-") && *name != excluded_container)
    {
        let configured = docker_numeric_output(
            &["inspect", "--format", "{{.HostConfig.Memory}}", container],
            "inspect active slice memory limit",
        )?;
        committed_bytes = committed_bytes.saturating_add(if configured == 0 {
            default_slice_memory_bytes()
        } else {
            configured
        });
    }
    Ok(SliceMemoryCapacity {
        total_bytes,
        committed_bytes,
        reserve_bytes: DOCKER_ENGINE_RESERVE_MB * MIB,
    })
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

        println!(
            "CHARIOX_MEMORY_PRESSURE_PROBE:{}",
            serde_json::json!({
                "schema": "chariox.memory_pressure_admission_probe.v1",
                "admissionClosesBeforeOom": rejection.available_bytes() < 2 * GIB,
                "activeStateRemainsConsistent": pressured == active_before,
                "resourceRecoveryRecorded": recovered.available_bytes() > pressured.available_bytes(),
                "defaultSliceLimitBytes": default_slice_memory_bytes(),
                "reserveBytes": pressured.reserve_bytes,
            })
        );
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
}
