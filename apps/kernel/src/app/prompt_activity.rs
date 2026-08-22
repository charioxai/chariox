use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use crate::runtime::workspace_coordinator::{WorkspaceClaimGuard, WorkspaceOperationClaimSnapshot};
use crate::session::{PromptOrigin, PromptQueueItem};

#[derive(Debug, Clone)]
pub(crate) struct ActivePromptState {
    pub(crate) last_output_at: Option<Instant>,
    pub(crate) saw_response_content: bool,
    pub(crate) completion_recorded: bool,
    pub(crate) settlement_requested: bool,
    pub(crate) active_tool_ids: BTreeSet<String>,
}

impl ActivePromptState {
    pub(crate) fn request_settlement(&mut self) {
        if self.settlement_requested {
            return;
        }
        self.last_output_at = Some(Instant::now());
        self.saw_response_content = true;
        self.settlement_requested = true;
    }

    pub(crate) fn observe_provider_tool(&mut self, merge_key: Option<&str>, bytes: &[u8]) {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
            return;
        };
        let Some(record) = value.as_object() else {
            return;
        };
        let status = record
            .get("status")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                record
                    .get("state")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|state| state.get("status"))
                    .and_then(serde_json::Value::as_str)
            })
            .map(|status| status.to_ascii_lowercase());
        let Some(status) = status else {
            return;
        };
        let id = ["id", "call_id", "tool_call_id"]
            .into_iter()
            .find_map(|field| record.get(field).and_then(serde_json::Value::as_str))
            .or(merge_key)
            .filter(|id| !id.is_empty());
        let Some(id) = id else {
            return;
        };
        if matches!(
            status.as_str(),
            "pending" | "queued" | "started" | "running" | "in_progress" | "inprogress" | "waiting"
        ) {
            self.active_tool_ids.insert(id.to_string());
        } else if matches!(
            status.as_str(),
            "completed"
                | "complete"
                | "succeeded"
                | "success"
                | "error"
                | "failed"
                | "declined"
                | "cancelled"
                | "canceled"
        ) {
            self.active_tool_ids.remove(id);
        }
    }

    pub(crate) fn has_active_provider_tools(&self) -> bool {
        !self.active_tool_ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActiveTurnPhase {
    Accepted,
    AwaitingFirstOutput,
    Streaming,
    Settling,
}

impl ActiveTurnPhase {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::AwaitingFirstOutput => "awaiting_first_output",
            Self::Streaming => "streaming",
            Self::Settling => "settling",
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Self::Accepted => 0,
            Self::AwaitingFirstOutput => 1,
            Self::Streaming => 2,
            Self::Settling => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveTurnState {
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) prompt_id: String,
    pub(crate) source_attachment_id: Option<String>,
    pub(crate) prompt_origin: Option<PromptOrigin>,
    pub(crate) external_observed_id: Option<crate::history::ExternalProviderObservedId>,
    pub(crate) provider_run_id: String,
    pub(crate) trace_id: String,
    pub(crate) started_at_ms: u64,
    pub(crate) phase: ActiveTurnPhase,
    pub(crate) settlement_requested: bool,
}

impl ActiveTurnState {
    pub(crate) fn new(
        session_id: String,
        agent_id: String,
        prompt_id: String,
        provider_run_id: String,
    ) -> Self {
        let trace_id = prompt_id.clone();
        Self {
            session_id,
            agent_id,
            prompt_id,
            source_attachment_id: None,
            prompt_origin: None,
            external_observed_id: None,
            provider_run_id,
            trace_id,
            started_at_ms: crate::session::unix_epoch_ms(),
            phase: ActiveTurnPhase::Accepted,
            settlement_requested: false,
        }
    }

    pub(crate) fn with_phase(mut self, phase: ActiveTurnPhase) -> Self {
        self.phase = phase;
        self
    }

    pub(crate) fn with_prompt_metadata(mut self, prompt: &PromptQueueItem) -> Self {
        self.source_attachment_id = Some(prompt.source_attachment_id().to_string());
        self.prompt_origin = Some(prompt.prompt_origin());
        self.external_observed_id = prompt.external_observed_id();
        self
    }

    pub(crate) fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        let trace_id = trace_id.into();
        if !trace_id.trim().is_empty() {
            self.trace_id = trace_id;
        }
        self
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ActiveTurnStore {
    inner: Arc<Mutex<BTreeMap<String, ActiveTurnState>>>,
}

impl ActiveTurnStore {
    pub(crate) fn start(&self, turn: ActiveTurnState) {
        let (turn, replaced) = {
            let mut guard = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let turn = if let Some(existing) = guard.get(&turn.provider_run_id) {
                merge_active_turn_start(existing, turn)
            } else {
                turn
            };
            let replaced_provider_run_ids = guard
                .iter()
                .filter_map(|(provider_run_id, existing)| {
                    (provider_run_id != &turn.provider_run_id
                        && existing.session_id == turn.session_id
                        && existing.agent_id == turn.agent_id)
                        .then(|| provider_run_id.clone())
                })
                .collect::<Vec<_>>();
            let replaced = replaced_provider_run_ids
                .into_iter()
                .filter_map(|provider_run_id| guard.remove(&provider_run_id))
                .collect::<Vec<_>>();
            guard.insert(turn.provider_run_id.clone(), turn.clone());
            (turn, replaced)
        };
        for replaced_turn in replaced {
            record_active_turn_clear(replaced_turn);
        }
        record_active_turn_event(&turn, "active_turn_start", turn.settlement_requested);
    }

    pub(crate) fn mark_awaiting_first_output(&self, provider_run_id: &str) {
        self.advance_phase(
            provider_run_id,
            ActiveTurnPhase::AwaitingFirstOutput,
            "active_turn_awaiting_first_output",
        );
    }

    pub(crate) fn mark_streaming(&self, provider_run_id: &str) {
        self.advance_phase(
            provider_run_id,
            ActiveTurnPhase::Streaming,
            "active_turn_streaming",
        );
    }

    pub(crate) fn mark_settling(&self, provider_run_id: &str) {
        if let Some(turn) = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(provider_run_id)
        {
            if turn.phase.rank() < ActiveTurnPhase::Settling.rank() {
                turn.phase = ActiveTurnPhase::Settling;
            }
            turn.settlement_requested = true;
            record_active_turn_event(turn, "active_turn_mark_settling", true);
        }
    }

    fn advance_phase(&self, provider_run_id: &str, phase: ActiveTurnPhase, event: &str) {
        if let Some(turn) = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(provider_run_id)
        {
            if turn.phase.rank() < phase.rank() {
                turn.phase = phase;
            }
            record_active_turn_event(turn, event, turn.settlement_requested);
        }
    }

    pub(crate) fn clear(&self, provider_run_id: &str) {
        let removed = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(provider_run_id);
        if let Some(turn) = removed {
            record_active_turn_clear(turn);
        }
    }

    pub(crate) fn get(&self, provider_run_id: &str) -> Option<ActiveTurnState> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(provider_run_id)
            .cloned()
    }

    pub(crate) fn clear_session(&self, session_id: &str) -> usize {
        self.clear_matching(|turn| turn.session_id == session_id)
    }

    pub(crate) fn clear_agent(&self, session_id: &str, agent_id: &str) -> usize {
        self.clear_matching(|turn| turn.session_id == session_id && turn.agent_id == agent_id)
    }

    fn clear_matching(&self, mut predicate: impl FnMut(&ActiveTurnState) -> bool) -> usize {
        let removed = {
            let mut guard = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let provider_run_ids = guard
                .iter()
                .filter_map(|(provider_run_id, turn)| {
                    predicate(turn).then(|| provider_run_id.clone())
                })
                .collect::<Vec<_>>();
            provider_run_ids
                .into_iter()
                .filter_map(|provider_run_id| guard.remove(&provider_run_id))
                .collect::<Vec<_>>()
        };
        let removed_count = removed.len();
        for turn in removed {
            record_active_turn_clear(turn);
        }
        removed_count
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<String, ActiveTurnState> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

fn record_active_turn_clear(turn: ActiveTurnState) {
    record_active_turn_event(&turn, "active_turn_clear", turn.settlement_requested);
}

fn record_active_turn_event(turn: &ActiveTurnState, event: &str, settlement_requested: bool) {
    crate::debug_trace::record_terminal_turn(
        &turn.session_id,
        event,
        serde_json::json!({
            "agent_id": &turn.agent_id,
            "prompt_id": &turn.prompt_id,
            "source_attachment_id": turn.source_attachment_id.as_deref(),
            "prompt_origin": turn.prompt_origin.map(prompt_origin_label),
            "external_provider": turn.external_observed_id.as_ref().map(|metadata| metadata.provider.as_str()),
            "external_provider_session_id": turn.external_observed_id.as_ref().map(|metadata| metadata.provider_session_id.as_str()),
            "external_provider_turn_id": turn.external_observed_id.as_ref().map(|metadata| metadata.provider_turn_id.as_str()),
            "provider_run_id": &turn.provider_run_id,
            "trace_id": &turn.trace_id,
            "started_at_ms": turn.started_at_ms,
            "phase": turn.phase.as_str(),
            "settlement_requested": settlement_requested,
        }),
    );
}

fn prompt_origin_label(prompt_origin: PromptOrigin) -> &'static str {
    match prompt_origin {
        PromptOrigin::Chariox => "chariox",
        PromptOrigin::External => "external",
    }
}

fn merge_active_turn_start(
    existing: &ActiveTurnState,
    mut incoming: ActiveTurnState,
) -> ActiveTurnState {
    if existing.prompt_id == incoming.prompt_id {
        incoming.started_at_ms = incoming.started_at_ms.min(existing.started_at_ms);
        if existing.trace_id != existing.prompt_id && incoming.trace_id == incoming.prompt_id {
            incoming.trace_id = existing.trace_id.clone();
        }
        if existing.phase.rank() > incoming.phase.rank() {
            incoming.phase = existing.phase.clone();
        }
        if incoming.prompt_origin.is_none() {
            incoming.prompt_origin = existing.prompt_origin;
        }
        if incoming.source_attachment_id.is_none() {
            incoming.source_attachment_id = existing.source_attachment_id.clone();
        }
        if incoming.external_observed_id.is_none() {
            incoming.external_observed_id = existing.external_observed_id.clone();
        }
        incoming.settlement_requested |= existing.settlement_requested;
        if incoming.settlement_requested && incoming.phase.rank() < ActiveTurnPhase::Settling.rank()
        {
            incoming.phase = ActiveTurnPhase::Settling;
        }
    }
    incoming
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptActivityStore {
    inner: Arc<Mutex<BTreeMap<String, ActivePromptState>>>,
}

impl PromptActivityStore {
    pub(crate) fn read(&self) -> MutexGuard<'_, BTreeMap<String, ActivePromptState>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn write(&self) -> MutexGuard<'_, BTreeMap<String, ActivePromptState>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptWorkspaceClaimStore {
    inner: Arc<Mutex<BTreeMap<String, WorkspaceClaimGuard>>>,
}

impl PromptWorkspaceClaimStore {
    pub(crate) fn contains(&self, provider_run_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(provider_run_id)
    }

    pub(crate) fn insert(&self, provider_run_id: String, claim: WorkspaceClaimGuard) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(provider_run_id, claim);
    }

    pub(crate) fn remove(&self, provider_run_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(provider_run_id)
            .is_some()
    }

    pub(crate) fn remove_matching(
        &self,
        mut predicate: impl FnMut(&WorkspaceOperationClaimSnapshot) -> bool,
    ) -> usize {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let provider_run_ids = guard
            .iter()
            .filter_map(|(provider_run_id, claim)| {
                claim
                    .snapshot()
                    .filter(|snapshot| predicate(snapshot))
                    .map(|_| provider_run_id.clone())
            })
            .collect::<Vec<_>>();
        let removed = provider_run_ids.len();
        for provider_run_id in provider_run_ids {
            guard.remove(&provider_run_id);
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_settlement_requests_preserve_the_quiet_window() {
        let mut activity = ActivePromptState {
            last_output_at: None,
            saw_response_content: false,
            completion_recorded: true,
            settlement_requested: false,
            active_tool_ids: BTreeSet::new(),
        };

        activity.request_settlement();
        let first_requested_at = activity
            .last_output_at
            .expect("first settlement request should start the quiet window");
        std::thread::sleep(std::time::Duration::from_millis(2));
        activity.request_settlement();

        assert_eq!(activity.last_output_at, Some(first_requested_at));
        assert!(activity.saw_response_content);
        assert!(activity.settlement_requested);
    }

    #[test]
    fn provider_tool_activity_tracks_running_calls_until_their_terminal_update() {
        let mut activity = ActivePromptState {
            last_output_at: None,
            saw_response_content: true,
            completion_recorded: false,
            settlement_requested: false,
            active_tool_ids: BTreeSet::new(),
        };

        activity.observe_provider_tool(
            Some("tool-1"),
            br#"{"id":"tool-1","tool":"bash","status":"running"}"#,
        );
        assert!(activity.has_active_provider_tools());

        activity.observe_provider_tool(
            Some("tool-1"),
            br#"{"id":"tool-1","tool":"bash","status":"completed"}"#,
        );
        assert!(!activity.has_active_provider_tools());
    }

    #[test]
    fn active_turn_start_does_not_infer_external_metadata_from_prompt_ids() {
        let turn = ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "external:codex:thread-1:user-1".to_string(),
            "run-1".to_string(),
        );

        assert_eq!(turn.prompt_origin, None);
        assert_eq!(turn.external_observed_id, None);
    }

    #[test]
    fn active_turn_uses_dispatch_timestamp_and_keeps_it_for_same_run() {
        let prompt = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "hello",
            crate::session::PromptStatus::Running,
        );
        let accepted_at_ms = prompt.created_at_ms();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let store = ActiveTurnStore::default();
        let dispatched = ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            prompt.id().to_string(),
            "run-1".to_string(),
        )
        .with_prompt_metadata(&prompt);
        let dispatched_at_ms = dispatched.started_at_ms;
        store.start(dispatched);
        std::thread::sleep(std::time::Duration::from_millis(10));
        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            prompt.id().to_string(),
            "run-1".to_string(),
        ));

        let turn = store.get("run-1").expect("turn should remain active");
        assert!(dispatched_at_ms > accepted_at_ms);
        assert_eq!(turn.started_at_ms, dispatched_at_ms);
    }

    #[test]
    fn active_turn_restart_preserves_existing_command_trace() {
        let store = ActiveTurnStore::default();
        store.start(
            ActiveTurnState::new(
                "session-1".to_string(),
                "agent-1".to_string(),
                "prompt-1".to_string(),
                "run-1".to_string(),
            )
            .with_trace_id("trace-1"),
        );
        let started_at_ms = store
            .snapshot()
            .get("run-1")
            .expect("turn should be active")
            .started_at_ms;

        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "prompt-1".to_string(),
            "run-1".to_string(),
        ));

        let turn = store
            .snapshot()
            .remove("run-1")
            .expect("turn should remain active");
        assert_eq!(turn.trace_id, "trace-1");
        assert_eq!(turn.started_at_ms, started_at_ms);
        assert_eq!(turn.phase, ActiveTurnPhase::Accepted);
    }

    #[test]
    fn active_turn_restart_does_not_regress_phase() {
        let store = ActiveTurnStore::default();
        store.start(
            ActiveTurnState::new(
                "session-1".to_string(),
                "agent-1".to_string(),
                "prompt-1".to_string(),
                "run-1".to_string(),
            )
            .with_phase(ActiveTurnPhase::Streaming),
        );

        store.start(
            ActiveTurnState::new(
                "session-1".to_string(),
                "agent-1".to_string(),
                "prompt-1".to_string(),
                "run-1".to_string(),
            )
            .with_phase(ActiveTurnPhase::AwaitingFirstOutput),
        );

        let turn = store
            .snapshot()
            .remove("run-1")
            .expect("turn should remain active");
        assert_eq!(turn.phase, ActiveTurnPhase::Streaming);
    }

    #[test]
    fn active_turn_restart_preserves_prompt_metadata() {
        let store = ActiveTurnStore::default();
        let external_prompt = PromptQueueItem::external_observed_running(
            "codex",
            "session-1",
            "user-1",
            "agent-1",
            "external prompt",
        );
        store.start(
            ActiveTurnState::new(
                "session-1".to_string(),
                "agent-1".to_string(),
                external_prompt.id().to_string(),
                "run-1".to_string(),
            )
            .with_prompt_metadata(&external_prompt),
        );

        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            external_prompt.id().to_string(),
            "run-1".to_string(),
        ));

        let turn = store
            .snapshot()
            .remove("run-1")
            .expect("turn should remain active");
        assert_eq!(turn.prompt_origin, Some(PromptOrigin::External));
        let external = turn
            .external_observed_id
            .expect("external metadata should survive restart");
        assert_eq!(external.provider, "codex");
        assert_eq!(external.provider_session_id, "session-1");
        assert_eq!(external.provider_turn_id, "user-1");
    }

    #[test]
    fn active_turn_restart_for_new_prompt_does_not_preserve_old_turn_state() {
        let store = ActiveTurnStore::default();
        let external_prompt = PromptQueueItem::external_observed_running(
            "codex",
            "session-1",
            "user-1",
            "agent-1",
            "external prompt",
        );
        store.start(
            ActiveTurnState::new(
                "session-1".to_string(),
                "agent-1".to_string(),
                external_prompt.id().to_string(),
                "run-1".to_string(),
            )
            .with_prompt_metadata(&external_prompt)
            .with_phase(ActiveTurnPhase::Streaming),
        );
        store.mark_settling("run-1");

        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "prompt-2".to_string(),
            "run-1".to_string(),
        ));

        let turn = store
            .snapshot()
            .remove("run-1")
            .expect("new turn should be active");
        assert_eq!(turn.prompt_id, "prompt-2");
        assert_eq!(turn.trace_id, "prompt-2");
        assert_eq!(turn.phase, ActiveTurnPhase::Accepted);
        assert!(!turn.settlement_requested);
        assert_eq!(turn.prompt_origin, None);
        assert_eq!(turn.external_observed_id, None);
    }

    #[test]
    fn active_turn_start_replaces_prior_turn_for_same_session_agent() {
        let store = ActiveTurnStore::default();
        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "prompt-1".to_string(),
            "run-1".to_string(),
        ));
        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-2".to_string(),
            "prompt-2".to_string(),
            "run-2".to_string(),
        ));
        store.start(ActiveTurnState::new(
            "session-2".to_string(),
            "agent-1".to_string(),
            "prompt-3".to_string(),
            "run-3".to_string(),
        ));

        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "prompt-4".to_string(),
            "run-4".to_string(),
        ));

        let snapshot = store.snapshot();
        assert!(!snapshot.contains_key("run-1"));
        assert!(snapshot.contains_key("run-2"));
        assert!(snapshot.contains_key("run-3"));
        assert_eq!(
            snapshot.get("run-4").map(|turn| turn.prompt_id.as_str()),
            Some("prompt-4")
        );
    }

    #[test]
    fn active_turn_phase_advances_without_regressing() {
        let store = ActiveTurnStore::default();
        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "prompt-1".to_string(),
            "run-1".to_string(),
        ));

        store.mark_awaiting_first_output("run-1");
        assert_eq!(
            store
                .snapshot()
                .get("run-1")
                .expect("turn should remain active")
                .phase,
            ActiveTurnPhase::AwaitingFirstOutput
        );

        store.mark_streaming("run-1");
        assert_eq!(
            store
                .snapshot()
                .get("run-1")
                .expect("turn should remain active")
                .phase,
            ActiveTurnPhase::Streaming
        );

        store.mark_settling("run-1");
        let settling = store
            .snapshot()
            .remove("run-1")
            .expect("turn should settle");
        assert_eq!(settling.phase, ActiveTurnPhase::Settling);
        assert!(settling.settlement_requested);

        store.mark_streaming("run-1");
        assert_eq!(
            store
                .snapshot()
                .get("run-1")
                .expect("turn should remain settling")
                .phase,
            ActiveTurnPhase::Settling
        );
    }

    #[test]
    fn active_turn_store_clears_by_session_and_agent() {
        let store = ActiveTurnStore::default();
        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "prompt-1".to_string(),
            "run-1".to_string(),
        ));
        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-2".to_string(),
            "prompt-2".to_string(),
            "run-2".to_string(),
        ));
        store.start(ActiveTurnState::new(
            "session-2".to_string(),
            "agent-1".to_string(),
            "prompt-3".to_string(),
            "run-3".to_string(),
        ));

        assert_eq!(store.clear_agent("session-1", "agent-1"), 1);
        assert!(!store.snapshot().contains_key("run-1"));
        assert!(store.snapshot().contains_key("run-2"));
        assert!(store.snapshot().contains_key("run-3"));

        assert_eq!(store.clear_session("session-1"), 1);
        assert_eq!(
            store.snapshot().keys().cloned().collect::<Vec<_>>(),
            vec!["run-3".to_string()]
        );
    }

    #[test]
    fn active_turn_store_get_reads_without_clearing() {
        let store = ActiveTurnStore::default();
        store.start(ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "prompt-1".to_string(),
            "run-1".to_string(),
        ));

        let turn = store.get("run-1").expect("active turn should be readable");

        assert_eq!(turn.prompt_id, "prompt-1");
        assert!(store.snapshot().contains_key("run-1"));
    }
}
