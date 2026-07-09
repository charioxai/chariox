use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

mod operational_archive;
mod operational_legacy_import;
mod operational_query;
mod operational_retention;
mod operational_session;
mod session_log;

pub use operational_archive::HistoryArchiveOutboxItem;
pub use operational_session::{ExternalImportHistoryEntry, ExternalImportHistoryIndex};
pub use session_log::{
    external_provider_observed_merge_key, external_provider_observed_merge_key_prefix,
    external_provider_observed_state_merge_key, parse_external_provider_observed_id,
    ExternalProviderObservedId, SessionHistoryEntry, SessionHistoryEntryKind,
    SessionHistoryEntrySource, SessionHistoryExternalObservation, SessionHistoryPromptAttachment,
    SessionHistoryStore, EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
    EXTERNAL_PROVIDER_ACTIVE_PROMPT_STARTED_REASON, EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS,
};

pub const OPERATIONAL_HISTORY_HARD_MAX_BYTES: u64 = 500 * 1024 * 1024;
pub const OPERATIONAL_HISTORY_HARD_MAX_MB: u32 =
    (OPERATIONAL_HISTORY_HARD_MAX_BYTES / 1024 / 1024) as u32;
pub const STEERING_PROMPT_MERGE_KEY_PREFIX: &str = "steering-prompt:";
const OPERATIONAL_HISTORY_SIZE_BUDGET_CHECK_BYTES: u64 = 1024 * 1024;

pub fn steering_prompt_merge_key(prompt_id: &str) -> String {
    format!("{STEERING_PROMPT_MERGE_KEY_PREFIX}{prompt_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryEventKind {
    UserPrompt,
    ProviderOutput,
    ProviderReasoning,
    ProviderTool,
    ProviderError,
    ProviderStatus,
    Notice,
    SessionCreated,
    AgentCreated,
    AgentMoved,
    WorkflowStarted,
    WorkflowNodeStarted,
    WorkflowNodeCompleted,
    McpGranted,
    SkillGranted,
    RemoteMachineConnected,
    RemoteMachineDisconnected,
    ArtifactStored,
    GitCommitDetected,
    GitWorktreeChanged,
    GitWorktreeDirty,
    GitWorktreeClean,
    GitPushDetected,
    WorkspaceLiveSyncModeChanged,
    PromptInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryEventRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAttributionConfidence {
    Definite,
    Likely,
    Ambiguous,
    Unattributed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEventTurnContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub event_id: String,
    pub sequence: u64,
    pub timestamp_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    pub kind: HistoryEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<HistoryEventRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_ref: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_agent_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_prompt_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_turn_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution_confidence: Option<HistoryAttributionConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caused_by_event_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HistoryEventQuery {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workflow_id: Option<String>,
    pub machine_id: Option<String>,
    pub repo_root: Option<String>,
    pub worktree_path: Option<String>,
    pub kind: Option<String>,
    pub text: Option<String>,
    pub after_sequence: Option<u64>,
    pub before_sequence: Option<u64>,
    pub limit: Option<usize>,
}

impl HistoryEvent {
    pub fn transcript(
        sequence: u64,
        entry: &SessionHistoryEntry,
        context: HistoryEventTurnContext,
    ) -> Self {
        let kind = HistoryEventKind::from(entry.kind);
        let role = HistoryEventRole::from_session_history_kind(entry.kind);
        let timestamp_ms = entry.timestamp_ms;
        let event_id = history_event_id(sequence, timestamp_ms);
        let mut metadata = BTreeMap::new();
        if let Some(merge_key) = entry.merge_key.clone() {
            metadata.insert(
                "merge_key".to_string(),
                serde_json::Value::String(merge_key),
            );
        }
        if let Some(source_attachment_id) = entry.source_attachment_id.clone() {
            metadata.insert(
                "source_attachment_id".to_string(),
                serde_json::Value::String(source_attachment_id),
            );
        }
        if let Some(prompt_origin) = entry.prompt_origin {
            metadata.insert(
                "prompt_origin".to_string(),
                serde_json::to_value(prompt_origin).unwrap_or(serde_json::Value::Null),
            );
        }
        if let Some(source) = entry.source {
            metadata.insert(
                "source".to_string(),
                serde_json::to_value(source).unwrap_or(serde_json::Value::Null),
            );
        }
        if let Some(provider) = entry.external_provider.clone() {
            metadata.insert(
                "external_provider".to_string(),
                serde_json::Value::String(provider),
            );
        }
        if let Some(provider_session_id) = entry.external_provider_session_id.clone() {
            metadata.insert(
                "external_provider_session_id".to_string(),
                serde_json::Value::String(provider_session_id),
            );
        }
        if let Some(provider_turn_id) = entry.external_provider_turn_id.clone() {
            metadata.insert(
                "external_provider_turn_id".to_string(),
                serde_json::Value::String(provider_turn_id),
            );
        }
        if let Some(observed_at_ms) = entry.observed_at_ms {
            metadata.insert(
                "observed_at_ms".to_string(),
                serde_json::Value::Number(observed_at_ms.into()),
            );
        }
        if let Some(external_observation) = entry.external_observation.clone() {
            metadata.insert(
                "external_observation".to_string(),
                serde_json::to_value(external_observation).unwrap_or(serde_json::Value::Null),
            );
        }
        if !entry.attachments.is_empty() {
            metadata.insert(
                "attachments".to_string(),
                serde_json::to_value(&entry.attachments).unwrap_or(serde_json::Value::Null),
            );
        }
        Self {
            event_id,
            sequence,
            timestamp_ms,
            workspace_id: context.workspace_id,
            session_id: context
                .session_id
                .or_else(|| Some(entry.session_id.clone())),
            agent_id: context.agent_id.or_else(|| entry.agent_id.clone()),
            agent_alias: context.agent_alias,
            provider: context.provider,
            model: context.model,
            turn_id: context.turn_id,
            prompt_id: context.prompt_id,
            provider_run_id: context
                .provider_run_id
                .or_else(|| entry.provider_run_id.clone()),
            provider_session_id: context.provider_session_id,
            workflow_id: context.workflow_id,
            workflow_run_id: context.workflow_run_id,
            workflow_node_id: context.workflow_node_id,
            machine_id: context.machine_id,
            repo_root: context.repo_root,
            worktree_path: context.worktree_path,
            kind,
            role: Some(role),
            content: Some(entry.text.clone()),
            content_ref: None,
            metadata,
            candidate_agent_ids: Vec::new(),
            candidate_prompt_ids: Vec::new(),
            candidate_turn_ids: Vec::new(),
            attribution_confidence: None,
            caused_by_event_id: None,
        }
    }

    pub fn to_session_history_entry(&self) -> Option<SessionHistoryEntry> {
        let kind = SessionHistoryEntryKind::try_from(self.kind).ok()?;
        let session_id = self.session_id.clone()?;
        let source_attachment_id = self
            .metadata
            .get("source_attachment_id")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let mut entry = SessionHistoryEntry {
            session_id,
            provider_run_id: self.provider_run_id.clone(),
            agent_id: self.agent_id.clone(),
            source_attachment_id,
            prompt_origin: self
                .metadata
                .get("prompt_origin")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok()),
            kind,
            merge_key: self
                .metadata
                .get("merge_key")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            source: self
                .metadata
                .get("source")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok()),
            external_provider: self
                .metadata
                .get("external_provider")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            external_provider_session_id: self
                .metadata
                .get("external_provider_session_id")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            external_provider_turn_id: self
                .metadata
                .get("external_provider_turn_id")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            observed_at_ms: self
                .metadata
                .get("observed_at_ms")
                .and_then(|value| value.as_u64()),
            external_observation: self
                .metadata
                .get("external_observation")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok()),
            attachments: self
                .metadata
                .get("attachments")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default(),
            text: self.content.clone().unwrap_or_default(),
            timestamp_ms: self.timestamp_ms,
        };
        entry.rehydrate_attachment_preview_urls();
        Some(entry)
    }

    pub fn session_history_external_observation(
        &self,
    ) -> Option<SessionHistoryExternalObservation> {
        self.to_session_history_entry()?.external_observation
    }

    pub fn operational(
        sequence: u64,
        kind: HistoryEventKind,
        role: Option<HistoryEventRole>,
        content: Option<String>,
        metadata: BTreeMap<String, serde_json::Value>,
        context: HistoryEventTurnContext,
    ) -> Self {
        let timestamp_ms = unix_epoch_ms();
        Self {
            event_id: history_event_id(sequence, timestamp_ms),
            sequence,
            timestamp_ms,
            workspace_id: context.workspace_id,
            session_id: context.session_id,
            agent_id: context.agent_id,
            agent_alias: context.agent_alias,
            provider: context.provider,
            model: context.model,
            turn_id: context.turn_id,
            prompt_id: context.prompt_id,
            provider_run_id: context.provider_run_id,
            provider_session_id: context.provider_session_id,
            workflow_id: context.workflow_id,
            workflow_run_id: context.workflow_run_id,
            workflow_node_id: context.workflow_node_id,
            machine_id: context.machine_id,
            repo_root: context.repo_root,
            worktree_path: context.worktree_path,
            kind,
            role,
            content,
            content_ref: None,
            metadata,
            candidate_agent_ids: Vec::new(),
            candidate_prompt_ids: Vec::new(),
            candidate_turn_ids: Vec::new(),
            attribution_confidence: None,
            caused_by_event_id: None,
        }
    }
}

impl From<SessionHistoryEntryKind> for HistoryEventKind {
    fn from(kind: SessionHistoryEntryKind) -> Self {
        match kind {
            SessionHistoryEntryKind::UserPrompt => Self::UserPrompt,
            SessionHistoryEntryKind::ProviderOutput => Self::ProviderOutput,
            SessionHistoryEntryKind::ProviderReasoning => Self::ProviderReasoning,
            SessionHistoryEntryKind::ProviderTool => Self::ProviderTool,
            SessionHistoryEntryKind::ProviderError => Self::ProviderError,
            SessionHistoryEntryKind::ProviderStatus => Self::ProviderStatus,
            SessionHistoryEntryKind::Notice => Self::Notice,
        }
    }
}

impl TryFrom<HistoryEventKind> for SessionHistoryEntryKind {
    type Error = ();

    fn try_from(kind: HistoryEventKind) -> Result<Self, Self::Error> {
        match kind {
            HistoryEventKind::UserPrompt => Ok(Self::UserPrompt),
            HistoryEventKind::ProviderOutput => Ok(Self::ProviderOutput),
            HistoryEventKind::ProviderReasoning => Ok(Self::ProviderReasoning),
            HistoryEventKind::ProviderTool => Ok(Self::ProviderTool),
            HistoryEventKind::ProviderError => Ok(Self::ProviderError),
            HistoryEventKind::ProviderStatus => Ok(Self::ProviderStatus),
            HistoryEventKind::Notice => Ok(Self::Notice),
            _ => Err(()),
        }
    }
}

impl HistoryEventRole {
    fn from_session_history_kind(kind: SessionHistoryEntryKind) -> Self {
        match kind {
            SessionHistoryEntryKind::UserPrompt => Self::User,
            SessionHistoryEntryKind::ProviderOutput
            | SessionHistoryEntryKind::ProviderReasoning
            | SessionHistoryEntryKind::ProviderError => Self::Assistant,
            SessionHistoryEntryKind::ProviderTool => Self::Tool,
            SessionHistoryEntryKind::ProviderStatus | SessionHistoryEntryKind::Notice => {
                Self::System
            }
        }
    }
}

fn history_event_id(sequence: u64, timestamp_ms: u64) -> String {
    format!("evt_{timestamp_ms}_{sequence}")
}

#[derive(Debug, Clone)]
pub struct OperationalHistoryStore {
    path: PathBuf,
    connection: Arc<Mutex<Connection>>,
    next_sequence: Arc<AtomicU64>,
    reclaim_in_progress: Arc<AtomicBool>,
    appended_bytes_since_size_check: Arc<AtomicU64>,
    read_delay_ms: u64,
    max_size_bytes: u64,
}

impl OperationalHistoryStore {
    pub fn open(path: PathBuf) -> Result<Self, DaemonError> {
        Self::open_with_read_delay(path, 0)
    }

    pub fn open_with_read_delay(path: PathBuf, read_delay_ms: u64) -> Result<Self, DaemonError> {
        Self::open_with_read_delay_and_max_size(
            path,
            read_delay_ms,
            OPERATIONAL_HISTORY_HARD_MAX_BYTES,
        )
    }

    pub fn open_with_read_delay_and_max_size(
        path: PathBuf,
        read_delay_ms: u64,
        max_size_bytes: u64,
    ) -> Result<Self, DaemonError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "create operational history directory",
                message: error.to_string(),
            })?;
        }
        let connection =
            Connection::open(&path).map_err(|error| operational_history_error("open", error))?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| operational_history_error("enable WAL mode", error))?;
        connection
            .execute_batch(OPERATIONAL_HISTORY_SCHEMA)
            .map_err(|error| operational_history_error("migrate schema", error))?;
        let max_sequence: u64 = connection
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM history_events",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value.max(0) as u64)
            .map_err(|error| operational_history_error("load max sequence", error))?;
        let store = Self {
            path,
            connection: Arc::new(Mutex::new(connection)),
            next_sequence: Arc::new(AtomicU64::new(max_sequence + 1)),
            reclaim_in_progress: Arc::new(AtomicBool::new(false)),
            appended_bytes_since_size_check: Arc::new(AtomicU64::new(0)),
            read_delay_ms,
            max_size_bytes: max_size_bytes.clamp(1, OPERATIONAL_HISTORY_HARD_MAX_BYTES),
        };
        store.enforce_size_budget()?;
        Ok(store)
    }

    pub(crate) fn delay_read_if_configured(&self) {
        if self.read_delay_ms > 0 {
            thread::sleep(Duration::from_millis(self.read_delay_ms));
        }
    }

    pub fn append_transcript(
        &self,
        entry: &SessionHistoryEntry,
        context: HistoryEventTurnContext,
    ) -> Result<HistoryEvent, DaemonError> {
        entry.validate_for_history_append("append operational history transcript")?;
        let sequence = self.reserve_sequence();
        let event = HistoryEvent::transcript(sequence, entry, context);
        self.append(&event)?;
        Ok(event)
    }

    pub fn append_transcripts(
        &self,
        entries: Vec<(&SessionHistoryEntry, HistoryEventTurnContext)>,
    ) -> Result<Vec<HistoryEvent>, DaemonError> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        for (entry, _) in &entries {
            entry.validate_for_history_append("append operational history transcripts")?;
        }
        let events = entries
            .into_iter()
            .map(|(entry, context)| {
                let sequence = self.reserve_sequence();
                HistoryEvent::transcript(sequence, entry, context)
            })
            .collect::<Vec<_>>();
        self.append_many(&events)?;
        Ok(events)
    }

    pub fn replace_transcript_by_merge_key(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
        merge_key: &str,
        entry: &SessionHistoryEntry,
        context: HistoryEventTurnContext,
    ) -> Result<Option<HistoryEvent>, DaemonError> {
        entry.validate_for_history_append("replace operational history transcript")?;
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let existing = {
            let mut statement = connection
                .prepare(
                    "SELECT event_json
                     FROM history_events
                     WHERE session_id = ?1
                     ORDER BY sequence ASC",
                )
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "prepare operational history replacement lookup",
                    message: error.to_string(),
                })?;
            let mut rows = statement.query(params![session_id]).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "query operational history replacement lookup",
                    message: error.to_string(),
                }
            })?;
            let mut existing = None;
            while let Some(row) =
                rows.next()
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.to_string()),
                        operation: "read operational history replacement lookup",
                        message: error.to_string(),
                    })?
            {
                let event_json =
                    row.get::<_, String>(0)
                        .map_err(|error| DaemonError::SessionHistoryFailed {
                            session_id: Some(session_id.to_string()),
                            operation: "read operational history replacement event",
                            message: error.to_string(),
                        })?;
                let event = serde_json::from_str::<HistoryEvent>(&event_json).map_err(|error| {
                    DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.to_string()),
                        operation: "decode operational history replacement event",
                        message: error.to_string(),
                    }
                })?;
                if event.agent_id.as_deref() == agent_id
                    && event
                        .metadata
                        .get("merge_key")
                        .and_then(|value| value.as_str())
                        == Some(merge_key)
                {
                    existing = Some(event);
                    break;
                }
            }
            existing
        };
        let Some(existing) = existing else {
            return Ok(None);
        };
        let mut replacement = HistoryEvent::transcript(existing.sequence, entry, context);
        replacement.event_id = existing.event_id.clone();
        replacement.sequence = existing.sequence;
        let event_json = serde_json::to_string(&replacement).map_err(|error| {
            DaemonError::SessionHistoryFailed {
                session_id: replacement.session_id.clone(),
                operation: "encode operational history replacement",
                message: error.to_string(),
            }
        })?;
        let metadata_text = searchable_metadata(&replacement);
        connection
            .execute(
                "UPDATE history_events
                 SET timestamp_ms = ?2,
                     kind = ?3,
                     session_id = ?4,
                     agent_id = ?5,
                     provider = ?6,
                     model = ?7,
                     turn_id = ?8,
                     prompt_id = ?9,
                     provider_run_id = ?10,
                     workflow_id = ?11,
                     workflow_run_id = ?12,
                     workflow_node_id = ?13,
                     machine_id = ?14,
                     repo_root = ?15,
                     worktree_path = ?16,
                     content = ?17,
                     content_ref = ?18,
                     metadata_text = ?19,
                     event_json = ?20
                 WHERE event_id = ?1",
                params![
                    replacement.event_id.as_str(),
                    replacement.timestamp_ms as i64,
                    history_event_kind_key(replacement.kind),
                    replacement.session_id.as_deref(),
                    replacement.agent_id.as_deref(),
                    replacement.provider.as_deref(),
                    replacement.model.as_deref(),
                    replacement.turn_id.as_deref(),
                    replacement.prompt_id.as_deref(),
                    replacement.provider_run_id.as_deref(),
                    replacement.workflow_id.as_deref(),
                    replacement.workflow_run_id.as_deref(),
                    replacement.workflow_node_id.as_deref(),
                    replacement.machine_id.as_deref(),
                    replacement.repo_root.as_deref(),
                    replacement.worktree_path.as_deref(),
                    replacement.content.as_deref(),
                    replacement.content_ref.as_deref(),
                    metadata_text,
                    event_json,
                ],
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: replacement.session_id.clone(),
                operation: "replace operational history event",
                message: error.to_string(),
            })?;
        Ok(Some(replacement))
    }

    pub fn reserve_sequence(&self) -> u64 {
        self.next_sequence.fetch_add(1, Ordering::Relaxed)
    }

    pub fn append_operational_event(
        &self,
        kind: HistoryEventKind,
        role: Option<HistoryEventRole>,
        content: Option<String>,
        metadata: BTreeMap<String, serde_json::Value>,
        context: HistoryEventTurnContext,
    ) -> Result<HistoryEvent, DaemonError> {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        let event = HistoryEvent::operational(sequence, kind, role, content, metadata, context);
        self.append(&event)?;
        Ok(event)
    }

    pub fn append(&self, event: &HistoryEvent) -> Result<(), DaemonError> {
        self.append_many(std::slice::from_ref(event))
    }

    pub fn append_many(&self, events: &[HistoryEvent]) -> Result<(), DaemonError> {
        if events.is_empty() {
            return Ok(());
        }
        let mut encoded_events = Vec::with_capacity(events.len());
        let mut estimated_append_bytes = 0_u64;
        for event in events {
            let event_json = serde_json::to_string(event).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: event.session_id.clone(),
                    operation: "encode operational history event",
                    message: error.to_string(),
                }
            })?;
            let metadata_text = searchable_metadata(event);
            estimated_append_bytes = estimated_append_bytes.saturating_add(
                estimate_history_event_storage_bytes(&event_json, &metadata_text),
            );
            encoded_events.push((event, event_json, metadata_text));
        }
        let mut connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: events.first().and_then(|event| event.session_id.clone()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let transaction = connection.transaction().map_err(|error| {
            operational_history_error("begin operational history append", error)
        })?;
        {
            let mut statement = transaction
                .prepare(OPERATIONAL_HISTORY_INSERT_SQL)
                .map_err(|error| {
                    operational_history_error("prepare operational history append", error)
                })?;
            for (event, event_json, metadata_text) in encoded_events {
                statement
                    .execute(params![
                        event.event_id.as_str(),
                        event.sequence as i64,
                        event.timestamp_ms as i64,
                        history_event_kind_key(event.kind),
                        event.session_id.as_deref(),
                        event.agent_id.as_deref(),
                        event.provider.as_deref(),
                        event.model.as_deref(),
                        event.turn_id.as_deref(),
                        event.prompt_id.as_deref(),
                        event.provider_run_id.as_deref(),
                        event.workflow_id.as_deref(),
                        event.workflow_run_id.as_deref(),
                        event.workflow_node_id.as_deref(),
                        event.machine_id.as_deref(),
                        event.repo_root.as_deref(),
                        event.worktree_path.as_deref(),
                        event.content.as_deref(),
                        event.content_ref.as_deref(),
                        metadata_text,
                        event_json,
                    ])
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: event.session_id.clone(),
                        operation: "append operational history event",
                        message: error.to_string(),
                    })?;
            }
        }
        transaction.commit().map_err(|error| {
            operational_history_error("commit operational history append", error)
        })?;
        drop(connection);
        self.enforce_size_budget_after_append(estimated_append_bytes)?;
        Ok(())
    }
}

const OPERATIONAL_HISTORY_INSERT_SQL: &str = "INSERT OR IGNORE INTO history_events (
                    event_id,
                    sequence,
                    timestamp_ms,
                    kind,
                    session_id,
                    agent_id,
                    provider,
                    model,
                    turn_id,
                    prompt_id,
                    provider_run_id,
                    workflow_id,
                    workflow_run_id,
                    workflow_node_id,
                    machine_id,
                    repo_root,
                    worktree_path,
                    content,
                    content_ref,
                    metadata_text,
                    event_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)";

impl OperationalHistoryStore {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn max_size_bytes(&self) -> u64 {
        self.max_size_bytes
    }

    fn size_budget_check_interval_bytes(&self) -> u64 {
        OPERATIONAL_HISTORY_SIZE_BUDGET_CHECK_BYTES
            .min((self.max_size_bytes / 4).max(1))
            .max(1)
    }
}

const OPERATIONAL_HISTORY_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS history_events (
    event_id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    session_id TEXT,
    agent_id TEXT,
    provider TEXT,
    model TEXT,
    turn_id TEXT,
    prompt_id TEXT,
    provider_run_id TEXT,
    workflow_id TEXT,
    workflow_run_id TEXT,
    workflow_node_id TEXT,
    machine_id TEXT,
    repo_root TEXT,
    worktree_path TEXT,
    content TEXT,
    content_ref TEXT,
    metadata_text TEXT,
    event_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_history_events_session_sequence
    ON history_events(session_id, sequence);
CREATE INDEX IF NOT EXISTS idx_history_events_agent_sequence
    ON history_events(agent_id, sequence);
CREATE INDEX IF NOT EXISTS idx_history_events_provider_model
    ON history_events(provider, model);
CREATE INDEX IF NOT EXISTS idx_history_events_kind_timestamp
    ON history_events(kind, timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_history_events_workflow
    ON history_events(workflow_id, workflow_run_id, workflow_node_id);
CREATE INDEX IF NOT EXISTS idx_history_events_machine
    ON history_events(machine_id, timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_history_events_repo
    ON history_events(repo_root, worktree_path);

CREATE TABLE IF NOT EXISTS history_archive_outbox (
    event_id TEXT PRIMARY KEY,
    event_json TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    archived_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_history_archive_outbox_pending
    ON history_archive_outbox(archived_at_ms, created_at_ms);

CREATE TABLE IF NOT EXISTS history_session_markers (
    session_id TEXT PRIMARY KEY,
    legacy_fallback_disabled_at_ms INTEGER
);
"#;

fn operational_history_error(operation: &'static str, error: rusqlite::Error) -> DaemonError {
    DaemonError::SessionHistoryFailed {
        session_id: None,
        operation,
        message: error.to_string(),
    }
}

fn history_event_kind_key(kind: HistoryEventKind) -> &'static str {
    match kind {
        HistoryEventKind::UserPrompt => "user_prompt",
        HistoryEventKind::ProviderOutput => "provider_output",
        HistoryEventKind::ProviderReasoning => "provider_reasoning",
        HistoryEventKind::ProviderTool => "provider_tool",
        HistoryEventKind::ProviderError => "provider_error",
        HistoryEventKind::ProviderStatus => "provider_status",
        HistoryEventKind::Notice => "notice",
        HistoryEventKind::SessionCreated => "session_created",
        HistoryEventKind::AgentCreated => "agent_created",
        HistoryEventKind::AgentMoved => "agent_moved",
        HistoryEventKind::WorkflowStarted => "workflow_started",
        HistoryEventKind::WorkflowNodeStarted => "workflow_node_started",
        HistoryEventKind::WorkflowNodeCompleted => "workflow_node_completed",
        HistoryEventKind::McpGranted => "mcp_granted",
        HistoryEventKind::SkillGranted => "skill_granted",
        HistoryEventKind::RemoteMachineConnected => "remote_machine_connected",
        HistoryEventKind::RemoteMachineDisconnected => "remote_machine_disconnected",
        HistoryEventKind::ArtifactStored => "artifact_stored",
        HistoryEventKind::GitCommitDetected => "git_commit_detected",
        HistoryEventKind::GitWorktreeChanged => "git_worktree_changed",
        HistoryEventKind::GitWorktreeDirty => "git_worktree_dirty",
        HistoryEventKind::GitWorktreeClean => "git_worktree_clean",
        HistoryEventKind::GitPushDetected => "git_push_detected",
        HistoryEventKind::WorkspaceLiveSyncModeChanged => "workspace_live_sync_mode_changed",
        HistoryEventKind::PromptInput => "prompt_input",
    }
}

fn searchable_metadata(event: &HistoryEvent) -> String {
    let mut parts = Vec::new();
    for value in event.metadata.values() {
        collect_searchable_json_strings(value, &mut parts);
    }
    parts.extend(event.candidate_agent_ids.iter().cloned());
    parts.extend(event.candidate_prompt_ids.iter().cloned());
    parts.extend(event.candidate_turn_ids.iter().cloned());
    parts.join("\n")
}

fn estimate_history_event_storage_bytes(event_json: &str, metadata_text: &str) -> u64 {
    event_json
        .len()
        .saturating_add(metadata_text.len())
        .saturating_add(512) as u64
}

fn collect_searchable_json_strings(value: &serde_json::Value, parts: &mut Vec<String>) {
    match value {
        serde_json::Value::String(value) => parts.push(value.clone()),
        serde_json::Value::Array(values) => {
            for value in values {
                collect_searchable_json_strings(value, parts);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                collect_searchable_json_strings(value, parts);
            }
        }
        _ => {}
    }
}

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;
