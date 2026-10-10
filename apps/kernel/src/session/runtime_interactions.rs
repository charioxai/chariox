use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;

mod subject;
pub use subject::RuntimeInteractionSubject;

#[cfg(test)]
mod subject_tests;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionKind {
    Choice,
    Permission,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionLevel {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionChoiceStyle {
    Primary,
    Secondary,
    Danger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractionChoice {
    id: String,
    label: String,
    reply: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    style: Option<RuntimeInteractionChoiceStyle>,
    /// Answering with this choice needs the Chariox passkey (the vault
    /// passphrase) or an open remember window. Only kernel-operation
    /// decisions may set it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    requires_passkey: bool,
}

impl RuntimeInteractionChoice {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        reply: impl Into<String>,
        style: Option<RuntimeInteractionChoiceStyle>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            reply: reply.into(),
            style,
            requires_passkey: false,
        }
    }

    /// Marks a critical approval: the kernel accepts it only with a verified
    /// passkey or within the owner's remember window.
    pub(crate) fn requiring_passkey(mut self) -> Self {
        self.requires_passkey = true;
        self
    }

    pub fn requires_passkey(&self) -> bool {
        self.requires_passkey
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn reply(&self) -> &str {
        &self.reply
    }

    pub fn style(&self) -> Option<RuntimeInteractionChoiceStyle> {
        self.style
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractionCustomChoice {
    id: String,
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_length: Option<usize>,
    #[serde(
        default,
        skip_serializing_if = "runtime_interaction_input_kind_is_text"
    )]
    input_kind: RuntimeInteractionInputKind,
}

impl RuntimeInteractionCustomChoice {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        placeholder: Option<String>,
        min_length: Option<usize>,
        max_length: Option<usize>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            placeholder,
            min_length,
            max_length,
            input_kind: RuntimeInteractionInputKind::Text,
        }
    }

    pub fn secret(
        id: impl Into<String>,
        label: impl Into<String>,
        placeholder: Option<String>,
        min_length: Option<usize>,
        max_length: Option<usize>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            placeholder,
            min_length,
            max_length,
            input_kind: RuntimeInteractionInputKind::Secret,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn placeholder(&self) -> Option<&str> {
        self.placeholder.as_deref()
    }

    pub fn min_length(&self) -> usize {
        self.min_length.unwrap_or(1)
    }

    pub fn max_length(&self) -> Option<usize> {
        self.max_length
    }

    pub fn input_kind(&self) -> RuntimeInteractionInputKind {
        self.input_kind
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionInputKind {
    Text,
    Secret,
}

impl Default for RuntimeInteractionInputKind {
    fn default() -> Self {
        Self::Text
    }
}

fn runtime_interaction_input_kind_is_text(kind: &RuntimeInteractionInputKind) -> bool {
    *kind == RuntimeInteractionInputKind::Text
}

/// Captured by the producer before handing an approval to an asynchronous bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum NativeInteractionOrigin {
    Prompt {
        provider_run_id: String,
        prompt_id: String,
    },
    NativeTurn {
        provider_run_id: String,
        native_turn_id: String,
    },
    /// Workspace trust is requested before a task is dispatched.
    ProviderStartup { provider_run_id: String },
}

impl NativeInteractionOrigin {
    pub fn provider_run_id(&self) -> &str {
        match self {
            Self::Prompt {
                provider_run_id, ..
            }
            | Self::NativeTurn {
                provider_run_id, ..
            }
            | Self::ProviderStartup { provider_run_id } => provider_run_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteraction {
    /// Protocol 470: structured OS requester for external access decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    requester: Option<crate::local::KernelAccessRequester>,
    id: String,
    #[serde(flatten)]
    subject: RuntimeInteractionSubject,
    kind: RuntimeInteractionKind,
    level: RuntimeInteractionLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    message: String,
    choices: Vec<RuntimeInteractionChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_choice: Option<RuntimeInteractionCustomChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_on_timeout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_origin: Option<NativeInteractionOrigin>,
    requested_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_environment_review: Option<crate::project_environment::ProjectEnvironmentReview>,
    /// Ephemeral provider-native login UI, never model context or history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_login: Option<RuntimeProviderLogin>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProviderLogin {
    pub kernel_id: String,
    pub login: crate::provider::ProviderLoginStart,
    pub terminal_output_base64: String,
}

impl std::fmt::Debug for RuntimeProviderLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeProviderLogin")
            .field("kernel_id", &self.kernel_id)
            .field("login", &self.login)
            .field("terminal_output_base64", &"[REDACTED]")
            .finish()
    }
}

impl RuntimeInteraction {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        agent_id: impl Into<String>,
        kind: RuntimeInteractionKind,
        level: RuntimeInteractionLevel,
        title: Option<String>,
        message: impl Into<String>,
        choices: Vec<RuntimeInteractionChoice>,
        custom_choice: Option<RuntimeInteractionCustomChoice>,
        timeout_sec: Option<u64>,
        default_on_timeout: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            subject: RuntimeInteractionSubject::Agent {
                agent_id: agent_id.into(),
            },
            kind,
            level,
            title,
            message: message.into(),
            choices,
            custom_choice,
            timeout_sec,
            default_on_timeout,
            native_origin: None,
            requested_at_ms: unix_epoch_ms(),
            project_environment_review: None,
            provider_login: None,
            requester: None,
        }
    }

    pub fn requester(&self) -> Option<&crate::local::KernelAccessRequester> {
        self.requester.as_ref()
    }

    pub(crate) fn with_requester(mut self, requester: crate::local::KernelAccessRequester) -> Self {
        self.requester = Some(requester);
        self
    }

    pub fn native_origin(&self) -> Option<&NativeInteractionOrigin> {
        self.native_origin.as_ref()
    }

    pub fn with_native_origin(mut self, origin: Option<NativeInteractionOrigin>) -> Self {
        self.native_origin = origin;
        self
    }

    pub fn with_project_environment_review(
        mut self,
        review: crate::project_environment::ProjectEnvironmentReview,
    ) -> Self {
        self.project_environment_review = Some(review);
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_id(&self) -> Option<&str> {
        self.subject.agent_id()
    }

    pub fn kernel_operation_id(&self) -> Option<&str> {
        self.subject.kernel_operation_id()
    }

    pub fn subject(&self) -> &RuntimeInteractionSubject {
        &self.subject
    }

    pub(crate) fn valid_subject(&self) -> bool {
        self.subject.valid()
    }

    /// A kernel-owned decision, projected without creating an agent or prompt.
    /// Only the dedicated registration path binds its authenticated owner.
    pub(crate) fn for_kernel_operation(
        id: impl Into<String>,
        operation_id: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
        choices: Vec<RuntimeInteractionChoice>,
    ) -> Self {
        Self {
            id: id.into(),
            subject: RuntimeInteractionSubject::KernelOperation {
                kernel_operation_id: operation_id.into(),
            },
            kind: RuntimeInteractionKind::Permission,
            level: RuntimeInteractionLevel::Warning,
            title: Some(title.into()),
            message: message.into(),
            choices,
            custom_choice: None,
            timeout_sec: Some(300),
            default_on_timeout: None,
            native_origin: None,
            requested_at_ms: unix_epoch_ms(),
            project_environment_review: None,
            provider_login: None,
            requester: None,
        }
    }

    /// A kernel decision that must close before its subject expires.
    pub(crate) fn with_timeout_sec(mut self, seconds: u64) -> Self {
        self.timeout_sec = Some(seconds);
        self
    }

    pub fn with_provider_login(mut self, login: RuntimeProviderLogin) -> Self {
        self.provider_login = Some(login);
        self
    }

    pub(crate) fn provider_login_is_human_only(&self) -> bool {
        self.provider_login.is_some() || self.id.starts_with("provider-auth-recovery:")
    }

    pub fn provider_login(&self) -> Option<&RuntimeProviderLogin> {
        self.provider_login.as_ref()
    }

    pub fn kind(&self) -> RuntimeInteractionKind {
        self.kind
    }

    pub fn level(&self) -> RuntimeInteractionLevel {
        self.level
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn choices(&self) -> &[RuntimeInteractionChoice] {
        &self.choices
    }

    pub fn custom_choice(&self) -> Option<&RuntimeInteractionCustomChoice> {
        self.custom_choice.as_ref()
    }

    pub fn timeout_sec(&self) -> Option<u64> {
        self.timeout_sec
    }

    pub fn default_on_timeout(&self) -> Option<&str> {
        self.default_on_timeout.as_deref()
    }

    pub fn requested_at_ms(&self) -> u64 {
        self.requested_at_ms
    }

    pub fn choice(&self, choice_id: &str) -> Option<&RuntimeInteractionChoice> {
        self.choices.iter().find(|choice| choice.id() == choice_id)
    }

    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.subject = RuntimeInteractionSubject::Agent {
            agent_id: agent_id.into(),
        };
        self
    }
}
