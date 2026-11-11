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
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};

/// A child never has more authority than its parent: a level or mode the spawn does not ask
/// for is the parent's current limit, and one it asks for cannot be above it. A spawn from a
/// person (no parent) keeps the requested values, filled from the room defaults.
fn child_authority(
    parent: Option<crate::session::EffectiveAgentExecutionConfig>,
    mode: Option<AgentExecutionMode>,
    level: Option<AgentPermissionLevel>,
    defaults: &crate::session::SessionAgentDefaults,
) -> Result<(Option<AgentExecutionMode>, Option<AgentPermissionLevel>), DaemonError> {
    let Some(limit) = parent else {
        return Ok((
            mode.or(defaults.execution_mode),
            level.or(defaults.permission_level),
        ));
    };
    let (parent_mode, parent_level) = (limit.mode, limit.permission_level);
    let child_mode = mode.unwrap_or(parent_mode);
    let child_level = level.unwrap_or(parent_level);
    if !parent_mode.permits(child_mode) || !parent_level.permits(child_level) {
        return Err(DaemonError::LocalTransport {
            operation: "agent.spawn",
            message: format!(
                "a sub-agent cannot have more authority than its parent: the parent runs {parent_level:?} in {parent_mode} mode; spawn with a permission and mode at or below it"
            ),
        });
    }
    Ok((Some(child_mode), Some(child_level)))
}

fn parent_limit(
    session: &crate::session::RuntimeSession,
    parent: Option<&crate::agent::AgentInstance>,
    agents: &[crate::agent::AgentInstance],
) -> Option<crate::session::EffectiveAgentExecutionConfig> {
    parent.map(|parent| {
        crate::session::effective_agent_execution_config_in_tree(session, parent, agents)
    })
}

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
                request.account_profile,
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
            Ok(agent) => match self.reconcile_room_environment_actors_if_started(&session_id) {
                Ok(()) => self
                    .state
                    .session_snapshot(&session_id)
                    .await
                    .map(|session| LocalDaemonResponse::AgentAliased { agent, session }),
                Err(error) => Err(error),
            },
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
        idempotency_key: Option<String>,
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
        let slice_admission = match self
            .state
            .guard_slice_execution(
                Some(&request.session_id),
                [(request.slice_ref.as_deref(), request.kernel_ref.as_deref())],
                "agent.spawn",
            )
            .await
        {
            Ok(guards) => guards,
            Err(error) => return self.with_session_projection_action_result(Err(error)).await,
        };
        let defaults = session.agent_defaults();
        let same_provider = request
            .provider
            .as_deref()
            .is_none_or(|provider| provider == defaults.provider);
        let model = request
            .model
            .or_else(|| same_provider.then(|| defaults.model.clone()).flatten());
        let effort = request
            .effort
            .or_else(|| same_provider.then(|| defaults.effort.clone()).flatten());
        let account_profile = request.account_profile.or_else(|| {
            same_provider
                .then(|| defaults.account_profile.clone())
                .flatten()
        });
        let parent = caller_metaagent_id.as_deref().and_then(|id| {
            self.state
                .session_agents(&request.session_id)
                .into_iter()
                .find(|agent| agent.id() == id)
        });
        // A parent's command id, or a peer spawn's request from another kernel, names one agent.
        if let Some(key) = idempotency_key.as_deref() {
            let parent_id = parent.as_ref().map(|parent| parent.id());
            let existing = self
                .state
                .session_agents(&request.session_id)
                .into_iter()
                .find(|agent| {
                    agent.parent_agent_id() == parent_id && agent.spawn_key() == Some(key)
                });
            if let Some(agent) = existing {
                return self
                    .with_session_projection_action_result(Ok(LocalDaemonResponse::AgentSpawned {
                        agent,
                        event_subscription_error: None,
                        deadline_ms: None,
                        branch: None,
                    }))
                    .await;
            }
        }
        let peer_limit = match caller_metaagent_id
            .as_deref()
            .map(|caller| self.state.peer_spawn_limit(caller, &request.session_id))
            .transpose()
        {
            Ok(limit) => limit.flatten(),
            Err(error) => return self.with_session_projection_action_result(Err(error)).await,
        };
        let is_peer = peer_limit.is_some();
        let (execution_mode, permission_level) = match child_authority(
            peer_limit.or_else(|| {
                parent_limit(
                    &session,
                    parent.as_ref(),
                    &self.state.session_agents(&request.session_id),
                )
            }),
            request.execution_mode,
            request.permission_level,
            defaults,
        ) {
            Ok(authority) => authority,
            Err(error) => return self.with_session_projection_action_result(Err(error)).await,
        };
        let spawn_provider = request
            .provider
            .clone()
            .unwrap_or_else(|| defaults.provider.clone());
        let owner_user_id = self
            .state
            .room_member_user_id(&request.session_id, &caller_user_id);
        let (model, effort) = if request.kernel_ref.is_none()
            && slice_admission.slice_ids.iter().all(Option::is_none)
        {
            match self
                .state
                .resolve_new_agent_selection(
                    &owner_user_id,
                    &spawn_provider,
                    account_profile.as_deref(),
                    model,
                    effort,
                )
                .await
            {
                Ok(selection) => selection,
                Err(error) => return self.with_session_projection_action_result(Err(error)).await,
            }
        } else {
            (model, effort)
        };
        let create_request = CreateAgentRequest::new(&request.session_id, spawn_provider)
            .with_owner_user_id(owner_user_id);
        let create_request = match caller_metaagent_id.as_deref() {
            Some(metaagent_id) if !request.metaagent && !is_peer => {
                create_request.with_parent_agent_id(metaagent_id)
            }
            _ => create_request,
        };
        let create_request = match idempotency_key {
            Some(key) => create_request.with_spawn_key(key),
            None => create_request,
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
        let slice_ref_for_agent = match slice_admission.slice_ids.as_slice() {
            [slice_ref] => slice_ref.clone(),
            _ => {
                return self
                    .with_session_projection_action_result(Err(DaemonError::InternalInvariant {
                        operation: "agent.spawn",
                        message: "slice admission target count mismatch".to_string(),
                    }))
                    .await;
            }
        };
        let slice_kernel_ref = match slice_ref_for_agent.as_deref() {
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
                        slice_ref,
                        session.workspace_id(),
                        requested_worktree_id,
                    )
                    .await;
                if let Err(error) = scope_result {
                    return self.with_session_projection_action_result(Err(error)).await;
                }
                match self.state.resolve_slice_worker_kernel_ref(slice_ref).await {
                    Ok(kernel_ref) => Some(kernel_ref),
                    Err(error) => {
                        return self.with_session_projection_action_result(Err(error)).await;
                    }
                }
            }
            None => None,
        };
        let create_request = if let Some(kernel_ref) = slice_kernel_ref.or(request.kernel_ref) {
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
                // A peer lives and dies with the room agent that made it.
                if let (true, Some(creator)) = (is_peer, caller_metaagent_id.as_deref()) {
                    if let Err(error) = self.state.record_created_local_peer(creator, &agent) {
                        return self.with_session_projection_action_result(Err(error)).await;
                    }
                }
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
                // A person's new agent becomes the focus and gets the session's
                // foreground App, like any focus change. An agent-initiated
                // spawn does not: it would bind without authorization.
                #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
                if caller_metaagent_id.is_none() {
                    self.state.bind_foreground_app(&session_id).await;
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
                match self.reconcile_room_environment_actors_if_started(&session_id) {
                    Ok(()) => self.state.session_snapshot(&session_id).await.map(|_| {
                        LocalDaemonResponse::AgentSpawned {
                            agent,
                            event_subscription_error: None,
                            deadline_ms: None,
                            branch: None,
                        }
                    }),
                    Err(error) => Err(error),
                }
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
        if caller_metaagent_id.is_none() && request.agents.len() > Self::MAX_BATCH_SPAWN_AGENTS {
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
        // New agents belong to the room member the caller is.
        let member_user_id = self
            .state
            .room_member_user_id(&request.session_id, &caller_user_id);
        if request.agents.is_empty() {
            let projection_action = self
                .state
                .session_snapshot(&request.session_id)
                .await
                .ok()
                .map(SessionProjectionAction::Update);
            return (
                Ok(LocalDaemonResponse::AgentsSpawned {
                    agents: Vec::new(),
                    event_subscription_error: None,
                }),
                projection_action,
            );
        }
        let session = match self.state.session_snapshot(&request.session_id).await {
            Ok(session) => session,
            Err(error) => return self.with_session_projection_action_result(Err(error)).await,
        };
        let slice_admission = match self
            .state
            .guard_slice_execution(
                Some(&request.session_id),
                request
                    .agents
                    .iter()
                    .map(|item| (item.slice_ref.as_deref(), item.kernel_ref.as_deref())),
                "agents.spawn",
            )
            .await
        {
            Ok(guards) => guards,
            Err(error) => return self.with_session_projection_action_result(Err(error)).await,
        };
        let defaults = session.agent_defaults();
        let default_provider = defaults.provider.clone();
        let default_model = defaults.model.clone();
        let default_effort = defaults.effort.clone();
        let default_account_profile = defaults.account_profile.clone();
        let room_agents = self.state.session_agents(&request.session_id);
        let parent = caller_metaagent_id
            .as_deref()
            .and_then(|id| room_agents.iter().find(|agent| agent.id() == id).cloned());
        let workspace_id = session.workspace_id().to_string();
        let default_worktree_id = session.worktree_id().to_string();

        if slice_admission.slice_ids.len() != request.agents.len() {
            return self
                .with_session_projection_action_result(Err(DaemonError::InternalInvariant {
                    operation: "agents.spawn",
                    message: "slice admission target count mismatch".to_string(),
                }))
                .await;
        }
        let mut slice_kernel_refs = HashMap::<String, String>::new();
        let mut create_requests = Vec::with_capacity(request.agents.len());
        let mut slice_refs_for_agents = Vec::with_capacity(request.agents.len());
        for (item, slice_ref_for_agent) in
            request.agents.into_iter().zip(&slice_admission.slice_ids)
        {
            if item.metaagent {
                return self
                    .with_session_projection_action_result(Err(DaemonError::LocalTransport {
                        operation: "agents.spawn",
                        message: "creating separate metaagents is deprecated; send `/meta <task>` to a regular agent to enter meta mode".to_string(),
                    }))
                    .await;
            }
            let same_provider = item
                .provider
                .as_deref()
                .is_none_or(|provider| provider == default_provider);
            let model = item
                .model
                .or_else(|| same_provider.then(|| default_model.clone()).flatten());
            let effort = item
                .effort
                .or_else(|| same_provider.then(|| default_effort.clone()).flatten());
            let (execution_mode, permission_level) = match child_authority(
                parent_limit(&session, parent.as_ref(), &room_agents),
                item.execution_mode,
                item.permission_level,
                defaults,
            ) {
                Ok(authority) => authority,
                Err(error) => return self.with_session_projection_action_result(Err(error)).await,
            };
            let requested_worktree_for_scope = item.worktree_id.clone();
            let slice_ref_for_agent = slice_ref_for_agent.clone();
            let slice_kernel_ref = match slice_ref_for_agent.clone() {
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

            let item_provider = item
                .provider
                .clone()
                .unwrap_or_else(|| default_provider.clone());
            let item_account_profile = item.account_profile.clone().or_else(|| {
                same_provider
                    .then(|| default_account_profile.clone())
                    .flatten()
            });
            let (model, effort) = if slice_kernel_ref.is_none() && item.kernel_ref.is_none() {
                match self
                    .state
                    .resolve_new_agent_selection(
                        &member_user_id,
                        &item_provider,
                        item_account_profile.as_deref(),
                        model,
                        effort,
                    )
                    .await
                {
                    Ok(selection) => selection,
                    Err(error) => {
                        return self.with_session_projection_action_result(Err(error)).await
                    }
                }
            } else {
                (model, effort)
            };
            let mut create_request = CreateAgentRequest::new(&request.session_id, item_provider)
                .with_owner_user_id(member_user_id.clone());
            if let Some(metaagent_id) = caller_metaagent_id.as_deref() {
                create_request = create_request.with_parent_agent_id(metaagent_id);
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
            if let Some(account_profile) = item_account_profile {
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
            if let Some(kernel_ref) = slice_kernel_ref.or(item.kernel_ref) {
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
            .spawn_agents(create_requests, &caller_user_id, &slice_refs_for_agents)
            .await
        {
            Ok(agents) => agents,
            Err(error) => {
                // A worker-backed failure may have created and then rolled back
                // agents. Publish the recovered state even though the request failed.
                let projection = self
                    .state
                    .session_snapshot(&request.session_id)
                    .await
                    .ok()
                    .map(SessionProjectionAction::Update);
                return (Err(error), projection);
            }
        };
        let slice_attachments = agents
            .iter()
            .zip(slice_refs_for_agents.iter())
            .filter_map(|(agent, slice_ref)| {
                slice_ref
                    .as_ref()
                    .map(|slice_ref| crate::slice::SliceAgentAttachment {
                        slice_ref: slice_ref.clone(),
                        session_id: agent.session_id().to_string(),
                        agent_id: agent.id().to_string(),
                    })
            })
            .collect::<Vec<_>>();
        if let Err(error) = self.state.attach_slice_agents(slice_attachments).await {
            return self.with_session_projection_action_result(Err(error)).await;
        }
        // Only the last (focus) agent of a person's batch gets the foreground App.
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if caller_metaagent_id.is_none() {
            self.state.bind_foreground_app(&request.session_id).await;
        }
        if caller_metaagent_id.is_none() {
            let _ = self
                .state
                .inject_metaagent_agent_lifecycle_event_for_agents(
                    &request.session_id,
                    &agents,
                    "agents.spawned",
                )
                .await;
        }
        if let Err(error) = self.reconcile_room_environment_actors_if_started(&request.session_id) {
            return self.with_session_projection_action_result(Err(error)).await;
        }
        self.with_session_projection_action_result(Ok(LocalDaemonResponse::AgentsSpawned {
            agents,
            event_subscription_error: None,
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
        let result = match self.state.fork_agent(request, caller_user_id).await {
            Ok((source_agent_id, agent, provider_run, session)) => {
                match self.reconcile_room_environment_actors_if_started(session.id()) {
                    Ok(()) => Ok(LocalDaemonResponse::AgentForked {
                        source_agent_id,
                        agent,
                        provider_run: provider_run.into(),
                        session,
                    }),
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn hide_agent(
        &self,
        request: crate::local::HideAgentRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = match self.state.hide_agent_in_session(&request, &caller_user_id) {
            Ok(agent) => self
                .state
                .session_snapshot(&request.session_id)
                .await
                .map(|session| LocalDaemonResponse::AgentHidden { session, agent }),
            Err(error) => Err(error),
        };
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
            .destroy_agent_in_session(&request.session_id, &request.agent_id, &caller_user_id)
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
                match self.reconcile_room_environment_actors_if_started(&session_id) {
                    Ok(()) => self
                        .state
                        .session_snapshot(&session_id)
                        .await
                        .map(|_| LocalDaemonResponse::AgentDestroyed { agent }),
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }
}
