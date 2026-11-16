//! `workflow responsible <workflow> [target] --room <room> [--kernel <kernel> | --machine <machine>]`:
//! the flags place the workflow, as they place the agent of `agent cancel`.
use super::agent_peer::PeerTarget;
use super::request::{responsible_target, RESPONSIBLE_USAGE};
use super::result::{meta_command_error, meta_command_failure_result};
use super::summary::summarize_meta_workflow;
use super::*;

impl CommandRouter {
    /// A workflow of another Room or kernel; `None` for a command that names no placement.
    pub(super) async fn dispatch_workflow_responsible_placed(
        &self,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        command: &str,
        tokens: &[String],
    ) -> Option<RuntimeToolResult> {
        if tokens.first().map(String::as_str) != Some("workflow")
            || tokens.get(1).map(String::as_str) != Some("responsible")
        {
            return None;
        }
        let args = tokens.get(2..).unwrap_or_default();
        let placed = args.iter().position(|arg| arg.starts_with("--"))?;
        let (positional, options) = args.split_at(placed);
        let result = async {
            let Some(PeerTarget {
                room: Some(room),
                kernel,
                ..
            }) = PeerTarget::parse(options, false)
            else {
                return Err(meta_command_error(RESPONSIBLE_USAGE));
            };
            let [workflow_ref, target @ ..] = positional else {
                return Err(meta_command_error(RESPONSIBLE_USAGE));
            };
            let agents = self.runtime_state.session_agents(session.id());
            let responsible = responsible_target(session, caller, target, &agents)?;
            let kernel = match kernel {
                Some(kernel) => self.runtime_state.resolve_peer_kernel(kernel).await?,
                None => None,
            };
            self.runtime_state
                .workflow_responsible_in(caller, room, kernel.as_deref(), workflow_ref, responsible)
                .await
        }
        .await;
        Some(match result {
            Ok(workflow) => RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "command": command,
                    "response": {"type": "Workflow", "workflow": summarize_meta_workflow(&workflow)},
                }),
            },
            Err(error) => meta_command_failure_result(command, error),
        })
    }
}
