//! Bootstrap for a disposable VM. After enrollment it runs the ordinary lease-only kernel.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{
    load_managed_cloud_relay_profile, load_or_create_managed_runtime_identity,
    persist_managed_cloud_relay_profile, write_private_file, LeaseWorkerHomeCaller,
    ManagedRuntimeIdentity, PersistedCloudRelayProfile,
};
use crate::error::DaemonError;

use super::cloud::{HttpBootstrapCloudClient, ManagedCloudRelayProfile};
use super::release::{verify_release, VerifiedRelease};
use super::state::{
    read_bounded_json, remove_envelope, valid_digest, valid_identifier, valid_secret,
    validate_cloud_url,
};
use super::{jittered, normalized_api_url, persisted_profile, valid_managed_relay_url};

const MIN_RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY: Duration = Duration::from_secs(60);
const CONFIRM_DEADLINE: Duration = Duration::from_secs(10 * 60);
const STABLE_RUNTIME: Duration = Duration::from_secs(30);

pub(crate) const ACTIVITY_RECEIPT_ENV: &str = "CHARIOX_DISPOSABLE_WORKER_RECEIPT";

/// Read the supervisor's enrollment binding without creating or refreshing identity.
/// An exchanged receipt is usable: the child starts before Cloud confirmation,
/// and the activity reporter retries until Cloud admits the confirmed worker.
pub(crate) fn activity_allocation(
    path: &Path,
    config: &crate::config::DaemonConfig,
    profile: &PersistedCloudRelayProfile,
) -> Result<String, DaemonError> {
    let receipt = WorkerReceipt::read(path)?
        .ok_or_else(|| worker_error("worker activity receipt is missing"))?;
    validate_cloud_url(&profile.api_url)?;
    validate_profile(&receipt, profile)?;
    if !config.accept_remote_leases
        || config.kernel_runtime_role != crate::config::KernelRuntimeRole::RemoteLeaseWorker
        || config.remote_lease_capacity != Some(1)
        || config.lease_worker_home_caller.as_ref() != Some(&receipt.home_caller.lease_binding())
        || config.host_machine_id != receipt.machine_id
        || config.daemon_id != receipt.kernel_id
        || config.relay_public_key != receipt.relay_public_key
    {
        return Err(worker_error(
            "worker activity identity conflicts with its receipt",
        ));
    }
    Ok(receipt.allocation_id)
}

#[derive(Debug, Clone)]
struct WorkerConfig {
    chariox_home: PathBuf,
    envelope_path: PathBuf,
    receipt_path: PathBuf,
    manifest_path: PathBuf,
    signature_path: PathBuf,
    public_key_path: PathBuf,
    kernel_binary: PathBuf,
    kernel_host: String,
    kernel_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkerEnvelope {
    schema_version: u32,
    cloud_api_url: String,
    allocation_id: String,
    token: String,
    expires_at: String,
    runtime_release_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkerReceiptStatus {
    Exchanged,
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkerReceipt {
    schema_version: u32,
    status: WorkerReceiptStatus,
    allocation_id: String,
    machine_id: String,
    kernel_id: String,
    relay_public_key: String,
    runtime_release_digest: String,
    home_caller: CloudHomeCaller,
    confirmed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CloudHomeCaller {
    account_id: String,
    user_id: String,
    realm_id: String,
    machine_id: String,
    kernel_id: String,
    relay_public_key: String,
}

impl CloudHomeCaller {
    fn lease_binding(&self) -> LeaseWorkerHomeCaller {
        LeaseWorkerHomeCaller {
            kernel_id: self.kernel_id.clone(),
            realm_id: self.realm_id.clone(),
            user_id: self.user_id.clone(),
            relay_public_key: self.relay_public_key.clone(),
        }
    }

    fn valid(&self) -> bool {
        [
            &self.account_id,
            &self.user_id,
            &self.realm_id,
            &self.machine_id,
            &self.kernel_id,
        ]
        .iter()
        .all(|value| valid_identifier(value))
            && !self.relay_public_key.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExchangeRequest {
    token: String,
    allocation_id: String,
    machine_id: String,
    kernel_id: String,
    relay_public_key: String,
    runtime_release_digest: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExchangeResponse {
    allocation_id: String,
    kernel_id: String,
    runtime_release_digest: String,
    lease_worker_capacity: u32,
    home_caller: CloudHomeCaller,
    cloud_relay: ManagedCloudRelayProfile,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmRequest {
    token: String,
    allocation_id: String,
    machine_id: String,
    kernel_id: String,
    machine_credential: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfirmResponse {
    confirmed: bool,
    observed_state: String,
}

trait WorkerCloudClient {
    fn exchange(
        &self,
        api_url: &str,
        request: &ExchangeRequest,
    ) -> Result<ExchangeResponse, DaemonError>;
    fn confirm(
        &self,
        api_url: &str,
        request: &ConfirmRequest,
    ) -> Result<ConfirmResponse, DaemonError>;
}

impl WorkerCloudClient for HttpBootstrapCloudClient {
    fn exchange(
        &self,
        api_url: &str,
        request: &ExchangeRequest,
    ) -> Result<ExchangeResponse, DaemonError> {
        self.post_managed(
            api_url,
            "/v1/disposable-workers/bootstrap/exchange",
            request,
        )
    }

    fn confirm(
        &self,
        api_url: &str,
        request: &ConfirmRequest,
    ) -> Result<ConfirmResponse, DaemonError> {
        self.post_managed(api_url, "/v1/disposable-workers/bootstrap/confirm", request)
    }
}

struct PreparedWorker {
    release: VerifiedRelease,
    receipt: WorkerReceipt,
    pending: Option<WorkerEnvelope>,
}

pub fn run_from_env() -> Result<(), DaemonError> {
    let cloud = HttpBootstrapCloudClient::default();
    let mut delay = MIN_RETRY;
    loop {
        let prepared = WorkerConfig::from_env()
            .and_then(|config| prepare(&config, &cloud, Utc::now()).map(|worker| (config, worker)));
        match prepared {
            Ok((config, worker)) => supervise(&config, worker, &cloud)?,
            Err(error) => crate::logging::warn_with_fields(
                "disposable_worker_bootstrap.prepare_failed",
                "disposable worker bootstrap failed; retrying",
                serde_json::json!({
                    "error": error.to_string(),
                    "retry_delay_ms": delay.as_millis(),
                }),
            ),
        }
        thread::sleep(jittered(delay));
        delay = delay.saturating_mul(2).min(MAX_RETRY);
    }
}

fn prepare(
    config: &WorkerConfig,
    cloud: &impl WorkerCloudClient,
    now: DateTime<Utc>,
) -> Result<PreparedWorker, DaemonError> {
    let receipt = WorkerReceipt::read(&config.receipt_path)?;
    let envelope = if config.envelope_path.exists() {
        Some(WorkerEnvelope::read(&config.envelope_path)?)
    } else {
        None
    };
    let installed_digest = receipt
        .as_ref()
        .map(|value| value.effective_release_digest(&config.receipt_path))
        .transpose()?;
    let expected_digest = installed_digest
        .as_deref()
        .or_else(|| {
            envelope
                .as_ref()
                .map(|value| value.runtime_release_digest.as_str())
        })
        .ok_or_else(|| worker_error("worker envelope and receipt are missing"))?;
    if let (Some(envelope), Some(receipt)) = (&envelope, &receipt) {
        if envelope.allocation_id != receipt.allocation_id
            || envelope.runtime_release_digest != receipt.runtime_release_digest
        {
            return Err(worker_error("worker envelope conflicts with its receipt"));
        }
    }
    let release = verify_release(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        expected_digest,
        &config.kernel_binary,
    )?;
    let identity =
        load_or_create_managed_runtime_identity(&config.kernel_host, config.kernel_port)?;
    let (receipt, pending) = match receipt {
        Some(receipt) => {
            receipt.validate_identity(&identity)?;
            let profile = load_managed_cloud_relay_profile()
                .ok_or_else(|| worker_error("worker Cloud profile is missing"))?;
            validate_profile(&receipt, &profile)?;
            match receipt.status {
                WorkerReceiptStatus::Confirmed => {
                    if config.envelope_path.exists() {
                        remove_envelope(&config.envelope_path)?;
                    }
                    (receipt, None)
                }
                WorkerReceiptStatus::Exchanged => {
                    let pending = envelope.ok_or_else(|| {
                        worker_error("worker envelope is required to finish confirmation")
                    })?;
                    (receipt, Some(pending))
                }
            }
        }
        None => {
            let envelope = envelope.ok_or_else(|| worker_error("worker envelope is missing"))?;
            if envelope.expires_at()? <= now {
                return Err(worker_error(
                    "worker bootstrap token expired before exchange",
                ));
            }
            let response = cloud.exchange(
                &normalized_api_url(&envelope.cloud_api_url),
                &ExchangeRequest {
                    token: envelope.token.clone(),
                    allocation_id: envelope.allocation_id.clone(),
                    machine_id: identity.machine_id.clone(),
                    kernel_id: identity.kernel_id.clone(),
                    relay_public_key: identity.relay_public_key.clone(),
                    runtime_release_digest: envelope.runtime_release_digest.clone(),
                },
            )?;
            validate_exchange(&envelope, &identity, &response)?;
            persist_managed_cloud_relay_profile(persisted_profile(response.cloud_relay))?;
            let receipt = WorkerReceipt {
                schema_version: 1,
                status: WorkerReceiptStatus::Exchanged,
                allocation_id: envelope.allocation_id.clone(),
                machine_id: identity.machine_id,
                kernel_id: identity.kernel_id,
                relay_public_key: identity.relay_public_key,
                runtime_release_digest: envelope.runtime_release_digest.clone(),
                home_caller: response.home_caller,
                confirmed_at: None,
            };
            receipt.persist(&config.receipt_path)?;
            (receipt, Some(envelope))
        }
    };
    Ok(PreparedWorker {
        release,
        receipt,
        pending,
    })
}

fn validate_exchange(
    envelope: &WorkerEnvelope,
    identity: &ManagedRuntimeIdentity,
    response: &ExchangeResponse,
) -> Result<(), DaemonError> {
    if response.allocation_id != envelope.allocation_id
        || response.kernel_id != identity.kernel_id
        || response.runtime_release_digest != envelope.runtime_release_digest
        || response.lease_worker_capacity != 1
        || !response.home_caller.valid()
        || response.cloud_relay.machine_id != identity.machine_id
        || response.cloud_relay.account_id != response.home_caller.account_id
        || response.cloud_relay.user_id != response.home_caller.user_id
        || response.cloud_relay.realm_id != response.home_caller.realm_id
        || normalized_api_url(&response.cloud_relay.api_url)
            != normalized_api_url(&envelope.cloud_api_url)
        || !valid_managed_relay_url(&response.cloud_relay.relay_url)
        || !valid_secret(&response.cloud_relay.machine_credential, "mcred_")
    {
        return Err(worker_error("Cloud worker bootstrap response is invalid"));
    }
    Ok(())
}

fn validate_profile(
    receipt: &WorkerReceipt,
    profile: &PersistedCloudRelayProfile,
) -> Result<(), DaemonError> {
    if profile.machine_id.as_deref() != Some(receipt.machine_id.as_str())
        || profile.account_id != receipt.home_caller.account_id
        || profile.user_id != receipt.home_caller.user_id
        || profile.realm_id != receipt.home_caller.realm_id
        || profile
            .machine_credential
            .as_deref()
            .is_none_or(|value| !valid_secret(value, "mcred_"))
        || !valid_managed_relay_url(&profile.relay_url)
    {
        return Err(worker_error(
            "worker Cloud profile conflicts with its receipt",
        ));
    }
    Ok(())
}

fn supervise(
    config: &WorkerConfig,
    mut worker: PreparedWorker,
    cloud: &impl WorkerCloudClient,
) -> Result<(), DaemonError> {
    let mut delay = MIN_RETRY;
    loop {
        let started = Instant::now();
        match run_once(config, &mut worker, cloud) {
            Ok(status) => crate::logging::warn_with_fields(
                "disposable_worker_bootstrap.kernel_exit",
                "disposable worker kernel exited; restarting",
                serde_json::json!({ "status": status.code() }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "disposable_worker_bootstrap.kernel_failed",
                "disposable worker kernel failed; retrying",
                serde_json::json!({ "error": error.to_string() }),
            ),
        }
        if started.elapsed() >= STABLE_RUNTIME {
            delay = MIN_RETRY;
        }
        thread::sleep(jittered(delay));
        delay = delay.saturating_mul(2).min(MAX_RETRY);
    }
}

fn run_once(
    config: &WorkerConfig,
    worker: &mut PreparedWorker,
    cloud: &impl WorkerCloudClient,
) -> Result<std::process::ExitStatus, DaemonError> {
    let mut child = spawn_kernel(config, &worker.release, &worker.receipt)?;
    if let Some(envelope) = &worker.pending {
        let confirmation = (|| {
            confirm_when_relay_ready(&mut child, &worker.receipt, envelope, cloud)?;
            worker.receipt.status = WorkerReceiptStatus::Confirmed;
            worker.receipt.confirmed_at = Some(Utc::now().to_rfc3339());
            worker.receipt.persist(&config.receipt_path)?;
            remove_envelope(&config.envelope_path)
        })();
        if let Err(error) = confirmation {
            stop_child(&mut child)?;
            return Err(error);
        }
        worker.pending = None;
    }
    child
        .wait()
        .map_err(|error| worker_error(format!("wait for worker kernel: {error}")))
}

fn spawn_kernel(
    config: &WorkerConfig,
    release: &VerifiedRelease,
    receipt: &WorkerReceipt,
) -> Result<Child, DaemonError> {
    let home_caller = serde_json::to_string(&receipt.home_caller.lease_binding())
        .map_err(|error| worker_error(format!("encode home caller: {error}")))?;
    Command::new(&release.kernel_binary)
        .current_dir(&config.chariox_home)
        .env("CHARIOX_HOME", &config.chariox_home)
        .env("CHARIOX_KERNEL_HOST", &config.kernel_host)
        .env("CHARIOX_KERNEL_PORT", config.kernel_port.to_string())
        .env("CHARIOX_ACCEPT_REMOTE_LEASES", "1")
        .env("CHARIOX_KERNEL_RUNTIME_ROLE", "remote_lease_worker")
        .env("CHARIOX_REMOTE_LEASE_CAPACITY", "1")
        .env("CHARIOX_LEASE_WORKER_HOME_CALLER", home_caller)
        .env(ACTIVITY_RECEIPT_ENV, &config.receipt_path)
        .env_remove("CHARIOX_MANAGED_BOOTSTRAP_PATH")
        .env_remove("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT")
        .env_remove("CHARIOX_MANAGED_PROVIDER_ISOLATION")
        .env_remove("CHARIOX_DAEMON_ID")
        .env_remove("CHARIOX_MACHINE_ID")
        .env_remove("CHARIOX_RELAY_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| worker_error(format!("start worker kernel: {error}")))
}

fn confirm_when_relay_ready(
    child: &mut Child,
    receipt: &WorkerReceipt,
    envelope: &WorkerEnvelope,
    cloud: &impl WorkerCloudClient,
) -> Result<(), DaemonError> {
    let started = Instant::now();
    let mut delay = MIN_RETRY;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| worker_error(format!("inspect worker kernel: {error}")))?
        {
            return Err(worker_error(format!(
                "worker kernel exited before relay confirmation: {status}"
            )));
        }
        let profile = load_managed_cloud_relay_profile()
            .ok_or_else(|| worker_error("worker Cloud profile is missing"))?;
        let credential = profile
            .machine_credential
            .ok_or_else(|| worker_error("worker machine credential is missing"))?;
        match cloud.confirm(
            &normalized_api_url(&envelope.cloud_api_url),
            &ConfirmRequest {
                token: envelope.token.clone(),
                allocation_id: receipt.allocation_id.clone(),
                machine_id: receipt.machine_id.clone(),
                kernel_id: receipt.kernel_id.clone(),
                machine_credential: credential,
            },
        ) {
            Ok(ConfirmResponse {
                confirmed: true,
                observed_state,
            }) if observed_state == "ready" => return Ok(()),
            Ok(_) => crate::logging::warn_with_fields(
                "disposable_worker_bootstrap.confirm_pending",
                "Cloud has not confirmed worker readiness",
                serde_json::json!({ "retry_delay_ms": delay.as_millis() }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "disposable_worker_bootstrap.confirm_pending",
                "worker relay presence is not ready; retrying confirmation",
                serde_json::json!({
                    "error": error.to_string(),
                    "retry_delay_ms": delay.as_millis(),
                }),
            ),
        }
        if started.elapsed() >= CONFIRM_DEADLINE {
            return Err(worker_error(
                "worker did not become relay-ready before deadline",
            ));
        }
        thread::sleep(jittered(delay));
        delay = delay.saturating_mul(2).min(MAX_RETRY);
    }
}

fn stop_child(child: &mut Child) -> Result<(), DaemonError> {
    if child
        .try_wait()
        .map_err(|error| worker_error(format!("inspect worker kernel: {error}")))?
        .is_none()
    {
        child
            .kill()
            .map_err(|error| worker_error(format!("stop worker kernel: {error}")))?;
        child
            .wait()
            .map_err(|error| worker_error(format!("reap worker kernel: {error}")))?;
    }
    Ok(())
}

impl WorkerConfig {
    fn from_env() -> Result<Self, DaemonError> {
        let chariox_home = required_path("CHARIOX_HOME", None)?;
        let envelope_path = required_path(
            "CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH",
            Some("/var/lib/chariox/disposable-worker-bootstrap.json"),
        )?;
        let receipt_path = chariox_home.join("disposable-worker/bootstrap-receipt.json");
        let manifest_path = required_path(
            "CHARIOX_MANAGED_RELEASE_MANIFEST",
            Some("/usr/lib/chariox/release-manifest.json"),
        )?;
        let signature_path = required_path(
            "CHARIOX_MANAGED_RELEASE_SIGNATURE",
            Some("/usr/lib/chariox/release-manifest.sig"),
        )?;
        let public_key_path = required_path(
            "CHARIOX_MANAGED_RELEASE_PUBLIC_KEY",
            Some("/usr/lib/chariox/release-public-key"),
        )?;
        let kernel_binary = required_path(
            "CHARIOX_MANAGED_KERNEL_BINARY",
            Some("/usr/local/bin/chariox-kernel"),
        )?;
        let kernel_host = env::var("CHARIOX_KERNEL_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        if !matches!(kernel_host.trim(), "127.0.0.1" | "localhost" | "::1") {
            return Err(worker_error("worker kernel must bind only to loopback"));
        }
        let kernel_port = env::var("CHARIOX_KERNEL_PORT")
            .ok()
            .map(|value| value.parse::<u16>())
            .transpose()
            .map_err(|_| worker_error("worker kernel port is invalid"))?
            .unwrap_or(43118);
        if kernel_port == 0 {
            return Err(worker_error("worker kernel port is invalid"));
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

impl WorkerEnvelope {
    fn read(path: &Path) -> Result<Self, DaemonError> {
        let value: Self = read_bounded_json(path, "disposable worker envelope")?;
        if value.schema_version != 1
            || !valid_identifier(&value.allocation_id)
            || !valid_secret(&value.token, "mboot_")
            || !valid_digest(&value.runtime_release_digest)
        {
            return Err(worker_error("disposable worker envelope is invalid"));
        }
        validate_cloud_url(&value.cloud_api_url)?;
        value.expires_at()?;
        Ok(value)
    }

    fn expires_at(&self) -> Result<DateTime<Utc>, DaemonError> {
        DateTime::parse_from_rfc3339(&self.expires_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|_| worker_error("worker bootstrap expiry is invalid"))
    }
}

impl WorkerReceipt {
    fn binding_digest(&self) -> Result<String, DaemonError> {
        let binding = serde_json::json!({
            "allocationId": self.allocation_id,
            "homeCaller": self.home_caller,
            "kernelId": self.kernel_id,
            "machineId": self.machine_id,
            "relayPublicKey": self.relay_public_key,
            "runtimeReleaseDigest": self.runtime_release_digest,
            "schemaVersion": self.schema_version,
        });
        let bytes = serde_json::to_vec(&binding)
            .map_err(|error| worker_error(format!("encode worker binding: {error}")))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    fn effective_release_digest(&self, receipt_path: &Path) -> Result<String, DaemonError> {
        let release_override = super::state::DisposableWorkerReleaseOverride::read_for_receipt(
            receipt_path,
            &self.binding_digest()?,
        )?;
        if release_override.is_some() && self.status != WorkerReceiptStatus::Confirmed {
            return Err(worker_error(
                "unconfirmed worker cannot use a release override",
            ));
        }
        Ok(release_override
            .map(|value| value.runtime_release_digest)
            .unwrap_or_else(|| self.runtime_release_digest.clone()))
    }

    fn read(path: &Path) -> Result<Option<Self>, DaemonError> {
        if !path.exists() {
            return Ok(None);
        }
        let value: Self = read_bounded_json(path, "disposable worker receipt")?;
        let status_valid = match (value.status, value.confirmed_at.as_deref()) {
            (WorkerReceiptStatus::Exchanged, None) => true,
            (WorkerReceiptStatus::Confirmed, Some(time)) => {
                DateTime::parse_from_rfc3339(time).is_ok()
            }
            _ => false,
        };
        if value.schema_version != 1
            || !status_valid
            || !valid_identifier(&value.allocation_id)
            || !valid_identifier(&value.machine_id)
            || !valid_identifier(&value.kernel_id)
            || value.relay_public_key.trim().is_empty()
            || !valid_digest(&value.runtime_release_digest)
            || !value.home_caller.valid()
        {
            return Err(worker_error("disposable worker receipt is invalid"));
        }
        Ok(Some(value))
    }

    fn persist(&self, path: &Path) -> Result<(), DaemonError> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| worker_error(format!("encode worker receipt: {error}")))?;
        if bytes.len() > 96 * 1024 {
            return Err(worker_error("worker receipt is too large"));
        }
        write_private_file(path, &bytes)
            .map_err(|error| worker_error(format!("write worker receipt: {error}")))
    }

    fn validate_identity(&self, identity: &ManagedRuntimeIdentity) -> Result<(), DaemonError> {
        if self.machine_id != identity.machine_id
            || self.kernel_id != identity.kernel_id
            || self.relay_public_key != identity.relay_public_key
        {
            return Err(worker_error("worker receipt conflicts with local identity"));
        }
        Ok(())
    }
}

fn required_path(
    name: &'static str,
    default: Option<&'static str>,
) -> Result<PathBuf, DaemonError> {
    let value = env::var_os(name)
        .filter(|value| !value.is_empty())
        .or_else(|| default.map(Into::into))
        .ok_or_else(|| worker_error(format!("{name} is required")))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(worker_error(format!("{name} must be absolute")));
    }
    Ok(path)
}

fn worker_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "bootstrap disposable worker",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> WorkerEnvelope {
        WorkerEnvelope {
            schema_version: 1,
            cloud_api_url: "https://staging.chariox.com".to_string(),
            allocation_id: "worker-1".to_string(),
            token: format!("mboot_{}", "a".repeat(40)),
            expires_at: "2026-09-13T12:00:00Z".to_string(),
            runtime_release_digest: format!("sha256:{}", "b".repeat(64)),
        }
    }

    fn identity() -> ManagedRuntimeIdentity {
        ManagedRuntimeIdentity {
            machine_id: "worker-machine".to_string(),
            kernel_id: "worker-kernel".to_string(),
            relay_public_key: "worker-public-key".to_string(),
        }
    }

    fn response() -> ExchangeResponse {
        ExchangeResponse {
            allocation_id: "worker-1".to_string(),
            kernel_id: "worker-kernel".to_string(),
            runtime_release_digest: envelope().runtime_release_digest,
            lease_worker_capacity: 1,
            home_caller: CloudHomeCaller {
                account_id: "account-1".to_string(),
                user_id: "owner-1".to_string(),
                realm_id: "realm-1".to_string(),
                machine_id: "home-machine".to_string(),
                kernel_id: "home-kernel".to_string(),
                relay_public_key: "home-public-key".to_string(),
            },
            cloud_relay: ManagedCloudRelayProfile {
                api_url: "https://staging.chariox.com".to_string(),
                email: "owner@example.test".to_string(),
                account_id: "account-1".to_string(),
                user_id: "owner-1".to_string(),
                account_slug: "account-1".to_string(),
                realm_id: "realm-1".to_string(),
                relay_url: "wss://relay.example.test".to_string(),
                issuer_id: "issuer-1".to_string(),
                machine_id: "worker-machine".to_string(),
                machine_alias: "worker-1".to_string(),
                machine_credential: format!("mcred_{}", "c".repeat(40)),
            },
        }
    }

    #[test]
    fn exchange_requires_exact_worker_and_home_binding() {
        let envelope = envelope();
        let identity = identity();
        let valid = response();
        validate_exchange(&envelope, &identity, &valid).expect("valid Cloud exchange");

        let mut changed = valid.clone();
        changed.lease_worker_capacity = 2;
        assert!(validate_exchange(&envelope, &identity, &changed).is_err());
        let mut changed = valid.clone();
        changed.cloud_relay.machine_id = "other-machine".to_string();
        assert!(validate_exchange(&envelope, &identity, &changed).is_err());
        let mut changed = valid.clone();
        changed.home_caller.realm_id = "other-realm".to_string();
        assert!(validate_exchange(&envelope, &identity, &changed).is_err());
        let mut changed = valid;
        changed.home_caller.account_id = "other-account".to_string();
        assert!(validate_exchange(&envelope, &identity, &changed).is_err());
    }

    #[test]
    fn worker_receipt_keeps_the_selected_home_and_not_the_bootstrap_token() {
        let root = std::env::temp_dir().join(format!(
            "chariox-disposable-worker-receipt-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).expect("create disposable test state");
        let path = root.join("receipt.json");
        let receipt = WorkerReceipt {
            schema_version: 1,
            status: WorkerReceiptStatus::Exchanged,
            allocation_id: envelope().allocation_id,
            machine_id: identity().machine_id,
            kernel_id: identity().kernel_id,
            relay_public_key: identity().relay_public_key,
            runtime_release_digest: envelope().runtime_release_digest,
            home_caller: response().home_caller,
            confirmed_at: None,
        };
        receipt.persist(&path).expect("persist worker receipt");
        let restored = WorkerReceipt::read(&path)
            .expect("read worker receipt")
            .expect("receipt exists");
        assert_eq!(restored, receipt);
        restored
            .validate_identity(&identity())
            .expect("receipt binds local identity");
        let mut runtime =
            crate::config::DaemonConfig::new("worker-kernel", "worker-machine", "worker");
        runtime.relay_public_key = identity().relay_public_key;
        runtime.kernel_runtime_role = crate::config::KernelRuntimeRole::RemoteLeaseWorker;
        runtime.remote_lease_capacity = Some(1);
        runtime.lease_worker_home_caller = Some(receipt.home_caller.lease_binding());
        let profile = persisted_profile(response().cloud_relay);
        assert_eq!(
            activity_allocation(&path, &runtime, &profile).unwrap(),
            "worker-1"
        );
        let mut confirmed = receipt.clone();
        confirmed.status = WorkerReceiptStatus::Confirmed;
        confirmed.confirmed_at = Some("2026-09-13T11:00:00Z".to_string());
        confirmed.persist(&path).unwrap();
        assert_eq!(
            confirmed.binding_digest().unwrap(),
            "sha256:3341e3d7f98f947c0c37ff9923923cffce466d4a632ca6f238439df6e23109c7"
        );
        let upgraded_digest = format!("sha256:{}", "e".repeat(64));
        let override_bytes = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "kind": "disposable_worker_release",
            "bindingDigest": confirmed.binding_digest().unwrap(),
            "runtimeReleaseDigest": upgraded_digest,
        }))
        .unwrap();
        write_private_file(&root.join("release-override.json"), &override_bytes).unwrap();
        assert_eq!(
            confirmed.effective_release_digest(&path).unwrap(),
            upgraded_digest
        );
        assert!(receipt.effective_release_digest(&path).is_err());
        let mut wrong_home = confirmed.clone();
        wrong_home.home_caller.kernel_id = "wrong-home".into();
        assert!(wrong_home.effective_release_digest(&path).is_err());
        assert_eq!(
            activity_allocation(&path, &runtime, &profile).unwrap(),
            "worker-1"
        );
        runtime.remote_lease_capacity = None;
        assert!(activity_allocation(&path, &runtime, &profile).is_err());
        runtime.remote_lease_capacity = Some(1);
        runtime.lease_worker_home_caller = None;
        assert!(activity_allocation(&path, &runtime, &profile).is_err());
        runtime.lease_worker_home_caller = Some(receipt.home_caller.lease_binding());
        runtime.daemon_id = "other-kernel".to_string();
        assert!(activity_allocation(&path, &runtime, &profile).is_err());
        let mut changed = identity();
        changed.kernel_id = "other-kernel".to_string();
        assert!(restored.validate_identity(&changed).is_err());
        assert!(!std::fs::read_to_string(&path)
            .expect("read receipt bytes")
            .contains(&envelope().token));
        std::fs::remove_dir_all(&root).expect("remove disposable test state");
    }
}
