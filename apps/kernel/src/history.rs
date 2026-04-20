use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::session::RuntimeSession;
use crate::terminal::TerminalOutputKind;

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
    GitCommitDetected,
    GitWorktreeChanged,
    GitWorktreeDirty,
    GitWorktreeClean,
    GitPushDetected,
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
        Some(SessionHistoryEntry {
            session_id,
            provider_run_id: self.provider_run_id.clone(),
            agent_id: self.agent_id.clone(),
            source_attachment_id,
            kind,
            merge_key: self
                .metadata
                .get("merge_key")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            text: self.content.clone().unwrap_or_default(),
            timestamp_ms: self.timestamp_ms,
        })
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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionHistoryEntryKind {
    UserPrompt,
    ProviderOutput,
    ProviderReasoning,
    ProviderTool,
    ProviderError,
    ProviderStatus,
    Notice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryEntry {
    pub session_id: String,
    pub provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub source_attachment_id: Option<String>,
    pub kind: SessionHistoryEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
    pub text: String,
    pub timestamp_ms: u64,
}

impl SessionHistoryEntry {
    pub fn user_prompt(
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        text: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            provider_run_id: None,
            agent_id: Some(agent_id.to_string()),
            source_attachment_id: Some(source_attachment_id.to_string()),
            kind: SessionHistoryEntryKind::UserPrompt,
            merge_key: None,
            text: text.into(),
            timestamp_ms: unix_epoch_ms(),
        }
    }

    pub fn provider_output(
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            provider_run_id: Some(provider_run_id.to_string()),
            agent_id: agent_id.map(str::to_string),
            source_attachment_id: None,
            kind: match kind {
                TerminalOutputKind::ProviderOutput => SessionHistoryEntryKind::ProviderOutput,
                TerminalOutputKind::ProviderReasoning => SessionHistoryEntryKind::ProviderReasoning,
                TerminalOutputKind::ProviderTool => SessionHistoryEntryKind::ProviderTool,
                TerminalOutputKind::ProviderError => SessionHistoryEntryKind::ProviderError,
                TerminalOutputKind::ProviderStatus => SessionHistoryEntryKind::ProviderStatus,
                TerminalOutputKind::PromptEcho => SessionHistoryEntryKind::UserPrompt,
            },
            merge_key,
            text: text.into(),
            timestamp_ms: unix_epoch_ms(),
        }
    }

    pub fn notice(
        session_id: &str,
        provider_run_id: Option<&str>,
        agent_id: Option<&str>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.map(str::to_string),
            agent_id: agent_id.map(str::to_string),
            source_attachment_id: None,
            kind: SessionHistoryEntryKind::Notice,
            merge_key: None,
            text: text.into(),
            timestamp_ms: unix_epoch_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHistoryStore {
    root: PathBuf,
    read_delay_ms: u64,
}

#[derive(Debug, Clone)]
pub struct OperationalHistoryStore {
    path: PathBuf,
    connection: Arc<Mutex<Connection>>,
    next_sequence: Arc<AtomicU64>,
}

impl OperationalHistoryStore {
    pub fn open(path: PathBuf) -> Result<Self, DaemonError> {
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
        Ok(Self {
            path,
            connection: Arc::new(Mutex::new(connection)),
            next_sequence: Arc::new(AtomicU64::new(max_sequence + 1)),
        })
    }

    pub fn append_transcript(
        &self,
        entry: &SessionHistoryEntry,
        context: HistoryEventTurnContext,
    ) -> Result<HistoryEvent, DaemonError> {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        let event = HistoryEvent::transcript(sequence, entry, context);
        self.append(&event)?;
        Ok(event)
    }

    pub fn append(&self, event: &HistoryEvent) -> Result<(), DaemonError> {
        let event_json =
            serde_json::to_string(event).map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: event.session_id.clone(),
                operation: "encode operational history event",
                message: error.to_string(),
            })?;
        let metadata_text = searchable_metadata(event);
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: event.session_id.clone(),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        connection
            .execute(
                "INSERT OR IGNORE INTO history_events (
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
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
                params![
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
                ],
            )
            .map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: event.session_id.clone(),
                    operation: "append operational history event",
                    message: error.to_string(),
                }
            })?;
        Ok(())
    }

    pub fn load_session_events(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Result<Vec<HistoryEvent>, DaemonError> {
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let sql = if agent_id.is_some() {
            "SELECT event_json FROM history_events WHERE session_id = ?1 AND agent_id = ?2 ORDER BY sequence ASC"
        } else {
            "SELECT event_json FROM history_events WHERE session_id = ?1 ORDER BY sequence ASC"
        };
        let mut statement =
            connection
                .prepare(sql)
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "prepare operational history load",
                    message: error.to_string(),
                })?;
        let mut rows = if let Some(agent_id) = agent_id {
            statement.query(params![session_id, agent_id])
        } else {
            statement.query(params![session_id])
        }
        .map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: Some(session_id.to_string()),
            operation: "load operational history events",
            message: error.to_string(),
        })?;
        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session_id.to_string()),
                operation: "read operational history event",
                message: error.to_string(),
            })?
        {
            let event_json =
                row.get::<_, String>(0)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.to_string()),
                        operation: "read operational history event",
                        message: error.to_string(),
                    })?;
            let event = serde_json::from_str::<HistoryEvent>(&event_json).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "decode operational history event",
                    message: error.to_string(),
                }
            })?;
            events.push(event);
        }
        Ok(events)
    }

    pub fn load_session_history_entries(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
        let events = self.load_session_events(session_id, agent_id)?;
        Ok(events
            .into_iter()
            .filter_map(|event| event.to_session_history_entry())
            .collect())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SessionHistoryStore {
    pub fn new(root: PathBuf) -> Result<Self, DaemonError> {
        Self::new_with_read_delay(root, 0)
    }

    pub fn new_with_read_delay(root: PathBuf, read_delay_ms: u64) -> Result<Self, DaemonError> {
        fs::create_dir_all(&root).map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: None,
            operation: "create session history directory",
            message: error.to_string(),
        })?;
        Ok(Self {
            root,
            read_delay_ms,
        })
    }

    pub fn load(&self, session: &RuntimeSession) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
        if self.read_delay_ms > 0 {
            thread::sleep(Duration::from_millis(self.read_delay_ms));
        }
        let path = self.path_for_session(session);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&path).map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: Some(session.id().to_string()),
            operation: "open session history",
            message: error.to_string(),
        })?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "read session history",
                message: error.to_string(),
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let entry = serde_json::from_str::<SessionHistoryEntry>(&line).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session.id().to_string()),
                    operation: "decode session history",
                    message: error.to_string(),
                }
            })?;
            entries.push(entry);
        }
        Ok(entries)
    }

    pub fn append(
        &self,
        session: &RuntimeSession,
        entry: &SessionHistoryEntry,
    ) -> Result<(), DaemonError> {
        if matches!(entry.kind, SessionHistoryEntryKind::UserPrompt)
            && entry.source_attachment_id.is_none()
        {
            return Err(DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "append session history",
                message: "user prompt history entry must include source attachment".to_string(),
            });
        }

        let path = self.path_for_session(session);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "prepare session history directory",
                message: error.to_string(),
            })?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "open session history for append",
                message: error.to_string(),
            })?;
        let encoded =
            serde_json::to_string(entry).map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "encode session history",
                message: error.to_string(),
            })?;
        file.write_all(encoded.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "write session history",
                message: error.to_string(),
            })
    }

    pub fn path_for_session(&self, session: &RuntimeSession) -> PathBuf {
        self.root.join(history_file_name(session))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn history_file_name(session: &RuntimeSession) -> String {
    format!("{}-{}.jsonl", session.id(), session.created_at_ms())
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
        HistoryEventKind::GitCommitDetected => "git_commit_detected",
        HistoryEventKind::GitWorktreeChanged => "git_worktree_changed",
        HistoryEventKind::GitWorktreeDirty => "git_worktree_dirty",
        HistoryEventKind::GitWorktreeClean => "git_worktree_clean",
        HistoryEventKind::GitPushDetected => "git_push_detected",
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
mod tests {
    use crate::config::DaemonConfig;
    use crate::session::{CreateSessionRequest, SessionService};
    use crate::terminal::TerminalOutputKind;

    use super::{
        HistoryEvent, HistoryEventKind, HistoryEventRole, HistoryEventTurnContext,
        OperationalHistoryStore, SessionHistoryEntry, SessionHistoryEntryKind, SessionHistoryStore,
    };

    #[test]
    fn appends_and_loads_session_history() {
        let config = DaemonConfig::for_tests();
        let mut sessions = SessionService::new(&config);
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let store = SessionHistoryStore::new(config.session_history_root.clone())
            .expect("history store should initialize");

        store
            .append(
                &session,
                &SessionHistoryEntry::user_prompt(
                    session.id(),
                    "attachment-1",
                    "agent-1",
                    "hello\n",
                ),
            )
            .expect("user prompt should persist");
        store
            .append(
                &session,
                &SessionHistoryEntry::provider_output(
                    session.id(),
                    "provider-run-1",
                    Some("agent-1"),
                    TerminalOutputKind::ProviderOutput,
                    None,
                    "world",
                ),
            )
            .expect("provider output should persist");

        let entries = store.load(&session).expect("history should load");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, SessionHistoryEntryKind::UserPrompt);
        assert_eq!(entries[1].kind, SessionHistoryEntryKind::ProviderOutput);
    }

    #[test]
    fn converts_session_history_entry_to_canonical_history_event() {
        let entry = SessionHistoryEntry::provider_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some("tool:browser".to_string()),
            "called browser",
        );
        let event = HistoryEvent::transcript(
            7,
            &entry,
            HistoryEventTurnContext {
                provider: Some("codex".to_string()),
                model: Some("gpt-5.2".to_string()),
                turn_id: Some("turn-1".to_string()),
                prompt_id: Some("prompt-1".to_string()),
                worktree_path: Some("/repo".to_string()),
                ..HistoryEventTurnContext::default()
            },
        );

        assert_eq!(event.event_id, format!("evt_{}_7", entry.timestamp_ms));
        assert_eq!(event.sequence, 7);
        assert_eq!(event.session_id.as_deref(), Some("session-1"));
        assert_eq!(event.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(event.provider.as_deref(), Some("codex"));
        assert_eq!(event.model.as_deref(), Some("gpt-5.2"));
        assert_eq!(event.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(event.prompt_id.as_deref(), Some("prompt-1"));
        assert_eq!(event.provider_run_id.as_deref(), Some("provider-run-1"));
        assert_eq!(event.worktree_path.as_deref(), Some("/repo"));
        assert_eq!(event.kind, HistoryEventKind::ProviderTool);
        assert_eq!(event.role, Some(HistoryEventRole::Tool));
        assert_eq!(event.content.as_deref(), Some("called browser"));
        assert_eq!(
            event
                .metadata
                .get("merge_key")
                .and_then(|value| value.as_str()),
            Some("tool:browser")
        );
        let round_tripped = event
            .to_session_history_entry()
            .expect("transcript event should convert back");
        assert_eq!(round_tripped.kind, SessionHistoryEntryKind::ProviderTool);
        assert_eq!(round_tripped.text, "called browser");
        assert_eq!(round_tripped.merge_key.as_deref(), Some("tool:browser"));
    }

    #[test]
    fn operational_history_store_appends_and_loads_events_idempotently() {
        let path = std::env::temp_dir().join(format!(
            "arroba-operational-history-{}-{}.db",
            std::process::id(),
            super::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));

        let store = OperationalHistoryStore::open(path.clone())
            .expect("operational history store should open");
        let entry = SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hi");
        let event = HistoryEvent::transcript(
            1,
            &entry,
            HistoryEventTurnContext {
                provider: Some("opencode".to_string()),
                model: Some("gpt-5.2".to_string()),
                ..HistoryEventTurnContext::default()
            },
        );

        store
            .append(&event)
            .expect("event should append to operational history");
        store
            .append(&event)
            .expect("duplicate event should be ignored");

        let all_events = store
            .load_session_events("session-1", None)
            .expect("session events should load");
        assert_eq!(all_events.len(), 1);
        assert_eq!(all_events[0].event_id, event.event_id);
        assert_eq!(all_events[0].kind, HistoryEventKind::UserPrompt);
        assert_eq!(all_events[0].provider.as_deref(), Some("opencode"));

        let agent_events = store
            .load_session_events("session-1", Some("agent-1"))
            .expect("agent events should load");
        assert_eq!(agent_events.len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
