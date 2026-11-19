use crate::agent::AgentInstance;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};

use super::RuntimeSession;

pub const SESSION_AGENT_MODE_CONFIG_KEY: &str = "agents.mode";
pub const SESSION_AGENT_PERMISSION_CONFIG_KEY: &str = "agents.permissions";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectiveAgentExecutionConfig {
    pub mode: AgentExecutionMode,
    pub permission_level: AgentPermissionLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveAgentUserAuthority {
    Full,
    ApprovalRequired,
}

impl EffectiveAgentUserAuthority {
    pub fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }

    pub fn requires_approval(self) -> bool {
        matches!(self, Self::ApprovalRequired)
    }
}

pub fn effective_agent_execution_config(
    session: &RuntimeSession,
    agent: Option<&AgentInstance>,
) -> EffectiveAgentExecutionConfig {
    EffectiveAgentExecutionConfig {
        mode: effective_agent_execution_mode(session, agent),
        permission_level: effective_agent_permission_level(session, agent),
    }
}

impl EffectiveAgentExecutionConfig {
    pub(crate) fn lower(self, other: Self) -> Self {
        Self {
            mode: self.mode.lower(other.mode),
            permission_level: self.permission_level.lower(other.permission_level),
        }
    }
}

/// Permission and mode are limited by each ancestor in its own Room.
pub fn effective_agent_execution_config_in_tree(
    session: &RuntimeSession,
    agent: &AgentInstance,
    agents: &[AgentInstance],
) -> EffectiveAgentExecutionConfig {
    effective_agent_execution_config_with_parents(
        session,
        agent,
        |id| {
            agents
                .iter()
                .find(|candidate| candidate.id() == id)
                .cloned()
        },
        |_| None,
    )
}

pub(crate) fn effective_agent_execution_config_with_parents(
    session: &RuntimeSession,
    agent: &AgentInstance,
    find_agent: impl Fn(&str) -> Option<AgentInstance>,
    find_room: impl Fn(&str) -> Option<RuntimeSession>,
) -> EffectiveAgentExecutionConfig {
    let mut config = effective_agent_execution_config(session, Some(agent));
    let mut seen = std::collections::HashSet::from([agent.id().to_owned()]);
    let mut parent_id = agent.parent_agent_id().map(str::to_owned);
    while let Some(id) = parent_id.take() {
        let Some(parent) = find_agent(&id).filter(|_| seen.insert(id)) else {
            return config.lower(parent_unavailable_config());
        };
        let parent_config = if parent.session_id() == session.id() {
            effective_agent_execution_config(session, Some(&parent))
        } else if let Some(room) = find_room(parent.session_id()) {
            effective_agent_execution_config(&room, Some(&parent))
        } else {
            parent_unavailable_config()
        };
        config = config.lower(parent_config);
        parent_id = parent.parent_agent_id().map(str::to_owned);
    }
    config
}

fn parent_unavailable_config() -> EffectiveAgentExecutionConfig {
    EffectiveAgentExecutionConfig {
        mode: AgentExecutionMode::Plan,
        permission_level: AgentPermissionLevel::Required,
    }
}

pub fn effective_agent_execution_mode(
    session: &RuntimeSession,
    agent: Option<&AgentInstance>,
) -> AgentExecutionMode {
    agent
        .and_then(AgentInstance::execution_mode_override)
        .or_else(|| {
            session
                .config_state()
                .values()
                .get(SESSION_AGENT_MODE_CONFIG_KEY)
                .and_then(|value| AgentExecutionMode::parse(value))
        })
        .or(session.agent_defaults().execution_mode)
        .unwrap_or_default()
}

pub fn effective_agent_permission_level(
    session: &RuntimeSession,
    agent: Option<&AgentInstance>,
) -> AgentPermissionLevel {
    agent
        .and_then(AgentInstance::permission_level_override)
        .or_else(|| {
            session
                .config_state()
                .values()
                .get(SESSION_AGENT_PERMISSION_CONFIG_KEY)
                .and_then(|value| AgentPermissionLevel::parse(value))
        })
        .or(session.agent_defaults().permission_level)
        .unwrap_or_default()
}

/// What `agent` may do without asking the person, at the level its parent chain allows.
pub fn effective_agent_user_authority(
    session: &RuntimeSession,
    agent: Option<&AgentInstance>,
    agents: &[AgentInstance],
) -> EffectiveAgentUserAuthority {
    let level = match agent {
        Some(agent) => {
            effective_agent_execution_config_in_tree(session, agent, agents).permission_level
        }
        None => effective_agent_permission_level(session, None),
    };
    match level {
        AgentPermissionLevel::Yolo => EffectiveAgentUserAuthority::Full,
        AgentPermissionLevel::Required => EffectiveAgentUserAuthority::ApprovalRequired,
    }
}

pub fn effective_agent_extension_registration_authority(
    session: &RuntimeSession,
    agent: Option<&AgentInstance>,
    agents: &[AgentInstance],
) -> EffectiveAgentUserAuthority {
    effective_agent_user_authority(session, agent, agents)
}

#[cfg(test)]
mod tests {
    use crate::agent::{AgentInstance, GridPosition};
    use crate::config::DaemonConfig;
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::{CreateSessionRequest, SessionAgentDefaults, SessionService};

    use super::*;

    #[test]
    fn effective_config_uses_session_defaults() {
        let mut sessions = SessionService::new(&DaemonConfig::for_tests());
        let request = CreateSessionRequest::new("workspace", "worktree").with_agent_defaults(
            SessionAgentDefaults::new("dev-stub")
                .with_execution_mode(AgentExecutionMode::Plan)
                .with_permission_level(AgentPermissionLevel::Required),
        );
        let session = sessions
            .create_session(request)
            .expect("session should be created");

        let config = effective_agent_execution_config(&session, None);

        assert_eq!(config.mode, AgentExecutionMode::Plan);
        assert_eq!(config.permission_level, AgentPermissionLevel::Required);
    }

    #[test]
    fn effective_config_prefers_agent_override_over_session_defaults() {
        let mut sessions = SessionService::new(&DaemonConfig::for_tests());
        let request = CreateSessionRequest::new("workspace", "worktree").with_agent_defaults(
            SessionAgentDefaults::new("dev-stub")
                .with_execution_mode(AgentExecutionMode::Plan)
                .with_permission_level(AgentPermissionLevel::Required),
        );
        let session = sessions
            .create_session(request)
            .expect("session should be created");
        let mut agent = AgentInstance::new(
            "agent-1",
            "ref-1",
            session.id(),
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
        agent.set_execution_mode_override(Some(AgentExecutionMode::Build));
        agent.set_permission_level_override(Some(AgentPermissionLevel::Yolo));

        let config = effective_agent_execution_config(&session, Some(&agent));

        assert_eq!(config.mode, AgentExecutionMode::Build);
        assert_eq!(config.permission_level, AgentPermissionLevel::Yolo);
    }

    #[test]
    fn effective_user_authority_tracks_permission_level() {
        let mut sessions = SessionService::new(&DaemonConfig::for_tests());
        let session = sessions
            .create_session(
                CreateSessionRequest::new("workspace", "worktree").with_agent_defaults(
                    SessionAgentDefaults::new("dev-stub")
                        .with_permission_level(AgentPermissionLevel::Required),
                ),
            )
            .expect("session should be created");

        assert_eq!(
            effective_agent_user_authority(&session, None, &[]),
            EffectiveAgentUserAuthority::ApprovalRequired
        );

        let mut agent = AgentInstance::new(
            "agent-1",
            "ref-1",
            session.id(),
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
        agent.set_permission_level_override(Some(AgentPermissionLevel::Yolo));

        assert_eq!(
            effective_agent_user_authority(&session, Some(&agent), &[]),
            EffectiveAgentUserAuthority::Full
        );
    }

    fn tree_agent(
        id: &str,
        session_id: &str,
        parent: Option<&str>,
        mode: AgentExecutionMode,
        level: AgentPermissionLevel,
    ) -> AgentInstance {
        let mut agent = AgentInstance::new(
            id,
            format!("ref-{id}"),
            session_id,
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
        agent.set_parent_agent(parent.map(crate::agent::AgentRef::local));
        agent.set_execution_mode_override(Some(mode));
        agent.set_permission_level_override(Some(level));
        agent
    }

    /// A5: effective(agent) = min(own, effective(parent)) for every parent level x child
    /// level x parent mode x child mode, and down a grandchild.
    #[test]
    fn an_agent_never_runs_above_its_parent_at_the_moment_of_use() {
        use AgentExecutionMode::{Build, Plan};
        use AgentPermissionLevel::{Required, Yolo};
        let mut sessions = SessionService::new(&DaemonConfig::for_tests());
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        for parent_level in [Required, Yolo] {
            for child_level in [Required, Yolo] {
                for parent_mode in [Plan, Build] {
                    for child_mode in [Plan, Build] {
                        let agents = vec![
                            tree_agent("p", session.id(), None, parent_mode, parent_level),
                            tree_agent("c", session.id(), Some("p"), child_mode, child_level),
                            tree_agent("g", session.id(), Some("c"), Build, Yolo),
                        ];
                        let child =
                            effective_agent_execution_config_in_tree(&session, &agents[1], &agents);
                        let lower_level = if parent_level == Required || child_level == Required {
                            Required
                        } else {
                            Yolo
                        };
                        let lower_mode = if parent_mode == Plan || child_mode == Plan {
                            Plan
                        } else {
                            Build
                        };
                        assert_eq!(
                            (child.permission_level, child.mode),
                            (lower_level, lower_mode),
                            "{parent_level:?}/{child_level:?} {parent_mode}/{child_mode}"
                        );
                        let grandchild =
                            effective_agent_execution_config_in_tree(&session, &agents[2], &agents);
                        assert_eq!(
                            (grandchild.permission_level, grandchild.mode),
                            (lower_level, lower_mode),
                            "a grandchild follows the whole chain"
                        );
                    }
                }
            }
        }
    }
}
