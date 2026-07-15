use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zeroize::Zeroizing;

use crate::error::DaemonError;
use crate::local::{
    deployment_credential_enrollment_interaction_id,
    deployment_credential_enrollment_service_subject, ArmDeploymentCredentialEnrollmentRequest,
    CredentialEnrollmentCallback, CredentialEnrollmentInteractionStatus, LocalDaemonResponse,
    RequestCredentialEnrollmentInteractionRequest,
};
use crate::runtime::command::{KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::runtime::state::KernelRuntimeState;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};

const ARM_TTL_MS: u64 = 5 * 60 * 1_000;
const MAX_ARM_ENTRIES: usize = 128;
const DEFAULT_INTERACTION_TIMEOUT_SEC: u64 = 5 * 60;
const MAX_INTERACTION_TIMEOUT_SEC: u64 = 10 * 60;
const MAX_IDENTIFIER_BYTES: usize = 256;
const MAX_PROVIDER_AUTHORIZATION_URL_BYTES: usize = 16 * 1_024;
const MAX_CALLBACK_CHARS: usize = 16 * 1_024;

const CALLBACK_CHOICE_ID: &str = "submit_callback";
const CANCEL_CHOICE_ID: &str = "cancel";

#[derive(Debug, Clone, PartialEq, Eq)]
struct CredentialEnrollmentBinding {
    enrollment_id: String,
    profile_id: String,
    target_version: u64,
    session_id: String,
    agent_id: String,
}

impl CredentialEnrollmentBinding {
    fn from_arm(request: &ArmDeploymentCredentialEnrollmentRequest) -> Self {
        Self {
            enrollment_id: request.enrollment_id.clone(),
            profile_id: request.profile_id.clone(),
            target_version: request.target_version,
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
        }
    }

    fn from_interaction(request: &RequestCredentialEnrollmentInteractionRequest) -> Self {
        Self {
            enrollment_id: request.enrollment_id.clone(),
            profile_id: request.profile_id.clone(),
            target_version: request.target_version,
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct CredentialEnrollmentArm {
    binding: CredentialEnrollmentBinding,
    owner_user_id: String,
    realm_id: String,
    expected_service_subject: String,
    expires_at_ms: u64,
}

#[derive(Debug, Clone)]
enum CredentialEnrollmentEntry {
    Armed(CredentialEnrollmentArm),
    Consumed { expires_at_ms: u64 },
}

impl CredentialEnrollmentEntry {
    fn expires_at_ms(&self) -> u64 {
        match self {
            Self::Armed(arm) => arm.expires_at_ms,
            Self::Consumed { expires_at_ms } => *expires_at_ms,
        }
    }
}

#[derive(Clone)]
pub(crate) struct CredentialEnrollmentControl {
    entries: Arc<Mutex<BTreeMap<String, CredentialEnrollmentEntry>>>,
    arm_ttl_ms: u64,
    max_entries: usize,
}

impl Default for CredentialEnrollmentControl {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            arm_ttl_ms: ARM_TTL_MS,
            max_entries: MAX_ARM_ENTRIES,
        }
    }
}

impl CredentialEnrollmentControl {
    fn arm(
        &self,
        binding: CredentialEnrollmentBinding,
        owner_user_id: String,
        realm_id: String,
        now_ms: u64,
    ) -> Result<u64, DaemonError> {
        let mut entries = self.lock_entries();
        prune_expired_entries(&mut entries, now_ms);
        if entries.contains_key(&binding.enrollment_id) {
            return Err(enrollment_error(
                "credential enrollment is already armed or consumed",
            ));
        }
        if entries.len() >= self.max_entries {
            return Err(enrollment_error(
                "credential enrollment arm capacity is full",
            ));
        }
        let expires_at_ms = now_ms.saturating_add(self.arm_ttl_ms);
        let enrollment_id = binding.enrollment_id.clone();
        let expected_service_subject =
            deployment_credential_enrollment_service_subject(&enrollment_id);
        entries.insert(
            enrollment_id,
            CredentialEnrollmentEntry::Armed(CredentialEnrollmentArm {
                binding,
                owner_user_id,
                realm_id,
                expected_service_subject,
                expires_at_ms,
            }),
        );
        Ok(expires_at_ms)
    }

    fn consume(
        &self,
        binding: &CredentialEnrollmentBinding,
        service_subject: &str,
        service_user_id: Option<&str>,
        service_realm_id: Option<&str>,
        now_ms: u64,
    ) -> Result<(), DaemonError> {
        let mut entries = self.lock_entries();
        prune_expired_entries(&mut entries, now_ms);
        let arm = match entries.get(&binding.enrollment_id) {
            Some(CredentialEnrollmentEntry::Armed(arm)) => arm,
            Some(CredentialEnrollmentEntry::Consumed { .. }) | None => {
                return Err(enrollment_error(
                    "credential enrollment arm is unavailable or unauthorized",
                ));
            }
        };
        if arm.binding != *binding {
            return Err(enrollment_error(
                "credential enrollment arm is unavailable or unauthorized",
            ));
        }
        if arm.expected_service_subject != service_subject
            || service_user_id != Some(arm.owner_user_id.as_str())
            || service_realm_id != Some(arm.realm_id.as_str())
        {
            return Err(enrollment_error(
                "credential enrollment arm is unavailable or unauthorized",
            ));
        }
        let expires_at_ms = now_ms.saturating_add(self.arm_ttl_ms);
        entries.insert(
            binding.enrollment_id.clone(),
            CredentialEnrollmentEntry::Consumed { expires_at_ms },
        );
        Ok(())
    }

    fn lock_entries(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<String, CredentialEnrollmentEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    fn with_limits(arm_ttl_ms: u64, max_entries: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            arm_ttl_ms,
            max_entries,
        }
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.lock_entries().len()
    }
}

pub(crate) async fn execute_arm_credential_enrollment(
    control: &CredentialEnrollmentControl,
    state: &KernelRuntimeState,
    command: &KernelCommand,
    request: ArmDeploymentCredentialEnrollmentRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    validate_binding_fields(&CredentialEnrollmentBinding::from_arm(&request))?;
    if !matches!(
        command.caller.caller_kind,
        KernelCallerKind::LocalClient | KernelCallerKind::RemoteClient
    ) || !matches!(
        command.source,
        KernelCommandSource::LocalCli
            | KernelCommandSource::LocalIpc
            | KernelCommandSource::RelayClient
    ) {
        return Err(enrollment_error(
            "credential enrollment arm requires an attached client",
        ));
    }
    let owner_user_id = command.caller.user_id.clone().ok_or_else(|| {
        enrollment_error("credential enrollment arm requires authenticated user identity")
    })?;
    let realm_id = command.caller.realm_id.clone().ok_or_else(|| {
        enrollment_error("credential enrollment arm requires an authenticated relay realm")
    })?;
    state
        .ensure_attachment_in_session(&request.session_id, &request.attachment_id)
        .await?;
    if state
        .attachment_owner_user_id(&request.attachment_id)
        .await?
        != owner_user_id
    {
        return Err(enrollment_error(
            "credential enrollment attachment is not owned by the caller",
        ));
    }
    if state
        .focused_agent_id(&request.session_id)
        .await?
        .as_deref()
        != Some(request.agent_id.as_str())
    {
        return Err(enrollment_error(
            "credential enrollment target is not the focused agent",
        ));
    }

    let binding = CredentialEnrollmentBinding::from_arm(&request);
    let expires_at_ms = control.arm(
        binding,
        owner_user_id,
        realm_id,
        crate::session::unix_epoch_ms(),
    )?;
    Ok(LocalDaemonResponse::DeploymentCredentialEnrollmentArmed {
        enrollment_id: request.enrollment_id,
        profile_id: request.profile_id,
        target_version: request.target_version,
        session_id: request.session_id,
        agent_id: request.agent_id,
        expires_at_ms,
    })
}

pub(crate) async fn execute_credential_enrollment_interaction(
    control: &CredentialEnrollmentControl,
    state: &KernelRuntimeState,
    command: &KernelCommand,
    request: RequestCredentialEnrollmentInteractionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let binding = CredentialEnrollmentBinding::from_interaction(&request);
    validate_binding_fields(&binding)?;
    if command.source != KernelCommandSource::RelayClient
        || command.caller.caller_kind != KernelCallerKind::HostedService
    {
        return Err(enrollment_error(
            "credential enrollment interaction requires verified hosted-service relay identity",
        ));
    }
    let timeout_sec = request
        .timeout_sec
        .unwrap_or(DEFAULT_INTERACTION_TIMEOUT_SEC);
    if timeout_sec == 0 || timeout_sec > MAX_INTERACTION_TIMEOUT_SEC {
        return Err(enrollment_error(
            "credential enrollment interaction timeout is invalid",
        ));
    }
    validate_provider_authorization_url(&request.provider_authorization_url)?;
    control.consume(
        &binding,
        &command.caller.caller_id,
        command.caller.user_id.as_deref(),
        command.caller.realm_id.as_deref(),
        crate::session::unix_epoch_ms(),
    )?;

    let interaction_id = deployment_credential_enrollment_interaction_id(&request.enrollment_id);
    let interaction = RuntimeInteraction::new(
        &interaction_id,
        &request.agent_id,
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Warning,
        Some("Authorize Claude".to_string()),
        format!(
            "Open this Claude authorization URL, then submit the callback value:\n{}",
            request.provider_authorization_url
        ),
        vec![RuntimeInteractionChoice::new(
            CANCEL_CHOICE_ID,
            "Cancel",
            "cancel",
            Some(RuntimeInteractionChoiceStyle::Secondary),
        )],
        Some(RuntimeInteractionCustomChoice::secret(
            CALLBACK_CHOICE_ID,
            "Submit callback",
            Some("Paste the Claude callback value".to_string()),
            Some(1),
            Some(MAX_CALLBACK_CHARS),
        )),
        Some(timeout_sec),
        None,
    );
    let receiver = state
        .create_runtime_interaction(&request.session_id, interaction)
        .await?;
    let timeout_state = state.clone();
    let timeout_session_id = request.session_id.clone();
    let timeout_interaction_id = interaction_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(timeout_sec)).await;
        let _ = timeout_state
            .timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id)
            .await;
    });
    let resolution = receiver.await.map_err(|_| {
        enrollment_error("credential enrollment interaction ended without a resolution")
    })?;
    let reply = resolution.reply.map(Zeroizing::new);
    match (resolution.status, resolution.choice_id.as_deref()) {
        ("answered", Some(CALLBACK_CHOICE_ID)) => {
            let callback = reply.ok_or_else(|| {
                enrollment_error("credential enrollment callback resolution was empty")
            })?;
            Ok(
                LocalDaemonResponse::CredentialEnrollmentInteractionResolved {
                    status: CredentialEnrollmentInteractionStatus::Submitted,
                    callback: Some(CredentialEnrollmentCallback::new(callback.to_string())),
                },
            )
        }
        ("answered", Some(CANCEL_CHOICE_ID)) => Ok(
            LocalDaemonResponse::CredentialEnrollmentInteractionResolved {
                status: CredentialEnrollmentInteractionStatus::Canceled,
                callback: None,
            },
        ),
        ("timed_out", None) => Ok(
            LocalDaemonResponse::CredentialEnrollmentInteractionResolved {
                status: CredentialEnrollmentInteractionStatus::TimedOut,
                callback: None,
            },
        ),
        _ => Err(enrollment_error(
            "credential enrollment interaction returned an invalid resolution",
        )),
    }
}

fn validate_binding_fields(binding: &CredentialEnrollmentBinding) -> Result<(), DaemonError> {
    for value in [
        binding.enrollment_id.as_str(),
        binding.profile_id.as_str(),
        binding.session_id.as_str(),
        binding.agent_id.as_str(),
    ] {
        if value.is_empty()
            || value.len() > MAX_IDENTIFIER_BYTES
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(enrollment_error(
                "credential enrollment binding contains an invalid identifier",
            ));
        }
    }
    if binding.target_version == 0 {
        return Err(enrollment_error(
            "credential enrollment target version must be positive",
        ));
    }
    Ok(())
}

fn validate_provider_authorization_url(value: &str) -> Result<(), DaemonError> {
    if value.len() > MAX_PROVIDER_AUTHORIZATION_URL_BYTES {
        return Err(enrollment_error(
            "credential enrollment provider authorization URL is invalid",
        ));
    }
    let parsed = url::Url::parse(value).map_err(|_| {
        enrollment_error("credential enrollment provider authorization URL is invalid")
    })?;
    let host = parsed.host_str().unwrap_or_default();
    let official_host = ["claude.ai", "claude.com", "anthropic.com"]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")));
    if parsed.scheme() != "https"
        || !official_host
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(enrollment_error(
            "credential enrollment provider authorization URL is invalid",
        ));
    }
    Ok(())
}

fn prune_expired_entries(entries: &mut BTreeMap<String, CredentialEnrollmentEntry>, now_ms: u64) {
    entries.retain(|_, entry| entry.expires_at_ms() > now_ms);
}

fn enrollment_error(message: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "credential enrollment control",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(enrollment_id: &str) -> CredentialEnrollmentBinding {
        CredentialEnrollmentBinding {
            enrollment_id: enrollment_id.to_string(),
            profile_id: "profile-1".to_string(),
            target_version: 7,
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
        }
    }

    fn arm(control: &CredentialEnrollmentControl, enrollment_id: &str, now_ms: u64) {
        control
            .arm(
                binding(enrollment_id),
                "user-1".to_string(),
                "realm-1".to_string(),
                now_ms,
            )
            .expect("arm should register");
    }

    fn consume(
        control: &CredentialEnrollmentControl,
        target: &CredentialEnrollmentBinding,
        now_ms: u64,
    ) -> Result<(), DaemonError> {
        control.consume(
            target,
            &deployment_credential_enrollment_service_subject(&target.enrollment_id),
            Some("user-1"),
            Some("realm-1"),
            now_ms,
        )
    }

    #[test]
    fn arm_requires_exact_tuple_and_target_without_consuming_on_mismatch() {
        let control = CredentialEnrollmentControl::with_limits(100, 4);
        arm(&control, "enrollment-1", 10);

        let mut wrong_profile = binding("enrollment-1");
        wrong_profile.profile_id = "profile-2".to_string();
        assert!(consume(&control, &wrong_profile, 20).is_err());
        let mut wrong_session = binding("enrollment-1");
        wrong_session.session_id = "session-2".to_string();
        assert!(consume(&control, &wrong_session, 20).is_err());
        let mut wrong_agent = binding("enrollment-1");
        wrong_agent.agent_id = "agent-2".to_string();
        assert!(consume(&control, &wrong_agent, 20).is_err());
        let mut wrong_version = binding("enrollment-1");
        wrong_version.target_version = 8;
        assert!(consume(&control, &wrong_version, 20).is_err());

        consume(&control, &binding("enrollment-1"), 20)
            .expect("mismatches must leave the exact arm available");
    }

    #[test]
    fn arm_rejects_wrong_subject_user_and_realm_without_consuming() {
        let control = CredentialEnrollmentControl::with_limits(100, 4);
        arm(&control, "enrollment-1", 10);
        let target = binding("enrollment-1");

        assert!(control
            .consume(
                &target,
                "deployment-credential-enrollment:other",
                Some("user-1"),
                Some("realm-1"),
                20,
            )
            .is_err());
        assert!(control
            .consume(
                &target,
                &deployment_credential_enrollment_service_subject("enrollment-1"),
                Some("user-2"),
                Some("realm-1"),
                20,
            )
            .is_err());
        assert!(control
            .consume(
                &target,
                &deployment_credential_enrollment_service_subject("enrollment-1"),
                Some("user-1"),
                Some("realm-2"),
                20,
            )
            .is_err());

        consume(&control, &target, 20).expect("identity mismatches must not consume the arm");
    }

    #[test]
    fn arm_expires_and_consumption_is_one_time() {
        let control = CredentialEnrollmentControl::with_limits(100, 4);
        arm(&control, "expired", 10);
        assert!(consume(&control, &binding("expired"), 110).is_err());

        arm(&control, "one-time", 120);
        consume(&control, &binding("one-time"), 130).expect("first consume should succeed");
        assert!(consume(&control, &binding("one-time"), 131).is_err());
        assert!(control
            .arm(
                binding("one-time"),
                "user-1".to_string(),
                "realm-1".to_string(),
                132,
            )
            .is_err());
    }

    #[test]
    fn arm_is_bound_to_one_kernel_control_target() {
        let armed_kernel = CredentialEnrollmentControl::with_limits(100, 4);
        let other_kernel = CredentialEnrollmentControl::with_limits(100, 4);
        arm(&armed_kernel, "target-bound", 10);

        assert!(consume(&other_kernel, &binding("target-bound"), 20).is_err());
        consume(&armed_kernel, &binding("target-bound"), 20)
            .expect("the exact armed kernel target should accept the helper");
    }

    #[test]
    fn arm_capacity_counts_live_arms_and_consumed_tombstones() {
        let control = CredentialEnrollmentControl::with_limits(100, 1);
        arm(&control, "enrollment-1", 10);
        consume(&control, &binding("enrollment-1"), 20).expect("consume should succeed");
        assert!(control
            .arm(
                binding("enrollment-2"),
                "user-1".to_string(),
                "realm-1".to_string(),
                111,
            )
            .is_err());

        arm(&control, "enrollment-2", 121);
        assert_eq!(control.entry_count(), 1);
    }

    #[test]
    fn authorization_url_accepts_only_bounded_official_https_hosts() {
        assert!(validate_provider_authorization_url(
            "https://claude.com/cai/oauth/authorize?state=opaque"
        )
        .is_ok());
        assert!(validate_provider_authorization_url("http://claude.com/authorize").is_err());
        assert!(
            validate_provider_authorization_url("https://claude.com.evil.test/authorize").is_err()
        );
        assert!(validate_provider_authorization_url("https://user@claude.com/authorize").is_err());
    }
}
