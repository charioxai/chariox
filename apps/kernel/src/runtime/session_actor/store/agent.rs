//! Agent-specific session-store request adapters.

use crate::agent::AgentRole;
use crate::agent::CreateAgentRequest;
use crate::error::DaemonError;
use crate::local::{
    AliasAgentRequest, DestroyAgentRequest, LocalDaemonResponse, SpawnAgentRequest,
    UpdateAgentConfigRequest, UpdateAgentProfileRequest, UpdateAgentSubstitutesRequest,
};

use super::super::projection_policy::SessionProjectionAction;
use super::SessionRuntimeStore;

impl SessionRuntimeStore {
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
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let session = match self.state.session_snapshot(&request.session_id).await {
            Ok(session) => session,
            Err(error) => return self.with_session_projection_action_result(Err(error)).await,
        };
        let defaults = session.agent_defaults();
        let model = request.model.or_else(|| defaults.model.clone());
        let effort = request.effort.or_else(|| defaults.effort.clone());
        let execution_mode = request.execution_mode.or(defaults.execution_mode);
        let permission_level = request.permission_level.or(defaults.permission_level);
        let create_request = CreateAgentRequest::new(
            &request.session_id,
            request
                .provider
                .unwrap_or_else(|| defaults.provider.clone()),
        )
        .with_owner_user_id(caller_user_id);
        let create_request = if request.metaagent {
            create_request.with_role(AgentRole::Meta)
        } else {
            create_request
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
        if request.metaagent && request.slice_ref.is_some() {
            return self
                .with_session_projection_action_result(Err(DaemonError::LocalTransport {
                    operation: "agent.spawn",
                    message: "metaagents cannot be launched in slices".to_string(),
                }))
                .await;
        }
        let slice_ref_for_agent = request.slice_ref.clone();
        let slice_kernel_ref = match request.slice_ref {
            Some(slice_ref) => {
                let session = match self.state.session_snapshot(&request.session_id).await {
                    Ok(session) => session,
                    Err(error) => {
                        return self.with_session_projection_action_result(Err(error)).await
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
                self.state
                    .session_snapshot(&session_id)
                    .await
                    .map(|_| LocalDaemonResponse::AgentSpawned { agent })
            }
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn destroy_agent(
        &self,
        request: DestroyAgentRequest,
        caller_user_id: String,
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
