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
    pub(crate) settlement_requested: bool,
}

impl ActiveTurnState {
    pub(crate) fn new(
        session_id: String,
        agent_id: String,
        prompt_id: String,
        provider_run_id: String,
    ) -> Self {
        Self {
            session_id,
            agent_id,
            prompt_id,
            provider_run_id,
            settlement_requested: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ActiveTurnStore {
    inner: Arc<Mutex<BTreeMap<String, ActiveTurnState>>>,
}

impl ActiveTurnStore {
    pub(crate) fn start(&self, turn: ActiveTurnState) {
        crate::debug_trace::record_terminal_turn(
            &turn.session_id,
            "active_turn_start",
            serde_json::json!({
                "agent_id": &turn.agent_id,
                "prompt_id": &turn.prompt_id,
                "provider_run_id": &turn.provider_run_id,
                "settlement_requested": turn.settlement_requested,
            }),
        );
        self.inner
            .lock()
            .expect("active turn mutex poisoned")
            .insert(turn.provider_run_id.clone(), turn);
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
