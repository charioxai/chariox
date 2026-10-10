//! Cloud grants belong to this account; session collaborators cannot spend
//! that authority to list or pivot into the account's other kernels.
//! The existing relay user_id pin identifies the account's login principal.
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::runtime::command::{KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource};

pub(crate) fn authorize_kernel_cloud_request(
    config: &DaemonConfig,
    command: &KernelCommand,
    request: &LocalDaemonRequest,
) -> Result<(), DaemonError> {
    let cloud_control = matches!(
        request,
        LocalDaemonRequest::PreflightProviderAccountPortability(_)
            | LocalDaemonRequest::CloudRelayStatus(_)
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
    let cloud_pairing = config.cloud_relay.is_some()
        && matches!(
            request,
            LocalDaemonRequest::CreateTerminalPairingLink(_)
                | LocalDaemonRequest::JoinTerminalPairingLink(_)
        );
    if !cloud_control && !directory && !cloud_pairing {
        return Ok(());
    }
    if kernel_cloud_owner(config, command) {
        return Ok(());
    }
    Err(DaemonError::LocalTransport { operation: "authorize kernel Cloud authority", message: "only this kernel's owner can manage enrollment, list My kernels or delegate a terminal connection".into() })
}

// Use the complete authenticated caller, not a user-ID-only projection.
// Session membership permits waiting-room access, never owner inventory.
pub(crate) fn kernel_cloud_owner(config: &DaemonConfig, command: &KernelCommand) -> bool {
    if command.caller.caller_kind == KernelCallerKind::LocalClient
        && matches!(
            command.source,
            KernelCommandSource::LocalCli
                | KernelCommandSource::LocalIpc
                | KernelCommandSource::DaemonBackground
        )
    {
        return true;
    }
    cloud_relay_caller_owner(config, &command.caller)
}

pub(crate) fn cloud_relay_caller_owner(config: &DaemonConfig, caller: &KernelCaller) -> bool {
    if let Some(profile) = &config.cloud_relay {
        if caller.caller_kind == KernelCallerKind::RemoteClient
            && caller.user_id.as_deref() == Some(profile.user_id.as_str())
            && caller.realm_id.as_deref() == Some(profile.realm_id.as_str())
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::ResolveKernelClientConnectionRequest;
    #[test]
    fn provider_portability_is_owner_scoped_before_session_membership() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "owner".into(),
            realm_id: "realm-a".into(),
            ..Default::default()
        });
        let request = LocalDaemonRequest::PreflightProviderAccountPortability(
            crate::local::PreflightProviderAccountPortabilityRequest {
                provider_accounts: crate::local::ManagedEnvironmentProviderAccounts::None,
            },
        );
        let mut command = KernelCommand::from_local_request("preflight", None, None, &request);
        assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
        command.source = KernelCommandSource::RelayClient;
        command.caller.caller_kind = KernelCallerKind::RemoteClient;
        command.caller.user_id = Some("collaborator".into());
        command.caller.realm_id = Some("realm-a".into());
        command.session_id = Some("shared-session".into());
        assert!(authorize_kernel_cloud_request(&config, &command, &request).is_err());
        command.caller.user_id = Some("owner".into());
        assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
    }
    #[test]
    fn cloud_pairing_redemption_rejects_shared_session_callers() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "owner".into(),
            realm_id: "realm-a".into(),
            ..Default::default()
        });
        for link in ["forged-link", "revoked-link", "issued-link"] {
            let request = LocalDaemonRequest::JoinTerminalPairingLink(
                crate::local::JoinTerminalPairingLinkRequest {
                    pairing_link: link.into(),
                    terminal_id: Some("viewer-terminal".into()),
                    terminal_type: None,
                    alias: None,
                    public_key_thumbprint: Some("a".repeat(64)),
                },
            );
            let mut command = KernelCommand::from_local_request("pairing", None, None, &request);
            command.source = KernelCommandSource::RelayClient;
            command.caller.caller_kind = KernelCallerKind::RemoteClient;
            command.caller.user_id = Some("collaborator".into());
            command.caller.realm_id = Some("realm-a".into());
            command.session_id = Some("shared-session".into());
            assert!(
                authorize_kernel_cloud_request(&config, &command, &request).is_err(),
                "session membership cannot spend enrollment authority"
            );
            command.caller.user_id = Some("owner".into());
            assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
        }
    }
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
