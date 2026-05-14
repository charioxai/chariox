use crate::agent::CreateAgentRequest;
use crate::error::DaemonError;
use crate::local::{
    AliasAgentRequest, AliasSessionRequest, AttachToSessionRequest, CycleAgentFocusRequest,
    DeleteSessionRequest, DestroyAgentRequest, DetachFromSessionRequest, EndSessionRequest,
    FocusAgentRequest, LocalDaemonResponse, PollRuntimeNoticesRequest, ResizeTerminalRequest,
    RespondToInteractionRequest, SendTerminalInputRequest, SpawnAgentRequest,
    UpdateAgentConfigRequest, UpdateAgentProfileRequest, UpdateAgentSubstitutesRequest,
    UpdateSessionConfigRequest,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::CreateSessionRequest;

use super::projection_policy::{
    session_id_for_projection_refresh, session_response_projection_action, SessionProjectionAction,
};

#[derive(Clone)]
pub(crate) struct SessionRuntimeStore {
    state: KernelRuntimeState,
}

impl SessionRuntimeStore {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self { state }
    }

    pub(super) async fn resolve_session_ref_id(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        self.state
            .resolve_session_ref_id(session_ref, workspace_id)
            .await
    }

    pub(super) async fn attachment_session_id(
        &self,
        attachment_id: &str,
    ) -> Result<String, DaemonError> {
        self.state.attachment_session_id(attachment_id).await
    }

    async fn with_session_projection_action_result(
        &self,
        result: Result<LocalDaemonResponse, DaemonError>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let projection_action = if let Ok(response) = result.as_ref() {
            if let Some(action) = session_response_projection_action(response) {
                Some(action)
            } else if let Some(session_id) = session_id_for_projection_refresh(&result) {
                self.state
                    .session_snapshot(&session_id)
                    .await
                    .ok()
                    .map(SessionProjectionAction::Update)
            } else {
                None
            }
        } else {
            None
        };
        (result, projection_action)
    }

    pub(super) async fn create_session(
        &self,
        request: CreateSessionRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .create_session_response(request.with_owner_user_id(caller_user_id))
            .await;
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn attach_to_session(
        &self,
        request: AttachToSessionRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let attach_request = crate::attachment::AttachRequest::new(
            request.session_id,
            request.client_id,
            request.capability_level,
        );
        let result = self
            .state
            .attach(attach_request)
            .await
            .map(|attachment| LocalDaemonResponse::SessionAttached { attachment });
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn detach_from_session(
        &self,
        request: DetachFromSessionRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .detach(&request.attachment_id)
            .await
            .map(|attachment| LocalDaemonResponse::SessionDetached { attachment });
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn focus_agent(
        &self,
        request: FocusAgentRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .focus_agent(&request.session_id, &request.agent_id, &caller_user_id)
            .await
            .map(|agent| LocalDaemonResponse::AgentFocused { agent });
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn cycle_agent_focus(
        &self,
        request: CycleAgentFocusRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .cycle_agent_focus(&request.session_id, &caller_user_id)
            .await
            .map(|agent| LocalDaemonResponse::AgentFocusCycled { agent });
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn resize_terminal(
        &self,
        request: ResizeTerminalRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .resize_terminal(&request.session_id, request.cols, request.rows)
            .await
            .map(|()| LocalDaemonResponse::TerminalResized {
                session_id: request.session_id,
                cols: request.cols,
                rows: request.rows,
            });
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn send_terminal_input(
        &self,
        request: SendTerminalInputRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .send_terminal_input(
                &request.session_id,
                &request.attachment_id,
                request.provider_run_id.as_deref(),
                &request.data_base64,
            )
            .await
            .map(|byte_count| LocalDaemonResponse::TerminalInputSent {
                session_id: request.session_id,
                attachment_id: request.attachment_id,
                byte_count,
            });
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn poll_runtime_notices(
        &self,
        request: PollRuntimeNoticesRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = match self
            .state
            .ensure_attachment_in_session(&request.session_id, &request.attachment_id)
            .await
        {
            Ok(()) => Ok(LocalDaemonResponse::RuntimeNotices {
                notices: self
                    .state
                    .drain_notice_records(&request.session_id, &request.attachment_id)
                    .await,
            }),
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn update_session_config(
        &self,
        request: UpdateSessionConfigRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let session_id = request.session_id.clone();
        let result = match self
            .state
            .update_session_config(
                &request.session_id,
                &request.attachment_id,
                request.values,
                request.requires_idle,
            )
            .await
        {
            Ok(config) => self
                .state
                .session_snapshot(&session_id)
                .await
                .map(|session| LocalDaemonResponse::SessionConfigUpdated { config, session }),
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn update_agent_config(
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

    pub(super) async fn update_agent_profile(
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

    pub(super) async fn alias_agent(
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

    pub(super) async fn update_agent_substitutes(
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

    pub(super) async fn respond_to_interaction(
        &self,
        request: RespondToInteractionRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let session_id = request.session_id.clone();
        let interaction_id = request.interaction_id.clone();
        let result = match self
            .state
            .resolve_runtime_interaction(
                &request.session_id,
                &request.interaction_id,
                &request.choice_id,
                request.custom_reply.as_deref(),
            )
            .await
        {
            Ok(()) => self
                .state
                .session_snapshot(&session_id)
                .await
                .map(|session| LocalDaemonResponse::InteractionResponded {
                    interaction_id,
                    session,
                }),
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn alias_session(
        &self,
        request: AliasSessionRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .alias_session(&request.session_id, request.alias)
            .await
            .map(|session| LocalDaemonResponse::SessionAliased { session });
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn spawn_agent(
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
        let slice_kernel_ref = match request.slice_ref {
            Some(slice_ref) => match self.state.resolve_slice_worker_kernel_ref(&slice_ref).await {
                Ok(kernel_ref) => Some(kernel_ref),
                Err(error) => return self.with_session_projection_action_result(Err(error)).await,
            },
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
                self.state
                    .session_snapshot(&session_id)
                    .await
                    .map(|_| LocalDaemonResponse::AgentSpawned { agent })
            }
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn destroy_agent(
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

    pub(super) async fn end_session(
        &self,
        request: EndSessionRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .end_session(&request.session_id)
            .await
            .map(|session| LocalDaemonResponse::SessionEnded { session });
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn delete_session(
        &self,
        request: DeleteSessionRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .delete_session_ref(&request.session_ref, request.workspace_id.as_deref())
            .await
            .map(|session| LocalDaemonResponse::SessionDeleted { session });
        self.with_session_projection_action_result(result).await
    }
}
