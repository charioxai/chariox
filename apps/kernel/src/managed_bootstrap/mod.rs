mod cloud;
mod context_plan;
mod release;
mod state;
mod supervisor;

use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rand::Rng;

use crate::config::{
    load_managed_cloud_relay_profile, load_or_create_managed_runtime_identity,
    load_or_initialize_disposable_worker_identity, persist_managed_cloud_relay_profile,
    ManagedRuntimeIdentity, PersistedCloudRelayProfile,
};
use crate::error::DaemonError;

use cloud::{
    BootstrapCloudClient, ConfirmRequest, DisposableWorkerBootstrapResult,
    DisposableWorkerExchangeOutcome, DisposableWorkerExchangeRequest,
    DisposableWorkerRecoveryOutcome, ExchangeRequest, HttpBootstrapCloudClient,
    ManagedCloudRelayProfile,
};
pub use context_plan::ManagedKernelContextPlan;
use release::{verify_release, VerifiedRelease};
use state::{
    remove_envelope, remove_receipt, valid_disposable_identifier, valid_identifier, valid_secret,
    BootstrapConfig, BootstrapEnvelope, BootstrapReceipt, BootstrapReceiptDocument,
    BootstrapReceiptStatus, DisposableWorkerBinding, DisposableWorkerBootstrapEnvelope,
    DisposableWorkerBootstrapReceipt, DisposableWorkerBootstrapReceiptStatus,
    ManagedBootstrapEnvelope,
};

const MAX_DISPOSABLE_WORKER_ENVELOPE_TTL_SECONDS: i64 = 30 * 60;

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
    let Some(chariox_home) = std::env::var_os("CHARIOX_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
    else {
        return Ok(None);
    };
    let receipt_path = std::env::var_os("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| chariox_home.join("managed").join("bootstrap-receipt.json"));
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
    let receipt = BootstrapReceiptDocument::read(&config.receipt_path)?;
    let envelope = if config.envelope_path.exists() {
        match BootstrapEnvelope::read(&config.envelope_path) {
            Ok(value) => Some(value),
            Err(error) => {
                remove_invalid_disposable_worker_envelope(&config.envelope_path)?;
                return Err(error);
            }
        }
    } else {
        None
    };
    let worker_release_override = match receipt.as_ref() {
        Some(BootstrapReceiptDocument::DisposableWorker(value)) => {
            state::DisposableWorkerReleaseOverride::read_for_receipt(
                &config.receipt_path,
                &value.binding_digest,
            )?
        }
        Some(BootstrapReceiptDocument::ManagedEnvironment(_)) | None => {
            let path = state::disposable_worker_release_override_path(&config.receipt_path)?;
            if path.exists() {
                return Err(bootstrap_error(
                    "disposable worker release override has no matching worker receipt",
                ));
            }
            None
        }
    };
    let receipt_digest = receipt.as_ref().map(|value| match value {
        BootstrapReceiptDocument::ManagedEnvironment(value) => {
            value.runtime_release_digest.as_str()
        }
        BootstrapReceiptDocument::DisposableWorker(value) => worker_release_override
            .as_ref()
            .map(|release| release.runtime_release_digest.as_str())
            .unwrap_or(value.binding.runtime_release_digest.as_str()),
    });
    let expected_digest = receipt_digest
        .or_else(|| {
            envelope
                .as_ref()
                .map(BootstrapEnvelope::runtime_release_digest)
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
    let disposable_binding = match (receipt.as_ref(), envelope.as_ref()) {
        (Some(BootstrapReceiptDocument::DisposableWorker(receipt)), _) => {
            Some(&receipt.binding)
        }
        (None, Some(BootstrapEnvelope::DisposableWorker(envelope))) => {
            Some(&envelope.binding)
        }
        _ => None,
    };
    let identity = if let Some(binding) = disposable_binding {
        match load_or_initialize_disposable_worker_identity(
            &config.kernel_host,
            config.kernel_port,
            &binding.worker_machine_id,
            &binding.worker_kernel_id,
        ) {
            Ok(identity) => identity,
            Err(error) => {
                if receipt.is_none()
                    && matches!(
                        envelope.as_ref(),
                        Some(BootstrapEnvelope::DisposableWorker(_))
                    )
                    && config.envelope_path.exists()
                {
                    remove_envelope(&config.envelope_path)?;
                }
                return Err(error);
            }
        }
    } else {
        load_or_create_managed_runtime_identity(&config.kernel_host, config.kernel_port)?
    };

    let confirmation = match (receipt, envelope) {
        (Some(BootstrapReceiptDocument::ManagedEnvironment(receipt)), envelope) => {
            let envelope = match envelope {
                Some(BootstrapEnvelope::ManagedEnvironment(value)) => Some(value),
                Some(BootstrapEnvelope::DisposableWorker(_)) => {
                    return Err(bootstrap_error("bootstrap envelope conflicts with its receipt"));
                }
                None => None,
            };
            if envelope.as_ref().is_some_and(|value| {
                value.environment_id != receipt.environment_id
                    || value.runtime_release_digest != receipt.runtime_release_digest
            }) {
                return Err(bootstrap_error(
                    "managed bootstrap envelope conflicts with its receipt",
                ));
            }
            resume_registration(config, envelope.as_ref(), receipt, &identity)?
        }
        (Some(BootstrapReceiptDocument::DisposableWorker(receipt)), envelope) => {
            let envelope = match envelope {
                Some(BootstrapEnvelope::DisposableWorker(value)) => Some(value),
                Some(BootstrapEnvelope::ManagedEnvironment(_)) => {
                    return Err(bootstrap_error("bootstrap envelope conflicts with its receipt"));
                }
                None => None,
            };
            resume_disposable_worker(config, cloud, now, envelope.as_ref(), &receipt, &identity)?;
            None
        }
        (None, Some(BootstrapEnvelope::ManagedEnvironment(envelope))) => {
            begin_registration(config, cloud, now, &envelope, &identity)?
        }
        (None, Some(BootstrapEnvelope::DisposableWorker(envelope))) => {
            begin_disposable_worker(config, cloud, now, &envelope, &identity)?;
            None
        }
        (None, None) => {
            return Err(bootstrap_error(
                "managed bootstrap envelope and receipt are both missing",
            ));
        }
    };
    Ok(PreparedManagedKernel {
        release,
        confirmation,
    })
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
    validate_exchange_response(envelope, identity, &exchanged)?;
    let profile = persisted_profile(exchanged.cloud_relay);
    persist_managed_cloud_relay_profile(profile.clone())?;
    let receipt = BootstrapReceipt {
        schema_version: 1,
        status: BootstrapReceiptStatus::Exchanged,
        environment_id: envelope.environment_id.clone(),
        machine_id: identity.machine_id.clone(),
        kernel_id: identity.kernel_id.clone(),
        relay_public_key: identity.relay_public_key.clone(),
        runtime_release_digest: envelope.runtime_release_digest.clone(),
        confirmed_at: None,
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

fn begin_disposable_worker(
    config: &BootstrapConfig,
    cloud: &impl BootstrapCloudClient,
    now: DateTime<Utc>,
    envelope: &DisposableWorkerBootstrapEnvelope,
    identity: &ManagedRuntimeIdentity,
) -> Result<(), DaemonError> {
    validate_disposable_worker_expiry(config, now, envelope)?;
    let binding = &envelope.binding;
    if binding.worker_machine_id != identity.machine_id
        || binding.worker_kernel_id != identity.kernel_id
    {
        remove_envelope(&config.envelope_path)?;
        return Err(bootstrap_error(
            "disposable worker bootstrap binding does not match the local identity",
        ));
    }
    let pending_receipt = DisposableWorkerBootstrapReceipt {
        schema_version: 1,
        kind: "disposable_worker".to_string(),
        status: DisposableWorkerBootstrapReceiptStatus::ExchangePending,
        cloud_api_url: normalized_api_url(&envelope.cloud_api_url),
        relay_public_key: identity.relay_public_key.clone(),
        binding_digest: envelope.binding_digest.clone(),
        binding: binding.clone(),
        enrollment_receipt: None,
        cloud_relay: None,
    };
    pending_receipt.persist(&config.receipt_path)?;
    exchange_disposable_worker(config, cloud, envelope, pending_receipt)
}

fn resume_disposable_worker(
    config: &BootstrapConfig,
    cloud: &impl BootstrapCloudClient,
    now: DateTime<Utc>,
    envelope: Option<&DisposableWorkerBootstrapEnvelope>,
    receipt: &DisposableWorkerBootstrapReceipt,
    identity: &ManagedRuntimeIdentity,
) -> Result<(), DaemonError> {
    if receipt.binding.worker_machine_id != identity.machine_id
        || receipt.binding.worker_kernel_id != identity.kernel_id
        || receipt.relay_public_key != identity.relay_public_key
    {
        return Err(bootstrap_error(
            "disposable worker receipt does not match the local identity",
        ));
    }
    if let Some(envelope) = envelope {
        if envelope.binding != receipt.binding
            || envelope.binding_digest != receipt.binding_digest
            || normalized_api_url(&envelope.cloud_api_url) != receipt.cloud_api_url
        {
            remove_envelope(&config.envelope_path)?;
            return Err(bootstrap_error(
                "disposable worker envelope conflicts with its receipt",
            ));
        }
    }
    if matches!(
        receipt.status,
        DisposableWorkerBootstrapReceiptStatus::ExchangePending
            | DisposableWorkerBootstrapReceiptStatus::ExchangeAccepted
    ) {
        if receipt.status == DisposableWorkerBootstrapReceiptStatus::ExchangeAccepted
            && config.envelope_path.exists()
        {
            remove_envelope(&config.envelope_path)?;
        }
        return match cloud.recover_disposable_worker_exchange(
            &receipt.cloud_api_url,
            &receipt.binding_digest,
            &receipt.binding,
        )? {
            DisposableWorkerRecoveryOutcome::Ready(result) => {
                finish_disposable_worker(config, receipt.clone(), result)
            }
            DisposableWorkerRecoveryOutcome::Rejected => {
                remove_receipt(&config.receipt_path)?;
                if config.envelope_path.exists() {
                    remove_envelope(&config.envelope_path)?;
                }
                Err(bootstrap_error(
                    "Cloud terminally rejected the disposable worker bootstrap",
                ))
            }
            DisposableWorkerRecoveryOutcome::Pending => match (receipt.status, envelope) {
                (DisposableWorkerBootstrapReceiptStatus::ExchangePending, Some(envelope)) => {
                    validate_disposable_worker_expiry(config, now, envelope)?;
                    exchange_disposable_worker(config, cloud, envelope, receipt.clone())
                }
                _ => Err(bootstrap_error(
                    "disposable worker exchange result is not yet recoverable",
                )),
            },
        };
    }
    validate_disposable_worker_result(receipt)?;
    let receipt_relay = receipt.cloud_relay.clone().ok_or_else(|| {
        bootstrap_error("disposable worker relay receipt is missing")
    })?;
    let expected_profile = persisted_profile(receipt_relay);
    let profile = match load_managed_cloud_relay_profile() {
        Some(profile) => profile,
        None => {
            persist_managed_cloud_relay_profile(expected_profile.clone())?;
            load_managed_cloud_relay_profile().ok_or_else(|| {
                bootstrap_error("disposable worker Cloud profile did not persist")
            })?
        }
    };
    if profile != expected_profile {
        if config.envelope_path.exists() {
            remove_envelope(&config.envelope_path)?;
        }
        return Err(bootstrap_error(
            "disposable worker Cloud profile does not exactly match its receipt",
        ));
    }
    if config.envelope_path.exists() {
        remove_envelope(&config.envelope_path)?;
    }
    Ok(())
}

fn validate_disposable_worker_expiry(
    config: &BootstrapConfig,
    now: DateTime<Utc>,
    envelope: &DisposableWorkerBootstrapEnvelope,
) -> Result<(), DaemonError> {
    let expiry = envelope.expires_at()?;
    if expiry <= now
        || expiry > now + chrono::Duration::seconds(MAX_DISPOSABLE_WORKER_ENVELOPE_TTL_SECONDS)
    {
        if config.receipt_path.exists() {
            remove_receipt(&config.receipt_path)?;
        }
        remove_envelope(&config.envelope_path)?;
        return Err(bootstrap_error(
            "disposable worker bootstrap expiry is outside the permitted window",
        ));
    }
    Ok(())
}

fn exchange_disposable_worker(
    config: &BootstrapConfig,
    cloud: &impl BootstrapCloudClient,
    envelope: &DisposableWorkerBootstrapEnvelope,
    pending_receipt: DisposableWorkerBootstrapReceipt,
) -> Result<(), DaemonError> {
    let binding = &pending_receipt.binding;
    let request = DisposableWorkerExchangeRequest {
        token: envelope.token.clone(),
        allocation_id: binding.allocation_id.clone(),
        worker_machine_id: binding.worker_machine_id.clone(),
        worker_kernel_id: binding.worker_kernel_id.clone(),
        image_digest: binding.image_digest.clone(),
        runtime_release_digest: binding.runtime_release_digest.clone(),
        manager_operation_id: binding.manager_operation_id.clone(),
        manager_operation_fence: binding.manager_operation_fence,
        manager_request_digest: binding.manager_request_digest.clone(),
    };
    match cloud.exchange_disposable_worker(&pending_receipt.cloud_api_url, &request)? {
        DisposableWorkerExchangeOutcome::Accepted(enrollment_receipt) => {
            accept_disposable_worker_enrollment(
                config,
                cloud,
                pending_receipt,
                enrollment_receipt,
            )
        }
        DisposableWorkerExchangeOutcome::Pending => Err(bootstrap_error(
            "Cloud has not completed the disposable worker bootstrap exchange",
        )),
        DisposableWorkerExchangeOutcome::Rejected => {
            remove_receipt(&config.receipt_path)?;
            remove_envelope(&config.envelope_path)?;
            Err(bootstrap_error(
                "Cloud terminally rejected the disposable worker bootstrap",
            ))
        }
    }
}

fn accept_disposable_worker_enrollment(
    config: &BootstrapConfig,
    cloud: &impl BootstrapCloudClient,
    mut receipt: DisposableWorkerBootstrapReceipt,
    enrollment_receipt: cloud::DisposableWorkerEnrollmentReceipt,
) -> Result<(), DaemonError> {
    receipt.status = DisposableWorkerBootstrapReceiptStatus::ExchangeAccepted;
    receipt.enrollment_receipt = Some(enrollment_receipt);
    if let Err(error) = validate_disposable_worker_enrollment(&receipt) {
        remove_receipt(&config.receipt_path)?;
        if config.envelope_path.exists() {
            remove_envelope(&config.envelope_path)?;
        }
        return Err(error);
    }
    receipt.persist(&config.receipt_path)?;
    if config.envelope_path.exists() {
        remove_envelope(&config.envelope_path)?;
    }
    match cloud.recover_disposable_worker_exchange(
        &receipt.cloud_api_url,
        &receipt.binding_digest,
        &receipt.binding,
    )? {
        DisposableWorkerRecoveryOutcome::Ready(result) => {
            finish_disposable_worker(config, receipt, result)
        }
        DisposableWorkerRecoveryOutcome::Pending => Err(bootstrap_error(
            "disposable worker exchange result is not yet recoverable",
        )),
        DisposableWorkerRecoveryOutcome::Rejected => {
            remove_receipt(&config.receipt_path)?;
            if config.envelope_path.exists() {
                remove_envelope(&config.envelope_path)?;
            }
            Err(bootstrap_error(
                "Cloud terminally rejected the disposable worker bootstrap result",
            ))
        }
    }
}

fn finish_disposable_worker(
    config: &BootstrapConfig,
    mut receipt: DisposableWorkerBootstrapReceipt,
    result: DisposableWorkerBootstrapResult,
) -> Result<(), DaemonError> {
    if receipt
        .enrollment_receipt
        .as_ref()
        .is_some_and(|expected| expected != &result.enrollment_receipt)
    {
        remove_receipt(&config.receipt_path)?;
        if config.envelope_path.exists() {
            remove_envelope(&config.envelope_path)?;
        }
        return Err(bootstrap_error(
            "recovered disposable worker receipt changed after exchange",
        ));
    }
    receipt.status = DisposableWorkerBootstrapReceiptStatus::Exchanged;
    receipt.enrollment_receipt = Some(result.enrollment_receipt);
    receipt.cloud_relay = Some(result.cloud_relay);
    if let Err(error) = validate_disposable_worker_result(&receipt) {
        remove_receipt(&config.receipt_path)?;
        if config.envelope_path.exists() {
            remove_envelope(&config.envelope_path)?;
        }
        return Err(error);
    }
    receipt.persist(&config.receipt_path)?;
    let expected_profile = persisted_profile(
        receipt
            .cloud_relay
            .clone()
            .ok_or_else(|| bootstrap_error("disposable worker relay receipt is missing"))?,
    );
    match load_managed_cloud_relay_profile() {
        Some(profile) if profile != expected_profile => {
            if config.envelope_path.exists() {
                remove_envelope(&config.envelope_path)?;
            }
            return Err(bootstrap_error(
                "existing Cloud profile conflicts with the disposable worker receipt",
            ));
        }
        Some(_) => {}
        None => persist_managed_cloud_relay_profile(expected_profile)?,
    }
    if config.envelope_path.exists() {
        remove_envelope(&config.envelope_path)?;
    }
    Ok(())
}

fn validate_disposable_worker_result(
    receipt: &DisposableWorkerBootstrapReceipt,
) -> Result<(), DaemonError> {
    validate_disposable_worker_enrollment(receipt)?;
    let binding = &receipt.binding;
    let relay = receipt.cloud_relay.as_ref().ok_or_else(|| {
        bootstrap_error("disposable worker relay receipt is missing")
    })?;
    if relay.machine_id != binding.worker_machine_id
        || relay.user_id != binding.user_id
        || relay.realm_id != binding.realm_id
        || relay.api_url != receipt.cloud_api_url
        || !valid_managed_relay_url(&relay.relay_url)
        || !valid_secret(&relay.machine_credential, "mcred_")
        || !valid_identifier(&relay.account_id)
        || !valid_identifier(&relay.account_slug)
        || !valid_identifier(&relay.issuer_id)
        || relay.email.trim().is_empty()
        || relay.email.len() > 320
        || relay.machine_alias.trim().is_empty()
        || relay.machine_alias.len() > 256
    {
        return Err(bootstrap_error(
            "Cloud disposable worker response does not match its immutable binding",
        ));
    }
    Ok(())
}

fn validate_disposable_worker_enrollment(
    receipt: &DisposableWorkerBootstrapReceipt,
) -> Result<(), DaemonError> {
    let binding = &receipt.binding;
    let enrollment = receipt.enrollment_receipt.as_ref().ok_or_else(|| {
        bootstrap_error("disposable worker enrollment receipt is missing")
    })?;
    if !valid_disposable_identifier(&enrollment.grant_id)
        || enrollment.allocation_id != binding.allocation_id
        || enrollment.worker_machine_id != binding.worker_machine_id
        || enrollment.worker_kernel_id != binding.worker_kernel_id
        || enrollment.image_digest != binding.image_digest
        || enrollment.runtime_release_digest != binding.runtime_release_digest
        || DateTime::parse_from_rfc3339(&enrollment.exchanged_at).is_err()
    {
        return Err(bootstrap_error(
            "Cloud disposable worker enrollment receipt does not match its immutable binding",
        ));
    }
    Ok(())
}

fn remove_invalid_disposable_worker_envelope(path: &std::path::Path) -> Result<(), DaemonError> {
    let disposable = std::fs::read(path)
        .ok()
        .filter(|bytes| bytes.len() <= 96 * 1024)
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| value.get("binding").is_some());
    if disposable {
        remove_envelope(path)?;
    }
    Ok(())
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
    if !confirmed.confirmed || confirmed.observed_state != "awaiting_context" {
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
) -> Result<(), DaemonError> {
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
    Ok(())
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
