use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::agent_control_executor::execute_agent_control_request;
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::session_actor::{SessionActor, SessionRuntime};
use crate::runtime::state::KernelRuntimeState;

pub(crate) fn is_interactive_command(request: &LocalDaemonRequest) -> bool {
    SessionActor::is_session_interactive_command(request)
        || matches!(
            request,
            LocalDaemonRequest::GrantAgentExtension(_)
                | LocalDaemonRequest::MoveAgentToRemote(_)
                | LocalDaemonRequest::RevokeAgentExtension(_)
                | LocalDaemonRequest::SubmitPrompt(_)
                | LocalDaemonRequest::CancelActivePrompt(_)
        )
}

pub(crate) async fn dispatch_interactive_command(
    session_runtime: &SessionRuntime,
    agent_runtime: &AgentRuntime,
    runtime_state: &KernelRuntimeState,
    command: KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    if SessionActor::is_session_interactive_command(&request) {
        return session_runtime
            .dispatch_session_command(command, request)
            .await;
    }

    match request {
        request @ (LocalDaemonRequest::GrantAgentExtension(_)
        | LocalDaemonRequest::MoveAgentToRemote(_)
        | LocalDaemonRequest::RevokeAgentExtension(_)) => {
            let caller_user_id = command_caller_user_id(&command);
            execute_agent_control_request(runtime_state, &caller_user_id, request).await
        }
        LocalDaemonRequest::SubmitPrompt(request) => {
            agent_runtime
                .dispatch_prompt_submit(&command, request)
                .await
        }
        LocalDaemonRequest::CancelActivePrompt(request) => {
            agent_runtime
                .dispatch_prompt_cancel(&command, request)
                .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "route interactive kernel command",
            message: format!(
                "unsupported interactive command `{}` reached the explicit interactive router",
                command.command_type
            ),
        }),
    }
}
