use std::collections::BTreeMap;

use crate::terminal::TerminalOutputKind;

use super::super::OpenCodeEventSubscription;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeOutputChunk {
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

pub(in crate::provider) struct OpenCodeEventDrainResult {
    pub chunks: Vec<OpenCodeOutputChunk>,
    pub completions: Vec<OpenCodeAssistantCompletion>,
    pub prompt_completed: bool,
    pub terminal_failure: Option<String>,
    pub notices: Vec<String>,
    pub resolved_model: Option<String>,
    pub resolved_model_source: Option<&'static str>,
    pub resolved_variant: Option<String>,
    pub resolved_usage_tokens_total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeAssistantCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug)]
pub(crate) struct OpenCodeRuntimeState {
    pub(super) base_url: String,
    pub(super) session_id: String,
    pub(super) emitted_text_offsets: BTreeMap<String, usize>,
    pub(super) emitted_tool_summaries: BTreeMap<String, String>,
    pub(super) buffered_text_deltas: BTreeMap<String, Vec<String>>,
    pub(super) message_roles: BTreeMap<String, String>,
    pub(super) part_kinds: BTreeMap<String, String>,
    pub(super) part_message_ids: BTreeMap<String, String>,
    pub(super) event_subscription: OpenCodeEventSubscription,
    pub(super) last_status_kind: Option<String>,
    pub(super) last_completed_assistant_message_id: Option<String>,
    pub(super) active_user_message_id: Option<String>,
}

impl OpenCodeRuntimeState {
    pub(in crate::provider) fn new(
        base_url: String,
        session_id: String,
        event_subscription: OpenCodeEventSubscription,
    ) -> Self {
        Self {
            base_url,
            session_id,
            emitted_text_offsets: BTreeMap::new(),
            emitted_tool_summaries: BTreeMap::new(),
            buffered_text_deltas: BTreeMap::new(),
            message_roles: BTreeMap::new(),
            part_kinds: BTreeMap::new(),
            part_message_ids: BTreeMap::new(),
            event_subscription,
            last_status_kind: None,
            last_completed_assistant_message_id: None,
            active_user_message_id: None,
        }
    }

    pub(in crate::provider) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(in crate::provider) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(in crate::provider) fn stop(self) {
        self.event_subscription.stop();
    }

    pub(in crate::provider) fn note_prompt_submitted(&mut self, user_message_id: String) {
        self.active_user_message_id = Some(user_message_id);
    }
}
