//! The rules a person granted with "Allow for this session", kept for the agent's whole session.
//!
//! Claude holds a session rule only inside one process. The kernel restarts that process whenever a
//! turn is cancelled, the model or permissions change, Claude exits, or the kernel itself restarts,
//! so a rule kept only there would end at a point the person cannot see. The kernel keeps what was
//! granted per agent and, when a new process asks about a call that grant already covers, answers
//! at once and hands the rules to the new process again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::provider::{AgentExecutionMode, AgentPermissionLevel};

use super::SessionGrant;

/// Agents whose grants are kept; the oldest agent's grants go first beyond this.
const MAX_AGENTS: usize = 256;

/// What a grant was given under. A person who changes the working directory, the permission level
/// or the execution mode has changed what the agent may do, so the grant ends there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GrantScope {
    pub working_directory: Option<PathBuf>,
    pub permission_level: AgentPermissionLevel,
    pub execution_mode: AgentExecutionMode,
}

struct Standing {
    scope: GrantScope,
    allow: BTreeSet<String>,
    ask: BTreeSet<String>,
}

/// Grants by agent id. The kernel's memory only: a kernel restart ends them.
#[derive(Clone, Default)]
pub(crate) struct StandingGrants {
    agents: Arc<Mutex<BTreeMap<String, Standing>>>,
}

impl StandingGrants {
    /// Adds what the person just granted to this agent's standing grant.
    pub(crate) fn remember(&self, agent_id: &str, scope: GrantScope, grant: &SessionGrant) {
        let mut agents = self.agents.lock().expect("standing grants lock");
        if !agents.contains_key(agent_id) && agents.len() >= MAX_AGENTS {
            agents.pop_first();
        }
        let standing = agents
            .entry(agent_id.to_string())
            .or_insert_with(|| Standing {
                scope: scope.clone(),
                allow: BTreeSet::new(),
                ask: BTreeSet::new(),
            });
        if standing.scope != scope {
            *standing = Standing {
                scope,
                allow: BTreeSet::new(),
                ask: BTreeSet::new(),
            };
        }
        standing.allow.extend(grant.allow.iter().cloned());
        standing.ask.extend(grant.ask.iter().cloned());
    }

    /// The whole standing grant when it already covers every rule `wanted` needs.
    pub(crate) fn covering(
        &self,
        agent_id: &str,
        scope: &GrantScope,
        wanted: &SessionGrant,
    ) -> Option<SessionGrant> {
        let mut agents = self.agents.lock().expect("standing grants lock");
        let standing = agents.get(agent_id)?;
        if &standing.scope != scope {
            agents.remove(agent_id);
            return None;
        }
        wanted
            .allow
            .iter()
            .all(|rule| standing.allow.contains(rule))
            .then(|| SessionGrant {
                allow: standing.allow.iter().cloned().collect(),
                ask: standing.ask.iter().cloned().collect(),
                ask_names: Vec::new(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(level: AgentPermissionLevel) -> GrantScope {
        GrantScope {
            working_directory: Some(PathBuf::from("/work/app")),
            permission_level: level,
            execution_mode: AgentExecutionMode::Build,
        }
    }

    fn grant(allow: &[&str]) -> SessionGrant {
        SessionGrant {
            allow: allow.iter().map(|rule| rule.to_string()).collect(),
            ask: Vec::new(),
            ask_names: Vec::new(),
        }
    }

    #[test]
    fn a_grant_covers_the_calls_it_was_given_for_and_hands_back_everything_granted() {
        let grants = StandingGrants::default();
        let required = scope(AgentPermissionLevel::Required);
        grants.remember("a", required.clone(), &grant(&["Bash(cargo test:*)"]));
        grants.remember("a", required.clone(), &grant(&["Bash(git add:*)"]));
        let covering = grants
            .covering("a", &required, &grant(&["Bash(cargo test:*)"]))
            .expect("covered");
        assert_eq!(covering.allow, ["Bash(cargo test:*)", "Bash(git add:*)"]);
        assert!(grants
            .covering(
                "a",
                &required,
                &grant(&["Bash(cargo test:*)", "Bash(git commit:*)"])
            )
            .is_none());
        assert!(grants
            .covering("other", &required, &grant(&["Bash(cargo test:*)"]))
            .is_none());
    }

    #[test]
    fn a_changed_scope_ends_the_grant() {
        let grants = StandingGrants::default();
        grants.remember(
            "a",
            scope(AgentPermissionLevel::Required),
            &grant(&["Bash(cargo test:*)"]),
        );
        let changed = scope(AgentPermissionLevel::Yolo);
        assert!(grants
            .covering("a", &changed, &grant(&["Bash(cargo test:*)"]))
            .is_none());
        assert!(grants
            .covering(
                "a",
                &scope(AgentPermissionLevel::Required),
                &grant(&["Bash(cargo test:*)"])
            )
            .is_none());
    }
}
