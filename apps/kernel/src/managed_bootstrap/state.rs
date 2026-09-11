use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::config::write_private_file;
use crate::error::DaemonError;

use super::context_plan::ManagedKernelContextPlan;
use super::cloud::ManagedCloudRelayProfile;

const MAX_STATE_BYTES: u64 = 96 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BootstrapConfig {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub(super) relay_public_key: String,
    pub(super) runtime_release_digest: String,
    pub(super) confirmed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) context_plan: Option<ManagedKernelContextPlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DisposableWorkerBootstrapReceiptStatus {
    ExchangePending,
    Exchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DisposableWorkerBootstrapReceipt {
    pub(super) schema_version: u32,
    pub(super) kind: String,
    pub(super) status: DisposableWorkerBootstrapReceiptStatus,
    pub(super) allocation_id: String,
    pub(super) user_id: String,
    pub(super) realm_id: String,
    pub(super) machine_id: String,
    pub(super) kernel_id: String,
    pub(super) relay_public_key: String,
    pub(super) runtime_release_digest: String,
    pub(super) binding_digest: String,
    pub(super) exchanged_at: Option<String>,
    pub(super) cloud_relay: Option<ManagedCloudRelayProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub(super) enum BootstrapReceiptDocument {
    ManagedEnvironment(BootstrapReceipt),
    DisposableWorker(DisposableWorkerBootstrapReceipt),
}

impl BootstrapConfig {
    pub(super) fn from_env() -> Result<Self, DaemonError> {
        let chariox_home = required_absolute_env_path("CHARIOX_HOME")?;
        let envelope_path = absolute_env_path(
            "CHARIOX_MANAGED_BOOTSTRAP_PATH",
            "/var/lib/chariox/managed-bootstrap.json",
        )?;
        let receipt_path = env::var_os("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| chariox_home.join("managed").join("bootstrap-receipt.json"));
        if !receipt_path.is_absolute() || !receipt_path.starts_with(&chariox_home) {
            return Err(state_error(
                "managed bootstrap receipt must remain inside CHARIOX_HOME",
            ));
        }
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

impl BootstrapEnvelope {
    pub(super) fn read(path: &Path) -> Result<Self, DaemonError> {
        let envelope: Self = read_bounded_json(path, "managed bootstrap envelope")?;
        match &envelope {
            Self::ManagedEnvironment(value) => value.validate()?,
            Self::DisposableWorker(value) => value.validate()?,
        }
        Ok(envelope)
    }

    pub(super) fn runtime_release_digest(&self) -> &str {
        match self {
            Self::ManagedEnvironment(value) => &value.runtime_release_digest,
            Self::DisposableWorker(value) => &value.binding.runtime_release_digest,
        }
    }
}

impl ManagedBootstrapEnvelope {
    pub(super) fn expires_at(&self) -> Result<DateTime<Utc>, DaemonError> {
        DateTime::parse_from_rfc3339(&self.expires_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|_| state_error("managed bootstrap expiry is invalid"))
    }

    fn validate(&self) -> Result<(), DaemonError> {
        if self.schema_version != 1
            || !valid_identifier(&self.environment_id)
            || !valid_secret(&self.token, "mkboot_")
            || !valid_digest(&self.runtime_release_digest)
        {
            return Err(state_error("managed bootstrap envelope is invalid"));
        }
        self.expires_at()?;
        validate_cloud_url(&self.cloud_api_url)
    }
}

impl DisposableWorkerBootstrapEnvelope {
    pub(super) fn expires_at(&self) -> Result<DateTime<Utc>, DaemonError> {
        DateTime::parse_from_rfc3339(&self.expires_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|_| state_error("disposable worker bootstrap expiry is invalid"))
    }

    fn validate(&self) -> Result<(), DaemonError> {
        let binding = &self.binding;
        if self.schema_version != 1
            || !valid_secret(&self.token, "dwboot_")
            || self.token.len() > "dwboot_".len() + 128
            || !valid_digest(&self.binding_digest)
            || !valid_disposable_identifier(&binding.allocation_id)
            || !valid_disposable_identifier(&binding.expected_home_kernel_id)
            || !valid_disposable_identifier(&binding.user_id)
            || !valid_disposable_identifier(&binding.realm_id)
            || !valid_disposable_identifier(&binding.worker_machine_id)
            || !valid_disposable_identifier(&binding.worker_kernel_id)
            || !valid_digest(&binding.image_digest)
            || !valid_digest(&binding.runtime_release_digest)
            || !valid_disposable_identifier(&binding.manager_operation_id)
            || binding.manager_operation_fence == 0
            || binding.manager_operation_fence > 9_007_199_254_740_991
            || !valid_digest(&binding.manager_request_digest)
            || !valid_digest(&binding.sender_key_thumbprint)
            || disposable_worker_binding_digest(binding)? != self.binding_digest
        {
            return Err(state_error("disposable worker bootstrap envelope is invalid"));
        }
        self.expires_at()?;
        validate_cloud_url(&self.cloud_api_url)
    }
}

pub(super) fn disposable_worker_binding_digest(
    binding: &DisposableWorkerBinding,
) -> Result<String, DaemonError> {
    let bytes = serde_json::to_vec(binding).map_err(|error| state_error(&error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
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
        if self.schema_version != 1
            || !valid_identifier(&self.environment_id)
            || !valid_identifier(&self.machine_id)
            || !valid_identifier(&self.kernel_id)
            || self.relay_public_key.trim().is_empty()
            || !valid_digest(&self.runtime_release_digest)
            || self
                .context_plan
                .as_ref()
                .is_some_and(|plan| plan.validate().is_err())
            || !confirmation_is_valid
        {
            return Err(state_error("managed bootstrap receipt is invalid"));
        }
        Ok(())
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
            || !valid_disposable_identifier(&self.allocation_id)
            || !valid_disposable_identifier(&self.user_id)
            || !valid_disposable_identifier(&self.realm_id)
            || !valid_disposable_identifier(&self.machine_id)
            || !valid_disposable_identifier(&self.kernel_id)
            || self.relay_public_key.trim().is_empty()
            || !valid_digest(&self.runtime_release_digest)
            || !valid_digest(&self.binding_digest)
            || match (&self.status, self.exchanged_at.as_deref(), &self.cloud_relay) {
                (DisposableWorkerBootstrapReceiptStatus::ExchangePending, None, None) => false,
                (DisposableWorkerBootstrapReceiptStatus::Exchanged, Some(value), Some(_)) => {
                    DateTime::parse_from_rfc3339(value).is_err()
                }
                _ => true,
            }
        {
            return Err(state_error("disposable worker bootstrap receipt is invalid"));
        }
        Ok(())
    }

    pub(super) fn persist(&self, path: &Path) -> Result<(), DaemonError> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| state_error(&error.to_string()))?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(state_error("disposable worker bootstrap receipt is too large"));
        }
        write_private_file(path, &bytes).map_err(|error| state_error(&error.to_string()))
    }
}

pub(super) fn remove_envelope(path: &Path) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| state_error(&error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(state_error(
            "managed bootstrap envelope is not a regular file",
        ));
    }
    fs::remove_file(path).map_err(|error| state_error(&error.to_string()))
}

pub(super) fn remove_receipt(path: &Path) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| state_error(&error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(state_error("managed bootstrap receipt is not a regular file"));
    }
    fs::remove_file(path).map_err(|error| state_error(&error.to_string()))
}

fn read_bounded_json<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T, DaemonError> {
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

fn validate_cloud_url(value: &str) -> Result<(), DaemonError> {
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

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
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
