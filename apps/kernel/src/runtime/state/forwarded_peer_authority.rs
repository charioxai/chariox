//! Admission shared by every forwarded home tool, independent of tool name.
use super::*;

#[derive(Clone)]
pub(super) struct ForwardedPeerBinding {
    session_id: String,
    agent_id: String,
    binding: crate::agent::RemoteAgentBinding,
    prompt_id: Option<String>,
}

impl KernelRuntimeState {
    pub(crate) fn with_relay_peer_authority(
        &self,
        peer: crate::runtime::relay_peer_authority::RelayPeerAuthority,
    ) -> Self {
        let mut state = self.clone();
        state.relay_peer_authority = Some(peer);
        state
    }

    pub(crate) fn authorize_forwarded_worker(&self, worker_id: &str) -> Result<(), DaemonError> {
        self.relay_peer_authority
            .as_ref()
            .ok_or_else(|| DaemonError::RelayTransport {
                operation: "authorize forwarded peer",
                code: "unauthorized".into(),
                message: "forwarded call has no authenticated peer authority".into(),
                retryable: false,
            })?
            .authorize_worker(worker_id)
    }

    pub(crate) fn authorize_forwarded_workspace_context(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.authorize_current_forwarded_binding()?;
        let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        let remote = agent
            .remote_execution()
            .ok_or_else(|| DaemonError::RelayTransport {
                operation: "authorize forwarded context",
                code: "unauthorized".into(),
                message: "forwarded agent is not remote-backed".into(),
                retryable: false,
            })?;
        self.authorize_forwarded_worker(&remote.worker_kernel_id)?;
        let prompt_matches = context.home_prompt_id.as_deref().is_none_or(|expected| {
            self.owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent.id())
                .is_some_and(|prompt| {
                    prompt.id() == expected
                        && prompt.status() == crate::session::PromptStatus::Running
                })
        });
        if agent.session_id() != context.home_session_id
            || self.owned.config_projection.snapshot().daemon_id != context.home_kernel_id
            || remote.worker_kernel_id != context.worker_kernel_id
            || remote.worker_machine_id != context.worker_machine_id
            || remote.leased_agent_id != context.leased_agent_id
            || remote.active_worker_provider_run_id.as_deref()
                != Some(context.worker_provider_run_id.as_str())
            || !prompt_matches
        {
            return Err(DaemonError::LocalTransport {
                operation: "authorize forwarded context",
                message: "sender prompt or leased worker binding is no longer current".into(),
            });
        }
        Ok(agent)
    }

    pub(crate) fn authorize_current_forwarded_binding(&self) -> Result<(), DaemonError> {
        let Some(expected) = &self.forwarded_peer_binding else {
            return Ok(());
        };
        let agent = self.owned.agent_store.get_agent(&expected.agent_id)?;
        self.authorize_forwarded_worker(&expected.binding.worker_kernel_id)?;
        let session = self.owned.session_store.get_session(&expected.session_id)?;
        let current_prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &expected.agent_id)
            .map(|prompt| prompt.id().to_string());
        if agent.session_id() != expected.session_id
            || agent.remote_execution() != Some(&expected.binding)
            || current_prompt != expected.prompt_id
        {
            return Err(DaemonError::RelayTransport { operation: "revalidate forwarded peer", code: "unauthorized".into(), message: "forwarded worker, lease, provider run or prompt changed while awaiting approval".into(), retryable: false });
        }
        Ok(())
    }

    /// The callback must use only the admitted resource, without reentering runtime stores.
    /// Holding the stores here prevents lease, session and prompt revocation during release.
    pub(crate) fn with_forwarded_binding_operation<R>(
        &self,
        operation: impl FnOnce() -> Result<R, DaemonError>,
    ) -> Result<R, DaemonError> {
        let Some(expected) = &self.forwarded_peer_binding else {
            return operation();
        };
        self.authorize_forwarded_worker(&expected.binding.worker_kernel_id)?;
        let sessions = self.owned.session_store.read();
        let agents = self.owned.agent_store.read();
        let session = sessions.get_session(&expected.session_id)?;
        let agent = agents.get_agent(&expected.agent_id)?;
        if agent.session_id() != expected.session_id
            || agent.remote_execution() != Some(&expected.binding)
        {
            return Err(DaemonError::RelayTransport {
                operation: "revalidate forwarded resource use",
                code: "unauthorized".into(),
                message: "forwarded worker or lease changed before resource use".into(),
                retryable: false,
            });
        }
        self.owned.prompt_state_owner.with_current_prompt(
            &session,
            &expected.agent_id,
            expected.prompt_id.as_deref(),
            operation,
        )
    }

    pub(crate) async fn prepare_forwarded_peer_request(
        &self,
        request: &RelayPeerRequest,
    ) -> Result<Self, DaemonError> {
        self.with_app_side_effect(|_| {
            // Both admission and its retained binding witness share the mutation lock.
            self.authorize_forwarded_peer_request_locked(request)?;
            let resource = match request {
                RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool { context, .. }
                | RelayPeerRequest::FinalizeWorkspaceLiveSyncRuntimeTool { context, .. }
                | RelayPeerRequest::ForwardCapabilityRuntimeTool { context, .. }
                | RelayPeerRequest::ForwardMetaRuntimeTool { context, .. } => {
                    Some((&context.home_session_id, &context.home_agent_id))
                }
                RelayPeerRequest::InvokeHomeExtensionTool { context, .. }
                | RelayPeerRequest::InvokeHomeMcpProxy { context, .. }
                | RelayPeerRequest::CancelHomeExtensionInvocation { context, .. }
                | RelayPeerRequest::InvokeHomeCredentialTool { context, .. }
                | RelayPeerRequest::ResolveHomeCredentialSecret { context, .. }
                | RelayPeerRequest::ForwardRoomBrowserRuntimeTool { context, .. } => {
                    Some((&context.home_session_id, &context.home_agent_id))
                }
                RelayPeerRequest::ForwardWorkflowRuntimeTool { context, .. } => {
                    Some((&context.home_session_id, &context.home_agent_id))
                }
                _ => None,
            };
            let mut state = self.clone();
            if let Some((session_id, agent_id)) = resource {
                let agent = self.owned.agent_store.get_agent(agent_id)?;
                let binding = agent.remote_execution().cloned().ok_or_else(|| {
                    DaemonError::LocalTransport {
                        operation: "retain forwarded peer binding",
                        message: "agent is not remote-backed".into(),
                    }
                })?;
                let session = self.owned.session_store.get_session(session_id)?;
                state.forwarded_peer_binding = Some(ForwardedPeerBinding {
                    session_id: session_id.clone(),
                    agent_id: agent_id.clone(),
                    binding,
                    prompt_id: self
                        .owned
                        .prompt_state_owner
                        .active_prompt_for_agent(&session, agent_id)
                        .map(|prompt| prompt.id().to_string()),
                });
            }
            Ok(state)
        })
        .await
    }

    fn authorize_forwarded_peer_request_locked(
        &self,
        request: &RelayPeerRequest,
    ) -> Result<(), DaemonError> {
        // Binding mutation and admission use the same app mutation lane.
        match request {
            RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool { context, .. }
            | RelayPeerRequest::FinalizeWorkspaceLiveSyncRuntimeTool { context, .. }
            | RelayPeerRequest::ForwardCapabilityRuntimeTool { context, .. }
            | RelayPeerRequest::ForwardMetaRuntimeTool { context, .. } => self
                .authorize_forwarded_workspace_context(context)
                .map(|_| ()),
            RelayPeerRequest::InvokeHomeExtensionTool { context, .. }
            | RelayPeerRequest::InvokeHomeMcpProxy { context, .. }
            | RelayPeerRequest::CancelHomeExtensionInvocation { context, .. }
            | RelayPeerRequest::InvokeHomeCredentialTool { context, .. }
            | RelayPeerRequest::ResolveHomeCredentialSecret { context, .. }
            | RelayPeerRequest::ForwardRoomBrowserRuntimeTool { context, .. } => {
                super::tool_dispatch::authorize_remote_home_context_for_peer(self, context)
                    .map(|_| ())
            }
            RelayPeerRequest::ForwardWorkflowRuntimeTool { context, .. } => {
                let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
                let remote =
                    agent
                        .remote_execution()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "authorize forwarded workflow",
                            message: "workflow agent is not remote-backed".into(),
                        })?;
                self.authorize_forwarded_worker(&remote.worker_kernel_id)?;
                if agent.session_id() != context.home_session_id
                    || self.owned.config_projection.snapshot().daemon_id != context.home_kernel_id
                    || remote.active_worker_provider_run_id.is_none()
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "authorize forwarded workflow",
                        message: "workflow binding is no longer current".into(),
                    });
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
