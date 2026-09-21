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

/// Project-environment commands require the worker's Cloud allocation to be
/// confirmed. An exchanged receipt binds identity, but it is not yet evidence
/// that the dedicated VM is admitted for project execution.
pub(crate) fn confirmed_activity_allocation(
    path: &Path,
    config: &crate::config::DaemonConfig,
    profile: &PersistedCloudRelayProfile,
) -> Result<String, DaemonError> {
    let receipt = WorkerReceipt::read(path)?
        .ok_or_else(|| worker_error("worker activity receipt is missing"))?;
    if receipt.status != WorkerReceiptStatus::Confirmed {
        return Err(worker_error(
            "worker Cloud allocation is not confirmed for project execution",
        ));
    }
    activity_allocation(path, config, profile)
}

/// Returns the Cloud-confirmed home caller for a disposable worker. A
/// remote-lease role without its confirmed receipt is not a managed-context
/// target; callers must fail closed instead of treating an ordinary worker as
/// disposable.
pub(crate) fn confirmed_disposable_worker_home_caller(
    config: &crate::config::DaemonConfig,
) -> Result<Option<CloudHomeCaller>, DaemonError> {
    if config.kernel_runtime_role != crate::config::KernelRuntimeRole::RemoteLeaseWorker {
        return Ok(None);
    }
    let receipt_path = env::var_os(ACTIVITY_RECEIPT_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| worker_error("disposable worker activity receipt path is missing"))?;
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| worker_error("worker Cloud profile is missing"))?;
    let receipt = WorkerReceipt::read(&receipt_path)?
        .ok_or_else(|| worker_error("worker activity receipt is missing"))?;
    confirmed_activity_allocation(&receipt_path, config, profile)?;
    Ok(Some(receipt.home_caller))
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
pub(crate) struct CloudHomeCaller {
    pub(crate) account_id: String,
    pub(crate) user_id: String,
    pub(crate) realm_id: String,
    pub(crate) machine_id: String,
    pub(crate) kernel_id: String,
    pub(crate) relay_public_key: String,
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
    let mut command = Command::new(&release.kernel_binary);
    command
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
        .env_remove("CHARIOX_DAEMON_ID")
        .env_remove("CHARIOX_MACHINE_ID")
        .env_remove("CHARIOX_RELAY_TOKEN")
        .env_remove("CHARIOX_DAEMON_SOCKET")
        .env_remove("CHARIOX_SLICE_DOCKER_BROKER_SOCKET")
        .env_remove("CHARIOX_SLICE_DOCKER_BROKER_FD")
        .env_remove("CHARIOX_SLICE_DOCKER_BROKER_REQUIRED")
        .env_remove("CHARIOX_MANAGED_SLICE_SERVICE_ROOT")
        .env_remove("CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT")
        .env_remove("CHARIOX_SLICE_ROOT")
        .env_remove("CHARIOX_MANAGED_PROVIDER_BWRAP")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(target_os = "linux")]
    {
        let provider_home = prepare_disposable_worker_provider_home(config)?;
        let isolation_root = match env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT") {
            Some(value) if value.is_empty() => {
                return Err(worker_error(
                    "CHARIOX_CAPABILITY_ISOLATION_ROOT must not be empty",
                ));
            }
            Some(value) => PathBuf::from(value),
            None => config.chariox_home.join("managed-context").join("kernel"),
        };
        command
            .env("CHARIOX_CAPABILITY_ISOLATION_ROOT", isolation_root)
            .env("CHARIOX_MANAGED_PROVIDER_ISOLATION", "1")
            .env("CHARIOX_MANAGED_PROVIDER_HOME", provider_home)
            .env(
                "CHARIOX_MANAGED_VAULT_PATH",
                config
                    .chariox_home
                    .join(".chariox")
                    .join("vault")
                    .join("vault.json"),
            );
    }
    command
        .spawn()
        .map_err(|error| worker_error(format!("start worker kernel: {error}")))
}

#[cfg(target_os = "linux")]
fn prepare_disposable_worker_provider_home(
    config: &WorkerConfig,
) -> Result<PathBuf, DaemonError> {
    use std::os::unix::fs::PermissionsExt;

    let path = match env::var_os("CHARIOX_MANAGED_PROVIDER_HOME") {
        Some(value) if value.is_empty() => {
            return Err(worker_error(
                "CHARIOX_MANAGED_PROVIDER_HOME must not be empty",
            ));
        }
        Some(value) => PathBuf::from(value),
        None => config
            .chariox_home
            .parent()
            .unwrap_or(&config.chariox_home)
            .join("provider-home"),
    };
    if !path.is_absolute()
        || path == Path::new("/")
        || path.starts_with(config.chariox_home.join(".chariox"))
    {
        return Err(worker_error(
            "managed provider HOME must be absolute, non-root, and separate from kernel state",
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(worker_error(
            "managed provider HOME must not contain a parent-directory component",
        ));
    }
    let mut current = path.clone();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(worker_error(
                        "managed provider HOME must not traverse symlinked directories",
                    ));
                }
                if !metadata.is_dir() {
                    return Err(worker_error(
                        "managed provider HOME has a non-directory ancestor",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(worker_error(format!(
                    "inspect managed provider HOME ancestor: {error}"
                )));
            }
        }
        if current == Path::new("/") {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }
    std::fs::create_dir_all(&path)
        .map_err(|error| worker_error(format!("create managed provider HOME: {error}")))?;
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| worker_error(format!("inspect managed provider HOME: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(worker_error(
            "managed provider HOME must be a real directory",
        ));
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| worker_error(format!("protect managed provider HOME: {error}")))?;
    let canonical = path
        .canonicalize()
        .map_err(|error| worker_error(format!("canonicalize managed provider HOME: {error}")))?;
    if canonical == Path::new("/") {
        return Err(worker_error(
            "managed provider HOME must not resolve to the root directory",
        ));
    }
    Ok(canonical)
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
        validate_profile(receipt, &profile)?;
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
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    use chariox_relay::{RelayAction, RelayAuthVerifier, RelayTokenClaims, ScopedTokenVerifier};

    use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
    use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
    use crate::managed_bootstrap::ManagedKernelContextPlan;
    use crate::managed_context::outbound_service::ManagedContextTransferTicket;
    use crate::runtime::command::KernelCommand;
    use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

    #[cfg(target_os = "linux")]
    #[test]
    fn disposable_worker_spawn_propagates_managed_provider_isolation_and_scrubs_parent_state() {
        use std::os::unix::fs::PermissionsExt;

        let _lock = crate::env_lock::lock();
        let fixture = super::super::tests::Fixture::new("worker-isolation-topology");
        let config = WorkerConfig {
            chariox_home: fixture.config.chariox_home.clone(),
            envelope_path: fixture.config.envelope_path.clone(),
            receipt_path: fixture
                .config
                .chariox_home
                .join("disposable-worker/bootstrap-receipt.json"),
            manifest_path: fixture.config.manifest_path.clone(),
            signature_path: fixture.config.signature_path.clone(),
            public_key_path: fixture.config.public_key_path.clone(),
            kernel_binary: fixture.config.kernel_binary.clone(),
            kernel_host: fixture.config.kernel_host.clone(),
            kernel_port: fixture.config.kernel_port,
        };
        fs::create_dir_all(&config.chariox_home).expect("worker HOME should exist");
        let marker = config.chariox_home.join("worker-isolation-env.txt");
        let provider_home = config
            .chariox_home
            .parent()
            .expect("worker fixture should have a parent")
            .join("provider-home");
        let capability_root = config.chariox_home.join("managed-context/kernel");
        let kernel_script = b"#!/bin/sh\nset -eu\nmarker=\"${CHARIOX_WORKER_ISOLATION_PROBE_MARKER:?}\"\nprintf 'isolation=%s\\n' \"${CHARIOX_MANAGED_PROVIDER_ISOLATION-<unset>}\" > \"$marker\"\nprintf 'provider_home=%s\\n' \"${CHARIOX_MANAGED_PROVIDER_HOME-<unset>}\" >> \"$marker\"\nprintf 'capability_root=%s\\n' \"${CHARIOX_CAPABILITY_ISOLATION_ROOT-<unset>}\" >> \"$marker\"\nprintf 'vault=%s\\n' \"${CHARIOX_MANAGED_VAULT_PATH-<unset>}\" >> \"$marker\"\nprintf 'daemon_socket=%s\\n' \"${CHARIOX_DAEMON_SOCKET-<unset>}\" >> \"$marker\"\nprintf 'broker_socket=%s\\n' \"${CHARIOX_SLICE_DOCKER_BROKER_SOCKET-<unset>}\" >> \"$marker\"\nprintf 'slice_root=%s\\n' \"${CHARIOX_SLICE_ROOT-<unset>}\" >> \"$marker\"\nprintf 'relay_token=%s\\n' \"${CHARIOX_RELAY_TOKEN-<unset>}\" >> \"$marker\"\nprintf 'daemon_id=%s\\n' \"${CHARIOX_DAEMON_ID-<unset>}\" >> \"$marker\"\nprintf 'machine_id=%s\\n' \"${CHARIOX_MACHINE_ID-<unset>}\" >> \"$marker\"\nprintf 'bootstrap_path=%s\\n' \"${CHARIOX_MANAGED_BOOTSTRAP_PATH-<unset>}\" >> \"$marker\"\nprintf ordinary > \"$HOME/ordinary-worker-write\"\n";
        fs::write(&config.kernel_binary, kernel_script).expect("write worker probe kernel");
        fs::set_permissions(&config.kernel_binary, fs::Permissions::from_mode(0o755))
            .expect("make worker probe kernel executable");

        let names = [
            "HOME",
            "CHARIOX_WORKER_ISOLATION_PROBE_MARKER",
            "CHARIOX_MANAGED_PROVIDER_ISOLATION",
            "CHARIOX_MANAGED_PROVIDER_HOME",
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            "CHARIOX_MANAGED_VAULT_PATH",
            "CHARIOX_DAEMON_SOCKET",
            "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
            "CHARIOX_SLICE_ROOT",
            "CHARIOX_RELAY_TOKEN",
            "CHARIOX_DAEMON_ID",
            "CHARIOX_MACHINE_ID",
            "CHARIOX_MANAGED_BOOTSTRAP_PATH",
        ];
        let previous = names
            .iter()
            .map(|name| (*name, env::var_os(name)))
            .collect::<Vec<_>>();
        env::set_var("HOME", &config.chariox_home);
        env::set_var("CHARIOX_WORKER_ISOLATION_PROBE_MARKER", &marker);
        env::set_var("CHARIOX_MANAGED_PROVIDER_ISOLATION", "0");
        env::set_var("CHARIOX_MANAGED_PROVIDER_HOME", &provider_home);
        env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &capability_root);
        env::set_var(
            "CHARIOX_MANAGED_VAULT_PATH",
            "/host/credentials/vault.json",
        );
        env::set_var("CHARIOX_DAEMON_SOCKET", "/run/chariox/daemon.sock");
        env::set_var(
            "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
            "/run/chariox/broker.sock",
        );
        env::set_var("CHARIOX_SLICE_ROOT", "/var/lib/chariox-slice-share");
        env::set_var("CHARIOX_RELAY_TOKEN", "mrelay_parent_secret");
        env::set_var("CHARIOX_DAEMON_ID", "parent-daemon");
        env::set_var("CHARIOX_MACHINE_ID", "parent-machine");
        env::set_var(
            "CHARIOX_MANAGED_BOOTSTRAP_PATH",
            "/var/lib/chariox/bootstrap.json",
        );

        let response = response();
        let receipt = WorkerReceipt {
            schema_version: 1,
            status: WorkerReceiptStatus::Confirmed,
            allocation_id: response.allocation_id,
            machine_id: "worker-machine".to_string(),
            kernel_id: response.kernel_id,
            relay_public_key: "worker-public-key".to_string(),
            runtime_release_digest: response.runtime_release_digest,
            home_caller: response.home_caller,
            confirmed_at: Some("2026-09-16T15:38:00Z".to_string()),
        };
        let release = VerifiedRelease {
            digest: "sha256:worker-isolation-probe".to_string(),
            kernel_binary: config.kernel_binary.clone(),
        };
        let mut child = spawn_kernel(&config, &release, &receipt)
            .expect("disposable worker should start the managed kernel");
        let status = child.wait().expect("worker probe kernel should exit");

        for (name, value) in previous {
            restore_worker_test_env(name, value);
        }

        assert!(status.success(), "worker probe kernel failed: {status}");
        let observed = fs::read_to_string(&marker).expect("worker probe should record its env");
        assert!(observed.contains("isolation=1"));
        assert!(observed.contains(&format!("provider_home={}\n", provider_home.display())));
        assert!(observed.contains(&format!(
            "capability_root={}\n",
            capability_root.display()
        )));
        assert!(observed.contains(&format!(
            "vault={}\n",
            config.chariox_home.join(".chariox/vault/vault.json").display()
        )));
        for name in [
            "daemon_socket",
            "broker_socket",
            "slice_root",
            "relay_token",
            "daemon_id",
            "machine_id",
            "bootstrap_path",
        ] {
            assert!(
                observed.contains(&format!("{name}=<unset>\n")),
                "worker provider must not inherit {name}: {observed}"
            );
        }
        assert!(config.chariox_home.join("ordinary-worker-write").is_file());
        let provider_mode = fs::metadata(&provider_home)
            .expect("managed provider HOME should be prepared")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(provider_mode, 0o700);
        fixture.cleanup();
    }

    #[cfg(unix)]
    #[test]
    fn confirmation_retry_revalidates_the_installed_worker_profile() {
        use std::sync::Mutex;
        struct Cloud {
            calls: Mutex<usize>,
            replace_profile: bool,
        }
        impl WorkerCloudClient for Cloud {
            fn exchange(
                &self,
                _: &str,
                _: &ExchangeRequest,
            ) -> Result<ExchangeResponse, DaemonError> {
                panic!("confirmation must not exchange credentials")
            }
            fn confirm(&self, _: &str, _: &ConfirmRequest) -> Result<ConfirmResponse, DaemonError> {
                let mut calls = self.calls.lock().unwrap();
                *calls += 1;
                if *calls == 1 {
                    if self.replace_profile {
                        let mut profile = load_managed_cloud_relay_profile().unwrap();
                        profile.machine_id = Some("different-worker".into());
                        persist_managed_cloud_relay_profile(profile).unwrap();
                    }
                    return Err(worker_error("simulated lost confirmation response"));
                }
                Ok(ConfirmResponse {
                    confirmed: true,
                    observed_state: "ready".into(),
                })
            }
        }
        let _lock = crate::env_lock::lock();
        for replace_profile in [false, true] {
            let fixture = super::super::tests::Fixture::new("worker-confirm-retry");
            let prior_home = env::var_os("CHARIOX_HOME");
            env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
            let response = response();
            let receipt = WorkerReceipt {
                schema_version: 1,
                status: WorkerReceiptStatus::Exchanged,
                allocation_id: response.allocation_id,
                machine_id: "worker-machine".into(),
                kernel_id: response.kernel_id,
                relay_public_key: "worker-public-key".into(),
                runtime_release_digest: response.runtime_release_digest,
                home_caller: response.home_caller,
                confirmed_at: None,
            };
            persist_managed_cloud_relay_profile(persisted_profile(response.cloud_relay)).unwrap();
            let cloud = Cloud {
                calls: Mutex::new(0),
                replace_profile,
            };
            let mut child = Command::new("sleep").arg("15").spawn().unwrap();
            let result = confirm_when_relay_ready(&mut child, &receipt, &envelope(), &cloud);
            stop_child(&mut child).unwrap();
            match prior_home {
                Some(value) => env::set_var("CHARIOX_HOME", value),
                None => env::remove_var("CHARIOX_HOME"),
            }
            fixture.cleanup();
            if replace_profile {
                assert!(
                    result.is_err(),
                    "must reject a replaced worker profile before retrying"
                );
                assert_eq!(*cloud.calls.lock().unwrap(), 1);
            } else {
                result.unwrap();
                assert_eq!(*cloud.calls.lock().unwrap(), 2);
            }
        }
    }

    #[test]
    fn active_bootstrap_retries_same_identity_and_resumes_confirmed_without_exchange() {
        use std::sync::Mutex;
        struct Cloud(Mutex<Vec<serde_json::Value>>);
        impl WorkerCloudClient for Cloud {
            fn exchange(
                &self,
                api: &str,
                request: &ExchangeRequest,
            ) -> Result<ExchangeResponse, DaemonError> {
                let mut calls = self.0.lock().unwrap();
                calls.push(serde_json::to_value(request).unwrap());
                if calls.len() == 1 {
                    return Err(worker_error("simulated lost exchange response"));
                }
                assert_eq!(
                    calls[0], calls[1],
                    "retry must retain the grant-bound worker identity"
                );
                let mut result = response();
                result.kernel_id = request.kernel_id.clone();
                result.runtime_release_digest = request.runtime_release_digest.clone();
                result.cloud_relay.machine_id = request.machine_id.clone();
                result.cloud_relay.api_url = api.into();
                Ok(result)
            }
            fn confirm(&self, _: &str, _: &ConfirmRequest) -> Result<ConfirmResponse, DaemonError> {
                panic!("preparation must not confirm before the child starts")
            }
        }
        let _lock = crate::env_lock::lock();
        let fixture = super::super::tests::Fixture::new("active-worker-recovery");
        let base = &fixture.config;
        let config = WorkerConfig {
            chariox_home: base.chariox_home.clone(),
            envelope_path: base.envelope_path.clone(),
            receipt_path: base
                .chariox_home
                .join("disposable-worker/bootstrap-receipt.json"),
            manifest_path: base.manifest_path.clone(),
            signature_path: base.signature_path.clone(),
            public_key_path: base.public_key_path.clone(),
            kernel_binary: base.kernel_binary.clone(),
            kernel_host: base.kernel_host.clone(),
            kernel_port: base.kernel_port,
        };
        let prior_home = env::var_os("CHARIOX_HOME");
        env::set_var("CHARIOX_HOME", &config.chariox_home);
        let now = DateTime::parse_from_rfc3339("2026-09-13T11:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let envelope = serde_json::json!({
            "schemaVersion": 1, "cloudApiUrl": "https://cloud.example.test",
            "allocationId": "worker-1", "token": format!("mboot_{}", "a".repeat(40)),
            "expiresAt": "2026-09-13T12:00:00Z", "runtimeReleaseDigest": fixture.release_digest,
        });
        write_private_file(
            &config.envelope_path,
            &serde_json::to_vec(&envelope).unwrap(),
        )
        .unwrap();
        let cloud = Cloud(Mutex::new(Vec::new()));
        assert!(prepare(&config, &cloud, now).is_err());
        assert!(!config.receipt_path.exists());
        let prepared = prepare(&config, &cloud, now).unwrap();
        assert!(prepared.pending.is_some());
        let mut receipt = prepared.receipt;
        receipt.status = WorkerReceiptStatus::Confirmed;
        receipt.confirmed_at = Some(now.to_rfc3339());
        receipt.persist(&config.receipt_path).unwrap();
        let resumed = prepare(&config, &cloud, now + chrono::Duration::days(1)).unwrap();
        assert!(resumed.pending.is_none());
        assert_eq!(resumed.receipt.home_caller, receipt.home_caller);
        assert_eq!(cloud.0.lock().unwrap().len(), 2);
        assert!(!config.envelope_path.exists());
        match prior_home {
            Some(value) => env::set_var("CHARIOX_HOME", value),
            None => env::remove_var("CHARIOX_HOME"),
        }
        fixture.cleanup();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn confirmed_disposable_worker_materializes_selected_project_via_public_context_transfer()
    {
        use std::sync::{Arc, Mutex as StdMutex};

        use chariox_relay::{RelayConfig, RelayServer};
        use tokio::sync::{oneshot, watch, Mutex};
        use tokio::time::{sleep, timeout};

        use crate::app::DaemonApp;
        use crate::config::KernelRuntimeRole;
        use crate::local::{
            GetManagedContextTransferStatusRequest, StartManagedContextTransferRequest,
            UpdateProjectWorkspacesRequest,
        };
        use crate::managed_context::development::DevelopmentRepositoryRole;
        use crate::managed_context::outbound_service::{
            ManagedContextOutboundOperationPhase, ManagedContextTransferTarget,
        };
        use crate::session::{CreateSessionRequest, SessionAgentDefaults, SessionProjectSelection};

        let _env = crate::env_lock::lock();
        let fixture = super::super::tests::Fixture::new("disposable-project-transfer");
        let task_root = fixture
            .config
            .chariox_home
            .parent()
            .expect("fixture home parent")
            .to_path_buf();
        let source_git = task_root.join("selected-git");
        let source_directory = task_root.join("selected-directory");
        let home_root = task_root.join("home-daemon");
        let worker_root = task_root.join("worker-daemon");
        let target_workspace_parent = worker_root.join("state").join("managed-context-workspaces");
        let unrelated_worker_file = target_workspace_parent.join("unrelated-preserved.txt");
        let worker_started_marker = fixture
            .config
            .chariox_home
            .join("disposable-worker/kernel-started");

        let previous_home = env::var_os("HOME");
        let previous_chariox_home = env::var_os("CHARIOX_HOME");
        let previous_receipt = env::var_os(ACTIVITY_RECEIPT_ENV);
        let previous_local_auth = env::var_os("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE");
        let previous_started_marker = env::var_os("CHARIOX_KERNEL_STARTED_MARKER");
        env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
        env::set_var("HOME", &task_root);
        env::set_var("CHARIOX_KERNEL_STARTED_MARKER", &worker_started_marker);

        let mut worker_app_config = isolated_worker_test_config(
            &worker_root,
            "worker-kernel",
            "worker-machine",
            "disposable-worker",
            43119,
        );
        let worker_private_key = worker_app_config.relay_private_key.clone();
        let worker_public_key = worker_app_config.relay_public_key.clone();
        init_worker_test_identity_with_key(
            &fixture.config.chariox_home,
            "worker-kernel",
            "worker-machine",
            &worker_private_key,
            &worker_public_key,
        );

        let worker_config = WorkerConfig {
            chariox_home: fixture.config.chariox_home.clone(),
            envelope_path: fixture.config.envelope_path.clone(),
            receipt_path: fixture
                .config
                .chariox_home
                .join("disposable-worker/bootstrap-receipt.json"),
            manifest_path: fixture.config.manifest_path.clone(),
            signature_path: fixture.config.signature_path.clone(),
            public_key_path: fixture.config.public_key_path.clone(),
            kernel_binary: fixture.config.kernel_binary.clone(),
            kernel_host: fixture.config.kernel_host.clone(),
            kernel_port: fixture.config.kernel_port,
        };
        let test_now = chrono::Utc::now();
        let worker_cloud_api_url = "https://cloud.example.test".to_string();
        let worker_envelope = serde_json::json!({
            "schemaVersion": 1,
            "cloudApiUrl": worker_cloud_api_url,
            "allocationId": "allocation-disposable-project",
            "token": format!("mboot_{}", "d".repeat(40)),
            "expiresAt": (test_now + chrono::Duration::hours(1)).to_rfc3339(),
            "runtimeReleaseDigest": fixture.release_digest.clone(),
        });
        write_private_file(
            &worker_config.envelope_path,
            &serde_json::to_vec(&worker_envelope).expect("encode worker envelope"),
        )
        .expect("write worker envelope");

        let home_machine_id = "source-machine-test";
        let home_kernel_id = "home-kernel";
        let mut home_app_config = isolated_worker_test_config(
            &home_root,
            home_kernel_id,
            home_machine_id,
            "home-kernel",
            43120,
        );
        let home_public_key = home_app_config.relay_public_key.clone();
        let exchange_response = {
            let mut response = response();
            response.allocation_id = "allocation-disposable-project".to_string();
            response.kernel_id = "worker-kernel".to_string();
            response.runtime_release_digest = fixture.release_digest.clone();
            response.home_caller = CloudHomeCaller {
                account_id: "account-1".to_string(),
                user_id: "local".to_string(),
                realm_id: "default".to_string(),
                machine_id: home_machine_id.to_string(),
                kernel_id: home_kernel_id.to_string(),
                relay_public_key: home_public_key.clone(),
            };
            response.cloud_relay.api_url = worker_cloud_api_url.clone();
            response.cloud_relay.account_id = "account-1".to_string();
            response.cloud_relay.user_id = "local".to_string();
            response.cloud_relay.realm_id = "default".to_string();
            response.cloud_relay.machine_id = "worker-machine".to_string();
            response
        };
        let worker_cloud = EnrollmentCloud {
            response: exchange_response,
            confirmations: StdMutex::new(Vec::new()),
        };
        let mut prepared = prepare(&worker_config, &worker_cloud, test_now)
            .expect("worker enrollment should create a receipt");
        assert_eq!(prepared.receipt.status, WorkerReceiptStatus::Exchanged);
        assert!(prepared.pending.is_some());

        let mut worker_runtime = DaemonConfig::new(
            prepared.receipt.kernel_id.clone(),
            prepared.receipt.machine_id.clone(),
            "tester",
        );
        worker_runtime.relay_public_key = prepared.receipt.relay_public_key.clone();
        worker_runtime.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
        worker_runtime.remote_lease_capacity = Some(1);
        worker_runtime.lease_worker_home_caller =
            Some(prepared.receipt.home_caller.lease_binding());
        assert!(
            confirmed_activity_allocation(
                &worker_config.receipt_path,
                &worker_runtime,
                &persisted_profile(worker_cloud.response.cloud_relay.clone()),
            )
            .is_err(),
            "an exchanged receipt must not authorize project execution"
        );

        let local_auth_path = fixture.config.chariox_home.join("worker-local-auth-token");
        write_private_file(&local_auth_path, b"test-local-auth")
            .expect("write synthetic worker auth fixture");
        env::set_var("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE", &local_auth_path);
        run_once(&worker_config, &mut prepared, &worker_cloud)
            .expect("confirmed worker bootstrap should persist the receipt");
        assert_eq!(prepared.receipt.status, WorkerReceiptStatus::Confirmed);
        assert!(prepared.pending.is_none());
        assert!(!worker_config.envelope_path.exists());
        assert_eq!(
            confirmed_activity_allocation(
                &worker_config.receipt_path,
                &worker_runtime,
                &persisted_profile(worker_cloud.response.cloud_relay.clone()),
            )
            .expect("confirmed receipt should authorize the worker runtime"),
            prepared.receipt.allocation_id
        );
        assert_eq!(
            worker_cloud
                .confirmations
                .lock()
                .expect("confirmation lock")
                .len(),
            1,
            "enrollment must confirm the actual worker before transfer"
        );

        init_test_repository(&source_git, "selected.txt", "selected source\n");
        fs::create_dir_all(&source_directory).expect("selected directory should exist");
        fs::write(
            source_directory.join("supporting.txt"),
            "selected supporting source\n",
        )
        .expect("selected directory file should exist");
        fs::create_dir_all(&target_workspace_parent).expect("target workspace parent");
        fs::write(&unrelated_worker_file, "untouched worker state\n")
            .expect("unrelated worker state should exist");

        let cloud_api = TestCloudApi::bind();
        let relay_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("relay listener should bind");
        let relay_address = relay_listener.local_addr().expect("relay address");
        home_app_config.relay_url = Some(format!("ws://{relay_address}"));
        home_app_config.relay_token = Some("relay-home-token".to_string());
        home_app_config.cloud_relay = Some(test_cloud_profile(
            cloud_api.url.clone(),
            home_machine_id,
            "mcred_home_test",
            "default",
        ));
        worker_app_config.relay_url = Some(format!("ws://{relay_address}"));
        worker_app_config.relay_token = Some("relay-worker-token".to_string());
        worker_app_config.cloud_relay = Some(test_cloud_profile(
            cloud_api.url.clone(),
            "worker-machine",
            &format!("mcred_{}", "w".repeat(40)),
            "default",
        ));
        worker_app_config.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
        worker_app_config.accept_remote_leases = true;
        worker_app_config.remote_lease_capacity = Some(1);
        worker_app_config.lease_worker_home_caller =
            Some(prepared.receipt.home_caller.lease_binding());

        let relay_auth = test_relay_auth(
            (
                "relay-home-token",
                home_kernel_id,
                home_machine_id,
                &home_public_key,
            ),
            (
                "relay-worker-token",
                &worker_app_config.daemon_id,
                &worker_app_config.host_machine_id,
                &worker_public_key,
            ),
        );
        let relay = Arc::new(RelayServer::with_auth_verifier(
            RelayConfig {
                host: relay_address.ip().to_string(),
                port: relay_address.port(),
                shared_token: None,
            },
            relay_auth,
        ));
        let relay_registry = relay.registry();
        let (relay_shutdown_tx, relay_shutdown_rx) = oneshot::channel();
        let relay_task = {
            let relay = Arc::clone(&relay);
            tokio::spawn(async move {
                relay
                    .run_listener_until(relay_listener, async {
                        let _ = relay_shutdown_rx.await;
                    })
                    .await
                    .expect("test relay should run");
            })
        };

        let home_app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(home_app_config.clone()).expect("home app should bootstrap"),
        ));
        // Mirror the environment that spawn_kernel gives the confirmed
        // disposable worker. The receipt was produced by prepare/run_once;
        // this test does not manufacture a registration or context receipt.
        env::set_var(ACTIVITY_RECEIPT_ENV, &worker_config.receipt_path);
        env::set_var("CHARIOX_KERNEL_RUNTIME_ROLE", "remote_lease_worker");
        env::set_var("CHARIOX_ACCEPT_REMOTE_LEASES", "1");
        env::set_var("CHARIOX_REMOTE_LEASE_CAPACITY", "1");
        env::set_var(
            "CHARIOX_LEASE_WORKER_HOME_CALLER",
            serde_json::to_string(&prepared.receipt.home_caller.lease_binding())
                .expect("encode worker home caller"),
        );
        let worker_app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(worker_app_config.clone()).expect("worker app should bootstrap"),
        ));
        let home_router = Arc::new(
            crate::runtime::router::CommandRouter::with_interactive_capacity(
                Arc::clone(&home_app),
                1,
            ),
        );
        let worker_router = Arc::new(
            crate::runtime::router::CommandRouter::with_interactive_capacity(
                Arc::clone(&worker_app),
                1,
            ),
        );
        let home_state = home_app.lock().await.relay_client_state();
        let worker_state = worker_app.lock().await.relay_client_state();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let home_connector = tokio::spawn(
            crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
                Arc::clone(&home_router),
                home_state,
                shutdown_rx.clone(),
                format!("ws://{relay_address}"),
                "relay-home-token".to_string(),
            ),
        );
        let worker_connector = tokio::spawn(
            crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
                Arc::clone(&worker_router),
                worker_state,
                shutdown_rx,
                format!("ws://{relay_address}"),
                "relay-worker-token".to_string(),
            ),
        );
        timeout(Duration::from_secs(5), async {
            loop {
                let registered = relay_registry.read().await;
                let ready = registered.daemon(home_kernel_id).is_some()
                    && registered.daemon(&worker_app_config.daemon_id).is_some();
                drop(registered);
                if ready {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("home and confirmed worker should register on the relay");

        let seeded = dispatch_public(
            &home_router,
            LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new(
                    source_git.display().to_string(),
                    source_git.display().to_string(),
                )
                .with_project_selection(SessionProjectSelection::New)
                .with_agent_defaults(SessionAgentDefaults::new("default")),
            ),
        )
        .await
        .expect("home should create the selected Project");
        let (seed_session_id, project_id) = match seeded {
            LocalDaemonResponse::SessionCreated { session, .. } => {
                (session.id().to_string(), session.project_id().to_string())
            }
            other => panic!("unexpected Project seed response: {other:?}"),
        };
        let updated = dispatch_public(
            &home_router,
            LocalDaemonRequest::UpdateProjectWorkspaces(UpdateProjectWorkspacesRequest {
                project_id: project_id.clone(),
                workspace_ids: vec![
                    source_git.display().to_string(),
                    source_directory.display().to_string(),
                ],
            }),
        )
        .await
        .expect("home should select both Project workspaces");
        match updated {
            LocalDaemonResponse::ProjectWorkspacesUpdated { project } => assert_eq!(
                project.workspace_ids(),
                &[
                    source_git.display().to_string(),
                    source_directory.display().to_string()
                ]
            ),
            other => panic!("unexpected Project update response: {other:?}"),
        }

        let context_plan = ManagedKernelContextPlan::source_project_with_repositories_for_tests(
            "context-disposable-project",
            "default",
            home_machine_id,
            home_kernel_id,
            &crate::runtime::terminal_pairings::public_key_thumbprint(&home_public_key),
            &project_id,
            vec![
                (
                    DevelopmentRepositoryRole::Primary,
                    source_git.display().to_string(),
                    None,
                ),
                (
                    DevelopmentRepositoryRole::Supporting,
                    source_directory.display().to_string(),
                    None,
                ),
            ],
        );
        let ticket = ManagedContextTransferTicket {
            environment_id: "environment-disposable-project".to_string(),
            context_plan,
            target: ManagedContextTransferTarget {
                relay_realm_id: "default".to_string(),
                machine_id: worker_app_config.host_machine_id.clone(),
                kernel_id: worker_app_config.daemon_id.clone(),
                relay_public_key: worker_public_key.clone(),
                key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                    &worker_public_key,
                ),
            },
        };
        cloud_api.set_ticket(ticket.clone());

        let started = dispatch_public(
            &home_router,
            LocalDaemonRequest::StartManagedContextTransfer(StartManagedContextTransferRequest {
                ticket: ticket.clone(),
            }),
        )
        .await;
        let mut final_status = None;
        if started.is_ok() {
            for _ in 0..200 {
                let status = dispatch_public(
                    &home_router,
                    LocalDaemonRequest::GetManagedContextTransferStatus(
                        GetManagedContextTransferStatusRequest {
                            context_id: ticket.context_plan.context_id().to_string(),
                        },
                    ),
                )
                .await;
                if let Ok(LocalDaemonResponse::ManagedContextTransferStatus { status }) = status {
                    if matches!(
                        status.phase,
                        ManagedContextOutboundOperationPhase::Completed
                            | ManagedContextOutboundOperationPhase::Failed
                    ) {
                        final_status = Some(status);
                        break;
                    }
                }
                sleep(Duration::from_millis(25)).await;
            }
        }

        let launch_result = if final_status
            .as_ref()
            .is_some_and(|status| status.phase == ManagedContextOutboundOperationPhase::Completed)
        {
            Some(
                worker_app
                    .lock()
                    .await
                    .managed_context_transfer_store()
                    .launch_target(
                        ticket.context_plan.context_id(),
                        &ticket.context_plan.package_binding().plan_digest,
                    ),
            )
        } else {
            None
        };

        let _ = shutdown_tx.send(true);
        timeout(Duration::from_secs(5), home_connector)
            .await
            .expect("home connector should stop")
            .expect("home connector should join");
        timeout(Duration::from_secs(5), worker_connector)
            .await
            .expect("worker connector should stop")
            .expect("worker connector should join");
        let _ = relay_shutdown_tx.send(());
        timeout(Duration::from_secs(5), relay_task)
            .await
            .expect("relay task should stop")
            .expect("relay task should join");
        let cloud_requests = cloud_api.stop();

        restore_worker_test_env("HOME", previous_home);
        restore_worker_test_env("CHARIOX_HOME", previous_chariox_home);
        restore_worker_test_env(ACTIVITY_RECEIPT_ENV, previous_receipt);
        restore_worker_test_env("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE", previous_local_auth);
        restore_worker_test_env("CHARIOX_KERNEL_STARTED_MARKER", previous_started_marker);
        env::remove_var("CHARIOX_KERNEL_RUNTIME_ROLE");
        env::remove_var("CHARIOX_ACCEPT_REMOTE_LEASES");
        env::remove_var("CHARIOX_REMOTE_LEASE_CAPACITY");
        env::remove_var("CHARIOX_LEASE_WORKER_HOME_CALLER");

        assert!(
            started.is_ok(),
            "public managed-worker transfer should start from the enrolled home: {started:?}"
        );
        let final_status = final_status.expect("public transfer should reach a terminal status");
        assert_eq!(
            final_status.phase,
            ManagedContextOutboundOperationPhase::Completed,
            "confirmed disposable worker should accept the selected Project transfer: {final_status:?}"
        );
        assert!(
            cloud_requests
                .iter()
                .any(|(path, _)| path.ends_with("/context/ticket")),
            "home must fetch the authoritative context ticket"
        );
        assert!(
            cloud_requests
                .iter()
                .any(|(path, _)| path.ends_with("/context/complete")),
            "worker must report the completed import to Cloud"
        );
        let ticket_request = cloud_requests
            .iter()
            .find(|(path, _)| path.ends_with("/context/ticket"))
            .expect("home must request an authoritative ticket");
        assert_eq!(ticket_request.1["machineId"], home_machine_id);
        assert_eq!(ticket_request.1["kernelId"], home_kernel_id);
        assert_eq!(ticket_request.1["relayRealmId"], "default");
        let Some(Ok(target)) = launch_result else {
            panic!("worker should retain the completed launch target: {launch_result:?}");
        };
        let crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
            repositories, ..
        } = target.development
        else {
            panic!("selected Project should produce source repositories");
        };
        assert_eq!(repositories.len(), 2);
        for repository in repositories {
            let path = std::path::PathBuf::from(repository.workspace_path);
            match repository.role {
                DevelopmentRepositoryRole::Primary => assert_eq!(
                    fs::read_to_string(path.join("selected.txt"))
                        .expect("selected Git file should be copied"),
                    "selected source\n"
                ),
                DevelopmentRepositoryRole::Supporting => assert_eq!(
                    fs::read_to_string(path.join("supporting.txt"))
                        .expect("selected directory file should be copied"),
                    "selected supporting source\n"
                ),
            }
        }
        assert_eq!(
            fs::read_to_string(&unrelated_worker_file).expect("unrelated worker state"),
            "untouched worker state\n"
        );
        assert_eq!(
            fs::read_to_string(source_git.join("selected.txt"))
                .expect("source Git file should remain intact"),
            "selected source\n"
        );
        assert_eq!(
            fs::read_to_string(source_directory.join("supporting.txt"))
                .expect("source directory file should remain intact"),
            "selected supporting source\n"
        );
        assert!(
            !seed_session_id.is_empty(),
            "the public Project selection must have created a home session"
        );
        fixture.cleanup();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn authenticated_peer_worker_setup_preserves_attempt_two_and_rejects_replays() {
        use crate::app::DaemonApp;
        use crate::config::KernelRuntimeRole;
        use crate::transport::relay_client::send_authenticated_peer_request_for_test;

        let _env = crate::env_lock::lock();
        let fixture = super::super::tests::Fixture::new("peer-attempt-two");
        let task_root = fixture
            .config
            .chariox_home
            .parent()
            .expect("fixture home parent")
            .to_path_buf();
        let worker_root = task_root.join("worker-daemon");
        let worker_worktree = task_root.join("worker-worktree");
        fs::create_dir_all(&worker_worktree).expect("worker worktree should exist");

        let previous_chariox_home = env::var_os("CHARIOX_HOME");
        let previous_receipt = env::var_os(ACTIVITY_RECEIPT_ENV);
        let previous_local_auth = env::var_os("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE");
        let previous_started_marker = env::var_os("CHARIOX_KERNEL_STARTED_MARKER");
        let worker_started_marker = fixture
            .config
            .chariox_home
            .join("disposable-worker/kernel-started");
        env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
        env::set_var("CHARIOX_KERNEL_STARTED_MARKER", &worker_started_marker);

        let mut worker_app_config = isolated_worker_test_config(
            &worker_root,
            "worker-kernel",
            "worker-machine",
            "disposable-worker",
            43121,
        );
        let worker_private_key = worker_app_config.relay_private_key.clone();
        let worker_public_key = worker_app_config.relay_public_key.clone();
        init_worker_test_identity_with_key(
            &fixture.config.chariox_home,
            "worker-kernel",
            "worker-machine",
            &worker_private_key,
            &worker_public_key,
        );

        let worker_config = WorkerConfig {
            chariox_home: fixture.config.chariox_home.clone(),
            envelope_path: fixture.config.envelope_path.clone(),
            receipt_path: fixture
                .config
                .chariox_home
                .join("disposable-worker/bootstrap-receipt.json"),
            manifest_path: fixture.config.manifest_path.clone(),
            signature_path: fixture.config.signature_path.clone(),
            public_key_path: fixture.config.public_key_path.clone(),
            kernel_binary: fixture.config.kernel_binary.clone(),
            kernel_host: fixture.config.kernel_host.clone(),
            kernel_port: fixture.config.kernel_port,
        };
        let test_now = chrono::Utc::now();
        let home_private_key = crate::transport::relay_crypto::generate_private_key_base64();
        let home_public_key = crate::transport::relay_crypto::public_key_from_private_key_base64(
            &home_private_key,
        )
        .expect("home public key should derive");
        let worker_envelope = serde_json::json!({
            "schemaVersion": 1,
            "cloudApiUrl": "https://cloud.example.test",
            "allocationId": "allocation-peer-attempt-two",
            "token": format!("mboot_{}", "e".repeat(40)),
            "expiresAt": (test_now + chrono::Duration::hours(1)).to_rfc3339(),
            "runtimeReleaseDigest": fixture.release_digest.clone(),
        });
        write_private_file(
            &worker_config.envelope_path,
            &serde_json::to_vec(&worker_envelope).expect("encode worker envelope"),
        )
        .expect("write worker envelope");

        let exchange_response = {
            let mut response = response();
            response.allocation_id = "allocation-peer-attempt-two".to_string();
            response.kernel_id = "worker-kernel".to_string();
            response.runtime_release_digest = fixture.release_digest.clone();
            response.home_caller = CloudHomeCaller {
                account_id: "account-1".to_string(),
                user_id: "local".to_string(),
                realm_id: "default".to_string(),
                machine_id: "home-machine".to_string(),
                kernel_id: "home-kernel".to_string(),
                relay_public_key: home_public_key.clone(),
            };
            response.cloud_relay.api_url = "https://cloud.example.test".to_string();
            response.cloud_relay.account_id = "account-1".to_string();
            response.cloud_relay.user_id = "local".to_string();
            response.cloud_relay.realm_id = "default".to_string();
            response.cloud_relay.machine_id = "worker-machine".to_string();
            response
        };
        let worker_cloud = EnrollmentCloud {
            response: exchange_response,
            confirmations: std::sync::Mutex::new(Vec::new()),
        };
        let mut prepared = prepare(&worker_config, &worker_cloud, test_now)
            .expect("worker enrollment should create a receipt");
        assert_eq!(prepared.receipt.status, WorkerReceiptStatus::Exchanged);
        assert!(prepared.pending.is_some());

        let local_auth_path = fixture.config.chariox_home.join("worker-local-auth-token");
        write_private_file(&local_auth_path, b"test-local-auth")
            .expect("write synthetic worker auth fixture");
        env::set_var("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE", &local_auth_path);
        run_once(&worker_config, &mut prepared, &worker_cloud)
            .expect("normal worker bootstrap should confirm the receipt");
        assert_eq!(prepared.receipt.status, WorkerReceiptStatus::Confirmed);
        assert!(prepared.pending.is_none());

        worker_app_config.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
        worker_app_config.accept_remote_leases = true;
        worker_app_config.remote_lease_capacity = Some(1);
        worker_app_config.cloud_relay = Some(persisted_profile(
            worker_cloud.response.cloud_relay.clone(),
        ));
        worker_app_config.lease_worker_home_caller = Some(prepared.receipt.home_caller.lease_binding());
        env::set_var(ACTIVITY_RECEIPT_ENV, &worker_config.receipt_path);
        env::set_var("CHARIOX_KERNEL_RUNTIME_ROLE", "remote_lease_worker");
        env::set_var("CHARIOX_ACCEPT_REMOTE_LEASES", "1");
        env::set_var("CHARIOX_REMOTE_LEASE_CAPACITY", "1");
        env::set_var(
            "CHARIOX_LEASE_WORKER_HOME_CALLER",
            serde_json::to_string(&prepared.receipt.home_caller.lease_binding())
                .expect("encode worker home caller"),
        );
        let worker_app = Arc::new(tokio::sync::Mutex::new(
            DaemonApp::bootstrap(worker_app_config.clone()).expect("worker app should bootstrap"),
        ));
        let worker_router = Arc::new(
            crate::runtime::router::CommandRouter::with_interactive_capacity(
                Arc::clone(&worker_app),
                1,
            ),
        );
        let worker_state = worker_app.lock().await.relay_client_state();
        let (outgoing_tx, _priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(1);
        let from_daemon_id =
            "home-kernel:peer-tmp:daemon-peer-tmp-4242-1767225600123-7";
        let caller_identity = chariox_relay::protocol::RelayCallerIdentity {
            realm_id: "default".to_string(),
            subject: "home-machine".to_string(),
            subject_kind: chariox_relay::auth::RelaySubjectKind::Machine,
            expires_at_ms: u64::MAX,
            token_id: Some("peer-token".to_string()),
            user_id: Some("local".to_string()),
            public_key_thumbprint: Some(
                crate::runtime::terminal_pairings::public_key_thumbprint(&home_public_key),
            ),
        };

        let lease = match send_authenticated_peer_request_for_test(
            &worker_router,
            &worker_state,
            &outgoing_tx,
            from_daemon_id,
            caller_identity.clone(),
            &home_private_key,
            &worker_public_key,
            RelayPeerRequest::CreateExecutionLease {
                home_kernel_id: "home-kernel".to_string(),
                home_session_id: "home-session".to_string(),
                home_agent_id: "home-agent".to_string(),
                home_agent_metaagent: false,
                owner_user_id: "local".to_string(),
            },
        )
        .await
        .expect("authenticated home should create a worker lease")
        {
            RelayPeerResponse::ExecutionLeaseCreated { lease, .. } => lease,
            other => panic!("unexpected lease response: {other:?}"),
        };
        let leased_agent_id = match send_authenticated_peer_request_for_test(
            &worker_router,
            &worker_state,
            &outgoing_tx,
            from_daemon_id,
            caller_identity.clone(),
            &home_private_key,
            &worker_public_key,
            RelayPeerRequest::SpawnLeasedAgent {
                lease_id: lease.id.clone(),
                provider: "managed-dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                workspace_live_sync_mode: None,
                worktree_id: Some(worker_worktree.display().to_string()),
                worktree_placement: None,
            },
        )
        .await
        .expect("authenticated home should spawn the worker agent")
        {
            RelayPeerResponse::LeasedAgentSpawned { leased_agent } => leased_agent.id,
            other => panic!("unexpected leased-agent response: {other:?}"),
        };
        let target_platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let operation_id = "peer-setup-attempt-two".to_string();
        let start = |attempt: u32, project_id: &str| {
            RelayPeerRequest::StartLeasedProjectEnvironmentSetup {
                leased_agent_id: leased_agent_id.clone(),
                operation_id: operation_id.clone(),
                attempt,
                project_id: project_id.to_string(),
                home_session_id: "home-session".to_string(),
                home_agent_id: "home-agent".to_string(),
                workspace_id: worker_worktree.display().to_string(),
                target_worker_id: "worker-machine".to_string(),
                target_platform: target_platform.clone(),
                definition: None,
                validation_commands: Vec::new(),
            }
        };

        let started = send_authenticated_peer_request_for_test(
            &worker_router,
            &worker_state,
            &outgoing_tx,
            from_daemon_id,
            caller_identity.clone(),
            &home_private_key,
            &worker_public_key,
            start(2, "project-attempt-two"),
        )
        .await
        .expect("worker peer handler should accept home attempt two");
        let started_status = match started {
            RelayPeerResponse::LeasedProjectEnvironmentSetupStarted { setup } => setup,
            other => panic!("unexpected setup start response: {other:?}"),
        };
        assert_eq!(started_status.status.operation_id, operation_id);
        assert_eq!(started_status.status.attempt, 2);

        let observed = send_authenticated_peer_request_for_test(
            &worker_router,
            &worker_state,
            &outgoing_tx,
            from_daemon_id,
            caller_identity.clone(),
            &home_private_key,
            &worker_public_key,
            RelayPeerRequest::GetLeasedProjectEnvironmentSetupStatus {
                leased_agent_id: leased_agent_id.clone(),
                operation_id: operation_id.clone(),
                home_session_id: "home-session".to_string(),
                home_agent_id: "home-agent".to_string(),
            },
        )
        .await
        .expect("worker peer handler should return authenticated attempt two status");
        match observed {
            RelayPeerResponse::LeasedProjectEnvironmentSetupStatus { setup } => {
                assert_eq!(setup.status.operation_id, operation_id);
                assert_eq!(setup.status.attempt, 2);
            }
            other => panic!("unexpected setup status response: {other:?}"),
        }

        let legacy_definition = crate::local::ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: crate::local::ProjectEnvironmentDefinitionOrigin::UserAuthored,
            source: crate::local::ProjectEnvironmentDefinitionSource::Devcontainer,
            target_platform: target_platform.clone(),
            source_path: Some(".devcontainer/devcontainer.json".to_string()),
            inputs: Vec::new(),
            path_entries: Vec::new(),
            setup_steps: vec![crate::local::ProjectEnvironmentSetupStep {
                kind: crate::local::ProjectEnvironmentSetupStepKind::Command,
                command: "command -v sh".to_string(),
            }],
            validation_commands: vec!["command -v sh".to_string()],
        };
        assert!(legacy_definition.is_unattested_file_backed());
        let repaired_admission = send_authenticated_peer_request_for_test(
            &worker_router,
            &worker_state,
            &outgoing_tx,
            from_daemon_id,
            caller_identity.clone(),
            &home_private_key,
            &worker_public_key,
            RelayPeerRequest::StartLeasedProjectEnvironmentSetup {
                leased_agent_id: leased_agent_id.clone(),
                operation_id: "peer-legacy-definition".to_string(),
                attempt: 1,
                project_id: "project-legacy-definition".to_string(),
                home_session_id: "home-session".to_string(),
                home_agent_id: "home-agent".to_string(),
                workspace_id: worker_worktree.display().to_string(),
                target_worker_id: "worker-machine".to_string(),
                target_platform: target_platform.clone(),
                definition: Some(legacy_definition),
                validation_commands: Vec::new(),
            },
        )
        .await
        .expect("home setup with a repairable legacy definition should reach the worker");
        match repaired_admission {
            RelayPeerResponse::LeasedProjectEnvironmentSetupStarted { setup } => {
                assert_eq!(setup.status.operation_id, "peer-legacy-definition");
                assert_eq!(setup.status.attempt, 1);
            }
            other => panic!("unexpected repairable setup response: {other:?}"),
        }

        for (label, request) in [
            ("stale attempt", start(1, "project-attempt-two")),
            ("zero attempt", start(0, "project-attempt-two")),
            ("conflicting fingerprint", start(2, "different-project")),
        ] {
            let error = send_authenticated_peer_request_for_test(
                &worker_router,
                &worker_state,
                &outgoing_tx,
                from_daemon_id,
                caller_identity.clone(),
                &home_private_key,
                &worker_public_key,
                request,
            )
            .await
            .expect_err("invalid worker replay must be rejected");
            assert_eq!(
                error.code,
                crate::transport::relay_peer::PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE,
                "{label} must use the setup rejection boundary"
            );
        }

        let _ = send_authenticated_peer_request_for_test(
            &worker_router,
            &worker_state,
            &outgoing_tx,
            from_daemon_id,
            caller_identity.clone(),
            &home_private_key,
            &worker_public_key,
            RelayPeerRequest::DestroyLeasedAgent {
                leased_agent_id: leased_agent_id.clone(),
            },
        )
        .await
        .expect("worker agent cleanup should remain authenticated");
        let _ = send_authenticated_peer_request_for_test(
            &worker_router,
            &worker_state,
            &outgoing_tx,
            from_daemon_id,
            caller_identity,
            &home_private_key,
            &worker_public_key,
            RelayPeerRequest::DestroyExecutionLease { lease_id: lease.id },
        )
        .await
        .expect("worker lease cleanup should remain authenticated");

        restore_worker_test_env("CHARIOX_HOME", previous_chariox_home);
        restore_worker_test_env(ACTIVITY_RECEIPT_ENV, previous_receipt);
        restore_worker_test_env("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE", previous_local_auth);
        restore_worker_test_env("CHARIOX_KERNEL_STARTED_MARKER", previous_started_marker);
        env::remove_var("CHARIOX_KERNEL_RUNTIME_ROLE");
        env::remove_var("CHARIOX_ACCEPT_REMOTE_LEASES");
        env::remove_var("CHARIOX_REMOTE_LEASE_CAPACITY");
        env::remove_var("CHARIOX_LEASE_WORKER_HOME_CALLER");
        fixture.cleanup();
    }

    struct EnrollmentCloud {
        response: ExchangeResponse,
        confirmations: std::sync::Mutex<Vec<ConfirmRequest>>,
    }

    impl WorkerCloudClient for EnrollmentCloud {
        fn exchange(
            &self,
            _api_url: &str,
            request: &ExchangeRequest,
        ) -> Result<ExchangeResponse, DaemonError> {
            assert_eq!(request.allocation_id, self.response.allocation_id);
            assert_eq!(
                request.runtime_release_digest,
                self.response.runtime_release_digest
            );
            Ok(self.response.clone())
        }

        fn confirm(
            &self,
            _api_url: &str,
            request: &ConfirmRequest,
        ) -> Result<ConfirmResponse, DaemonError> {
            self.confirmations
                .lock()
                .expect("confirmation lock")
                .push(request.clone());
            Ok(ConfirmResponse {
                confirmed: true,
                observed_state: "ready".to_string(),
            })
        }
    }

    struct TestCloudApi {
        url: String,
        ticket: std::sync::Arc<std::sync::Mutex<Option<ManagedContextTransferTicket>>>,
        requests: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
        shutdown: Option<std::sync::mpsc::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TestCloudApi {
        fn bind() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("Cloud fixture should bind");
            listener
                .set_nonblocking(true)
                .expect("Cloud fixture should be nonblocking");
            let address = listener.local_addr().expect("Cloud fixture address");
            let ticket = std::sync::Arc::new(std::sync::Mutex::new(None));
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let thread_ticket = std::sync::Arc::clone(&ticket);
            let thread_requests = std::sync::Arc::clone(&requests);
            let (shutdown, shutdown_rx) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                while shutdown_rx.try_recv().is_err() {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if let Some((path, body)) = read_worker_http_request(&mut stream) {
                                thread_requests
                                    .lock()
                                    .expect("Cloud request lock")
                                    .push((path.clone(), body.clone()));
                                let (status, response) = if path.ends_with("/context/ticket") {
                                    match thread_ticket.lock().expect("Cloud ticket lock").clone() {
                                        Some(ticket) => (
                                            "200 OK",
                                            serde_json::to_vec(&ticket)
                                                .expect("encode Cloud ticket"),
                                        ),
                                        None => ("503 Service Unavailable", b"{}".to_vec()),
                                    }
                                } else if path.ends_with("/context/complete") {
                                    let digest = body
                                        .get("contextManifestDigest")
                                        .cloned()
                                        .unwrap_or(serde_json::Value::Null);
                                    (
                                        "200 OK",
                                        serde_json::to_vec(&serde_json::json!({
                                            "ready": true,
                                            "observedState": "ready",
                                            "contextManifestDigest": digest,
                                        }))
                                        .expect("encode Cloud completion"),
                                    )
                                } else {
                                    ("404 Not Found", b"{}".to_vec())
                                };
                                let header = format!(
                                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                    response.len()
                                );
                                let _ = stream.write_all(header.as_bytes());
                                let _ = stream.write_all(&response);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                url: format!("http://{address}"),
                ticket,
                requests,
                shutdown: Some(shutdown),
                thread: Some(thread),
            }
        }

        fn set_ticket(&self, ticket: ManagedContextTransferTicket) {
            *self.ticket.lock().expect("Cloud ticket lock") = Some(ticket);
        }

        fn stop(mut self) -> Vec<(String, serde_json::Value)> {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(thread) = self.thread.take() {
                thread.join().expect("Cloud fixture should stop");
            }
            self.requests.lock().expect("Cloud request lock").clone()
        }
    }

    fn read_worker_http_request(stream: &mut TcpStream) -> Option<(String, serde_json::Value)> {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("Cloud fixture timeout");
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                return None;
            }
            request.extend_from_slice(&chunk[..read]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })?;
            if request.len() < header_end + 4 + content_length {
                continue;
            }
            let path = headers
                .lines()
                .next()?
                .split_whitespace()
                .nth(1)?
                .to_string();
            let body =
                serde_json::from_slice(&request[header_end + 4..header_end + 4 + content_length])
                    .ok()?;
            return Some((path, body));
        }
    }

    fn isolated_worker_test_config(
        root: &std::path::Path,
        daemon_id: &str,
        machine_id: &str,
        alias: &str,
        websocket_port: u16,
    ) -> DaemonConfig {
        let mut config = DaemonConfig::for_tests();
        config.user_config_path = root.join("config.toml");
        config.daemon_id = daemon_id.to_string();
        config.host_machine_id = machine_id.to_string();
        config.daemon_alias = Some(alias.to_string());
        config.local_socket_path = root.join("run/kernel.sock");
        config = config.with_session_history_root(root.join("sessions"));
        config.kernel_websocket_host = "127.0.0.1".to_string();
        config.kernel_websocket_port = websocket_port;
        config.runtime_mcp_host = "127.0.0.1".to_string();
        config.runtime_mcp_port = websocket_port.saturating_add(1);
        config.user_config.history.operational.path =
            Some(root.join("history.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        config.user_config.state.path = Some(root.join("state/state.db").display().to_string());
        config.user_config.credential_vault.path = root.join("vault.json").display().to_string();
        config
    }

    fn init_worker_test_identity_with_key(
        home: &std::path::Path,
        daemon_id: &str,
        machine_id: &str,
        private_key: &str,
        public_key: &str,
    ) {
        write_private_file(
            &home.join("machine/identity.json"),
            &serde_json::to_vec(&serde_json::json!({
                "machine_id": machine_id,
                "machine_alias": "test-worker",
            }))
            .expect("encode machine identity"),
        )
        .expect("write machine identity");
        write_private_file(
            &home.join("state/daemon/identity.json"),
            &serde_json::to_vec(&serde_json::json!({
                "daemon_id": daemon_id,
                "machine_id": machine_id,
                "machine_alias": "test-worker",
                "daemon_alias": "disposable-worker",
                "relay_public_key": public_key,
                "relay_private_key": private_key,
            }))
            .expect("encode worker runtime identity"),
        )
        .expect("write worker runtime identity");
    }

    fn test_cloud_profile(
        api_url: String,
        machine_id: &str,
        machine_credential: &str,
        realm_id: &str,
    ) -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            api_url,
            email: "owner@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "local".to_string(),
            account_slug: "account-1".to_string(),
            realm_id: realm_id.to_string(),
            relay_url: "wss://relay.example.test".to_string(),
            issuer_id: "issuer-test".to_string(),
            client_id: None,
            client_alias: None,
            machine_id: Some(machine_id.to_string()),
            machine_alias: Some(machine_id.to_string()),
            machine_credential: Some(machine_credential.to_string()),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        }
    }

    fn test_relay_auth(
        home: (&str, &str, &str, &str),
        worker: (&str, &str, &str, &str),
    ) -> RelayAuthVerifier {
        let claims = [home, worker]
            .into_iter()
            .map(|(token, subject, machine_id, public_key)| {
                (
                    token.to_string(),
                    RelayTokenClaims {
                        issuer: "disposable-worker-test".to_string(),
                        subject: subject.to_string(),
                        subject_kind: chariox_relay::RelaySubjectKind::Kernel,
                        realm_id: "default".to_string(),
                        allowed_actions: vec![
                            RelayAction::DaemonRegister,
                            RelayAction::DaemonHeartbeat,
                            RelayAction::PeerRequest,
                            RelayAction::PeerEvent,
                            RelayAction::ClientMetadataRead,
                            RelayAction::PacketRoute,
                        ],
                        allowed_targets: None,
                        issued_at_ms: 1,
                        expires_at_ms: u64::MAX,
                        token_id: token.to_string(),
                        account_id: Some("account-1".to_string()),
                        organization_id: None,
                        user_id: Some("local".to_string()),
                        device_id: None,
                        machine_id: Some(machine_id.to_string()),
                        client_id: None,
                        session_id: None,
                        public_key_thumbprint: Some(
                            crate::runtime::terminal_pairings::public_key_thumbprint(public_key),
                        ),
                        entitlements_version: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), None))
    }

    async fn dispatch_public(
        router: &crate::runtime::router::CommandRouter,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let command = KernelCommand::from_local_request(
            format!("disposable-project-{}", rand::random::<u64>()),
            None,
            None,
            &request,
        );
        router.dispatch(command, request).await
    }

    fn restore_worker_test_env(name: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => env::set_var(name, value),
            None => env::remove_var(name),
        }
    }

    fn init_test_repository(root: &std::path::Path, filename: &str, contents: &str) {
        fs::create_dir_all(root).expect("test repository should exist");
        run_git(root, &["init", "-b", "main"]);
        run_git(root, &["config", "user.email", "chariox@example.test"]);
        run_git(root, &["config", "user.name", "Chariox Test"]);
        fs::write(root.join(filename), contents).expect("test repository file should exist");
        run_git(root, &["add", filename]);
        run_git(root, &["commit", "-m", "selected source"]);
    }

    fn run_git(root: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git should be available for the materialization regression");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

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
        assert!(confirmed_activity_allocation(&path, &runtime, &profile).is_err());
        let mut confirmed = receipt.clone();
        confirmed.status = WorkerReceiptStatus::Confirmed;
        confirmed.confirmed_at = Some("2026-09-13T11:00:00Z".to_string());
        confirmed.persist(&path).unwrap();
        assert_eq!(
            confirmed_activity_allocation(&path, &runtime, &profile).unwrap(),
            "worker-1"
        );
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
