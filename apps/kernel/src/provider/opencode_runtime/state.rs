use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::terminal::TerminalOutputKind;

use super::super::{OpenCodeEventSubscription, OpenCodeMessage};
use crate::provider::opencode_client::OpenCodeSessionStatus;

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
    pub explicit_provider_error: bool,
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
    pub(super) emitted_text_by_part: BTreeMap<String, String>,
    pub(super) emitted_tool_summaries: BTreeMap<String, String>,
    pub(super) buffered_text_deltas: BTreeMap<String, Vec<String>>,
    pub(super) message_roles: BTreeMap<String, String>,
    pub(super) part_kinds: BTreeMap<String, String>,
    pub(super) part_message_ids: BTreeMap<String, String>,
    pub(super) message_parent_ids: BTreeMap<String, Option<String>>,
    pub(super) preexisting_message_ids: BTreeSet<String>,
    pub(super) event_subscription: OpenCodeEventSubscription,
    pub(super) last_status: Option<OpenCodeSessionStatus>,
    pub(super) completed_assistant_message_ids: BTreeSet<String>,
    pub(super) active_terminal_assistant_message_id: Option<String>,
    pub(super) active_user_message_id: Option<String>,
    active_prompt_submitted_at: Option<Instant>,
    pub(super) session_errors_require_prompt_match: bool,
    unmatched_session_error: Option<String>,
    unmatched_session_error_idle_since: Option<Instant>,
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
            emitted_text_by_part: BTreeMap::new(),
            emitted_tool_summaries: BTreeMap::new(),
            buffered_text_deltas: BTreeMap::new(),
            message_roles: BTreeMap::new(),
            part_kinds: BTreeMap::new(),
            part_message_ids: BTreeMap::new(),
            message_parent_ids: BTreeMap::new(),
            preexisting_message_ids: BTreeSet::new(),
            event_subscription,
            last_status: None,
            completed_assistant_message_ids: BTreeSet::new(),
            active_terminal_assistant_message_id: None,
            active_user_message_id: None,
            active_prompt_submitted_at: None,
            session_errors_require_prompt_match: false,
            unmatched_session_error: None,
            unmatched_session_error_idle_since: None,
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
        self.active_terminal_assistant_message_id = None;
        self.active_prompt_submitted_at = Some(Instant::now());
        self.unmatched_session_error = None;
        self.unmatched_session_error_idle_since = None;
    }

    pub(super) fn note_unmatched_session_error(&mut self, message: String) {
        if self.active_user_message_id.is_some() {
            self.unmatched_session_error = Some(message);
        }
    }

    pub(super) fn unmatched_session_error_after_idle_grace(
        &mut self,
        current_user_without_assistant_is_idle: bool,
        grace: Duration,
    ) -> Option<String> {
        if !current_user_without_assistant_is_idle {
            self.unmatched_session_error_idle_since = None;
            return None;
        }
        let message = self.unmatched_session_error.as_ref()?;
        let idle_since = self
            .unmatched_session_error_idle_since
            .get_or_insert_with(Instant::now);
        (idle_since.elapsed() >= grace).then(|| message.clone())
    }

    pub(super) fn active_prompt_has_elapsed(&self, duration: Duration) -> bool {
        self.active_prompt_submitted_at
            .is_some_and(|submitted_at| submitted_at.elapsed() >= duration)
    }

    pub(in crate::provider) fn baseline_existing_messages(&mut self, messages: &[OpenCodeMessage]) {
        self.preexisting_message_ids = messages
            .iter()
            .map(|message| message.info.id.clone())
            .collect();
        for message in messages {
            self.message_roles
                .insert(message.info.id.clone(), message.info.role.clone());
            self.message_parent_ids
                .insert(message.info.id.clone(), message.info.parent_id.clone());
            if message.info.role == "assistant" && message.info.time.completed.is_some() {
                self.completed_assistant_message_ids
                    .insert(message.info.id.clone());
            }
            for part in &message.parts {
                self.part_message_ids
                    .insert(part.id.clone(), part.message_id.clone());
                self.part_kinds.insert(part.id.clone(), part.kind.clone());
            }
        }
    }

    pub(in crate::provider) fn settle_aborted_turn(&mut self, messages: &[OpenCodeMessage]) {
        self.session_errors_require_prompt_match = true;
        while self.event_subscription.receiver.try_recv().is_ok() {}
        self.emitted_text_by_part.clear();
        self.emitted_tool_summaries.clear();
        self.buffered_text_deltas.clear();
        self.message_roles.clear();
        self.part_kinds.clear();
        self.part_message_ids.clear();
        self.message_parent_ids.clear();
        self.preexisting_message_ids.clear();
        self.completed_assistant_message_ids.clear();
        self.active_user_message_id = None;
        self.active_terminal_assistant_message_id = None;
        self.active_prompt_submitted_at = None;
        self.unmatched_session_error = None;
        self.unmatched_session_error_idle_since = None;
        self.last_status = Some("idle".into());
        self.baseline_existing_messages(messages);
    }

    pub(super) fn message_belongs_to_active_prompt(&self, message_id: &str) -> bool {
        let Some(active_user_message_id) = self.active_user_message_id.as_deref() else {
            return false;
        };
        if self.preexisting_message_ids.contains(message_id) {
            return false;
        }
        self.message_parent_ids
            .get(message_id)
            .and_then(|parent_id| parent_id.as_deref())
            .is_none_or(|parent_id| parent_id == active_user_message_id)
    }
}
