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
    #[serde(default = "default_resume_container")]
    resume_container: bool,
    #[serde(default)]
    helpers: Vec<String>,
}

fn default_resume_container() -> bool {
    true
}

pub(super) fn is_snapshot_helper(name: &str) -> bool {
    if !name.starts_with("chariox-slice-") {
        return false;
    }
    name.rsplit_once("-disk-admission-")
        .is_some_and(|(_, suffix)| {
            suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
        })
        || name
            .rsplit_once("-home-archive-")
            .is_some_and(|(_, suffix)| {
                !suffix.is_empty()
                    && suffix.len() <= 20
                    && suffix.bytes().all(|b| b.is_ascii_digit())
            })
}

fn owns_helper(container: &str, helper: &str) -> bool {
    helper.strip_prefix(container).is_some_and(|suffix| {
        suffix.strip_prefix("-disk-admission-").is_some_and(|id| {
            id.len() == 16
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) || suffix.strip_prefix("-home-archive-").is_some_and(|id| {
            !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
        })
    })
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
    resume_container: bool,
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
        restart_desktop: resume_container && record.display_mode == SliceDisplayMode::Headed,
        resume_container,
        helpers: Vec::new(),
    };
    match super::state::write_state_manifest(&path, &obligation)? {
        super::state::ManifestPublication::Durable => Ok(()),
        super::state::ManifestPublication::PublishedDurabilityUncertain { message } => {
            Err(error(message))
        }
    }
}

pub(super) fn register_helper(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    helper: &str,
) -> Result<(), DaemonError> {
    let path = path(record, options)?;
    let mut obligation: ResumeObligation = super::state::read_state_manifest(
        &path,
        "slice.snapshot.resume",
        "snapshot resume obligation",
    )?;
    if obligation.version != 1
        || obligation.slice_id != record.id
        || obligation.container != local_docker_container_name(record)
        || !owns_helper(&obligation.container, helper)
        || obligation.helpers.len() >= 2
        || obligation.helpers.iter().any(|name| name == helper)
    {
        return Err(error("invalid snapshot helper registration"));
    }
    obligation.helpers.push(helper.to_string());
    match super::state::write_state_manifest(&path, &obligation)? {
        super::state::ManifestPublication::Durable => Ok(()),
        super::state::ManifestPublication::PublishedDurabilityUncertain { message } => {
            Err(error(message))
        }
    }
}

pub(super) fn create_helper(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    helper: &str,
) -> Result<(), DaemonError> {
    register_helper(record, options, helper)?;
    let volume = format!("{}-home:/home-src:ro", local_docker_container_name(record));
    let owner = format!("io.chariox.snapshot-helper={helper}");
    let created = docker_command()
        .args([
            "create",
            "--name",
            helper,
            "--memory",
            "512m",
            "--cpus",
            "1",
            "--pids-limit",
            "64",
            "--network",
            "none",
            "--label",
            &owner,
            "--user",
            "root",
            "-v",
            &volume,
            &options.docker_image,
            "sleep",
            "infinity",
        ])
        .status()
        .map_err(|e| error(e.to_string()))?;
    if !created.success() {
        return Err(error("failed to create bounded snapshot helper"));
    }
    Ok(())
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
        || obligation.helpers.len() > 2
        || obligation
            .helpers
            .iter()
            .any(|helper| !owns_helper(&container, helper))
    {
        return Err(error("snapshot resume record does not match this slice"));
    }
    if !obligation.helpers.is_empty() {
        let listed = docker_command()
            .args(["ps", "-a", "--format", "{{.Names}}"])
            .output()
            .map_err(|e| error(e.to_string()))?;
        if !listed.status.success() {
            return Err(error("cannot inspect snapshot helpers"));
        }
        let names = String::from_utf8_lossy(&listed.stdout);
        for helper in &obligation.helpers {
            if names.lines().any(|name| name == helper) {
                let owner = docker_command()
                    .args([
                        "inspect",
                        "--format",
                        "{{index .Config.Labels \"io.chariox.snapshot-helper\"}}",
                        helper,
                    ])
                    .output()
                    .map_err(|e| error(e.to_string()))?;
                if !owner.status.success()
                    || String::from_utf8_lossy(&owner.stdout).trim() != helper
                {
                    return Err(error("snapshot helper ownership could not be verified"));
                }
                let removed = docker_command()
                    .args(["rm", "-f", helper])
                    .status()
                    .map_err(|e| error(e.to_string()))?;
                if !removed.success() {
                    return Err(error("cannot remove abandoned snapshot helper"));
                }
            }
        }
    }
    if obligation.resume_container {
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
