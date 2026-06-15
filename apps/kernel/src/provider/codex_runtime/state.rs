//! Codex runtime state and poll result types.

use std::collections::BTreeMap;

use crate::provider::{CodexNotification, CodexRunSelection, CodexSocket, ProviderRunTokenUsage};
use crate::terminal::TerminalOutputKind;

use super::transcript::{CodexTextTranscriptState, CodexToolTranscriptState};
use super::turn::CodexTurnTracker;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexPollResult {
    pub chunks: Vec<CodexOutputChunk>,
    pub completions: Vec<CodexAssistantCompletion>,
    pub prompt_completed: bool,
    pub terminal_failure: Option<String>,
    pub notices: Vec<String>,
    pub resolved_usage: Option<ProviderRunTokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexOutputChunk {
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAssistantCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
}

pub struct CodexRuntimeState {
    endpoint: String,
    thread_id: String,
    thread_ready: bool,
    developer_instructions_fingerprint: Option<String>,
    context_hot_reload_enabled: bool,
    pub(super) socket: CodexSocket,
    pub(super) next_request_id: u64,
    pub(super) buffered_notifications: Vec<CodexNotification>,
    pub(super) active_turn_id: Option<String>,
    pub(super) turn_tracker: CodexTurnTracker,
    pub(super) text_items: BTreeMap<String, CodexTextTranscriptState>,
    pub(super) tool_items: BTreeMap<String, CodexToolTranscriptState>,
}

impl std::fmt::Debug for CodexRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexRuntimeState")
            .field("endpoint", &self.endpoint)
            .field("thread_id", &self.thread_id)
            .field("thread_ready", &self.thread_ready)
            .field(
                "developer_instructions_fingerprint",
                &self.developer_instructions_fingerprint,
            )
            .field(
                "context_hot_reload_enabled",
                &self.context_hot_reload_enabled,
            )
            .field("next_request_id", &self.next_request_id)
            .field("buffered_notifications", &self.buffered_notifications)
            .field("active_turn_id", &self.active_turn_id)
            .field("turn_tracker", &self.turn_tracker)
            .field("text_items", &self.text_items)
            .field("tool_items", &self.tool_items)
            .finish()
    }
}

impl CodexRuntimeState {
    pub(super) fn new(
        endpoint: String,
        thread_id: String,
        socket: CodexSocket,
        next_request_id: u64,
    ) -> Self {
        Self {
            endpoint,
            thread_id,
            thread_ready: true,
            developer_instructions_fingerprint: None,
            context_hot_reload_enabled: false,
            socket,
            next_request_id,
            buffered_notifications: Vec::new(),
            active_turn_id: None,
            turn_tracker: CodexTurnTracker::default(),
            text_items: BTreeMap::new(),
            tool_items: BTreeMap::new(),
        }
    }

    pub(super) fn pending(
        endpoint: String,
        thread_id: Option<String>,
        socket: CodexSocket,
        next_request_id: u64,
    ) -> Self {
        Self {
            endpoint,
            thread_id: thread_id.unwrap_or_default(),
            thread_ready: false,
            developer_instructions_fingerprint: None,
            context_hot_reload_enabled: true,
            socket,
            next_request_id,
            buffered_notifications: Vec::new(),
            active_turn_id: None,
            turn_tracker: CodexTurnTracker::default(),
            text_items: BTreeMap::new(),
            tool_items: BTreeMap::new(),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub(super) fn pending_thread_id(&self) -> Option<&str> {
        (!self.thread_id.trim().is_empty()).then_some(self.thread_id.as_str())
    }

    pub(super) fn thread_ready(&self) -> bool {
        self.thread_ready
    }

    pub(super) fn developer_instructions_fingerprint(&self) -> Option<&str> {
        self.developer_instructions_fingerprint.as_deref()
    }

    pub(super) fn context_hot_reload_enabled(&self) -> bool {
        self.context_hot_reload_enabled
    }

    pub(super) fn mark_thread_ready(
        &mut self,
        thread_id: impl Into<String>,
        developer_instructions_fingerprint: Option<String>,
    ) {
        self.thread_id = thread_id.into();
        self.thread_ready = true;
        self.developer_instructions_fingerprint = developer_instructions_fingerprint;
    }

    pub(super) fn replace_thread(
        &mut self,
        thread_id: impl Into<String>,
        developer_instructions_fingerprint: Option<String>,
    ) {
        self.thread_id = thread_id.into();
        self.thread_ready = true;
        self.developer_instructions_fingerprint = developer_instructions_fingerprint;
        self.buffered_notifications.clear();
        self.active_turn_id = None;
        self.turn_tracker = CodexTurnTracker::default();
        self.text_items.clear();
        self.tool_items.clear();
    }
}

pub struct CodexRuntimeBinding {
    pub state: CodexRuntimeState,
    pub selection: CodexRunSelection,
}
