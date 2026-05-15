use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::error::DaemonError;
use crate::provider::opencode_runtime::OpenCodeRuntimeState;
use crate::provider::{ClaudeRuntimeState, CodexRuntimeState};

pub(super) type ClaudeRuntimeSlot = Arc<Mutex<Option<ClaudeRuntimeState>>>;
pub(super) type CodexRuntimeSlot = Arc<Mutex<Option<CodexRuntimeState>>>;
pub(super) type OpenCodeRuntimeSlot = Arc<Mutex<Option<OpenCodeRuntimeState>>>;

fn claude_slot(
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    run_id: &str,
) -> Result<ClaudeRuntimeSlot, DaemonError> {
    claude_runs
        .lock()
        .expect("claude runtime map poisoned")
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
        .expect("codex runtime map poisoned")
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
        .expect("opencode runtime map poisoned")
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
        .expect("claude runtime slot poisoned")
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
        .expect("codex runtime slot poisoned")
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
        .expect("opencode runtime slot poisoned")
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
        Ok(slot) => slot.lock().expect("claude runtime slot poisoned").is_none(),
        Err(_) => true,
    }
}

pub(super) fn runtime_slot_missing_or_empty_codex(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    run_id: &str,
) -> bool {
    match codex_slot(codex_runs, run_id) {
        Ok(slot) => slot.lock().expect("codex runtime slot poisoned").is_none(),
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
            .expect("opencode runtime slot poisoned")
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
        *slot.lock().expect("claude runtime slot poisoned") = Some(state);
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
        *slot.lock().expect("codex runtime slot poisoned") = Some(state);
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
        *slot.lock().expect("opencode runtime slot poisoned") = Some(state);
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
        .expect("cleared provider run set poisoned")
        .contains(run_id)
    {
        return false;
    }
    runs.lock()
        .expect("runtime map poisoned")
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
        .expect("claude runtime map poisoned")
        .remove(run_id)
    {
        let _ = slot.lock().expect("claude runtime slot poisoned").take();
    }
    if let Some(slot) = codex_runs
        .lock()
        .expect("codex runtime map poisoned")
        .remove(run_id)
    {
        let _ = slot.lock().expect("codex runtime slot poisoned").take();
    }
    if let Some(slot) = opencode_runs
        .lock()
        .expect("opencode runtime map poisoned")
        .remove(run_id)
    {
        let state = slot.lock().expect("opencode runtime slot poisoned").take();
        if let Some(state) = state {
            if stop_opencode {
                state.stop();
            }
        }
    }
}
