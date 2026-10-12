//! MP-08 / MP-11: shared room admission and immutable direct-creator policy.
use crate::{agent::AgentInstance, error::DaemonError};

pub(crate) fn denied(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "room tool admission",
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

pub(crate) fn direct_child(
    actor: &AgentInstance,
    target: &AgentInstance,
) -> Result<(), DaemonError> {
    if target.session_id() == actor.session_id()
        && target.id() != actor.id()
        && target.spawned_by_agent_id() == Some(actor.id())
    {
        Ok(())
    } else {
        Err(denied(
            "destructive agent actions require an immutable direct child in this room",
        ))
    }
}

/// Workflow execution can reset provider context, so peers are not bindings.
pub(crate) fn workflow_node(
    actor: &AgentInstance,
    target: &AgentInstance,
) -> Result<(), DaemonError> {
    if target.id() == actor.id() && target.session_id() == actor.session_id() {
        Ok(())
    } else {
        direct_child(actor, target)
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
                .any(|child| child.id() == id && direct_child(actor, child).is_ok())
        })
}

/// PR1 admits only room commands. Later capability/sudo PRs expand the typed policy.
pub(crate) fn command(tokens: &[String]) -> Result<(), DaemonError> {
    match tokens.first().map(String::as_str) {
        Some("agent")
            if matches!(
                tokens.get(1).map(String::as_str),
                Some("list" | "ls" | "spawn" | "alias" | "name" | "delete" | "destroy" | "remove")
            ) =>
        {
            Ok(())
        }
        Some("workflow" | "workflows") if tokens.get(1).map(String::as_str) != Some("pane") => {
            Ok(())
        }
        Some("prompt") => Ok(()),
        // MP-08/MP-11 A05: App bindings on the user's request or as a child subset.
        Some("extension" | "extensions")
            if matches!(tokens.get(1).map(String::as_str), Some("grant" | "revoke"))
                && tokens.get(2).map(String::as_str) == Some("app") =>
        {
            Ok(())
        }
        Some("session") if tokens.get(1).map(String::as_str) == Some("overview") => Ok(()),
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
