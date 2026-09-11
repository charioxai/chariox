use sha2::{Digest, Sha256};
use std::time::Instant;

use super::KernelRuntimeState;
use crate::attachment::ClientCapabilityLevel;
use crate::error::DaemonError;
use crate::local::{
    BrowserImportConsentStatus, BrowserImportSelection, LocalDaemonRequest, LocalDaemonResponse,
};
use crate::runtime::browser_import_admission::{ImportBinding, ImportRequestId};
use crate::runtime::command::{KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::session::{EnvironmentLifecycle, DEFAULT_LOCAL_USER_ID};

/// Cannot be constructed from a caller-supplied identity or detached from its lock.
pub(crate) struct BrowserImportDestination {
    runtime: KernelRuntimeState,
    environment_id: String,
    id: ImportRequestId,
    verified: bool,
    _guard: tokio::sync::OwnedRwLockWriteGuard<()>,
}

impl BrowserImportDestination {
    /// Trusted executor only, after verified application or rollback. Cleanup
    /// must acknowledge removal of the matching encrypted recovery journal.
    /// Any failure retains the durable admission block for restart recovery.
    pub(crate) async fn complete_after_verification<F>(self, cleanup: F) -> Result<(), DaemonError>
    where
        F: std::future::Future<Output = Result<(), DaemonError>>,
    {
        let store = &self.runtime.owned.durable_state_store;
        if !self.verified {
            store
                .mark_browser_import_recovered(&self.environment_id, self.id.as_str())
                .map_err(|_| denied())?;
        }
        cleanup.await.map_err(|_| denied())?;
        store
            .clear_recovered_browser_import(&self.environment_id, self.id.as_str())
            .map_err(|_| denied())?;
        self.runtime
            .owned
            .browser_import_admission
            .finish(&self.id)
            .map_err(|_| denied())
    }
}

impl KernelRuntimeState {
    /// Private encrypted-transport destination entry point. The delivery payload
    /// is never projected through LocalDaemonRequest or an Environment action.
    pub(crate) async fn execute_browser_import_delivery(
        &self,
        command: &KernelCommand,
        request: crate::runtime::browser_import_payload::BrowserImportDeliveryRequest,
    ) -> Result<u16, DaemonError> {
        let (request_id, selection, payload) = request.into_parts();
        let destination = self
            .claim_browser_import_destination(command, &selection, &request_id)
            .await?;
        let target = self
            .room_environment_controller_tab_binding(&selection.session_id, &selection.tab_id)
            .map_err(|_| denied())?;
        if target.document_revision != selection.document_revision {
            return Err(denied());
        }
        let binding = crate::transport::room_browser_controller::RoomBrowserImportBinding {
            request_id,
            user_id: command.caller.user_id.clone().ok_or_else(denied)?,
            room_id: selection.session_id.clone(),
            environment_id: selection.environment_id.clone(),
        };
        let result = self
            .room_browser_controller_command(
                &selection.session_id,
                crate::transport::room_browser_controller::RoomBrowserControllerCommand::ImportCookies {
                    binding,
                    browser_generation: selection.runtime_generation,
                    target_id: target.runtime_target_id,
                    document_id: target.document_id,
                    source_store_id: selection.source_store_id,
                    domains: selection.domains,
                    partition_sites: selection.partition_sites,
                    overwrite: selection.overwrite,
                    payload,
                },
            )
            .await?;
        match result {
            crate::transport::room_browser_controller::RoomBrowserControllerResult::CookiesImported {
                cookie_count,
            } => {
                destination
                    .complete_after_verification(async { Ok(()) })
                    .await?;
                Ok(cookie_count)
            }
            crate::transport::room_browser_controller::RoomBrowserControllerResult::CookieImportRolledBack => {
                destination
                    .complete_after_verification(async { Ok(()) })
                    .await?;
                Err(denied())
            }
            _ => Err(denied()),
        }
    }

    /// Resume cleanup only when verification was durably acknowledged. This
    /// does not authorize replay of cookie writes or treat missing state as success.
    pub(crate) async fn resume_browser_import_cleanup(
        &self,
        command: &KernelCommand,
        session_id: &str,
        attachment_id: &str,
        request_id: &str,
    ) -> Result<BrowserImportDestination, DaemonError> {
        self.browser_import_human(command, session_id, attachment_id)
            .await?;
        let guard = self
            .owned
            .environment_execution_gates
            .for_room(session_id)
            .write_owned()
            .await;
        let (user_id, _) = self
            .browser_import_human(command, session_id, attachment_id)
            .await?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|_| denied())?;
        let pending = self
            .owned
            .durable_state_store
            .pending_browser_import(&environment.environment_id)
            .map_err(|_| denied())?
            .ok_or_else(denied)?;
        if pending.recovery_required
            || pending.user_id != user_id
            || pending.room_id != session_id
            || pending.request_id != request_id
        {
            return Err(denied());
        }
        let id = ImportRequestId::from_wire(request_id).map_err(|_| denied())?;
        Ok(BrowserImportDestination {
            runtime: self.clone(),
            environment_id: environment.environment_id,
            id,
            verified: true,
            _guard: guard,
        })
    }

    /// Destination entry point. Revalidate consent after draining room actions,
    /// then acknowledge durable recovery state before permitting cookie mutation.
    pub(crate) async fn claim_browser_import_destination(
        &self,
        command: &KernelCommand,
        selection: &BrowserImportSelection,
        request_id: &str,
    ) -> Result<BrowserImportDestination, DaemonError> {
        let guard = self
            .owned
            .environment_execution_gates
            .for_room(&selection.session_id)
            .write_owned()
            .await;
        let binding = self.browser_import_binding(command, selection).await?;
        let id = ImportRequestId::from_wire(request_id).map_err(|_| denied())?;
        self.owned
            .browser_import_admission
            .claim(&id, &binding, Instant::now())
            .map_err(|_| denied())?;
        self.owned
            .durable_state_store
            .begin_browser_import_recovery(
                &binding.environment_id,
                id.as_str(),
                &binding.user_id,
                &binding.room_id,
            )
            .map_err(|_| denied())?;
        Ok(BrowserImportDestination {
            runtime: self.clone(),
            environment_id: binding.environment_id,
            id,
            verified: false,
            _guard: guard,
        })
    }

    /// Terminal consent only. No cookie payload, controller write or agent turn.
    pub(crate) async fn execute_browser_import_consent(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let store = &self.owned.browser_import_admission;
        let (id, status) = match request {
            LocalDaemonRequest::PrepareBrowserImport(request) => {
                let binding = self
                    .browser_import_binding(command, &request.selection)
                    .await?;
                let id = store
                    .prepare(binding, Instant::now())
                    .map_err(|_| denied())?;
                (id, BrowserImportConsentStatus::Prepared)
            }
            LocalDaemonRequest::ApproveBrowserImport(request) => {
                let binding = self
                    .browser_import_binding(command, &request.selection)
                    .await?;
                let id = ImportRequestId::from_wire(&request.request_id).map_err(|_| denied())?;
                store
                    .approve(&id, &binding, Instant::now())
                    .map_err(|_| denied())?;
                (id, BrowserImportConsentStatus::Approved)
            }
            LocalDaemonRequest::ClaimBrowserImportSource(payload)
            | LocalDaemonRequest::AuthorizeBrowserImportSource(payload) => {
                let binding = self
                    .browser_import_binding(command, &payload.selection)
                    .await?;
                let id = ImportRequestId::from_wire(&payload.request_id).map_err(|_| denied())?;
                let status = if matches!(request, LocalDaemonRequest::ClaimBrowserImportSource(_)) {
                    store
                        .claim_source(&id, &binding, Instant::now())
                        .map_err(|_| denied())?;
                    BrowserImportConsentStatus::SourceClaimed
                } else {
                    store
                        .authorize_source(&id, &binding, Instant::now())
                        .map_err(|_| denied())?;
                    BrowserImportConsentStatus::SourceAuthorized
                };
                (id, status)
            }
            LocalDaemonRequest::CancelBrowserImport(request) => {
                // Cancellation remains possible after navigation or Environment shutdown.
                let (user_id, _) = self
                    .browser_import_human(command, &request.session_id, &request.attachment_id)
                    .await?;
                let id = ImportRequestId::from_wire(&request.request_id).map_err(|_| denied())?;
                store
                    .cancel(&id, &user_id, &request.session_id)
                    .map_err(|_| denied())?;
                (id, BrowserImportConsentStatus::Cancelled)
            }
            _ => return Err(denied()),
        };
        Ok(LocalDaemonResponse::BrowserImportConsent {
            request_id: id.as_str().to_string(),
            status,
        })
    }

    async fn browser_import_binding(
        &self,
        command: &KernelCommand,
        selection: &BrowserImportSelection,
    ) -> Result<ImportBinding, DaemonError> {
        let (user_id, source_identity) = self
            .browser_import_human(command, &selection.session_id, &selection.attachment_id)
            .await?;
        let environment = self
            .room_environment_snapshot(&selection.session_id)
            .map_err(|_| denied())?;
        self.ensure_browser_import_execution_allowed(&selection.session_id)
            .map_err(|_| denied())?;
        if environment.lifecycle != EnvironmentLifecycle::Ready
            || environment.environment_id != selection.environment_id
            || environment.runtime_generation != selection.runtime_generation
            || !environment.tabs.iter().any(|tab| {
                tab.tab_id == selection.tab_id
                    && tab.document_revision == selection.document_revision
            })
        {
            return Err(denied());
        }
        Ok(ImportBinding {
            user_id,
            source_identity,
            source_attachment_id: selection.attachment_id.clone(),
            room_id: selection.session_id.clone(),
            environment_id: environment.environment_id,
            runtime_generation: environment.runtime_generation,
            tab_id: selection.tab_id.clone(),
            document_revision: selection.document_revision,
            source_store_id: selection.source_store_id.clone(),
            domains: selection.domains.clone(),
            partition_sites: selection.partition_sites.clone(),
            overwrite: selection.overwrite,
        })
    }

    async fn browser_import_human(
        &self,
        command: &KernelCommand,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(String, String), DaemonError> {
        if command.caller.metaagent_id.is_some() {
            return Err(denied());
        }
        let user_id = match (&command.source, &command.caller.caller_kind) {
            (
                KernelCommandSource::LocalCli | KernelCommandSource::LocalIpc,
                KernelCallerKind::LocalClient,
            ) => command
                .caller
                .user_id
                .as_deref()
                .unwrap_or(DEFAULT_LOCAL_USER_ID),
            (KernelCommandSource::RelayClient, KernelCallerKind::RemoteClient) => {
                if [
                    &command.caller.user_id,
                    &command.caller.client_id,
                    &command.caller.realm_id,
                    &command.caller.public_key_thumbprint,
                ]
                .iter()
                .any(|value| value.as_deref().is_none_or(str::is_empty))
                {
                    return Err(denied());
                }
                command.caller.user_id.as_deref().ok_or_else(denied)?
            }
            _ => return Err(denied()),
        };
        if !self
            .session_snapshot(session_id)
            .await
            .map_err(|_| denied())?
            .has_member(user_id)
        {
            return Err(denied());
        }
        let attachment = self
            .owned
            .ensure_attachment_in_session(session_id, attachment_id)
            .map_err(|_| denied())?;
        if attachment.owner_user_id() != user_id
            || !matches!(
                attachment.capability_level(),
                ClientCapabilityLevel::FullTerminal | ClientCapabilityLevel::InteractiveStructured
            )
            || (command.source == KernelCommandSource::RelayClient
                && (command.caller.client_id.as_deref() != Some(attachment.client_id())
                    || command.caller.caller_id != attachment.client_id()))
        {
            return Err(denied());
        }
        // This identity is constructed from verified transport context, not wire fields.
        // A different relay key/realm must obtain new consent, even for the same user.
        let identity = serde_json::to_vec(&(
            &command.source,
            user_id,
            attachment.client_id(),
            &command.caller.realm_id,
            &command.caller.public_key_thumbprint,
        ))
        .map_err(|_| denied())?;
        Ok((
            user_id.to_string(),
            format!("{:x}", Sha256::digest(identity)),
        ))
    }
}

fn denied() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "browser import consent",
        message: "browser import consent is unavailable or unauthorized".into(),
    }
}
