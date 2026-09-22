use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

use super::release::{verify_managed_bootstrap_service_binding, VerifiedReleaseEvidence};
use super::state::BootstrapConfig;

const LINUX_BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
const OS_MACHINE_ID_PATH: &str = "/etc/machine-id";
const PROC_ROOT: &str = "/proc";
const MANAGED_BOOTSTRAP_SERVICE_PATH: &str =
    "/etc/systemd/system/chariox-managed-bootstrap.service";
const MANAGED_BOOTSTRAP_WANTS_PATH: &str =
    "/etc/systemd/system/multi-user.target.wants/chariox-managed-bootstrap.service";
const MAX_ID_BYTES: u64 = 128;
const MAX_PROC_COMM_BYTES: u64 = 256;
const MAX_PROC_STAT_BYTES: u64 = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManagedKernelFreshnessEvidence {
    pub(super) linux_boot_id: String,
    pub(super) os_machine_id: String,
    pub(super) runtime_release_digest: String,
    pub(super) runtime_source_commit: String,
    pub(super) runtime_source_tree: String,
    pub(super) residue_checks: ManagedKernelResidueChecks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManagedKernelResidueChecks {
    pub(super) old_services_absent: bool,
    pub(super) old_processes_absent: bool,
    pub(super) old_state_absent: bool,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FreshnessHostPaths {
    pub(super) boot_id: PathBuf,
    pub(super) machine_id: PathBuf,
    pub(super) proc_root: PathBuf,
    pub(super) service_unit: PathBuf,
    pub(super) service_wants: PathBuf,
}

impl Default for FreshnessHostPaths {
    fn default() -> Self {
        Self {
            boot_id: PathBuf::from(LINUX_BOOT_ID_PATH),
            machine_id: PathBuf::from(OS_MACHINE_ID_PATH),
            proc_root: PathBuf::from(PROC_ROOT),
            service_unit: PathBuf::from(MANAGED_BOOTSTRAP_SERVICE_PATH),
            service_wants: PathBuf::from(MANAGED_BOOTSTRAP_WANTS_PATH),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProcessObservation {
    pub(super) pid: u32,
    pub(super) parent_pid: u32,
    pub(super) executable_name: String,
    pub(super) executable_path: PathBuf,
}

pub(super) fn capture_freshness_evidence(
    config: &BootstrapConfig,
    release: &VerifiedReleaseEvidence,
) -> Result<ManagedKernelFreshnessEvidence, DaemonError> {
    capture_freshness_evidence_with_paths(config, release, &FreshnessHostPaths::default())
}

pub(super) fn capture_freshness_evidence_with_paths(
    config: &BootstrapConfig,
    release: &VerifiedReleaseEvidence,
    paths: &FreshnessHostPaths,
) -> Result<ManagedKernelFreshnessEvidence, DaemonError> {
    let linux_boot_id = read_linux_boot_id(&paths.boot_id)?;
    let os_machine_id = read_os_machine_id(&paths.machine_id)?;
    verify_managed_bootstrap_service_binding(release, &paths.service_unit, &paths.service_wants)?;
    let processes = collect_process_observations(&paths.proc_root)?;
    validate_process_observations(&processes, std::process::id(), &release.active_release_path)?;
    validate_state_residue(config)?;

    Ok(ManagedKernelFreshnessEvidence {
        linux_boot_id,
        os_machine_id,
        runtime_release_digest: release.digest.clone(),
        runtime_source_commit: release.source_commit.clone(),
        runtime_source_tree: release.source_tree.clone(),
        residue_checks: ManagedKernelResidueChecks {
            old_services_absent: true,
            old_processes_absent: true,
            old_state_absent: true,
        },
    })
}

pub(super) fn valid_provider_rebuild_action_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=18).contains(&bytes.len())
        && bytes[0].is_ascii_digit()
        && bytes[0] != b'0'
        && bytes[1..].iter().all(|byte| byte.is_ascii_digit())
}

pub(super) fn capture_old_generation_runtime_identity_report(
    environment_id: &str,
    machine_id: &str,
    kernel_id: &str,
    generation: u64,
    machine_credential: &str,
    release: &VerifiedReleaseEvidence,
    observed_at: DateTime<Utc>,
) -> Result<ManagedKernelRuntimeIdentityReport, DaemonError> {
    capture_old_generation_runtime_identity_report_with_paths(
        environment_id,
        machine_id,
        kernel_id,
        generation,
        machine_credential,
        release,
        observed_at,
        &FreshnessHostPaths::default(),
    )
}

pub(super) fn capture_old_generation_runtime_identity_report_with_paths(
    environment_id: &str,
    machine_id: &str,
    kernel_id: &str,
    generation: u64,
    machine_credential: &str,
    release: &VerifiedReleaseEvidence,
    observed_at: DateTime<Utc>,
    paths: &FreshnessHostPaths,
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
    validate_old_generation_runtime_identity_report(
        &report,
        environment_id,
        machine_id,
        kernel_id,
        generation,
        release,
    )
    .map(|()| report)
}

pub(super) fn validate_old_generation_runtime_identity_report(
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
        return Err(freshness_error(
            "old-generation runtime identity report is invalid",
        ));
    }
    Ok(())
}

pub(super) fn validate_freshness_evidence(
    evidence: &ManagedKernelFreshnessEvidence,
    expected_release_digest: &str,
) -> Result<(), DaemonError> {
    if !is_linux_boot_id(&evidence.linux_boot_id)
        || !is_os_machine_id(&evidence.os_machine_id)
        || !is_release_digest(&evidence.runtime_release_digest)
        || evidence.runtime_release_digest != expected_release_digest
        || !is_git_object_id(&evidence.runtime_source_commit)
        || !is_git_object_id(&evidence.runtime_source_tree)
        || evidence.residue_checks
            != (ManagedKernelResidueChecks {
                old_services_absent: true,
                old_processes_absent: true,
                old_state_absent: true,
            })
    {
        return Err(freshness_error("managed freshness evidence is invalid"));
    }
    Ok(())
}

fn read_linux_boot_id(path: &Path) -> Result<String, DaemonError> {
    let value = read_canonical_line(path, "Linux boot ID")?;
    if !is_linux_boot_id(&value) {
        return Err(freshness_error("Linux boot ID is malformed"));
    }
    Ok(value)
}

fn read_os_machine_id(path: &Path) -> Result<String, DaemonError> {
    let value = read_canonical_line(path, "OS machine ID")?;
    if !is_os_machine_id(&value) {
        return Err(freshness_error("OS machine ID is malformed"));
    }
    Ok(value)
}

fn read_canonical_line(path: &Path, label: &str) -> Result<String, DaemonError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| freshness_error(&format!("{label} cannot be read: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_ID_BYTES {
        return Err(freshness_error(&format!(
            "{label} is not a bounded regular file"
        )));
    }
    let bytes = fs::read(path)
        .map_err(|error| freshness_error(&format!("{label} cannot be read: {error}")))?;
    if bytes.len() as u64 > MAX_ID_BYTES {
        return Err(freshness_error(&format!("{label} is too large")));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| freshness_error(&format!("{label} is not UTF-8")))?;
    let value = text.strip_suffix('\n').unwrap_or(text);
    if value.is_empty()
        || value.contains('\n')
        || value.contains('\r')
        || value.trim() != value
        || value.chars().any(char::is_whitespace)
    {
        return Err(freshness_error(&format!("{label} is not canonical")));
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

fn collect_process_observations(proc_root: &Path) -> Result<Vec<ProcessObservation>, DaemonError> {
    let entries = fs::read_dir(proc_root)
        .map_err(|error| freshness_error(&format!("process table cannot be inspected: {error}")))?;
    let mut observations = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            freshness_error(&format!("process table cannot be inspected: {error}"))
        })?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse().ok())
        else {
            continue;
        };
        let process_dir = entry.path();
        let executable_path = match fs::read_link(process_dir.join("exe")) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(freshness_error(&format!(
                    "process executable cannot be inspected: {error}"
                )))
            }
        };
        let executable_name = bounded_proc_text(
            &process_dir.join("comm"),
            MAX_PROC_COMM_BYTES,
            "process name",
        )?
        .trim_end_matches('\n')
        .to_string();
        let stat = bounded_proc_text(
            &process_dir.join("stat"),
            MAX_PROC_STAT_BYTES,
            "process stat",
        )?;
        let parent_pid = parse_parent_pid(&stat)?;
        observations.push(ProcessObservation {
            pid,
            parent_pid,
            executable_name,
            executable_path,
        });
    }
    Ok(observations)
}

fn bounded_proc_text(path: &Path, max_bytes: u64, label: &str) -> Result<String, DaemonError> {
    let bytes = fs::read(path)
        .map_err(|error| freshness_error(&format!("{label} cannot be inspected: {error}")))?;
    if bytes.len() as u64 > max_bytes {
        return Err(freshness_error(&format!("{label} is too large")));
    }
    String::from_utf8(bytes).map_err(|_| freshness_error(&format!("{label} is not UTF-8")))
}

fn parse_parent_pid(stat: &str) -> Result<u32, DaemonError> {
    let closing_name = stat
        .rfind(')')
        .ok_or_else(|| freshness_error("process stat is malformed"))?;
    let fields = stat[closing_name + 1..]
        .split_whitespace()
        .collect::<Vec<_>>();
    fields
        .get(1)
        .ok_or_else(|| freshness_error("process stat is malformed"))?
        .parse::<u32>()
        .map_err(|_| freshness_error("process parent ID is malformed"))
}

pub(super) fn validate_process_observations(
    observations: &[ProcessObservation],
    current_pid: u32,
    active_release_path: &Path,
) -> Result<(), DaemonError> {
    let by_pid = observations
        .iter()
        .map(|observation| (observation.pid, observation))
        .collect::<BTreeMap<_, _>>();
    let current = by_pid
        .get(&current_pid)
        .ok_or_else(|| freshness_error("current managed bootstrap process is not observable"))?;
    if !is_current_managed_bootstrap(current, active_release_path) {
        return Err(freshness_error(
            "current process is not bound to the managed release",
        ));
    }
    for observation in observations {
        if is_bubblewrap(observation) {
            return Err(freshness_error(
                "Bubblewrap process or ancestor residue is present",
            ));
        }
        if is_chariox_runtime(observation) && observation.pid != current_pid {
            return Err(freshness_error(
                "retired Chariox process residue is present",
            ));
        }
    }
    let mut pid = current_pid;
    let mut seen = BTreeMap::new();
    while let Some(observation) = by_pid.get(&pid) {
        if seen.insert(pid, ()).is_some() {
            return Err(freshness_error("process ancestor chain is cyclic"));
        }
        if is_bubblewrap(observation) {
            return Err(freshness_error(
                "Bubblewrap process or ancestor residue is present",
            ));
        }
        if observation.parent_pid == 0 {
            break;
        }
        if observation.parent_pid == pid {
            return Err(freshness_error("process ancestor chain is cyclic"));
        }
        if !by_pid.contains_key(&observation.parent_pid) {
            return Err(freshness_error("process ancestor is not observable"));
        }
        pid = observation.parent_pid;
    }
    Ok(())
}

fn is_current_managed_bootstrap(
    observation: &ProcessObservation,
    active_release_path: &Path,
) -> bool {
    observation.executable_name.trim() == "chariox-managed-bootstrap"
        && observation.executable_path
            == active_release_path.join("usr/local/bin/chariox-managed-bootstrap")
}

fn is_bubblewrap(observation: &ProcessObservation) -> bool {
    matches!(observation.executable_name.trim(), "bwrap" | "bubblewrap")
        || matches!(
            observation
                .executable_path
                .file_name()
                .and_then(|value| value.to_str()),
            Some("bwrap" | "bubblewrap")
        )
}

fn is_chariox_runtime(observation: &ProcessObservation) -> bool {
    matches!(
        observation.executable_name.trim(),
        "chariox" | "chariox-kernel" | "chariox-relay" | "chariox-managed-bootstrap"
    ) || matches!(
        observation
            .executable_path
            .file_name()
            .and_then(|value| value.to_str()),
        Some("chariox" | "chariox-kernel" | "chariox-relay" | "chariox-managed-bootstrap")
    )
}

fn validate_state_residue(config: &BootstrapConfig) -> Result<(), DaemonError> {
    let state_dir = config
        .receipt_path
        .parent()
        .ok_or_else(|| freshness_error("managed receipt has no state directory"))?;
    let receipt_name = config
        .receipt_path
        .file_name()
        .ok_or_else(|| freshness_error("managed receipt has no file name"))?;
    for entry in fs::read_dir(state_dir)
        .map_err(|error| freshness_error(&format!("managed state cannot be inspected: {error}")))?
    {
        let entry = entry.map_err(|error| {
            freshness_error(&format!("managed state cannot be inspected: {error}"))
        })?;
        let path = entry.path();
        let allowed_receipt = entry.file_name() == receipt_name;
        let allowed_envelope = path == config.envelope_path;
        if !allowed_receipt && !allowed_envelope {
            return Err(freshness_error("retired managed state residue is present"));
        }
    }
    for path in [
        state_dir.join("release-override.json"),
        state_dir.join("disposable-worker"),
        config.process_home.join("managed"),
        config.process_home.join("disposable-worker"),
    ] {
        if fs::symlink_metadata(&path).is_ok() {
            return Err(freshness_error("retired managed state residue is present"));
        }
    }
    Ok(())
}

fn freshness_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "produce managed bootstrap freshness evidence",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::Value;

    use super::{
        is_linux_boot_id, is_os_machine_id, is_release_digest, parse_parent_pid,
        validate_old_generation_runtime_identity_report, validate_process_observations,
        ManagedKernelFreshnessEvidence, ManagedKernelResidueChecks,
        ManagedKernelRuntimeIdentityReport, ProcessObservation,
    };
    use crate::managed_bootstrap::release::VerifiedReleaseEvidence;
    use std::path::PathBuf;

    #[test]
    fn identity_and_release_fields_are_strictly_canonical() {
        assert!(is_linux_boot_id("01234567-89ab-cdef-0123-456789abcdef"));
        assert!(!is_linux_boot_id("01234567-89ab-cdef-0123-456789ABCDEf"));
        assert!(is_os_machine_id(&"a".repeat(32)));
        assert!(!is_os_machine_id(&"A".repeat(32)));
        assert!(is_release_digest(&format!("sha256:{}", "a".repeat(64))));
        assert!(!is_release_digest(&format!("sha256:{}", "A".repeat(64))));
    }

    #[test]
    fn process_observation_rejects_bubblewrap_and_retired_runtime() {
        let release = PathBuf::from("/release");
        let current = ProcessObservation {
            pid: 10,
            parent_pid: 0,
            executable_name: "chariox-managed-bootstrap".to_string(),
            executable_path: release.join("usr/local/bin/chariox-managed-bootstrap"),
        };
        let bwrap = ProcessObservation {
            pid: 11,
            parent_pid: 10,
            executable_name: "bwrap".to_string(),
            executable_path: PathBuf::from("/usr/bin/bwrap"),
        };
        assert!(validate_process_observations(&[current.clone(), bwrap], 10, &release).is_err());

        let retired = ProcessObservation {
            pid: 12,
            parent_pid: 0,
            executable_name: "chariox-kernel".to_string(),
            executable_path: PathBuf::from("/old/usr/local/bin/chariox-kernel"),
        };
        assert!(validate_process_observations(&[current, retired], 10, &release).is_err());
    }

    #[test]
    fn process_stat_parser_does_not_accept_missing_parent_fields() {
        assert_eq!(parse_parent_pid("(chariox) S 1 2 3").unwrap(), 1);
        assert!(parse_parent_pid("(chariox)").is_err());
    }

    #[test]
    fn freshness_wire_has_only_new_observations() {
        let evidence = ManagedKernelFreshnessEvidence {
            linux_boot_id: "01234567-89ab-cdef-0123-456789abcdef".to_string(),
            os_machine_id: "a".repeat(32),
            runtime_release_digest: format!("sha256:{}", "b".repeat(64)),
            runtime_source_commit: "c".repeat(40),
            runtime_source_tree: "d".repeat(40),
            residue_checks: ManagedKernelResidueChecks {
                old_services_absent: true,
                old_processes_absent: true,
                old_state_absent: true,
            },
        };
        let value: Value = serde_json::to_value(evidence).expect("freshness evidence serializes");
        assert_eq!(value["linuxBootId"], "01234567-89ab-cdef-0123-456789abcdef");
        assert!(value.get("providerRebuildActionId").is_none());
        assert!(value.get("oldKernelIdentityReport").is_none());
        assert!(value["residueChecks"].get("oldGenerationRetired").is_none());
        assert!(value["residueChecks"]
            .get("oldRelayRealmDisabled")
            .is_none());
        assert!(value["residueChecks"].get("oldHeartbeatsAbsent").is_none());
        assert_eq!(
            value["residueChecks"]
                .as_object()
                .expect("residue object")
                .len(),
            3
        );
    }

    #[test]
    fn old_generation_report_is_current_identity_only_and_strictly_bound() {
        let release = VerifiedReleaseEvidence {
            digest: format!("sha256:{}", "b".repeat(64)),
            source_commit: "c".repeat(40),
            source_tree: "d".repeat(40),
            target: "x86_64-unknown-linux-gnu".to_string(),
            active_release_path: PathBuf::from("/release"),
            manifest_signature_verified: true,
            manifest_digest_verified: true,
            kernel_artifact_verified: true,
        };
        let report = ManagedKernelRuntimeIdentityReport {
            environment_id: "environment-1".to_string(),
            machine_id: "machine-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            generation: 4,
            machine_credential: format!("mcred_{}", "x".repeat(40)),
            linux_boot_id: "01234567-89ab-cdef-0123-456789abcdef".to_string(),
            os_machine_id: "a".repeat(32),
            runtime_release_digest: release.digest.clone(),
            runtime_source_commit: release.source_commit.clone(),
            runtime_source_tree: release.source_tree.clone(),
            observed_at: Utc
                .with_ymd_and_hms(2026, 9, 22, 1, 2, 3)
                .single()
                .expect("valid test timestamp")
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        validate_old_generation_runtime_identity_report(
            &report,
            "environment-1",
            "machine-1",
            "kernel-1",
            4,
            &release,
        )
        .expect("valid old-generation report");
        let value: Value = serde_json::to_value(report.clone()).expect("report serializes");
        assert!(value.get("oldKernelIdentityReport").is_none());
        assert_eq!(value["observedAt"], "2026-09-22T01:02:03.000Z");
        assert_eq!(value["generation"], 4);

        let mut malformed = report;
        malformed.os_machine_id = "A".repeat(32);
        assert!(validate_old_generation_runtime_identity_report(
            &malformed,
            "environment-1",
            "machine-1",
            "kernel-1",
            4,
            &release,
        )
        .is_err());

        let mut noncanonical_timestamp = ManagedKernelRuntimeIdentityReport {
            environment_id: "environment-1".to_string(),
            machine_id: "machine-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            generation: 4,
            machine_credential: format!("mcred_{}", "x".repeat(40)),
            linux_boot_id: "01234567-89ab-cdef-0123-456789abcdef".to_string(),
            os_machine_id: "a".repeat(32),
            runtime_release_digest: release.digest.clone(),
            runtime_source_commit: release.source_commit.clone(),
            runtime_source_tree: release.source_tree.clone(),
            observed_at: "2026-09-22T01:02:03Z".to_string(),
        };
        assert!(validate_old_generation_runtime_identity_report(
            &noncanonical_timestamp,
            "environment-1",
            "machine-1",
            "kernel-1",
            4,
            &release,
        )
        .is_err());
        noncanonical_timestamp.observed_at = "2026-09-22T03:02:03.000+02:00".to_string();
        assert!(validate_old_generation_runtime_identity_report(
            &noncanonical_timestamp,
            "environment-1",
            "machine-1",
            "kernel-1",
            4,
            &release,
        )
        .is_err());
    }
}
