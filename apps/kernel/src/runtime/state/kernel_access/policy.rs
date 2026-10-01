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

    /// Global requests fail closed. SessionRef and attachment scopes resolve through kernel state.
    pub(crate) fn authorize_external_request(
        &self,
        grant_id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<String, DaemonError> {
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
        let permitted = match request_session_scope(request) {
            Some(SessionMembershipScope::SessionId(id)) => id == *session_id,
            Some(SessionMembershipScope::SessionIds(ids)) => {
                !ids.is_empty() && ids.iter().all(|id| id == session_id)
            }
            Some(SessionMembershipScope::AllSessions) => {
                matches!(request, LocalDaemonRequest::ListSessions(_))
            }
            Some(SessionMembershipScope::SessionRef {
                session_ref,
                workspace_id,
            }) => self.access_session(session_id).is_ok_and(|session| {
                (session_ref == *session_id || session.alias() == Some(&session_ref))
                    && workspace_id
                        .as_deref()
                        .is_none_or(|id| id == session.workspace_id())
            }),
            Some(SessionMembershipScope::AttachmentId(id)) => {
                self.owned
                    .session_projection
                    .session_id_for_attachment(&id)
                    .as_ref()
                    == Some(session_id)
            }
            None => false,
        };
        let forbidden = matches!(
            request,
            LocalDaemonRequest::RespondToInteraction(_)
                | LocalDaemonRequest::RequestNativeProviderInteraction(_)
                | LocalDaemonRequest::ArmDeploymentCredentialEnrollment(_)
                | LocalDaemonRequest::ExportDebugBundle(_)
                | LocalDaemonRequest::ImportExternalProviderAgent(_)
        );
        // Responding to routine agent questions is allowed; kernel decisions, even denials, remain human owned.
        let forbidden = if let LocalDaemonRequest::RespondToInteraction(response) = request {
            response.passkey.is_some()
                || response.passkey_remember_minutes.is_some()
                || self
                    .owned
                    .pending_interactions
                    .write()
                    .get(&response.interaction_id)
                    .is_none_or(|pending| pending.kernel_operation_owner.is_some())
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
        Ok(grant.summary.owner_user_id)
    }
}
