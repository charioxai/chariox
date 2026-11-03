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
    // MP-08 / MP-10 / MP-11: a paired remote caller can renew its own
    // identity only. Otherwise a revoked CLIENT could request an unregistered
    // attached-local subject with the same still-valid delegated token.
    let hosted_transport =
        crate::runtime::cloud_relay_control::cloud_relay_transport_selected(config);
    if command.caller.caller_kind == KernelCallerKind::RemoteClient {
        let binding = match request {
            LocalDaemonRequest::IssueCloudRelayClientToken(request) => Some((
                Some(request.client_id.as_str()),
                request.public_key_thumbprint.as_deref(),
            )),
            LocalDaemonRequest::ResolveKernelClientConnection(request) => Some((
                request.client_id.as_deref(),
                request.public_key_thumbprint.as_deref(),
            )),
            LocalDaemonRequest::JoinTerminalPairingLink(request) if hosted_transport => Some((
                request.terminal_id.as_deref(),
                request.public_key_thumbprint.as_deref(),
            )),
            _ => None,
        };
        if let Some((client_id, key)) = binding {
            let owns_identity = client_id
                .zip(command.caller.client_id.as_deref())
                .is_some_and(|(requested, authenticated)| {
                    if requested == authenticated {
                        return true;
                    }
                    use sha2::{Digest, Sha256};
                    // Pivoting may reach another owner kernel; the signed
                    // issuer prefix changes, the receiving terminal hash does not.
                    authenticated
                        .strip_prefix("kernel-client:")
                        .and_then(|subject| subject.split_once(':'))
                        .is_some_and(|(issuer_hash, client_hash)| {
                            issuer_hash.len() == 64
                                && issuer_hash.bytes().all(|byte| {
                                    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                                })
                                && client_hash
                                    == format!("{:x}", Sha256::digest(requested.as_bytes()))
                        })
                });
            if !owns_identity
                || key.is_none()
                || key != command.caller.public_key_thumbprint.as_deref()
            {
                return Err(DaemonError::LocalTransport { operation: "authorize paired terminal renewal", message: "terminal delegation must preserve the authenticated receiving identity and key".into() });
            }
        }
    }

    let cloud_control = matches!(
        request,
        LocalDaemonRequest::PreflightProviderAccountPortability(_)
            | LocalDaemonRequest::AddSshMachine(_)
            | LocalDaemonRequest::RemoveSshMachine(_)
            | LocalDaemonRequest::CloudRelayStatus(_)
            | LocalDaemonRequest::StartCloudRelayLogin(_)
            | LocalDaemonRequest::PollCloudRelayLogin(_)
            | LocalDaemonRequest::LogoutCloudRelay(_)
            | LocalDaemonRequest::PairCloudRelayClient(_)
            | LocalDaemonRequest::PairCloudRelayMachine(_)
            | LocalDaemonRequest::ConnectCloudRelay(_)
            | LocalDaemonRequest::IssueCloudRelayClientToken(_)
            // A private pivot reuses the kernel's own relay credential.
            | LocalDaemonRequest::ResolveKernelClientConnection(_)
    );
    let directory = hosted_transport
        && matches!(
            request,
            LocalDaemonRequest::ListRemoteMachines(_)
                | LocalDaemonRequest::ListRemoteMachineKernels(_)
                | LocalDaemonRequest::QueryFreshRemoteMachineKernels(_)
        );
    let cloud_pairing = hosted_transport
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
    if let Some(profile) = crate::runtime::cloud_relay_control::active_cloud_profile(config) {
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
    fn retained_profile_private_requests_do_not_spend_cloud_authority() {
        // MP-08 / MP-10 / MP-11: private grants admit private operations only.
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("ws://operator-relay".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            relay_url: "wss://cloud-relay".into(),
            user_id: "cloud-owner".into(),
            realm_id: "cloud-realm".into(),
            ..Default::default()
        });
        for value in [
            serde_json::json!({"ListRemoteMachines": null}),
            serde_json::json!({"JoinTerminalPairingLink": {"pairing_link": "synthetic-link"}}),
        ] {
            let request: LocalDaemonRequest = serde_json::from_value(value).unwrap();
            let mut command = KernelCommand::from_local_request("private", None, None, &request);
            command.source = KernelCommandSource::RelayClient;
            command.caller.caller_kind = KernelCallerKind::RemoteClient;
            command.caller.user_id = Some(crate::session::DEFAULT_LOCAL_USER_ID.into());
            assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
            let cloud = LocalDaemonRequest::CloudRelayStatus(crate::local::CloudRelayStatusRequest);
            assert!(authorize_kernel_cloud_request(&config, &command, &cloud).is_err());
            command.caller.user_id = Some("cloud-owner".into());
            command.caller.realm_id = Some("cloud-realm".into());
            assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
            assert!(authorize_kernel_cloud_request(&config, &command, &cloud).is_err());
        }
    }

    #[test]
    fn paired_terminal_cannot_renew_a_different_identity_or_key() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "owner".into(),
            realm_id: "realm".into(),
            ..Default::default()
        });
        for (client_id, key, allowed) in [
            ("paired-terminal", "bound-key", true),
            ("new-local-identity", "bound-key", false),
            ("paired-terminal", "new-key", false),
        ] {
            let request = LocalDaemonRequest::IssueCloudRelayClientToken(
                crate::local::IssueCloudRelayClientTokenRequest {
                    target_daemon_alias: "kernel".into(),
                    client_id: client_id.into(),
                    session_id: None,
                    public_key_thumbprint: Some(key.into()),
                },
            );
            let mut command = KernelCommand::from_local_request("renewal", None, None, &request);
            command.source = KernelCommandSource::RelayClient;
            command.caller.caller_kind = KernelCallerKind::RemoteClient;
            command.caller.user_id = Some("owner".into());
            command.caller.realm_id = Some("realm".into());
            command.caller.client_id = Some("paired-terminal".into());
            command.caller.public_key_thumbprint = Some("bound-key".into());
            assert_eq!(
                authorize_kernel_cloud_request(&config, &command, &request).is_ok(),
                allowed
            );
            command.source = KernelCommandSource::LocalIpc;
            command.caller.caller_kind = KernelCallerKind::LocalClient;
            assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
        }
    }

    #[test]
    fn paired_terminal_renewal_retains_the_original_client_hash() {
        use sha2::{Digest, Sha256};
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            kernel_id: Some("issuer".into()),
            user_id: "owner".into(),
            realm_id: "realm".into(),
            ..Default::default()
        });
        let subject = format!(
            "kernel-client:{:x}:{:x}",
            Sha256::digest(b"issuer"),
            Sha256::digest(b"original-cli")
        );
        let other_issuer = format!(
            "kernel-client:{:x}:{:x}",
            Sha256::digest(b"another-owner-kernel"),
            Sha256::digest(b"original-cli")
        );
        for (authenticated, client_id) in [
            (subject.as_str(), "original-cli"),
            (subject.as_str(), subject.as_str()),
            (other_issuer.as_str(), "original-cli"),
        ] {
            let request = LocalDaemonRequest::IssueCloudRelayClientToken(
                crate::local::IssueCloudRelayClientTokenRequest {
                    target_daemon_alias: "target".into(),
                    client_id: client_id.into(),
                    session_id: None,
                    public_key_thumbprint: Some("key".into()),
                },
            );
            let mut command = KernelCommand::from_local_request("renewal", None, None, &request);
            command.source = KernelCommandSource::RelayClient;
            command.caller.caller_kind = KernelCallerKind::RemoteClient;
            command.caller.user_id = Some("owner".into());
            command.caller.realm_id = Some("realm".into());
            command.caller.client_id = Some(authenticated.into());
            command.caller.public_key_thumbprint = Some("key".into());
            assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
        }
    }

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
            command.caller.client_id = Some("viewer-terminal".into());
            command.caller.public_key_thumbprint = Some("a".repeat(64));
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
        command.caller.client_id = Some("cli-a".into());
        command.caller.public_key_thumbprint = Some("a".repeat(64));
        assert!(authorize_kernel_cloud_request(&config, &command, &request).is_ok());
        command.caller.caller_kind = KernelCallerKind::RemoteKernel;
        assert!(authorize_kernel_cloud_request(&config, &command, &request).is_err());
    }
}
