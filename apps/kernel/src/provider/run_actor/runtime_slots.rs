use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::error::DaemonError;
use crate::provider::opencode_runtime::OpenCodeRuntimeState;
use crate::provider::{ClaudeRuntimeState, CodexRuntimeState};

pub(super) type ClaudeRuntimeSlot = Arc<Mutex<Option<ClaudeRuntimeState>>>;
pub(super) type CodexRuntimeSlot = Arc<Mutex<Option<CodexRuntimeState>>>;
pub(super) type OpenCodeRuntimeSlot = Arc<Mutex<Option<OpenCodeRuntimeState>>>;

struct RetiringClaudeRuntime {
    process: crate::provider::claude_runtime::ClaudeRuntimeProcess,
    // Keep the old I/O slot alive until its exact child and descendants exit.
    slot: Option<ClaudeRuntimeSlot>,
}

#[derive(Clone, Default)]
pub(super) struct ProviderRunRuntimeRegistry {
    claude_runs: Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    claude_processes:
        Arc<Mutex<BTreeMap<String, crate::runtime::kernel_access::process::ProcessIdentitySlot>>>,
    claude_owned_processes:
        Arc<Mutex<BTreeMap<String, crate::provider::claude_runtime::ClaudeRuntimeProcess>>>,
    retiring_claude_runs: Arc<Mutex<BTreeMap<String, Vec<Arc<RetiringClaudeRuntime>>>>>,
    codex_runs: Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    opencode_runs: Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    cleared_runs: Arc<Mutex<BTreeSet<String>>>,
    assessment_outputs: Arc<Mutex<BTreeMap<String, crate::provider::ProviderPromptSignalBatch>>>,
}

impl ProviderRunRuntimeRegistry {
    pub(super) fn store_assessment_output(
        &self,
        run_id: &str,
        output: crate::provider::ProviderPromptSignalBatch,
    ) {
        let cleared = self
            .cleared_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !cleared.contains(run_id) {
            self.assessment_outputs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(run_id.into(), output);
        }
    }

    pub(super) fn take_assessment_output(
        &self,
        run_id: &str,
    ) -> Option<crate::provider::ProviderPromptSignalBatch> {
        self.assessment_outputs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run_id)
    }
    pub(super) fn insert_claude_runtime(&self, run_id: String, state: ClaudeRuntimeState) {
        self.retire_claude_runtime(&run_id);
        self.clear_tombstone(&run_id);
        self.claude_owned_processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run_id.clone(), state.owned_process());
        self.claude_processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run_id.clone(), state.process_identity_slot());
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

    pub(super) fn claude_process_identities(
        &self,
    ) -> Vec<(
        String,
        crate::runtime::kernel_access::process::ProcessIdentity,
    )> {
        self.claude_processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|(id, slot)| {
                slot.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
                    .map(|identity| (id.clone(), identity))
            })
            .collect()
    }

    pub(super) fn claude_owned_process(
        &self,
        run_id: &str,
    ) -> Option<crate::provider::claude_runtime::ClaudeRuntimeProcess> {
        self.claude_owned_processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(run_id)
            .cloned()
    }

    pub(super) fn claude_owned_processes(
        &self,
        run_id: &str,
    ) -> Vec<crate::provider::claude_runtime::ClaudeRuntimeProcess> {
        let mut processes: Vec<_> = self.claude_owned_process(run_id).into_iter().collect();
        if let Some(retiring) = self
            .retiring_claude_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(run_id)
        {
            processes.extend(retiring.iter().map(|record| record.process.clone()));
        }
        processes
    }

    fn retire_claude_runtime(&self, run_id: &str) {
        let process = self
            .claude_owned_processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run_id);
        self.claude_processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run_id);
        let slot = self
            .claude_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run_id);
        let Some(process) = process else {
            return;
        };
        // Only revoke replacement admission here. Physical I/O runs outside the
        // shared provider store lock and keeps this generation's ownership.
        process.begin_retirement();
        let record = Arc::new(RetiringClaudeRuntime { process, slot });
        self.retiring_claude_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(run_id.to_string())
            .or_default()
            .push(record.clone());
        let retiring = self.retiring_claude_runs.clone();
        let run_id = run_id.to_string();
        super::worker::provider_actor_runtime().spawn(async move {
            let mut reported = false;
            loop {
                let retired = record.clone();
                let result = tokio::task::spawn_blocking(move || {
                    retired.process.settle_prompt_abort(true)?;
                    // Reader teardown belongs outside both shared registries.
                    if let Some(slot) = &retired.slot {
                        let state = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                        drop(state);
                    }
                    Ok::<(), std::io::Error>(())
                }).await;
                match result {
                    Ok(Ok(_)) => break,
                    failure if !reported => {
                        crate::logging::warn_with_fields(
                            "daemon.provider", "Claude runtime retirement is pending",
                            serde_json::json!({"provider_run_id": run_id, "error": format!("{failure:?}")}),
                        );
                        reported = true;
                    }
                    _ => {}
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            let mut records = retiring.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(generations) = records.get_mut(&run_id) {
                generations.retain(|other| !Arc::ptr_eq(other, &record));
                if generations.is_empty() { records.remove(&run_id); }
            }
        });
    }

    pub(super) fn clear_runtime_state(&self, run_id: &str, stop_opencode: bool) {
        self.take_assessment_output(run_id);
        self.retire_claude_runtime(run_id);
        clear_runtime_state(&self.codex_runs, &self.opencode_runs, run_id, stop_opencode);
    }

    /// What a terminated Claude run still owes the history (see `take_closing_compaction_chunks`).
    pub(super) fn take_claude_closing_chunks(
        &self,
        run_id: &str,
    ) -> Vec<crate::provider::ProviderPromptChunk> {
        let Ok(slot) = claude_slot(&self.claude_runs, run_id) else {
            return Vec::new();
        };
        let mut guard = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_mut()
            .map(|state| {
                crate::provider::claude_runtime::take_closing_compaction_chunks(run_id, state)
            })
            .unwrap_or_default()
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

    /// Claude runs whose process ended while idle, each with whether it ended
    /// before it took any turn. A run a command holds now is not idle: that
    /// command sees the exit itself.
    pub(super) fn claude_runs_exited_while_idle(&self, only: Option<&str>) -> Vec<(String, bool)> {
        let slots = self
            .claude_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(run_id, _)| only.is_none_or(|only| only == run_id.as_str()))
            .map(|(run_id, slot)| (run_id.clone(), slot.clone()))
            .collect::<Vec<_>>();
        slots
            .into_iter()
            .filter_map(|(run_id, slot)| {
                let mut slot = slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let state = slot.as_mut()?;
                crate::provider::claude_runtime::claude_runtime_child_exited(state)
                    .then(|| (run_id, state.took_no_turn()))
            })
            .collect()
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

    /// Runs a command holds right now, so a hot reload cannot hand their turns
    /// over yet: a command takes the runtime out of its slot while it works and
    /// puts it back when done.
    pub(super) fn busy_runs(&self) -> Vec<String> {
        let mut runs = busy_slot_runs(&self.claude_runs);
        runs.extend(busy_slot_runs(&self.codex_runs));
        runs
    }

    fn clear_tombstone(&self, run_id: &str) {
        self.cleared_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run_id);
    }
}

/// A slot with no runtime in it, or locked by a command, is not at a safe
/// point. A poisoned slot with its runtime is.
#[allow(
    clippy::type_complexity,
    reason = "Keep the explicit runs state or return type at the existing boundary"
)]
fn busy_slot_runs<T>(runs: &Mutex<BTreeMap<String, Arc<Mutex<Option<T>>>>>) -> Vec<String> {
    let slots = runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|(run_id, slot)| (run_id.clone(), Arc::clone(slot)))
        .collect::<Vec<_>>();
    slots
        .into_iter()
        .filter(|(_, slot)| match slot.try_lock() {
            Ok(state) => state.is_none(),
            Err(std::sync::TryLockError::Poisoned(state)) => state.into_inner().is_none(),
            Err(std::sync::TryLockError::WouldBlock) => true,
        })
        .map(|(run_id, _)| run_id)
        .collect()
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

#[allow(
    clippy::type_complexity,
    reason = "Keep the explicit runtime_should_restore state or return type at the existing boundary"
)]
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
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    run_id: &str,
    stop_opencode: bool,
) {
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
    fn review971_assessment_output_is_one_shot_and_cannot_resurrect_cleared_run() {
        let registry = ProviderRunRuntimeRegistry::default();
        registry.store_assessment_output(
            "run",
            crate::provider::ProviderPromptSignalBatch {
                prompt_completed: true,
                ..Default::default()
            },
        );
        assert!(
            registry
                .take_assessment_output("run")
                .unwrap()
                .prompt_completed
        );
        assert!(registry.take_assessment_output("run").is_none());
        registry.store_assessment_output("run", Default::default());
        registry.clear_runtime("run", false);
        assert!(registry.take_assessment_output("run").is_none());
        registry.store_assessment_output("run", Default::default());
        assert!(registry.take_assessment_output("run").is_none());
    }

    #[test]
    fn a_run_whose_runtime_a_command_has_taken_or_holds_is_busy_and_a_resting_one_is_not() {
        let runs: Mutex<BTreeMap<String, Arc<Mutex<Option<i32>>>>> = Mutex::default();
        let resting = Arc::new(Mutex::new(Some(1)));
        let taken = Arc::new(Mutex::new(Some(2)));
        let locked = Arc::new(Mutex::new(Some(3)));
        for (id, slot) in [
            ("resting", &resting),
            ("taken", &taken),
            ("locked", &locked),
        ] {
            runs.lock()
                .unwrap()
                .insert(id.to_string(), Arc::clone(slot));
        }
        assert!(super::busy_slot_runs(&runs).is_empty());
        // A command takes the runtime out of its slot while it works.
        let state = taken.lock().unwrap().take();
        assert_eq!(super::busy_slot_runs(&runs), vec!["taken".to_string()]);
        let command = locked.lock().unwrap();
        let mut busy = super::busy_slot_runs(&runs);
        busy.sort();
        assert_eq!(busy, vec!["locked".to_string(), "taken".to_string()]);
        drop(command);
        *taken.lock().unwrap() = state;
        assert!(super::busy_slot_runs(&runs).is_empty());
    }

    #[test]
    fn a_poisoned_slot_with_its_runtime_is_not_busy() {
        let runs: Mutex<BTreeMap<String, Arc<Mutex<Option<i32>>>>> = Mutex::default();
        let slot = Arc::new(Mutex::new(Some(1)));
        runs.lock()
            .unwrap()
            .insert("run".to_string(), Arc::clone(&slot));
        let poisoning = Arc::clone(&slot);
        let _ = std::panic::catch_unwind(move || {
            let _guard = poisoning.lock().unwrap();
            panic!("poison");
        });
        assert!(super::busy_slot_runs(&runs).is_empty());
    }

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
        #[allow(
            clippy::type_complexity,
            reason = "Keep the explicit runs state or return type at the existing boundary"
        )]
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
        #[allow(
            clippy::type_complexity,
            reason = "Keep the explicit runs state or return type at the existing boundary"
        )]
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
        #[allow(
            clippy::type_complexity,
            reason = "Keep the explicit runs state or return type at the existing boundary"
        )]
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

#[cfg(all(test, unix))]
mod process_identity_tests {
    use super::*;
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };

    #[test]
    fn claude_process_identity_registry_recovers_poisoned_map_and_slot() {
        let registry = ProviderRunRuntimeRegistry::default();
        let identity = crate::runtime::kernel_access::process::inspect(std::process::id())
            .unwrap()
            .0;
        let slot = Arc::new(Mutex::new(Some(identity.clone())));
        registry
            .claude_processes
            .lock()
            .unwrap()
            .insert("fixture".into(), slot.clone());
        let map = registry.claude_processes.clone();
        assert!(std::thread::spawn(move || {
            let _guard = map.lock().unwrap();
            panic!("fixture map poison");
        })
        .join()
        .is_err());
        assert!(std::thread::spawn(move || {
            let _guard = slot.lock().unwrap();
            panic!("fixture slot poison");
        })
        .join()
        .is_err());
        assert_eq!(
            registry.claude_process_identities(),
            vec![("fixture".into(), identity)]
        );
        registry.clear_runtime_state("fixture", false);
        assert!(registry.claude_process_identities().is_empty());
    }

    #[test]
    fn finishing_old_claude_retirement_preserves_the_replacement_generation() {
        let run = RuntimeProviderRun::new(
            "retirement-generation",
            &LaunchProviderRequest::new("s", "claude", "claude", "default", "default"),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "fixture-pipe".into(),
                pty_target: None,
                pty_program: Some("/bin/sh".into()),
                pty_args: vec!["-c".into(), "while IFS= read -r line; do :; done".into()],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        let registry = ProviderRunRuntimeRegistry::default();
        let old = crate::provider::initialize_claude_runtime(&run).unwrap();
        let process = old.state.owned_process();
        struct Release(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Release {
            fn drop(&mut self) {
                self.0.store(false, std::sync::atomic::Ordering::Release);
            }
        }
        let release = Release(process.defer_cleanup_for_test());
        registry.insert_claude_runtime(run.id().into(), old.state);
        registry.clear_runtime(run.id(), false);
        let fresh = crate::provider::initialize_claude_runtime(&run).unwrap();
        let fresh_identity = fresh
            .state
            .process_identity_slot()
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        registry.insert_claude_runtime(run.id().into(), fresh.state);
        assert_eq!(registry.claude_owned_processes(run.id()).len(), 2);
        drop(release);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while registry.claude_owned_processes(run.id()).len() != 1 {
            assert!(
                std::time::Instant::now() < until,
                "old generation cleanup must finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            registry.claude_process_identities(),
            vec![(run.id().to_string(), fresh_identity)]
        );
        assert!(registry.state_bound(run.id()));
        registry.clear_runtime(run.id(), false);
    }

    #[test]
    fn claude_launch_identity_survives_actor_lease_and_clear_blocks_late_restore() {
        let run = RuntimeProviderRun::new(
            "piped-process-identity",
            &LaunchProviderRequest::new("s", "claude", "claude", "default", "default"),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "fixture-pipe".into(),
                pty_target: None,
                pty_program: Some("/bin/sh".into()),
                pty_args: vec!["-c".into(), "while IFS= read -r line; do :; done".into()],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        let binding = crate::provider::initialize_claude_runtime(&run).unwrap();
        let registry = ProviderRunRuntimeRegistry::default();
        registry.insert_claude_runtime(run.id().into(), binding.state);
        let before = registry.claude_process_identities();
        assert_eq!(before.len(), 1);
        let (slot, state) = registry.take_claude_runtime(run.id()).unwrap();
        assert_eq!(registry.claude_process_identities(), before);
        // Restart updates the shared identity even while the actor owns state.
        state
            .process_identity_slot()
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .start += 1;
        assert_ne!(registry.claude_process_identities(), before);
        registry.clear_runtime(run.id(), false);
        assert!(registry.claude_process_identities().is_empty());
        registry.restore_claude_runtime_if_live(run.id(), &slot, state);
        assert!(registry.claude_process_identities().is_empty());
    }
}
