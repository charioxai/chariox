//! MP-08 / MP-11: shared room admission and the subtree control policy.
use crate::{agent::AgentInstance, error::DaemonError};

/// Code a client sees for a refusal; the policy cannot change on a retry.
pub(crate) const REFUSED_CODE: &str = "room_tool_refused";

/// Code a client sees when the same call can succeed later; it may repeat it.
pub(crate) const RETRY_CODE: &str = "room_tool_retry";

/// Codes whose message the room tool wrote itself, as the sentence the user reads.
pub(crate) fn is_public_code(code: &str) -> bool {
    code == REFUSED_CODE || code == RETRY_CODE
}

/// The one way a room policy refuses: the message is the sentence the user reads.
pub(crate) fn denied(message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: REFUSED_CODE.into(),
        message: message.into(),
    }
}

/// A call that failed for a reason that passes: the sentence says when to repeat it.
pub(crate) fn retry_later(message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: RETRY_CODE.into(),
        message: message.into(),
    }
}

pub(crate) fn resolve_agent<'a>(
    agents: &'a [AgentInstance],
    room: &str,
    reference: &str,
) -> Result<&'a AgentInstance, DaemonError> {
    let mut matches = agents.iter().filter(|a| {
        a.session_id() == room
            && (a.id() == reference || a.agent_ref() == reference || a.alias() == Some(reference))
    });
    let target = matches
        .next()
        .ok_or_else(|| denied("agent is unavailable in this room"))?;
    if matches.next().is_some() {
        return Err(denied("ambiguous room agent reference"));
    }
    Ok(target)
}

/// Bounds the walk up the parent edges so a corrupt cycle cannot loop.
const TREE_WALK_LIMIT: usize = 16;

/// A caller controls every agent below it, at any depth, and nothing else.
/// Membership only: the permission level or mode of either agent never decides.
pub(crate) fn in_subtree(
    actor: &AgentInstance,
    target: &AgentInstance,
    agents: &[AgentInstance],
) -> Result<(), DaemonError> {
    if target.session_id() == actor.session_id() && target.id() != actor.id() {
        let mut current = target;
        for _ in 0..TREE_WALK_LIMIT {
            let Some(parent_id) = current.parent_agent_id() else {
                break;
            };
            if parent_id == actor.id() {
                return Ok(());
            }
            let Some(parent) = agents.iter().find(|agent| agent.id() == parent_id) else {
                break;
            };
            current = parent;
        }
    }
    Err(denied(
        "only the agents above an agent in this room can control it",
    ))
}

/// Destructive identity changes belong only to the target's immediate parent.
pub(crate) fn direct_child(
    actor: &AgentInstance,
    target: &AgentInstance,
) -> Result<(), DaemonError> {
    if target.session_id() == actor.session_id() && target.parent_agent_id() == Some(actor.id()) {
        Ok(())
    } else {
        Err(denied(
            "only a direct child in this room can be renamed or deleted",
        ))
    }
}

/// Workflow execution can reset provider context, so peers are not bindings.
pub(crate) fn workflow_node(
    actor: &AgentInstance,
    target: &AgentInstance,
    agents: &[AgentInstance],
) -> Result<(), DaemonError> {
    if target.id() == actor.id() && target.session_id() == actor.session_id() {
        Ok(())
    } else {
        in_subtree(actor, target, agents)
    }
}

/// MP-08 / MP-10 / MP-11 A01-S04: execute an existing public graph in the same room.
/// New bindings still use `workflow_node`; running conveys no mutation authority.
pub(crate) fn workflow_execution_target(
    actor: &AgentInstance,
    target: &AgentInstance,
) -> Result<(), DaemonError> {
    if target.session_id() == actor.session_id() && target.owner_user_id() == actor.owner_user_id()
    {
        Ok(())
    } else {
        Err(denied(
            "workflow execution target is outside the caller room",
        ))
    }
}

pub(crate) fn owns_object(
    actor: &AgentInstance,
    creator: Option<&str>,
    agents: &[AgentInstance],
) -> bool {
    creator == Some(actor.id())
        || creator.is_some_and(|id| {
            agents
                .iter()
                .any(|child| child.id() == id && in_subtree(actor, child, agents).is_ok())
        })
}

/// PR1 admits only room commands. Later capability/sudo PRs expand the typed policy.
pub(crate) fn command(tokens: &[String]) -> Result<(), DaemonError> {
    match tokens.first().map(String::as_str) {
        Some("agent")
            if matches!(
                tokens.get(1).map(String::as_str),
                Some(
                    "list"
                        | "ls"
                        | "spawn"
                        | "alias"
                        | "name"
                        | "delete"
                        | "destroy"
                        | "remove"
                        | "cancel"
                        | "steer"
                        | "message"
                        | "promote"
                        | "deadline"
                        | "children"
                        | "revoke"
                        | "answer"
                        | "pass"
                        | "usage"
                        | "profiles"
                )
            ) =>
        {
            Ok(())
        }
        Some("workflow" | "workflows") if tokens.get(1).map(String::as_str) != Some("pane") => {
            Ok(())
        }
        Some("prompt") => Ok(()),
        // The room's own Project Environment: names and states, and what the user's other machines can hand over.
        Some("environment")
            if matches!(
                tokens.get(1).map(String::as_str),
                Some("status" | "update" | "setup" | "check")
            ) =>
        {
            Ok(())
        }
        // An agent subscribes itself; the request carries no agent argument.
        Some("trigger")
            if matches!(
                tokens.get(1).map(String::as_str),
                Some("notification" | "add" | "list" | "remove")
            ) =>
        {
            Ok(())
        }
        // MP-08/MP-11 A05: App bindings on the user's request or as a child subset.
        Some("extension" | "extensions")
            if matches!(tokens.get(1).map(String::as_str), Some("grant" | "revoke"))
                && tokens.get(2).map(String::as_str) == Some("app") =>
        {
            Ok(())
        }
        Some("session") if tokens.get(1).map(String::as_str) == Some("overview") => Ok(()),
        // What `--room` and `--slice` accept for the caller; the slice lifecycle stays out.
        Some("room")
            if matches!(
                tokens.get(1).map(String::as_str),
                Some("list" | "agents" | "cards")
            ) =>
        {
            Ok(())
        }
        Some("slice") if tokens.len() == 2 && matches!(tokens[1].as_str(), "list" | "ls") => Ok(()),
        _ => Err(denied("command is outside the regular room surface")),
    }
}

/// MP-08 / MP-11: discovery must describe the same room policy as execution.
pub(crate) fn command_doc(mut value: serde_json::Value) -> serde_json::Value {
    let name = value["name"].as_str().unwrap_or_default();
    let tokens =
        crate::runtime::metaagent_command_registry::tokenize_command(name).unwrap_or_default();
    let admitted = command(&tokens).is_ok();
    let destructive_agent = tokens.first().map(String::as_str) == Some("agent")
        && matches!(tokens.get(1).map(String::as_str), Some("alias" | "delete"));
    if let Some(doc) = value.as_object_mut() {
        doc.insert("scope".into(), "room".into());
        doc.insert(
            "authority".into(),
            if !admitted {
                "unavailable on the PR1 regular room surface"
            } else if destructive_agent {
                "immutable direct spawned child"
            } else if matches!(tokens.as_slice(), [a, b] if matches!((a.as_str(), b.as_str()), ("room", "list") | ("slice", "list")))
            {
                "read-only; what --room and --slice accept for this agent"
            } else if tokens.first().map(String::as_str) == Some("room")
                || matches!(tokens.as_slice(), [a,b] if a == "agent" && b == "answer") {
                "owner rooms on this kernel; current authority; secret cards are owner-only; guests cannot reach other rooms"
            } else if tokens.first().map(String::as_str) == Some("extension") {
                "self on the user's request in this turn, or a direct child for an App you hold"
            } else {
                "current room; workflow mutations require self or direct-child creator"
            }
            .into(),
        );
        if !admitted {
            doc.insert("metaagent_policy".into(), "deny".into());
            doc.insert("routed".into(), false.into());
        }
        for key in ["usage", "description"] {
            if let Some(text) = doc.get(key).and_then(|v| v.as_str()) {
                doc.insert(
                    key.into(),
                    text.replace("chariox.meta.session_overview", "chariox.room.overview")
                        .replace("<owned-agent-ref>", "<room-agent-ref>")
                        .replace("metaagent", "agent")
                        .replace("Metaagent", "Agent")
                        .replace("owned regular agents", "room agents")
                        .into(),
                );
            }
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::GridPosition;
    use crate::transport::kernel_protocol::map_kernel_error;

    fn agent(id: &str, room: &str, owner: &str) -> AgentInstance {
        let mut agent = AgentInstance::new(
            id,
            format!("ref-{id}"),
            room,
            None,
            "dev-stub",
            None,
            None,
            None,
            GridPosition {
                row: 0,
                col: 0,
                row_span: 1,
                col_span: 1,
            },
        );
        agent.set_owner_user_id(owner);
        agent
    }

    /// A refusal reaches the user as its own sentence under its own code, and a client
    /// must not retry it.
    fn assert_plain_refusal(error: DaemonError, sentence: &str) {
        assert_eq!(error.to_string(), sentence);
        let mapped = map_kernel_error(&error);
        assert_eq!(mapped.code, REFUSED_CODE);
        assert_eq!(mapped.message, sentence);
        assert!(!mapped.retryable);
    }

    #[test]
    fn a_failure_that_passes_is_a_plain_retryable_sentence() {
        let error = retry_later("Public history changed; retry the query");
        assert_eq!(error.to_string(), "Public history changed; retry the query");
        let mapped = map_kernel_error(&error);
        assert_eq!(mapped.code, RETRY_CODE);
        assert_eq!(mapped.message, error.to_string());
        assert!(mapped.retryable);
    }

    #[test]
    fn every_admission_refusal_is_a_plain_non_retryable_sentence() {
        let actor = agent("actor", "room", "alice");
        let other_room = agent("stranger", "elsewhere", "bob");
        let sibling = agent("sibling", "room", "alice");
        let agents = vec![actor.clone(), sibling.clone()];

        assert_plain_refusal(
            resolve_agent(&agents, "room", "nobody").unwrap_err(),
            "agent is unavailable in this room",
        );
        let twin = agent("twin", "room", "alice");
        let mut aliased = vec![twin.clone(), twin];
        aliased.push(actor.clone());
        assert_plain_refusal(
            resolve_agent(&aliased, "room", "twin").unwrap_err(),
            "ambiguous room agent reference",
        );
        assert_plain_refusal(
            in_subtree(&actor, &sibling, &agents).unwrap_err(),
            "only the agents above an agent in this room can control it",
        );
        assert_plain_refusal(
            workflow_node(&actor, &sibling, &agents).unwrap_err(),
            "only the agents above an agent in this room can control it",
        );
        assert_plain_refusal(
            workflow_execution_target(&actor, &other_room).unwrap_err(),
            "workflow execution target is outside the caller room",
        );
        assert_plain_refusal(
            command(&["agent".to_owned(), "unknown".to_owned()]).unwrap_err(),
            "command is outside the regular room surface",
        );
    }
}
