//! A sub-agent never has more authority than the agent that started it: permission level, mode,
//! folders and fan-out are checked where the child is created, against the parent's current
//! settings. Tools, secrets, Apps and the account profile are not limited.

use std::path::Path;

use super::{AgentInstance, AgentState, CreateAgentRequest};
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::error::DaemonError;
use crate::session::{effective_agent_execution_config_in_tree, RuntimeSession};

pub(crate) const MAX_DEPTH: usize = 4;
pub(crate) const MAX_CHILDREN_PER_PARENT: usize = 16;
pub(crate) const MAX_AGENTS_PER_TREE: usize = 64;

/// A child holds one of its parent's sixteen places while it can still be working: one that has
/// had its turn and is idle again, or has failed, is finished and only counts in the team total.
/// A child that was never prompted is a prepared agent, not a finished one. The stored state is
/// not the prompt activity (a running turn leaves it Idle), so a child also stays unfinished
/// while it has an active or queued prompt, an unfinished task or an armed wake.
fn counts_toward_fan_out(agent: &AgentInstance, session: &RuntimeSession) -> bool {
    !agent.archived()
        && (agent.state() == AgentState::Working
            || agent.last_prompt_sent_at_ms().is_none()
            || session.active_prompt_for_agent(agent.id()).is_some()
            || session
                .queued_prompts_for_agent(agent.id())
                .is_some_and(|queued| !queued.is_empty())
            || session.agent_tasks().iter().any(|task| {
                task.agent_id == agent.id()
                    && matches!(
                        task.state,
                        ExecutionState::Working | ExecutionState::Waiting | ExecutionState::Blocked
                    )
            })
            || session
                .agent_wakes()
                .iter()
                .any(|wake| wake.agent_id == agent.id() && wake.is_armed()))
}

fn refused(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "agent.spawn",
        message,
    }
}

/// The agents above `agent`, nearest first. Bounded, so a corrupt cycle cannot loop.
pub(crate) fn ancestors<'a>(
    agent: &'a AgentInstance,
    agents: &'a [AgentInstance],
) -> Vec<&'a AgentInstance> {
    let mut found = Vec::new();
    let mut current = agent;
    for _ in 0..=MAX_DEPTH * 4 {
        let Some(parent) = current
            .parent_agent_id()
            .and_then(|id| agents.iter().find(|candidate| candidate.id() == id))
        else {
            break;
        };
        found.push(parent);
        current = parent;
    }
    found
}

/// The agents below `agent_id`, at any depth.
pub(crate) fn descendants<'a>(
    agent_id: &str,
    agents: &'a [AgentInstance],
) -> Vec<&'a AgentInstance> {
    agents
        .iter()
        .filter(|agent| ancestors(agent, agents).iter().any(|a| a.id() == agent_id))
        .collect()
}

/// The agents above `agent` up to its root, nearest first, or `None` when the walk meets an
/// agent twice (a cycle). Unbounded by depth, so a long chain is never taken for a root.
fn chain_above<'a>(
    agent: &'a AgentInstance,
    agents: &'a [AgentInstance],
) -> Option<Vec<&'a AgentInstance>> {
    let mut seen = std::collections::HashSet::from([agent.id()]);
    let mut found = Vec::new();
    let mut current = agent;
    while let Some(parent) = current
        .parent_agent_id()
        .and_then(|id| agents.iter().find(|candidate| candidate.id() == id))
    {
        if !seen.insert(parent.id()) {
            return None;
        }
        found.push(parent);
        current = parent;
    }
    Some(found)
}

/// Refuses placing `agent` (with its subtree) under `parent` when that closes a cycle or the
/// moved subtree would exceed the depth, fan-out or team size a spawn is held to.
pub(crate) fn check_placement(
    agent: &AgentInstance,
    parent: &AgentInstance,
    agents: &[AgentInstance],
    session: &RuntimeSession,
) -> Result<(), String> {
    let above = chain_above(parent, agents)
        .ok_or("the agent tree has a cycle; place the agents as room agents first")?;
    if parent.id() == agent.id() || above.iter().any(|a| a.id() == agent.id()) {
        return Err(
            "an agent cannot be placed under itself or under one of its descendants".into(),
        );
    }
    let subtree = descendants(agent.id(), agents)
        .into_iter()
        .filter(|a| !a.archived())
        .collect::<Vec<_>>();
    let height = subtree
        .iter()
        .map(|a| {
            chain_above(a, agents)
                .unwrap_or_default()
                .iter()
                .position(|b| b.id() == agent.id())
                .map_or(0, |at| at + 1)
        })
        .max()
        .unwrap_or(0);
    if above.len() + 1 + height > MAX_DEPTH {
        return Err(format!(
            "sub-agents are limited to {MAX_DEPTH} levels below a room agent; place this agent higher or move part of its team first"
        ));
    }
    let children = agents
        .iter()
        .filter(|a| {
            counts_toward_fan_out(a, session)
                && a.id() != agent.id()
                && a.parent_agent_id() == Some(parent.id())
        })
        .count();
    if children >= MAX_CHILDREN_PER_PARENT {
        return Err(format!(
            "an agent can have at most {MAX_CHILDREN_PER_PARENT} sub-agents that are not finished; place this agent under another one"
        ));
    }
    let root = above.last().copied().unwrap_or(parent);
    let moved = std::iter::once(agent.id())
        .chain(subtree.iter().map(|a| a.id()))
        .collect::<std::collections::HashSet<_>>();
    let team = agents
        .iter()
        .filter(|a| {
            !a.archived()
                && !moved.contains(a.id())
                && chain_above(a, agents)
                    .unwrap_or_default()
                    .iter()
                    .any(|b| b.id() == root.id())
        })
        .count();
    if team + moved.len() > MAX_AGENTS_PER_TREE {
        return Err(format!(
            "a team can have at most {MAX_AGENTS_PER_TREE} sub-agents below its room agent; place this agent under another room agent"
        ));
    }
    Ok(())
}

/// Refuses a child above its parent's current level or mode, or beyond the fan-out caps.
/// `agents` are the agents of the parent's room.
pub(crate) fn check_spawn(
    request: &CreateAgentRequest,
    parent: &AgentInstance,
    agents: &[AgentInstance],
    earlier_in_batch: &[CreateAgentRequest],
    session: &RuntimeSession,
) -> Result<(), DaemonError> {
    let limit = effective_agent_execution_config_in_tree(session, parent, agents);
    let level_ok = request
        .permission_level_override
        .is_none_or(|level| limit.permission_level.permits(level));
    let mode_ok = request
        .execution_mode_override
        .is_none_or(|mode| limit.mode.permits(mode));
    if !level_ok || !mode_ok {
        return Err(refused(format!(
            "a sub-agent cannot have more authority than its parent: the parent runs {:?} in {} mode; spawn with a permission and mode at or below it",
            limit.permission_level, limit.mode
        )));
    }
    let above_parent = ancestors(parent, agents);
    if above_parent.len() + 1 > MAX_DEPTH {
        return Err(refused(format!(
            "sub-agents are limited to {MAX_DEPTH} levels below a room agent; do the work in this agent or ask its parent"
        )));
    }
    let children = agents
        .iter()
        .filter(|agent| {
            counts_toward_fan_out(agent, session) && agent.parent_agent_id() == Some(parent.id())
        })
        .count()
        + earlier_in_batch
            .iter()
            .filter(|earlier| earlier.parent_agent_id == request.parent_agent_id)
            .count();
    if children >= MAX_CHILDREN_PER_PARENT {
        return Err(refused(format!(
            "an agent can have at most {MAX_CHILDREN_PER_PARENT} sub-agents that are not finished; wait for one to finish"
        )));
    }
    let root = above_parent.last().copied().unwrap_or(parent);
    let tree = agents
        .iter()
        .filter(|agent| {
            !agent.archived() && ancestors(agent, agents).iter().any(|a| a.id() == root.id())
        })
        .count()
        + earlier_in_batch
            .iter()
            .filter(|earlier| earlier.parent_agent_id.is_some())
            .count();
    if tree >= MAX_AGENTS_PER_TREE {
        return Err(refused(format!(
            "a team can have at most {MAX_AGENTS_PER_TREE} sub-agents below its room agent; delete the ones that are done"
        )));
    }
    Ok(())
}

/// A child's folder lies inside the parent's worktree or workspace. Both must exist: the spawn
/// already requires the folder to exist, so a path that cannot be resolved is refused.
pub(crate) fn check_directory(
    parent: &AgentInstance,
    session: &RuntimeSession,
    directory: &str,
) -> Result<(), DaemonError> {
    let wanted = Path::new(directory).canonicalize().map_err(|error| {
        refused(format!(
            "the directory `{directory}` cannot be used: {error}"
        ))
    })?;
    keep_inside_parent_folders(parent, session, directory, &wanted)
}

/// The same limit for a folder a spawn is about to create, such as a git worktree: the deepest
/// existing ancestor is resolved, and the part that does not exist yet may not climb out.
pub(crate) fn check_directory_to_create(
    parent: &AgentInstance,
    session: &RuntimeSession,
    directory: &Path,
) -> Result<(), DaemonError> {
    let mut existing = directory;
    let mut missing = Vec::new();
    while !existing.exists() {
        let (Some(name), Some(above)) = (existing.file_name(), existing.parent()) else {
            return Err(refused(format!(
                "the directory `{}` cannot be used",
                directory.display()
            )));
        };
        missing.push(name);
        existing = above;
    }
    let mut wanted = existing.canonicalize().map_err(|error| {
        refused(format!(
            "the directory `{}` cannot be used: {error}",
            directory.display()
        ))
    })?;
    wanted.extend(missing.into_iter().rev());
    keep_inside_parent_folders(parent, session, &directory.display().to_string(), &wanted)
}

fn keep_inside_parent_folders(
    parent: &AgentInstance,
    session: &RuntimeSession,
    directory: &str,
    wanted: &Path,
) -> Result<(), DaemonError> {
    let roots = [
        parent
            .worktree_id()
            .unwrap_or_else(|| session.worktree_id()),
        parent
            .workspace_id()
            .unwrap_or_else(|| session.workspace_id()),
    ];
    if roots
        .iter()
        .filter_map(|root| Path::new(root).canonicalize().ok())
        .any(|root| wanted.starts_with(root))
    {
        return Ok(());
    }
    Err(refused(format!(
        "a sub-agent works inside its parent's folders; `{directory}` is outside them. Use a folder under the parent's workspace"
    )))
}

/// Test hook: runs once between a spawn's check and its effect, so a test can change the
/// parent exactly there.
#[cfg(test)]
type SpawnCommitHook = (String, Box<dyn FnOnce() + Send>);

#[cfg(test)]
static BEFORE_SPAWN_COMMIT: std::sync::Mutex<Vec<SpawnCommitHook>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
pub(crate) fn set_hook_before_spawn_commit(session_id: &str, hook: impl FnOnce() + Send + 'static) {
    BEFORE_SPAWN_COMMIT
        .lock()
        .unwrap()
        .push((session_id.to_string(), Box::new(hook)));
}

#[cfg(test)]
pub(crate) fn run_hook_before_spawn_commit(session_id: &str) {
    let hook = {
        let mut hooks = BEFORE_SPAWN_COMMIT.lock().unwrap();
        hooks
            .iter()
            .position(|(id, _)| id == session_id)
            .map(|index| hooks.remove(index).1)
    };
    if let Some(hook) = hook {
        hook();
    }
}
