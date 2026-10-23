//! Grant holders request authorization; only the host terminal can supply it.
use super::*;
use crate::runtime::kernel_access::{RemoteRequester, REMOTE_REQUESTER_LEASE};

impl KernelRuntimeState {
    pub(crate) async fn request_kernel_sudo(
        &self,
        grant_id: &str,
        request: RequestKernelSudoRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_external_request(
            grant_id,
            &LocalDaemonRequest::RequestKernelSudo(request.clone()),
        )?;
        let session_id = self
            .owned
            .agent_store
            .get_agent(&request.agent_id)?
            .session_id()
            .to_owned();
        let grant = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .get(grant_id)
            .cloned()
            .ok_or_else(|| error("grant revoked or expired"))?;
        let owner = grant.summary.owner_user_id.clone();
        self.raise_sudo_request(session_id, request, &owner, grant.summary)
            .await
    }

    /// Raises the popup for `request` on the named agent's session. A private source
    /// attachment uses ordinary prompt admission. It does not represent a terminal and
    /// is removed on success, refusal or drop.
    async fn raise_sudo_request(
        &self,
        session_id: String,
        request: RequestKernelSudoRequest,
        owner: &str,
        requester: KernelAccessGrant,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if request.prompt.trim().is_empty() {
            return Err(error("sudo request needs a prompt"));
        }
        let attachment = self.attach_external_sudo_source(&session_id, owner)?;
        let _attachment = ExternalSudoAttachment {
            state: self.clone(),
            id: attachment.id().into(),
        };
        self.submit_sudo_entry(
            SubmitPromptRequest {
                session_id,
                attachment_id: attachment.id().into(),
                target_agent_id: Some(request.agent_id.clone()),
                prompt: request.prompt.clone(),
                attachments: vec![],
            },
            &request.prompt,
            owner,
            "",
            Some(requester),
        )
        .await?;
        Ok(LocalDaemonResponse::KernelSudoRequested {
            agent_id: request.agent_id,
        })
    }

    /// A sudo request another kernel's gateway forwarded for its agent. Same window
    /// and binding as a local request; only who asked differs, and the origin kernel
    /// (not a local grant) keeps the requester alive until it reports its grant ended.
    pub(crate) async fn request_remote_kernel_sudo(
        &self,
        on_behalf_of: &OnBehalfOf,
        request: RequestKernelSudoRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = self
            .owned
            .agent_store
            .get_agent(&request.agent_id)?
            .session_id()
            .to_owned();
        let owner = self.owned.config_projection.local_owner_user_id();
        let requester = remote_requester_grant(on_behalf_of, &owner);
        self.owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .remote_requesters
            .insert(
                requester.grant_id.clone(),
                RemoteRequester {
                    origin_name: on_behalf_of
                        .origin_name
                        .clone()
                        .unwrap_or_else(|| on_behalf_of.origin_kernel.clone()),
                    harness: on_behalf_of.harness.clone(),
                    lease_until: std::time::Instant::now() + REMOTE_REQUESTER_LEASE,
                },
            );
        self.raise_sudo_request(session_id, request, &owner, requester)
            .await
    }

    /// The origin kernel's grant ended: its pending requests and running windows end
    /// here through the same sweep a local grant's end takes.
    pub(crate) fn renew_remote_kernel_sudo(&self, on_behalf_of: &OnBehalfOf) -> bool {
        let now = std::time::Instant::now();
        let renewed = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .remote_requesters
            .get_mut(&remote_requester_id(on_behalf_of))
            .filter(|remote| remote.live(now))
            .map(|remote| remote.lease_until = now + REMOTE_REQUESTER_LEASE)
            .is_some();
        self.sweep_sudo();
        renewed
    }

    pub(crate) fn end_remote_kernel_sudo(&self, on_behalf_of: &OnBehalfOf) -> usize {
        let ended = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .remote_requesters
            .remove(&remote_requester_id(on_behalf_of))
            .is_some();
        self.sweep_sudo();
        usize::from(ended)
    }

    pub(crate) async fn submit_external_sudo_prompt(
        &self,
        grant_id: &str,
        request: SubmitPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_external_request(
            grant_id,
            &LocalDaemonRequest::SubmitPrompt(request.clone()),
        )?;
        let grant = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .get(grant_id)
            .map(|grant| grant.summary.clone())
            .ok_or_else(|| error("grant revoked or expired"))?;
        let owner = grant.owner_user_id.clone();
        let prompt = policy::parse_sudo_prompt(&request.prompt)
            .unwrap_or_default()
            .to_owned();
        self.submit_sudo_entry(request, &prompt, &owner, "", Some(grant))
            .await
    }

    pub(super) fn attach_external_sudo_source(
        &self,
        session_id: &str,
        owner: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        // Attach ordinarily reopens ended sessions. External requests must never
        // do that: check status and host ownership under the same writer used
        // by attach, so session teardown cannot race this check.
        let mut sessions = self.owned.session_store.write();
        let session = sessions.get_session(session_id)?;
        if session.status() == SessionStatus::Ended {
            return Err(error("session has ended"));
        }
        if session.owner_user_id() != owner {
            return Err(super::entry::not_session_host(session.owner_user_id()));
        }
        self.owned.attachment_store.attach(
            &mut sessions,
            crate::attachment::AttachRequest::for_user(
                session_id,
                format!("external-sudo:{:016x}", rand::random::<u64>()),
                crate::attachment::ClientCapabilityLevel::AutomationOnly,
                owner,
            ),
        )
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
                Some(terminal),
                false,
                true,
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

fn remote_requester_id(on_behalf_of: &OnBehalfOf) -> String {
    format!("{}:{}", on_behalf_of.origin_kernel, on_behalf_of.grant_id)
}

/// The requester a sudo entry carries for a request from another kernel: no OS
/// process here, so the holder facts are empty and the id names origin and grant.
fn remote_requester_grant(on_behalf_of: &OnBehalfOf, owner: &str) -> KernelAccessGrant {
    KernelAccessGrant {
        grant_id: remote_requester_id(on_behalf_of),
        owner_user_id: owner.into(),
        holder_pid: 0,
        holder_executable: on_behalf_of.harness.clone(),
        holder_harness: None,
        term: KernelAccessTerm::UntilHolderExits,
        expires_at_ms: None,
    }
}

pub(super) fn remote_requester_projection(remote: &RemoteRequester) -> KernelAccessRequester {
    let harness = remote.harness.to_ascii_lowercase();
    KernelAccessRequester {
        executable: remote.harness.clone(),
        pid: 0,
        process_start_id: "0".into(),
        process_exec_version: 0,
        provider_harness: [
            ("claude", KernelAccessProviderHarness::Claude),
            ("codex", KernelAccessProviderHarness::Codex),
            ("opencode", KernelAccessProviderHarness::Opencode),
        ]
        .into_iter()
        .find_map(|(name, kind)| harness.contains(name).then_some(kind)),
    }
}
