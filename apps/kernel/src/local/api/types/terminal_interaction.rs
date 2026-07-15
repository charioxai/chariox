use super::*;

use std::fmt;

use zeroize::Zeroize;

pub const DEPLOYMENT_CREDENTIAL_ENROLLMENT_SERVICE_SUBJECT_PREFIX: &str =
    "deployment-credential-enrollment:";
pub const DEPLOYMENT_CREDENTIAL_ENROLLMENT_INTERACTION_ID_PREFIX: &str = "credential-enrollment:";

pub fn deployment_credential_enrollment_service_subject(enrollment_id: &str) -> String {
    format!("{DEPLOYMENT_CREDENTIAL_ENROLLMENT_SERVICE_SUBJECT_PREFIX}{enrollment_id}")
}

pub fn deployment_credential_enrollment_interaction_id(enrollment_id: &str) -> String {
    format!("{DEPLOYMENT_CREDENTIAL_ENROLLMENT_INTERACTION_ID_PREFIX}{enrollment_id}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollRuntimeNoticesRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RespondToInteractionRequest {
    pub session_id: String,
    pub interaction_id: String,
    pub choice_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_reply: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmDeploymentCredentialEnrollmentRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub agent_id: String,
    pub enrollment_id: String,
    pub profile_id: String,
    pub target_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCredentialEnrollmentInteractionRequest {
    pub session_id: String,
    pub agent_id: String,
    pub enrollment_id: String,
    pub profile_id: String,
    pub target_version: u64,
    pub provider_authorization_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialEnrollmentInteractionStatus {
    Submitted,
    Canceled,
    TimedOut,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialEnrollmentCallback(String);

impl CredentialEnrollmentCallback {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CredentialEnrollmentCallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialEnrollmentCallback([REDACTED])")
    }
}

impl Drop for CredentialEnrollmentCallback {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestNativeProviderInteractionRequest {
    pub session_id: String,
    pub agent_id: String,
    pub interaction_id: String,
    pub level: RuntimeInteractionLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub message: String,
    pub choices: Vec<RuntimeInteractionChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_choice: Option<RuntimeInteractionCustomChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_on_timeout: Option<String>,
}

impl RequestNativeProviderInteractionRequest {
    pub fn allow_deny(
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        interaction_id: impl Into<String>,
        title: Option<String>,
        message: impl Into<String>,
        timeout_sec: Option<u64>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            interaction_id: interaction_id.into(),
            level: RuntimeInteractionLevel::Warning,
            title,
            message: message.into(),
            choices: vec![
                RuntimeInteractionChoice::new(
                    "allow_once",
                    "Allow once",
                    "allow",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            custom_choice: None,
            timeout_sec,
            default_on_timeout: Some("deny".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeProviderInteractionResolution {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResizeTerminalRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendTerminalInputRequest {
    pub session_id: String,
    pub attachment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
    pub data_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PumpTerminalOutputRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendNativeProviderOutputRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub provider_run_id: String,
    pub kind: TerminalOutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendNativeProviderOutputBatchRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub outputs: Vec<AppendNativeProviderOutputBatchItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendNativeProviderOutputBatchItem {
    pub provider_run_id: String,
    pub kind: TerminalOutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
    pub text: String,
}
