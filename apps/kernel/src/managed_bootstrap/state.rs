use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use url::Url;

use crate::config::write_private_file;
use crate::error::DaemonError;

const MAX_STATE_BYTES: u64 = 64 * 1024;

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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BootstrapEnvelope {
    pub(super) schema_version: u32,
    pub(super) cloud_api_url: String,
    pub(super) environment_id: String,
    pub(super) token: String,
    pub(super) expires_at: String,
    pub(super) runtime_release_digest: String,
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
        envelope.validate()?;
        Ok(envelope)
    }

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

impl BootstrapReceipt {
    pub(super) fn read(path: &Path) -> Result<Option<Self>, DaemonError> {
        if !path.exists() {
            return Ok(None);
        }
        let receipt: Self = read_bounded_json(path, "managed bootstrap receipt")?;
        if receipt.schema_version != 1
            || !valid_identifier(&receipt.environment_id)
            || !valid_identifier(&receipt.machine_id)
            || !valid_identifier(&receipt.kernel_id)
            || receipt.relay_public_key.trim().is_empty()
            || !valid_digest(&receipt.runtime_release_digest)
        {
            return Err(state_error("managed bootstrap receipt is invalid"));
        }
        Ok(Some(receipt))
    }

    pub(super) fn persist(&self, path: &Path) -> Result<(), DaemonError> {
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|error| state_error(&error.to_string()))?;
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
