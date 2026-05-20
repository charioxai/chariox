use crate::error::DaemonError;
use crate::local::{
    ExtensionKind, GrantAgentExtensionRequest, LocalDaemonRequest, LocalDaemonResponse,
    MoveAgentToRemoteRequest, RevokeAgentExtensionRequest,
};
use crate::runtime::capability_registry::{
    ensure_connector_exists, ensure_credential_exists, ensure_environment_exists,
    ensure_mcp_exists, ensure_script_exists, ensure_skill_exists,
};
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_agent_control_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GrantAgentExtension(request) => {
            execute_grant_agent_extension_request(runtime_state, caller_user_id, request).await
        }
        LocalDaemonRequest::MoveAgentToRemote(request) => {
            execute_move_agent_to_remote_request(runtime_state, caller_user_id, request).await
        }
        LocalDaemonRequest::RevokeAgentExtension(request) => {
            execute_revoke_agent_extension_request(runtime_state, caller_user_id, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "agent control request",
            message: "unsupported agent control request".to_string(),
        }),
    }
}

pub(crate) async fn execute_grant_agent_extension_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    request: GrantAgentExtensionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request.kind {
        ExtensionKind::Mcp => {
            ensure_mcp_exists(request.workspace_id.as_deref(), &request.name)?;
            let agent = runtime_state
                .grant_agent_mcp(&request.agent_ref, request.name, caller_user_id)
                .await?;
            Ok(LocalDaemonResponse::AgentExtensionGranted { agent })
        }
        ExtensionKind::Skill => {
            ensure_skill_exists(request.workspace_id.as_deref(), &request.name)?;
            let agent = runtime_state
                .grant_agent_skill(&request.agent_ref, request.name, caller_user_id)
                .await?;
            Ok(LocalDaemonResponse::AgentExtensionGranted { agent })
        }
        ExtensionKind::Script => {
            let environment = request
                .environment
                .ok_or_else(|| DaemonError::InvalidConfig {
                    field: "environment",
                    message: "script extension grants require an environment",
                })?;
            ensure_script_exists(request.workspace_id.as_deref(), &request.name)?;
            ensure_environment_exists(request.workspace_id.as_deref(), &environment)?;
            let grant = crate::extension::ExtensionGrant::script(request.name, environment);
            let agent = runtime_state
                .grant_agent_extension(&request.agent_ref, grant, caller_user_id)
                .await?;
            Ok(LocalDaemonResponse::AgentExtensionGranted { agent })
        }
        ExtensionKind::Connector => {
            ensure_connector_exists(&request.name)?;
            if let Some(credential) = request.credential.as_deref() {
                ensure_credential_exists(credential)?;
            }
            let max_safety =
                crate::connector::ConnectorSafety::parse(request.max_safety.as_deref())?;
            let grant = crate::extension::ExtensionGrant::connector(
                request.name,
                request.credential,
                max_safety.as_str(),
            );
            let agent = runtime_state
                .grant_agent_extension(&request.agent_ref, grant, caller_user_id)
                .await?;
            Ok(LocalDaemonResponse::AgentExtensionGranted { agent })
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

pub(crate) async fn execute_revoke_agent_extension_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    request: RevokeAgentExtensionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let agent = match request.kind {
        ExtensionKind::Mcp => {
            runtime_state
                .revoke_agent_mcp(&request.agent_ref, &request.name, caller_user_id)
                .await?
        }
        ExtensionKind::Skill => {
            runtime_state
                .revoke_agent_skill(&request.agent_ref, &request.name, caller_user_id)
                .await?
        }
        ExtensionKind::Script => {
            runtime_state
                .revoke_agent_extension(
                    &request.agent_ref,
                    crate::extension::ExtensionKind::Script,
                    &request.name,
                    caller_user_id,
                )
                .await?
        }
        ExtensionKind::Connector => {
            runtime_state
                .revoke_agent_extension(
                    &request.agent_ref,
                    crate::extension::ExtensionKind::Connector,
                    &request.name,
                    caller_user_id,
                )
                .await?
        }
    };
    Ok(LocalDaemonResponse::AgentExtensionRevoked { agent })
}
