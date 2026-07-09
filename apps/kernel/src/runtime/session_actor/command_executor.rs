use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (result, projection_action) = if let Some(result) = projected_runtime_notices_response(
            &self.session_projection,
            &self.terminal_stream,
            &request,
        ) {
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
            self.execute_store_request(request, caller_user_id, caller_metaagent_id)
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

    async fn execute_store_request(
        &self,
        request: LocalDaemonRequest,
        caller_user_id: String,
        caller_metaagent_id: Option<String>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        match request {
            LocalDaemonRequest::CreateSession(request) => {
                self.store.create_session(request, caller_user_id).await
            }
            LocalDaemonRequest::AttachToSession(request) => {
                self.store.attach_to_session(request, caller_user_id).await
            }
            LocalDaemonRequest::DetachFromSession(request) => {
                self.store.detach_from_session(request).await
            }
            LocalDaemonRequest::FocusAgent(request) => {
                self.store.focus_agent(request, caller_user_id).await
            }
            LocalDaemonRequest::AcknowledgeAgentOutputSeen(request) => {
                self.store
                    .acknowledge_agent_output_seen(request, caller_user_id)
                    .await
            }
            LocalDaemonRequest::CycleAgentFocus(request) => {
                self.store.cycle_agent_focus(request, caller_user_id).await
            }
            LocalDaemonRequest::ResizeTerminal(request) => {
                self.store.resize_terminal(request).await
            }
            LocalDaemonRequest::SendTerminalInput(request) => {
                self.store.send_terminal_input(request).await
            }
            LocalDaemonRequest::PollRuntimeNotices(request) => {
                self.store.poll_runtime_notices(request).await
            }
            LocalDaemonRequest::UpdateSessionConfig(request) => {
                self.store.update_session_config(request).await
            }
            LocalDaemonRequest::UpdateAgentConfig(request) => {
                self.store
                    .update_agent_config(request, caller_user_id)
                    .await
            }
            LocalDaemonRequest::UpdateAgentProfile(request) => {
                self.store
                    .update_agent_profile(request, caller_user_id)
                    .await
            }
            LocalDaemonRequest::AliasAgent(request) => {
                self.store.alias_agent(request, caller_user_id).await
            }
            LocalDaemonRequest::UpdateAgentSubstitutes(request) => {
                self.store
                    .update_agent_substitutes(request, caller_user_id)
                    .await
            }
            LocalDaemonRequest::RespondToInteraction(request) => {
                self.store.respond_to_interaction(request).await
            }
            LocalDaemonRequest::AliasSession(request) => self.store.alias_session(request).await,
            LocalDaemonRequest::SpawnAgent(request) => {
                self.store
                    .spawn_agent(request, caller_user_id, caller_metaagent_id)
                    .await
            }
            LocalDaemonRequest::SpawnAgents(request) => {
                self.store
                    .spawn_agents(request, caller_user_id, caller_metaagent_id)
                    .await
            }
            LocalDaemonRequest::UndoTurn(request) => {
                self.store.undo_turn(request, caller_user_id).await
            }
            LocalDaemonRequest::ForkAgent(request) => {
                self.store.fork_agent(request, caller_user_id).await
            }
            LocalDaemonRequest::DestroyAgent(request) => {
                self.store
                    .destroy_agent(request, caller_user_id, caller_metaagent_id.is_some())
                    .await
            }
            LocalDaemonRequest::EndSession(request) => self.store.end_session(request).await,
            LocalDaemonRequest::DeleteSession(request) => self.store.delete_session(request).await,
            _ => (
                Err(DaemonError::LocalTransport {
                    operation: "execute session request",
                    message: "request is not handled by the session runtime".to_string(),
                }),
                None,
            ),
        }
    }
}
