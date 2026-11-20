//! Command approvals when the official Codex workspace sandbox cannot start.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde_json::Value;

#[derive(Clone, Default)]
pub(in crate::provider) struct UnavailableWorkspaceSandbox(Arc<Mutex<ApprovalState>>);

#[derive(Default)]
struct ApprovalState {
    explained: bool,
    turn_id: String,
    commands: BTreeSet<(Option<String>, Option<String>, String)>,
}

impl UnavailableWorkspaceSandbox {
    /// None declines a repeat rather than carrying the first approval over to
    /// a command Codex proposes to run outside its workspace sandbox.
    pub(super) fn approval_body(
        &self,
        params: &Value,
        active_turn_id: Option<&str>,
        mut body: String,
    ) -> Option<String> {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Do not conflate stdin callbacks, subcommand reviews or network
        // permissions with a new approval for the same shell command.
        if params.get("approvalId").and_then(Value::as_str).is_some()
            || params
                .get("networkApprovalContext")
                .is_some_and(|value| !value.is_null())
            || params
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind != "command")
        {
            return Some(body);
        }
        let command = params.get("command").and_then(Value::as_str);
        let turn = params
            .get("turnId")
            .and_then(Value::as_str)
            .or(active_turn_id);
        if let (Some(command), Some(turn)) = (command, turn) {
            if state.turn_id != turn {
                state.commands.clear();
                state.turn_id = turn.to_string();
            }
            let field = |name| params.get(name).and_then(Value::as_str).map(str::to_string);
            if !state
                .commands
                .insert((field("environmentId"), field("cwd"), command.to_string()))
            {
                return None;
            }
        }
        if !state.explained {
            state.explained = true;
            body = format!(
                "Codex's workspace sandbox is unavailable. Required permission still applies. An approval does not allow a retry outside the sandbox. Restore the workspace sandbox or explicitly change the agent's permission to run commands without it.\n\n{body}"
            );
        }
        Some(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unavailable_sandbox_explains_once_and_declines_the_same_command_retry() {
        let sandbox = UnavailableWorkspaceSandbox::default();
        let first = json!({"turnId":"t","itemId":"a","command":"date","cwd":"/work"});
        let retry = json!({"turnId":"t","itemId":"b","command":"date","cwd":"/work"});
        let later = json!({"turnId":"later","itemId":"c","command":"date","cwd":"/work"});
        assert!(sandbox
            .approval_body(&first, None, "date".into())
            .unwrap()
            .contains("workspace sandbox is unavailable"));
        assert!(sandbox
            .approval_body(&retry, None, "date outside the sandbox".into())
            .is_none());
        assert_eq!(
            sandbox
                .approval_body(&later, None, "date".into())
                .as_deref(),
            Some("date")
        );
    }
}
