use std::future::Future;
use std::pin::Pin;

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::agent_control_executor::execute_agent_control_request;
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::session_actor::{SessionActor, SessionRuntime};
use crate::runtime::state::KernelRuntimeState;

type InteractiveCommandFuture<'a> =
    Pin<Box<dyn Future<Output = Result<LocalDaemonResponse, DaemonError>> + Send + 'a>>;

pub(crate) fn is_interactive_command(request: &LocalDaemonRequest) -> bool {
    SessionActor::is_session_interactive_command(request)
        || matches!(
            request,
            LocalDaemonRequest::RespondToHandoff(_)
                | LocalDaemonRequest::GrantAgentExtension(_)
                | LocalDaemonRequest::MoveAgentToRemote(_)
                | LocalDaemonRequest::MoveAgentToLocal(_)
                | LocalDaemonRequest::SyncRemoteExtensionManifest(_)
                | LocalDaemonRequest::ListHomeExtensionAudit(_)
                | LocalDaemonRequest::RevokeAgentExtension(_)
                | LocalDaemonRequest::SubmitPrompt(_)
                | LocalDaemonRequest::SubmitPrompts(_)
                | LocalDaemonRequest::CancelActivePrompt(_)
                | LocalDaemonRequest::SteerQueuedPrompt(_)
                | LocalDaemonRequest::CancelQueuedPrompt(_)
                | LocalDaemonRequest::UpdateQueuedPrompt(_)
        )
}

// MP-08 / MP-10 / MP-11: allocate only the selected operation. An async match
// stores its largest branch in every caller, including unrelated owner controls.
pub(crate) fn dispatch_interactive_command<'a>(
    session_runtime: &'a SessionRuntime,
    agent_runtime: &'a AgentRuntime,
    runtime_state: &KernelRuntimeState,
    command: KernelCommand,
    request: LocalDaemonRequest,
) -> InteractiveCommandFuture<'a> {
    let grant_id = command.external_grant_id();
    let runtime_state =
        runtime_state.with_external_command_authority(grant_id.as_deref().map(|id| (id, &request)));
    if let LocalDaemonRequest::CreateSession(mut inner) = request {
        return boxed_interactive_handler(move || async move {
            inner.agent_defaults = runtime_state
                .resolve_session_agent_defaults(inner.agent_defaults)
                .await?;
            session_runtime
                .dispatch_session_command(command, LocalDaemonRequest::CreateSession(inner))
                .await
        });
    }
    if SessionActor::is_session_interactive_command(&request) {
        return boxed_interactive_handler(move || {
            session_runtime.dispatch_session_command(command, request)
        });
    }
    match request {
        LocalDaemonRequest::RespondToHandoff(request) => {
            boxed_interactive_handler(move || async move {
                let outcome = runtime_state.respond_to_handoff(&command, request).await?;
                Ok(LocalDaemonResponse::HandoffResolved { outcome })
            })
        }
        request @ (LocalDaemonRequest::GrantAgentExtension(_)
        | LocalDaemonRequest::MoveAgentToRemote(_)
        | LocalDaemonRequest::MoveAgentToLocal(_)
        | LocalDaemonRequest::SyncRemoteExtensionManifest(_)
        | LocalDaemonRequest::ListHomeExtensionAudit(_)
        | LocalDaemonRequest::RevokeAgentExtension(_)) => {
            boxed_interactive_handler(move || async move {
                let caller_user_id = command_caller_user_id(&command);
                execute_agent_control_request(&runtime_state, &caller_user_id, request).await
            })
        }
        LocalDaemonRequest::SubmitPrompt(request) => {
            boxed_interactive_handler(move || async move {
                agent_runtime
                    .dispatch_prompt_submit(&command, request)
                    .await
            })
        }
        LocalDaemonRequest::SubmitPrompts(request) => {
            boxed_interactive_handler(move || async move {
                agent_runtime
                    .dispatch_prompt_submit_batch(&command, request)
                    .await
            })
        }
        LocalDaemonRequest::CancelActivePrompt(request) => {
            boxed_interactive_handler(move || async move {
                agent_runtime
                    .dispatch_prompt_cancel(&command, request)
                    .await
            })
        }
        LocalDaemonRequest::SteerQueuedPrompt(request) => {
            boxed_interactive_handler(move || async move {
                agent_runtime
                    .dispatch_prompt_steer_queued(&command, request)
                    .await
            })
        }
        LocalDaemonRequest::CancelQueuedPrompt(request) => {
            boxed_interactive_handler(move || async move {
                agent_runtime
                    .dispatch_prompt_cancel_queued(&command, request)
                    .await
            })
        }
        LocalDaemonRequest::UpdateQueuedPrompt(request) => {
            boxed_interactive_handler(move || async move {
                agent_runtime
                    .dispatch_prompt_update_queued(&command, request)
                    .await
            })
        }
        _ => boxed_interactive_handler(move || async move {
            Err(DaemonError::LocalTransport {
                operation: "route interactive kernel command",
                message: format!(
                    "unsupported interactive command `{}` reached the explicit interactive router",
                    command.command_type,
                ),
            })
        }),
    }
}

// Keep construction/copies of large operation futures off the dispatcher's
// stack; authority is still scoped before selecting and executing the handler.
#[inline(never)]
fn boxed_interactive_handler<'a, F, Fut>(make: F) -> InteractiveCommandFuture<'a>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<LocalDaemonResponse, DaemonError>> + Send + 'a,
{
    Box::pin(make())
}
