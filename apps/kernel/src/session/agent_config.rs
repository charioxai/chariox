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

/// Bounds the walk up the parent edges so a corrupt cycle cannot loop.
const PARENT_WALK_LIMIT: usize = 16;

/// `effective(agent) = min(own, effective(parent))` for permission level and mode, read at the
/// moment of use: lowering a parent lowers every agent below it, and an agent the user raised
/// above its parent still runs at the parent's limit. `agents` are the agents of the room.
pub fn effective_agent_execution_config_in_tree(
    session: &RuntimeSession,
    agent: &AgentInstance,
    agents: &[AgentInstance],
) -> EffectiveAgentExecutionConfig {
    let mut config = effective_agent_execution_config(session, Some(agent));
    let mut current = agent;
    for _ in 0..PARENT_WALK_LIMIT {
        let Some(parent) = current
            .parent_agent_id()
            .and_then(|id| agents.iter().find(|candidate| candidate.id() == id))
        else {
            break;
        };
        let parent_config = effective_agent_execution_config(session, Some(parent));
        config.mode = config.mode.lower(parent_config.mode);
        config.permission_level = config
            .permission_level
            .lower(parent_config.permission_level);
        current = parent;
    }
    config
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
