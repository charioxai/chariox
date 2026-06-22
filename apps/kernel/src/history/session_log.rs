use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::session::RuntimeSession;
use crate::terminal::TerminalOutputKind;

const SESSION_HISTORY_HARD_MAX_BYTES: u64 = 500 * 1024 * 1024;
const SESSION_HISTORY_PRUNE_TARGET_BYTES: u64 = 450 * 1024 * 1024;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SessionHistoryEntrySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    pub text: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionHistoryEntrySource {
    ExternalProviderObserved,
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
            source: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            observed_at_ms: None,
            text: text.into(),
            timestamp_ms: super::unix_epoch_ms(),
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
            source: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            observed_at_ms: None,
            text: text.into(),
            timestamp_ms: super::unix_epoch_ms(),
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
            source: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            observed_at_ms: None,
            text: text.into(),
            timestamp_ms: super::unix_epoch_ms(),
        }
    }

    pub fn external_provider_observed(
        session_id: &str,
        provider_run_id: Option<&str>,
        agent_id: &str,
        kind: SessionHistoryEntryKind,
        text: impl Into<String>,
        provider: &str,
        provider_session_id: &str,
        provider_turn_id: Option<String>,
        observed_at_ms: Option<u64>,
    ) -> Self {
        let observed_at_ms = observed_at_ms.unwrap_or_else(super::unix_epoch_ms);
        Self {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.map(str::to_string),
            agent_id: Some(agent_id.to_string()),
            source_attachment_id: None,
            kind,
            merge_key: provider_turn_id
                .as_ref()
                .map(|turn_id| format!("external:{provider}:{provider_session_id}:{turn_id}")),
            source: Some(SessionHistoryEntrySource::ExternalProviderObserved),
            external_provider: Some(provider.to_string()),
            external_provider_session_id: Some(provider_session_id.to_string()),
            external_provider_turn_id: provider_turn_id,
            observed_at_ms: Some(observed_at_ms),
            text: text.into(),
            timestamp_ms: observed_at_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionHistoryStore {
    root: PathBuf,
    read_delay_ms: u64,
    write_lock: Arc<Mutex<()>>,
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
            write_lock: Arc::new(Mutex::new(())),
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
            && entry.source != Some(SessionHistoryEntrySource::ExternalProviderObserved)
        {
            return Err(DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "append session history",
                message: "user prompt history entry must include source attachment".to_string(),
            });
        }

        let path = self.path_for_session(session);
        let _guard = self
            .write_lock
            .lock()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "lock session history append",
                message: error.to_string(),
            })?;
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
            })?;
        drop(file);
        self.enforce_size_budget(session, &path)
    }

    pub fn replace_by_merge_key(
        &self,
        session: &RuntimeSession,
        merge_key: &str,
        entry: &SessionHistoryEntry,
    ) -> Result<bool, DaemonError> {
        let path = self.path_for_session(session);
        if !path.exists() {
            return Ok(false);
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "lock session history replace",
                message: error.to_string(),
            })?;
        let input = fs::File::open(&path).map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: Some(session.id().to_string()),
            operation: "open session history for replace",
            message: error.to_string(),
        })?;
        let mut entries = Vec::new();
        let mut replaced = false;
        for line in BufReader::new(input).lines() {
            let line = line.map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "read session history for replace",
                message: error.to_string(),
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let mut decoded =
                serde_json::from_str::<SessionHistoryEntry>(&line).map_err(|error| {
                    DaemonError::SessionHistoryFailed {
                        session_id: Some(session.id().to_string()),
                        operation: "decode session history for replace",
                        message: error.to_string(),
                    }
                })?;
            if decoded.merge_key.as_deref() == Some(merge_key) {
                decoded = entry.clone();
                replaced = true;
            }
            entries.push(decoded);
        }
        if !replaced {
            return Ok(false);
        }
        let temp_path = path.with_extension("jsonl.replace");
        let mut output =
            fs::File::create(&temp_path).map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "create replacement session history",
                message: error.to_string(),
            })?;
        for entry in entries {
            let encoded = serde_json::to_string(&entry).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: Some(session.id().to_string()),
                    operation: "encode replacement session history",
                    message: error.to_string(),
                }
            })?;
            output
                .write_all(encoded.as_bytes())
                .and_then(|_| output.write_all(b"\n"))
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session.id().to_string()),
                    operation: "write replacement session history",
                    message: error.to_string(),
                })?;
        }
        output
            .flush()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "flush replacement session history",
                message: error.to_string(),
            })?;
        fs::rename(&temp_path, &path).map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: Some(session.id().to_string()),
            operation: "replace session history",
            message: error.to_string(),
        })?;
        Ok(true)
    }

    pub fn path_for_session(&self, session: &RuntimeSession) -> PathBuf {
        self.root.join(history_file_name(session))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn enforce_size_budget(
        &self,
        session: &RuntimeSession,
        path: &Path,
    ) -> Result<(), DaemonError> {
        let Ok(metadata) = fs::metadata(path) else {
            return Ok(());
        };
        if metadata.len() <= SESSION_HISTORY_HARD_MAX_BYTES {
            return Ok(());
        }
        let bytes_to_skip = metadata
            .len()
            .saturating_sub(SESSION_HISTORY_PRUNE_TARGET_BYTES);
        let temp_path = path.with_extension("jsonl.prune");
        let input = fs::File::open(path).map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: Some(session.id().to_string()),
            operation: "open session history for prune",
            message: error.to_string(),
        })?;
        let mut output =
            fs::File::create(&temp_path).map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "create pruned session history",
                message: error.to_string(),
            })?;
        let mut skipped = 0_u64;
        let mut deleted_lines = 0_u64;
        for line in BufReader::new(input).split(b'\n') {
            let mut line = line.map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "read session history for prune",
                message: error.to_string(),
            })?;
            line.push(b'\n');
            if skipped < bytes_to_skip {
                skipped = skipped.saturating_add(line.len() as u64);
                deleted_lines = deleted_lines.saturating_add(1);
                continue;
            }
            output
                .write_all(&line)
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session.id().to_string()),
                    operation: "write pruned session history",
                    message: error.to_string(),
                })?;
        }
        output
            .flush()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: Some(session.id().to_string()),
                operation: "flush pruned session history",
                message: error.to_string(),
            })?;
        fs::rename(&temp_path, path).map_err(|error| DaemonError::SessionHistoryFailed {
            session_id: Some(session.id().to_string()),
            operation: "replace pruned session history",
            message: error.to_string(),
        })?;
        crate::logging::warn_with_fields(
            "daemon.history",
            "pruned session history to enforce hard size budget",
            serde_json::json!({
                "session_id": session.id(),
                "path": path.display().to_string(),
                "deleted_lines": deleted_lines,
                "previous_size_bytes": metadata.len(),
                "max_size_bytes": SESSION_HISTORY_HARD_MAX_BYTES,
                "target_size_bytes": SESSION_HISTORY_PRUNE_TARGET_BYTES,
            }),
        );
        Ok(())
    }
}

fn history_file_name(session: &RuntimeSession) -> String {
    format!("{}-{}.jsonl", session.id(), session.created_at_ms())
}
