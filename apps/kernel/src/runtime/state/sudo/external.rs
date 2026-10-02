//! Grant holders request authorization; only the host terminal can supply it.
use super::*;

impl KernelRuntimeState {
    pub(crate) async fn request_kernel_sudo(
        &self,
        grant_id: &str,
        request: RequestKernelSudoRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = self.authorize_external_request(
            grant_id,
            &LocalDaemonRequest::RequestKernelSudo(request.clone()),
        )?;
        let grant = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .get(grant_id)
            .cloned()
            .ok_or_else(|| error("grant revoked or expired"))?;
        let session = self.owned.session_store.get_session(&session_id)?;
        if session.owner_user_id() != grant.summary.owner_user_id {
            return Err(error("only the session host can authorize sudo"));
        }
        if request.prompt.trim().is_empty() {
            return Err(error("sudo request needs a prompt"));
        }
        // A private source attachment uses ordinary prompt admission. It does
        // not represent a terminal and is removed on success, refusal or drop.
        let attachment = self.owned.attachment_store.attach(
            &mut self.owned.session_store.write(),
            crate::attachment::AttachRequest::for_user(
                &session_id,
                format!("external-sudo:{:016x}", rand::random::<u64>()),
                crate::attachment::ClientCapabilityLevel::AutomationOnly,
                &grant.summary.owner_user_id,
            ),
        )?;
        let _attachment = ExternalSudoAttachment {
            state: self.clone(),
            id: attachment.id().into(),
        };
        let owner = grant.summary.owner_user_id.clone();
        self.submit_sudo_entry(
            SubmitPromptRequest {
                session_id,
                attachment_id: attachment.id().into(),
                target_agent_id: Some(request.agent_id.clone()),
                prompt: format!("/sudo {}", request.prompt),
                attachments: vec![],
            },
            &owner,
            "",
            Some(grant.summary),
        )
        .await?;
        Ok(LocalDaemonResponse::KernelSudoRequested {
            agent_id: request.agent_id,
        })
    }

    pub(crate) fn sudo_entry_pending(&self, session_id: &str, entry_id: &str) -> bool {
        self.owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get(entry_id)
            .is_some_and(|turn| turn.session_id == session_id && turn.prompt_id.is_none())
    }

    pub(crate) async fn answer_sudo_entry_from_terminal(
        &self,
        owner: &str,
        terminal: &str,
        answer: &RespondToInteractionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let authorization = self
            .authorize_critical_approval(
                &answer.session_id,
                &answer.interaction_id,
                &answer.choice_id,
                Some(owner),
                answer.passkey.as_ref(),
                answer.passkey_remember_minutes,
                Some(KernelConnectionClass::Terminal),
            )
            .await?;
        self.owned
            .resolve_runtime_interaction_authorized(
                &answer.session_id,
                &answer.interaction_id,
                &answer.choice_id,
                answer.custom_reply.as_deref(),
                Some(owner),
                authorization.verified,
                None,
                Some(terminal),
            )
            .map_err(|error| {
                self.owned.closed_interaction_error(
                    &answer.session_id,
                    &answer.interaction_id,
                    error,
                )
            })?;
        Ok(LocalDaemonResponse::InteractionResponded {
            interaction_id: answer.interaction_id.clone(),
            session: self.session_snapshot(&answer.session_id).await?,
        })
    }
}

struct ExternalSudoAttachment {
    state: KernelRuntimeState,
    id: String,
}
impl Drop for ExternalSudoAttachment {
    fn drop(&mut self) {
        let detached = self
            .state
            .owned
            .attachment_store
            .detach(&mut self.state.owned.session_store.write(), &self.id);
        if let Ok(attachment) = detached {
            let _ = self.state.owned.session_snapshot(attachment.session_id());
        }
    }
}
