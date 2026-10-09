//! Codex runtime state and poll result types.

use std::collections::BTreeMap;

use crate::provider::{CodexNotification, CodexRunSelection, CodexSocket, ProviderRunTokenUsage};
use crate::terminal::TerminalOutputKind;

use super::backfill::CodexAuthoritativeBackfillGate;
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
    /// Read-only discovery must keep its permission and MCP policy when the
    /// event drain reconstructs a client for server requests.
    read_only_discovery_permissions: bool,
    pub(super) ephemeral: bool,
    pub(super) socket: CodexSocket,
    pub(super) next_request_id: u64,
    pub(super) buffered_notifications: Vec<CodexNotification>,
    pub(super) active_turn_id: Option<String>,
    pub(crate) native_approval_origin: Option<crate::session::NativeInteractionOrigin>,
    pub(super) turn_tracker: CodexTurnTracker,
    pub(super) authoritative_backfill_gate: CodexAuthoritativeBackfillGate,
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
                "read_only_discovery_permissions",
                &self.read_only_discovery_permissions,
            )
            .field("next_request_id", &self.next_request_id)
            .field("buffered_notifications", &self.buffered_notifications)
            .field("active_turn_id", &self.active_turn_id)
            .field("turn_tracker", &self.turn_tracker)
            .field(
                "authoritative_backfill_gate",
                &self.authoritative_backfill_gate,
            )
            .field("text_items", &self.text_items)
            .field("tool_items", &self.tool_items)
            .finish()
    }
}

impl CodexRuntimeState {
    #[cfg(test)]
    pub(crate) fn active_turn_binding_fixture(
        endpoint: String,
        socket: CodexSocket,
    ) -> crate::provider::ProviderRuntimeBinding {
        let mut state = Self::new(endpoint, "notification-thread".into(), socket, 1);
        state.active_turn_id = Some("notification-turn".into());
        crate::provider::ProviderRuntimeBinding::Codex(CodexRuntimeBinding {
            state,
            selection: CodexRunSelection {
                model: None,
                variant: None,
            },
        })
    }

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
            read_only_discovery_permissions: false,
            ephemeral: false,
            socket,
            next_request_id,
            buffered_notifications: Vec::new(),
            active_turn_id: None,
            native_approval_origin: None,
            turn_tracker: CodexTurnTracker::default(),
            authoritative_backfill_gate: CodexAuthoritativeBackfillGate::default(),
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
            read_only_discovery_permissions: false,
            ephemeral: false,
            socket,
            next_request_id,
            buffered_notifications: Vec::new(),
            active_turn_id: None,
            native_approval_origin: None,
            turn_tracker: CodexTurnTracker::default(),
            authoritative_backfill_gate: CodexAuthoritativeBackfillGate::default(),
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

    pub(super) fn read_only_discovery_permissions(&self) -> bool {
        self.read_only_discovery_permissions
    }

    pub(super) fn set_read_only_discovery_permissions(&mut self, enabled: bool) {
        self.read_only_discovery_permissions = enabled;
    }

    pub(super) fn mark_thread_ready(&mut self, thread_id: impl Into<String>) {
        self.thread_id = thread_id.into();
        self.thread_ready = true;
    }
}

pub struct CodexRuntimeBinding {
    pub state: CodexRuntimeState,
    pub selection: CodexRunSelection,
}
