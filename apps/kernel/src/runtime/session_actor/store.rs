use crate::error::DaemonError;
use crate::local::{
    AcknowledgeAgentOutputSeenRequest, AliasSessionRequest, ArchiveProjectRequest,
    AttachToSessionRequest, CancelRoomEnvironmentActionRequest, CycleAgentFocusRequest,
    DeleteProjectRequest, DeleteSessionRequest, DetachFromSessionRequest, EndSessionRequest,
    FocusAgentRequest, ListProjectsRequest, LocalDaemonResponse,
    ReleaseRoomEnvironmentInputRequest, RenameProjectRequest,
    RequestRoomEnvironmentInputTakeoverRequest, RespondToInteractionRequest, RestoreProjectRequest,
    RetryRoomEnvironmentRequest, StartRoomEnvironmentRequest, StopRoomEnvironmentRequest,
    UpdateRoomEnvironmentViewportRequest, UpdateSessionConfigRequest,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::CreateSessionRequest;

use super::projection_policy::{
    session_id_for_projection_refresh, session_response_projection_action, SessionProjectionAction,
};

mod agent;
mod terminal;

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

    pub(super) async fn start_room_environment(
        &self,
        request: StartRoomEnvironmentRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let viewport = match self.state.room_environment_snapshot(&request.session_id) {
            Ok(environment) => Ok(environment.viewport),
            Err(crate::session::EnvironmentError::EnvironmentNotFound { .. }) => {
                crate::session::CanonicalViewport::new(
                    request.viewport.css_width,
                    request.viewport.css_height,
                    request.viewport.device_scale_factor,
                    request.viewport.desktop_pixel_width,
                    request.viewport.desktop_pixel_height,
                )
            }
            Err(error) => Err(error),
        }
        .map_err(|error| room_environment_control_error("environment.start", error));
        let result = match viewport {
            Ok(viewport) => self
                .state
                .start_room_environment(&request.session_id, viewport)
                .map_err(|error| room_environment_control_error("environment.start", error)),
            Err(error) => Err(error),
        };
        let result = match result {
            Ok(_) => self
                .finish_room_environment_controller_start(&request.session_id, "environment.start")
                .await
                .and_then(|_| {
                    self.state
                        .reconcile_room_environment_actors(
                            &request.session_id,
                            Some(&caller_user_id),
                        )
                        .map_err(|error| room_environment_control_error("environment.start", error))
                })
                .map(|environment| LocalDaemonResponse::RoomEnvironmentUpdated { environment }),
            Err(error) => Err(error),
        };
        (result, None)
    }

    pub(super) async fn stop_room_environment(
        &self,
        request: StopRoomEnvironmentRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = if self.state.browser_controller_process_enabled() {
            self.state
                .stop_managed_room_environment_runtime(&request.session_id)
                .await
        } else {
            self.state
                .stop_room_environment(&request.session_id)
                .map_err(|error| room_environment_control_error("environment.stop", error))
        }
        .map(|environment| LocalDaemonResponse::RoomEnvironmentUpdated { environment });
        (result, None)
    }

    pub(super) async fn retry_room_environment(
        &self,
        request: RetryRoomEnvironmentRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .retry_room_environment(&request.session_id)
            .map_err(|error| room_environment_control_error("environment.retry", error));
        let result = match result {
            Ok(_) => self
                .finish_room_environment_controller_start(&request.session_id, "environment.retry")
                .await
                .map(|environment| LocalDaemonResponse::RoomEnvironmentUpdated { environment }),
            Err(error) => Err(error),
        };
        (result, None)
    }

    async fn finish_room_environment_controller_start(
        &self,
        session_id: &str,
        operation: &'static str,
    ) -> Result<crate::session::RoomEnvironmentSnapshot, DaemonError> {
        if !self.state.browser_controller_process_enabled() {
            return self
                .state
                .room_environment_snapshot(session_id)
                .map_err(|error| room_environment_control_error(operation, error));
        }
        self.state
            .update_room_environment_component_health(
                session_id,
                crate::session::EnvironmentComponent::BrowserController,
                crate::session::EnvironmentComponentHealthState::Starting,
                None,
            )
            .map_err(|error| room_environment_control_error(operation, error))?;
        if let Err(error) = self
            .state
            .ensure_browser_controller_process_started(session_id)
            .await
        {
            let _ = self.state.update_room_environment_component_health(
                session_id,
                crate::session::EnvironmentComponent::BrowserController,
                crate::session::EnvironmentComponentHealthState::Unavailable,
                Some("controller_start_failed"),
            );
            let _ = self.state.transition_room_environment(
                session_id,
                crate::session::EnvironmentLifecycle::Failed,
            );
            return Err(error);
        }
        self.state
            .update_room_environment_component_health(
                session_id,
                crate::session::EnvironmentComponent::BrowserController,
                crate::session::EnvironmentComponentHealthState::Ready,
                None,
            )
            .map_err(|error| room_environment_control_error(operation, error))?;
        self.state
            .update_room_environment_component_health(
                session_id,
                crate::session::EnvironmentComponent::Browser,
                crate::session::EnvironmentComponentHealthState::Starting,
                None,
            )
            .map_err(|error| room_environment_control_error(operation, error))?;
        match self
            .state
            .reconcile_browser_controller_environment(session_id)
            .await
        {
            Ok(_) => self
                .state
                .update_room_environment_component_health(
                    session_id,
                    crate::session::EnvironmentComponent::Browser,
                    crate::session::EnvironmentComponentHealthState::Ready,
                    None,
                )
                .map_err(|error| room_environment_control_error(operation, error)),
            Err(error) => {
                let _ = self.state.update_room_environment_component_health(
                    session_id,
                    crate::session::EnvironmentComponent::Browser,
                    crate::session::EnvironmentComponentHealthState::Unavailable,
                    Some("browser_reconcile_failed"),
                );
                let _ = self.state.transition_room_environment(
                    session_id,
                    crate::session::EnvironmentLifecycle::Failed,
                );
                Err(error)
            }
        }
    }

    pub(super) async fn update_room_environment_viewport(
        &self,
        request: UpdateRoomEnvironmentViewportRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let viewport = crate::session::CanonicalViewport::new(
            request.viewport.css_width,
            request.viewport.css_height,
            request.viewport.device_scale_factor,
            request.viewport.desktop_pixel_width,
            request.viewport.desktop_pixel_height,
        )
        .map_err(|error| room_environment_control_error("environment.viewport.update", error));
        let result = viewport.and_then(|viewport| {
            let actor_id = crate::session::human_environment_actor_id(&caller_user_id);
            let display_label = crate::session::human_environment_actor_label(&caller_user_id);
            self.state
                .update_room_environment_viewport_as_actor(
                    &request.session_id,
                    crate::session::EnvironmentActor::new(
                        actor_id,
                        crate::session::EnvironmentActorKind::Human,
                        display_label,
                    ),
                    request.expected_revision,
                    viewport,
                )
                .map_err(|error| {
                    room_environment_control_error("environment.viewport.update", error)
                })
        });
        let result = match result {
            Ok(environment) if self.state.browser_controller_process_enabled() => {
                match self
                    .state
                    .reconcile_browser_controller_environment(&request.session_id)
                    .await
                {
                    Ok(_) => self
                        .state
                        .update_room_environment_component_health(
                            &request.session_id,
                            crate::session::EnvironmentComponent::Browser,
                            crate::session::EnvironmentComponentHealthState::Ready,
                            None,
                        )
                        .map_err(|error| {
                            room_environment_control_error("environment.viewport.update", error)
                        }),
                    Err(_) => {
                        let degraded = self
                            .state
                            .update_room_environment_component_health(
                                &request.session_id,
                                crate::session::EnvironmentComponent::Browser,
                                crate::session::EnvironmentComponentHealthState::Degraded,
                                Some("viewport_apply_failed"),
                            )
                            .unwrap_or(environment);
                        if degraded.lifecycle == crate::session::EnvironmentLifecycle::Ready {
                            self.state
                                .transition_room_environment(
                                    &request.session_id,
                                    crate::session::EnvironmentLifecycle::Degraded,
                                )
                                .map_err(|error| {
                                    room_environment_control_error(
                                        "environment.viewport.update",
                                        error,
                                    )
                                })
                        } else {
                            Ok(degraded)
                        }
                    }
                }
            }
            other => other,
        }
        .map(|environment| LocalDaemonResponse::RoomEnvironmentUpdated { environment });
        (result, None)
    }

    pub(super) async fn request_room_environment_input_takeover(
        &self,
        request: RequestRoomEnvironmentInputTakeoverRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let actor = crate::session::EnvironmentActor::new(
            crate::session::human_environment_actor_id(&caller_user_id),
            crate::session::EnvironmentActorKind::Human,
            crate::session::human_environment_actor_label(&caller_user_id),
        );
        let result = self
            .state
            .request_room_environment_takeover_as_actor(&request.session_id, actor, request.target)
            .map(
                |(outcome, environment)| LocalDaemonResponse::RoomEnvironmentTakeoverUpdated {
                    outcome,
                    environment,
                },
            )
            .map_err(|error| room_environment_control_error("environment.input.takeover", error));
        (result, None)
    }

    pub(super) async fn release_room_environment_input(
        &self,
        request: ReleaseRoomEnvironmentInputRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let actor_id = crate::session::human_environment_actor_id(&caller_user_id);
        let result = self
            .state
            .release_room_environment_input(&request.session_id, &actor_id, &request.target)
            .map(|environment| LocalDaemonResponse::RoomEnvironmentInputReleased { environment })
            .map_err(|error| room_environment_control_error("environment.input.release", error));
        (result, None)
    }

    pub(super) async fn cancel_room_environment_action(
        &self,
        request: CancelRoomEnvironmentActionRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let actor = crate::session::EnvironmentActor::new(
            crate::session::human_environment_actor_id(&caller_user_id),
            crate::session::EnvironmentActorKind::Human,
            crate::session::human_environment_actor_label(&caller_user_id),
        );
        let result = self
            .state
            .cancel_room_environment_action_as_actor(&request.session_id, actor, &request.action_id)
            .map(|(outcome, environment)| {
                LocalDaemonResponse::RoomEnvironmentActionCancellationUpdated {
                    outcome,
                    environment,
                }
            })
            .map_err(|error| room_environment_control_error("environment.action.cancel", error));
        (result, None)
    }

    pub(super) async fn verify_metaagent_caller(
        &self,
        session_id: &str,
        metaagent_id: &str,
        caller_user_id: &str,
    ) -> Result<String, DaemonError> {
        let Some(agent) = self.state.list_agents().into_iter().find(|agent| {
            agent.id() == metaagent_id
                && agent.session_id() == session_id
                && agent.owner_user_id() == caller_user_id
                && agent.is_metaagent()
        }) else {
            return Err(DaemonError::LocalTransport {
                operation: "dispatch session metaagent command",
                message: format!(
                    "metaagent caller `{metaagent_id}` is not an owned metaagent in session `{session_id}`"
                ),
            });
        };
        Ok(agent.id().to_string())
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

    pub(super) async fn list_projects(
        &self,
        request: ListProjectsRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let projects = self
            .state
            .list_projects(&caller_user_id, request.include_archived)
            .await;
        (Ok(LocalDaemonResponse::ProjectsListed { projects }), None)
    }

    pub(super) async fn rename_project(
        &self,
        request: RenameProjectRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .rename_project(&request.project_id, request.name, &caller_user_id)
            .await
            .map(|project| LocalDaemonResponse::ProjectRenamed { project });
        (result, None)
    }

    pub(super) async fn archive_project(
        &self,
        request: ArchiveProjectRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .archive_project(&request.project_id, &caller_user_id)
            .await
            .map(|(project, sessions)| LocalDaemonResponse::ProjectArchived { project, sessions });
        (result, None)
    }

    pub(super) async fn delete_project(
        &self,
        request: DeleteProjectRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .delete_project(&request.project_id, &caller_user_id)
            .await
            .map(|(project, sessions)| LocalDaemonResponse::ProjectDeleted { project, sessions });
        (result, None)
    }

    pub(super) async fn restore_project(
        &self,
        request: RestoreProjectRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .restore_project(&request.project_id, &caller_user_id)
            .await
            .map(|(project, sessions)| LocalDaemonResponse::ProjectRestored { project, sessions });
        (result, None)
    }

    pub(super) async fn attach_to_session(
        &self,
        request: AttachToSessionRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let attach_request = crate::attachment::AttachRequest::for_user(
            request.session_id,
            request.client_id,
            request.capability_level,
            caller_user_id,
        );
        let result = match self.state.attach(attach_request).await {
            Ok(attachment) => self
                .reconcile_room_environment_actors_if_started(attachment.session_id())
                .map(|()| LocalDaemonResponse::SessionAttached { attachment }),
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn detach_from_session(
        &self,
        request: DetachFromSessionRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = match self.state.detach(&request.attachment_id).await {
            Ok(attachment) => self
                .reconcile_room_environment_actors_if_started(attachment.session_id())
                .map(|()| LocalDaemonResponse::SessionDetached { attachment }),
            Err(error) => Err(error),
        };
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

    pub(super) async fn acknowledge_agent_output_seen(
        &self,
        request: AcknowledgeAgentOutputSeenRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let session_id = request.session_id.clone();
        let agent_id = request.agent_id.clone();
        match self
            .state
            .acknowledge_agent_output_seen(&request.session_id, &request.agent_id, &caller_user_id)
            .await
        {
            Ok(ack) => {
                let response = LocalDaemonResponse::AgentOutputSeenAcknowledged {
                    session_id,
                    agent_id,
                };
                let projection_action = ack
                    .changed
                    .then_some(SessionProjectionAction::Update(ack.session));
                (Ok(response), projection_action)
            }
            Err(error) => (Err(error), None),
        }
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

    pub(super) async fn create_agent_prompt_schedule(
        &self,
        request: crate::local::CreateAgentPromptScheduleRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self.state.create_agent_prompt_schedule(request).await;
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn cancel_agent_prompt_schedule(
        &self,
        request: crate::local::CancelAgentPromptScheduleRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self.state.cancel_agent_prompt_schedule(request).await;
        self.with_session_projection_action_result(result).await
    }

    pub(super) async fn respond_to_interaction(
        &self,
        request: RespondToInteractionRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let RespondToInteractionRequest {
            session_id,
            interaction_id,
            choice_id,
            custom_reply,
        } = request;
        let custom_reply = custom_reply.map(zeroize::Zeroizing::new);
        let result = match self
            .state
            .resolve_runtime_interaction(
                &session_id,
                &interaction_id,
                &choice_id,
                custom_reply.as_deref().map(String::as_str),
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

    fn reconcile_room_environment_actors_if_started(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        match self
            .state
            .reconcile_room_environment_actors(session_id, None)
        {
            Ok(_) | Err(crate::session::EnvironmentError::EnvironmentNotFound { .. }) => Ok(()),
            Err(error) => Err(room_environment_control_error(
                "environment.actors.reconcile",
                error,
            )),
        }
    }
}

fn room_environment_control_error(
    operation: &'static str,
    error: crate::session::EnvironmentError,
) -> DaemonError {
    match error {
        crate::session::EnvironmentError::RoomNotFound { session_id } => {
            DaemonError::SessionNotFound { session_id }
        }
        other => DaemonError::LocalTransport {
            operation,
            message: format!("{}: {other:?}", other.code()),
        },
    }
}
