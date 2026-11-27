use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptStatus {
    Queued,
    Dispatching,
    Running,
    Cancelling,
    Completed,
    Cancelled,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptOrigin {
    #[default]
    #[serde(alias = "arroba")]
    Chariox,
    CompletionCheck,
    External,
    /// A schedule trigger fired this prompt.
    Schedule,
    /// A local HTTP trigger fired this prompt.
    Http,
    /// A workflow notification was delivered as this prompt.
    Notification,
}

impl PromptOrigin {
    /// Prompts Chariox itself shows as its own: a person typed them, or a
    /// trigger or notification fired them. Not an external provider's prompts
    /// and not the hidden completion check.
    pub fn is_chariox_prompt(self) -> bool {
        matches!(
            self,
            Self::Chariox | Self::Schedule | Self::Http | Self::Notification
        )
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DurablePromptDeliveryPhase {
    Accepted,
    Dispatching,
    Delivered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptAttachment {
    url: String,
    mime: String,
    filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contents_base64: Option<String>,
}

impl PromptAttachment {
    pub fn new(url: impl Into<String>, mime: impl Into<String>, filename: Option<String>) -> Self {
        Self {
            url: url.into(),
            mime: mime.into(),
            filename,
            contents_base64: None,
        }
    }

    pub fn with_contents_base64(mut self, contents_base64: impl Into<String>) -> Self {
        self.contents_base64 = Some(contents_base64.into());
        self
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn mime(&self) -> &str {
        &self.mime
    }

    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    pub fn contents_base64(&self) -> Option<&str> {
        self.contents_base64.as_deref()
    }

    /// Text decoding shared by provider input and Room observation protection.
    pub(crate) fn text_contents(&self) -> Option<String> {
        use base64::Engine as _;
        let textual = self.mime.starts_with("text/")
            || matches!(
                self.mime.as_str(),
                "application/json"
                    | "application/javascript"
                    | "application/typescript"
                    | "application/xml"
                    | "application/yaml"
                    | "application/x-yaml"
            )
            || self.mime.ends_with("+json")
            || self.mime.ends_with("+xml");
        if !textual {
            return None;
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(self.contents_base64()?)
            .ok()?;
        String::from_utf8(bytes).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptQueueItem {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_prompt_id: Option<String>,
    source_attachment_id: String,
    target_agent_id: String,
    prompt: String,
    attachments: Vec<PromptAttachment>,
    #[serde(default = "crate::session::unix_epoch_ms")]
    created_at_ms: u64,
    #[serde(default = "crate::session::unix_epoch_ms")]
    updated_at_ms: u64,
    #[serde(default, skip_serializing, skip_deserializing)]
    private_metadata: Option<Box<PromptPrivateMetadata>>,
    #[serde(skip)]
    remote_steer_reservation: RemoteSteerReservation,
    status: PromptStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    launch_wait: Option<String>,
    #[serde(default)]
    prompt_origin: PromptOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider_turn_id: Option<String>,
    workflow_run_id: Option<String>,
    workflow_node_run_id: Option<String>,
}

/// Durable identity needed to keep an ambiguously delivered queued steer blocked
/// until a future exact worker receipt can reconcile it. Relay credentials are
/// deliberately excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DurableRemoteQueuedSteerUncertainty {
    target_home_prompt_id: String,
    worker_provider_run_id: Option<String>,
    worker_kernel_id: String,
    worker_machine_id: String,
    execution_lease_id: String,
    leased_agent_id: String,
}

/// Clone-shared runtime reservation and fail-closed uncertainty marker. The
/// transient reservation is not serialized; uncertainty is persisted separately
/// in `DurablePromptPrivateState`.
#[derive(Debug, Clone, Default)]
struct RemoteSteerReservation {
    id: Arc<AtomicU64>,
    uncertainty: Arc<Mutex<Option<DurableRemoteQueuedSteerUncertainty>>>,
}

impl PartialEq for RemoteSteerReservation {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for RemoteSteerReservation {}

static NEXT_REMOTE_STEER_RESERVATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DurablePromptPrivateState {
    pub(crate) session_id: String,
    pub(crate) prompt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent_trigger_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notification_deadline_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) hidden_system_context: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) skill_selections: Vec<crate::skill::v2::SkillSelection>,
    #[serde(
        default,
        skip_serializing_if = "crate::transport::relay_peer::LeasedNativeSkills::is_unbound"
    )]
    pub(crate) leased_native_skills: crate::transport::relay_peer::LeasedNativeSkills,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) leased_skill_context:
        Option<crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext>,
    #[serde(default, skip_serializing_if = "is_false_bool")]
    pub(crate) user_skill_syntax: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) skill_turn_authority: Option<crate::skill::turn_authority::SkillTurnAuthority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) operation_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) initially_queued: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) delivery_phase: Option<DurablePromptDeliveryPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) delivery_provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) delivery_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false_bool")]
    pub(crate) delivery_reconciliation_pending: bool,
    #[serde(default, skip_serializing_if = "is_false_bool")]
    pub(crate) delivery_failure_pending: bool,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub(crate) recovery_generation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_phase: Option<DurablePromptDeliveryPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_steer_uncertainty: Option<DurableRemoteQueuedSteerUncertainty>,
    /// The kernel was about to hand this queued firing to the provider as a
    /// steer. A restart cannot tell whether the provider received it, so the
    /// firing is never replayed.
    #[serde(default, skip_serializing_if = "is_false_bool")]
    local_steer_submitted: bool,
    /// The kernel stopped while this turn waited on a question, so nothing can
    /// answer it after the restart.
    #[serde(default, skip_serializing_if = "is_false_bool")]
    question_wait_stopped: bool,
    /// The occurrences this queued notification prompt delivers, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    queued_notifications: Vec<QueuedNotification>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    inbox_wakes: Vec<u64>,
    /// A person changed the text of this queued notification prompt: the kernel keeps their words
    /// and no longer rewrites the prompt from its occurrences.
    #[serde(default, skip_serializing_if = "is_false_bool")]
    queued_text_edited: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PromptPrivateMetadata {
    agent_trigger_id: Option<String>,
    notification_deadline_ms: Option<u64>,
    source_client_id: Option<String>,
    source_user_id: Option<String>,
    hidden_system_context: String,
    skill_selections: Vec<crate::skill::v2::SkillSelection>,
    leased_native_skills: crate::transport::relay_peer::LeasedNativeSkills,
    leased_skill_context: Option<crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext>,
    user_skill_syntax: bool,
    skill_turn_authority: Option<crate::skill::turn_authority::SkillTurnAuthority>,
    operation_id: Option<String>,
    operation_fingerprint: Option<String>,
    initially_queued: Option<bool>,
    delivery_phase: Option<DurablePromptDeliveryPhase>,
    delivery_provider_run_id: Option<String>,
    delivery_provider_session_id: Option<String>,
    delivery_reconciliation_pending: bool,
    delivery_failure_pending: bool,
    recovery_generation: u32,
    recovery_operation_id: Option<String>,
    recovery_phase: Option<DurablePromptDeliveryPhase>,
    local_steer_submitted: bool,
    question_wait_stopped: bool,
    queued_notifications: Vec<QueuedNotification>,
    inbox_wakes: Vec<u64>,
    queued_text_edited: bool,
}

/// One notification occurrence a queued prompt delivers: what happened, where it
/// came from, and the agent's inbox row that holds it in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QueuedNotification {
    pub(crate) title: String,
    pub(crate) source: String,
    pub(crate) sequence: u64,
    #[serde(default)]
    pub(crate) arrived_at_ms: Option<u64>,
    /// A queue-mode subscription's trusted prompt. A steer or a workflow's
    /// responsible-agent delivery always keeps its own prompt.
    #[serde(default)]
    pub(crate) batch_instruction: Option<String>,
    /// The notification operation whose receipt this prompt delivers; the prompt-state
    /// commit that carries the occurrence binds that receipt in the same transaction.
    pub(crate) operation_id: String,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn is_false_bool(value: &bool) -> bool {
    !*value
}

impl DurablePromptPrivateState {
    pub(crate) fn from_prompt(session_id: &str, prompt: &PromptQueueItem) -> Option<Self> {
        let remote_steer_uncertainty = prompt
            .remote_steer_reservation
            .uncertainty
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let metadata = prompt
            .private_metadata
            .as_deref()
            .cloned()
            .unwrap_or_default();
        (prompt.private_metadata.is_some() || remote_steer_uncertainty.is_some()).then(|| Self {
            session_id: session_id.to_string(),
            prompt_id: prompt.id.clone(),
            agent_trigger_id: metadata.agent_trigger_id,
            notification_deadline_ms: metadata.notification_deadline_ms,
            source_client_id: metadata.source_client_id,
            source_user_id: metadata.source_user_id,
            hidden_system_context: metadata.hidden_system_context,
            skill_selections: metadata.skill_selections,
            leased_native_skills: metadata.leased_native_skills,
            leased_skill_context: metadata.leased_skill_context,
            user_skill_syntax: metadata.user_skill_syntax,
            skill_turn_authority: metadata.skill_turn_authority,
            operation_id: metadata.operation_id,
            operation_fingerprint: metadata.operation_fingerprint,
            initially_queued: metadata.initially_queued,
            delivery_phase: metadata.delivery_phase,
            delivery_provider_run_id: metadata.delivery_provider_run_id,
            delivery_provider_session_id: metadata.delivery_provider_session_id,
            delivery_reconciliation_pending: metadata.delivery_reconciliation_pending,
            delivery_failure_pending: metadata.delivery_failure_pending,
            recovery_generation: metadata.recovery_generation,
            recovery_operation_id: metadata.recovery_operation_id,
            recovery_phase: metadata.recovery_phase,
            remote_steer_uncertainty,
            local_steer_submitted: metadata.local_steer_submitted,
            question_wait_stopped: metadata.question_wait_stopped,
            queued_notifications: metadata.queued_notifications,
            inbox_wakes: metadata.inbox_wakes,
            queued_text_edited: metadata.queued_text_edited,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPromptSubmission {
    pending_prompt_id: String,
    source_attachment_id: String,
    target_agent_id: String,
    prompt: String,
    attachments: Vec<PromptAttachment>,
    created_at_ms: u64,
    updated_at_ms: u64,
    #[serde(default, skip_serializing, skip_deserializing)]
    private_metadata: Option<Box<PromptPrivateMetadata>>,
    #[serde(skip)]
    remote_steer_reservation: RemoteSteerReservation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    launch_wait: Option<String>,
    prompt_origin: PromptOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_author: Option<String>,
    external_provider: Option<String>,
    external_provider_session_id: Option<String>,
    external_provider_turn_id: Option<String>,
    workflow_run_id: Option<String>,
    workflow_node_run_id: Option<String>,
}

impl PendingPromptSubmission {
    fn from_prompt_queue_item(
        pending_prompt_id: impl Into<String>,
        prompt: PromptQueueItem,
    ) -> Self {
        let now = crate::session::unix_epoch_ms();
        Self {
            pending_prompt_id: pending_prompt_id.into(),
            source_attachment_id: prompt.source_attachment_id,
            target_agent_id: prompt.target_agent_id,
            prompt: prompt.prompt,
            attachments: prompt.attachments,
            created_at_ms: now,
            updated_at_ms: now,
            private_metadata: prompt.private_metadata,
            remote_steer_reservation: prompt.remote_steer_reservation,
            launch_wait: prompt.launch_wait,
            prompt_origin: prompt.prompt_origin,
            prompt_author: prompt.prompt_author,
            external_provider: prompt.external_provider,
            external_provider_session_id: prompt.external_provider_session_id,
            external_provider_turn_id: prompt.external_provider_turn_id,
            workflow_run_id: prompt.workflow_run_id,
            workflow_node_run_id: prompt.workflow_node_run_id,
        }
    }

    fn into_queue_item(self) -> PromptQueueItem {
        PromptQueueItem {
            id: self.pending_prompt_id.clone(),
            pending_prompt_id: Some(self.pending_prompt_id),
            source_attachment_id: self.source_attachment_id,
            target_agent_id: self.target_agent_id,
            prompt: self.prompt,
            attachments: self.attachments,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            private_metadata: self.private_metadata,
            remote_steer_reservation: self.remote_steer_reservation,
            status: PromptStatus::Queued,
            launch_wait: self.launch_wait,
            prompt_origin: self.prompt_origin,
            prompt_author: self.prompt_author,
            external_provider: self.external_provider,
            external_provider_session_id: self.external_provider_session_id,
            external_provider_turn_id: self.external_provider_turn_id,
            workflow_run_id: self.workflow_run_id,
            workflow_node_run_id: self.workflow_node_run_id,
        }
    }
}

impl PromptQueueItem {
    pub fn new(
        id: impl Into<String>,
        source_attachment_id: impl Into<String>,
        target_agent_id: impl Into<String>,
        prompt: impl Into<String>,
        status: PromptStatus,
    ) -> Self {
        let now = crate::session::unix_epoch_ms();
        Self {
            id: id.into(),
            pending_prompt_id: None,
            source_attachment_id: source_attachment_id.into(),
            target_agent_id: target_agent_id.into(),
            prompt: prompt.into(),
            attachments: Vec::new(),
            created_at_ms: now,
            updated_at_ms: now,
            private_metadata: None,
            remote_steer_reservation: RemoteSteerReservation::default(),
            status,
            launch_wait: None,
            prompt_origin: PromptOrigin::Chariox,
            prompt_author: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            workflow_run_id: None,
            workflow_node_run_id: None,
        }
    }

    pub fn external_observed_running(
        provider: impl Into<String>,
        provider_session_id: impl Into<String>,
        provider_turn_id: impl Into<String>,
        target_agent_id: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        let external_observed_id = crate::history::ExternalProviderObservedId {
            provider: provider.into(),
            provider_session_id: provider_session_id.into(),
            provider_turn_id: provider_turn_id.into(),
        };
        let id = crate::history::external_provider_observed_merge_key(
            &external_observed_id.provider,
            &external_observed_id.provider_session_id,
            &external_observed_id.provider_turn_id,
        );
        let prompt = Self::new(
            id,
            format!("external:{}", external_observed_id.provider),
            target_agent_id,
            prompt,
            PromptStatus::Running,
        )
        .with_prompt_origin(PromptOrigin::External)
        .with_user_skill_syntax(true);
        prompt.with_external_observed_id(external_observed_id)
    }

    pub fn with_attachments(mut self, attachments: Vec<PromptAttachment>) -> Self {
        self.attachments = attachments;
        self
    }

    pub(crate) fn with_prompt_text(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }

    /// MP-08 / MP-10 / MP-11: bind source identities before queue admission.
    /// Public prompt JSON cannot supply or restore this private authority.
    /// Set only by authenticated terminal admission or official native user observation.
    pub(crate) fn with_user_skill_syntax(mut self, selected_by_user: bool) -> Self {
        if selected_by_user {
            self.private_metadata
                .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
                .user_skill_syntax = true;
        }
        self
    }
    pub(crate) fn user_skill_syntax(&self) -> bool {
        self.private_metadata
            .as_deref()
            .is_some_and(|metadata| metadata.user_skill_syntax)
    }
    // MP-08 / MP-10 / MP-11: admitted worker instructions share prompt durability.
    pub(crate) fn with_leased_skills(
        mut self,
        selections: crate::transport::relay_peer::LeasedNativeSkills,
        context: Option<crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext>,
    ) -> Self {
        if !selections.is_unbound() || context.is_some() {
            let metadata = self.private_metadata.get_or_insert_with(Default::default);
            metadata.leased_native_skills = selections;
            metadata.leased_skill_context = context;
        }
        self
    }
    pub(crate) fn leased_native_skills(&self) -> crate::transport::relay_peer::LeasedNativeSkills {
        self.private_metadata
            .as_ref()
            .map(|metadata| metadata.leased_native_skills.clone())
            .unwrap_or_default()
    }
    /// Runtime tools act for the home turn; backing prompt IDs only route worker delivery.
    pub(crate) fn runtime_origin_prompt_id(
        &self,
        leased: bool,
    ) -> Result<&str, crate::error::DaemonError> {
        match self.leased_skill_context() {
            None if !leased => Ok(self.id()),
            context => context.and_then(|context| context.home_prompt_id.as_deref()).filter(|id| !id.is_empty())
                .ok_or_else(|| crate::error::DaemonError::LocalTransport {
                    operation: "resolve runtime turn origin",
                    message: "leased prompt has no authoritative home origin; resubmit it from the home kernel".into(),
                }),
        }
    }

    pub(crate) fn leased_skill_context(
        &self,
    ) -> Option<&crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.leased_skill_context.as_ref())
    }

    pub(crate) fn with_skill_selections(
        mut self,
        selections: Vec<crate::skill::v2::SkillSelection>,
    ) -> Self {
        if !selections.is_empty() || self.private_metadata.is_some() {
            self.private_metadata
                .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
                .skill_selections = selections;
        }
        self
    }

    pub(crate) fn skill_selections(&self) -> &[crate::skill::v2::SkillSelection] {
        self.private_metadata
            .as_deref()
            .map(|metadata| metadata.skill_selections.as_slice())
            .unwrap_or(&[])
    }

    pub fn with_hidden_system_context(mut self, hidden_system_context: impl Into<String>) -> Self {
        let hidden_system_context = hidden_system_context.into();
        if hidden_system_context.is_empty() {
            if let Some(metadata) = self.private_metadata.as_mut() {
                metadata.hidden_system_context.clear();
                if metadata.skill_selections.is_empty()
                    && metadata.leased_native_skills.is_unbound()
                    && metadata.leased_skill_context.is_none()
                    && !metadata.user_skill_syntax
                    && metadata.skill_turn_authority.is_none()
                    && metadata.operation_id.is_none()
                    && metadata.agent_trigger_id.is_none()
                    && metadata.delivery_phase.is_none()
                    && metadata.recovery_operation_id.is_none()
                    && metadata.source_client_id.is_none()
                    && metadata.source_user_id.is_none()
                {
                    self.private_metadata = None;
                }
            }
            return self;
        }
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .hidden_system_context = hidden_system_context;
        self
    }

    pub(crate) fn with_skill_turn_authority(
        mut self,
        authority: crate::skill::turn_authority::SkillTurnAuthority,
    ) -> Self {
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .skill_turn_authority = Some(authority);
        self
    }
    pub(crate) fn skill_turn_authority(
        &self,
    ) -> Option<&crate::skill::turn_authority::SkillTurnAuthority> {
        self.private_metadata
            .as_deref()
            .and_then(|metadata| metadata.skill_turn_authority.as_ref())
    }

    pub(crate) fn with_source_attribution(
        mut self,
        source_client_id: impl Into<String>,
        source_user_id: impl Into<String>,
    ) -> Self {
        let metadata = self
            .private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()));
        metadata.source_client_id = Some(source_client_id.into());
        metadata.source_user_id = Some(source_user_id.into());
        self
    }

    /// MP-08/MP-11: attribute the submitting attachment.
    pub(crate) fn with_source_attachment(
        self,
        attachment: &crate::attachment::RuntimeAttachment,
    ) -> Self {
        self.with_source_attribution(attachment.client_id(), attachment.owner_user_id())
    }

    /// Only notification admission sets this; prompt text and client edits cannot extend it.
    pub(crate) fn with_notification_deadline(mut self, deadline_ms: u64) -> Self {
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .notification_deadline_ms = Some(deadline_ms);
        self
    }

    pub(crate) fn notification_expired(&self, now: u64) -> bool {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.notification_deadline_ms)
            .is_some_and(|deadline| now >= deadline)
    }

    pub(crate) fn with_agent_trigger(mut self, trigger_id: impl Into<String>) -> Self {
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .agent_trigger_id = Some(trigger_id.into());
        self
    }

    pub(crate) fn with_queued_notification(mut self, notification: QueuedNotification) -> Self {
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .queued_notifications = vec![notification];
        self
    }

    /// The notification occurrences this prompt delivers, oldest first.
    pub(crate) fn queued_notifications(&self) -> &[QueuedNotification] {
        self.private_metadata
            .as_deref()
            .map_or(&[], |metadata| metadata.queued_notifications.as_slice())
    }

    /// Adds one more occurrence to this queued prompt: its text
    /// becomes `text` and it now expires with the newest occurrence.
    pub(crate) fn join_queued_notification(
        &mut self,
        notification: QueuedNotification,
        deadline_ms: u64,
        text: String,
    ) {
        let metadata = self
            .private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()));
        metadata.queued_notifications.push(notification);
        metadata.notification_deadline_ms = Some(
            metadata
                .notification_deadline_ms
                .map_or(deadline_ms, |deadline| deadline.max(deadline_ms)),
        );
        self.prompt = text;
        self.updated_at_ms = crate::session::unix_epoch_ms();
    }

    /// A person edited this queued notification prompt's text: it is theirs now, and no later
    /// occurrence or wake rewrites it.
    pub(crate) fn mark_queued_text_edited(&mut self) {
        if self.queued_notifications().is_empty() {
            return;
        }
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .queued_text_edited = true;
    }

    pub(crate) fn queued_text_edited(&self) -> bool {
        self.private_metadata
            .as_deref()
            .is_some_and(|metadata| metadata.queued_text_edited)
    }

    pub(crate) fn with_inbox_wakes(mut self, sequences: Vec<u64>) -> Self {
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .inbox_wakes = sequences;
        self
    }

    pub(crate) fn inbox_wakes(&self) -> &[u64] {
        self.private_metadata
            .as_deref()
            .map_or(&[], |metadata| metadata.inbox_wakes.as_slice())
    }

    pub(crate) fn with_durable_operation(
        mut self,
        operation_id: impl Into<String>,
        fingerprint: impl Into<String>,
    ) -> Self {
        let metadata = self
            .private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()));
        metadata.operation_id = Some(operation_id.into());
        metadata.operation_fingerprint = Some(fingerprint.into());
        self
    }

    pub(crate) const TRIGGER_RESEND_OPERATION_PREFIX: &str = "agent-trigger-resend:";

    /// The operation identity is kept in the admission event, so recovery can
    /// consume the original resend only when its replacement was admitted.
    pub(crate) fn with_trigger_resend(self, original_prompt_id: &str) -> Self {
        self.with_durable_operation(
            format!(
                "{}{original_prompt_id}",
                Self::TRIGGER_RESEND_OPERATION_PREFIX
            ),
            original_prompt_id,
        )
    }

    pub(crate) fn trigger_resend_of(&self) -> Option<&str> {
        self.agent_trigger_id()?;
        self.durable_operation_id()?
            .strip_prefix(Self::TRIGGER_RESEND_OPERATION_PREFIX)
    }

    pub fn with_workflow_context(
        mut self,
        workflow_run_id: impl Into<String>,
        workflow_node_run_id: impl Into<String>,
    ) -> Self {
        self.workflow_run_id = Some(workflow_run_id.into());
        self.workflow_node_run_id = Some(workflow_node_run_id.into());
        self
    }

    pub fn with_prompt_origin(mut self, prompt_origin: PromptOrigin) -> Self {
        self.prompt_origin = prompt_origin;
        self
    }

    pub fn with_external_observed_id(
        mut self,
        external_observed_id: crate::history::ExternalProviderObservedId,
    ) -> Self {
        self.external_provider = Some(external_observed_id.provider);
        self.external_provider_session_id = Some(external_observed_id.provider_session_id);
        self.external_provider_turn_id = Some(external_observed_id.provider_turn_id);
        self
    }

    /// Drops what the prompt told the agent; the identity and status remain.
    pub(crate) fn clear_instruction(&mut self) {
        self.prompt.clear();
        self.attachments.clear();
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self.pending_prompt_id = None;
        self
    }

    pub fn with_pending_prompt_id(mut self, pending_prompt_id: impl Into<String>) -> Self {
        let pending_prompt_id = pending_prompt_id.into();
        self.id = pending_prompt_id.clone();
        self.pending_prompt_id = Some(pending_prompt_id);
        self.status = PromptStatus::Queued;
        self
    }

    pub fn into_pending_queue_item(self, pending_prompt_id: impl Into<String>) -> Self {
        PendingPromptSubmission::from_prompt_queue_item(pending_prompt_id, self).into_queue_item()
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn pending_prompt_id(&self) -> Option<&str> {
        self.pending_prompt_id.as_deref()
    }

    pub fn source_attachment_id(&self) -> &str {
        &self.source_attachment_id
    }

    pub fn target_agent_id(&self) -> &str {
        &self.target_agent_id
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn attachments(&self) -> &[PromptAttachment] {
        &self.attachments
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    pub(crate) fn reserve_remote_steer(&self) -> Option<u64> {
        if self.remote_steer_outcome_uncertainty().is_some() {
            return None;
        }
        let reservation = NEXT_REMOTE_STEER_RESERVATION.fetch_add(1, Ordering::Relaxed);
        self.remote_steer_reservation
            .id
            .compare_exchange(0, reservation, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| reservation)
    }

    pub(crate) fn remote_steer_reserved(&self) -> bool {
        // Local trigger steers share the queue reservation. Their durable
        // delivery hold outlives the mailbox claim if settlement cannot commit.
        self.local_steer_submitted()
            || self.remote_steer_reservation.id.load(Ordering::Acquire) != 0
            || self.remote_steer_outcome_uncertainty().is_some()
    }

    pub(crate) fn remote_steer_reservation_matches(&self, reservation: u64) -> bool {
        self.remote_steer_reservation.id.load(Ordering::Acquire) == reservation
    }

    pub(crate) fn release_remote_steer(&self, reservation: u64) -> bool {
        self.remote_steer_reservation
            .id
            .compare_exchange(reservation, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Keeps the existing session, worker and lease binding fields explicit without a second protocol representation."
    )]
    pub(crate) fn mark_remote_steer_outcome_uncertain(
        &self,
        reservation: u64,
        target_home_prompt_id: String,
        worker_provider_run_id: Option<String>,
        worker_kernel_id: String,
        worker_machine_id: String,
        execution_lease_id: String,
        leased_agent_id: String,
    ) -> bool {
        if !self.remote_steer_reservation_matches(reservation) {
            return false;
        }
        *self
            .remote_steer_reservation
            .uncertainty
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(DurableRemoteQueuedSteerUncertainty {
                target_home_prompt_id,
                worker_provider_run_id,
                worker_kernel_id,
                worker_machine_id,
                execution_lease_id,
                leased_agent_id,
            });
        true
    }

    pub(crate) fn clear_remote_steer_outcome_uncertainty(
        &self,
        reservation: u64,
    ) -> Option<(String, Option<String>, String, String, String, String)> {
        if !self.remote_steer_reservation_matches(reservation) {
            return None;
        }
        self.remote_steer_reservation
            .uncertainty
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .map(|uncertainty| {
                (
                    uncertainty.target_home_prompt_id,
                    uncertainty.worker_provider_run_id,
                    uncertainty.worker_kernel_id,
                    uncertainty.worker_machine_id,
                    uncertainty.execution_lease_id,
                    uncertainty.leased_agent_id,
                )
            })
    }

    pub(crate) fn clear_remote_steer_outcome_uncertainty_for_receipt(
        &self,
        target_home_prompt_id: &str,
        worker_provider_run_id: &str,
        worker_kernel_id: &str,
        worker_machine_id: &str,
        execution_lease_id: &str,
        leased_agent_id: &str,
    ) -> bool {
        let mut uncertainty = self
            .remote_steer_reservation
            .uncertainty
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(current) = uncertainty.as_ref() else {
            return false;
        };
        if current.target_home_prompt_id != target_home_prompt_id
            || current.worker_provider_run_id.as_deref() != Some(worker_provider_run_id)
            || current.worker_kernel_id != worker_kernel_id
            || current.worker_machine_id != worker_machine_id
            || current.execution_lease_id != execution_lease_id
            || current.leased_agent_id != leased_agent_id
        {
            return false;
        }
        uncertainty.take();
        self.remote_steer_reservation.id.store(0, Ordering::Release);
        true
    }

    pub(crate) fn remote_steer_outcome_uncertainty(
        &self,
    ) -> Option<(String, Option<String>, String, String, String, String)> {
        self.remote_steer_reservation
            .uncertainty
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|uncertainty| {
                (
                    uncertainty.target_home_prompt_id.clone(),
                    uncertainty.worker_provider_run_id.clone(),
                    uncertainty.worker_kernel_id.clone(),
                    uncertainty.worker_machine_id.clone(),
                    uncertainty.execution_lease_id.clone(),
                    uncertainty.leased_agent_id.clone(),
                )
            })
    }

    fn restore_remote_steer_outcome_uncertainty(
        &self,
        uncertainty: Option<DurableRemoteQueuedSteerUncertainty>,
    ) {
        if let Some(uncertainty) = uncertainty {
            *self
                .remote_steer_reservation
                .uncertainty
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(uncertainty);
        }
    }

    pub fn hidden_system_context(&self) -> &str {
        self.private_metadata
            .as_ref()
            .map(|metadata| metadata.hidden_system_context.as_str())
            .unwrap_or_default()
    }

    pub(crate) fn source_client_id(&self) -> Option<&str> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.source_client_id.as_deref())
    }

    pub(crate) fn agent_trigger_id(&self) -> Option<&str> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.agent_trigger_id.as_deref())
    }

    pub(crate) fn source_user_id(&self) -> Option<&str> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.source_user_id.as_deref())
    }

    pub(crate) fn durable_operation_id(&self) -> Option<&str> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.operation_id.as_deref())
    }

    pub(crate) fn durable_operation_fingerprint(&self) -> Option<&str> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.operation_fingerprint.as_deref())
    }

    pub(crate) fn durable_initially_queued(&self) -> Option<bool> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.initially_queued)
    }

    pub(crate) fn set_durable_initially_queued(&mut self, queued: bool) {
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .initially_queued = Some(queued);
    }

    pub(crate) fn durable_delivery_phase(&self) -> Option<DurablePromptDeliveryPhase> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.delivery_phase)
    }

    /// Returns true while a durable prompt has been accepted by the kernel but has not
    /// yet been acknowledged by its provider. Provider output from a previous turn must
    /// not be allowed to settle this prompt during that window.
    pub(crate) fn delivery_pending(&self) -> bool {
        self.status == PromptStatus::Dispatching
            || matches!(
                self.durable_delivery_phase(),
                Some(
                    DurablePromptDeliveryPhase::Accepted | DurablePromptDeliveryPhase::Dispatching
                )
            )
    }

    pub(crate) fn durable_delivery_provider_run_id(&self) -> Option<&str> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.delivery_provider_run_id.as_deref())
    }

    pub(crate) fn durable_delivery_provider_session_id(&self) -> Option<&str> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.delivery_provider_session_id.as_deref())
    }

    pub(crate) fn durable_delivery_failure_pending(&self) -> bool {
        self.private_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.delivery_failure_pending)
    }

    pub(crate) fn set_durable_delivery_failure_pending(&mut self, pending: bool) {
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .delivery_failure_pending = pending;
    }

    pub(crate) fn question_wait_stopped(&self) -> bool {
        self.private_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.question_wait_stopped)
    }

    pub(crate) fn set_question_wait_stopped(&mut self) {
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .question_wait_stopped = true;
    }

    pub(crate) fn durable_delivery_reconciliation_pending(&self) -> bool {
        self.private_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.delivery_reconciliation_pending)
    }

    pub(crate) fn set_durable_delivery_reconciliation_pending(&mut self, pending: bool) {
        self.private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
            .delivery_reconciliation_pending = pending;
    }

    pub(crate) fn set_durable_delivery(
        &mut self,
        phase: DurablePromptDeliveryPhase,
        provider_run_id: Option<String>,
        provider_session_id: Option<String>,
    ) {
        let metadata = self
            .private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()));
        metadata.delivery_phase = Some(phase);
        metadata.delivery_provider_run_id = provider_run_id;
        metadata.delivery_provider_session_id = provider_session_id;
        if phase == DurablePromptDeliveryPhase::Delivered {
            metadata.delivery_failure_pending = false;
            metadata.delivery_reconciliation_pending = false;
        }
    }

    pub(crate) fn durable_recovery_operation_id(&self) -> Option<&str> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.recovery_operation_id.as_deref())
    }

    pub(crate) fn local_steer_submitted(&self) -> bool {
        self.private_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.local_steer_submitted)
    }

    pub(crate) fn set_local_steer_submitted(&mut self, submitted: bool) {
        if submitted || self.private_metadata.is_some() {
            self.private_metadata
                .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()))
                .local_steer_submitted = submitted;
        }
    }

    pub(crate) fn durable_recovery_phase(&self) -> Option<DurablePromptDeliveryPhase> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.recovery_phase)
    }

    pub(crate) fn begin_durable_recovery_operation(&mut self) -> String {
        let metadata = self
            .private_metadata
            .get_or_insert_with(|| Box::new(PromptPrivateMetadata::default()));
        if metadata.recovery_phase == Some(DurablePromptDeliveryPhase::Accepted) {
            if let Some(operation_id) = metadata.recovery_operation_id.as_ref() {
                return operation_id.clone();
            }
        }
        metadata.recovery_generation = metadata.recovery_generation.saturating_add(1).max(1);
        let operation_id = format!(
            "chariox-recovery:{}:{}",
            self.id, metadata.recovery_generation
        );
        metadata.recovery_operation_id = Some(operation_id.clone());
        metadata.recovery_phase = Some(DurablePromptDeliveryPhase::Accepted);
        operation_id
    }

    pub(crate) fn mark_durable_recovery_phase(
        &mut self,
        operation_id: &str,
        phase: DurablePromptDeliveryPhase,
    ) -> bool {
        let Some(metadata) = self.private_metadata.as_mut() else {
            return false;
        };
        if metadata.recovery_operation_id.as_deref() != Some(operation_id) {
            return false;
        }
        metadata.recovery_phase = Some(phase);
        true
    }

    pub(crate) fn restore_durable_private_state(&mut self, private: &DurablePromptPrivateState) {
        if private.hidden_system_context.is_empty()
            && private.agent_trigger_id.is_none()
            && private.notification_deadline_ms.is_none()
            && private.skill_selections.is_empty()
            && !private.user_skill_syntax
            && private.skill_turn_authority.is_none()
            && private.leased_native_skills.is_unbound()
            && private.leased_skill_context.is_none()
            && private.operation_id.is_none()
            && private.delivery_phase.is_none()
            && !private.delivery_reconciliation_pending
            && !private.delivery_failure_pending
            && private.recovery_operation_id.is_none()
            && private.source_client_id.is_none()
            && private.source_user_id.is_none()
            && private.remote_steer_uncertainty.is_none()
            && !private.local_steer_submitted
            && !private.question_wait_stopped
        {
            self.private_metadata = None;
            self.restore_remote_steer_outcome_uncertainty(private.remote_steer_uncertainty.clone());
            return;
        }
        self.private_metadata = Some(Box::new(PromptPrivateMetadata {
            agent_trigger_id: private.agent_trigger_id.clone(),
            notification_deadline_ms: private.notification_deadline_ms,
            source_client_id: private.source_client_id.clone(),
            source_user_id: private.source_user_id.clone(),
            hidden_system_context: private.hidden_system_context.clone(),
            skill_selections: private.skill_selections.clone(),
            leased_native_skills: private.leased_native_skills.clone(),
            leased_skill_context: private.leased_skill_context.clone(),
            user_skill_syntax: private.user_skill_syntax,
            skill_turn_authority: private.skill_turn_authority.clone(),
            operation_id: private.operation_id.clone(),
            operation_fingerprint: private.operation_fingerprint.clone(),
            initially_queued: private.initially_queued,
            delivery_phase: private.delivery_phase,
            delivery_provider_run_id: private.delivery_provider_run_id.clone(),
            delivery_provider_session_id: private.delivery_provider_session_id.clone(),
            delivery_reconciliation_pending: private.delivery_reconciliation_pending,
            delivery_failure_pending: private.delivery_failure_pending,
            recovery_generation: private.recovery_generation,
            recovery_operation_id: private.recovery_operation_id.clone(),
            recovery_phase: private.recovery_phase,
            local_steer_submitted: private.local_steer_submitted,
            question_wait_stopped: private.question_wait_stopped,
            queued_notifications: private.queued_notifications.clone(),
            inbox_wakes: private.inbox_wakes.clone(),
            queued_text_edited: private.queued_text_edited,
        }));
        self.restore_remote_steer_outcome_uncertainty(private.remote_steer_uncertainty.clone());
    }

    pub fn status(&self) -> PromptStatus {
        self.status
    }

    pub(crate) fn with_prompt_author(mut self, name: &str) -> Self {
        self.prompt_author = Some(name.to_string());
        self
    }

    pub(crate) fn prompt_author(&self) -> Option<&str> {
        self.prompt_author.as_deref()
    }

    pub fn prompt_origin(&self) -> PromptOrigin {
        self.prompt_origin
    }

    pub fn is_external(&self) -> bool {
        self.prompt_origin == PromptOrigin::External
    }

    pub fn is_chariox_owned(&self) -> bool {
        self.prompt_origin.is_chariox_prompt()
            || self.prompt_origin == PromptOrigin::CompletionCheck
    }

    pub fn external_observed_id(&self) -> Option<crate::history::ExternalProviderObservedId> {
        if !self.is_external() {
            return None;
        }
        Some(crate::history::ExternalProviderObservedId {
            provider: self.external_provider.clone()?,
            provider_session_id: self.external_provider_session_id.clone()?,
            provider_turn_id: self.external_provider_turn_id.clone()?,
        })
    }

    pub fn external_provider(&self) -> Option<&str> {
        self.external_provider.as_deref()
    }

    pub fn external_provider_session_id(&self) -> Option<&str> {
        self.external_provider_session_id.as_deref()
    }

    pub fn external_provider_turn_id(&self) -> Option<&str> {
        self.external_provider_turn_id.as_deref()
    }

    pub fn workflow_run_id(&self) -> Option<&str> {
        self.workflow_run_id.as_deref()
    }

    pub fn workflow_node_run_id(&self) -> Option<&str> {
        self.workflow_node_run_id.as_deref()
    }

    pub fn set_status(&mut self, status: PromptStatus) {
        if self.status != status {
            self.status = status;
            self.updated_at_ms = crate::session::unix_epoch_ms();
        }
        if status != PromptStatus::Queued {
            self.launch_wait = None;
        }
    }

    pub(crate) fn launch_wait(&self) -> Option<&str> {
        self.launch_wait.as_deref()
    }

    pub(crate) fn set_launch_wait(&mut self, wait: Option<String>) {
        self.launch_wait = wait;
    }

    /// Keep the person's turn ahead of automated work while admission is held.
    pub(crate) fn comes_from_person(&self) -> bool {
        self.prompt_origin == PromptOrigin::Chariox
            && self.workflow_run_id.is_none()
            && self.source_client_id().is_some_and(|client| {
                !client.starts_with("agent:") && !client.starts_with("metaagent:")
            })
    }

    pub fn set_prompt(&mut self, prompt: impl Into<String>) {
        self.prompt = prompt.into();
        self.updated_at_ms = crate::session::unix_epoch_ms();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSubmissionOutcome<T = PromptQueueItem> {
    Started { prompt: T },
    Queued { prompt: T },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentPromptState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::session) active_prompt: Option<PromptQueueItem>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub(in crate::session) queued_prompts: VecDeque<PromptQueueItem>,
}

impl AgentPromptState {
    pub(in crate::session) fn from_parts(
        active_prompt: Option<PromptQueueItem>,
        queued_prompts: VecDeque<PromptQueueItem>,
    ) -> Self {
        Self {
            active_prompt,
            queued_prompts,
        }
    }

    pub fn active_prompt(&self) -> Option<&PromptQueueItem> {
        self.active_prompt.as_ref()
    }

    pub fn queued_prompts(&self) -> &VecDeque<PromptQueueItem> {
        &self.queued_prompts
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCompletion<T = PromptQueueItem> {
    pub completed: T,
    pub started_next: Option<T>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCancellation<T = PromptQueueItem> {
    pub prompt: T,
    pub started_next: Option<T>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_v2_typed_sources_survive_private_restore_without_public_prompt_authority() {
        let selection = crate::skill::v2::SkillSelection::Chariox {
            skill_id: "stable-skill".into(),
            name: "review".into(),
            catalog_revision: 19,
        };
        let prompt =
            PromptQueueItem::new("p", "a", "agent", "$review inspect", PromptStatus::Queued)
                .with_skill_selections(vec![selection.clone()])
                .with_user_skill_syntax(true)
                .with_hidden_system_context("temporary context")
                .with_hidden_system_context("");
        let authority = crate::skill::turn_authority::SkillTurnAuthority {
            catalog_revision: 7,
            project_id: "project".into(),
            provider_run_id: "actual-run".into(),
            context_generation: "actual-turn".into(),
            explicit_skill_ids: vec!["skill-1".into()],
            body_budget_tokens: 2048,
            reserved_body_tokens: std::collections::BTreeMap::new(),
            pending_loads: vec![],
            prompt_delivery_skill_ids: Default::default(),
            remote_roots: Default::default(),
            native_tool_loads: Default::default(),
            compaction_revision: None,
        };
        let prompt = prompt.with_skill_turn_authority(authority.clone());
        assert_eq!(prompt.skill_selections(), std::slice::from_ref(&selection));
        let mut public = serde_json::to_value(&prompt).unwrap();
        assert!(public.get("skill_selections").is_none());
        assert!(public.get("skill_turn_authority").is_none());
        assert!(public.get("user_skill_syntax").is_none());
        public["user_skill_syntax"] = serde_json::json!(true);
        public["skill_turn_authority"] = serde_json::to_value(&authority).unwrap();
        assert!(!public.to_string().contains("stable-skill"));
        // A caller's untrusted public JSON must never mint source-bound refs.
        public["skill_selections"] = serde_json::to_value(vec![selection.clone()]).unwrap();
        let mut restored: PromptQueueItem = serde_json::from_value(public).unwrap();
        assert!(restored.skill_selections().is_empty());
        assert!(restored.skill_turn_authority().is_none());
        assert!(!restored.user_skill_syntax());
        let durable = DurablePromptPrivateState::from_prompt("session", &prompt).unwrap();
        let persisted = serde_json::to_vec(&durable).unwrap();
        restored.restore_durable_private_state(&serde_json::from_slice(&persisted).unwrap());
        assert_eq!(restored.skill_selections(), &[selection]);
        assert_eq!(restored.skill_turn_authority(), Some(&authority));
        assert!(restored.user_skill_syntax());
        assert_eq!(restored.hidden_system_context(), "");
        let promoted = restored.into_pending_queue_item("promoted");
        assert_eq!(promoted.skill_selections(), prompt.skill_selections());
    }

    // MP-08 / MP-10 / MP-11: no process-wide staging limit can evict admitted work.
    #[test]
    fn leased_native_private_queue_survives_restart_beyond_256_prompts() {
        let native = crate::transport::relay_peer::LeasedNativeSkills {
            catalog_revision: 23,
            selections: vec![crate::skill::v2::SkillSelection::Native {
                entry_id: "entry".into(),
                name: "review".into(),
                provider: "codex".into(),
                account_profile: "profile".into(),
                execution_id: "execution".into(),
                inventory_revision: "inventory".into(),
            }],
        };
        let mut journal = Vec::new();
        for index in 0..258 {
            let prompt = PromptQueueItem::new(
                format!("queued-{index}"),
                "attachment",
                "agent",
                "review task",
                PromptStatus::Queued,
            )
            .with_leased_skills(native.clone(), None);
            let public = serde_json::to_value(&prompt).unwrap();
            assert!(public.get("leased_native_skills").is_none());
            journal.push((
                public,
                serde_json::to_value(
                    DurablePromptPrivateState::from_prompt("session", &prompt).unwrap(),
                )
                .unwrap(),
            ));
        }
        for (mut public, private) in journal.into_iter().rev() {
            public["leased_native_skills"] = serde_json::to_value(&native).unwrap();
            let mut recovered: PromptQueueItem = serde_json::from_value(public).unwrap();
            assert!(
                recovered.leased_native_skills().is_unbound(),
                "public JSON cannot mint a worker invocation"
            );
            recovered.restore_durable_private_state(&serde_json::from_value(private).unwrap());
            assert_eq!(
                recovered
                    .into_pending_queue_item("promoted")
                    .leased_native_skills(),
                native
            );
        }
    }

    // MP-08 / MP-10 / MP-11: a body-only leased turn keeps its catalog binding.
    #[test]
    fn body_only_leased_catalog_revision_survives_restart() {
        let native = crate::transport::relay_peer::LeasedNativeSkills {
            catalog_revision: 41,
            selections: vec![],
        };
        let prompt = PromptQueueItem::new("p", "a", "agent", "task", PromptStatus::Queued)
            .with_leased_skills(native.clone(), None);
        let durable = DurablePromptPrivateState::from_prompt("session", &prompt).unwrap();
        let persisted = serde_json::to_vec(&durable).unwrap();
        let mut restored = PromptQueueItem::new("p", "a", "agent", "task", PromptStatus::Queued);
        restored.restore_durable_private_state(&serde_json::from_slice(&persisted).unwrap());
        assert_eq!(restored.leased_native_skills(), native);
    }

    #[test]
    fn prompt_queue_item_does_not_serialize_private_runtime_metadata() {
        let mut item = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "VISIBLE_PROMPT_TOKEN",
            PromptStatus::Queued,
        )
        .with_source_attribution("client-private", "user-private")
        .with_agent_trigger("schedule-private")
        .with_hidden_system_context("HIDDEN_CONTEXT_TOKEN")
        .with_durable_operation("operation-private", "fingerprint-private");
        item.set_durable_initially_queued(true);
        item.set_durable_delivery(
            DurablePromptDeliveryPhase::Delivered,
            Some("provider-run-private".to_string()),
            Some("provider-session-private".to_string()),
        );
        let recovery_operation_id = item.begin_durable_recovery_operation();

        let payload = serde_json::to_string(&item).expect("prompt should serialize");

        assert!(payload.contains("VISIBLE_PROMPT_TOKEN"));
        assert!(!payload.contains("HIDDEN_CONTEXT_TOKEN"));
        assert!(!payload.contains("hidden_system_context"));
        assert!(!payload.contains("operation-private"));
        assert!(!payload.contains("fingerprint-private"));
        assert!(!payload.contains("durable_initially_queued"));
        assert!(!payload.contains("private_metadata"));
        assert!(!payload.contains("client-private"));
        assert!(!payload.contains("user-private"));
        assert!(!payload.contains("schedule-private"));
        assert!(!payload.contains("provider-run-private"));
        assert!(!payload.contains("provider-session-private"));
        assert!(!payload.contains(&recovery_operation_id));
    }

    #[test]
    fn remote_steer_reservation_is_clone_shared_exact_and_transient() {
        let prompt = PromptQueueItem::new(
            "prompt-queued",
            "attachment-1",
            "agent-1",
            "queued prompt",
            PromptStatus::Queued,
        );
        let clone = prompt.clone();
        let before = serde_json::to_value(&prompt).expect("prompt should serialize");
        let reservation = prompt
            .reserve_remote_steer()
            .expect("the first remote steer should own the queue item");

        assert!(clone.remote_steer_reserved());
        assert!(clone.reserve_remote_steer().is_none());
        assert!(!clone.release_remote_steer(reservation + 1));
        assert!(prompt.remote_steer_reservation_matches(reservation));
        assert_eq!(
            serde_json::to_value(&prompt).expect("reserved prompt should serialize"),
            before,
            "the transient steer reservation must not change serialized prompt state"
        );
        let restored: PromptQueueItem =
            serde_json::from_value(before).expect("serialized prompt should deserialize");
        assert!(!restored.remote_steer_reserved());
        assert!(clone.release_remote_steer(reservation));
        assert!(!prompt.remote_steer_reserved());
    }

    #[test]
    fn local_steer_hold_survives_durable_restore_and_stays_private() {
        let mut prompt = PromptQueueItem::new(
            "queued-steer-1",
            "attachment-1",
            "agent-1",
            "queued body",
            PromptStatus::Queued,
        );
        assert!(DurablePromptPrivateState::from_prompt("session-1", &prompt).is_none());
        prompt.set_local_steer_submitted(true);
        let private = DurablePromptPrivateState::from_prompt("session-1", &prompt)
            .expect("the hold is durable private state");
        let private: DurablePromptPrivateState =
            serde_json::from_value(serde_json::to_value(private).unwrap()).unwrap();
        let public_prompt = serde_json::to_value(&prompt).unwrap();
        assert!(public_prompt.get("local_steer_submitted").is_none());
        let mut restored: PromptQueueItem = serde_json::from_value(public_prompt).unwrap();
        assert!(!restored.local_steer_submitted());
        restored.restore_durable_private_state(&private);
        assert!(restored.local_steer_submitted());
        restored.set_local_steer_submitted(false);
        assert!(!restored.local_steer_submitted());
    }

    #[test]
    fn uncertain_remote_steer_survives_durable_restore_and_remains_reserved() {
        let prompt = PromptQueueItem::new(
            "queued-steer-1",
            "attachment-1",
            "agent-1",
            "queued body",
            PromptStatus::Queued,
        );
        let reservation = prompt
            .reserve_remote_steer()
            .expect("steer should reserve before delivery");
        let uncertainty = DurableRemoteQueuedSteerUncertainty {
            target_home_prompt_id: "home-prompt-1".to_string(),
            worker_provider_run_id: Some("worker-run-1".to_string()),
            worker_kernel_id: "worker-kernel-1".to_string(),
            worker_machine_id: "worker-machine-1".to_string(),
            execution_lease_id: "lease-1".to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
        };
        assert!(prompt.mark_remote_steer_outcome_uncertain(
            reservation,
            uncertainty.target_home_prompt_id.clone(),
            uncertainty.worker_provider_run_id.clone(),
            uncertainty.worker_kernel_id.clone(),
            uncertainty.worker_machine_id.clone(),
            uncertainty.execution_lease_id.clone(),
            uncertainty.leased_agent_id.clone(),
        ));
        assert!(prompt.release_remote_steer(reservation));
        assert!(
            prompt.remote_steer_reserved(),
            "dropping the transient reservation must not release an uncertain prompt"
        );

        let private = DurablePromptPrivateState::from_prompt("session-1", &prompt)
            .expect("uncertain state should be included in durable private state");
        let private: DurablePromptPrivateState =
            serde_json::from_value(serde_json::to_value(private).unwrap()).unwrap();
        let public_prompt = serde_json::to_value(&prompt).unwrap();
        assert!(public_prompt.get("remote_steer_uncertainty").is_none());
        let mut restored: PromptQueueItem =
            serde_json::from_value(public_prompt).expect("public prompt should restore");
        assert!(!restored.remote_steer_reserved());
        restored.restore_durable_private_state(&private);

        assert!(restored.remote_steer_reserved());
        assert_eq!(restored.id(), "queued-steer-1");
        assert_eq!(
            restored.remote_steer_outcome_uncertainty(),
            Some((
                uncertainty.target_home_prompt_id,
                uncertainty.worker_provider_run_id,
                uncertainty.worker_kernel_id,
                uncertainty.worker_machine_id,
                uncertainty.execution_lease_id,
                uncertainty.leased_agent_id,
            ))
        );
        assert!(
            restored.reserve_remote_steer().is_none(),
            "restored uncertain prompt must not be retried as a new steer"
        );
    }

    #[test]
    fn prompt_queue_item_deserialization_drops_hidden_system_context() {
        let payload = r#"{
            "id":"prompt-1",
            "source_attachment_id":"attachment-1",
            "target_agent_id":"agent-1",
            "prompt":"VISIBLE_PROMPT_TOKEN",
            "attachments":[],
            "hidden_system_context":"HIDDEN_CONTEXT_TOKEN",
            "status":"Queued",
            "workflow_run_id":null,
            "workflow_node_run_id":null
        }"#;

        let item: PromptQueueItem =
            serde_json::from_str(payload).expect("prompt should deserialize");

        assert_eq!(item.prompt(), "VISIBLE_PROMPT_TOKEN");
        assert_eq!(item.hidden_system_context(), "");
    }

    #[test]
    fn agent_trigger_provenance_survives_private_state_and_queue_promotion() {
        let prompt = PromptQueueItem::new(
            "scheduled-prompt",
            "attachment-1",
            "agent-1",
            "same text",
            PromptStatus::Queued,
        )
        .with_agent_trigger("wait-1")
        .with_hidden_system_context("")
        .into_pending_queue_item("pending-1");
        let private = DurablePromptPrivateState::from_prompt("session-1", &prompt).unwrap();
        let private: DurablePromptPrivateState =
            serde_json::from_value(serde_json::to_value(private).unwrap()).unwrap();
        let mut restored: PromptQueueItem =
            serde_json::from_value(serde_json::to_value(&prompt).unwrap()).unwrap();
        assert_eq!(restored.agent_trigger_id(), None);
        restored.restore_durable_private_state(&private);
        let promoted = restored.with_id("running-1");
        assert_eq!(promoted.agent_trigger_id(), Some("wait-1"));
        assert!(promoted.hidden_system_context().is_empty());
    }

    #[test]
    fn reconciliation_pending_survives_private_state_restore_until_delivery_is_durable() {
        let mut prompt = PromptQueueItem::new(
            "prompt-uncertain",
            "attachment-1",
            "agent-1",
            "same prompt body",
            PromptStatus::Running,
        );
        prompt.set_durable_delivery(DurablePromptDeliveryPhase::Dispatching, None, None);
        prompt.set_durable_delivery_reconciliation_pending(true);

        let private = DurablePromptPrivateState::from_prompt("session-1", &prompt).unwrap();
        let private: DurablePromptPrivateState =
            serde_json::from_value(serde_json::to_value(private).unwrap()).unwrap();
        let mut restored: PromptQueueItem =
            serde_json::from_value(serde_json::to_value(&prompt).unwrap()).unwrap();
        restored.restore_durable_private_state(&private);

        assert_eq!(restored.id(), "prompt-uncertain");
        assert_eq!(
            restored.durable_delivery_phase(),
            Some(DurablePromptDeliveryPhase::Dispatching)
        );
        assert!(restored.durable_delivery_reconciliation_pending());

        restored.set_durable_delivery(
            DurablePromptDeliveryPhase::Delivered,
            Some("worker-run-1".to_string()),
            None,
        );
        assert!(!restored.durable_delivery_reconciliation_pending());
    }

    #[test]
    fn renamed_prompt_origin_restores_and_serializes_canonically() {
        let origin: PromptOrigin =
            serde_json::from_str(r#""arroba""#).expect("renamed prompt origin should restore");

        assert_eq!(origin, PromptOrigin::Chariox);
        assert_eq!(
            serde_json::to_string(&origin).expect("prompt origin should serialize"),
            r#""chariox""#
        );
    }

    #[test]
    fn prompt_queue_item_classifies_prompt_ownership() {
        let chariox_prompt = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "prompt",
            PromptStatus::Queued,
        );
        let external_prompt = chariox_prompt
            .clone()
            .with_prompt_origin(PromptOrigin::External);

        let completion_check = chariox_prompt
            .clone()
            .with_prompt_origin(PromptOrigin::CompletionCheck);
        assert!(
            completion_check.is_chariox_owned(),
            "MP-08/MP-09/MP-10/MP-11 A12: normal dispatch must admit the assessment turn"
        );
        assert!(!completion_check.is_external());
        assert!(chariox_prompt.is_chariox_owned());
        assert!(!chariox_prompt.is_external());
        assert!(external_prompt.is_external());
        assert!(!external_prompt.is_chariox_owned());
        for origin in [
            PromptOrigin::Schedule,
            PromptOrigin::Http,
            PromptOrigin::Notification,
        ] {
            let fired = chariox_prompt.clone().with_prompt_origin(origin);
            assert!(
                fired.is_chariox_owned() && !fired.is_external(),
                "a firing is Chariox's own prompt, so steering and dispatch admit it"
            );
        }
    }

    #[test]
    fn prompt_queue_item_external_observed_running_sets_runtime_identity() {
        let prompt = PromptQueueItem::external_observed_running(
            "codex", "thread-1", "user-1", "agent-1", "run this",
        );

        assert_eq!(prompt.id(), "external:codex:thread-1:user-1");
        assert_eq!(prompt.source_attachment_id(), "external:codex");
        assert_eq!(prompt.target_agent_id(), "agent-1");
        assert_eq!(prompt.prompt(), "run this");
        assert_eq!(prompt.status(), PromptStatus::Running);
        assert!(prompt.is_external());
        assert!(!prompt.is_chariox_owned());
        assert_eq!(
            prompt.external_observed_id(),
            Some(crate::history::ExternalProviderObservedId {
                provider: "codex".to_string(),
                provider_session_id: "thread-1".to_string(),
                provider_turn_id: "user-1".to_string(),
            })
        );
        assert_eq!(prompt.external_provider(), Some("codex"));
        assert_eq!(prompt.external_provider_session_id(), Some("thread-1"));
        assert_eq!(prompt.external_provider_turn_id(), Some("user-1"));

        let payload = serde_json::to_value(&prompt).expect("prompt should serialize");
        assert_eq!(
            payload.pointer("/external_provider"),
            Some(&serde_json::json!("codex"))
        );
        assert_eq!(
            payload.pointer("/external_provider_session_id"),
            Some(&serde_json::json!("thread-1"))
        );
        assert_eq!(
            payload.pointer("/external_provider_turn_id"),
            Some(&serde_json::json!("user-1"))
        );
    }

    #[test]
    fn prompt_queue_item_pending_conversion_preserves_external_metadata() {
        let prompt = PromptQueueItem::external_observed_running(
            "codex", "thread-1", "user-1", "agent-1", "run this",
        );

        let pending = prompt.into_pending_queue_item("pending-1");

        assert_eq!(pending.id(), "pending-1");
        assert_eq!(
            pending.external_observed_id(),
            Some(crate::history::ExternalProviderObservedId {
                provider: "codex".to_string(),
                provider_session_id: "thread-1".to_string(),
                provider_turn_id: "user-1".to_string(),
            })
        );
    }
}
