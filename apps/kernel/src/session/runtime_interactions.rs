use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;

mod handoff;
pub(crate) use handoff::bounded_display_text;
pub use handoff::{
    HandoffChangeLine, HandoffChangeOp, HandoffKind, HandoffReason, HandoffTarget, RuntimeHandoff,
    HANDOFF_CHANGE_LINE_MAX_CHARS, HANDOFF_CHANGE_MAX_LINES, HANDOFF_EXPLANATION_MAX_CHARS,
    HANDOFF_LABEL_MAX_CHARS, HANDOFF_MAX_TIMEOUT_SEC, HANDOFF_MIN_TIMEOUT_SEC,
};
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
    /// passphrase) or an open remember window; a card payment always needs
    /// the typed passkey. Only kernel decisions may set it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    requires_passkey: bool,
    /// A Vault unlock answer: this choice unlocks for this long, with the
    /// passphrase as the custom reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vault_unlock: Option<crate::local::VaultUnlockDuration>,
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
            vault_unlock: None,
        }
    }

    pub(crate) fn unlocking_vault(mut self, duration: crate::local::VaultUnlockDuration) -> Self {
        self.vault_unlock = Some(duration);
        self
    }

    pub fn vault_unlock(&self) -> Option<crate::local::VaultUnlockDuration> {
        self.vault_unlock
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
    /// Run-scoped interaction: workspace trust before dispatch, or a kernel
    /// review while the provider is idle.
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
    /// Kernel-resolved presentation; agents supply only the observed element reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    view: Option<RuntimeInteractionView>,
    /// Protocol 936: a question owned by a durable task, rather than one turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asking_task_id: Option<String>,
    /// The agent a kernel-owned task decision is about; this is presentation, not authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    related_agent_id: Option<String>,
    /// Person cards: the existing wake source whose occurrence closes this card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    closes_with_wake: Option<String>,
    /// A sub-agent's decision the user holds is listed with the top parent, whose panel shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shown_in_agent_id: Option<String>,
    /// The kernel resolves the asker's alias or ref with the card, before hiding sub-agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asker_name: Option<String>,
    /// MP-08 / MP-11 protocol 538: kernel-established sudo presentation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sudo_request: Option<crate::local::SudoRequestPresentation>,
    /// An App install, update or reinstall the owner decides on, from the verified package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_install: Option<crate::local::AppInstallPresentation>,
    /// Protocol 470: structured OS requester for external access decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    requester: Option<crate::local::KernelAccessRequester>,
    /// MP-08 / MP-09 / MP-10 / MP-11: the payment a `vault-card-*` decision approves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payment: Option<crate::local::VaultPaymentApproval>,
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
    /// Ephemeral provider-native login UI, never model context or history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_login: Option<RuntimeProviderLogin>,
    /// Protocol 477: a protected owner hand-off (safe target metadata only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handoff: Option<RuntimeHandoff>,
    /// Local only: when a forwarded decision stops being actionable.
    #[serde(skip)]
    decision_expires_at: Option<tokio::time::Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractionView {
    pub browser_region: RuntimeInteractionBrowserRegion,
}

/// CSS pixels in the existing Tab display. No page text or pixel bytes are copied into the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractionBrowserRegion {
    pub tab_id: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub generation: u64,
    pub document_id: String,
    pub origin: String,
    pub rect: RuntimeInteractionBrowserRect,
    pub viewport_width: u32,
    pub viewport_height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<RuntimeInteractionBrowserVerification>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_result: Option<RuntimeInteractionBrowserVerificationResult>,
}

/// A positive HTTP result for this card's exact document and challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInteractionBrowserVerification {
    pub challenge_id: String,
    pub url: String,
    pub accepted_json_pointer: String,
}

/// Fixed labels and transport metadata only; no request or response values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInteractionBrowserVerificationResult {
    pub challenge_id: String,
    pub request_id: String,
    pub input_delivered: bool,
    pub http_status: Option<u16>,
    pub result: RuntimeInteractionBrowserVerificationOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionBrowserVerificationOutcome {
    Delivered,
    Rejected,
    Accepted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractionBrowserRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProviderLogin {
    /// Kernel retry feedback, absent during normal sign-in instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
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

/// The longest value an owner may type for an outside agent's Vault request: a long bearer token
/// fits. The clients advertise the same limit (`VAULT_SECRET_MAX_LENGTH` in the kernel client, the
/// secret page and popups in Cloud), and the interaction resolver enforces it.
pub(crate) const VAULT_SECRET_MAX_CHARS: usize = 8192;

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
            view: None,
            asking_task_id: None,
            related_agent_id: None,
            closes_with_wake: None,
            shown_in_agent_id: None,
            asker_name: None,
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
            provider_login: None,
            handoff: None,
            requester: None,
            payment: None,
            decision_expires_at: None,
            sudo_request: None,
            app_install: None,
        }
    }

    pub fn related_agent_id(&self) -> Option<&str> {
        self.related_agent_id.as_deref()
    }
    pub(crate) fn with_related_agent_id(mut self, agent: String) -> Self {
        self.related_agent_id = Some(agent);
        self
    }

    pub fn asking_task_id(&self) -> Option<&str> {
        self.asking_task_id.as_deref()
    }

    pub fn view(&self) -> Option<&RuntimeInteractionView> {
        self.view.as_ref()
    }

    pub(crate) fn with_view(mut self, view: RuntimeInteractionView) -> Self {
        self.view = Some(view);
        self
    }

    /// Asking again after an answer that did not work: the refusal takes the place of the
    /// prompt sentence, so the card keeps its height.
    pub(crate) fn showing_problem(mut self, problem: &str) -> Self {
        self.message = problem.to_owned();
        self
    }

    pub fn shown_in_agent_id(&self) -> Option<&str> {
        self.shown_in_agent_id.as_deref()
    }

    pub(crate) fn shown_in(mut self, agent: Option<String>) -> Self {
        self.shown_in_agent_id = agent;
        self
    }

    pub fn asker_name(&self) -> Option<&str> {
        self.asker_name.as_deref()
    }

    pub(crate) fn set_asker_name(&mut self, name: Option<String>) {
        self.asker_name = name;
    }

    pub fn closes_with_wake(&self) -> Option<&str> {
        self.closes_with_wake.as_deref()
    }

    pub(crate) fn closing_with_wake(mut self, wake_id: String) -> Self {
        self.closes_with_wake = Some(wake_id);
        self
    }

    pub(crate) fn with_asking_task_id(mut self, task: String) -> Self {
        self.asking_task_id = Some(task);
        self
    }

    pub fn sudo_request(&self) -> Option<&crate::local::SudoRequestPresentation> {
        self.sudo_request.as_ref()
    }
    pub fn app_install(&self) -> Option<&crate::local::AppInstallPresentation> {
        self.app_install.as_ref()
    }
    pub(crate) fn with_app_install(
        mut self,
        presentation: crate::local::AppInstallPresentation,
    ) -> Self {
        self.app_install = Some(presentation);
        self
    }
    pub(crate) fn with_sudo_request(
        mut self,
        request: crate::local::SudoRequestPresentation,
    ) -> Self {
        self.sudo_request = Some(request);
        self
    }

    pub fn payment(&self) -> Option<&crate::local::VaultPaymentApproval> {
        self.payment.as_ref()
    }

    pub(crate) fn with_payment(mut self, payment: crate::local::VaultPaymentApproval) -> Self {
        self.payment = Some(payment);
        self
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
            && self.related_agent_id.as_ref().is_none_or(|agent| {
                self.kernel_operation_id().is_some()
                    && !agent.is_empty()
                    && agent.len() <= 128
                    && !agent.chars().any(char::is_control)
            })
            && self.handoff.as_ref().is_none_or(|h| {
                h.valid()
                    && self.id == RuntimeHandoff::interaction_id(&h.obligation_id)
                    && self.kernel_operation_id() == Some(self.id.as_str())
                    && self.custom_choice.is_none()
                    && self.default_on_timeout.is_none()
            })
    }

    /// An outside agent's request for a Vault value: the owner answers with the secret
    /// choice or cancels. Critical like every Vault value request, so no grant holder answers it.
    pub(crate) fn for_vault_secret(
        id: impl Into<String>,
        operation_id: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
        timeout_sec: u64,
    ) -> Self {
        let mut interaction = Self::for_kernel_operation(
            id,
            operation_id,
            title,
            message,
            vec![RuntimeInteractionChoice::new(
                "cancel",
                "Cancel",
                "cancel",
                Some(RuntimeInteractionChoiceStyle::Danger),
            )],
        );
        interaction.kind = RuntimeInteractionKind::Choice;
        interaction.level = RuntimeInteractionLevel::Critical;
        interaction.custom_choice = Some(RuntimeInteractionCustomChoice::secret(
            "secret",
            "Secret",
            None,
            Some(1),
            Some(VAULT_SECRET_MAX_CHARS),
        ));
        interaction.timeout_sec = Some(timeout_sec);
        interaction
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
            view: None,
            asking_task_id: None,
            related_agent_id: None,
            closes_with_wake: None,
            shown_in_agent_id: None,
            asker_name: None,
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
            provider_login: None,
            handoff: None,
            requester: None,
            payment: None,
            decision_expires_at: None,
            sudo_request: None,
            app_install: None,
        }
    }

    /// A kernel decision that must close before its subject expires.
    pub(crate) fn with_timeout_sec(mut self, seconds: u64) -> Self {
        self.timeout_sec = Some(seconds);
        self
    }

    /// Task owner decisions live until their task is resolved or cancelled.
    pub(crate) fn without_timeout(mut self) -> Self {
        self.timeout_sec = None;
        self
    }

    /// Fixes the instant the decision expires, however long it takes to reach
    /// the kernel that shows it.
    pub(crate) fn with_decision_expiry(mut self, expires_at: tokio::time::Instant) -> Self {
        self.decision_expires_at = Some(expires_at);
        self.rebase_decision_budget();
        self
    }

    /// Restates the relative timeout from the fixed expiry; transports call it
    /// right before serializing so setup time is not handed to the receiver.
    pub(crate) fn rebase_decision_budget(&mut self) {
        let Some(expires_at) = self.decision_expires_at else {
            return;
        };
        let left = expires_at.saturating_duration_since(tokio::time::Instant::now());
        let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        self.timeout_sec = Some(seconds.max(1));
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

    /// A kernel-operation decision carrying a protected owner hand-off.
    pub(crate) fn with_handoff(mut self, handoff: RuntimeHandoff) -> Self {
        self.handoff = Some(handoff);
        self
    }

    pub fn handoff(&self) -> Option<&RuntimeHandoff> {
        self.handoff.as_ref()
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

    pub(crate) fn with_message(mut self, message: String) -> Self {
        self.message = message;
        self
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
