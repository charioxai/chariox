//! Command approvals when the official Codex workspace sandbox cannot start.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;

#[derive(Clone, Default)]
pub(in crate::provider) struct UnavailableWorkspaceSandbox(Arc<AtomicBool>);

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
    pub(super) fn approval_body(&self, params: &Value, body: String) -> String {
        if !is_shell_command_approval(params) || self.0.swap(true, Ordering::Relaxed) {
            return body;
        }

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
