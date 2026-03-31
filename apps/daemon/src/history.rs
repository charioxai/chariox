use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::session::RuntimeSession;
use crate::terminal::TerminalOutputKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl SessionHistoryStore {
    pub fn new(root: PathBuf) -> Result<Self, DaemonError> {
        fs::create_dir_all(&root).map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: None,
            operation: "create session history directory",
            message: error.to_string(),
        })?;
        Ok(Self { root })
    }

    pub fn load(&self, session: &RuntimeSession) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
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

    use super::{SessionHistoryEntry, SessionHistoryEntryKind, SessionHistoryStore};

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
}
