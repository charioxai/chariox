use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::os::unix::fs::MetadataExt;
use url::Url;

use crate::config::write_private_file;
use crate::error::DaemonError;

use super::cloud::{DisposableWorkerEnrollmentReceipt, ManagedCloudRelayProfile};
use super::context_plan::ManagedKernelContextPlan;
use super::freshness::{
    valid_provider_rebuild_action_id, validate_freshness_evidence, ManagedKernelFreshnessEvidence,
};

const MAX_STATE_BYTES: u64 = 96 * 1024;
pub(super) const DEFAULT_MANAGED_REPOSITORY_ROOT: &str = "/home/chariox";
pub(super) const TRUSTED_BUILDER_PUBLIC_KEY_ENV: &str = "CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY";
pub(super) const MANAGED_BOOTSTRAP_PATH: &str = "/var/lib/chariox/managed-bootstrap.json";
pub(super) const PROTECTED_BOOTSTRAP_DIRECTORY: &str = "/etc/chariox/bootstrap";
pub(super) const PROTECTED_MANAGED_BOOTSTRAP_PATH: &str =
    "/etc/chariox/bootstrap/managed-bootstrap.json";
pub(super) const PROTECTED_DISPOSABLE_WORKER_BOOTSTRAP_PATH: &str =
    "/etc/chariox/bootstrap/disposable-worker-bootstrap.json";
const MIN_HETZNER_VOLUME_SIZE_GB: u32 = 10;
const MAX_HETZNER_VOLUME_SIZE_GB: u32 = 10_000;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BootstrapConfig {
    pub(super) process_home: PathBuf,
    pub(super) chariox_home: PathBuf,
    pub(super) envelope_path: PathBuf,
    pub(super) receipt_path: PathBuf,
    pub(super) manifest_path: PathBuf,
    pub(super) signature_path: PathBuf,
    pub(super) public_key_path: PathBuf,
    pub(super) kernel_binary: PathBuf,
    pub(super) kernel_host: String,
    pub(super) kernel_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub(super) enum BootstrapEnvelope {
    ManagedEnvironment(ManagedBootstrapEnvelope),
    DisposableWorker(DisposableWorkerBootstrapEnvelope),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManagedBootstrapEnvelope {
    pub(super) schema_version: u32,
    pub(super) cloud_api_url: String,
    pub(super) environment_id: String,
    pub(super) token: String,
    pub(super) expires_at: String,
    pub(super) runtime_release_digest: String,
    #[serde(default)]
    pub(super) managed_repository_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) provider_rebuild_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) expected_data_volume_serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) expected_data_volume_size_gb: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DisposableWorkerBinding {
    pub(super) allocation_id: String,
    pub(super) expected_home_kernel_id: String,
    pub(super) user_id: String,
    pub(super) realm_id: String,
    pub(super) worker_machine_id: String,
    pub(super) worker_kernel_id: String,
    pub(super) image_digest: String,
    pub(super) runtime_release_digest: String,
    pub(super) manager_operation_id: String,
    pub(super) manager_operation_fence: u64,
    pub(super) manager_request_digest: String,
    pub(super) sender_key_thumbprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DisposableWorkerBootstrapEnvelope {
    pub(super) schema_version: u32,
    pub(super) cloud_api_url: String,
    pub(super) token: String,
    pub(super) expires_at: String,
    pub(super) binding_digest: String,
    pub(super) binding: DisposableWorkerBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BootstrapReceiptStatus {
    Exchanged,
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BootstrapReceipt {
    pub(super) schema_version: u32,
    pub(super) status: BootstrapReceiptStatus,
    pub(super) environment_id: String,
    pub(super) machine_id: String,
    pub(super) kernel_id: String,
    #[serde(default = "initial_managed_environment_generation")]
    pub(super) generation: u64,
    pub(super) relay_public_key: String,
    pub(super) runtime_release_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) managed_repository_root: Option<String>,
    pub(super) confirmed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) context_plan: Option<ManagedKernelContextPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) provider_rebuild_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) freshness_evidence: Option<ManagedKernelFreshnessEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManagedBootstrapGrantBinding {
    pub(super) schema_version: u32,
    pub(super) envelope_schema_version: u32,
    pub(super) grant_binding_digest: String,
    pub(super) exchange_identity_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) receipt_binding_digest: Option<String>,
}

fn initial_managed_environment_generation() -> u64 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DisposableWorkerBootstrapReceiptStatus {
    ExchangePending,
    ExchangeAccepted,
    Exchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DisposableWorkerBootstrapReceipt {
    pub(super) schema_version: u32,
    pub(super) kind: String,
    pub(super) status: DisposableWorkerBootstrapReceiptStatus,
    pub(super) cloud_api_url: String,
    pub(super) relay_public_key: String,
    pub(super) binding_digest: String,
    pub(super) binding: DisposableWorkerBinding,
    pub(super) enrollment_receipt: Option<DisposableWorkerEnrollmentReceipt>,
    pub(super) cloud_relay: Option<ManagedCloudRelayProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DisposableWorkerReleaseOverride {
    pub(super) schema_version: u32,
    pub(super) kind: String,
    pub(super) binding_digest: String,
    pub(super) runtime_release_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub(super) enum BootstrapReceiptDocument {
    ManagedEnvironment(BootstrapReceipt),
    DisposableWorker(DisposableWorkerBootstrapReceipt),
}

impl BootstrapConfig {
    pub(super) fn from_env() -> Result<Self, DaemonError> {
        trusted_builder_public_key_path()?;
        let (process_home, chariox_home) = managed_home_paths()?;
        let configured_envelope_path =
            env::var_os("CHARIOX_MANAGED_BOOTSTRAP_PATH").filter(|value| !value.is_empty());
        let envelope_path = match super::managed_provider_topology()? {
            super::ManagedProviderTopology::Path1 => {
                if configured_envelope_path.as_ref().is_some_and(|value| {
                    Path::new(value) != Path::new(PROTECTED_MANAGED_BOOTSTRAP_PATH)
                }) {
                    return Err(state_error(
                        "Path-1 managed bootstrap must use the protected bootstrap path",
                    ));
                }
                PathBuf::from(PROTECTED_MANAGED_BOOTSTRAP_PATH)
            }
            super::ManagedProviderTopology::SharedHost => configured_envelope_path
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(MANAGED_BOOTSTRAP_PATH)),
        };
        validate_managed_state_path(&envelope_path, "CHARIOX_MANAGED_BOOTSTRAP_PATH")?;
        let receipt_path = env::var_os("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_managed_bootstrap_receipt_path);
        validate_managed_state_path(&receipt_path, "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT")?;
        let manifest_path = absolute_env_path(
            "CHARIOX_MANAGED_RELEASE_MANIFEST",
            "/usr/lib/chariox/release-manifest.json",
        )?;
        let signature_path = absolute_env_path(
            "CHARIOX_MANAGED_RELEASE_SIGNATURE",
            "/usr/lib/chariox/release-manifest.sig",
        )?;
        let public_key_path = absolute_env_path(
            "CHARIOX_MANAGED_RELEASE_PUBLIC_KEY",
            "/usr/lib/chariox/release-public-key",
        )?;
        let kernel_binary = absolute_env_path(
            "CHARIOX_MANAGED_KERNEL_BINARY",
            "/usr/local/bin/chariox-kernel",
        )?;
        let kernel_host =
            env::var("CHARIOX_KERNEL_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        if !matches!(kernel_host.trim(), "127.0.0.1" | "localhost" | "::1") {
            return Err(state_error("managed kernel must bind only to loopback"));
        }
        let kernel_port = env::var("CHARIOX_KERNEL_PORT")
            .ok()
            .map(|value| value.parse::<u16>())
            .transpose()
            .map_err(|_| state_error("managed kernel port is invalid"))?
            .unwrap_or(43118);
        if kernel_port == 0 {
            return Err(state_error("managed kernel port is invalid"));
        }
        Ok(Self {
            process_home,
            chariox_home,
            envelope_path,
            receipt_path,
            manifest_path,
            signature_path,
            public_key_path,
            kernel_binary,
            kernel_host: kernel_host.trim().to_string(),
            kernel_port,
        })
    }
}

pub(super) fn trusted_builder_public_key_path() -> Result<Option<PathBuf>, DaemonError> {
    match super::managed_provider_topology()? {
        super::ManagedProviderTopology::Path1 => {
            let path = required_absolute_env_path(TRUSTED_BUILDER_PUBLIC_KEY_ENV)?;
            validate_managed_state_path(&path, TRUSTED_BUILDER_PUBLIC_KEY_ENV)?;
            Ok(Some(path))
        }
        super::ManagedProviderTopology::SharedHost => Ok(None),
    }
}

pub(super) fn managed_home_paths() -> Result<(PathBuf, PathBuf), DaemonError> {
    let process_home = required_absolute_env_path("HOME")?;
    let chariox_home = required_absolute_env_path("CHARIOX_HOME")?;
    if process_home == Path::new("/") || chariox_home != process_home.join(".chariox") {
        return Err(state_error(
            "CHARIOX_HOME must equal HOME/.chariox for a managed kernel",
        ));
    }
    Ok((process_home, chariox_home))
}

pub(super) fn default_managed_bootstrap_receipt_path() -> PathBuf {
    PathBuf::from("/var/lib/chariox/managed/bootstrap-receipt.json")
}

pub(super) fn default_disposable_worker_receipt_path() -> PathBuf {
    PathBuf::from("/var/lib/chariox/disposable-worker/bootstrap-receipt.json")
}

pub(super) fn validate_managed_state_path(
    path: &Path,
    variable_name: &str,
) -> Result<(), DaemonError> {
    if !path.is_absolute()
        || path == Path::new("/")
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(state_error(&format!(
            "{variable_name} must be an absolute non-root path without parent-directory components"
        )));
    }
    Ok(())
}

impl BootstrapEnvelope {
    pub(super) fn read(path: &Path) -> Result<Self, DaemonError> {
        let protected_path = path == Path::new(PROTECTED_MANAGED_BOOTSTRAP_PATH);
        if protected_path {
            validate_protected_bootstrap_file(path, PROTECTED_MANAGED_BOOTSTRAP_PATH)?;
        }
        let envelope: Self = read_bounded_json(path, "managed bootstrap envelope")?;
        match &envelope {
            Self::ManagedEnvironment(value) => value.validate(protected_path)?,
            Self::DisposableWorker(value) => {
                if protected_path {
                    return Err(state_error(
                        "protected managed bootstrap file has the wrong envelope kind",
                    ));
                }
                value.validate()?;
            }
        }
        Ok(envelope)
    }
}

impl ManagedBootstrapEnvelope {
    pub(super) fn expires_at(&self) -> Result<DateTime<Utc>, DaemonError> {
        DateTime::parse_from_rfc3339(&self.expires_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|_| state_error("managed bootstrap expiry is invalid"))
    }

    fn validate(&self, protected_storage_input: bool) -> Result<(), DaemonError> {
        if !(matches!(self.schema_version, 1 | 2) && !protected_storage_input
            || self.schema_version == 3 && protected_storage_input)
            || !valid_identifier(&self.environment_id)
            || !valid_secret(&self.token, "mkboot_")
            || !valid_digest(&self.runtime_release_digest)
            || managed_repository_root_for_schema(
                self.schema_version,
                self.managed_repository_root.as_deref(),
            )
            .is_err()
            || self
                .provider_rebuild_action_id
                .as_deref()
                .is_some_and(|value| !valid_provider_rebuild_action_id(value))
        {
            return Err(state_error("managed bootstrap envelope is invalid"));
        }
        validate_volume_binding_for_schema(
            self.schema_version,
            protected_storage_input,
            self.expected_data_volume_serial.as_deref(),
            self.expected_data_volume_size_gb,
        )?;
        self.expires_at()?;
        validate_cloud_url(&self.cloud_api_url)
    }

    pub(super) fn managed_repository_root(&self) -> Result<String, DaemonError> {
        managed_repository_root_for_schema(
            self.schema_version,
            self.managed_repository_root.as_deref(),
        )
    }

    /// Bind every envelope field, including the bearer token, without
    /// persisting or formatting the token itself.
    pub(super) fn grant_binding_digest(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"chariox/managed-bootstrap-grant-binding/v1\0");
        update_binding_component(&mut digest, &self.schema_version.to_be_bytes());
        update_binding_component(&mut digest, self.cloud_api_url.as_bytes());
        update_binding_component(&mut digest, self.environment_id.as_bytes());
        update_binding_component(&mut digest, self.token.as_bytes());
        update_binding_component(&mut digest, self.expires_at.as_bytes());
        update_binding_component(&mut digest, self.runtime_release_digest.as_bytes());
        update_optional_binding_component(&mut digest, self.managed_repository_root.as_deref());
        update_optional_binding_component(&mut digest, self.provider_rebuild_action_id.as_deref());
        update_optional_binding_component(&mut digest, self.expected_data_volume_serial.as_deref());
        match self.expected_data_volume_size_gb {
            Some(size_gb) => {
                digest.update([1]);
                update_binding_component(&mut digest, &size_gb.to_be_bytes());
            }
            None => digest.update([0]),
        }
        format!("sha256:{:x}", digest.finalize())
    }
}

impl DisposableWorkerBootstrapEnvelope {
    pub(super) fn expires_at(&self) -> Result<DateTime<Utc>, DaemonError> {
        DateTime::parse_from_rfc3339(&self.expires_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|_| state_error("disposable worker bootstrap expiry is invalid"))
    }

    fn validate(&self) -> Result<(), DaemonError> {
        if self.schema_version != 1
            || !valid_secret(&self.token, "dwboot_")
            || self.token.len() > "dwboot_".len() + 128
            || !valid_digest(&self.binding_digest)
            || self.binding.validate().is_err()
            || disposable_worker_binding_digest(&self.binding)? != self.binding_digest
        {
            return Err(state_error(
                "disposable worker bootstrap envelope is invalid",
            ));
        }
        self.expires_at()?;
        validate_cloud_url(&self.cloud_api_url)
    }
}

impl DisposableWorkerBinding {
    fn validate(&self) -> Result<(), DaemonError> {
        if !valid_disposable_identifier(&self.allocation_id)
            || !valid_disposable_identifier(&self.expected_home_kernel_id)
            || !valid_disposable_identifier(&self.user_id)
            || !valid_disposable_identifier(&self.realm_id)
            || !valid_disposable_identifier(&self.worker_machine_id)
            || !valid_disposable_identifier(&self.worker_kernel_id)
            || !valid_digest(&self.image_digest)
            || !valid_digest(&self.runtime_release_digest)
            || !valid_disposable_identifier(&self.manager_operation_id)
            || self.manager_operation_fence == 0
            || self.manager_operation_fence > 9_007_199_254_740_991
            || !valid_digest(&self.manager_request_digest)
            || !valid_digest(&self.sender_key_thumbprint)
        {
            return Err(state_error("disposable worker binding is invalid"));
        }
        Ok(())
    }
}

pub(super) fn disposable_worker_binding_digest(
    binding: &DisposableWorkerBinding,
) -> Result<String, DaemonError> {
    let bytes = serde_json::to_vec(binding).map_err(|error| state_error(&error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

impl ManagedBootstrapGrantBinding {
    pub(super) fn pending_exchange(
        envelope: &ManagedBootstrapEnvelope,
        machine_id: &str,
        kernel_id: &str,
        relay_public_key: &str,
    ) -> Self {
        Self {
            schema_version: 1,
            envelope_schema_version: envelope.schema_version,
            grant_binding_digest: envelope.grant_binding_digest(),
            exchange_identity_digest: exchange_identity_digest(
                &envelope.environment_id,
                machine_id,
                kernel_id,
                relay_public_key,
                &envelope.runtime_release_digest,
            ),
            generation: None,
            receipt_binding_digest: None,
        }
    }

    /// The exchange happened on the envelope's provisioned release; the receipt
    /// binding covers identity only, so an in-place update keeps the binding.
    pub(super) fn for_receipt(
        envelope: &ManagedBootstrapEnvelope,
        receipt: &BootstrapReceipt,
    ) -> Self {
        Self {
            schema_version: RECEIPT_BINDING_SCHEMA,
            envelope_schema_version: envelope.schema_version,
            grant_binding_digest: envelope.grant_binding_digest(),
            exchange_identity_digest: exchange_identity_digest(
                &receipt.environment_id,
                &receipt.machine_id,
                &receipt.kernel_id,
                &receipt.relay_public_key,
                &envelope.runtime_release_digest,
            ),
            generation: Some(receipt.generation),
            receipt_binding_digest: Some(receipt_binding_digest(receipt)),
        }
    }

    /// A schema 1 binding, as written before in-place updates: it hashed the
    /// installed release into both digests. That release was the envelope's
    /// provisioned one, which the upgrader's staged receipt no longer carries.
    pub(super) fn legacy_for_receipt(
        envelope: &ManagedBootstrapEnvelope,
        receipt: &BootstrapReceipt,
    ) -> Self {
        Self {
            schema_version: 1,
            envelope_schema_version: envelope.schema_version,
            grant_binding_digest: envelope.grant_binding_digest(),
            exchange_identity_digest: exchange_identity_digest(
                &receipt.environment_id,
                &receipt.machine_id,
                &receipt.kernel_id,
                &receipt.relay_public_key,
                &envelope.runtime_release_digest,
            ),
            generation: Some(receipt.generation),
            receipt_binding_digest: Some(legacy_receipt_binding_digest(
                receipt,
                &envelope.runtime_release_digest,
            )),
        }
    }

    pub(super) fn read_for_receipt(receipt_path: &Path) -> Result<Option<Self>, DaemonError> {
        let path = managed_bootstrap_grant_binding_path(receipt_path)?;
        if !path.exists() {
            return Ok(None);
        }
        let binding: Self = read_bounded_json(&path, "managed bootstrap grant binding")?;
        binding.validate()?;
        Ok(Some(binding))
    }

    pub(super) fn persist_for_receipt(&self, receipt_path: &Path) -> Result<(), DaemonError> {
        self.validate()?;
        let path = managed_bootstrap_grant_binding_path(receipt_path)?;
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|error| state_error(&error.to_string()))?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(state_error("managed bootstrap grant binding is too large"));
        }
        write_private_file(&path, &bytes).map_err(|error| state_error(&error.to_string()))
    }

    fn validate(&self) -> Result<(), DaemonError> {
        if !matches!(self.schema_version, 1 | RECEIPT_BINDING_SCHEMA)
            || self.envelope_schema_version != 3
            || !valid_digest(&self.grant_binding_digest)
            || !valid_digest(&self.exchange_identity_digest)
            || match (self.generation, self.receipt_binding_digest.as_deref()) {
                (None, None) => false,
                (Some(generation), Some(digest)) => {
                    !(1..=i32::MAX as u64).contains(&generation) || !valid_digest(digest)
                }
                _ => true,
            }
        {
            return Err(state_error("managed bootstrap grant binding is invalid"));
        }
        Ok(())
    }
}

pub(super) fn managed_bootstrap_grant_binding_path(
    receipt_path: &Path,
) -> Result<PathBuf, DaemonError> {
    receipt_path
        .parent()
        .map(|parent| parent.join("bootstrap-grant-binding.json"))
        .ok_or_else(|| state_error("managed bootstrap receipt has no state directory"))
}

fn exchange_identity_digest(
    environment_id: &str,
    machine_id: &str,
    kernel_id: &str,
    relay_public_key: &str,
    runtime_release_digest: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"chariox/managed-bootstrap-exchange-identity/v1\0");
    for component in [
        environment_id.as_bytes(),
        machine_id.as_bytes(),
        kernel_id.as_bytes(),
        relay_public_key.as_bytes(),
        runtime_release_digest.as_bytes(),
    ] {
        update_binding_component(&mut digest, component);
    }
    format!("sha256:{:x}", digest.finalize())
}

/// Schema 2 binds the receipt's identity, not its installed release: a
/// Cloud-coordinated in-place update changes the release of the same machine.
const RECEIPT_BINDING_SCHEMA: u32 = 2;

fn receipt_binding_digest(receipt: &BootstrapReceipt) -> String {
    let mut digest = Sha256::new();
    digest.update(b"chariox/managed-bootstrap-receipt-binding/v2\0");
    update_binding_component(&mut digest, &receipt.schema_version.to_be_bytes());
    update_binding_component(&mut digest, receipt.environment_id.as_bytes());
    update_binding_component(&mut digest, receipt.machine_id.as_bytes());
    update_binding_component(&mut digest, receipt.kernel_id.as_bytes());
    update_binding_component(&mut digest, &receipt.generation.to_be_bytes());
    update_binding_component(&mut digest, receipt.relay_public_key.as_bytes());
    update_optional_binding_component(&mut digest, receipt.managed_repository_root.as_deref());
    format!("sha256:{:x}", digest.finalize())
}

fn legacy_receipt_binding_digest(
    receipt: &BootstrapReceipt,
    runtime_release_digest: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"chariox/managed-bootstrap-receipt-binding/v1\0");
    update_binding_component(&mut digest, &receipt.schema_version.to_be_bytes());
    update_binding_component(&mut digest, receipt.environment_id.as_bytes());
    update_binding_component(&mut digest, receipt.machine_id.as_bytes());
    update_binding_component(&mut digest, receipt.kernel_id.as_bytes());
    update_binding_component(&mut digest, &receipt.generation.to_be_bytes());
    update_binding_component(&mut digest, receipt.relay_public_key.as_bytes());
    update_binding_component(&mut digest, runtime_release_digest.as_bytes());
    update_optional_binding_component(&mut digest, receipt.managed_repository_root.as_deref());
    format!("sha256:{:x}", digest.finalize())
}

impl BootstrapReceipt {
    pub(super) fn read(path: &Path) -> Result<Option<Self>, DaemonError> {
        if !path.exists() {
            return Ok(None);
        }
        let receipt: Self = read_bounded_json(path, "managed bootstrap receipt")?;
        receipt.validate()?;
        Ok(Some(receipt))
    }

    fn validate(&self) -> Result<(), DaemonError> {
        let confirmation_is_valid = match (&self.status, self.confirmed_at.as_deref()) {
            (BootstrapReceiptStatus::Exchanged, None) => true,
            (BootstrapReceiptStatus::Confirmed, Some(value)) => {
                DateTime::parse_from_rfc3339(value).is_ok()
            }
            _ => false,
        };
        if !matches!(self.schema_version, 1..=3)
            || !valid_identifier(&self.environment_id)
            || !valid_identifier(&self.machine_id)
            || !valid_identifier(&self.kernel_id)
            || !(1..=i32::MAX as u64).contains(&self.generation)
            || self.relay_public_key.trim().is_empty()
            || !valid_digest(&self.runtime_release_digest)
            || managed_repository_root_for_schema(
                self.schema_version,
                self.managed_repository_root.as_deref(),
            )
            .is_err()
            || self
                .context_plan
                .as_ref()
                .is_some_and(|plan| plan.validate().is_err())
            || self
                .provider_rebuild_action_id
                .as_deref()
                .is_some_and(|value| !valid_provider_rebuild_action_id(value))
            || self.freshness_evidence.as_ref().is_some_and(|evidence| {
                self.provider_rebuild_action_id.is_none()
                    && !(self.schema_version == 3 && self.generation > 1)
                    || validate_freshness_evidence(evidence, &self.runtime_release_digest).is_err()
            })
            || !confirmation_is_valid
        {
            return Err(state_error("managed bootstrap receipt is invalid"));
        }
        Ok(())
    }

    pub(super) fn managed_repository_root(&self) -> Result<String, DaemonError> {
        managed_repository_root_for_schema(
            self.schema_version,
            self.managed_repository_root.as_deref(),
        )
    }

    pub(super) fn persist(&self, path: &Path) -> Result<(), DaemonError> {
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|error| state_error(&error.to_string()))?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(state_error("managed bootstrap receipt is too large"));
        }
        write_private_file(path, &bytes).map_err(|error| state_error(&error.to_string()))
    }
}

pub(super) fn managed_repository_root_for_schema(
    schema_version: u32,
    value: Option<&str>,
) -> Result<String, DaemonError> {
    match (schema_version, value) {
        (1, None) => Ok(DEFAULT_MANAGED_REPOSITORY_ROOT.to_string()),
        (2 | 3, Some(value)) => {
            let normalized = normalize_managed_repository_root(value)?;
            if normalized != value {
                return Err(state_error("managed repository root must be normalized"));
            }
            Ok(normalized)
        }
        _ => Err(state_error(
            "managed repository root does not match the bootstrap schema",
        )),
    }
}

pub(super) fn validate_protected_bootstrap_file(
    path: &Path,
    expected_path: &str,
) -> Result<(), DaemonError> {
    if path != Path::new(expected_path) {
        return Err(state_error("protected bootstrap path is not canonical"));
    }
    let directory = Path::new(PROTECTED_BOOTSTRAP_DIRECTORY);
    let root_directory = Path::new("/");
    let root_metadata = fs::symlink_metadata(root_directory)
        .map_err(|error| state_error(&format!("inspect / for protected bootstrap: {error}")))?;
    let etc_metadata = fs::symlink_metadata("/etc")
        .map_err(|error| state_error(&format!("inspect /etc for protected bootstrap: {error}")))?;
    let chariox_directory = Path::new("/etc/chariox");
    let chariox_metadata = fs::symlink_metadata(chariox_directory).map_err(|error| {
        state_error(&format!(
            "inspect /etc/chariox for protected bootstrap: {error}"
        ))
    })?;
    let directory_metadata = fs::symlink_metadata(directory)
        .map_err(|error| state_error(&format!("inspect protected bootstrap directory: {error}")))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| state_error(&format!("inspect protected bootstrap file: {error}")))?;
    let expected_gid = unsafe { libc::getegid() };
    let root_mode = root_metadata.mode();
    let etc_mode = etc_metadata.mode();
    let chariox_mode = chariox_metadata.mode();
    if root_metadata.file_type().is_symlink()
        || !root_metadata.is_dir()
        || root_metadata.uid() != 0
        || root_mode & 0o022 != 0
        || root_mode & 0o001 == 0
        || etc_metadata.file_type().is_symlink()
        || !etc_metadata.is_dir()
        || etc_metadata.uid() != 0
        || etc_mode & 0o022 != 0
        || etc_mode & 0o001 == 0
        || chariox_metadata.file_type().is_symlink()
        || !chariox_metadata.is_dir()
        || chariox_metadata.uid() != 0
        || chariox_mode & 0o022 != 0
        || !(chariox_metadata.gid() == expected_gid && chariox_mode & 0o010 != 0
            || chariox_mode & 0o001 != 0)
        || directory_metadata.file_type().is_symlink()
        || !directory_metadata.is_dir()
        || directory_metadata.uid() != 0
        || directory_metadata.gid() != expected_gid
        || directory_metadata.mode() & 0o7777 != 0o750
        || metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != 0
        || metadata.gid() != expected_gid
        || metadata.mode() & 0o7777 != 0o640
    {
        return Err(state_error(
            "protected bootstrap input must be root:chariox with directory mode 0750 and file mode 0640",
        ));
    }
    Ok(())
}

pub(super) fn validate_volume_binding_for_schema(
    schema_version: u32,
    protected_storage_input: bool,
    serial: Option<&str>,
    size_gb: Option<u32>,
) -> Result<(), DaemonError> {
    let storage_schema = protected_storage_input && matches!(schema_version, 2 | 3);
    if !storage_schema {
        if serial.is_some() || size_gb.is_some() {
            return Err(state_error(
                "legacy bootstrap envelope cannot include a data-volume binding",
            ));
        }
        return Ok(());
    }
    let valid_serial = serial.is_some_and(|value| {
        !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && value.parse::<u64>().is_ok_and(|number| {
                number > 0 && number <= MAX_SAFE_INTEGER && number.to_string() == value
            })
    });
    if !valid_serial
        || !size_gb.is_some_and(|value| {
            (MIN_HETZNER_VOLUME_SIZE_GB..=MAX_HETZNER_VOLUME_SIZE_GB).contains(&value)
        })
    {
        return Err(state_error("data-volume bootstrap binding is invalid"));
    }
    Ok(())
}

pub(super) fn normalize_managed_repository_root(value: &str) -> Result<String, DaemonError> {
    if value.is_empty()
        || value.len() > 4096
        || value.trim() != value
        || value.chars().any(char::is_control)
        || !value.starts_with('/')
        || value.starts_with("//")
    {
        return Err(state_error("managed repository root is invalid"));
    }
    let mut segments = Vec::new();
    for segment in value.split('/').skip(1) {
        if segment.is_empty() {
            continue;
        }
        if matches!(segment, "." | "..") {
            return Err(state_error("managed repository root is invalid"));
        }
        segments.push(segment);
    }
    let normalized = if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    };
    if overlaps_protected_managed_root(&normalized) {
        return Err(state_error("managed repository root is protected"));
    }
    Ok(normalized)
}

pub(super) fn overlaps_protected_managed_root(candidate: &str) -> bool {
    ["/", "/var/lib/chariox", "/usr/lib/chariox"]
        .iter()
        .any(|protected| candidate == *protected || candidate.starts_with(&format!("{protected}/")))
        || candidate == "/home/chariox/.chariox"
        || candidate.starts_with("/home/chariox/.chariox/")
}

impl BootstrapReceiptDocument {
    pub(super) fn read(path: &Path) -> Result<Option<Self>, DaemonError> {
        if !path.exists() {
            return Ok(None);
        }
        let document: Self = read_bounded_json(path, "managed bootstrap receipt")?;
        match &document {
            Self::ManagedEnvironment(receipt) => receipt.validate()?,
            Self::DisposableWorker(receipt) => receipt.validate()?,
        }
        Ok(Some(document))
    }
}

impl DisposableWorkerBootstrapReceipt {
    fn validate(&self) -> Result<(), DaemonError> {
        if self.schema_version != 1
            || self.kind != "disposable_worker"
            || validate_cloud_url(&self.cloud_api_url).is_err()
            || self.relay_public_key.trim().is_empty()
            || !valid_digest(&self.binding_digest)
            || self.binding.validate().is_err()
            || disposable_worker_binding_digest(&self.binding)? != self.binding_digest
            || match (
                &self.status,
                self.enrollment_receipt.as_ref(),
                self.cloud_relay.as_ref(),
            ) {
                (DisposableWorkerBootstrapReceiptStatus::ExchangePending, None, None) => false,
                (DisposableWorkerBootstrapReceiptStatus::ExchangeAccepted, Some(receipt), None) => {
                    DateTime::parse_from_rfc3339(&receipt.exchanged_at).is_err()
                }
                (DisposableWorkerBootstrapReceiptStatus::Exchanged, Some(receipt), Some(_)) => {
                    DateTime::parse_from_rfc3339(&receipt.exchanged_at).is_err()
                }
                _ => true,
            }
        {
            return Err(state_error(
                "disposable worker bootstrap receipt is invalid",
            ));
        }
        Ok(())
    }
}

impl DisposableWorkerReleaseOverride {
    pub(super) fn read_for_receipt(
        receipt_path: &Path,
        binding_digest: &str,
    ) -> Result<Option<Self>, DaemonError> {
        let path = disposable_worker_release_override_path(receipt_path)?;
        if !path.exists() {
            return Ok(None);
        }
        let value: Self = read_bounded_json(&path, "disposable worker release override")?;
        if value.schema_version != 1
            || value.kind != "disposable_worker_release"
            || value.binding_digest != binding_digest
            || !valid_digest(&value.runtime_release_digest)
        {
            return Err(state_error("disposable worker release override is invalid"));
        }
        Ok(Some(value))
    }
}

pub(super) fn disposable_worker_release_override_path(
    receipt_path: &Path,
) -> Result<PathBuf, DaemonError> {
    receipt_path
        .parent()
        .map(|parent| parent.join("release-override.json"))
        .ok_or_else(|| state_error("managed bootstrap receipt has no state directory"))
}

pub(super) fn remove_envelope(path: &Path) -> Result<(), DaemonError> {
    if path == Path::new(PROTECTED_MANAGED_BOOTSTRAP_PATH)
        || path == Path::new(PROTECTED_DISPOSABLE_WORKER_BOOTSTRAP_PATH)
    {
        // Cloud writes this root-owned input before systemd. Keep it available
        // for root disk admission on every boot; the chariox supervisor cannot
        // remove or replace files beneath the protected directory.
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| state_error(&error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(state_error(
            "managed bootstrap envelope is not a regular file",
        ));
    }
    fs::remove_file(path).map_err(|error| state_error(&error.to_string()))
}

pub(super) fn read_bounded_json<T: DeserializeOwned>(
    path: &Path,
    label: &str,
) -> Result<T, DaemonError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| state_error(&format!("{label}: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_STATE_BYTES
    {
        return Err(state_error(&format!(
            "{label} is not a bounded regular file"
        )));
    }
    let bytes = fs::read(path).map_err(|error| state_error(&format!("{label}: {error}")))?;
    serde_json::from_slice(&bytes).map_err(|_| state_error(&format!("{label} is invalid")))
}

pub(super) fn validate_cloud_url(value: &str) -> Result<(), DaemonError> {
    let url =
        Url::parse(value).map_err(|_| state_error("managed bootstrap Cloud URL is invalid"))?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(state_error(
            "managed bootstrap Cloud URL contains forbidden components",
        ));
    }
    let secure = url.scheme() == "https";
    let loopback = url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"));
    if !secure && !loopback {
        return Err(state_error("managed bootstrap Cloud URL must use HTTPS"));
    }
    Ok(())
}

pub(super) fn valid_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    value.len() <= 128
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._:-".contains(&byte)
        })
}

pub(super) fn valid_secret(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| {
        suffix.len() >= 40
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn update_binding_component(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn update_optional_binding_component(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update([1]);
            update_binding_component(digest, value.as_bytes());
        }
        None => digest.update([0]),
    }
}

pub(super) fn valid_disposable_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    value.len() <= 128
        && first.is_ascii_alphanumeric()
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

fn required_absolute_env_path(name: &'static str) -> Result<PathBuf, DaemonError> {
    let value = env::var_os(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| state_error(&format!("{name} must be set for a managed kernel")))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(state_error(&format!("{name} must be absolute")));
    }
    Ok(path)
}

fn absolute_env_path(name: &'static str, default: &'static str) -> Result<PathBuf, DaemonError> {
    let path = env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default));
    if !path.is_absolute() {
        return Err(state_error(&format!("{name} must be absolute")));
    }
    Ok(path)
}

fn state_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed kernel bootstrap state",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::MANAGED_PROVIDER_TOPOLOGY_ENV;
    use super::*;

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => env::set_var(name, value),
            None => env::remove_var(name),
        }
    }

    #[test]
    fn managed_paths_keep_process_home_separate_from_chariox_state() {
        crate::test_support::isolated_env_test!();
        let _lock = crate::env_lock::lock();
        let previous_home = env::var_os("HOME");
        let previous_chariox_home = env::var_os("CHARIOX_HOME");
        let previous_receipt = env::var_os("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT");
        let previous_topology = env::var_os(MANAGED_PROVIDER_TOPOLOGY_ENV);
        let root = env::temp_dir().join(format!(
            "chariox-managed-home-paths-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let process_home = root.join("home");
        let chariox_home = process_home.join(".chariox");
        env::set_var("HOME", &process_home);
        env::set_var("CHARIOX_HOME", &chariox_home);
        env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "shared_host");
        env::remove_var("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT");

        let (observed_process_home, observed_chariox_home) =
            managed_home_paths().expect("managed HOME relationship should be accepted");
        assert_eq!(observed_process_home, process_home);
        assert_eq!(observed_chariox_home, chariox_home);

        let config = BootstrapConfig::from_env().expect("managed config should be accepted");
        assert_eq!(config.process_home, process_home);
        assert_eq!(config.chariox_home, chariox_home);
        assert_eq!(
            config.receipt_path,
            PathBuf::from("/var/lib/chariox/managed/bootstrap-receipt.json")
        );

        let root_receipt = root.join("var/lib/chariox/managed/bootstrap-receipt.json");
        env::set_var("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT", &root_receipt);
        assert_eq!(
            BootstrapConfig::from_env()
                .expect("explicit root-owned receipt should be accepted")
                .receipt_path,
            root_receipt
        );

        env::set_var(
            "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
            root.join("../unsafe-receipt.json"),
        );
        assert!(BootstrapConfig::from_env().is_err());

        env::set_var("CHARIOX_HOME", &process_home);
        assert!(managed_home_paths().is_err());

        restore_env("HOME", previous_home);
        restore_env("CHARIOX_HOME", previous_chariox_home);
        restore_env("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT", previous_receipt);
        restore_env(MANAGED_PROVIDER_TOPOLOGY_ENV, previous_topology);
    }

    #[test]
    fn path1_requires_an_explicit_external_builder_key_path() {
        crate::test_support::isolated_env_test!();
        let _lock = crate::env_lock::lock();
        let previous_topology = env::var_os(MANAGED_PROVIDER_TOPOLOGY_ENV);
        let previous_key = env::var_os(TRUSTED_BUILDER_PUBLIC_KEY_ENV);
        env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "path1");
        env::remove_var(TRUSTED_BUILDER_PUBLIC_KEY_ENV);
        assert!(trusted_builder_public_key_path().is_err());
        env::set_var(TRUSTED_BUILDER_PUBLIC_KEY_ENV, "relative/key");
        assert!(trusted_builder_public_key_path().is_err());
        env::set_var(
            TRUSTED_BUILDER_PUBLIC_KEY_ENV,
            "/etc/chariox/trusted-builder-public-key",
        );
        assert_eq!(
            trusted_builder_public_key_path().expect("Path-1 trust path"),
            Some(PathBuf::from("/etc/chariox/trusted-builder-public-key"))
        );
        env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "shared_host");
        env::remove_var(TRUSTED_BUILDER_PUBLIC_KEY_ENV);
        assert_eq!(
            trusted_builder_public_key_path().expect("shared host"),
            None
        );
        restore_env(MANAGED_PROVIDER_TOPOLOGY_ENV, previous_topology);
        restore_env(TRUSTED_BUILDER_PUBLIC_KEY_ENV, previous_key);
    }

    #[test]
    fn managed_repository_root_matches_the_cloud_trust_boundary() {
        for value in [
            "/home",
            "/home/chariox",
            "/tmp",
            "/var",
            "/usr/lib",
            "/srv/managed workspaces",
            "/srv/$(literal);workspace",
        ] {
            assert_eq!(
                normalize_managed_repository_root(value).unwrap(),
                value,
                "accepted root {value}"
            );
        }
        assert_eq!(
            normalize_managed_repository_root("/srv//managed/").unwrap(),
            "/srv/managed"
        );
        for value in [
            "",
            "relative",
            "//srv/workspaces",
            "/srv/./workspaces",
            "/srv/../workspaces",
            "/",
            "/var/lib/chariox",
            "/var/lib/chariox/workspaces",
            "/usr/lib/chariox",
            "/usr/lib/chariox/releases/v1",
            "/home/chariox/.chariox",
            "/home/chariox/.chariox/workspaces",
            "/srv/workspaces\nother",
        ] {
            assert!(
                normalize_managed_repository_root(value).is_err(),
                "rejected root {value:?}"
            );
        }
    }

    #[test]
    fn managed_repository_root_is_bound_to_the_bootstrap_schema() {
        assert_eq!(
            managed_repository_root_for_schema(1, None).unwrap(),
            DEFAULT_MANAGED_REPOSITORY_ROOT
        );
        assert_eq!(
            managed_repository_root_for_schema(2, Some("/srv/workspaces")).unwrap(),
            "/srv/workspaces"
        );
        assert!(managed_repository_root_for_schema(1, Some("/srv/workspaces")).is_err());
        assert!(managed_repository_root_for_schema(2, None).is_err());
        assert!(managed_repository_root_for_schema(2, Some("/srv//workspaces")).is_err());
        assert_eq!(
            managed_repository_root_for_schema(3, Some("/srv/workspaces")).unwrap(),
            "/srv/workspaces"
        );
        assert!(managed_repository_root_for_schema(4, Some("/srv/workspaces")).is_err());
    }
}
