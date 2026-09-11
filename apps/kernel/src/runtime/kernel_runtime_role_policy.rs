use crate::config::{DaemonConfig, KernelRuntimeRole};
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;

pub(crate) fn ensure_public_request_allowed(
    config: &DaemonConfig,
    request: &LocalDaemonRequest,
) -> Result<(), DaemonError> {
    if config.kernel_runtime_role != KernelRuntimeRole::RemoteLeaseWorker {
        return Ok(());
    }
    let operation = match request {
        LocalDaemonRequest::CreateSession(_) => "session.create",
        LocalDaemonRequest::JoinSessionInvite(_) => "session.invite.join",
        LocalDaemonRequest::ImportExternalProviderSession(_) => "external_session.import",
        LocalDaemonRequest::ImportExternalProviderAgent(_) => "external_agent.import",
        _ => return Ok(()),
    };
    Err(role_denied(config.kernel_runtime_role, operation))
}

pub(crate) fn ensure_managed_context_import_allowed(
    role: KernelRuntimeRole,
) -> Result<(), DaemonError> {
    if role == KernelRuntimeRole::RemoteLeaseWorker {
        return Err(role_denied(role, "managed_context.import"));
    }
    Ok(())
}

fn role_denied(role: KernelRuntimeRole, operation: &'static str) -> DaemonError {
    DaemonError::KernelRuntimeRoleDenied {
        role: role.as_str(),
        operation,
    }
}
