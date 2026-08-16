use std::collections::VecDeque;

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
    External,
}

/// Identifies the interaction surface that submitted or observed a prompt.
/// This is intentionally orthogonal to [`PromptOrigin`]: agent-terminal
/// prompts are Chariox-owned prompts, while provider-native turns remain
/// `PromptOrigin::External` and use `ProviderExternal` as their source.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSource {
    #[default]
    Human,
    AgentTerminal,
    ProviderExternal,
}

fn is_human_prompt_source(source: &PromptSource) -> bool {
    *source == PromptSource::Human
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
    status: PromptStatus,
    #[serde(default)]
    prompt_origin: PromptOrigin,
    #[serde(default, skip_serializing_if = "is_human_prompt_source")]
    prompt_source: PromptSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider_turn_id: Option<String>,
    workflow_run_id: Option<String>,
    workflow_node_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DurablePromptPrivateState {
    pub(crate) session_id: String,
    pub(crate) prompt_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) hidden_system_context: String,
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
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub(crate) recovery_generation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_phase: Option<DurablePromptDeliveryPhase>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PromptPrivateMetadata {
    hidden_system_context: String,
    operation_id: Option<String>,
    operation_fingerprint: Option<String>,
    initially_queued: Option<bool>,
    delivery_phase: Option<DurablePromptDeliveryPhase>,
    delivery_provider_run_id: Option<String>,
    delivery_provider_session_id: Option<String>,
    recovery_generation: u32,
    recovery_operation_id: Option<String>,
    recovery_phase: Option<DurablePromptDeliveryPhase>,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

impl DurablePromptPrivateState {
    pub(crate) fn from_prompt(session_id: &str, prompt: &PromptQueueItem) -> Option<Self> {
        prompt.private_metadata.as_ref().map(|metadata| Self {
            session_id: session_id.to_string(),
            prompt_id: prompt.id.clone(),
            hidden_system_context: metadata.hidden_system_context.clone(),
            operation_id: metadata.operation_id.clone(),
            operation_fingerprint: metadata.operation_fingerprint.clone(),
            initially_queued: metadata.initially_queued,
            delivery_phase: metadata.delivery_phase,
            delivery_provider_run_id: metadata.delivery_provider_run_id.clone(),
            delivery_provider_session_id: metadata.delivery_provider_session_id.clone(),
            recovery_generation: metadata.recovery_generation,
            recovery_operation_id: metadata.recovery_operation_id.clone(),
            recovery_phase: metadata.recovery_phase,
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
    prompt_origin: PromptOrigin,
    #[serde(default)]
    prompt_source: PromptSource,
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
            prompt_origin: prompt.prompt_origin,
            prompt_source: prompt.prompt_source,
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
            status: PromptStatus::Queued,
            prompt_origin: self.prompt_origin,
            prompt_source: self.prompt_source,
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
            status,
            prompt_origin: PromptOrigin::Chariox,
            prompt_source: PromptSource::Human,
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
        .with_prompt_origin(PromptOrigin::External);
        let prompt = prompt.with_prompt_source(PromptSource::ProviderExternal);
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

    pub fn with_hidden_system_context(mut self, hidden_system_context: impl Into<String>) -> Self {
        let hidden_system_context = hidden_system_context.into();
        if hidden_system_context.is_empty() {
            if let Some(metadata) = self.private_metadata.as_mut() {
                metadata.hidden_system_context.clear();
                if metadata.operation_id.is_none()
                    && metadata.delivery_phase.is_none()
                    && metadata.recovery_operation_id.is_none()
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

    pub fn with_prompt_source(mut self, prompt_source: PromptSource) -> Self {
        self.prompt_source = prompt_source;
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

    pub fn hidden_system_context(&self) -> &str {
        self.private_metadata
            .as_ref()
            .map(|metadata| metadata.hidden_system_context.as_str())
            .unwrap_or_default()
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
        if let Some(metadata) = self.private_metadata.as_mut() {
            if metadata.operation_id.is_none() {
                return;
            }
            metadata.initially_queued = Some(queued);
        }
    }

    pub(crate) fn durable_delivery_phase(&self) -> Option<DurablePromptDeliveryPhase> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.delivery_phase)
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
    }

    pub(crate) fn durable_recovery_operation_id(&self) -> Option<&str> {
        self.private_metadata
            .as_ref()
            .and_then(|metadata| metadata.recovery_operation_id.as_deref())
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
            && private.operation_id.is_none()
            && private.delivery_phase.is_none()
            && private.recovery_operation_id.is_none()
        {
            self.private_metadata = None;
            return;
        }
        self.private_metadata = Some(Box::new(PromptPrivateMetadata {
            hidden_system_context: private.hidden_system_context.clone(),
            operation_id: private.operation_id.clone(),
            operation_fingerprint: private.operation_fingerprint.clone(),
            initially_queued: private.initially_queued,
            delivery_phase: private.delivery_phase,
            delivery_provider_run_id: private.delivery_provider_run_id.clone(),
            delivery_provider_session_id: private.delivery_provider_session_id.clone(),
            recovery_generation: private.recovery_generation,
            recovery_operation_id: private.recovery_operation_id.clone(),
            recovery_phase: private.recovery_phase,
        }));
    }

    pub fn status(&self) -> PromptStatus {
        self.status
    }

    pub fn prompt_origin(&self) -> PromptOrigin {
        self.prompt_origin
    }

    pub fn prompt_source(&self) -> PromptSource {
        self.prompt_source
    }

    pub fn is_external(&self) -> bool {
        self.prompt_origin == PromptOrigin::External
    }

    pub fn is_chariox_owned(&self) -> bool {
        self.prompt_origin == PromptOrigin::Chariox
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
        self.status = status;
    }

    pub fn set_prompt(&mut self, prompt: impl Into<String>) {
        self.prompt = prompt.into();
        self.updated_at_ms = crate::session::unix_epoch_ms();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSubmissionOutcome {
    Started { prompt: PromptQueueItem },
    Queued { prompt: PromptQueueItem },
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
pub struct PromptCompletion {
    pub completed: PromptQueueItem,
    pub started_next: Option<PromptQueueItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCancellation {
    pub prompt: PromptQueueItem,
    pub started_next: Option<PromptQueueItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PromptDetachEffect {
    pub removed_active_prompt: bool,
    pub removed_queued_prompt_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_queue_item_does_not_serialize_hidden_system_context() {
        let mut item = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "VISIBLE_PROMPT_TOKEN",
            PromptStatus::Queued,
        )
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
        assert!(!payload.contains("provider-run-private"));
        assert!(!payload.contains("provider-session-private"));
        assert!(!payload.contains(&recovery_operation_id));
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
    fn agent_terminal_source_is_orthogonal_to_prompt_origin() {
        let item = PromptQueueItem::new(
            "prompt-agent-terminal",
            "attachment-1",
            "agent-1",
            "hello",
            PromptStatus::Queued,
        )
        .with_prompt_source(PromptSource::AgentTerminal);

        assert_eq!(item.prompt_origin(), PromptOrigin::Chariox);
        assert_eq!(item.prompt_source(), PromptSource::AgentTerminal);
        let value = serde_json::to_value(item).expect("prompt should serialize");
        assert_eq!(
            value.get("prompt_origin"),
            Some(&serde_json::json!("chariox"))
        );
        assert_eq!(
            value.get("prompt_source"),
            Some(&serde_json::json!("agent_terminal"))
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

        assert!(chariox_prompt.is_chariox_owned());
        assert!(!chariox_prompt.is_external());
        assert!(external_prompt.is_external());
        assert!(!external_prompt.is_chariox_owned());
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
