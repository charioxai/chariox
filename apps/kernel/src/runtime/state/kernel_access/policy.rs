use super::*;
use crate::runtime::session_membership::scope::{request_session_scope, SessionMembershipScope};

impl KernelRuntimeState {
    pub(crate) fn audit_access_terminal_attempt(
        &self,
        terminal: &str,
        user: &str,
        command_id: &str,
        answer: &RespondToInteractionRequest,
    ) -> Result<(), DaemonError> {
        let pending = self
            .owned
            .pending_interactions
            .write()
            .get(&answer.interaction_id)
            .cloned();
        let Some(pending) = pending.filter(|p| {
            p.session_id == answer.session_id
                && p.kernel_operation_owner.as_deref() == Some(user)
                && p.passkey_prompt.as_ref().is_some_and(|p| {
                    matches!(
                        p.kind,
                        PasskeyPromptKind::AccessGrant | PasskeyPromptKind::AccessExtension
                    )
                })
        }) else {
            return Ok(());
        };
        let prompt = pending.passkey_prompt.as_ref().expect("access prompt");
        self.owned.durable_state_store.append_event("kernel_access.terminal_answer", Some(command_id.into()),
            serde_json::json!({"terminal_id":terminal,"user_id":user,"connection_class":KernelConnectionClass::Terminal,
                "interaction_id":prompt.interaction_id,"choice_id":answer.choice_id,"outcome":"submitted"}))?;
        // critical_approval.passkey records verification under this same interaction id.
        Ok(())
    }

    pub(crate) fn external_request_in_session(
        &self,
        session_id: &str,
        request: &LocalDaemonRequest,
    ) -> bool {
        match request_session_scope(request) {
            Some(SessionMembershipScope::SessionId(id)) => id == session_id,
            Some(SessionMembershipScope::SessionIds(ids)) => {
                !ids.is_empty() && ids.iter().all(|id| id == session_id)
            }
            Some(SessionMembershipScope::AllSessions) => {
                matches!(request, LocalDaemonRequest::ListSessions(_))
            }
            Some(SessionMembershipScope::SessionRef {
                session_ref,
                workspace_id,
            }) => self
                .owned
                .session_store
                .read()
                .resolve_session_ref(&session_ref, workspace_id.as_deref())
                .is_ok_and(|session| session.id() == session_id),
            Some(SessionMembershipScope::AttachmentId(id)) => {
                self.owned
                    .session_projection
                    .session_id_for_attachment(&id)
                    .as_deref()
                    == Some(session_id)
            }
            None => false,
        }
    }

    /// Return the authorized session. Global requests fail closed; references
    /// and attachments resolve through kernel state.
    pub(crate) fn authorize_external_request(
        &self,
        grant_id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<String, DaemonError> {
        if grant_id.starts_with("sudo:") {
            return self.authorize_sudo_request(grant_id, request);
        }
        if matches!(request, LocalDaemonRequest::RespondToInteraction(answer) if answer.passkey.is_some())
        {
            return Err(DaemonError::LocalTransport {
                operation: "critical approval",
                message: "PASSKEY_NOT_ACCEPTED: only a Chariox terminal can submit the passkey"
                    .into(),
            });
        }
        self.sweep_kernel_access();
        let grant = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .get(grant_id)
            .cloned()
            .ok_or_else(|| error("grant revoked or expired"))?;
        let session_id = &grant.summary.session_id;
        let permitted = self.external_request_in_session(session_id, request);
        let forbidden = matches!(
            request,
            LocalDaemonRequest::RespondToInteraction(_)
                | LocalDaemonRequest::RequestNativeProviderTurnInteraction(_)
                | LocalDaemonRequest::ArmDeploymentCredentialEnrollment(_)
                | LocalDaemonRequest::ExportDebugBundle(_)
                | LocalDaemonRequest::ImportExternalProviderAgent(_)
                // Saved artifacts are kernel/user resources, not session-owned.
                | LocalDaemonRequest::CreateWorkflowCodeArtifact(_)
                | LocalDaemonRequest::UpdateWorkflowCodeArtifact(_)
                | LocalDaemonRequest::GetWorkflowCodeArtifact(_)
                | LocalDaemonRequest::ListWorkflowCodeArtifacts(_)
                | LocalDaemonRequest::DeleteWorkflowCodeArtifact(_)
                | LocalDaemonRequest::ExportWorkflowCodeArtifact(_)
                | LocalDaemonRequest::ImportWorkflowCodeArtifact(_)
                | LocalDaemonRequest::ImportWorkflowCodePackage(_)
        ) || matches!(request, LocalDaemonRequest::ExportWorkflowCodePackage(request)
            if !matches!(request.target, Some(crate::local::WorkflowCodePackageExportTarget::Workflow { .. })))
            || matches!(request, LocalDaemonRequest::ExportWorkflowCodeSource(request)
                if matches!(request.target, crate::local::WorkflowCodeSourceExportTarget::Artifact { .. }));
        // Responding to routine agent questions is allowed; kernel decisions, even denials, remain human owned.
        let forbidden = if let LocalDaemonRequest::RespondToInteraction(response) = request {
            response.passkey.is_some()
                || response.passkey_remember_minutes.is_some()
                || self
                    .owned
                    .pending_interactions
                    .write()
                    .get(&response.interaction_id)
                    .is_none_or(|pending| {
                        pending.kernel_operation_owner.is_some()
                            || pending.terminal_credential_owner.is_some()
                    })
        } else {
            forbidden
        };
        if !permitted || forbidden {
            let mut state = self
                .owned
                .kernel_access
                .lock()
                .expect("access state poisoned");
            let now = Instant::now();
            if state
                .last_denial
                .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(60))
            {
                let _ = self.owned.durable_state_store.append_event("kernel_access.denied", Some(grant_id.into()),
                    serde_json::json!({ "session_id": session_id, "suppressed": state.suppressed_denials }));
                state.last_denial = Some(now);
                state.suppressed_denials = 0;
            } else {
                state.suppressed_denials += 1;
            }
            return Err(error(
                "request is outside this external agent's session authority",
            ));
        }
        Ok(session_id.clone())
    }
}
