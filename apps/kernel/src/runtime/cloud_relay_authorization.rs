//! Cloud grants belong to the kernel owner; shared-session viewers cannot
//! spend that authority to list or pivot into the owner's other kernels.
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::runtime::command::{KernelCallerKind, KernelCommand, KernelCommandSource};

pub(crate) fn authorize_kernel_cloud_request(
    config: &DaemonConfig,
    command: &KernelCommand,
    request: &LocalDaemonRequest,
) -> Result<(), DaemonError> {
    let cloud_control = matches!(
        request,
        LocalDaemonRequest::CloudRelayStatus(_)
            | LocalDaemonRequest::StartCloudRelayLogin(_)
            | LocalDaemonRequest::PollCloudRelayLogin(_)
            | LocalDaemonRequest::LogoutCloudRelay(_)
            | LocalDaemonRequest::PairCloudRelayClient(_)
            | LocalDaemonRequest::PairCloudRelayMachine(_)
            | LocalDaemonRequest::ConnectCloudRelay(_)
            | LocalDaemonRequest::IssueCloudRelayClientToken(_)
            | LocalDaemonRequest::ResolveKernelClientConnection(_)
    );
    let directory = config.cloud_relay.is_some()
        && matches!(
            request,
            LocalDaemonRequest::ListRemoteMachines(_)
                | LocalDaemonRequest::ListRemoteMachineKernels(_)
                | LocalDaemonRequest::QueryFreshRemoteMachineKernels(_)
        );
    if !cloud_control && !directory {
        return Ok(());
    }
    if command.caller.caller_kind == KernelCallerKind::LocalClient
        && matches!(
            command.source,
            KernelCommandSource::LocalCli
                | KernelCommandSource::LocalIpc
                | KernelCommandSource::DaemonBackground
        )
    {
        return Ok(());
    }
    if let Some(profile) = &config.cloud_relay {
        if command.caller.caller_kind == KernelCallerKind::RemoteClient
            && command.caller.user_id.as_deref() == Some(profile.user_id.as_str())
            && command.caller.realm_id.as_deref() == Some(profile.realm_id.as_str())
        {
            return Ok(());
        }
    }
    Err(DaemonError::LocalTransport { operation: "authorize kernel Cloud authority", message: "only this kernel's owner can manage enrollment, list My kernels or delegate a terminal connection".into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::ResolveKernelClientConnectionRequest;
    #[test]
    fn cloud_pivot_and_status_deny_foreign_session_viewers_and_peers() {
        let mut config = DaemonConfig::new("kernel-a", "machine-a", "fixture");
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "owner".into(),
            realm_id: "realm-a".into(),
            ..Default::default()
        });
        let request = LocalDaemonRequest::ResolveKernelClientConnection(
            ResolveKernelClientConnectionRequest {
                kernel_ref: "kernel-b".into(),
                machine_ref: None,
                client_id: Some("cli-a".into()),
                session_id: None,
                public_key_thumbprint: Some("a".repeat(64)),
            },
        );
        let mut command = KernelCommand::from_local_request("fixture", None, None, &request);
        assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
        command.source = KernelCommandSource::RelayClient;
        command.caller.caller_kind = KernelCallerKind::RemoteClient;
        command.caller.user_id = Some("viewer".into());
        command.caller.realm_id = Some("realm-a".into());
        assert!(authorize_kernel_cloud_request(&config, &command, &request).is_err());
        assert!(authorize_kernel_cloud_request(
            &config,
            &command,
            &LocalDaemonRequest::CloudRelayStatus(crate::local::CloudRelayStatusRequest)
        )
        .is_err());
        command.caller.user_id = Some("owner".into());
        assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
        command.caller.caller_kind = KernelCallerKind::RemoteKernel;
        assert!(authorize_kernel_cloud_request(&config, &command, &request).is_err());
    }
}
