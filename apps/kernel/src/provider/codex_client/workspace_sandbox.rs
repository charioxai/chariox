//! Command approvals when the official Codex workspace sandbox cannot start.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Default)]
pub(in crate::provider) struct UnavailableWorkspaceSandbox(Arc<Mutex<WorkspaceApprovalState>>);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(in crate::provider) struct WorkspaceApprovalState {
    explained: bool,
    pending: Option<PendingWorkspaceApproval>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PendingWorkspaceApproval {
    pub(super) request_id: Value,
    pub(super) interaction: crate::session::RuntimeInteraction,
    pub(super) handover: Option<crate::provider::CodexApprovalHandover>,
}

#[cfg(unix)]
impl WorkspaceApprovalState {
    pub(in crate::provider) fn pending_goal_approval(
        &self,
    ) -> Option<&crate::provider::NativeApprovalGoal> {
        self.pending.as_ref()?.handover.as_ref()?.goal.as_ref()
    }
}

pub(super) fn is_shell_command_approval(params: &Value) -> bool {
    // stdin callbacks, subcommand reviews and network permissions are distinct.
    params.get("approvalId").and_then(Value::as_str).is_none()
        && params
            .get("networkApprovalContext")
            .is_none_or(Value::is_null)
        && params
            .get("kind")
            .and_then(Value::as_str)
            .is_none_or(|kind| kind == "command")
}

impl UnavailableWorkspaceSandbox {
    pub(super) fn approval_is_waiting_for_successor(&self) -> bool {
        self.lock().pending.is_some()
    }

    pub(super) fn store_for_successor(&self, approval: PendingWorkspaceApproval) {
        self.lock().pending = Some(approval);
    }

    pub(super) fn take_adopted_approval(&self) -> Option<PendingWorkspaceApproval> {
        self.lock().pending.take()
    }

    #[cfg(unix)]
    pub(in crate::provider) fn hand_over(&self) -> WorkspaceApprovalState {
        std::mem::take(&mut *self.lock())
    }

    #[cfg(unix)]
    pub(in crate::provider) fn adopt(state: WorkspaceApprovalState) -> Self {
        Self(Arc::new(Mutex::new(state)))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, WorkspaceApprovalState> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn approval_body(&self, params: &Value, body: String) -> String {
        if !is_shell_command_approval(params) {
            return body;
        }
        let mut state = self.lock();
        if state.explained {
            return body;
        }

        state.explained = true;
        format!(
            "Codex's workspace sandbox is unavailable. Required permission still applies. Each new command request requires its own approval. Restore the workspace sandbox before relying on its isolation.\n\n{body}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unavailable_sandbox_explains_once_and_leaves_the_same_command_retry_eligible() {
        let sandbox = UnavailableWorkspaceSandbox::default();
        let first = json!({"turnId":"t","itemId":"a","command":"date","cwd":"/work"});
        let retry = json!({"turnId":"t","itemId":"b","command":"date","cwd":"/work"});
        assert!(sandbox
            .approval_body(&first, "date".into())
            .contains("workspace sandbox is unavailable"));
        assert_eq!(sandbox.approval_body(&retry, "date".into()), "date");
    }
}
