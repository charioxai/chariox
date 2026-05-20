use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use crate::runtime::workspace_coordinator::{WorkspaceClaimGuard, WorkspaceOperationClaimSnapshot};

#[derive(Debug, Clone)]
pub(crate) struct ActivePromptState {
    pub(crate) last_output_at: Option<Instant>,
    pub(crate) saw_response_content: bool,
    pub(crate) completion_recorded: bool,
    pub(crate) settlement_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveTurnState {
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) prompt_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) trace_id: String,
    pub(crate) started_at_ms: u64,
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
            provider_run_id,
            trace_id,
            started_at_ms: crate::session::unix_epoch_ms(),
            settlement_requested: false,
        }
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
        let turn = {
            let mut guard = self.inner.lock().expect("active turn mutex poisoned");
            let turn = if let Some(existing) = guard.get(&turn.provider_run_id) {
                merge_active_turn_start(existing, turn)
            } else {
                turn
            };
            guard.insert(turn.provider_run_id.clone(), turn.clone());
            turn
        };
        crate::debug_trace::record_terminal_turn(
            &turn.session_id,
            "active_turn_start",
            serde_json::json!({
                "agent_id": &turn.agent_id,
                "prompt_id": &turn.prompt_id,
                "provider_run_id": &turn.provider_run_id,
                "trace_id": &turn.trace_id,
                "started_at_ms": turn.started_at_ms,
                "settlement_requested": turn.settlement_requested,
            }),
        );
    }

    pub(crate) fn mark_settling(&self, provider_run_id: &str) {
        if let Some(turn) = self
            .inner
            .lock()
            .expect("active turn mutex poisoned")
            .get_mut(provider_run_id)
        {
            turn.settlement_requested = true;
            crate::debug_trace::record_terminal_turn(
                &turn.session_id,
                "active_turn_mark_settling",
                serde_json::json!({
                    "agent_id": &turn.agent_id,
                    "prompt_id": &turn.prompt_id,
                    "provider_run_id": &turn.provider_run_id,
                    "trace_id": &turn.trace_id,
                    "started_at_ms": turn.started_at_ms,
                    "settlement_requested": true,
                }),
            );
        }
    }

    pub(crate) fn clear(&self, provider_run_id: &str) {
        let removed = self
            .inner
            .lock()
            .expect("active turn mutex poisoned")
            .remove(provider_run_id);
        if let Some(turn) = removed {
            crate::debug_trace::record_terminal_turn(
                &turn.session_id,
                "active_turn_clear",
                serde_json::json!({
                    "agent_id": turn.agent_id,
                    "prompt_id": turn.prompt_id,
                    "provider_run_id": turn.provider_run_id,
                    "trace_id": turn.trace_id,
                    "started_at_ms": turn.started_at_ms,
                    "settlement_requested": turn.settlement_requested,
                }),
            );
        }
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<String, ActiveTurnState> {
        self.inner
            .lock()
            .expect("active turn mutex poisoned")
            .clone()
    }
}

fn merge_active_turn_start(
    existing: &ActiveTurnState,
    mut incoming: ActiveTurnState,
) -> ActiveTurnState {
    if existing.prompt_id == incoming.prompt_id
        && existing.trace_id != existing.prompt_id
        && incoming.trace_id == incoming.prompt_id
    {
        incoming.trace_id = existing.trace_id.clone();
        incoming.started_at_ms = existing.started_at_ms;
    }
    incoming
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptActivityStore {
    inner: Arc<Mutex<BTreeMap<String, ActivePromptState>>>,
}

impl PromptActivityStore {
    pub(crate) fn read(&self) -> MutexGuard<'_, BTreeMap<String, ActivePromptState>> {
        self.inner.lock().expect("prompt activity mutex poisoned")
    }

    pub(crate) fn write(&self) -> MutexGuard<'_, BTreeMap<String, ActivePromptState>> {
        self.inner.lock().expect("prompt activity mutex poisoned")
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
            .expect("prompt workspace claim mutex poisoned")
            .contains_key(provider_run_id)
    }

    pub(crate) fn insert(&self, provider_run_id: String, claim: WorkspaceClaimGuard) {
        self.inner
            .lock()
            .expect("prompt workspace claim mutex poisoned")
            .insert(provider_run_id, claim);
    }

    pub(crate) fn remove(&self, provider_run_id: &str) -> bool {
        self.inner
            .lock()
            .expect("prompt workspace claim mutex poisoned")
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
            .expect("prompt workspace claim mutex poisoned");
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
    }
}
