use crate::error::DaemonError;
use std::process::Command;

// MP-08/MP-11: snapshot documented inherited settings into common typed options.
// Both execution adapters receive these explicit values; no ambient environment
// or broker control capability is copied to the privileged service.
struct DockerTuning {
    pids: u32,
    nofile: u32,
    minimum_free_mb: u32,
}

fn value(
    name: &'static str,
    default: u32,
    minimum: u32,
    maximum: u32,
    leading_zeroes: bool,
) -> Result<u32, DaemonError> {
    let raw = match std::env::var(name) {
        Ok(raw) if !raw.is_empty() => raw,
        Ok(_) | Err(std::env::VarError::NotPresent) => return Ok(default),
        Err(_) => return Err(invalid(name)),
    };
    if !raw.bytes().all(|b| b.is_ascii_digit()) || (!leading_zeroes && raw.starts_with('0')) {
        return Err(invalid(name));
    }
    raw.parse::<u32>()
        .ok()
        .filter(|v| *v >= minimum && *v <= maximum)
        .ok_or_else(|| invalid(name))
}
fn invalid(name: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "slice.local_docker",
        message: format!("{name} is invalid"),
    }
}

pub(super) fn project(command: &mut Command) -> Result<(), DaemonError> {
    let options = DockerTuning {
        pids: value(
            "CHARIOX_SLICE_DOCKER_PIDS_LIMIT",
            1024,
            1,
            2_147_483_647,
            false,
        )?,
        nofile: value(
            "CHARIOX_SLICE_DOCKER_NOFILE_LIMIT",
            8192,
            1024,
            1_048_576,
            false,
        )?,
        minimum_free_mb: value("CHARIOX_SLICE_MIN_FREE_MB", 256, 0, u32::MAX, true)?,
    };
    command
        .env("CHARIOX_SLICE_DOCKER_PIDS_LIMIT", options.pids.to_string())
        .env(
            "CHARIOX_SLICE_DOCKER_NOFILE_LIMIT",
            options.nofile.to_string(),
        )
        .env(
            "CHARIOX_SLICE_MIN_FREE_MB",
            options.minimum_free_mb.to_string(),
        );
    Ok(())
}
