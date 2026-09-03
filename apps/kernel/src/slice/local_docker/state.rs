use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::error::DaemonError;

use super::{
    broker, docker_command, ensure_host_docker_ready, local_docker_container_is_running,
    local_docker_container_name, run_local_docker_slice_screen, LocalDockerSliceOptions,
};
use crate::slice::model::{
    SliceBackendKind, SliceBackupRecord, SliceDisplayMode, SliceRecord, SliceSavedStateRecord,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct LocalDockerDefaultSavedStateRef {
    backend: SliceBackendKind,
    os: String,
    state_id: String,
    manifest_path: String,
    updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliceSnapshotQuiesce {
    Container,
    Desktop,
}

pub fn save_local_docker_slice_state(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
) -> Result<SliceSavedStateRecord, DaemonError> {
    save_local_docker_slice_state_inner(record, options, SliceSnapshotQuiesce::Container)
}

pub fn save_local_docker_slice_state_live(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
) -> Result<SliceSavedStateRecord, DaemonError> {
    save_local_docker_slice_state_inner(record, options, SliceSnapshotQuiesce::Desktop)
}

fn save_local_docker_slice_state_inner(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    quiesce: SliceSnapshotQuiesce,
) -> Result<SliceSavedStateRecord, DaemonError> {
    ensure_local_docker_state_target(record, "slice.state.save")?;
    ensure_host_docker_ready()?;
    let state_id = active_state_id(record);
    let image_ref = active_state_image_ref(&state_id);
    let state_dir = options.root.join("states").join(&state_id);
    let manifest_path = state_dir.join("manifest.json");
    let home_archive_path = active_state_home_archive_path(&state_dir);
    let previous_state = if manifest_path.exists() {
        Some(read_state_manifest::<SliceSavedStateRecord>(
            &manifest_path,
            "slice.state.save",
            "saved state manifest",
        )?)
    } else {
        None
    };
    std::fs::create_dir_all(&state_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "slice.state.save",
        message: format!(
            "failed to create slice state directory {}: {error}",
            state_dir.display()
        ),
    })?;
    with_local_docker_slice_snapshot_quiesced(record, quiesce, "slice.state.save", || {
        docker_commit_container(record, &image_ref, "slice.state.save")?;
        let archive_result = archive_local_docker_home_volume(
            record,
            options,
            &home_archive_path,
            "state",
            &state_id,
            "slice.state.save",
        );
        let (home_archive_path, size_bytes) = match archive_result {
            Ok(captured) => captured,
            Err(error) => {
                remove_docker_image_best_effort(&image_ref);
                return Err(error);
            }
        };
        let now_ms = crate::session::unix_epoch_ms();
        let state = SliceSavedStateRecord {
            id: state_id,
            slice_name: record.name.clone(),
            source_slice_id: record.id.clone(),
            backend: record.backend.clone(),
            os: record.os.clone(),
            image_ref: image_ref.clone(),
            home_archive_path: home_archive_path.display().to_string(),
            manifest_path: manifest_path.display().to_string(),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            size_bytes: Some(size_bytes),
            last_operation: Some("state.save".to_string()),
            last_operation_status: Some(crate::slice::model::SliceOperationStatus::Completed),
            last_error: None,
        };
        publish_saved_state_generation_with(
            &manifest_path,
            &state,
            previous_state.as_ref(),
            write_state_manifest,
        )?;
        Ok(state)
    })
}

pub fn create_local_docker_slice_backup(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    name: Option<&str>,
) -> Result<SliceBackupRecord, DaemonError> {
    create_local_docker_slice_backup_inner(record, options, name, SliceSnapshotQuiesce::Container)
}

pub fn create_local_docker_slice_backup_live(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    name: Option<&str>,
) -> Result<SliceBackupRecord, DaemonError> {
    create_local_docker_slice_backup_inner(record, options, name, SliceSnapshotQuiesce::Desktop)
}

fn create_local_docker_slice_backup_inner(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    name: Option<&str>,
    quiesce: SliceSnapshotQuiesce,
) -> Result<SliceBackupRecord, DaemonError> {
    ensure_local_docker_state_target(record, "slice.backup.create")?;
    ensure_host_docker_ready()?;
    let backup_id = backup_id(record, name);
    let state_id = active_state_id(record);
    let image_ref = format!("chariox-slice-backup:{backup_id}");
    let backup_dir = options.root.join("backups").join(&backup_id);
    let manifest_path = backup_dir.join("manifest.json");
    std::fs::create_dir_all(&backup_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "slice.backup.create",
        message: format!(
            "failed to create slice backup directory {}: {error}",
            backup_dir.display()
        ),
    })?;
    with_local_docker_slice_snapshot_quiesced(record, quiesce, "slice.backup.create", || {
        docker_commit_container(record, &image_ref, "slice.backup.create")?;
        let (home_archive_path, size_bytes) = archive_local_docker_home_volume(
            record,
            options,
            &backup_dir.join("home.tar.zst"),
            "backup",
            &backup_id,
            "slice.backup.create",
        )?;
        let now_ms = crate::session::unix_epoch_ms();
        let backup = SliceBackupRecord {
            id: backup_id,
            name: name
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(&record.name)
                .to_string(),
            source_slice_id: record.id.clone(),
            source_state_id: state_id,
            image_ref: image_ref.clone(),
            home_archive_path: home_archive_path.display().to_string(),
            manifest_path: manifest_path.display().to_string(),
            created_at_ms: now_ms,
            size_bytes: Some(size_bytes),
        };
        if let Err(error) = write_state_manifest(&manifest_path, &backup) {
            remove_home_archive_generation_best_effort("backup", &backup.id, &home_archive_path);
            remove_docker_image_best_effort(&image_ref);
            let _ = std::fs::remove_dir(&backup_dir);
            return Err(error);
        }
        Ok(backup)
    })
}

pub fn remove_local_docker_saved_state(state: &SliceSavedStateRecord) -> Result<(), DaemonError> {
    let _ = docker_command()
        .args(["image", "rm", "-f", &state.image_ref])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let manifest_path = PathBuf::from(&state.manifest_path);
    if let Some(dir) = manifest_path.parent() {
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(|error| DaemonError::LocalTransport {
                operation: "slice.state.reset",
                message: format!(
                    "failed to remove slice saved state directory {}: {error}",
                    dir.display()
                ),
            })?;
        }
    }
    broker::remove_home_archive("state", &state.id).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "slice.state.reset",
            message: format!("failed to remove managed home archive: {error}"),
        }
    })?;
    Ok(())
}

pub fn set_local_docker_default_saved_state(
    state: &SliceSavedStateRecord,
    options: &LocalDockerSliceOptions,
) -> Result<(), DaemonError> {
    let default_ref = LocalDockerDefaultSavedStateRef {
        backend: state.backend.clone(),
        os: state.os.clone(),
        state_id: state.id.clone(),
        manifest_path: state.manifest_path.clone(),
        updated_at_ms: crate::session::unix_epoch_ms(),
    };
    let path = default_saved_state_path(options, &state.backend, &state.os);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
            operation: "slice.state.default.set",
            message: format!(
                "failed to create slice default directory {}: {error}",
                parent.display()
            ),
        })?;
    }
    write_state_manifest(&path, &default_ref)
}

pub fn default_local_docker_saved_state(
    options: &LocalDockerSliceOptions,
    backend: SliceBackendKind,
    os: &str,
) -> Result<Option<SliceSavedStateRecord>, DaemonError> {
    let path = default_saved_state_path(options, &backend, os);
    if !path.exists() {
        return Ok(None);
    }
    let default_ref: LocalDockerDefaultSavedStateRef = read_state_manifest(
        &path,
        "slice.state.default.get",
        "default saved state reference",
    )?;
    if default_ref.backend != backend || default_ref.os != os {
        return Err(DaemonError::LocalTransport {
            operation: "slice.state.default.get",
            message: format!(
                "default saved state {} targets {:?}/{} but requested {:?}/{}",
                path.display(),
                default_ref.backend,
                default_ref.os,
                backend,
                os
            ),
        });
    }
    let manifest_path = PathBuf::from(&default_ref.manifest_path);
    if !manifest_path.exists() {
        return Err(DaemonError::LocalTransport {
            operation: "slice.state.default.get",
            message: format!(
                "default saved state `{}` manifest is missing at {}; create with --clean or save another default state",
                default_ref.state_id,
                manifest_path.display()
            ),
        });
    }
    let state: SliceSavedStateRecord = read_state_manifest(
        &manifest_path,
        "slice.state.default.get",
        "saved state manifest",
    )?;
    Ok(Some(state))
}

fn ensure_local_docker_state_target(
    record: &SliceRecord,
    operation: &'static str,
) -> Result<(), DaemonError> {
    if record.backend != SliceBackendKind::LocalDocker {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("slice `{}` is not a local Docker slice", record.name),
        });
    }
    if record.os != "linux" {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "local Docker saved state only supports linux slices, got `{}`",
                record.os
            ),
        });
    }
    if !container_exists_by_name(&local_docker_container_name(record)) {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "slice container `{}` does not exist; start the slice before saving state",
                local_docker_container_name(record)
            ),
        });
    }
    Ok(())
}

fn container_exists_by_name(container_name: &str) -> bool {
    docker_command()
        .args(["container", "inspect", container_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn stop_local_docker_container_if_running(record: &SliceRecord) -> Result<(), DaemonError> {
    if !local_docker_container_is_running(record) {
        return Ok(());
    }
    let container = local_docker_container_name(record);
    if record.display_mode == SliceDisplayMode::Headed {
        run_local_docker_slice_screen(record, "stop", "slice.state.stop_desktop")?;
    }
    let _ = docker_command()
        .args([
            "exec",
            "-u",
            "slice",
            &container,
            "bash",
            "-lc",
            "screen -S chariox-slice-relay -X quit >/dev/null 2>&1 || true; screen -S chariox-slice-kernel -X quit >/dev/null 2>&1 || true; pkill -f 'codex app-server' >/dev/null 2>&1 || true; pkill -f 'opencode serve' >/dev/null 2>&1 || true",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let status = docker_command()
        .args(["stop", &container])
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.state.stop",
            message: format!("failed to stop slice container `{container}`: {error}"),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation: "slice.state.stop",
            message: format!("docker stop `{container}` failed with status {status}"),
        })
    }
}

fn with_local_docker_slice_snapshot_quiesced<T>(
    record: &SliceRecord,
    quiesce: SliceSnapshotQuiesce,
    operation: &'static str,
    snapshot: impl FnOnce() -> Result<T, DaemonError>,
) -> Result<T, DaemonError> {
    let resume = match quiesce {
        SliceSnapshotQuiesce::Container => {
            stop_local_docker_container_if_running(record)?;
            SliceSnapshotResume::None
        }
        SliceSnapshotQuiesce::Desktop => stop_local_docker_slice_desktop_for_snapshot(record)?,
    };
    let result = snapshot();
    let resume_result = resume_after_local_docker_slice_snapshot(record, resume, operation);
    match (result, resume_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(resume_error)) => {
            tracing::warn!(
                operation,
                resume_error = %resume_error,
                "failed to resume slice desktop after snapshot error"
            );
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliceSnapshotResume {
    None,
    Desktop,
}

fn stop_local_docker_slice_desktop_for_snapshot(
    record: &SliceRecord,
) -> Result<SliceSnapshotResume, DaemonError> {
    if record.display_mode != SliceDisplayMode::Headed || !local_docker_container_is_running(record)
    {
        return Ok(SliceSnapshotResume::None);
    }
    run_local_docker_slice_screen(record, "stop", "slice.screen.stop_for_snapshot")?;
    Ok(SliceSnapshotResume::Desktop)
}

fn resume_after_local_docker_slice_snapshot(
    record: &SliceRecord,
    resume: SliceSnapshotResume,
    operation: &'static str,
) -> Result<(), DaemonError> {
    match resume {
        SliceSnapshotResume::None => Ok(()),
        SliceSnapshotResume::Desktop => {
            run_local_docker_slice_screen(record, "start", operation)?;
            Ok(())
        }
    }
}

fn docker_commit_container(
    record: &SliceRecord,
    image_ref: &str,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let container = local_docker_container_name(record);
    let status = docker_command()
        .args(["commit", &container, image_ref])
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to commit slice container `{container}`: {error}"),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "docker commit `{container}` to `{image_ref}` failed with status {status}"
            ),
        })
    }
}

fn archive_local_docker_home_volume(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    archive_path: &Path,
    archive_scope: &str,
    archive_id: &str,
    operation: &'static str,
) -> Result<(PathBuf, u64), DaemonError> {
    let volume = format!("{}-home", local_docker_container_name(record));
    let helper = format!(
        "{}-home-archive-{}",
        local_docker_container_name(record),
        crate::session::unix_epoch_ms()
    );
    let _ = docker_command()
        .args(["rm", "-f", &helper])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let result = archive_local_docker_home_volume_with_helper(
        &helper,
        &volume,
        &options.docker_image,
        archive_path,
        archive_scope,
        archive_id,
        operation,
    );
    let _ = docker_command()
        .args(["rm", "-f", &helper])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let (archive_path, size) = result?;
    if size > 0 {
        Ok((archive_path, size))
    } else {
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "home volume archive `{}` was not created or is empty",
                archive_path.display()
            ),
        })
    }
}

fn archive_local_docker_home_volume_with_helper(
    helper: &str,
    volume: &str,
    image: &str,
    archive_path: &Path,
    archive_scope: &str,
    archive_id: &str,
    operation: &'static str,
) -> Result<(PathBuf, u64), DaemonError> {
    let status = docker_command()
        .args([
            "create",
            "--name",
            helper,
            "--user",
            "root",
            "-v",
            &format!("{volume}:/home-src:ro"),
            image,
            "sleep",
            "infinity",
        ])
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to create home archive helper `{helper}`: {error}"),
        })?;
    if !status.success() {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("docker create home archive helper `{helper}` failed with {status}"),
        });
    }
    let status = docker_command()
        .args(["start", helper])
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to start home archive helper `{helper}`: {error}"),
        })?;
    if !status.success() {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("docker start home archive helper `{helper}` failed with {status}"),
        });
    }
    let output = docker_command()
        .args([
            "exec",
            "-u",
            "root",
            helper,
            "bash",
            "-lc",
            "set -euo pipefail; cd /home-src; tar --zstd -cf /tmp/home.tar.zst .",
        ])
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to archive slice home volume `{volume}`: {error}"),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "home volume archive failed with status {}: {}{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        });
    }
    if let Some(captured) = broker::capture_home_archive(helper, archive_scope, archive_id)
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to capture managed home archive: {error}"),
        })?
    {
        return Ok(captured);
    }
    let status = docker_command()
        .args([
            "cp",
            &format!("{helper}:/tmp/home.tar.zst"),
            &archive_path.display().to_string(),
        ])
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!(
                "failed to copy home archive from helper `{helper}` to `{}`: {error}",
                archive_path.display()
            ),
        })?;
    if status.success() {
        Ok((
            archive_path.to_path_buf(),
            file_size(archive_path).unwrap_or(0),
        ))
    } else {
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "docker cp home archive from helper `{helper}` to `{}` failed with {status}",
                archive_path.display()
            ),
        })
    }
}

fn active_state_id(record: &SliceRecord) -> String {
    sanitize_state_component(&record.name)
}

fn active_state_image_ref(state_id: &str) -> String {
    format!(
        "chariox-slice-state:{state_id}-{:016x}",
        rand::random::<u64>()
    )
}

pub(super) fn active_state_home_archive_path(state_dir: &Path) -> PathBuf {
    state_dir.join(format!("home-{:016x}.tar.zst", rand::random::<u64>()))
}

fn backup_id(record: &SliceRecord, name: Option<&str>) -> String {
    let label = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&record.name);
    format!(
        "{}-{}",
        sanitize_state_component(label),
        crate::session::unix_epoch_ms()
    )
}

fn sanitize_state_component(value: &str) -> String {
    let sanitized = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "slice".to_string()
    } else {
        sanitized
    }
}

fn write_state_manifest<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), DaemonError> {
    let payload =
        serde_json::to_vec_pretty(value).map_err(|error| DaemonError::LocalTransport {
            operation: "slice.state.manifest",
            message: format!("failed to encode saved state manifest: {error}"),
        })?;
    let parent = path.parent().ok_or_else(|| DaemonError::LocalTransport {
        operation: "slice.state.manifest",
        message: format!("saved state manifest {} has no parent", path.display()),
    })?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}-{:016x}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("manifest"),
        std::process::id(),
        rand::random::<u64>()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(DaemonError::LocalTransport {
            operation: "slice.state.manifest",
            message: format!(
                "failed to atomically write saved state manifest {}: {error}",
                path.display()
            ),
        });
    }
    Ok(())
}

pub(super) fn publish_saved_state_generation_with<F>(
    manifest_path: &Path,
    state: &SliceSavedStateRecord,
    previous_state: Option<&SliceSavedStateRecord>,
    publish_manifest: F,
) -> Result<(), DaemonError>
where
    F: FnOnce(&Path, &SliceSavedStateRecord) -> Result<(), DaemonError>,
{
    if let Err(error) = publish_manifest(manifest_path, state) {
        remove_home_archive_generation_best_effort(
            "state",
            &state.id,
            Path::new(&state.home_archive_path),
        );
        remove_docker_image_best_effort(&state.image_ref);
        return Err(error);
    }
    if let Some(previous) = previous_state {
        if previous.home_archive_path != state.home_archive_path {
            remove_home_archive_generation_best_effort(
                "state",
                &previous.id,
                Path::new(&previous.home_archive_path),
            );
        }
        if previous.image_ref != state.image_ref {
            remove_docker_image_best_effort(&previous.image_ref);
        }
    }
    Ok(())
}

fn remove_home_archive_generation_best_effort(scope: &str, id: &str, path: &Path) {
    match broker::remove_home_archive_path(scope, id, path) {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(
                scope,
                id,
                archive = %path.display(),
                %error,
                "failed to remove obsolete managed home archive generation"
            );
            return;
        }
    }
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                scope,
                id,
                archive = %path.display(),
                %error,
                "failed to remove obsolete local home archive generation"
            );
        }
    }
}

fn remove_docker_image_best_effort(image_ref: &str) {
    let _ = docker_command()
        .args(["image", "rm", "-f", image_ref])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn read_state_manifest<T: serde::de::DeserializeOwned>(
    path: &Path,
    operation: &'static str,
    label: &str,
) -> Result<T, DaemonError> {
    let payload = std::fs::read(path).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!("failed to read {label} {}: {error}", path.display()),
    })?;
    serde_json::from_slice(&payload).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!("failed to decode {label} {}: {error}", path.display()),
    })
}

fn default_saved_state_path(
    options: &LocalDockerSliceOptions,
    backend: &SliceBackendKind,
    os: &str,
) -> PathBuf {
    let backend = match backend {
        SliceBackendKind::LocalDocker => "local_docker",
        SliceBackendKind::SshDocker => "ssh_docker",
    };
    options
        .root
        .join("defaults")
        .join(format!("{}-{}.json", backend, sanitize_state_component(os)))
}

fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|metadata| metadata.len())
}
