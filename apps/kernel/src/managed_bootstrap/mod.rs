mod cloud;
mod context_plan;
mod freshness;
mod provider_path;
mod release;
mod state;
mod supervisor;
pub mod worker;

use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rand::Rng;

use crate::config::{
    load_managed_cloud_relay_profile, load_or_create_managed_runtime_identity,
    persist_managed_cloud_relay_profile, DaemonConfig, ManagedRuntimeIdentity,
    PersistedCloudRelayProfile,
};
use crate::error::DaemonError;

use cloud::{
    BootstrapCloudClient, ConfirmRequest, ExchangeRequest, HttpBootstrapCloudClient,
    ManagedCloudRelayProfile,
};
pub use context_plan::ManagedKernelContextPlan;
use freshness::{
    capture_freshness_evidence, capture_old_generation_runtime_identity_report,
    validate_freshness_evidence, ManagedKernelFreshnessEvidence,
    ManagedKernelRuntimeIdentityReport,
};
use release::{verify_release, verify_release_evidence, VerifiedRelease};
use state::{
    default_managed_bootstrap_receipt_path, remove_envelope, trusted_builder_public_key_path,
    valid_identifier, valid_secret, BootstrapConfig, BootstrapEnvelope, BootstrapReceipt,
    BootstrapReceiptDocument, BootstrapReceiptStatus, ManagedBootstrapEnvelope,
};

const MIN_PREPARE_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_PREPARE_RETRY_DELAY: Duration = Duration::from_secs(60);
pub(crate) const MANAGED_REPOSITORY_ROOT_ENV: &str = "CHARIOX_MANAGED_REPOSITORY_ROOT";
pub(crate) const MANAGED_PROVIDER_TOPOLOGY_ENV: &str = "CHARIOX_MANAGED_PROVIDER_TOPOLOGY";
pub(crate) const PATH1_SHARED_HOST_SELECTOR_ENVS: &[&str] = &[
    "CHARIOX_CAPABILITY_ISOLATION_ROOT",
    "CHARIOX_MANAGED_PROVIDER_ISOLATION",
    "CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE",
    "CHARIOX_MANAGED_PROVIDER_BWRAP",
    "CHARIOX_MANAGED_SLICE_SERVICE_ROOT",
    "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT",
    "CHARIOX_SLICE_ROOT",
    "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
    "CHARIOX_SLICE_DOCKER_BROKER_FD",
    "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedProviderTopology {
    Path1,
    SharedHost,
}

impl ManagedProviderTopology {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Path1 => "path1",
            Self::SharedHost => "shared_host",
        }
    }
}

pub(crate) fn managed_provider_topology() -> Result<ManagedProviderTopology, DaemonError> {
    match std::env::var(MANAGED_PROVIDER_TOPOLOGY_ENV).as_deref() {
        Ok("path1") => Ok(ManagedProviderTopology::Path1),
        Ok("shared_host") => Ok(ManagedProviderTopology::SharedHost),
        Ok(_) => Err(DaemonError::LocalTransport {
            operation: "select managed bootstrap topology",
            message: format!("{MANAGED_PROVIDER_TOPOLOGY_ENV} must be path1 or shared_host"),
        }),
        Err(_) => Err(DaemonError::LocalTransport {
            operation: "select managed bootstrap topology",
            message: format!(
                "{MANAGED_PROVIDER_TOPOLOGY_ENV} must be explicitly set to path1 or shared_host"
            ),
        }),
    }
}

pub(crate) fn managed_repository_root_from_env() -> Result<std::path::PathBuf, DaemonError> {
    let value = match std::env::var(MANAGED_REPOSITORY_ROOT_ENV) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => state::DEFAULT_MANAGED_REPOSITORY_ROOT.to_string(),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(bootstrap_error(
                "managed repository root environment is not Unicode",
            ))
        }
    };
    let normalized = state::normalize_managed_repository_root(&value)?;
    if normalized != value {
        return Err(bootstrap_error(
            "managed repository root environment must be normalized",
        ));
    }
    Ok(std::path::PathBuf::from(normalized))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfirmedManagedKernelRegistration {
    pub environment_id: String,
    pub machine_id: String,
    pub kernel_id: String,
    pub context_plan: Option<ManagedKernelContextPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedReleaseEvidence {
    pub(crate) runtime_release_digest: String,
    pub(crate) source_commit: String,
    pub(crate) source_tree: String,
    pub(crate) target: String,
    pub(crate) active_release_path: String,
    pub(crate) manifest_signature_verified: bool,
    pub(crate) manifest_digest_verified: bool,
    pub(crate) kernel_artifact_verified: bool,
    pub(crate) bootstrap_receipt_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreReimageObservationAcknowledgement {
    pub(crate) environment_id: String,
    pub(crate) generation: u64,
    pub(crate) observed_at: String,
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
    let topology = managed_provider_topology()?;
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
                    topology,
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

pub(crate) fn authoritative_managed_release_evidence_from_env(
) -> Result<Option<ManagedReleaseEvidence>, DaemonError> {
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
    let receipt = match BootstrapReceiptDocument::read(&config.receipt_path)? {
        Some(BootstrapReceiptDocument::ManagedEnvironment(receipt)) => receipt,
        Some(BootstrapReceiptDocument::DisposableWorker(_)) => {
            return Err(bootstrap_error(
                "managed resource telemetry cannot use a disposable-worker receipt",
            ));
        }
        None => {
            return Err(bootstrap_error(
                "managed bootstrap receipt disappeared during release verification",
            ));
        }
    };
    if receipt.status != BootstrapReceiptStatus::Confirmed {
        return Err(bootstrap_error(
            "managed resource telemetry requires a confirmed bootstrap receipt",
        ));
    }
    let verified = verify_release_evidence(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &receipt.runtime_release_digest,
        &config.kernel_binary,
        trusted_builder_public_key_path()?.as_deref(),
    )?;
    if verified.digest != receipt.runtime_release_digest {
        return Err(bootstrap_error(
            "verified release digest does not match the bootstrap receipt",
        ));
    }
    Ok(Some(ManagedReleaseEvidence {
        runtime_release_digest: verified.digest,
        source_commit: verified.source_commit,
        source_tree: verified.source_tree,
        target: verified.target,
        active_release_path: verified.active_release_path.display().to_string(),
        manifest_signature_verified: verified.manifest_signature_verified,
        manifest_digest_verified: verified.manifest_digest_verified,
        kernel_artifact_verified: verified.kernel_artifact_verified,
        bootstrap_receipt_verified: true,
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
                let envelope_repository_root = value.managed_repository_root()?;
                let receipt_repository_root = receipt.managed_repository_root()?;
                if value.environment_id != receipt.environment_id
                    || value.runtime_release_digest != receipt.runtime_release_digest
                    || envelope_repository_root != receipt_repository_root
                    || value.provider_rebuild_action_id.is_some()
                        && receipt.provider_rebuild_action_id.as_deref()
                            != value.provider_rebuild_action_id.as_deref()
                {
                    return Err(bootstrap_error(
                        "managed bootstrap envelope conflicts with its receipt",
                    ));
                }
            }
            resume_registration(config, envelope.as_ref(), receipt, &identity, &release)?
        }
        (None, Some(envelope)) => {
            begin_registration(config, cloud, now, &envelope, &identity, &release)?
        }
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
    release: &VerifiedRelease,
) -> Result<Option<PendingConfirmation>, DaemonError> {
    if envelope.expires_at()? <= now {
        return Err(bootstrap_error(
            "managed bootstrap token expired before exchange",
        ));
    }
    let freshness_evidence = capture_rebuild_freshness_evidence(config, release, envelope)?;
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
    let managed_repository_root = envelope.managed_repository_root()?;
    let profile = persisted_profile(exchanged.cloud_relay);
    persist_managed_cloud_relay_profile(profile.clone())?;
    let receipt = BootstrapReceipt {
        schema_version: envelope.schema_version,
        status: BootstrapReceiptStatus::Exchanged,
        environment_id: envelope.environment_id.clone(),
        machine_id: identity.machine_id.clone(),
        kernel_id: identity.kernel_id.clone(),
        generation: exchanged.generation,
        relay_public_key: identity.relay_public_key.clone(),
        runtime_release_digest: envelope.runtime_release_digest.clone(),
        managed_repository_root: (envelope.schema_version == 2).then_some(managed_repository_root),
        confirmed_at: None,
        context_plan: Some(exchanged.context_plan),
        provider_rebuild_action_id: envelope.provider_rebuild_action_id.clone(),
        freshness_evidence,
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
    release: &VerifiedRelease,
) -> Result<Option<PendingConfirmation>, DaemonError> {
    validate_receipt_identity(&receipt, identity)?;
    let profile = load_managed_cloud_relay_profile()
        .ok_or_else(|| bootstrap_error("managed Cloud profile is missing after exchange"))?;
    validate_profile(&profile, &receipt)?;
    let receipt = ensure_rebuild_freshness_evidence(config, envelope, receipt, release)?;
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
    let freshness_evidence = match receipt.provider_rebuild_action_id.as_deref() {
        Some(_action_id) => {
            let evidence = receipt
                .freshness_evidence
                .as_ref()
                .ok_or_else(|| bootstrap_error("managed reimage freshness evidence is missing"))?;
            validate_freshness_evidence(evidence, &receipt.runtime_release_digest)?;
            let verified = verify_release_evidence(
                &config.manifest_path,
                &config.signature_path,
                &config.public_key_path,
                &receipt.runtime_release_digest,
                &config.kernel_binary,
                trusted_builder_public_key_path()?.as_deref(),
            )?;
            if evidence.runtime_source_commit != verified.source_commit
                || evidence.runtime_source_tree != verified.source_tree
            {
                return Err(bootstrap_error(
                    "managed reimage source identity does not match the signed release",
                ));
            }
            Some(evidence.clone())
        }
        None => {
            if receipt.freshness_evidence.is_some() {
                return Err(bootstrap_error(
                    "managed freshness evidence has no rebuild action binding",
                ));
            }
            None
        }
    };
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
            freshness_evidence,
        },
    )?;
    if !confirmed.confirmed || confirmed.observed_state != "awaiting_context" {
        return Err(bootstrap_error(
            "Cloud did not confirm the managed kernel bootstrap",
        ));
    }
    let confirmed_repository_root = state::managed_repository_root_for_schema(
        receipt.schema_version,
        confirmed.managed_repository_root.as_deref(),
    )?;
    if confirmed_repository_root != receipt.managed_repository_root()? {
        return Err(bootstrap_error(
            "Cloud confirmed a different managed repository root",
        ));
    }
    receipt.status = BootstrapReceiptStatus::Confirmed;
    receipt.confirmed_at = Some(now.to_rfc3339());
    receipt.provider_rebuild_action_id = None;
    receipt.freshness_evidence = None;
    receipt.persist(&config.receipt_path)?;
    remove_envelope(&config.envelope_path)
}

fn report_pre_reimage_runtime_identity(
    config: &BootstrapConfig,
    cloud: &impl BootstrapCloudClient,
    api_url: &str,
    environment_id: &str,
    machine_id: &str,
    kernel_id: &str,
    generation: u64,
    profile: &PersistedCloudRelayProfile,
    release: &VerifiedRelease,
    observed_at: DateTime<Utc>,
) -> Result<ManagedKernelRuntimeIdentityReport, DaemonError> {
    if profile.machine_id.as_deref() != Some(machine_id) {
        return Err(bootstrap_error(
            "managed pre-reimage report machine identity does not match the current Cloud profile",
        ));
    }
    let machine_credential = profile
        .machine_credential
        .as_deref()
        .ok_or_else(|| bootstrap_error("managed machine credential is missing"))?;
    let verified = verify_release_evidence(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &release.digest,
        &config.kernel_binary,
        trusted_builder_public_key_path()?.as_deref(),
    )?;
    let report = capture_old_generation_runtime_identity_report(
        environment_id,
        machine_id,
        kernel_id,
        generation,
        machine_credential,
        &verified,
        observed_at,
    )?;
    let response = cloud.report_runtime_identity(&normalized_api_url(api_url), &report)?;
    if !response.accepted
        || response.environment_id != report.environment_id
        || response.generation != report.generation
        || response.observed_at != report.observed_at
    {
        return Err(bootstrap_error(
            "Cloud did not accept the managed pre-reimage runtime identity",
        ));
    }
    Ok(report)
}

pub(crate) fn observe_managed_environment_pre_reimage(
    daemon_config: &DaemonConfig,
    registration: &ConfirmedManagedKernelRegistration,
    expected_environment_id: &str,
    expected_generation: u64,
    observed_at: DateTime<Utc>,
) -> Result<PreReimageObservationAcknowledgement, DaemonError> {
    let config = BootstrapConfig::from_env()?;
    let receipt = match BootstrapReceiptDocument::read(&config.receipt_path)? {
        Some(BootstrapReceiptDocument::ManagedEnvironment(receipt)) => receipt,
        Some(BootstrapReceiptDocument::DisposableWorker(_)) => {
            return Err(bootstrap_error(
                "managed pre-reimage observation cannot use a disposable-worker receipt",
            ));
        }
        None => {
            return Err(bootstrap_error(
                "managed pre-reimage observation requires a bootstrap receipt",
            ));
        }
    };
    if receipt.status != BootstrapReceiptStatus::Confirmed {
        return Err(bootstrap_error(
            "managed pre-reimage observation requires a confirmed bootstrap receipt",
        ));
    }
    let profile = validate_pre_reimage_observation_binding(
        daemon_config,
        registration,
        &receipt,
        expected_environment_id,
        expected_generation,
    )?;
    let release = verify_release(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &receipt.runtime_release_digest,
        &config.kernel_binary,
    )?;
    let report = report_pre_reimage_runtime_identity(
        &config,
        &HttpBootstrapCloudClient::default(),
        &profile.api_url,
        &receipt.environment_id,
        &receipt.machine_id,
        &receipt.kernel_id,
        receipt.generation,
        profile,
        &release,
        observed_at,
    )?;
    Ok(PreReimageObservationAcknowledgement {
        environment_id: report.environment_id,
        generation: report.generation,
        observed_at: report.observed_at,
    })
}

fn validate_pre_reimage_observation_binding<'a>(
    daemon_config: &'a DaemonConfig,
    registration: &ConfirmedManagedKernelRegistration,
    receipt: &BootstrapReceipt,
    expected_environment_id: &str,
    expected_generation: u64,
) -> Result<&'a PersistedCloudRelayProfile, DaemonError> {
    if expected_environment_id != receipt.environment_id
        || expected_generation != receipt.generation
    {
        return Err(bootstrap_error(
            "managed pre-reimage observation does not match the current environment generation",
        ));
    }
    if registration.environment_id != receipt.environment_id
        || registration.machine_id != receipt.machine_id
        || registration.kernel_id != receipt.kernel_id
        || daemon_config.daemon_id != receipt.kernel_id
        || daemon_config.host_machine_id != receipt.machine_id
        || daemon_config.relay_public_key != receipt.relay_public_key
    {
        return Err(bootstrap_error(
            "managed pre-reimage observation does not match the running kernel identity",
        ));
    }
    let profile = daemon_config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| bootstrap_error("managed Cloud profile is missing"))?;
    validate_profile(profile, receipt)?;
    Ok(profile)
}

fn capture_rebuild_freshness_evidence(
    config: &BootstrapConfig,
    release: &VerifiedRelease,
    envelope: &ManagedBootstrapEnvelope,
) -> Result<Option<ManagedKernelFreshnessEvidence>, DaemonError> {
    let Some(_action_id) = envelope.provider_rebuild_action_id.as_deref() else {
        return Ok(None);
    };
    let verified = verify_release_evidence(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &release.digest,
        &config.kernel_binary,
        trusted_builder_public_key_path()?.as_deref(),
    )?;
    Ok(Some(capture_freshness_evidence(config, &verified)?))
}

fn ensure_rebuild_freshness_evidence(
    config: &BootstrapConfig,
    envelope: Option<&ManagedBootstrapEnvelope>,
    mut receipt: BootstrapReceipt,
    release: &VerifiedRelease,
) -> Result<BootstrapReceipt, DaemonError> {
    let envelope_action_id = envelope.and_then(|value| value.provider_rebuild_action_id.as_deref());
    if envelope_action_id.is_some()
        && receipt
            .provider_rebuild_action_id
            .as_deref()
            .is_some_and(|value| Some(value) != envelope_action_id)
    {
        return Err(bootstrap_error(
            "managed rebuild action ID conflicts with its receipt",
        ));
    }
    let action_id = envelope_action_id.or(receipt.provider_rebuild_action_id.as_deref());
    let Some(action_id) = action_id else {
        if receipt.freshness_evidence.is_some() {
            return Err(bootstrap_error(
                "managed freshness evidence is not bound to a rebuild operation",
            ));
        }
        return Ok(receipt);
    };
    receipt.provider_rebuild_action_id = Some(action_id.to_string());
    let verified = verify_release_evidence(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &release.digest,
        &config.kernel_binary,
        trusted_builder_public_key_path()?.as_deref(),
    )?;
    if receipt.status == BootstrapReceiptStatus::Confirmed {
        let evidence = receipt.freshness_evidence.as_ref().ok_or_else(|| {
            bootstrap_error("managed confirmed reimage freshness evidence is missing")
        })?;
        validate_freshness_evidence(evidence, &verified.digest)?;
        if evidence.runtime_source_commit != verified.source_commit
            || evidence.runtime_source_tree != verified.source_tree
        {
            return Err(bootstrap_error(
                "managed confirmed reimage source identity does not match the signed release",
            ));
        }
        return Ok(receipt);
    }
    if envelope_action_id.is_none() {
        return Err(bootstrap_error(
            "managed reimage confirmation envelope is missing",
        ));
    }
    receipt.freshness_evidence = Some(capture_freshness_evidence(config, &verified)?);
    receipt.persist(&config.receipt_path)?;
    Ok(receipt)
}

fn validate_exchange_response(
    envelope: &ManagedBootstrapEnvelope,
    identity: &ManagedRuntimeIdentity,
    response: &cloud::ExchangeResponse,
) -> Result<(), DaemonError> {
    let response_repository_root = state::managed_repository_root_for_schema(
        envelope.schema_version,
        response.managed_repository_root.as_deref(),
    )?;
    if response.environment_id != envelope.environment_id
        || response.kernel_id != identity.kernel_id
        || !(1..=i32::MAX as u64).contains(&response.generation)
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
        || response_repository_root != envelope.managed_repository_root()?
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
