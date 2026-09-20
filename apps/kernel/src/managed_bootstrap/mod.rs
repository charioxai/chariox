mod cloud;
mod context_plan;
mod release;
mod state;
mod supervisor;
pub mod worker;

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rand::Rng;

use crate::config::{
    load_managed_cloud_relay_profile, load_or_create_managed_runtime_identity,
    persist_managed_cloud_relay_profile, ManagedRuntimeIdentity, PersistedCloudRelayProfile,
};
use crate::error::DaemonError;
use crate::managed_context::development::{
    normalize_managed_repository_root, DEFAULT_MANAGED_REPOSITORY_ROOT,
};

use cloud::{
    BootstrapCloudClient, ConfirmRequest, ExchangeRequest, HttpBootstrapCloudClient,
    ManagedCloudRelayProfile,
};
pub use context_plan::ManagedKernelContextPlan;
use release::{verify_release, VerifiedRelease};
use state::{
    default_managed_bootstrap_receipt_path, remove_envelope, valid_identifier, valid_secret,
    BootstrapConfig, BootstrapEnvelope, BootstrapReceipt, BootstrapReceiptDocument,
    BootstrapReceiptStatus, ManagedBootstrapEnvelope,
};

const MIN_PREPARE_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_PREPARE_RETRY_DELAY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfirmedManagedKernelRegistration {
    pub environment_id: String,
    pub machine_id: String,
    pub kernel_id: String,
    pub context_plan: Option<ManagedKernelContextPlan>,
}

#[derive(Debug)]
struct PreparedManagedKernel {
    release: VerifiedRelease,
    confirmation: Option<PendingConfirmation>,
}

#[derive(Debug)]
struct PendingConfirmation {
    envelope: ManagedBootstrapEnvelope,
    receipt: BootstrapReceipt,
    profile: PersistedCloudRelayProfile,
}

pub fn run_from_env() -> Result<(), DaemonError> {
    supervisor::initialize_managed_docker_broker();
    let cloud = HttpBootstrapCloudClient::default();
    let mut retry_delay = MIN_PREPARE_RETRY_DELAY;
    loop {
        let prepared = BootstrapConfig::from_env().and_then(|config| {
            prepare_managed_kernel(&config, &cloud, Utc::now()).map(|prepared| (config, prepared))
        });
        match prepared {
            Ok((config, prepared)) => {
                supervisor::supervise_kernel(
                    &config,
                    &prepared.release,
                    prepared.confirmation,
                    &cloud,
                )?;
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "managed_bootstrap.prepare_failed",
                    "managed kernel bootstrap preparation failed; supervisor will retry",
                    serde_json::json!({
                        "error": error.to_string(),
                        "retry_delay_ms": retry_delay.as_millis(),
                    }),
                );
            }
        }
        thread::sleep(jittered(retry_delay));
        retry_delay = retry_delay.saturating_mul(2).min(MAX_PREPARE_RETRY_DELAY);
    }
}

pub(crate) fn confirmed_managed_kernel_registration_from_env(
) -> Result<Option<ConfirmedManagedKernelRegistration>, DaemonError> {
    let Some(_) = std::env::var_os("CHARIOX_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
    else {
        return Ok(None);
    };
    let receipt_path = std::env::var_os("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_managed_bootstrap_receipt_path);
    if !receipt_path.exists() {
        return Ok(None);
    }
    let config = BootstrapConfig::from_env()?;
    let Some(receipt) = BootstrapReceiptDocument::read(&config.receipt_path)? else {
        return Ok(None);
    };
    let BootstrapReceiptDocument::ManagedEnvironment(receipt) = receipt else {
        return Ok(None);
    };
    if receipt.status != BootstrapReceiptStatus::Confirmed {
        return Ok(None);
    }
    Ok(Some(ConfirmedManagedKernelRegistration {
        environment_id: receipt.environment_id,
        machine_id: receipt.machine_id,
        kernel_id: receipt.kernel_id,
        context_plan: receipt.context_plan,
    }))
}

fn prepare_managed_kernel(
    config: &BootstrapConfig,
    cloud: &impl BootstrapCloudClient,
    now: DateTime<Utc>,
) -> Result<PreparedManagedKernel, DaemonError> {
    // Legacy worker records are readable only for an explicit rejection. They
    // must never create an ordinary machine identity or consume a Cloud grant.
    let receipt = match BootstrapReceiptDocument::read(&config.receipt_path)? {
        Some(BootstrapReceiptDocument::ManagedEnvironment(value)) => Some(value),
        Some(BootstrapReceiptDocument::DisposableWorker(_)) => return Err(legacy_worker_error()),
        None => None,
    };
    let envelope = if config.envelope_path.exists() {
        match BootstrapEnvelope::read(&config.envelope_path)? {
            BootstrapEnvelope::ManagedEnvironment(value) => Some(value),
            BootstrapEnvelope::DisposableWorker(_) => return Err(legacy_worker_error()),
        }
    } else {
        None
    };
    if state::disposable_worker_release_override_path(&config.receipt_path)?.exists() {
        return Err(bootstrap_error(
            "disposable worker release override has no matching worker receipt",
        ));
    }
    let expected_digest = receipt
        .as_ref()
        .map(|value| value.runtime_release_digest.as_str())
        .or_else(|| {
            envelope
                .as_ref()
                .map(|value| value.runtime_release_digest.as_str())
        })
        .ok_or_else(|| {
            bootstrap_error("managed bootstrap envelope and receipt are both missing")
        })?;
    let release = verify_release(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        expected_digest,
        &config.kernel_binary,
    )?;
    let identity =
        load_or_create_managed_runtime_identity(&config.kernel_host, config.kernel_port)?;
    let confirmation = match (receipt, envelope) {
        (Some(receipt), envelope) => {
            if let Some(value) = envelope.as_ref() {
                if value.environment_id != receipt.environment_id
                    || value.runtime_release_digest != receipt.runtime_release_digest
                    || value.managed_repository_root()? != receipt.managed_repository_root()?
                {
                    return Err(bootstrap_error(
                        "managed bootstrap envelope conflicts with its receipt",
                    ));
                }
            }
            resume_registration(config, envelope.as_ref(), receipt, &identity)?
        }
        (None, Some(envelope)) => begin_registration(config, cloud, now, &envelope, &identity)?,
        (None, None) => {
            return Err(bootstrap_error(
                "managed bootstrap envelope and receipt are both missing",
            ))
        }
    };
    Ok(PreparedManagedKernel {
        release,
        confirmation,
    })
}

fn legacy_worker_error() -> DaemonError {
    bootstrap_error("legacy disposable worker bootstrap is unsupported; preserve the records and reprovision through the allocation-worker service")
}

fn begin_registration(
    config: &BootstrapConfig,
    cloud: &impl BootstrapCloudClient,
    now: DateTime<Utc>,
    envelope: &ManagedBootstrapEnvelope,
    identity: &ManagedRuntimeIdentity,
) -> Result<Option<PendingConfirmation>, DaemonError> {
    if envelope.expires_at()? <= now {
        return Err(bootstrap_error(
            "managed bootstrap token expired before exchange",
        ));
    }
    let exchanged = cloud.exchange(
        &normalized_api_url(&envelope.cloud_api_url),
        &ExchangeRequest {
            token: envelope.token.clone(),
            environment_id: envelope.environment_id.clone(),
            machine_id: identity.machine_id.clone(),
            kernel_id: identity.kernel_id.clone(),
            relay_public_key: identity.relay_public_key.clone(),
            runtime_release_digest: envelope.runtime_release_digest.clone(),
        },
    )?;
    let managed_repository_root = validate_exchange_response(envelope, identity, &exchanged)?;
    let profile = persisted_profile(exchanged.cloud_relay);
    persist_managed_cloud_relay_profile(profile.clone())?;
    let receipt = BootstrapReceipt {
        schema_version: 2,
        status: BootstrapReceiptStatus::Exchanged,
        environment_id: envelope.environment_id.clone(),
        machine_id: identity.machine_id.clone(),
        kernel_id: identity.kernel_id.clone(),
        relay_public_key: identity.relay_public_key.clone(),
        runtime_release_digest: envelope.runtime_release_digest.clone(),
        confirmed_at: None,
        managed_repository_root: Some(managed_repository_root.display().to_string()),
        context_plan: Some(exchanged.context_plan),
    };
    receipt.persist(&config.receipt_path)?;
    Ok(Some(PendingConfirmation {
        envelope: envelope.clone(),
        receipt,
        profile,
    }))
}

fn resume_registration(
    config: &BootstrapConfig,
    envelope: Option<&ManagedBootstrapEnvelope>,
    receipt: BootstrapReceipt,
    identity: &ManagedRuntimeIdentity,
) -> Result<Option<PendingConfirmation>, DaemonError> {
    validate_receipt_identity(&receipt, identity)?;
    let profile = load_managed_cloud_relay_profile()
        .ok_or_else(|| bootstrap_error("managed Cloud profile is missing after exchange"))?;
    validate_profile(&profile, &receipt)?;
    match receipt.status {
        BootstrapReceiptStatus::Confirmed => {
            if config.envelope_path.exists() {
                remove_envelope(&config.envelope_path)?;
            }
            Ok(None)
        }
        BootstrapReceiptStatus::Exchanged => {
            let envelope = envelope.ok_or_else(|| {
                bootstrap_error("managed bootstrap envelope is required to resume confirmation")
            })?;
            Ok(Some(PendingConfirmation {
                envelope: envelope.clone(),
                receipt,
                profile,
            }))
        }
    }
}

impl PendingConfirmation {
    fn confirm(
        &self,
        config: &BootstrapConfig,
        cloud: &impl BootstrapCloudClient,
        now: DateTime<Utc>,
    ) -> Result<(), DaemonError> {
        confirm_registration(
            config,
            cloud,
            now,
            &self.envelope,
            self.receipt.clone(),
            &self.profile,
        )
    }
}

fn confirm_registration(
    config: &BootstrapConfig,
    cloud: &impl BootstrapCloudClient,
    now: DateTime<Utc>,
    envelope: &ManagedBootstrapEnvelope,
    mut receipt: BootstrapReceipt,
    profile: &PersistedCloudRelayProfile,
) -> Result<(), DaemonError> {
    let confirmed = cloud.confirm(
        &normalized_api_url(&envelope.cloud_api_url),
        &ConfirmRequest {
            token: envelope.token.clone(),
            environment_id: receipt.environment_id.clone(),
            machine_id: receipt.machine_id.clone(),
            machine_credential: profile
                .machine_credential
                .clone()
                .ok_or_else(|| bootstrap_error("managed machine credential is missing"))?,
        },
    )?;
    let expected_root = envelope.managed_repository_root()?;
    let confirmed_root = response_managed_repository_root(
        envelope.schema_version,
        confirmed.managed_repository_root.as_deref(),
    )?;
    if !confirmed.confirmed
        || confirmed.observed_state != "awaiting_context"
        || confirmed_root != expected_root
    {
        return Err(bootstrap_error(
            "Cloud did not confirm the managed kernel bootstrap",
        ));
    }
    receipt.status = BootstrapReceiptStatus::Confirmed;
    receipt.confirmed_at = Some(now.to_rfc3339());
    receipt.persist(&config.receipt_path)?;
    remove_envelope(&config.envelope_path)
}

fn validate_exchange_response(
    envelope: &ManagedBootstrapEnvelope,
    identity: &ManagedRuntimeIdentity,
    response: &cloud::ExchangeResponse,
) -> Result<PathBuf, DaemonError> {
    let expected_root = envelope.managed_repository_root()?;
    let response_root = response_managed_repository_root(
        envelope.schema_version,
        response.managed_repository_root.as_deref(),
    )?;
    if response_root != expected_root {
        return Err(bootstrap_error(
            "Cloud bootstrap response managed repository root does not match the envelope",
        ));
    }
    if response.environment_id != envelope.environment_id
        || response.kernel_id != identity.kernel_id
        || response.runtime_release_digest != envelope.runtime_release_digest
        || response.cloud_relay.machine_id != identity.machine_id
        || normalized_api_url(&response.cloud_relay.api_url)
            != normalized_api_url(&envelope.cloud_api_url)
        || !valid_identifier(&response.cloud_relay.account_id)
        || !valid_identifier(&response.cloud_relay.user_id)
        || !valid_identifier(&response.cloud_relay.account_slug)
        || !valid_identifier(&response.cloud_relay.realm_id)
        || !valid_identifier(&response.cloud_relay.issuer_id)
        || response.cloud_relay.email.trim().is_empty()
        || response.cloud_relay.email.len() > 320
        || response.cloud_relay.machine_alias.trim().is_empty()
        || response.cloud_relay.machine_alias.len() > 256
        || !valid_managed_relay_url(&response.cloud_relay.relay_url)
        || !valid_secret(&response.cloud_relay.machine_credential, "mcred_")
        || response.context_plan.validate().is_err()
        || response
            .context_plan
            .source_binding()
            .is_some_and(|source| source.relay_realm_id != response.cloud_relay.realm_id)
    {
        return Err(bootstrap_error(
            "Cloud bootstrap response does not match the local identity",
        ));
    }
    Ok(expected_root)
}

fn response_managed_repository_root(
    envelope_schema_version: u32,
    value: Option<&str>,
) -> Result<PathBuf, DaemonError> {
    if envelope_schema_version == 2 && value.is_none() {
        return Err(bootstrap_error(
            "Cloud bootstrap response is missing managedRepositoryRoot",
        ));
    }
    let root = normalize_managed_repository_root(value)
        .map_err(|_| bootstrap_error("Cloud bootstrap response has an invalid repository root"))?;
    if envelope_schema_version == 1
        && value.is_some()
        && root != Path::new(DEFAULT_MANAGED_REPOSITORY_ROOT)
    {
        return Err(bootstrap_error(
            "legacy Cloud bootstrap response may only use the default repository root",
        ));
    }
    Ok(root)
}

fn validate_receipt_identity(
    receipt: &BootstrapReceipt,
    identity: &ManagedRuntimeIdentity,
) -> Result<(), DaemonError> {
    if receipt.machine_id != identity.machine_id
        || receipt.kernel_id != identity.kernel_id
        || receipt.relay_public_key != identity.relay_public_key
    {
        return Err(bootstrap_error(
            "managed bootstrap receipt does not match the local identity",
        ));
    }
    Ok(())
}

fn validate_profile(
    profile: &PersistedCloudRelayProfile,
    receipt: &BootstrapReceipt,
) -> Result<(), DaemonError> {
    if profile.machine_id.as_deref() != Some(receipt.machine_id.as_str())
        || profile
            .machine_credential
            .as_deref()
            .is_none_or(|value| !valid_secret(value, "mcred_"))
        || !valid_managed_relay_url(&profile.relay_url)
        || receipt
            .context_plan
            .as_ref()
            .and_then(ManagedKernelContextPlan::source_binding)
            .is_some_and(|source| source.relay_realm_id != profile.realm_id)
    {
        return Err(bootstrap_error(
            "managed Cloud profile does not match its receipt",
        ));
    }
    Ok(())
}

fn persisted_profile(profile: ManagedCloudRelayProfile) -> PersistedCloudRelayProfile {
    PersistedCloudRelayProfile {
        api_url: profile.api_url,
        email: profile.email,
        account_id: profile.account_id,
        user_id: profile.user_id,
        account_slug: profile.account_slug,
        realm_id: profile.realm_id,
        relay_url: profile.relay_url,
        issuer_id: profile.issuer_id,
        client_id: None,
        client_alias: None,
        machine_id: Some(profile.machine_id),
        machine_alias: Some(profile.machine_alias),
        machine_credential: Some(profile.machine_credential),
        cloud_session_token: None,
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: None,
    }
}

fn normalized_api_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn valid_managed_relay_url(value: &str) -> bool {
    url::Url::parse(value).ok().is_some_and(|url| {
        let safe_components = url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none();
        safe_components
            && ((url.scheme() == "wss" && url.host_str().is_some())
                || (url.scheme() == "ws"
                    && url
                        .host_str()
                        .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"))))
    })
}

fn bootstrap_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "bootstrap managed kernel",
        message: message.to_string(),
    }
}

fn jittered(delay: Duration) -> Duration {
    let millis = delay.as_millis().min(u64::MAX as u128) as u64;
    let minimum = millis.saturating_mul(4) / 5;
    let maximum = millis.saturating_mul(6) / 5;
    Duration::from_millis(rand::thread_rng().gen_range(minimum..=maximum.max(minimum)))
}

#[cfg(test)]
mod tests;
