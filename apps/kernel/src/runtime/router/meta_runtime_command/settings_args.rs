//! `agent settings set`: an agent changes its own settings, or those of an agent below it.

use super::result::meta_command_error;
use super::*;
use crate::local::{UpdateAgentConfigRequest, UpdateAgentProfileRequest};

const USAGE: &str = "usage: agent settings set [<agent>] [--model M] [--effort E | --clear-effort] [--account-profile P] [--provider P] [--reason TEXT] [--at-turn-end]  |  agent settings set [<agent>] [--mode build|plan] [--permission required|yolo] [--dir PATH] [--reason TEXT]";

/// The request the arguments after `agent settings` ask for: the profile request, or the config
/// request when they name a mode, permission level or directory. One command changes one kind.
pub(super) fn meta_agent_settings_request(
    session: &crate::session::RuntimeSession,
    actor: &crate::agent::AgentInstance,
    args: &[String],
    agents: &[crate::agent::AgentInstance],
) -> Result<LocalDaemonRequest, DaemonError> {
    let [verb, rest @ ..] = args else {
        return Err(meta_command_error(USAGE));
    };
    if verb != "set" {
        return Err(meta_command_error(USAGE));
    }
    let mut reference = None;
    let (mut provider, mut account_profile, mut model, mut effort) = (None, None, None, None);
    let (mut clear_effort, mut mode, mut permission, mut directory, mut reason) =
        (false, None, None, None, None);
    let mut at_turn_end = false;
    let mut index = 0;
    while index < rest.len() {
        let arg = rest[index].as_str();
        if !arg.starts_with("--") {
            if reference.replace(arg).is_some() {
                return Err(meta_command_error(USAGE));
            }
            index += 1;
            continue;
        }
        if arg == "--clear-effort" {
            clear_effort = true;
            index += 1;
            continue;
        }
        if arg == "--at-turn-end" {
            at_turn_end = true;
            index += 1;
            continue;
        }
        let value = rest
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| meta_command_error(format!("{arg} needs a value. {USAGE}")))?;
        match arg {
            "--model" => model = Some(value.clone()),
            "--effort" => effort = Some(value.clone()),
            "--account-profile" => account_profile = Some(value.clone()),
            "--provider" => provider = Some(value.clone()),
            "--reason" => reason = Some(value.clone()),
            "--dir" => directory = Some(value.clone()),
            "--mode" => {
                mode = Some(
                    crate::provider::AgentExecutionMode::parse(value)
                        .ok_or_else(|| meta_command_error("--mode is build or plan"))?,
                )
            }
            "--permission" => {
                permission = Some(
                    crate::provider::AgentPermissionLevel::parse(value)
                        .ok_or_else(|| meta_command_error("--permission is required or yolo"))?,
                )
            }
            _ => return Err(meta_command_error(USAGE)),
        }
        index += 2;
    }
    let target = settings_target(actor, agents, reference)?;
    let profile = provider.is_some()
        || account_profile.is_some()
        || model.is_some()
        || effort.is_some()
        || clear_effort;
    let config = mode.is_some() || permission.is_some() || directory.is_some();
    if config && at_turn_end {
        return Err(meta_command_error(
            "--at-turn-end applies to provider profile settings",
        ));
    }
    if clear_effort && effort.is_some() {
        return Err(meta_command_error("choose --effort or --clear-effort"));
    }
    match (profile, config) {
        (true, false) => {
            let request = UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: target.id().to_string(),
                provider,
                account_profile,
                model,
                effort,
                clear_effort,
                reason,
            };
            Ok(if at_turn_end {
                LocalDaemonRequest::UpdateAgentProfileAtTurnEnd(request)
            } else {
                LocalDaemonRequest::UpdateAgentProfile(request)
            })
        }
        (false, true) => Ok(LocalDaemonRequest::UpdateAgentConfig(
            UpdateAgentConfigRequest {
                session_id: session.id().to_string(),
                agent_id: target.id().to_string(),
                execution_mode: mode,
                clear_execution_mode: false,
                permission_level: permission,
                clear_permission_level: false,
                max_context_tokens: None,
                clear_max_context_tokens: false,
                workspace_id: None,
                clear_workspace_id: false,
                worktree_id: directory,
                clear_worktree_id: false,
     usage_limit: None,
                reason,
            },
        )),
        (true, true) => Err(meta_command_error(
            "change the model, effort and account in one command and the mode, permission and directory in another",
        )),
        (false, false) => Err(meta_command_error(USAGE)),
    }
}

pub(super) fn settings_target<'a>(
    actor: &crate::agent::AgentInstance,
    agents: &'a [crate::agent::AgentInstance],
    reference: Option<&str>,
) -> Result<&'a crate::agent::AgentInstance, DaemonError> {
    let target = crate::runtime::room_tool_admission::resolve_agent(
        agents,
        actor.session_id(),
        reference.unwrap_or_else(|| actor.id()),
    )?;
    crate::runtime::room_tool_admission::self_or_below(actor, target, agents)?;
    Ok(target)
}
