use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::error::DaemonError;
use crate::provider::opencode_runtime::OpenCodeRuntimeState;
use crate::provider::{ClaudeRuntimeState, CodexRuntimeState};

pub(super) type ClaudeRuntimeSlot = Arc<Mutex<Option<ClaudeRuntimeState>>>;
pub(super) type CodexRuntimeSlot = Arc<Mutex<Option<CodexRuntimeState>>>;
pub(super) type OpenCodeRuntimeSlot = Arc<Mutex<Option<OpenCodeRuntimeState>>>;

#[derive(Clone, Default)]
pub(super) struct ProviderRunRuntimeRegistry {
    claude_runs: Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    codex_runs: Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    opencode_runs: Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    cleared_runs: Arc<Mutex<BTreeSet<String>>>,
}

impl ProviderRunRuntimeRegistry {
    pub(super) fn insert_claude_runtime(&self, run_id: String, state: ClaudeRuntimeState) {
        self.clear_tombstone(&run_id);
        self.claude_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run_id, Arc::new(Mutex::new(Some(state))));
    }

    pub(super) fn insert_codex_runtime(&self, run_id: String, state: CodexRuntimeState) {
        self.clear_tombstone(&run_id);
        self.codex_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run_id, Arc::new(Mutex::new(Some(state))));
    }

    pub(super) fn insert_opencode_runtime(&self, run_id: String, state: OpenCodeRuntimeState) {
        self.clear_tombstone(&run_id);
        self.opencode_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run_id, Arc::new(Mutex::new(Some(state))));
    }

    pub(super) fn state_bound(&self, run_id: &str) -> bool {
        if self
            .claude_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(run_id)
            .is_some_and(|slot| {
                slot.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_some()
            })
        {
            return true;
        }
        if self
            .codex_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(run_id)
            .is_some_and(|slot| {
                slot.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_some()
            })
        {
            return true;
        }
        self.opencode_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(run_id)
            .is_some_and(|slot| {
                slot.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_some()
            })
    }

    pub(super) fn clear_runtime(&self, run_id: &str, stop_opencode: bool) {
        self.cleared_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run_id.to_string());
        self.clear_runtime_state(run_id, stop_opencode);
    }

    pub(super) fn clear_runtime_state(&self, run_id: &str, stop_opencode: bool) {
        clear_runtime_state(
            &self.claude_runs,
            &self.codex_runs,
            &self.opencode_runs,
            run_id,
            stop_opencode,
        );
    }

    pub(super) fn take_claude_runtime(
        &self,
        run_id: &str,
    ) -> Result<(ClaudeRuntimeSlot, ClaudeRuntimeState), DaemonError> {
        take_claude_runtime(&self.claude_runs, run_id)
    }

    pub(super) fn take_codex_runtime(
        &self,
        run_id: &str,
    ) -> Result<(CodexRuntimeSlot, CodexRuntimeState), DaemonError> {
        take_codex_runtime(&self.codex_runs, run_id)
    }

    pub(super) fn take_opencode_runtime(
        &self,
        run_id: &str,
    ) -> Result<(OpenCodeRuntimeSlot, OpenCodeRuntimeState), DaemonError> {
        take_opencode_runtime(&self.opencode_runs, run_id)
    }

    pub(super) fn opencode_slot(&self, run_id: &str) -> Result<OpenCodeRuntimeSlot, DaemonError> {
        opencode_slot(&self.opencode_runs, run_id)
    }

    pub(super) fn runtime_slot_missing_or_empty_claude(&self, run_id: &str) -> bool {
        runtime_slot_missing_or_empty_claude(&self.claude_runs, run_id)
    }

    pub(super) fn runtime_slot_missing_or_empty_codex(&self, run_id: &str) -> bool {
        runtime_slot_missing_or_empty_codex(&self.codex_runs, run_id)
    }

    pub(super) fn runtime_slot_missing_or_empty_opencode(&self, run_id: &str) -> bool {
        runtime_slot_missing_or_empty_opencode(&self.opencode_runs, run_id)
    }

    pub(super) fn restore_claude_runtime_if_live(
        &self,
        run_id: &str,
        slot: &ClaudeRuntimeSlot,
        state: ClaudeRuntimeState,
    ) {
        restore_claude_runtime_if_live(&self.claude_runs, &self.cleared_runs, run_id, slot, state);
    }

    pub(super) fn restore_codex_runtime_if_live(
        &self,
        run_id: &str,
        slot: &CodexRuntimeSlot,
        state: CodexRuntimeState,
    ) {
        restore_codex_runtime_if_live(&self.codex_runs, &self.cleared_runs, run_id, slot, state);
    }

    pub(super) fn restore_opencode_runtime_if_live(
        &self,
        run_id: &str,
        slot: &OpenCodeRuntimeSlot,
        state: OpenCodeRuntimeState,
    ) {
        restore_opencode_runtime_if_live(
            &self.opencode_runs,
            &self.cleared_runs,
            run_id,
            slot,
            state,
        );
    }

    fn clear_tombstone(&self, run_id: &str) {
        self.cleared_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run_id);
    }
}

fn claude_slot(
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    run_id: &str,
) -> Result<ClaudeRuntimeSlot, DaemonError> {
    claude_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(run_id)
        .cloned()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run_id.to_string(),
            operation: "claude_session_missing",
            message: "no Claude Code session is bound to this provider run".to_string(),
        })
}

fn codex_slot(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    run_id: &str,
) -> Result<CodexRuntimeSlot, DaemonError> {
    codex_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(run_id)
        .cloned()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run_id.to_string(),
            operation: "codex_thread_missing",
            message: "no Codex thread is bound to this provider run".to_string(),
        })
}

pub(super) fn opencode_slot(
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    run_id: &str,
) -> Result<OpenCodeRuntimeSlot, DaemonError> {
    opencode_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(run_id)
        .cloned()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run_id.to_string(),
            operation: "opencode_session_missing",
            message: "no OpenCode session is bound to this provider run".to_string(),
        })
}

pub(super) fn take_claude_runtime(
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    run_id: &str,
) -> Result<(ClaudeRuntimeSlot, ClaudeRuntimeState), DaemonError> {
    let slot = claude_slot(claude_runs, run_id)?;
    let state = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run_id.to_string(),
            operation: "claude_session_missing",
            message: "no Claude Code session is bound to this provider run".to_string(),
        })?;
    Ok((slot, state))
}

pub(super) fn take_codex_runtime(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    run_id: &str,
) -> Result<(CodexRuntimeSlot, CodexRuntimeState), DaemonError> {
    let slot = codex_slot(codex_runs, run_id)?;
    let state = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run_id.to_string(),
            operation: "codex_thread_missing",
            message: "no Codex thread is bound to this provider run".to_string(),
        })?;
    Ok((slot, state))
}

pub(super) fn take_opencode_runtime(
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    run_id: &str,
) -> Result<(OpenCodeRuntimeSlot, OpenCodeRuntimeState), DaemonError> {
    let slot = opencode_slot(opencode_runs, run_id)?;
    let state = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run_id.to_string(),
            operation: "opencode_session_missing",
            message: "no OpenCode session is bound to this provider run".to_string(),
        })?;
    Ok((slot, state))
}

pub(super) fn runtime_slot_missing_or_empty_claude(
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    run_id: &str,
) -> bool {
    match claude_slot(claude_runs, run_id) {
        Ok(slot) => slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none(),
        Err(_) => true,
    }
}

pub(super) fn runtime_slot_missing_or_empty_codex(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    run_id: &str,
) -> bool {
    match codex_slot(codex_runs, run_id) {
        Ok(slot) => slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none(),
        Err(_) => true,
    }
}

pub(super) fn runtime_slot_missing_or_empty_opencode(
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    run_id: &str,
) -> bool {
    match opencode_slot(opencode_runs, run_id) {
        Ok(slot) => slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none(),
        Err(_) => true,
    }
}

pub(super) fn restore_claude_runtime_if_live(
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    cleared_runs: &Arc<Mutex<BTreeSet<String>>>,
    run_id: &str,
    slot: &ClaudeRuntimeSlot,
    state: ClaudeRuntimeState,
) {
    if runtime_should_restore(cleared_runs, claude_runs, run_id, slot) {
        *slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(state);
    }
}

pub(super) fn restore_codex_runtime_if_live(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    cleared_runs: &Arc<Mutex<BTreeSet<String>>>,
    run_id: &str,
    slot: &CodexRuntimeSlot,
    state: CodexRuntimeState,
) {
    if runtime_should_restore(cleared_runs, codex_runs, run_id, slot) {
        *slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(state);
    }
}

pub(super) fn restore_opencode_runtime_if_live(
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    cleared_runs: &Arc<Mutex<BTreeSet<String>>>,
    run_id: &str,
    slot: &OpenCodeRuntimeSlot,
    state: OpenCodeRuntimeState,
) {
    if runtime_should_restore(cleared_runs, opencode_runs, run_id, slot) {
        *slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(state);
    } else {
        state.stop();
    }
}

pub(super) fn runtime_should_restore<T>(
    cleared_runs: &Arc<Mutex<BTreeSet<String>>>,
    runs: &Arc<Mutex<BTreeMap<String, Arc<Mutex<Option<T>>>>>>,
    run_id: &str,
    slot: &Arc<Mutex<Option<T>>>,
) -> bool {
    if cleared_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(run_id)
    {
        return false;
    }
    runs.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(run_id)
        .is_some_and(|current_slot| Arc::ptr_eq(current_slot, slot))
}

pub(super) fn clear_runtime_state(
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    run_id: &str,
    stop_opencode: bool,
) {
    if let Some(slot) = claude_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(run_id)
    {
        let _ = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
    if let Some(slot) = codex_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(run_id)
    {
        let _ = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
    if let Some(slot) = opencode_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(run_id)
    {
        let state = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(state) = state {
            if stop_opencode {
                state.stop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Mutex};

    use super::{runtime_should_restore, ProviderRunRuntimeRegistry};

    #[test]
    fn state_bound_recovers_from_a_poisoned_map_lock() {
        // A panic while holding a provider-run map lock must not cascade into
        // daemon-wide panics on every later registry access; the registry
        // recovers the poisoned guard instead of re-panicking.
        let registry = ProviderRunRuntimeRegistry::default();
        let claude_runs = Arc::clone(&registry.claude_runs);
        let poisoning = std::panic::catch_unwind(move || {
            let _guard = claude_runs
                .lock()
                .expect("fresh registry lock should not be poisoned");
            panic!("poison the claude runtime map lock");
        });
        assert!(poisoning.is_err());
        assert!(registry.claude_runs.is_poisoned());
        // Reads through the same registry must still succeed after poisoning.
        assert!(!registry.state_bound("missing-run"));
    }

    #[test]
    fn runtime_tombstone_rejects_stale_state_restore_after_cleanup() {
        let cleared_runs = Arc::new(Mutex::new(BTreeSet::new()));
        let runs: Arc<Mutex<BTreeMap<String, Arc<Mutex<Option<i32>>>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let slot = Arc::new(Mutex::new(None));
        runs.lock()
            .expect("runtime map should not be poisoned")
            .insert("run-1".to_string(), Arc::clone(&slot));

        assert!(runtime_should_restore(&cleared_runs, &runs, "run-1", &slot));

        cleared_runs
            .lock()
            .expect("cleared set should not be poisoned")
            .insert("run-1".to_string());
        runs.lock()
            .expect("runtime map should not be poisoned")
            .remove("run-1");

        assert!(!runtime_should_restore(
            &cleared_runs,
            &runs,
            "run-1",
            &slot
        ));
    }

    #[test]
    fn runtime_restore_drops_taken_state_after_cleanup_tombstone() {
        let cleared_runs = Arc::new(Mutex::new(BTreeSet::new()));
        let runs: Arc<Mutex<BTreeMap<String, Arc<Mutex<Option<i32>>>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let slot = Arc::new(Mutex::new(Some(7)));
        runs.lock()
            .expect("runtime map should not be poisoned")
            .insert("run-1".to_string(), Arc::clone(&slot));

        let taken_state = slot
            .lock()
            .expect("runtime slot should not be poisoned")
            .take()
            .expect("runtime state should be present");

        cleared_runs
            .lock()
            .expect("cleared set should not be poisoned")
            .insert("run-1".to_string());
        runs.lock()
            .expect("runtime map should not be poisoned")
            .remove("run-1");

        if runtime_should_restore(&cleared_runs, &runs, "run-1", &slot) {
            *slot.lock().expect("runtime slot should not be poisoned") = Some(taken_state);
        }

        assert!(!runs
            .lock()
            .expect("runtime map should not be poisoned")
            .contains_key("run-1"));
        assert!(slot
            .lock()
            .expect("runtime slot should not be poisoned")
            .is_none());
    }

    #[test]
    fn runtime_restore_rejects_old_slot_after_same_run_replacement() {
        let cleared_runs = Arc::new(Mutex::new(BTreeSet::new()));
        let runs: Arc<Mutex<BTreeMap<String, Arc<Mutex<Option<i32>>>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let old_slot = Arc::new(Mutex::new(Some(7)));
        runs.lock()
            .expect("runtime map should not be poisoned")
            .insert("run-1".to_string(), Arc::clone(&old_slot));

        let taken_state = old_slot
            .lock()
            .expect("old runtime slot should not be poisoned")
            .take()
            .expect("old runtime state should be present");
        let replacement_slot = Arc::new(Mutex::new(Some(42)));
        runs.lock()
            .expect("runtime map should not be poisoned")
            .insert("run-1".to_string(), Arc::clone(&replacement_slot));

        if runtime_should_restore(&cleared_runs, &runs, "run-1", &old_slot) {
            *old_slot
                .lock()
                .expect("old runtime slot should not be poisoned") = Some(taken_state);
        }

        assert!(old_slot
            .lock()
            .expect("old runtime slot should not be poisoned")
            .is_none());
        assert_eq!(
            *replacement_slot
                .lock()
                .expect("replacement runtime slot should not be poisoned"),
            Some(42)
        );
    }
}
