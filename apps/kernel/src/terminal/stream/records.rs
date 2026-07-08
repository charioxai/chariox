use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInputRecord {
    pub session_id: String,
    pub provider_run_id: String,
    pub source_attachment_id: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutputKind {
    ProviderOutput,
    PromptEcho,
    ProviderReasoning,
    ProviderTool,
    ProviderError,
    ProviderStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOutputRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<u64>,
    pub timestamp_ms: u64,
    pub session_id: String,
    pub provider_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_origin: Option<PromptOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_attachment_id: Option<String>,
    pub kind: TerminalOutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
    pub recipient_attachment_ids: Vec<String>,
    pub pending_recipient_attachment_ids: Vec<String>,
    pub bytes: Vec<u8>,
    #[serde(default, flatten, skip_serializing_if = "Option::is_none")]
    pub external_observation_metadata: Option<TerminalOutputExternalObservationMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOutputExternalObservationMetadata {
    pub source: SessionHistoryEntrySource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_observation: Option<SessionHistoryExternalObservation>,
}

impl TerminalOutputExternalObservationMetadata {
    pub fn from_session_history_entry(entry: &crate::history::SessionHistoryEntry) -> Option<Self> {
        let source = entry.source?;
        Some(Self {
            source,
            external_provider: entry.external_provider.clone(),
            external_provider_session_id: entry.external_provider_session_id.clone(),
            external_provider_turn_id: entry.external_provider_turn_id.clone(),
            observed_at_ms: entry.observed_at_ms,
            external_observation: entry.external_observation.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOutputAppend {
    pub session_id: String,
    pub provider_run_id: String,
    pub agent_id: Option<String>,
    pub prompt_origin: Option<PromptOrigin>,
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub recipient_attachment_ids: Arc<[String]>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeNoticeRecord {
    pub session_id: String,
    pub provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub recipient_attachment_ids: Vec<String>,
    pub pending_recipient_attachment_ids: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantMessageCompletionRecord {
    pub session_id: String,
    pub provider_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub recipient_attachment_ids: Vec<String>,
    pub pending_recipient_attachment_ids: Vec<String>,
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TerminalStreamHealthSnapshot {
    pub pending_output_records: usize,
    pub pending_notice_records: usize,
    pub pending_completion_records: usize,
    pub pending_output_record_limit_per_attachment: usize,
    pub trimmed_pending_output_recipients: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TerminalStreamHealthStore {
    snapshot: Arc<StdMutex<TerminalStreamHealthSnapshot>>,
}

impl TerminalStreamHealthStore {
    pub fn snapshot(&self) -> TerminalStreamHealthSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn update(&self, snapshot: TerminalStreamHealthSnapshot) {
        *self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot;
    }
}
