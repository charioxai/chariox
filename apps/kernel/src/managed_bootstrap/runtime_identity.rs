use super::release::{verify_managed_bootstrap_service_binding, VerifiedReleaseEvidence};
use crate::error::DaemonError;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
const MAX_ID_BYTES: u64 = 128;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManagedKernelRuntimeIdentityReport {
    pub(super) environment_id: String,
    pub(super) machine_id: String,
    pub(super) kernel_id: String,
    pub(super) generation: u64,
    pub(super) machine_credential: String,
    pub(super) linux_boot_id: String,
    pub(super) os_machine_id: String,
    pub(super) runtime_release_digest: String,
    pub(super) runtime_source_commit: String,
    pub(super) runtime_source_tree: String,
    pub(super) observed_at: String,
}

#[derive(Debug, Clone)]
pub(super) struct RuntimeIdentityHostPaths {
    pub(super) boot_id: PathBuf,
    pub(super) machine_id: PathBuf,
    pub(super) proc_self_exe: PathBuf,
    pub(super) service_unit: PathBuf,
    pub(super) service_wants: PathBuf,
}
impl Default for RuntimeIdentityHostPaths {
    fn default() -> Self {
        Self {
            boot_id: PathBuf::from("/proc/sys/kernel/random/boot_id"),
            machine_id: PathBuf::from("/etc/machine-id"),
            proc_self_exe: PathBuf::from("/proc/self/exe"),
            service_unit: PathBuf::from("/etc/systemd/system/chariox-managed-bootstrap.service"),
            service_wants: PathBuf::from(
                "/etc/systemd/system/multi-user.target.wants/chariox-managed-bootstrap.service",
            ),
        }
    }
}
pub(super) fn capture_runtime_identity_report(
    environment_id: &str,
    machine_id: &str,
    kernel_id: &str,
    generation: u64,
    machine_credential: &str,
    release: &VerifiedReleaseEvidence,
    observed_at: DateTime<Utc>,
) -> Result<ManagedKernelRuntimeIdentityReport, DaemonError> {
    capture_runtime_identity_report_with_paths(
        environment_id,
        machine_id,
        kernel_id,
        generation,
        machine_credential,
        release,
        observed_at,
        &RuntimeIdentityHostPaths::default(),
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Preserve the existing capture_runtime_identity_report_with_paths operation signature and explicit context arguments"
)]
pub(super) fn capture_runtime_identity_report_with_paths(
    environment_id: &str,
    machine_id: &str,
    kernel_id: &str,
    generation: u64,
    machine_credential: &str,
    release: &VerifiedReleaseEvidence,
    observed_at: DateTime<Utc>,
    paths: &RuntimeIdentityHostPaths,
) -> Result<ManagedKernelRuntimeIdentityReport, DaemonError> {
    let report = ManagedKernelRuntimeIdentityReport {
        environment_id: environment_id.to_string(),
        machine_id: machine_id.to_string(),
        kernel_id: kernel_id.to_string(),
        generation,
        machine_credential: machine_credential.to_string(),
        linux_boot_id: read_linux_boot_id(&paths.boot_id)?,
        os_machine_id: read_os_machine_id(&paths.machine_id)?,
        runtime_release_digest: release.digest.clone(),
        runtime_source_commit: release.source_commit.clone(),
        runtime_source_tree: release.source_tree.clone(),
        observed_at: observed_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    verify_managed_bootstrap_service_binding(release, &paths.service_unit, &paths.service_wants)?;
    verify_running_kernel_release(release, &paths.proc_self_exe)?;
    validate_runtime_identity_report(
        &report,
        environment_id,
        machine_id,
        kernel_id,
        generation,
        release,
    )
    .map(|()| report)
}

fn verify_running_kernel_release(
    release: &VerifiedReleaseEvidence,
    proc_self_exe: &Path,
) -> Result<(), DaemonError> {
    if !release.manifest_signature_verified
        || !release.manifest_digest_verified
        || !release.kernel_artifact_verified
    {
        return Err(runtime_identity_error(
            "running kernel release evidence is not verified",
        ));
    }

    let expected_kernel = release
        .active_release_path
        .join("usr/local/bin/chariox-kernel");
    let metadata = fs::symlink_metadata(&expected_kernel).map_err(|error| {
        runtime_identity_error(&format!(
            "verified kernel artifact cannot be inspected: {error}"
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(runtime_identity_error(
            "verified kernel artifact is not a regular file",
        ));
    }
    let expected_kernel = fs::canonicalize(expected_kernel).map_err(|error| {
        runtime_identity_error(&format!(
            "verified kernel artifact cannot be resolved: {error}"
        ))
    })?;
    let running_kernel = fs::canonicalize(proc_self_exe).map_err(|error| {
        runtime_identity_error(&format!(
            "running kernel executable cannot be inspected: {error}"
        ))
    })?;
    if running_kernel != expected_kernel {
        return Err(runtime_identity_error(
            "running kernel executable is not the verified managed release artifact",
        ));
    }
    Ok(())
}

pub(super) fn validate_runtime_identity_report(
    report: &ManagedKernelRuntimeIdentityReport,
    expected_environment_id: &str,
    expected_machine_id: &str,
    expected_kernel_id: &str,
    expected_generation: u64,
    release: &VerifiedReleaseEvidence,
) -> Result<(), DaemonError> {
    if !valid_runtime_identifier(&report.environment_id)
        || !valid_runtime_identifier(&report.machine_id)
        || !valid_runtime_identifier(&report.kernel_id)
        || report.environment_id != expected_environment_id
        || report.machine_id != expected_machine_id
        || report.kernel_id != expected_kernel_id
        || report.generation != expected_generation
        || !(1..=i32::MAX as u64).contains(&report.generation)
        || !valid_machine_credential(&report.machine_credential)
        || !is_linux_boot_id(&report.linux_boot_id)
        || !is_os_machine_id(&report.os_machine_id)
        || !is_release_digest(&report.runtime_release_digest)
        || report.runtime_release_digest != release.digest
        || !is_git_object_id(&report.runtime_source_commit)
        || !is_git_object_id(&report.runtime_source_tree)
        || report.runtime_source_commit != release.source_commit
        || report.runtime_source_tree != release.source_tree
        || !is_canonical_observed_at(&report.observed_at)
    {
        return Err(runtime_identity_error("runtime identity report is invalid"));
    }
    Ok(())
}

fn read_linux_boot_id(path: &Path) -> Result<String, DaemonError> {
    let value = read_canonical_line(path, "Linux boot ID")?;
    if !is_linux_boot_id(&value) {
        return Err(runtime_identity_error("Linux boot ID is malformed"));
    }
    Ok(value)
}

fn read_os_machine_id(path: &Path) -> Result<String, DaemonError> {
    let value = read_canonical_line(path, "OS machine ID")?;
    if !is_os_machine_id(&value) {
        return Err(runtime_identity_error("OS machine ID is malformed"));
    }
    Ok(value)
}

fn read_canonical_line(path: &Path, label: &str) -> Result<String, DaemonError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| runtime_identity_error(&format!("{label} cannot be read: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_ID_BYTES {
        return Err(runtime_identity_error(&format!(
            "{label} is not a bounded regular file"
        )));
    }
    let bytes = fs::read(path)
        .map_err(|error| runtime_identity_error(&format!("{label} cannot be read: {error}")))?;
    if bytes.len() as u64 > MAX_ID_BYTES {
        return Err(runtime_identity_error(&format!("{label} is too large")));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| runtime_identity_error(&format!("{label} is not UTF-8")))?;
    let value = text.strip_suffix('\n').unwrap_or(text);
    if value.is_empty()
        || value.contains('\n')
        || value.contains('\r')
        || value.trim() != value
        || value.chars().any(char::is_whitespace)
    {
        return Err(runtime_identity_error(&format!("{label} is not canonical")));
    }
    Ok(value.to_string())
}

fn is_linux_boot_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .iter()
            .all(|index| bytes.get(*index) == Some(&b'-'))
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || is_lower_hex(*byte))
}

fn is_os_machine_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(is_lower_hex)
}

fn is_release_digest(value: &str) -> bool {
    value.len() == 71 && value.starts_with("sha256:") && value[7..].bytes().all(is_lower_hex)
}

fn is_git_object_id(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(is_lower_hex)
}

fn is_canonical_observed_at(value: &str) -> bool {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
        .is_some_and(|parsed| parsed.to_rfc3339_opts(SecondsFormat::Millis, true) == value)
}

fn valid_runtime_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._:-".contains(byte)
        })
}

fn valid_machine_credential(value: &str) -> bool {
    value.strip_prefix("mcred_").is_some_and(|suffix| {
        suffix.len() >= 40
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn runtime_identity_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "observe kernel runtime identity",
        message: message.to_string(),
    }
}
