//! `agent resume`: an agent above takes back or re-arms the resume of an agent a usage limit stopped.

use super::result::meta_command_error;
use super::*;
use crate::local::{UpdateAgentConfigRequest, UsageLimitStopAction};

pub(super) const USAGE: &str = "usage: agent resume [cancel] <agent-ref> [--room <room>]";

pub(super) fn meta_agent_resume_request(
    session: &crate::session::RuntimeSession,
    actor: &crate::agent::AgentInstance,
    args: &[String],
    agents: &[crate::agent::AgentInstance],
) -> Result<LocalDaemonRequest, DaemonError> {
    let (action, reference) = match args {
        [verb, reference] if verb == "cancel" => (UsageLimitStopAction::Cancel, reference),
        [reference] => (UsageLimitStopAction::ResumeWhenBack, reference),
        _ => return Err(meta_command_error(USAGE)),
    };
    let target =
        crate::runtime::room_tool_admission::resolve_agent(agents, actor.session_id(), reference)?;
    // Admission judges it: an agent above it, or another of the owner's agents (row 405).
    Ok(usage_limit_request(session.id(), target.id(), action))
}

/// The one change `agent resume` makes: the stopped agent's usage-limit action.
pub(super) fn usage_limit_request(
    session_id: &str,
    agent_id: &str,
    action: UsageLimitStopAction,
) -> LocalDaemonRequest {
    LocalDaemonRequest::UpdateAgentConfig(UpdateAgentConfigRequest {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        execution_mode: None,
        clear_execution_mode: false,
        permission_level: None,
        clear_permission_level: false,
        max_context_tokens: None,
        clear_max_context_tokens: false,
        workspace_id: None,
        clear_workspace_id: false,
        worktree_id: None,
        clear_worktree_id: false,
        usage_limit: Some(action),
        reason: None,
    })
}
