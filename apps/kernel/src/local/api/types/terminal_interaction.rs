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
    /// The Chariox passkey (the vault passphrase), only for a choice marked
    /// `requires_passkey`. The kernel verifies it and never stores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passkey: Option<ApprovalPasskey>,
    /// With a verified passkey: accept this owner's critical approvals
    /// without it for this many minutes (1 to 15). Off when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passkey_remember_minutes: Option<u32>,
}

/// Longest optional window in which critical approvals need no passkey.
pub const PASSKEY_REMEMBER_MAX_MINUTES: u32 = 15;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApprovalPasskey(String);

impl ApprovalPasskey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApprovalPasskey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApprovalPasskey([REDACTED])")
    }
}

impl Drop for ApprovalPasskey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// MP-08 / MP-10 / MP-11 A07 (protocol 477): the owner's answer to a
/// protected hand-off. Only a Chariox terminal of the hand-off owner may send
/// it. A value goes from here directly to the bound browser field, under
/// observation protection; it never becomes an interaction reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RespondToHandoffRequest {
    pub session_id: String,
    pub interaction_id: String,
    pub action: HandoffResponseAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HandoffResponseAction {
    /// Click the bound element now, as the owner.
    Click,
    /// Type the value into the bound field, as the owner.
    EnterValue {
        value: HandoffValue,
        /// Secret hand-offs only: also store the value in the Vault under
        /// this new key. Grants no model read authority.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_to_vault_key: Option<String>,
    },
    /// The owner completed the step in the live browser.
    Done,
    Cancel,
}

impl<'de> Deserialize<'de> for HandoffResponseAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // MP-11 A07: tagged unit variants otherwise discard extra fields,
        // including a generic reply. Empty struct variants enforce the boundary.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Click {},
            EnterValue {
                value: HandoffValue,
                #[serde(default)]
                save_to_vault_key: Option<String>,
            },
            Done {},
            Cancel {},
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Click {} => Self::Click,
            Wire::EnterValue {
                value,
                save_to_vault_key,
            } => Self::EnterValue {
                value,
                save_to_vault_key,
            },
            Wire::Done {} => Self::Done,
            Wire::Cancel {} => Self::Cancel,
        })
    }
}

impl HandoffResponseAction {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::EnterValue { .. } => "enter_value",
            Self::Done => "done",
            Self::Cancel => "cancel",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HandoffValue(String);

impl HandoffValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for HandoffValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HandoffValue([REDACTED])")
    }
}

impl Drop for HandoffValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    Completed,
    Failed,
    Cancelled,
    Expired,
    /// Input may have reached the page; it is never replayed.
    Uncertain,
}

/// The only result an agent or another terminal ever learns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffOutcome {
    pub handoff_id: String,
    pub status: HandoffStatus,
    /// `click`, `enter_value`, `done`, `cancel`, `timeout`, `withdrawn`.
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub saved_to_vault: bool,
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
pub struct RequestNativeProviderTurnInteractionRequest {
    pub origin: crate::session::NativeInteractionOrigin,
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

impl RequestNativeProviderTurnInteractionRequest {
    pub fn allow_deny(
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        interaction_id: impl Into<String>,
        title: Option<String>,
        message: impl Into<String>,
        timeout_sec: Option<u64>,
        origin: crate::session::NativeInteractionOrigin,
    ) -> Self {
        Self {
            origin,
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
