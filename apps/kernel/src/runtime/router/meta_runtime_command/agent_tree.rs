//! Room commands for the agent tree: a parent reads and controls the agents below it.
use super::agent_peer::PeerTarget;
use super::request::meta_kernel_command;
use super::result::{meta_command_error, meta_command_failure_result};
use super::*;

/// What a spawner is told about the peer it created in another room.
pub(super) const PEER_LIFETIME: &str = "the new agent belongs to that room and its owner and stays until the owner deletes it; you cannot delete it, and `agent cancel <agent> --room <room>` stops its turn";

/// The value of `--room` in a command's arguments.
fn room_option(args: &[String]) -> Option<&str> {
    args.iter()
        .position(|arg| arg == "--room")
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

/// `agent children` lists at most this many descendants.
const CHILDREN_LISTED_LIMIT: usize = 64;

impl CommandRouter {
    /// The commands that read or control the tree below the caller; `None` for any other command.
    pub(super) async fn dispatch_meta_agent_tree_command(
        &self,
        provider_run: Option<&crate::provider::RuntimeProviderRun>,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        command: &str,
        tokens: &[String],
    ) -> Result<Option<RuntimeToolResult>, DaemonError> {
        if let Some(listed) = self.reach_list_result(session, caller, command, tokens) {
            return Ok(Some(listed));
        }
        if tokens.first().map(String::as_str) != Some("agent") {
            return Ok(None);
        }
        let args = tokens.get(2..).unwrap_or_default();
        Ok(match tokens.get(1).map(String::as_str) {
            Some("cancel") => Some(
                self.dispatch_meta_agent_cancel_command(
                    provider_run,
                    session,
                    caller,
                    command,
                    args,
                )
                .await?,
            ),
            Some(verb @ ("steer" | "message")) => Some(
                self.dispatch_meta_agent_message_command(
                    provider_run,
                    session,
                    caller,
                    command,
                    verb == "steer",
                    args,
                )
                .await,
            ),
            // A peer created in another room is that room's owner's to delete.
            Some("delete" | "destroy" | "remove")
                if args
                    .iter()
                    .any(|arg| matches!(arg.as_str(), "--room" | "--kernel" | "--machine")) =>
            {
                let own_room = match room_option(args) {
                    Some(room) => self
                        .runtime_state
                        .resolve_session_ref_id(room, None)
                        .await
                        .is_ok_and(|room| room == session.id()),
                    None => false,
                };
                let message = if own_room {
                    "an agent in your own room is deleted without --room: `agent delete <agent>`"
                } else {
                    "an agent in another room belongs to that room and its owner: only the owner deletes it; `agent cancel <agent> --room <room>` stops its turn"
                };
                Some(meta_command_failure_result(
                    command,
                    meta_command_error(message),
                ))
            }
            Some("children") => {
                Some(self.meta_agent_children_result(session, caller, command, args))
            }
            Some("answer" | "pass") => Some(
                self.dispatch_meta_child_decision_command(session, caller, command, tokens)
                    .await?,
            ),
            _ => None,
        })
    }

    /// Sets the child's deadline to `after` from now, never later than the
    /// parent's own. A child that cannot be bounded is not left running.
    pub(super) async fn arm_child_deadline(
        &self,
        provider_run: Option<&crate::provider::RuntimeProviderRun>,
        session: &crate::session::RuntimeSession,
        parent: &crate::agent::AgentInstance,
        child: &crate::agent::AgentInstance,
        after: std::time::Duration,
    ) -> Result<(), DaemonError> {
        let mut deadline_ms = crate::session::unix_epoch_ms()
            .saturating_add(u64::try_from(after.as_millis()).unwrap_or(u64::MAX));
        if let LocalDaemonResponse::AgentDeadline {
            deadline: Some(parent_deadline),
        } = self
            .runtime_state
            .get_agent_deadline(crate::local::GetAgentDeadlineRequest {
                session_id: session.id().to_string(),
                agent_id: parent.id().to_string(),
            })?
        {
            deadline_ms = deadline_ms.min(parent_deadline.deadline_ms);
        }
        // A retried spawn finds its child already bounded; a deadline only ever shortens.
        if let LocalDaemonResponse::AgentDeadline {
            deadline: Some(current),
        } = self
            .runtime_state
            .get_agent_deadline(crate::local::GetAgentDeadlineRequest {
                session_id: session.id().to_string(),
                agent_id: child.id().to_string(),
            })?
        {
            if current.deadline_ms <= deadline_ms {
                return Ok(());
            }
        }
        let request = LocalDaemonRequest::SetAgentDeadline(crate::local::SetAgentDeadlineRequest {
            session_id: session.id().to_string(),
            agent_id: child.id().to_string(),
            deadline_ms,
        });
        if let Err(error) =
            Box::pin(self.dispatch(meta_kernel_command(provider_run, parent, &request), request))
                .await
        {
            let destroy = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
                session_id: session.id().to_string(),
                agent_id: child.id().to_string(),
            });
            let _ = Box::pin(
                self.dispatch(meta_kernel_command(provider_run, parent, &destroy), destroy),
            )
            .await;
            return Err(error);
        }
        Ok(())
    }

    /// `agent cancel <agent>` stops the active turn of an agent below the caller and keeps the agent.
    pub(super) async fn dispatch_meta_agent_cancel_command(
        &self,
        provider_run: Option<&crate::provider::RuntimeProviderRun>,
        session: &crate::session::RuntimeSession,
        parent: &crate::agent::AgentInstance,
        command: &str,
        args: &[String],
    ) -> Result<RuntimeToolResult, DaemonError> {
        let usage = || {
            Ok(meta_command_failure_result(
                command,
                meta_command_error(
                    "usage: agent cancel <agent-ref> [--room <room> [--kernel <kernel>|--machine <machine>]]",
                ),
            ))
        };
        let Some((reference, options)) = args.split_first() else {
            return usage();
        };
        let Some(PeerTarget { room, kernel, .. }) = PeerTarget::parse(options, false) else {
            return usage();
        };
        if let (Some(kernel), Some(room)) = (kernel, room) {
            match self.runtime_state.resolve_peer_kernel(kernel).await {
                Ok(Some(kernel)) => {
                    return Ok(
                        match self
                            .cancel_peer_agent_on_kernel(parent, &kernel, room, reference)
                            .await
                        {
                            Ok(cancelled) => RuntimeToolResult {
                                ok: true,
                                payload: serde_json::json!({"command": command, "response": cancelled}),
                            },
                            Err(error) => meta_command_failure_result(command, error),
                        },
                    )
                }
                Ok(None) => {}
                Err(error) => return Ok(meta_command_failure_result(command, error)),
            }
        }
        let target = match room {
            Some(room) => {
                self.runtime_state
                    .resolve_message_target(session, parent, Some(room), reference)
            }
            None => {
                self.resolve_owned_regular_agent(session.id(), parent, reference)
                    .await
            }
        };
        let target = match target {
            Ok(target) => target,
            Err(error) => return Ok(meta_command_failure_result(command, error)),
        };
        // In another room the kernel cancels through that agent's own delivery seat, so the
        // caller never joins the room; the command still records the caller.
        let attachment_id = if target.session_id() == session.id() {
            self.ensure_metaagent_command_attachment(session.id(), parent)
                .await
        } else {
            self.runtime_state
                .ensure_agent_message_attachment(target.session_id(), &target)
        };
        let attachment_id = match attachment_id {
            Ok(id) => id,
            Err(error) => return Ok(meta_command_failure_result(command, error)),
        };
        let request =
            LocalDaemonRequest::CancelActivePrompt(crate::local::CancelActivePromptRequest {
                session_id: target.session_id().to_string(),
                attachment_id,
                target_agent_id: Some(target.id().to_string()),
                expected_prompt_id: None,
            });
        match Box::pin(self.dispatch(meta_kernel_command(provider_run, parent, &request), request))
            .await
        {
            Ok(response) => Ok(super::result::meta_command_success_result(
                command,
                &response,
                parent,
                self.runtime_state.room_agent_tools_enabled(),
            )),
            Err(error) => Ok(meta_command_failure_result(command, error)),
        }
    }

    /// `agent cancel <agent> --room <room> --kernel <kernel>` on another kernel of the owner.
    async fn cancel_peer_agent_on_kernel(
        &self,
        parent: &crate::agent::AgentInstance,
        kernel: &str,
        room: &str,
        reference: &str,
    ) -> Result<serde_json::Value, DaemonError> {
        let request_id = self
            .runtime_state
            .peer_cancel_request_id(parent, kernel, room, reference)?;
        let (target, duplicate) = self
            .runtime_state
            .relay_peer_agent(
                parent,
                kernel,
                room,
                reference,
                crate::runtime::state::peer_agent_outbound::PeerAgentCall::Cancel,
                request_id,
            )
            .await?;
        Ok(
            serde_json::json!({"cancelled": true, "duplicate": duplicate, "kernel": target.kernel_id, "room": target.room_id, "agent": target.agent_id}),
        )
    }

    /// `agent children [agent]` lists everything below the caller, or below one of its descendants.
    pub(super) fn meta_agent_children_result(
        &self,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        command: &str,
        args: &[String],
    ) -> RuntimeToolResult {
        let agents = self.runtime_state.session_agents(session.id());
        let root = match args {
            [] => caller.clone(),
            [reference] => {
                match crate::runtime::room_tool_admission::resolve_agent(
                    &agents,
                    caller.session_id(),
                    reference,
                )
                .cloned()
                {
                    Ok(root) if root.id() == caller.id() => root,
                    Ok(root) => {
                        if let Err(error) =
                            crate::runtime::room_tool_admission::in_subtree(caller, &root, &agents)
                        {
                            return meta_command_failure_result(command, error);
                        }
                        root
                    }
                    Err(error) => return meta_command_failure_result(command, error),
                }
            }
            _ => {
                return meta_command_failure_result(
                    command,
                    meta_command_error("usage: agent children [agent-ref]"),
                )
            }
        };
        let mut listed = Vec::new();
        let mut frontier = vec![(root.id().to_string(), 1_u32)];
        while let Some((parent_id, depth)) = frontier.pop() {
            for agent in agents
                .iter()
                .filter(|agent| agent.parent_agent_id() == Some(parent_id.as_str()))
            {
                if listed.len() == CHILDREN_LISTED_LIMIT {
                    break;
                }
                let deadline = match self.runtime_state.get_agent_deadline(
                    crate::local::GetAgentDeadlineRequest {
                        session_id: session.id().to_string(),
                        agent_id: agent.id().to_string(),
                    },
                ) {
                    Ok(LocalDaemonResponse::AgentDeadline { deadline }) => deadline,
                    _ => None,
                };
                listed.push(serde_json::json!({
                    "id": agent.id(),
                    "agent_ref": agent.agent_ref(),
                    "alias": agent.alias(),
                    "parent": parent_id,
                    "depth": depth,
                    "state": format!("{:?}", agent.state()),
                    "is_processing": agent.is_processing(),
                    "has_active_prompt": session.active_prompt_for_agent(agent.id()).is_some(),
                    "deadline": deadline,
                }));
                frontier.push((agent.id().to_string(), depth + 1));
            }
        }
        RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "command": command,
                "response": {"type": "AgentChildren", "root": root.id(), "children": listed},
            }),
        }
    }
}

impl CommandRouter {
    /// `agent answer <interaction> <choice|text>` and `agent pass <interaction>`: only the
    /// agent that holds a sub-agent's decision may settle or hand it on. The result carries a
    /// typed status and no text.
    async fn dispatch_meta_child_decision_command(
        &self,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        command: &str,
        tokens: &[String],
    ) -> Result<RuntimeToolResult, DaemonError> {
        use crate::runtime::state::ChildDecisionStatus;
        if tokens
            .iter()
            .skip(2)
            .any(|token| matches!(token.as_str(), "--room" | "--kernel" | "--machine"))
        {
            return Ok(meta_command_failure_result(
                command,
                meta_command_error(
                    "peer_decision_refused: a decision is answered or passed only by the agent that holds it in its own room; a peer never settles it, send the agent a message instead",
                ),
            ));
        }
        let status = match (
            tokens.get(1).map(String::as_str),
            tokens.get(2..).unwrap_or_default(),
        ) {
            (Some("answer"), [interaction, answer @ ..]) if !answer.is_empty() => {
                self.runtime_state
                    .answer_child_decision(
                        session.id(),
                        caller.id(),
                        interaction,
                        &answer.join(" "),
                    )
                    .await?
            }
            (Some("pass"), [interaction]) => {
                self.runtime_state
                    .pass_child_decision(session.id(), caller.id(), interaction)
                    .await?
            }
            _ => return Ok(meta_command_failure_result(
                command,
                meta_command_error(
                    "usage: agent answer <interaction> <choice|text> | agent pass <interaction>",
                ),
            )),
        };
        Ok(RuntimeToolResult {
            ok: matches!(
                status,
                ChildDecisionStatus::Answered | ChildDecisionStatus::Passed
            ),
            payload: serde_json::json!({
                "command": super::result::redacted_meta_command_for_payload(command),
                "response": {"type": "ChildDecision", "status": status},
            }),
        })
    }
}
