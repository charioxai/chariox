use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{KernelConnectionClass, LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::kernel_access::error;

impl CommandRouter {
    pub(super) fn authorize_external_request(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        if let Some(authority) = command.external_grant_id() {
            self.runtime_state
                .authorize_external_request(&authority, request)?;
        }
        Ok(())
    }

    pub(crate) fn kernel_local_socket_path(&self) -> std::path::PathBuf {
        self.config_projection.snapshot().local_socket_path
    }

    pub(crate) fn session_id_for_attachment_access(&self, id: &str) -> Option<String> {
        self.session_projection.session_id_for_attachment(id)
    }

    pub(super) fn audit_access_terminal_attempt(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        if command.caller.connection_class == Some(KernelConnectionClass::Terminal) {
            if let LocalDaemonRequest::RespondToInteraction(answer) = request {
                self.runtime_state.audit_access_terminal_attempt(
                    &command.caller.caller_id,
                    &command_caller_user_id(command),
                    &command.command_id,
                    answer,
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn dispatch_kernel_access(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        if matches!(request, LocalDaemonRequest::RequestKernelAccess(_)) {
            return Err(error(format!(
                "request access through ws+unix://{}; grants cannot be used over TCP or relay",
                self.kernel_local_socket_path().display()
            )));
        }
        if !matches!(
            request,
            LocalDaemonRequest::ListKernelAccessGrants(_)
                | LocalDaemonRequest::RevokeKernelAccessGrant(_)
        ) {
            return Ok(None);
        }
        if !matches!(
            command.caller.connection_class,
            Some(KernelConnectionClass::Terminal | KernelConnectionClass::ExternalAgent)
        ) {
            return Err(error(
                "only a Chariox terminal or local grant holder can list or revoke access grants",
            ));
        }
        let owner = command_caller_user_id(command);
        Ok(Some(match request {
            LocalDaemonRequest::ListKernelAccessGrants(_) => {
                LocalDaemonResponse::KernelAccessGrantsListed {
                    grants: self.runtime_state.list_kernel_access(&owner),
                    sudo_turns: self.runtime_state.list_sudo_turns(&owner),
                }
            }
            LocalDaemonRequest::RevokeKernelAccessGrant(request) => {
                LocalDaemonResponse::KernelAccessRevoked {
                    revoked: self.runtime_state.revoke_kernel_access(
                        Some(&owner),
                        request.grant_id.as_deref(),
                        "explicit_revoke",
                    )?,
                }
            }
            _ => unreachable!(),
        }))
    }
}
