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

pub fn effective_agent_user_authority(
    session: &RuntimeSession,
    agent: Option<&AgentInstance>,
) -> EffectiveAgentUserAuthority {
    match effective_agent_permission_level(session, agent) {
        AgentPermissionLevel::Yolo => EffectiveAgentUserAuthority::Full,
        AgentPermissionLevel::Required => EffectiveAgentUserAuthority::ApprovalRequired,
    }
}

pub fn effective_agent_extension_registration_authority(
    session: &RuntimeSession,
    agent: Option<&AgentInstance>,
) -> EffectiveAgentUserAuthority {
    effective_agent_user_authority(session, agent)
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
            effective_agent_user_authority(&session, None),
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
            effective_agent_user_authority(&session, Some(&agent)),
            EffectiveAgentUserAuthority::Full
        );
    }
}
