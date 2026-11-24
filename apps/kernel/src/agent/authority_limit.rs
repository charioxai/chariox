//! A sub-agent never has more authority than the agent that started it: permission level, mode
//! and folders are checked where the child is created, against the parent's current settings.
//! Tools, secrets, Apps and the account profile are not limited.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::{AgentInstance, CreateAgentRequest};
use crate::error::DaemonError;
use crate::session::{EffectiveAgentExecutionConfig, RuntimeSession};

fn refused(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "agent.spawn",
        message,
    }
}

/// The agents above `agent`, nearest first. A repeated agent ends the walk.
pub(crate) fn ancestors<'a>(
    agent: &'a AgentInstance,
    agents: &'a [AgentInstance],
) -> Vec<&'a AgentInstance> {
    parent_chain(agent, agents).0
}

fn parent_chain<'a>(
    agent: &'a AgentInstance,
    agents: &'a [AgentInstance],
) -> (Vec<&'a AgentInstance>, bool) {
    let mut seen = HashSet::from([agent.id()]);
    let mut found = Vec::new();
    let mut current = agent;
    while let Some(parent) = current
        .parent_agent_id()
        .and_then(|id| agents.iter().find(|candidate| candidate.id() == id))
    {
        if !seen.insert(parent.id()) {
            return (found, true);
        }
        found.push(parent);
        current = parent;
    }
    (found, false)
}

/// The agents below `agent_id`, at any depth, in their original order.
pub(crate) fn descendants<'a>(
    agent_id: &str,
    agents: &'a [AgentInstance],
) -> Vec<&'a AgentInstance> {
    let mut children = HashMap::<&str, Vec<&str>>::new();
    for agent in agents {
        if let Some(parent) = agent.parent_agent_id() {
            children.entry(parent).or_default().push(agent.id());
        }
    }
    let mut seen = HashSet::from([agent_id]);
    let mut pending = vec![agent_id];
    while let Some(parent) = pending.pop() {
        if let Some(children) = children.get(parent) {
            pending.extend(children.iter().copied().filter(|child| seen.insert(*child)));
        }
    }
    agents
        .iter()
        .filter(|agent| agent.id() != agent_id && seen.contains(agent.id()))
        .collect()
}

/// Placing an agent under a parent must keep the tree acyclic.
pub(crate) fn check_placement(
    agent: &AgentInstance,
    parent: &AgentInstance,
    agents: &[AgentInstance],
) -> Result<(), String> {
    let (above, cyclic) = parent_chain(parent, agents);
    if cyclic {
        return Err("the agent tree has a cycle; place the agents as room agents first".into());
    }
    if parent.id() == agent.id() || above.iter().any(|a| a.id() == agent.id()) {
        return Err(
            "an agent cannot be placed under itself or under one of its descendants".into(),
        );
    }
    Ok(())
}

/// Refuses a child above its parent's current permission level or mode.
pub(crate) fn check_spawn(
    request: &CreateAgentRequest,
    limit: EffectiveAgentExecutionConfig,
) -> Result<(), DaemonError> {
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
    Ok(())
}

/// A child works inside its parent's folders or in another worktree of the same repository.
/// Existing folders are resolved before checking either grant, including symlink targets.
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
    check_directory_scope(parent, session, directory, &wanted, &wanted)
}

/// A new Git worktree may leave the parent's folders only when its source is the parent's
/// repository. Git creates the target in that repository before the agent is registered.
pub(crate) fn check_directory_to_create(
    parent: &AgentInstance,
    session: &RuntimeSession,
    directory: &Path,
    source_directory: &Path,
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
    check_directory_scope(
        parent,
        session,
        &directory.display().to_string(),
        &wanted,
        source_directory,
    )
}

fn check_directory_scope(
    parent: &AgentInstance,
    session: &RuntimeSession,
    directory: &str,
    wanted: &Path,
    repository_directory: &Path,
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
        || shares_parent_repository(parent, session, repository_directory)
    {
        return Ok(());
    }
    Err(refused(format!(
        "a sub-agent works inside its parent's folders or in a worktree of the same Git repository; `{directory}` is outside that scope. Use a folder under the parent's workspace or a worktree sharing its repository's Git common directory"
    )))
}

fn shares_parent_repository(
    parent: &AgentInstance,
    session: &RuntimeSession,
    directory: &Path,
) -> bool {
    use crate::runtime::workspace_git_common::git_common_directory;

    let parent_directory = parent
        .worktree_id()
        .unwrap_or_else(|| session.worktree_id());
    git_common_directory(Path::new(parent_directory))
        .zip(git_common_directory(directory))
        .is_some_and(|(parent, child)| parent == child)
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
