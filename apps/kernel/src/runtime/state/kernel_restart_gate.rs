//! Row 551: who may restart a kernel (a hot reload of it). One gate, checked where the request enters the router.
use super::*;
use crate::local::{KernelConnectionClass, LocalDaemonRequest};
use crate::runtime::command::{KernelCommand, KernelCommandSource};

const OWNER_ONLY: &str = "kernel_restart_owner_only";
const WORDS: &str = "Only the owner of this kernel or its agents can restart it.";

impl KernelRuntimeState {
    /// `Ok` for every request that is not a kernel restart. A restart is allowed for the owner's own client, for an agent of the
    /// owner, and for another kernel of the same owner asking as a relay peer.
    pub(crate) fn ensure_kernel_restart_allowed(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        if !matches!(request, LocalDaemonRequest::RestartKernel(_)) {
            return Ok(());
        }
        let class = command.caller.connection_class;
        let agent = command.caller.metaagent_id.is_some()
            || matches!(
                class,
                Some(KernelConnectionClass::KernelAgent | KernelConnectionClass::ExternalAgent)
            );
        let owner = self.owned.config_projection.local_owner_user_id();
        let same_owner_peer = command.source == KernelCommandSource::RelayPeer
            && crate::runtime::command::command_caller_user_id(command) == owner;
        if class == Some(KernelConnectionClass::Host)
            || agent
            || same_owner_peer
            || self.owner_client_user_id(command).is_ok()
        {
            Ok(())
        } else {
            Err(DaemonError::MachineRequest {
                code: OWNER_ONLY.into(),
                message: WORDS.into(),
            })
        }
    }
}

/// The refusal of a restart whose hot reload could not be prepared: the reason, and that the kernel keeps running.
pub(crate) fn restart_refused(reason: &str) -> DaemonError {
    DaemonError::MachineRequest {
        code: "kernel_restart_refused".into(),
        message: format!(
            "The kernel was not restarted: hot reload refused ({reason}). It keeps running."
        ),
    }
}
