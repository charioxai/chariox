//! Claude `-p` runs ask for tool approval through the runtime MCP
//! `chariox.permission_prompt` tool. Claude emits no stream events while the
//! user decides, so the turn stall watchdog must treat that wait as activity.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

fn pending_waits() -> MutexGuard<'static, BTreeMap<String, usize>> {
    static PENDING: OnceLock<Mutex<BTreeMap<String, usize>>> = OnceLock::new();
    PENDING
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct ClaudePermissionPromptWait {
    provider_run_id: String,
}

pub(crate) fn begin_claude_permission_prompt_wait(
    provider_run_id: &str,
) -> ClaudePermissionPromptWait {
    *pending_waits()
        .entry(provider_run_id.to_string())
        .or_default() += 1;
    ClaudePermissionPromptWait {
        provider_run_id: provider_run_id.to_string(),
    }
}

pub(super) fn claude_permission_prompt_pending(provider_run_id: &str) -> bool {
    pending_waits().contains_key(provider_run_id)
}

impl Drop for ClaudePermissionPromptWait {
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
        let run_id = "provider-run-permission-prompt-wait-test";
        assert!(!claude_permission_prompt_pending(run_id));
        let first = begin_claude_permission_prompt_wait(run_id);
        let second = begin_claude_permission_prompt_wait(run_id);
        drop(first);
        assert!(claude_permission_prompt_pending(run_id));
        drop(second);
        assert!(!claude_permission_prompt_pending(run_id));
    }
}
