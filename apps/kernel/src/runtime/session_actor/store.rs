use crate::error::DaemonError;
use crate::local::{
    AliasSessionRequest, AttachToSessionRequest, CycleAgentFocusRequest, DeleteSessionRequest,
    DetachFromSessionRequest, EndSessionRequest, FocusAgentRequest, LocalDaemonResponse,
    PollRuntimeNoticesRequest, ResizeTerminalRequest, RespondToInteractionRequest,
    SendTerminalInputRequest, UpdateSessionConfigRequest,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::CreateSessionRequest;

use super::projection_policy::{
    session_id_for_projection_refresh, session_response_projection_action, SessionProjectionAction,
};

mod agent;

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
