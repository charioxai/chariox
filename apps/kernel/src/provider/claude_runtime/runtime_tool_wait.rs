//! A Claude `-p` run emits no stream events while it waits on a Chariox
//! runtime MCP tool call: the permission prompt or a popup waiting for a
//! person, an App binding approval, a long App call. The turn stall watchdog
//! treats that wait as activity; the call itself is bounded by the kernel's
//! own deadlines and by Claude's per-server MCP `timeout`.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

fn pending_waits() -> MutexGuard<'static, BTreeMap<String, usize>> {
    static PENDING: OnceLock<Mutex<BTreeMap<String, usize>>> = OnceLock::new();
    PENDING
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct ClaudeRuntimeToolWait {
    provider_run_id: String,
}

pub(crate) fn begin_claude_runtime_tool_wait(provider_run_id: &str) -> ClaudeRuntimeToolWait {
    *pending_waits()
        .entry(provider_run_id.to_string())
        .or_default() += 1;
    ClaudeRuntimeToolWait {
        provider_run_id: provider_run_id.to_string(),
    }
}

pub(crate) fn claude_runtime_tool_wait_pending(provider_run_id: &str) -> bool {
    pending_waits().contains_key(provider_run_id)
}

impl Drop for ClaudeRuntimeToolWait {
    fn drop(&mut self) {
        let mut pending = pending_waits();
        if let Some(count) = pending.get_mut(&self.provider_run_id) {
            *count -= 1;
            if *count == 0 {
                pending.remove(&self.provider_run_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_is_pending_until_every_guard_drops() {
        let run_id = "provider-run-runtime-tool-wait-test";
        assert!(!claude_runtime_tool_wait_pending(run_id));
        let first = begin_claude_runtime_tool_wait(run_id);
        let second = begin_claude_runtime_tool_wait(run_id);
        drop(first);
        assert!(claude_runtime_tool_wait_pending(run_id));
        drop(second);
        assert!(!claude_runtime_tool_wait_pending(run_id));
    }
}
