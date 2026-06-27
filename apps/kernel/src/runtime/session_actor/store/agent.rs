//! Agent-specific session-store request adapters.

use std::collections::HashMap;

use crate::agent::CreateAgentRequest;
use crate::error::DaemonError;
use crate::local::{
    AliasAgentRequest, DestroyAgentRequest, ForkAgentRequest, LocalDaemonResponse,
    SpawnAgentRequest, SpawnAgentsRequest, UndoTurnRequest, UpdateAgentConfigRequest,
    UpdateAgentProfileRequest, UpdateAgentSubstitutesRequest,
};

use super::super::projection_policy::SessionProjectionAction;
use super::SessionRuntimeStore;

impl SessionRuntimeStore {
    const MAX_BATCH_SPAWN_AGENTS: usize = 500;

    pub(in crate::runtime::session_actor) async fn update_agent_config(
        &self,
        request: UpdateAgentConfigRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let session_id = request.session_id.clone();
        let execution_mode_override = if request.clear_execution_mode {
            Some(None)
        } else {
            request.execution_mode.map(Some)
        };
        let permission_level_override = if request.clear_permission_level {
            Some(None)
        } else {
            request.permission_level.map(Some)
        };
        let workspace_id = if request.clear_workspace_id {
            Some(None)
        } else {
            request.workspace_id.map(Some)
        };
        let worktree_id = if request.clear_worktree_id {
            Some(None)
        } else {
            request.worktree_id.map(Some)
        };
        let result = match self
            .state
            .update_agent_config(
                &request.session_id,
                &request.agent_id,
                &caller_user_id,
                execution_mode_override,
                permission_level_override,
                workspace_id,
                worktree_id,
            )
            .await
        {
            Ok(agent) => self
                .state
                .session_snapshot(&session_id)
                .await
                .map(|session| LocalDaemonResponse::AgentConfigUpdated { agent, session }),
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn update_agent_profile(
        &self,
        request: UpdateAgentProfileRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let session_id = request.session_id.clone();
        let effort = if request.clear_effort {
            Some(None)
        } else {
            request.effort.map(Some)
        };
        let result = match self
            .state
            .update_agent_profile(
                &request.session_id,
                &request.agent_id,
                &caller_user_id,
                request.provider,
                request.model,
                effort,
            )
            .await
        {
            Ok(agent) => self
                .state
                .session_snapshot(&session_id)
                .await
                .map(|session| LocalDaemonResponse::AgentProfileUpdated { agent, session }),
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn alias_agent(
        &self,
        request: AliasAgentRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let session_id = request.session_id.clone();
        let alias = request.alias.trim();
        let alias = if alias.is_empty() || matches!(alias, "clear" | "none" | "-") {
            None
        } else {
            Some(alias.to_string())
        };
        let result = match self
            .state
            .alias_agent(
                &request.session_id,
                &request.agent_id,
                &caller_user_id,
                alias,
            )
            .await
        {
            Ok(agent) => self
                .state
                .session_snapshot(&session_id)
                .await
                .map(|session| LocalDaemonResponse::AgentAliased { agent, session }),
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn update_agent_substitutes(
        &self,
        request: UpdateAgentSubstitutesRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let session_id = request.session_id.clone();
        let result = match self
            .state
            .update_agent_substitutes(
                &request.session_id,
                &request.agent_id,
                &caller_user_id,
                request.action,
            )
            .await
        {
            Ok(agent) => self
                .state
                .session_snapshot(&session_id)
                .await
                .map(|session| LocalDaemonResponse::AgentConfigUpdated { agent, session }),
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn spawn_agent(
        &self,
        request: SpawnAgentRequest,
        caller_user_id: String,
        caller_metaagent_id: Option<String>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        if request.metaagent {
            return self
                .with_session_projection_action_result(Err(DaemonError::LocalTransport {
                    operation: "agent.spawn",
                    message: "creating separate metaagents is deprecated; send `/meta <task>` to a regular agent to enter meta mode".to_string(),
                }))
                .await;
        }
        let session = match self.state.session_snapshot(&request.session_id).await {
            Ok(session) => session,
            Err(error) => return self.with_session_projection_action_result(Err(error)).await,
        };
        let defaults = session.agent_defaults();
        let model = request.model.or_else(|| defaults.model.clone());
        let effort = request.effort.or_else(|| defaults.effort.clone());
        let account_profile = defaults.account_profile.clone();
        let execution_mode = request.execution_mode.or(defaults.execution_mode);
        let permission_level = request.permission_level.or(defaults.permission_level);
        let create_request = CreateAgentRequest::new(
            &request.session_id,
            request
                .provider
                .unwrap_or_else(|| defaults.provider.clone()),
        )
        .with_owner_user_id(caller_user_id);
        let create_request = create_request;
        let create_request = match caller_metaagent_id.as_deref() {
            Some(metaagent_id) if !request.metaagent => {
                create_request.with_controlled_by_metaagent_id(metaagent_id)
            }
            _ => create_request,
        };
        let create_request = if let Some(alias) = request.alias {
            create_request.with_alias(alias)
        } else {
            create_request
        };
        let create_request = if let Some(model) = model {
            create_request.with_model(model)
        } else {
            create_request
        };
        let create_request = if let Some(effort) = effort {
            create_request.with_effort(effort)
        } else {
            create_request
        };
        let create_request = if let Some(account_profile) = account_profile {
            create_request.with_account_profile(account_profile)
        } else {
            create_request
        };
        let create_request = if let Some(execution_mode) = execution_mode {
            create_request.with_execution_mode_override(execution_mode)
        } else {
            create_request
        };
        let create_request = if let Some(permission_level) = permission_level {
            create_request.with_permission_level_override(permission_level)
        } else {
            create_request
        };
        let requested_worktree_for_scope = request.worktree_id.clone();
        let create_request = if let Some(worktree_id) = request.worktree_id {
            create_request.with_worktree(worktree_id)
        } else {
            create_request
        };
        if request.kernel_ref.is_some() && request.slice_ref.is_some() {
            return self
                .with_session_projection_action_result(Err(DaemonError::LocalTransport {
                    operation: "agent.spawn",
                    message: "use either kernel_ref or slice_ref, not both".to_string(),
                }))
                .await;
        }
        let slice_ref_for_agent = request.slice_ref.clone();
        let slice_kernel_ref = match request.slice_ref {
            Some(slice_ref) => {
                let session = match self.state.session_snapshot(&request.session_id).await {
                    Ok(session) => session,
                    Err(error) => {
                        return self.with_session_projection_action_result(Err(error)).await;
                    }
                };
                let requested_worktree_id = requested_worktree_for_scope
                    .as_deref()
                    .unwrap_or_else(|| session.worktree_id());
                let scope_result = self
                    .state
                    .ensure_slice_worktree_scope(
                        &slice_ref,
                        session.workspace_id(),
                        requested_worktree_id,
                    )
                    .await;
                if let Err(error) = scope_result {
                    return self.with_session_projection_action_result(Err(error)).await;
                }
                match self.state.resolve_slice_worker_kernel_ref(&slice_ref).await {
                    Ok(kernel_ref) => Some(kernel_ref),
                    Err(error) => {
                        return self.with_session_projection_action_result(Err(error)).await;
                    }
                }
            }
            None => None,
        };
        let create_request = if let Some(kernel_ref) = request.kernel_ref {
            create_request.with_kernel(kernel_ref)
        } else if let Some(kernel_ref) = slice_kernel_ref {
            create_request.with_kernel(kernel_ref)
        } else {
            create_request
        };
        let create_request = if let Some(placement) = request.worktree_placement {
            create_request.with_worktree_placement(placement)
        } else {
            create_request
        };
        let result = match self.state.spawn_agent(create_request).await {
            Ok(agent) => {
                let session_id = agent.session_id().to_string();
                if let Some(slice_ref) = slice_ref_for_agent {
                    let agent_id = agent.id().to_string();
                    if let Err(error) = self
                        .state
                        .attach_slice_agent(&slice_ref, &session_id, &agent_id)
                        .await
                    {
                        return self.with_session_projection_action_result(Err(error)).await;
                    }
                }
                if caller_metaagent_id.is_none() && !agent.is_metaagent() {
                    let _ = self
                        .state
                        .inject_metaagent_agent_lifecycle_event_for_agent(
                            &session_id,
                            &agent,
                            "agent.spawned",
                        )
                        .await;
                }
                self.state
                    .session_snapshot(&session_id)
                    .await
                    .map(|_| LocalDaemonResponse::AgentSpawned { agent })
            }
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn spawn_agents(
        &self,
        request: SpawnAgentsRequest,
        caller_user_id: String,
        caller_metaagent_id: Option<String>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        if request.agents.len() > Self::MAX_BATCH_SPAWN_AGENTS {
            return self
                .with_session_projection_action_result(Err(DaemonError::LocalTransport {
                    operation: "agents.spawn",
                    message: format!(
                        "batch spawn is limited to {} agents per request",
                        Self::MAX_BATCH_SPAWN_AGENTS
                    ),
                }))
                .await;
        }
        if request.agents.is_empty() {
            let projection_action = self
                .state
                .session_snapshot(&request.session_id)
                .await
                .ok()
                .map(SessionProjectionAction::Update);
            return (
                Ok(LocalDaemonResponse::AgentsSpawned { agents: Vec::new() }),
                projection_action,
            );
        }
        let session = match self.state.session_snapshot(&request.session_id).await {
            Ok(session) => session,
            Err(error) => return self.with_session_projection_action_result(Err(error)).await,
        };
        let defaults = session.agent_defaults();
        let default_provider = defaults.provider.clone();
        let default_model = defaults.model.clone();
        let default_effort = defaults.effort.clone();
        let default_account_profile = defaults.account_profile.clone();
        let default_execution_mode = defaults.execution_mode;
        let default_permission_level = defaults.permission_level;
        let workspace_id = session.workspace_id().to_string();
        let default_worktree_id = session.worktree_id().to_string();

        let mut slice_kernel_refs = HashMap::<String, String>::new();
        let mut create_requests = Vec::with_capacity(request.agents.len());
        let mut slice_refs_for_agents = Vec::with_capacity(request.agents.len());
        for item in request.agents {
            if item.metaagent {
                return self
                    .with_session_projection_action_result(Err(DaemonError::LocalTransport {
                        operation: "agents.spawn",
                        message: "creating separate metaagents is deprecated; send `/meta <task>` to a regular agent to enter meta mode".to_string(),
                    }))
                    .await;
            }
            if item.kernel_ref.is_some() && item.slice_ref.is_some() {
                return self
                    .with_session_projection_action_result(Err(DaemonError::LocalTransport {
                        operation: "agents.spawn",
                        message: "use either kernel_ref or slice_ref, not both".to_string(),
                    }))
                    .await;
            }

            let model = item.model.or_else(|| default_model.clone());
            let effort = item.effort.or_else(|| default_effort.clone());
            let execution_mode = item.execution_mode.or(default_execution_mode);
            let permission_level = item.permission_level.or(default_permission_level);
            let requested_worktree_for_scope = item.worktree_id.clone();
            let slice_ref_for_agent = item.slice_ref.clone();
            let slice_kernel_ref = match item.slice_ref {
                Some(slice_ref) => {
                    let requested_worktree_id = requested_worktree_for_scope
                        .as_deref()
                        .unwrap_or(default_worktree_id.as_str());
                    if let Err(error) = self
                        .state
                        .ensure_slice_worktree_scope(
                            &slice_ref,
                            &workspace_id,
                            requested_worktree_id,
                        )
                        .await
                    {
                        return self.with_session_projection_action_result(Err(error)).await;
                    }
                    match slice_kernel_refs.get(&slice_ref) {
                        Some(kernel_ref) => Some(kernel_ref.clone()),
                        None => {
                            match self.state.resolve_slice_worker_kernel_ref(&slice_ref).await {
                                Ok(kernel_ref) => {
                                    slice_kernel_refs.insert(slice_ref, kernel_ref.clone());
                                    Some(kernel_ref)
                                }
                                Err(error) => {
                                    return self
                                        .with_session_projection_action_result(Err(error))
                                        .await;
                                }
                            }
                        }
                    }
                }
                None => None,
            };

            let mut create_request = CreateAgentRequest::new(
                &request.session_id,
                item.provider.unwrap_or_else(|| default_provider.clone()),
            )
            .with_owner_user_id(caller_user_id.clone());
            if let Some(metaagent_id) = caller_metaagent_id.as_deref() {
                create_request = create_request.with_controlled_by_metaagent_id(metaagent_id);
            }
            if let Some(alias) = item.alias {
                create_request = create_request.with_alias(alias);
            }
            if let Some(model) = model {
                create_request = create_request.with_model(model);
            }
            if let Some(effort) = effort {
                create_request = create_request.with_effort(effort);
            }
            if let Some(account_profile) = default_account_profile.clone() {
                create_request = create_request.with_account_profile(account_profile);
            }
            if let Some(execution_mode) = execution_mode {
                create_request = create_request.with_execution_mode_override(execution_mode);
            }
            if let Some(permission_level) = permission_level {
                create_request = create_request.with_permission_level_override(permission_level);
            }
            if let Some(worktree_id) = item.worktree_id {
                create_request = create_request.with_worktree(worktree_id);
            }
            if let Some(kernel_ref) = item.kernel_ref.or(slice_kernel_ref) {
                create_request = create_request.with_kernel(kernel_ref);
            }
            if let Some(placement) = item.worktree_placement {
                create_request = create_request.with_worktree_placement(placement);
            }
            create_requests.push(create_request);
            slice_refs_for_agents.push(slice_ref_for_agent);
        }
        let agents = match self
            .state
            .spawn_agents(create_requests, &caller_user_id)
            .await
        {
            Ok(agents) => agents,
            Err(error) => return self.with_session_projection_action_result(Err(error)).await,
        };
        for (agent, slice_ref) in agents.iter().zip(slice_refs_for_agents.iter()) {
            if let Some(slice_ref) = slice_ref {
                if let Err(error) = self
                    .state
                    .attach_slice_agent(slice_ref, agent.session_id(), agent.id())
                    .await
                {
                    return self.with_session_projection_action_result(Err(error)).await;
                }
            }
            if caller_metaagent_id.is_none() && !agent.is_metaagent() {
                let _ = self
                    .state
                    .inject_metaagent_agent_lifecycle_event_for_agent(
                        agent.session_id(),
                        agent,
                        "agent.spawned",
                    )
                    .await;
            }
        }
        self.with_session_projection_action_result(Ok(LocalDaemonResponse::AgentsSpawned {
            agents,
        }))
        .await
    }

    pub(in crate::runtime::session_actor) async fn undo_turn(
        &self,
        request: UndoTurnRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .undo_turn(request, &caller_user_id)
            .await
            .map(|result| LocalDaemonResponse::TurnUndone { result });
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn fork_agent(
        &self,
        request: ForkAgentRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self.state.fork_agent(request, caller_user_id).await.map(
            |(source_agent_id, agent, provider_run, session)| LocalDaemonResponse::AgentForked {
                source_agent_id,
                agent,
                provider_run,
                session,
            },
        );
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn destroy_agent(
        &self,
        request: DestroyAgentRequest,
        caller_user_id: String,
        caller_is_metaagent: bool,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = match self
            .state
            .destroy_agent(&request.agent_id, &caller_user_id)
            .await
        {
            Ok(agent) => {
                let session_id = agent.session_id().to_string();
                if !caller_is_metaagent && !agent.is_metaagent() {
                    let _ = self
                        .state
                        .inject_metaagent_agent_lifecycle_event_for_agent(
                            &session_id,
                            &agent,
                            "agent.deleted",
                        )
                        .await;
                }
                self.state
                    .session_snapshot(&session_id)
                    .await
                    .map(|_| LocalDaemonResponse::AgentDestroyed { agent })
            }
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }
}
