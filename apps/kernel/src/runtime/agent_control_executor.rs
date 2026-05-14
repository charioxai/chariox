use crate::error::DaemonError;
use crate::local::{
    AgentGrantKind, GrantAgentCapabilityRequest, LocalDaemonRequest, LocalDaemonResponse,
    MoveAgentToRemoteRequest, RevokeAgentCapabilityRequest,
};
use crate::runtime::capability_registry::{ensure_mcp_exists, ensure_skill_exists};
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_agent_control_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GrantAgentCapability(request) => {
            execute_grant_agent_capability_request(runtime_state, caller_user_id, request).await
        }
        LocalDaemonRequest::MoveAgentToRemote(request) => {
            execute_move_agent_to_remote_request(runtime_state, caller_user_id, request).await
        }
        LocalDaemonRequest::RevokeAgentCapability(request) => {
            execute_revoke_agent_capability_request(runtime_state, caller_user_id, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "agent control request",
            message: "unsupported agent control request".to_string(),
        }),
    }
}

pub(crate) async fn execute_grant_agent_capability_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    request: GrantAgentCapabilityRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request.kind {
        AgentGrantKind::Mcp => {
            ensure_mcp_exists(request.workspace_id.as_deref(), &request.name)?;
            let agent = runtime_state
                .grant_agent_mcp(&request.agent_ref, request.name, caller_user_id)
                .await?;
            Ok(LocalDaemonResponse::AgentCapabilityGranted { agent })
        }
        AgentGrantKind::Skill => {
            ensure_skill_exists(request.workspace_id.as_deref(), &request.name)?;
            let agent = runtime_state
                .grant_agent_skill(&request.agent_ref, request.name, caller_user_id)
                .await?;
            Ok(LocalDaemonResponse::AgentCapabilityGranted { agent })
        }
    }
}

pub(crate) async fn execute_move_agent_to_remote_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    request: MoveAgentToRemoteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let agent = runtime_state
        .move_agent_to_remote(
            &request.session_id,
            &request.agent_ref,
            &request.machine_ref,
            caller_user_id,
        )
        .await?;
    Ok(LocalDaemonResponse::AgentMovedToRemote { agent })
}

pub(crate) async fn execute_revoke_agent_capability_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    request: RevokeAgentCapabilityRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let agent = match request.kind {
        AgentGrantKind::Mcp => {
            runtime_state
                .revoke_agent_mcp(&request.agent_ref, &request.name, caller_user_id)
                .await?
        }
        AgentGrantKind::Skill => {
            runtime_state
                .revoke_agent_skill(&request.agent_ref, &request.name, caller_user_id)
                .await?
        }
    };
    Ok(LocalDaemonResponse::AgentCapabilityRevoked { agent })
}
