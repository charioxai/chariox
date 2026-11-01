//! MP-08 / MP-10 / MP-11: one client flight, ordinary command admission and execution.
use crate::error::DaemonError;
use crate::local::{
    AttachSessionViewRequest, AttachToSessionRequest, GetSessionHistoryOutlineRequest,
    GetSessionStateRequest, LocalDaemonRequest, LocalDaemonResponse,
};
use crate::runtime::command::KernelCommand;

use super::CommandRouter;

const INITIAL_PANE_COUNT: usize = 6;
/// The most turns per agent an initial view may ask for; a pane needs a screenful, not the history.
const MAX_INITIAL_VIEW_PROMPTS: u32 = 20;

/// A view that does not ask gets the latest turn; one that does gets what its pane needs, capped.
fn initial_view_prompt_count(requested: Option<u32>) -> usize {
    usize::try_from(requested.unwrap_or(1).clamp(1, MAX_INITIAL_VIEW_PROMPTS)).unwrap_or(1)
}

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
        // Register the ordinary recipient before either read. The owned,
        // caller-redacted snapshot selects only the initial pane group; both
        // serialized components still enter the usual dispatch/admission path.
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
        // Read the ordinary caller-protected state once. Use that SAME state
        // to select the six-pane history group and return it to the client.
        let state = match self
            .session_view_component(
                command,
                "state",
                LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                    session_id: request.session_id.clone(),
                }),
            )
            .await
        {
            Ok(state @ LocalDaemonResponse::SessionState { .. }) => state,
            result => {
                let error = result.err().unwrap_or_else(unexpected_response);
                return Err(self
                    .cleanup_failed_session_view(command, attachment.id(), error)
                    .await);
            }
        };
        let LocalDaemonResponse::SessionState {
            session: selected, ..
        } = &state
        else {
            unreachable!()
        };
        let agent_ids = initial_view_agent_ids(selected, request.preferred_agent_id.as_deref());
        let mut history = match self
            .session_view_component(
                command,
                "history",
                LocalDaemonRequest::GetSessionHistoryOutline(GetSessionHistoryOutlineRequest {
                    session_id: request.session_id,
                    agent_ids: Some(agent_ids),
                    latest_prompt_count: Some(initial_view_prompt_count(
                        request.latest_prompt_count,
                    )),
                    cursor: None,
                    include_prefetched_content: false,
                }),
            )
            .await
        {
            Ok(history @ LocalDaemonResponse::SessionHistoryOutline { .. }) => history,
            result => {
                let error = result.err().unwrap_or_else(unexpected_response);
                return Err(self
                    .cleanup_failed_session_view(command, attachment.id(), error)
                    .await);
            }
        };
        // Membership/agent visibility may change while the two reads run.
        // Never return history outside the final caller-scoped state.
        if let (
            LocalDaemonResponse::SessionState { session, .. },
            LocalDaemonResponse::SessionHistoryOutline { agents },
        ) = (&state, &mut history)
        {
            agents.retain(|outline| {
                session
                    .agents()
                    .iter()
                    .any(|agent| agent.id() == outline.agent_id)
            });
        }
        Ok(LocalDaemonResponse::SessionViewAttached {
            attachment,
            state: Box::new(state),
            history: Box::new(history),
        })
    }

    async fn cleanup_failed_session_view(
        &self,
        command: &KernelCommand,
        attachment_id: &str,
        error: DaemonError,
    ) -> DaemonError {
        match self
            .session_view_component(
                command,
                "cleanup",
                LocalDaemonRequest::DetachFromSession(crate::local::DetachFromSessionRequest {
                    attachment_id: attachment_id.into(),
                }),
            )
            .await
        {
            Ok(_) => error,
            Err(_) => DaemonError::LocalTransport {
                operation: "attach initial session view",
                message: "initial view and attachment cleanup both failed".into(),
            },
        }
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

fn initial_view_agent_ids(
    session: &crate::session::RuntimeSession,
    preferred: Option<&str>,
) -> Vec<String> {
    let agents = session.agents();
    let target = preferred.or(session.focused_agent_id());
    let index = target
        .and_then(|id| agents.iter().position(|agent| agent.id() == id))
        .unwrap_or(0);
    agents
        .iter()
        .skip(index / INITIAL_PANE_COUNT * INITIAL_PANE_COUNT)
        .take(INITIAL_PANE_COUNT)
        .map(|agent| agent.id().to_string())
        .collect()
}

fn unexpected_response() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "attach initial session view",
        message: "unexpected component response".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_view_prompt_count_defaults_to_one_and_is_capped() {
        assert_eq!(initial_view_prompt_count(None), 1);
        assert_eq!(initial_view_prompt_count(Some(0)), 1);
        assert_eq!(initial_view_prompt_count(Some(8)), 8);
        assert_eq!(initial_view_prompt_count(Some(500)), 20);
    }
}
