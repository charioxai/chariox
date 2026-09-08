//! Durable resume obligation for live snapshots interrupted by home-kernel death.

use std::path::PathBuf;

use crate::error::DaemonError;
use crate::slice::{SliceBackendKind, SliceDisplayMode, SliceRecord};

use super::{
    docker_command, local_docker_container_name, run_local_docker_slice_screen,
    LocalDockerSliceOptions,
};

#[cfg(all(test, unix))]
#[path = "snapshot_pause_tests.rs"]
mod tests;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResumeObligation {
    version: u32,
    slice_id: String,
    container: String,
    restart_desktop: bool,
}

fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "slice.snapshot.resume",
        message: message.into(),
    }
}

fn path(record: &SliceRecord, options: &LocalDockerSliceOptions) -> Result<PathBuf, DaemonError> {
    if record.id.is_empty()
        || !record
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
        || record.id == "."
        || record.id == ".."
    {
        return Err(error("invalid slice identity for snapshot resume record"));
    }
    Ok(options
        .root
        .join("runtime")
        .join(&record.id)
        .join("snapshot-resume.json"))
}

pub(super) fn begin(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
) -> Result<(), DaemonError> {
    let path = path(record, options)?;
    if path.try_exists().map_err(|e| error(e.to_string()))? {
        return Err(error(
            "an earlier snapshot resume obligation is still pending",
        ));
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| error(e.to_string()))?;
    // Persist newly created directory links as well as the manifest itself.
    #[cfg(unix)]
    for directory in [options.root.join("runtime"), options.root.clone()] {
        std::fs::File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|e| error(e.to_string()))?;
    }
    let obligation = ResumeObligation {
        version: 1,
        slice_id: record.id.clone(),
        container: local_docker_container_name(record),
        restart_desktop: record.display_mode == SliceDisplayMode::Headed,
    };
    match super::state::write_state_manifest(&path, &obligation)? {
        super::state::ManifestPublication::Durable => Ok(()),
        super::state::ManifestPublication::PublishedDurabilityUncertain { message } => {
            Err(error(message))
        }
    }
}

pub(crate) fn recover(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
) -> Result<(), DaemonError> {
    if record.backend != SliceBackendKind::LocalDocker || record.os != "linux" {
        return Ok(());
    }
    let path = path(record, options)?;
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(error(e.to_string())),
    };
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err(error("invalid snapshot resume record"));
    }
    let obligation: ResumeObligation = super::state::read_state_manifest(
        &path,
        "slice.snapshot.resume",
        "snapshot resume obligation",
    )?;
    let container = local_docker_container_name(record);
    if obligation.version != 1
        || obligation.slice_id != record.id
        || obligation.container != container
    {
        return Err(error("snapshot resume record does not match this slice"));
    }
    let inspected = docker_command()
        .args([
            "inspect",
            "--format",
            "{{.State.Running}} {{.State.Status}}",
            &container,
        ])
        .output()
        .map_err(|e| error(e.to_string()))?;
    let running = if inspected.status.success() {
        match String::from_utf8_lossy(&inspected.stdout).trim() {
            "true paused" => {
                let status = docker_command()
                    .args(["unpause", &container])
                    .status()
                    .map_err(|e| error(e.to_string()))?;
                if !status.success() {
                    return Err(error("failed to resume snapshot-paused container"));
                }
                true
            }
            "true running" => true,
            "false exited" | "false created" | "false dead" => false,
            _ => {
                return Err(error(
                    "unrecognized container state while resuming snapshot",
                ))
            }
        }
    } else {
        // A failed inspect can also mean Docker is down. Only a successful
        // inventory proving absence permits retiring a vanished container's lease.
        let listed = docker_command()
            .args(["ps", "-a", "--format", "{{.Names}}"])
            .output()
            .map_err(|e| error(e.to_string()))?;
        if !listed.status.success()
            || String::from_utf8_lossy(&listed.stdout)
                .lines()
                .any(|name| name == container)
        {
            return Err(error("cannot establish snapshot container state"));
        }
        false
    };
    if running && obligation.restart_desktop {
        run_local_docker_slice_screen(record, "start", "slice.snapshot.resume")?;
    }
    // Retain the obligation across unpause/desktop failures, including a second
    // kernel crash. Repeating recovery is safe after unpause already succeeded.
    std::fs::remove_file(&path).map_err(|e| error(e.to_string()))?;
    #[cfg(unix)]
    std::fs::File::open(path.parent().unwrap())
        .and_then(|file| file.sync_all())
        .map_err(|e| error(e.to_string()))?;
    Ok(())
}
