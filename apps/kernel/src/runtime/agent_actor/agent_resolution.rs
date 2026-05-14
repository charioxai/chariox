//! Prompt target and active-agent resolution for the agent actor.

use super::*;
use crate::runtime::projection::AgentRuntimeProjection;

impl AgentRuntime {
    pub(super) async fn resolve_active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(agent_id) = self
            .resolve_projected_active_prompt_agent_id(session_id)
            .await
        {
            return Ok(agent_id);
        }
        if let Some(agent_id) = self
            .session_projection
            .get(session_id)
            .and_then(|session| self.prompt_state_owner.active_prompt_agent_id(&session))
        {
            return Ok(agent_id);
        }
        if self.session_projection.get(session_id).is_some()
            || !self
                .agent_runtime_projection
                .list_for_session(session_id)
                .is_empty()
        {
            return Err(DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            });
        }
        if self.session_projection.has_warmed_list() {
            return Err(DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.store
            .active_prompt_agent_id(session_id)
            .await?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })
    }

    async fn resolve_projected_active_prompt_agent_id(&self, session_id: &str) -> Option<String> {
        if let Some(focused_agent_id) = self.focus_projection.focused_agent_id(session_id).await {
            if self
                .agent_runtime_projection
                .get(&focused_agent_id)
                .is_some_and(|projection| {
                    projection.session_id == session_id && projection.active_prompt.is_some()
                })
            {
                return Some(focused_agent_id);
            }
        }

        let session_focused_agent_id = self
            .session_projection
            .get(session_id)
            .and_then(|session| session.focused_agent_id().map(str::to_string));
        if let Some(focused_agent_id) = session_focused_agent_id.as_deref() {
            if self
                .agent_runtime_projection
                .get(focused_agent_id)
                .is_some_and(|projection| {
                    projection.session_id == session_id && projection.active_prompt.is_some()
                })
            {
                return Some(focused_agent_id.to_string());
            }
        }

        active_prompt_agent_id_from_projections(
            session_focused_agent_id.as_deref(),
            &self.agent_runtime_projection.list_for_session(session_id),
        )
    }

    pub(super) async fn resolve_submit_agent_id(
        &self,
        session_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        let session_projection = self.session_projection.get(session_id);
        if session_projection.is_none()
            && self.session_projection.has_warmed_list()
            && self
                .agent_runtime_projection
                .list_for_session(session_id)
                .is_empty()
        {
            return Err(DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            });
        }
        if let Some(agent_id) = target_agent_id {
            if let Some(session) = session_projection.as_ref() {
                if !session.agents().iter().any(|agent| agent.id() == agent_id) {
                    return Err(DaemonError::AgentNotInSession {
                        session_id: session_id.to_string(),
                        agent_id: agent_id.to_string(),
                    });
                }
            }
            return Ok(agent_id.to_string());
        }
        if let Some(agent_id) = self.focus_projection.focused_agent_id(session_id).await {
            return Ok(agent_id);
        }
        if let Some(agent_id) =
            session_projection.and_then(|session| session.focused_agent_id().map(str::to_string))
        {
            return Ok(agent_id);
        }
        if let Some(agent_id) =
            single_agent_projection_id(&self.agent_runtime_projection.list_for_session(session_id))
        {
            return Ok(agent_id);
        }
        self.store
            .focused_agent_id(session_id)
            .await?
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "no focused agent".to_string(),
            })
    }
}

fn active_prompt_agent_id_from_projections(
    focused_agent_id: Option<&str>,
    projections: &[AgentRuntimeProjection],
) -> Option<String> {
    if let Some(focused_agent_id) = focused_agent_id {
        if projections.iter().any(|projection| {
            projection.agent_id == focused_agent_id && projection.active_prompt.is_some()
        }) {
            return Some(focused_agent_id.to_string());
        }
    }
    let mut active_agents = projections
        .iter()
        .filter(|projection| projection.active_prompt.is_some())
        .map(|projection| projection.agent_id.clone());
    let agent_id = active_agents.next()?;
    if active_agents.next().is_none() {
        Some(agent_id)
    } else {
        None
    }
}

fn single_agent_projection_id(projections: &[AgentRuntimeProjection]) -> Option<String> {
    let mut agent_ids = projections
        .iter()
        .map(|projection| projection.agent_id.clone())
        .collect::<Vec<_>>();
    agent_ids.sort();
    agent_ids.dedup();
    if agent_ids.len() == 1 {
        agent_ids.into_iter().next()
    } else {
        None
    }
}
