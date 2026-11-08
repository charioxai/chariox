//! MP-08 / MP-10 / MP-11: a Room collaborator reaches this kernel through the relay to work in
//! that Room. A request without a session scope acts on the kernel itself (its Vault, files,
//! machines, pairings, relay and config, its owner's browser), so a relay client that does not
//! act for this kernel's owner may send only what a Room client needs to open and use the Room.
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::runtime::command::{KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::runtime::session_membership::scope::request_session_scope;

use super::CommandRouter;

impl CommandRouter {
    pub(crate) fn authorize_relay_collaborator_request(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        if command.source != KernelCommandSource::RelayClient
            || command.caller.caller_kind != KernelCallerKind::RemoteClient
            || request_session_scope(request).is_some()
            || room_client_request(request)
        {
            return Ok(());
        }
        let config = self.config_projection.snapshot();
        let owner_user_id = crate::runtime::self_host_terminal_grants::owner_user_id(&config);
        let acts_for_owner =
            if crate::runtime::cloud_relay_control::active_cloud_profile(&config).is_some() {
                crate::runtime::cloud_relay_authorization::cloud_relay_caller_owner(
                    &config,
                    &command.caller,
                )
            } else {
                // Without Cloud, every admitted relay client is a terminal the owner paired.
                command.caller.user_id.as_deref() == Some(owner_user_id.as_str())
            };
        if acts_for_owner {
            return Ok(());
        }
        Err(DaemonError::OwnershipAccessDenied {
            user_id: command.caller.user_id.clone().unwrap_or_default(),
            owner_user_id,
            resource: "this kernel".into(),
            operation: "use this kernel outside a room",
        })
    }
}

/// What a collaborator's Room client sends without a session scope; each carries its own
/// per-caller rule. Everything else without a session scope is the kernel owner's.
fn room_client_request(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        // Connection health and kernel clock.
        LocalDaemonRequest::GetDaemonHealth(_)
            // Entering a Room: the invite, then the waiting room until admitted.
            | LocalDaemonRequest::JoinSessionInvite(_)
            | LocalDaemonRequest::GetWaitingRoomInventory(_)
            | LocalDaemonRequest::GetWaitingRoomPublicSnapshot(_)
            // The composer's command and model catalogs, from the caller's own accounts.
            | LocalDaemonRequest::GetTerminalCommandCatalog(_)
            | LocalDaemonRequest::GetProviderCatalog(_)
            | LocalDaemonRequest::GetClientCatalogSnapshot(_)
            | LocalDaemonRequest::GetClientCatalogRevisions(_)
            // Agent actions addressed by agent; the session actor applies the agent-owner rule.
            | LocalDaemonRequest::ForkAgent(_)
            | LocalDaemonRequest::UndoTurn(_)
            // Pairing a terminal: the link proves the owner's consent.
            | LocalDaemonRequest::JoinTerminalPairingLink(_)
    )
}
