//! Prompt target and active-agent resolution for the agent actor.

use std::borrow::Cow;

use super::*;
use crate::runtime::projection::AgentRuntimeProjection;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct PromptAgentAliasRoute<'a> {
    pub(super) alias: Cow<'a, str>,
    pub(super) prompt: &'a str,
}

pub(super) fn parse_prompt_agent_alias_route(prompt: &str) -> Option<PromptAgentAliasRoute<'_>> {
    let prompt = prompt.trim_start();
    let route = prompt.strip_prefix('@')?;
    if route.starts_with('"') {
        let alias_end = quoted_alias_end(route)?;
        let remainder = &route[alias_end..];
        if !remainder.is_empty() && !remainder.chars().next().is_some_and(char::is_whitespace) {
            return None;
        }
        let alias = serde_json::from_str::<String>(&route[..alias_end]).ok()?;
        if alias.trim().is_empty() {
            return None;
        }
        return Some(PromptAgentAliasRoute {
            alias: Cow::Owned(alias),
            prompt: remainder.trim_start(),
        });
    }
    let alias_end = route.find(char::is_whitespace).unwrap_or(route.len());
    let alias = &route[..alias_end];
    if alias.is_empty() {
        return None;
    }
    Some(PromptAgentAliasRoute {
        alias: Cow::Borrowed(alias),
        prompt: route[alias_end..].trim_start(),
    })
}

fn quoted_alias_end(route: &str) -> Option<usize> {
    let mut escaped = false;
    for (offset, character) in route[1..].char_indices() {
        let index = offset + 1;
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Some(index + character.len_utf8());
        }
    }
    None
}

impl AgentRuntime {
    pub(super) async fn resolve_submit_agent_alias(
        &self,
        session_id: &str,
        alias: &str,
    ) -> Result<String, DaemonError> {
        let session = match self.session_projection.get(session_id) {
            Some(session) => session,
            None => self.store.session_snapshot(session_id).await?,
        };
        session
            .agents()
            .iter()
            .find(|agent| {
                agent.alias().is_some_and(|candidate| {
                    candidate.trim().to_lowercase() == alias.trim().to_lowercase()
                })
            })
            .map(|agent| agent.id().to_string())
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: format!("@{alias}"),
            })
    }

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

#[cfg(test)]
mod alias_route_tests {
    use super::*;

    #[test]
    fn leading_alias_route_strips_only_the_routing_prefix() {
        assert_eq!(
            parse_prompt_agent_alias_route("  @Reviewer   /meta inspect the repo"),
            Some(PromptAgentAliasRoute {
                alias: Cow::Borrowed("Reviewer"),
                prompt: "/meta inspect the repo",
            })
        );
        assert_eq!(
            parse_prompt_agent_alias_route(r#"  @"Review Agent" inspect the repo"#),
            Some(PromptAgentAliasRoute {
                alias: Cow::Owned("Review Agent".to_string()),
                prompt: "inspect the repo",
            })
        );
        assert_eq!(
            parse_prompt_agent_alias_route(r#"@"Review \"Agent\"" inspect the repo"#),
            Some(PromptAgentAliasRoute {
                alias: Cow::Owned(r#"Review "Agent""#.to_string()),
                prompt: "inspect the repo",
            })
        );
        assert_eq!(parse_prompt_agent_alias_route("email @reviewer"), None);
        assert_eq!(parse_prompt_agent_alias_route("@"), None);
        assert_eq!(parse_prompt_agent_alias_route(r#"@"unterminated"#), None);
    }
}
