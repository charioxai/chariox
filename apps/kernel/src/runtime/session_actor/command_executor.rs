use crate::error::DaemonError;
use crate::local::{KernelConnectionClass, LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::projection::{
    publish_session_runtime_projection, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::terminal::TerminalStreamStore;

use super::focus_projection::FocusedAgentProjection;
use super::projection_policy::{
    projected_config_update_absence_response, projected_resize_terminal_response,
    projected_runtime_notices_response, projected_session_absence_response,
    projected_terminal_input_absence_response, session_id_for_projection_refresh,
    update_focus_projection_after_session_command, SessionProjectionAction,
};
use super::store::SessionRuntimeStore;

type StoreRequestFuture<'a> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = (
                    Result<LocalDaemonResponse, DaemonError>,
                    Option<SessionProjectionAction>,
                ),
            > + Send
            + 'a,
    >,
>;

#[derive(Clone)]
pub(super) struct SessionRuntimeCommandExecutor {
    store: SessionRuntimeStore,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    terminal_stream: TerminalStreamStore,
    session_id: String,
}

impl SessionRuntimeCommandExecutor {
    pub(super) fn with_external_command_authority(
        &self,
        authority: Option<(&str, &LocalDaemonRequest)>,
        sudo_binding: Option<(String, String)>,
    ) -> Self {
        let mut executor = self.clone();
        executor.store = self
            .store
            .with_external_command_authority(authority, sudo_binding);
        executor
    }

    pub(super) fn with_room_request_origin(
        &self,
        actor: Option<&str>,
        request: &LocalDaemonRequest,
    ) -> Self {
        let mut executor = self.clone();
        executor.store = self.store.with_room_request_origin(actor, request);
        executor
    }

    pub(super) fn with_room_provider_origin(&self, actor: Option<&str>, run: Option<&str>) -> Self {
        let mut executor = self.clone();
        executor.store = self.store.with_room_provider_origin(actor, run);
        executor
    }

    pub(super) fn new(
        store: SessionRuntimeStore,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        terminal_stream: TerminalStreamStore,
        session_id: String,
    ) -> Self {
        Self {
            store,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            terminal_stream,
            session_id,
        }
    }

    pub(super) async fn execute(
        &self,
        request: LocalDaemonRequest,
        caller_user_id: String,
        caller_metaagent_id: Option<String>,
        terminal_caller: bool,
        connection_class: Option<KernelConnectionClass>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if let Some(actor) = caller_metaagent_id.as_deref() {
            self.store.authorize_room_agent_request(actor, &request)?;
        }
        let (result, projection_action) = if let Some(result) = projected_runtime_notices_response(
            &self.session_projection,
            &self.terminal_stream,
            &request,
        ) {
            let result = match result {
                Ok(response) => {
                    let LocalDaemonRequest::PollRuntimeNotices(request) = &request else {
                        unreachable!("projected runtime notices require a notice poll");
                    };
                    self.store
                        .record_terminal_attachment_heartbeat(
                            &request.session_id,
                            &request.attachment_id,
                        )
                        .await
                        .map(|()| response)
                }
                Err(error) => Err(error),
            };
            let projection_action = if result.is_ok() {
                session_id_for_projection_refresh(&result)
                    .and_then(|session_id| self.session_projection.get(&session_id))
                    .map(SessionProjectionAction::Update)
            } else {
                None
            };
            (result, projection_action)
        } else if let Some(result) =
            projected_resize_terminal_response(&self.session_projection, &request)
        {
            let projection_action = if result.is_ok() {
                session_id_for_projection_refresh(&result)
                    .and_then(|session_id| self.session_projection.get(&session_id))
                    .map(SessionProjectionAction::Update)
            } else {
                None
            };
            (result, projection_action)
        } else if let Some(result) =
            projected_terminal_input_absence_response(&self.session_projection, &request)
        {
            (result, None)
        } else if let Some(result) =
            projected_config_update_absence_response(&self.session_projection, &request)
        {
            (result, None)
        } else if let Some(result) =
            projected_session_absence_response(&self.session_projection, &request)
        {
            (result, None)
        } else {
            self.execute_store_request(
                request,
                caller_user_id,
                caller_metaagent_id,
                terminal_caller,
                connection_class,
            )
            .await
        };
        let projected_session = match projection_action {
            Some(SessionProjectionAction::Update(session)) => {
                publish_session_runtime_projection(
                    &self.session_projection,
                    &self.agent_runtime_projection,
                    &session,
                );
                Some(session)
            }
            Some(SessionProjectionAction::Remove { session_id }) => {
                self.agent_runtime_projection.remove_session(&session_id);
                self.session_projection.remove(&session_id);
                None
            }
            None => None,
        };
        update_focus_projection_after_session_command(
            &self.focus_projection,
            &self.session_id,
            &result,
            projected_session
                .as_ref()
                .and_then(|session| session.focused_agent_id()),
        )
        .await;
        if matches!(
            result,
            Ok(LocalDaemonResponse::SessionEnded { .. })
                | Ok(LocalDaemonResponse::SessionDeleted { .. })
        ) {
            self.terminal_stream.remove_session(&self.session_id);
        }
        result
    }

    // MD-4: select before polling so FocusAgent never reserves unrelated store futures.
    fn execute_store_request(
        &self,
        request: LocalDaemonRequest,
        caller_user_id: String,
        caller_metaagent_id: Option<String>,
        terminal_caller: bool,
        connection_class: Option<KernelConnectionClass>,
    ) -> StoreRequestFuture<'_> {
        match request {
            LocalDaemonRequest::CreateSession(request) => {
                Box::pin(async move { self.store.create_session(request, caller_user_id).await })
            }
            LocalDaemonRequest::ListProjects(request) => {
                Box::pin(async move { self.store.list_projects(request, caller_user_id).await })
            }
            LocalDaemonRequest::RenameProject(request) => {
                Box::pin(async move { self.store.rename_project(request, caller_user_id).await })
            }
            LocalDaemonRequest::UpdateProjectWorkspaces(request) => Box::pin(async move {
                self.store
                    .update_project_workspaces(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::ArchiveProject(request) => {
                Box::pin(async move { self.store.archive_project(request, caller_user_id).await })
            }
            LocalDaemonRequest::DeleteProject(request) => {
                Box::pin(async move { self.store.delete_project(request, caller_user_id).await })
            }
            LocalDaemonRequest::RestoreProject(request) => {
                Box::pin(async move { self.store.restore_project(request, caller_user_id).await })
            }
            LocalDaemonRequest::AttachToSession(request) => Box::pin(async move {
                self.store
                    .attach_to_session(request, caller_user_id, terminal_caller)
                    .await
            }),
            LocalDaemonRequest::DetachFromSession(request) => {
                Box::pin(async move { self.store.detach_from_session(request).await })
            }
            LocalDaemonRequest::FocusAgent(request) => Box::pin(async move {
                self.store
                    .focus_agent(request, caller_user_id, terminal_caller)
                    .await
            }),
            LocalDaemonRequest::AcknowledgeAgentOutputSeen(request) => Box::pin(async move {
                self.store
                    .acknowledge_agent_output_seen(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::CycleAgentFocus(request) => Box::pin(async move {
                self.store
                    .cycle_agent_focus(request, caller_user_id, terminal_caller)
                    .await
            }),
            LocalDaemonRequest::ResizeTerminal(request) => {
                Box::pin(async move { self.store.resize_terminal(request).await })
            }
            LocalDaemonRequest::SendTerminalInput(request) => {
                Box::pin(async move { self.store.send_terminal_input(request).await })
            }
            LocalDaemonRequest::PollRuntimeNotices(request) => {
                Box::pin(async move { self.store.poll_runtime_notices(request).await })
            }
            LocalDaemonRequest::UpdateSessionConfig(request) => {
                Box::pin(async move { self.store.update_session_config(request).await })
            }
            LocalDaemonRequest::StartRoomEnvironment(request) => Box::pin(async move {
                self.store
                    .start_room_environment(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::BindRoomEnvironmentSlice(request) => Box::pin(async move {
                self.store
                    .bind_room_environment_slice(request, caller_user_id)
            }),
            LocalDaemonRequest::StopRoomEnvironment(request) => {
                Box::pin(async move { self.store.stop_room_environment(request).await })
            }
            LocalDaemonRequest::RetryRoomEnvironment(request) => {
                Box::pin(async move { self.store.retry_room_environment(request).await })
            }
            LocalDaemonRequest::UpdateRoomEnvironmentViewport(request) => Box::pin(async move {
                self.store
                    .update_room_environment_viewport(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::SetRoomBrowserBar(request) => Box::pin(async move {
                self.store
                    .set_room_browser_bar(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::UpdateRoomEnvironmentPointer(request) => Box::pin(async move {
                self.store
                    .update_room_environment_pointer(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(request) => {
                Box::pin(async move {
                    self.store
                        .request_room_environment_input_takeover(request, caller_user_id)
                        .await
                })
            }
            LocalDaemonRequest::ReleaseRoomEnvironmentInput(request) => Box::pin(async move {
                self.store
                    .release_room_environment_input(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::SubmitRoomEnvironmentAction(request) => Box::pin(async move {
                self.store
                    .submit_room_environment_action(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::SubmitRoomEnvironmentBrowserAction(request) => {
                Box::pin(async move {
                    self.store
                        .submit_room_environment_browser_action(request, caller_user_id)
                        .await
                })
            }
            LocalDaemonRequest::ReadRoomEnvironmentClipboard(request) => Box::pin(async move {
                self.store
                    .read_room_environment_clipboard(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::CancelRoomEnvironmentAction(request) => Box::pin(async move {
                self.store
                    .cancel_room_environment_action(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::CreateAgentPromptSchedule(request) => {
                Box::pin(async move { self.store.create_agent_prompt_schedule(request).await })
            }
            LocalDaemonRequest::CancelAgentPromptSchedule(request) => {
                Box::pin(async move { self.store.cancel_agent_prompt_schedule(request).await })
            }
            LocalDaemonRequest::UpdateAgentConfig(request) => Box::pin(async move {
                self.store
                    .update_agent_config(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::UpdateAgentProfile(request) => Box::pin(async move {
                self.store
                    .update_agent_profile(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::AliasAgent(request) => {
                Box::pin(async move { self.store.alias_agent(request, caller_user_id).await })
            }
            LocalDaemonRequest::UpdateAgentSubstitutes(request) => Box::pin(async move {
                self.store
                    .update_agent_substitutes(request, caller_user_id)
                    .await
            }),
            LocalDaemonRequest::RespondToInteraction(request) => Box::pin(async move {
                // Managed host controllers keep their existing owner decision
                // route, without becoming terminals. The interaction gate still
                // refuses host passkeys and credential-prompt answers.
                let owner_caller = terminal_caller
                    || (caller_metaagent_id.is_none()
                        && matches!(
                            connection_class,
                            Some(
                                KernelConnectionClass::Host | KernelConnectionClass::ExternalAgent
                            )
                        ));
                self.store
                    .respond_to_interaction(
                        request,
                        owner_caller.then_some(caller_user_id),
                        connection_class,
                    )
                    .await
            }),
            LocalDaemonRequest::AliasSession(request) => {
                Box::pin(async move { self.store.alias_session(request).await })
            }
            LocalDaemonRequest::SpawnAgent(request) => Box::pin(async move {
                self.store
                    .spawn_agent(request, caller_user_id, caller_metaagent_id)
                    .await
            }),
            LocalDaemonRequest::SpawnAgents(request) => Box::pin(async move {
                self.store
                    .spawn_agents(request, caller_user_id, caller_metaagent_id)
                    .await
            }),
            LocalDaemonRequest::UndoTurn(request) => {
                Box::pin(async move { self.store.undo_turn(request, caller_user_id).await })
            }
            LocalDaemonRequest::ForkAgent(request) => {
                Box::pin(async move { self.store.fork_agent(request, caller_user_id).await })
            }
            LocalDaemonRequest::DestroyAgent(request) => Box::pin(async move {
                self.store
                    .destroy_agent(request, caller_user_id, caller_metaagent_id.is_some())
                    .await
            }),
            LocalDaemonRequest::EndSession(request) => {
                Box::pin(async move { self.store.end_session(request).await })
            }
            LocalDaemonRequest::DeleteSession(request) => {
                Box::pin(async move { self.store.delete_session(request).await })
            }
            _ => Box::pin(async move {
                (
                    Err(DaemonError::LocalTransport {
                        operation: "execute session request",
                        message: "request is not handled by the session runtime".to_string(),
                    }),
                    None,
                )
            }),
        }
    }
}
