use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::Engine;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_observation: Option<SessionHistoryExternalObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<SessionHistoryPromptAttachment>,
    pub text: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryExternalObservation {
    pub settles_active_prompt: bool,
    pub passive_telemetry: bool,
}

impl SessionHistoryExternalObservation {
    pub fn useful(self) -> Option<Self> {
        (self.settles_active_prompt || self.passive_telemetry).then_some(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryPromptAttachment {
    pub url: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
}

impl SessionHistoryPromptAttachment {
    pub fn from_prompt_attachment(attachment: &crate::session::PromptAttachment) -> Self {
        Self {
            url: attachment.url().to_string(),
            mime: attachment.mime().to_string(),
            filename: attachment.filename().map(str::to_string),
            preview_url: image_prompt_attachment_preview_url(attachment),
        }
    }

    pub fn rehydrate_preview_url(&mut self) {
        if self.preview_url.is_some() || !self.mime.starts_with("image/") {
            return;
        }
        self.preview_url = image_attachment_preview_url_from_url(&self.url, &self.mime);
    }
}

fn image_prompt_attachment_preview_url(
    attachment: &crate::session::PromptAttachment,
) -> Option<String> {
    if !attachment.mime().starts_with("image/") {
        return None;
    }
    if let Some(contents_base64) = attachment.contents_base64() {
        return Some(format!(
            "data:{};base64,{contents_base64}",
            attachment.mime()
        ));
    }
    image_attachment_preview_url_from_url(attachment.url(), attachment.mime())
}

fn image_attachment_preview_url_from_url(url: &str, mime: &str) -> Option<String> {
    let local_path = local_file_url_path(url)?;
    let bytes = fs::read(local_path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn local_file_url_path(url: &str) -> Option<String> {
    if url.starts_with('/') {
        return Some(url.to_string());
    }
    let stripped = url
        .strip_prefix("file://localhost")
        .or_else(|| url.strip_prefix("file://"))?;
    stripped
        .starts_with('/')
        .then(|| percent_decode_path(stripped))
}

fn percent_decode_path(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hi = decode_hex_nibble(bytes[index + 1]);
            let lo = decode_hex_nibble(bytes[index + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                decoded.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| value.to_string())
}

fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionHistoryEntrySource {
    ExternalProviderObserved,
}

impl SessionHistoryEntrySource {
    pub const EXTERNAL_PROVIDER_OBSERVED_METADATA_LINE: &'static str = "external_provider_observed";

    pub fn metadata_line(self) -> &'static str {
        match self {
            Self::ExternalProviderObserved => Self::EXTERNAL_PROVIDER_OBSERVED_METADATA_LINE,
        }
    }

    pub fn metadata_text_contains(metadata_text: &str, source: Self) -> bool {
        metadata_text
            .lines()
            .any(|line| line == source.metadata_line())
    }

    pub fn metadata_text_contains_external_provider_observed(metadata_text: &str) -> bool {
        Self::metadata_text_contains(metadata_text, Self::ExternalProviderObserved)
    }
}

pub fn external_provider_observed_merge_key_prefix(
    provider: &str,
    provider_session_id: &str,
) -> String {
    format!("external:{provider}:{provider_session_id}:")
}

pub fn external_provider_observed_merge_key(
    provider: &str,
    provider_session_id: &str,
    provider_turn_id: &str,
) -> String {
    format!(
        "{}{}",
        external_provider_observed_merge_key_prefix(provider, provider_session_id),
        provider_turn_id
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalProviderObservedId {
    pub provider: String,
    pub provider_session_id: String,
    pub provider_turn_id: String,
}

pub fn parse_external_provider_observed_id(value: &str) -> Option<ExternalProviderObservedId> {
    let mut parts = value.splitn(4, ':');
    let marker = parts.next()?;
    if marker != "external" {
        return None;
    }
    let provider = parts.next()?.trim();
    let provider_session_id = parts.next()?.trim();
    let provider_turn_id = parts.next()?.trim();
    if provider.is_empty() || provider_session_id.is_empty() || provider_turn_id.is_empty() {
        return None;
    }
    Some(ExternalProviderObservedId {
        provider: provider.to_string(),
        provider_session_id: provider_session_id.to_string(),
        provider_turn_id: provider_turn_id.to_string(),
    })
}

pub fn external_provider_observed_state_merge_key(
    provider: &str,
    provider_session_id: &str,
    reason: &str,
    latest_merge_key: &str,
) -> String {
    format!(
        "{}state:{reason}:{latest_merge_key}",
        external_provider_observed_merge_key_prefix(provider, provider_session_id)
    )
}

#[cfg(test)]
pub fn external_provider_observed_merge_key_is_state_signal(
    provider: &str,
    provider_session_id: &str,
    value: &str,
) -> bool {
    external_provider_observed_merge_key_with_prefix_is_state_signal(
        &external_provider_observed_merge_key_prefix(provider, provider_session_id),
        value,
    )
}

pub fn external_provider_observed_merge_key_with_prefix_is_state_signal(
    external_merge_key_prefix: &str,
    value: &str,
) -> bool {
    let prefix = external_merge_key_prefix
        .strip_suffix(':')
        .unwrap_or(external_merge_key_prefix);
    value.starts_with(&format!("{prefix}:state:"))
}

impl SessionHistoryEntry {
    pub fn is_external_provider_observed(&self) -> bool {
        self.source == Some(SessionHistoryEntrySource::ExternalProviderObserved)
    }

    pub fn external_provider_observed_turn_id(&self) -> Option<&str> {
        self.is_external_provider_observed()
            .then_some(self.external_provider_turn_id.as_deref())
            .flatten()
    }

    pub fn user_prompt(
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        text: impl Into<String>,
    ) -> Self {
        Self::user_prompt_with_attachments(session_id, source_attachment_id, agent_id, text, &[])
    }

    pub fn user_prompt_with_attachments(
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        text: impl Into<String>,
        attachments: &[crate::session::PromptAttachment],
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
            external_observation: None,
            attachments: attachments
                .iter()
                .map(SessionHistoryPromptAttachment::from_prompt_attachment)
                .collect(),
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
            external_observation: None,
            attachments: Vec::new(),
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
            external_observation: None,
            attachments: Vec::new(),
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
        Self::external_provider_observed_with_merge_key(
            session_id,
            provider_run_id,
            agent_id,
            kind,
            text,
            provider,
            provider_session_id,
            provider_turn_id.as_ref().map(|turn_id| {
                external_provider_observed_merge_key(provider, provider_session_id, turn_id)
            }),
            provider_turn_id,
            observed_at_ms,
        )
    }

    pub fn external_provider_observed_with_merge_key(
        session_id: &str,
        provider_run_id: Option<&str>,
        agent_id: &str,
        kind: SessionHistoryEntryKind,
        text: impl Into<String>,
        provider: &str,
        provider_session_id: &str,
        merge_key: Option<String>,
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
            merge_key,
            source: Some(SessionHistoryEntrySource::ExternalProviderObserved),
            external_provider: Some(provider.to_string()),
            external_provider_session_id: Some(provider_session_id.to_string()),
            external_provider_turn_id: provider_turn_id,
            observed_at_ms: Some(observed_at_ms),
            external_observation: None,
            attachments: Vec::new(),
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
            && !entry.is_external_provider_observed()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_provider_observed_merge_keys_are_centralized() {
        assert_eq!(
            external_provider_observed_merge_key_prefix("codex", "thread-1"),
            "external:codex:thread-1:"
        );
        assert_eq!(
            external_provider_observed_merge_key("codex", "thread-1", "item-1"),
            "external:codex:thread-1:item-1"
        );
        assert_eq!(
            parse_external_provider_observed_id("external: codex : thread-1 : item-1"),
            Some(ExternalProviderObservedId {
                provider: "codex".to_string(),
                provider_session_id: "thread-1".to_string(),
                provider_turn_id: "item-1".to_string(),
            })
        );
        assert_eq!(parse_external_provider_observed_id("external:codex"), None);
        assert_eq!(parse_external_provider_observed_id("prompt-1"), None);
        assert_eq!(
            external_provider_observed_state_merge_key(
                "codex",
                "thread-1",
                "settled",
                "external:codex:thread-1:item-1"
            ),
            "external:codex:thread-1:state:settled:external:codex:thread-1:item-1"
        );
        assert!(external_provider_observed_merge_key_is_state_signal(
            "codex",
            "thread-1",
            "external:codex:thread-1:state:settled:external:codex:thread-1:item-1"
        ));
        assert!(!external_provider_observed_merge_key_is_state_signal(
            "codex",
            "thread-1",
            "external:codex:thread-1:item-1"
        ));
    }
}
