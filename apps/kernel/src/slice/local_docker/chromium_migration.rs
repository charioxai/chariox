//! Physical migration operations. The kernel's saved-state registry owns the
//! checkpoint and lifecycle; Docker labels only bind physical object identity.
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::slice::{SliceOperationStatus, SliceRecord, SliceSavedStateRecord};

use super::{
    broker, docker_command, local_docker_container_name, LocalDockerSliceOptions,
    LocalDockerSliceRelay,
};

pub(crate) const MIGRATION_OPERATION: &str = "chromium.sandbox.migrate";
const SOURCE_LABEL: &str = "io.chariox.chromium-source-container";
// The restore script bounds each Docker operation and removes its helper on
// exit. Leave enough time for those bounds and helper cleanup to complete.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Container {
    pub id: String,
    pub running: bool,
    pub policy: Option<String>,
    pub migration: Option<String>,
    mounts: Vec<Mount>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Mount {
    #[serde(rename = "Type")]
    kind: String,
    name: Option<String>,
    destination: String,
}

pub(crate) fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "slice.chromium.migrate",
        message: message.into(),
    }
}

pub(crate) fn prepare_host(record: &SliceRecord) -> Result<(), DaemonError> {
    if record.backend != crate::slice::SliceBackendKind::LocalDocker || record.os != "linux" {
        return Err(error(
            "Chromium migration requires a Linux local Docker slice",
        ));
    }
    super::ensure_host_docker_ready()
}

fn command(args: &[&str]) -> Result<String, DaemonError> {
    command_with_timeout(args, Duration::from_secs(30))
}

fn command_with_timeout(args: &[&str], timeout: Duration) -> Result<String, DaemonError> {
    let output = docker_command()
        .args(args)
        .output_with_timeout(timeout)
        .map_err(|cause| error(format!("Docker migration operation failed: {cause}")))?;
    if !output.status.success() {
        return Err(error("Docker migration operation failed; retained checkpoint and containers require recovery"));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| error("Docker returned invalid migration metadata"))
}

pub(crate) fn inspect(name: &str) -> Result<Option<Container>, DaemonError> {
    let names = command(&["container", "ls", "--all", "--format", "{{.Names}}"])?;
    if !names.lines().any(|entry| entry == name) {
        return Ok(None);
    }
    let output = command(&[
        "container",
        "inspect",
        "--format",
        r#"{"id":{{json .Id}},"running":{{json .State.Running}},"policy":{{json (index .Config.Labels "io.chariox.chromium-seccomp")}},"migration":{{json (index .Config.Labels "io.chariox.chromium-migration")}},"mounts":{{json .Mounts}}}"#,
        name,
    ])?;
    let container: Container = serde_json::from_str(&output)
        .map_err(|_| error("Docker returned invalid container identity"))?;
    validate_container_id(&container.id)?;
    Ok(Some(container))
}

fn validate_container_id(value: &str) -> Result<(), DaemonError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(error("invalid immutable Docker container ID"))
    }
}

pub(crate) fn policy_digest() -> Result<String, DaemonError> {
    use std::io::Read;
    let path = super::linux_docker_slice_script()?.with_file_name("chromium-seccomp.json");
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .and_then(|file| file.take(128 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|cause| error(format!("cannot read Chromium policy: {cause}")))?;
    if bytes.len() > 128 * 1024 {
        return Err(error("Chromium policy exceeds its bound"));
    }
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn rollback_name(record: &SliceRecord, checkpoint: &SliceSavedStateRecord) -> String {
    format!(
        "{}-rollback-{}",
        local_docker_container_name(record),
        checkpoint.id
    )
}

pub(crate) fn canonical_name(record: &SliceRecord) -> String {
    local_docker_container_name(record)
}

pub(crate) fn require_home(container: &Container, record: &SliceRecord) -> Result<(), DaemonError> {
    let expected = format!("{}-home", local_docker_container_name(record));
    let mounts = container
        .mounts
        .iter()
        .filter(|mount| mount.destination == "/home/slice")
        .collect::<Vec<_>>();
    if mounts.len() != 1
        || mounts[0].kind != "volume"
        || mounts[0].name.as_deref() != Some(expected.as_str())
    {
        return Err(error(
            "container does not own the expected slice home volume; migration left it untouched",
        ));
    }
    Ok(())
}

pub(crate) fn checkpoint(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    source: &Container,
) -> Result<SliceSavedStateRecord, DaemonError> {
    validate_container_id(&source.id)?;
    require_home(source, record)?;
    if source.running {
        // Old screen scripts can hang on CDP close, and headless slices also
        // have a browser. Use the current bounded helper before the existing
        // full-container snapshot quiesces the provider/kernel processes.
        let helper = super::linux_docker_slice_script()?
            .with_file_name("docker")
            .join("slice-screen.sh");
        let temporary = format!(
            "/tmp/chariox-chromium-stop-{:016x}.sh",
            rand::random::<u64>()
        );
        command(&[
            "cp",
            &helper.to_string_lossy(),
            &format!("{}:{temporary}", source.id),
        ])?;
        let stopped = command_with_timeout(
            &[
                "exec",
                "-u",
                "slice",
                &source.id,
                "timeout",
                "--kill-after=2s",
                "60s",
                "bash",
                &temporary,
                "stop",
            ],
            Duration::from_secs(90),
        );
        let cleaned = command(&["exec", "-u", "root", &source.id, "rm", "-f", &temporary]);
        stopped?;
        cleaned?;
    }
    super::state::save_chromium_migration_checkpoint(record, options, &source.id)
}

pub(super) fn with_checkpoint_source_stopped<T>(
    source_id: &str,
    snapshot: impl FnOnce() -> Result<T, DaemonError>,
) -> Result<T, DaemonError> {
    with_source_stopped(source_id, command_with_timeout, snapshot)
}

fn with_source_stopped<T>(
    source_id: &str,
    mut run: impl FnMut(&[&str], Duration) -> Result<String, DaemonError>,
    snapshot: impl FnOnce() -> Result<T, DaemonError>,
) -> Result<T, DaemonError> {
    validate_container_id(source_id)?;
    let inspect = [
        "container",
        "inspect",
        "--format",
        "{{.State.Running}}",
        source_id,
    ];
    match run(&inspect, Duration::from_secs(30))?.trim() {
        "false" => {}
        "true" => {
            // Browser shutdown already ran the current trusted helper. Stop
            // provider/kernel processes through the same lifecycle commands,
            // using the immutable ID even if its old name is now unrelated.
            let _ = run(&[
                "exec", "-u", "slice", source_id, "bash", "-lc",
                "screen -S chariox-slice-relay -X quit >/dev/null 2>&1 || true; screen -S chariox-slice-kernel -X quit >/dev/null 2>&1 || true; pkill -f 'codex app-server' >/dev/null 2>&1 || true; pkill -f 'opencode serve' >/dev/null 2>&1 || true",
            ], Duration::from_secs(30));
            run(
                &["stop", "--time", "30", source_id],
                Duration::from_secs(60),
            )?;
            if run(&inspect, Duration::from_secs(30))?.trim() != "false" {
                return Err(error(
                    "migration source did not remain stopped; checkpoint not captured",
                ));
            }
        }
        _ => {
            return Err(error(
                "invalid migration source running state; checkpoint not captured",
            ))
        }
    }
    snapshot()
}

pub(super) fn commit_checkpoint_image(source_id: &str, image: &str) -> Result<(), DaemonError> {
    validate_container_id(source_id)?;
    command_with_timeout(
        &[
            "commit",
            "--change",
            &format!("LABEL {SOURCE_LABEL}={source_id}"),
            source_id,
            image,
        ],
        Duration::from_secs(300),
    )?;
    Ok(())
}

pub(super) fn archive_checkpoint_home(
    record: &SliceRecord,
    image: &str,
    path: &std::path::Path,
) -> Result<(std::path::PathBuf, u64), DaemonError> {
    let helper = format!(
        "{}-home-archive-{:016x}",
        canonical_name(record),
        rand::random::<u64>()
    );
    let volume = format!("{}-home:/home-src:ro", canonical_name(record));
    with_created_helper(
        &[
            "create", "--name", &helper, "--user", "root", "-v", &volume, image, "sleep",
            "infinity",
        ],
        command_with_timeout,
        |id, run| {
            run(&["start", id], Duration::from_secs(60))?;
            run(
                &[
                    "exec",
                    "-u",
                    "root",
                    id,
                    "timeout",
                    "--kill-after=2s",
                    "300s",
                    "bash",
                    "-lc",
                    "set -euo pipefail; cd /home-src; tar --zstd -cf /tmp/home.tar.zst .",
                ],
                Duration::from_secs(330),
            )?;
            run(
                &[
                    "cp",
                    &format!("{id}:/tmp/home.tar.zst"),
                    &path.to_string_lossy(),
                ],
                Duration::from_secs(300),
            )?;
            let size = std::fs::metadata(path)
                .map_err(|cause| error(format!("cannot inspect migration home archive: {cause}")))?
                .len();
            if size == 0 {
                return Err(error("migration home archive is empty"));
            }
            Ok((path.to_owned(), size))
        },
    )
}

fn with_created_helper<T>(
    create: &[&str],
    mut run: impl FnMut(&[&str], Duration) -> Result<String, DaemonError>,
    operation: impl FnOnce(
        &str,
        &mut dyn FnMut(&[&str], Duration) -> Result<String, DaemonError>,
    ) -> Result<T, DaemonError>,
) -> Result<T, DaemonError> {
    // Never pre-remove a generated name. Creation failure cannot authorize
    // cleanup, and every subsequent command targets the returned immutable ID.
    let output = run(create, Duration::from_secs(60))?;
    let id = output.trim();
    validate_container_id(id)?;
    let result = operation(id, &mut run);
    let cleanup = run(&["rm", "--force", id], Duration::from_secs(30));
    let value = result?;
    cleanup?;
    Ok(value)
}

pub(crate) fn source_id(checkpoint: &SliceSavedStateRecord) -> Result<String, DaemonError> {
    let id = command(&[
        "image",
        "inspect",
        "--format",
        &format!("{{{{index .Config.Labels {SOURCE_LABEL:?}}}}}"),
        &checkpoint.image_ref,
    ])?;
    let id = id.trim().to_owned();
    validate_container_id(&id)?;
    Ok(id)
}

pub(crate) fn rename(id: &str, name: &str) -> Result<(), DaemonError> {
    validate_container_id(id)?;
    command(&["rename", id, name])?;
    Ok(())
}

pub(crate) fn remove_candidate(
    candidate: &Container,
    record: &SliceRecord,
    checkpoint: &SliceSavedStateRecord,
) -> Result<(), DaemonError> {
    validate_container_id(&candidate.id)?;
    require_home(candidate, record)?;
    if candidate.migration.as_deref() != Some(checkpoint.id.as_str()) {
        return Err(error(
            "replacement container identity differs; it was left untouched",
        ));
    }
    // Use the immutable ID, never a name that another container could acquire.
    command(&["rm", "--force", &candidate.id])?;
    Ok(())
}

pub(crate) fn remove_original(original: &Container, source_id: &str) -> Result<(), DaemonError> {
    validate_container_id(source_id)?;
    if original.id != source_id || original.running {
        return Err(error(
            "rollback container identity/state differs; it was left untouched",
        ));
    }
    command(&["rm", &original.id])?;
    Ok(())
}

pub(crate) fn provision(
    record: &SliceRecord,
    relay: Option<LocalDockerSliceRelay>,
    options: &LocalDockerSliceOptions,
    checkpoint: Option<&SliceSavedStateRecord>,
) -> Result<(), DaemonError> {
    let options = checkpoint.map_or_else(
        || options.clone(),
        |state| options.clone().with_saved_state(state),
    );
    super::run_local_docker_slice_action_with_migration(
        record,
        crate::slice::LocalDockerSliceAction::Provision,
        relay,
        None,
        None,
        &options,
        checkpoint.map(|state| state.id.as_str()),
    )
}

fn control(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    action: &str,
    migration_id: Option<&str>,
) -> Result<(), DaemonError> {
    if broker::configured() {
        return Err(error(
            "ordinary local migration cannot bypass the managed Docker broker",
        ));
    }
    let mut command = Command::new(super::linux_docker_slice_script()?);
    command.arg(action);
    super::configure_local_docker_slice_command(&mut command, record, None, options, false)?;
    if let Some(id) = migration_id {
        command.env("CHARIOX_SLICE_CHROMIUM_MIGRATION_ID", id);
    }
    let output = broker::bounded_local_output(&mut command, CONTROL_TIMEOUT)
        .map_err(|cause| error(format!("Chromium migration control failed: {cause}")))?;
    if !output.status.success() {
        return Err(error(format!(
            "Chromium migration {action} failed; checkpoint retained"
        )));
    }
    Ok(())
}

pub(crate) fn restore_home(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    checkpoint: &SliceSavedStateRecord,
) -> Result<(), DaemonError> {
    control(
        record,
        &options.clone().with_saved_state(checkpoint),
        "restore-migration-home",
        Some(&checkpoint.id),
    )
}

pub(crate) fn verify(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
) -> Result<(), DaemonError> {
    control(record, options, "verify-chromium-sandbox", None)
}

pub(crate) fn mark_checkpoint(
    checkpoint: &mut SliceSavedStateRecord,
    status: SliceOperationStatus,
) -> Result<(), DaemonError> {
    checkpoint.last_operation_status = Some(status);
    checkpoint.updated_at_ms = crate::session::unix_epoch_ms();
    super::state::update_checkpoint_manifest(checkpoint)
}

#[cfg(test)]
mod tests;
