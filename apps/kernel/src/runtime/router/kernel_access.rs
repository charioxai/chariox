use std::sync::Arc;

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
        // These public routes are human frontend channels; agents use the
        // focus-admitted MCP seam. Refuse before generic session-grant lookup
        // so expired/foreign grants cannot flatten the same admission denial.
        if !command.is_terminal_caller()
            && matches!(
                request,
                LocalDaemonRequest::KernelBrowser(_)
                    | LocalDaemonRequest::RespondToHandoff(_)
                    | LocalDaemonRequest::Notes(_)
                    | LocalDaemonRequest::CaptureVisibleRegion(_)
                    | LocalDaemonRequest::OpenUserAppView(_)
                    | LocalDaemonRequest::ListUserAppViews(_)
                    | LocalDaemonRequest::CloseUserAppView(_)
                    | LocalDaemonRequest::GetUserAppViewFrontend(_)
                    | LocalDaemonRequest::CallUserAppView(_)
                    | LocalDaemonRequest::SubscribeUserAppViews(_)
                    | LocalDaemonRequest::AnswerUserDomainInteraction(_)
            )
        {
            return Err(DaemonError::UserDomainRefused {
                reason: crate::error::UserDomainRefusalReason::NotGranted,
            });
        }
        if self.runtime_state.room_agent_tools_enabled()
            && !command.is_terminal_caller()
            && (matches!(
                request,
                LocalDaemonRequest::FocusAgent(_) | LocalDaemonRequest::CycleAgentFocus(_)
            ) || matches!(request, LocalDaemonRequest::GrantAgentExtension(grant) if grant.kind == crate::local::ExtensionKind::App))
        {
            return Err(crate::runtime::room_tool_admission::denied(
                "owner focus and App acquisition require a terminal decision; agents use approved resource tools",
            ));
        }
        if let Some(authority) = command.external_grant_id() {
            self.runtime_state
                .authorize_external_request(&authority, request)?;
        }
        self.runtime_state
            .authorize_prompt_attachment_role(command, request)
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

    pub(crate) fn machine_gateway(&self) -> &Arc<crate::runtime::machine_gateway::MachineGateway> {
        &self.machine_gateway
    }

    pub(super) async fn list_gateway_machines(
        &self,
        command: &KernelCommand,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if !matches!(
            command.caller.connection_class,
            Some(KernelConnectionClass::Terminal | KernelConnectionClass::ExternalAgent)
        ) {
            return Err(error(
                "only a Chariox terminal or local grant holder can list machines",
            ));
        }
        let machines = self
            .machine_gateway
            .machines()
            .await
            .map_err(|failure| error(format!("{}: {}", failure.code(), failure.message())))?;
        Ok(LocalDaemonResponse::GatewayMachinesListed { machines })
    }

    /// Records the terminal heartbeat of the caller's own attachments; any that
    /// is gone, in another session or owned by someone else is reported lapsed.
    pub(super) async fn heartbeat_attachments(
        &self,
        command: &KernelCommand,
        request: &crate::local::HeartbeatAttachmentsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let owner = crate::runtime::command::command_caller_user_id(command);
        let mut lapsed = Vec::new();
        for attachment in &request.attachments {
            let mine = self
                .runtime_state
                .attachment_owner_user_id(&attachment.attachment_id)
                .await
                .is_ok_and(|id| id == owner);
            let alive = mine
                && self
                    .runtime_state
                    .record_terminal_attachment_heartbeat(
                        &attachment.session_id,
                        &attachment.attachment_id,
                        crate::session::unix_epoch_ms(),
                    )
                    .await
                    .is_ok();
            if !alive {
                lapsed.push(attachment.attachment_id.clone());
            }
        }
        Ok(LocalDaemonResponse::AttachmentsHeartbeated { lapsed })
    }

    /// Pairing the gateway with a machine, or forgetting one, is a person's act at
    /// a Chariox terminal; a grant holder or a forwarded request never reaches it.
    pub(super) async fn dispatch_machine_pairing(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        if !matches!(
            request,
            LocalDaemonRequest::JoinMachineLink(_) | LocalDaemonRequest::ForgetGatewayMachine(_)
        ) {
            return None;
        }
        let owner = command.is_terminal_caller()
            && command_caller_user_id(command) == self.config_projection.local_owner_user_id();
        if !owner {
            return Some(Err(error(
                "only the kernel's owner at a Chariox terminal can pair or forget a machine",
            )));
        }
        let outcome = match request {
            LocalDaemonRequest::JoinMachineLink(join) => self
                .machine_gateway
                .join(join.clone())
                .await
                .map(|machine| LocalDaemonResponse::GatewayMachineJoined { machine }),
            LocalDaemonRequest::ForgetGatewayMachine(forget) => self
                .machine_gateway
                .forget(&forget.name)
                .map(|kernel_id| LocalDaemonResponse::GatewayMachineForgotten { kernel_id }),
            _ => unreachable!("filtered above"),
        };
        Some(outcome.map_err(|failure| error(format!("{}: {}", failure.code(), failure.message()))))
    }

    /// The machine file requests belong to the kernel's owner, at a Chariox terminal
    /// or as a local grant holder (or the gateway acting for one).
    pub(super) async fn dispatch_machine_file(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        use crate::runtime::machine_files as files;
        if !matches!(
            request,
            LocalDaemonRequest::StatMachineFile(_)
                | LocalDaemonRequest::ListMachineDirectory(_)
                | LocalDaemonRequest::ReadMachineFile(_)
                | LocalDaemonRequest::WriteMachineFile(_)
        ) {
            return None;
        }
        let owner = matches!(
            command.caller.connection_class,
            Some(KernelConnectionClass::Terminal | KernelConnectionClass::ExternalAgent)
        ) && command_caller_user_id(command)
            == self.config_projection.local_owner_user_id();
        if !owner {
            return Some(Err(error(
                "only the kernel's owner at a Chariox terminal or as a local grant holder can use machine files",
            )));
        }
        let request = request.clone();
        // Hashing and copying a large file must not stall the request loop.
        let outcome = tokio::task::spawn_blocking(move || match &request {
            LocalDaemonRequest::StatMachineFile(r) => files::stat(r),
            LocalDaemonRequest::ListMachineDirectory(r) => files::list(r),
            LocalDaemonRequest::ReadMachineFile(r) => files::read(r),
            LocalDaemonRequest::WriteMachineFile(r) => files::write(r),
            _ => unreachable!("filtered above"),
        })
        .await;
        Some(outcome.unwrap_or_else(|join| Err(error(format!("machine file task failed: {join}")))))
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

#[cfg(test)]
mod user_domain_admission_tests {
    use super::*;
    use crate::runtime::command::KernelCommandSource;

    #[test]
    fn revoked_grants_and_transport_peers_get_the_same_bounded_frontend_refusal() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let app =
                crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
            let router = CommandRouter::with_interactive_capacity(
                std::sync::Arc::new(tokio::sync::Mutex::new(app)),
                4,
            );
            for request in [
                LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
                    command: crate::local::KernelBrowserCommand::State,
                }),
                LocalDaemonRequest::ListUserAppViews(crate::local::ListUserAppViewsRequest {}),
            ] {
                for class in [
                    KernelConnectionClass::ExternalAgent,
                    KernelConnectionClass::KernelAgent,
                    KernelConnectionClass::RelayPeer,
                    KernelConnectionClass::Unauthenticated,
                ] {
                    let mut command = KernelCommand::from_local_request_with_source(
                        "invalid-admission",
                        KernelCommandSource::RelayPeer,
                        None,
                        None,
                        &request,
                    );
                    command.caller.connection_class = Some(class);
                    command.caller.caller_id = "revoked-grant".into();
                    let error = router
                        .authorize_external_request(&command, &request)
                        .unwrap_err();
                    assert!(matches!(
                        error,
                        DaemonError::UserDomainRefused {
                            reason: crate::error::UserDomainRefusalReason::NotGranted
                        }
                    ));
                    assert_eq!(error.to_string(), "User-domain request refused");
                }
            }
            router.runtime_state.shutdown_cleanup().await.unwrap();
        });
    }
}
