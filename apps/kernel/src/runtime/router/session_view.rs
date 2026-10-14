//! MP-08 / MP-10 / MP-11: one client flight, ordinary command admission and execution.
use crate::error::DaemonError;
use crate::local::{
    AttachSessionViewRequest, AttachToSessionRequest, GetSessionHistoryOutlineRequest,
    GetSessionStateRequest, LocalDaemonRequest, LocalDaemonResponse,
};
use crate::runtime::command::KernelCommand;

use super::CommandRouter;

const INITIAL_PANE_COUNT: usize = 6;

impl CommandRouter {
    pub(super) async fn attach_session_view(
        &self,
        command: &KernelCommand,
        request: AttachSessionViewRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        // A full terminal view is not an external-agent response surface.
        // Every component still passes the usual role, grant, membership and
        // caller-specific response protection in dispatch below.
        if command.external_grant_id().is_some() {
            return Err(crate::runtime::kernel_access::error(
                "initial session view requires a terminal",
            ));
        }
        let state = self
            .session_view_component(
                command,
                "state",
                LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                    session_id: request.session_id.clone(),
                }),
            )
            .await?;
        let LocalDaemonResponse::SessionState { session, .. } = &state else {
            return Err(unexpected_response());
        };
        let visible_agents = session
            .agents()
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        let target = request
            .preferred_agent_id
            .as_deref()
            .or(session.focused_agent_id());
        let target_index = target
            .and_then(|id| visible_agents.iter().position(|agent| agent == id))
            .unwrap_or(0);
        let start = target_index / INITIAL_PANE_COUNT * INITIAL_PANE_COUNT;
        let agent_ids = visible_agents
            .into_iter()
            .skip(start)
            .take(INITIAL_PANE_COUNT)
            .collect();
        // Register the recipient before reading history, so output produced
        // after the read remains buffered for the initial event subscription.
        let attached = self
            .session_view_component(
                command,
                "attach",
                LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
                    session_id: request.session_id.clone(),
                    client_id: request.client_id,
                    capability_level: request.capability_level,
                }),
            )
            .await?;
        let LocalDaemonResponse::SessionAttached { attachment } = attached else {
            return Err(unexpected_response());
        };
        let history_result = self
            .session_view_component(
                command,
                "history",
                LocalDaemonRequest::GetSessionHistoryOutline(GetSessionHistoryOutlineRequest {
                    session_id: request.session_id.clone(),
                    agent_ids: Some(agent_ids),
                    latest_prompt_count: Some(4),
                    cursor: None,
                    include_prefetched_content: true,
                }),
            )
            .await;
        let history = match history_result {
            Ok(history @ LocalDaemonResponse::SessionHistoryOutline { .. }) => history,
            result => {
                let error = result.err().unwrap_or_else(unexpected_response);
                self.session_view_component(
                    command,
                    "cleanup",
                    LocalDaemonRequest::DetachFromSession(crate::local::DetachFromSessionRequest {
                        attachment_id: attachment.id().into(),
                    }),
                )
                .await
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "attach initial session view",
                    message: "initial history and attachment cleanup both failed".into(),
                })?;
                return Err(error);
            }
        };
        Ok(LocalDaemonResponse::SessionViewAttached {
            attachment,
            state: Box::new(state),
            history: Box::new(history),
        })
    }

    async fn session_view_component(
        &self,
        command: &KernelCommand,
        component: &str,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let component_command = KernelCommand::from_local_request_with_caller(
            format!("{}:view:{component}", command.command_id),
            command.source.clone(),
            command.caller.clone(),
            Some(command.correlation_id.clone()),
            Some(command.command_id.clone()),
            &request,
        );
        Box::pin(self.dispatch(component_command, request)).await
    }
}

fn unexpected_response() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "attach initial session view",
        message: "unexpected component response".into(),
    }
}
