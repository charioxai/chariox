use super::*;

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
                        PasskeyPromptKind::AccessGrant
                            | PasskeyPromptKind::AccessExtension
                            | PasskeyPromptKind::Sudo
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

    /// Every ordinary request on this LOCAL kernel; no grant-specific session scope.
    pub(crate) fn authorize_external_request(
        &self,
        grant_id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        if grant_id.starts_with("sudo:") {
            return self.authorize_sudo_request(grant_id, request).map(|_| ());
        }
        if carries_passkey(request) {
            return Err(Refusal::HumanOnly.error());
        }
        let owner = self.grant_owner(grant_id)?;
        let forbidden = restricted_request(request)
            || matches!(request, LocalDaemonRequest::RespondToInteraction(answer)
                if self.interaction_requires_terminal(answer));
        if forbidden {
            self.audit_human_only_denial(grant_id, &owner);
            return Err(Refusal::HumanOnly.error());
        }
        Ok(())
    }

    /// Origin half of a forwarded request: the grant is live and the request
    /// shape is allowed. The interaction an answer belongs to is owned by the
    /// target kernel, which classifies it itself (`authorize_on_behalf_of_request`).
    pub(crate) fn authorize_gateway_request(
        &self,
        grant_id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        if grant_id.starts_with("sudo:") {
            return Err(error("a sudo window does not reach other kernels"));
        }
        if carries_passkey(request) {
            return Err(Refusal::HumanOnly.error());
        }
        let owner = self.grant_owner(grant_id)?;
        if restricted_request(request) {
            self.audit_human_only_denial(grant_id, &owner);
            return Err(Refusal::HumanOnly.error());
        }
        Ok(())
    }

    /// Target half of a forwarded request: the same shape check as the origin,
    /// plus the target's own classification of the interaction being answered.
    /// The origin's grant is not looked up here: it lives on the origin kernel.
    pub(crate) fn authorize_on_behalf_of_request(
        &self,
        on_behalf_of: &OnBehalfOf,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        let forbidden = carries_passkey(request)
            || restricted_request(request)
            || matches!(request, LocalDaemonRequest::RespondToInteraction(answer)
                if self.interaction_requires_terminal(answer));
        if forbidden {
            self.audit_human_only_denial(
                &format!("{}:{}", on_behalf_of.origin_kernel, on_behalf_of.grant_id),
                &on_behalf_of.harness,
            );
            return Err(Refusal::HumanOnly.error());
        }
        Ok(())
    }

    /// Sweeps ended grants, then names the grant's owner; an ended grant is an error.
    fn grant_owner(&self, grant_id: &str) -> Result<String, DaemonError> {
        self.sweep_kernel_access();
        self.owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .get(grant_id)
            .map(|grant| grant.summary.owner_user_id.clone())
            .ok_or_else(|| error("grant revoked or expired"))
    }

    /// A state-changing request arrived from another kernel's gateway for one of its
    /// agents. Only who and what kind, never the payload.
    pub(crate) fn audit_gateway_request(&self, on_behalf_of: &OnBehalfOf, request_kind: &str) {
        let _ = self.owned.durable_state_store.append_event(
            "kernel_gateway.request",
            Some(format!(
                "{}:{}",
                on_behalf_of.origin_kernel, on_behalf_of.grant_id
            )),
            serde_json::json!({
                "origin_kernel": on_behalf_of.origin_kernel,
                "harness": on_behalf_of.harness,
                "request": request_kind,
            }),
        );
    }

    /// A state-changing request this kernel's agent sent to another kernel.
    pub(crate) fn audit_gateway_forward(
        &self,
        grant_id: &str,
        kernel: &str,
        request_kind: &str,
        outcome: &str,
        latency_ms: u64,
    ) {
        let _ = self.owned.durable_state_store.append_event(
            "kernel_access.forward",
            Some(grant_id.to_string()),
            serde_json::json!({
                "kernel": kernel,
                "request": request_kind,
                "outcome": outcome,
                "latency_ms": latency_ms,
            }),
        );
    }

    fn audit_human_only_denial(&self, subject: &str, owner: &str) {
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
            let _ = self.owned.durable_state_store.append_event("kernel_access.denied", Some(subject.into()),
                serde_json::json!({ "owner_user_id": owner, "suppressed": state.suppressed_denials }));
            state.last_denial = Some(now);
            state.suppressed_denials = 0;
        } else {
            state.suppressed_denials += 1;
        }
    }

    // MP-08/MP-11: ordinary grant holders may answer routine Room questions.
    // Critical and credential decisions retain the terminal boundary for every
    // choice, including refusal, without exposing interaction metadata.
    pub(crate) fn interaction_requires_terminal(
        &self,
        answer: &RespondToInteractionRequest,
    ) -> bool {
        let pending = self
            .owned
            .pending_interactions
            .write()
            .get(&answer.interaction_id)
            .cloned();
        let Some(pending) = pending else { return false };
        if pending.passkey_prompt.is_some() || pending.terminal_credential_owner.is_some() {
            return true;
        }
        let interaction = pending
            .kernel_wide_interaction
            .or(pending.user_domain_interaction)
            .or_else(|| {
                self.owned
                    .session_store
                    .get_session(&answer.session_id)
                    .ok()?
                    .active_interactions()
                    .iter()
                    .find(|interaction| interaction.id() == answer.interaction_id)
                    .cloned()
            });
        interaction.is_some_and(|interaction| {
            interaction.level() == crate::session::RuntimeInteractionLevel::Critical
                || interaction
                    .choices()
                    .iter()
                    .any(|choice| choice.requires_passkey())
        })
    }
}

fn protected_config(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    ["kernel_access", "credential_vault", "relay"]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}.")))
}

fn carries_passkey(request: &LocalDaemonRequest) -> bool {
    matches!(request, LocalDaemonRequest::RespondToInteraction(answer) if answer.passkey.is_some())
}

/// Request shapes a grant holder never sends, locally or through the gateway:
/// credentials, pairing, relay and Cloud identity, remote admission, protected
/// configuration. It reads no state, so the origin and the target call the same function.
pub(crate) fn restricted_request(request: &LocalDaemonRequest) -> bool {
    carries_passkey(request)
        || matches!(
            request,
            LocalDaemonRequest::RequestKernelAccess(_)
                | LocalDaemonRequest::ManageCredentialVault(_)
                // Raw credential configs can contain literal injection headers.
                | LocalDaemonRequest::GetCredential(_)
                | LocalDaemonRequest::ListCredentials(_)
                // Registry reads and imports return literal env/header credentials.
                | LocalDaemonRequest::GetMcpServer(_)
                | LocalDaemonRequest::ListMcpServers(_)
                | LocalDaemonRequest::ImportMcpServers(_)
                // These expose remote admission credentials or connect to another kernel.
                | LocalDaemonRequest::CreatePairingInvite(_)
                | LocalDaemonRequest::JoinPairingInvite(_)
                | LocalDaemonRequest::CreateTerminalPairingLink(_)
                | LocalDaemonRequest::JoinTerminalPairingLink(_)
                | LocalDaemonRequest::JoinKernelLink(_)
                | LocalDaemonRequest::ForgetGatewayKernel(_)
                | LocalDaemonRequest::RecordPairedClient(_)
                | LocalDaemonRequest::ApproveRemoteMachine(_)
                | LocalDaemonRequest::JoinSessionInvite(_)
                | LocalDaemonRequest::CreateCloudSessionInvite(_)
                | LocalDaemonRequest::ShowCloudSessionInvite(_)
                | LocalDaemonRequest::AcceptCloudSessionInvite(_)
                | LocalDaemonRequest::ConfigureRelay(_)
                | LocalDaemonRequest::CloudRelayStatus(_)
                | LocalDaemonRequest::StartCloudRelayLogin(_)
                | LocalDaemonRequest::PollCloudRelayLogin(_)
                | LocalDaemonRequest::LogoutCloudRelay(_)
                | LocalDaemonRequest::PairCloudRelayClient(_)
                | LocalDaemonRequest::PairCloudRelayMachine(_)
                | LocalDaemonRequest::ConnectCloudRelay(_)
                | LocalDaemonRequest::IssueCloudRelayClientToken(_)
                | LocalDaemonRequest::ResolveKernelClientConnection(_)
        )
        || matches!(request, LocalDaemonRequest::SetUserConfigValue(config)
            if protected_config(&config.path))
        || matches!(request, LocalDaemonRequest::UnsetUserConfigValue(config)
            if protected_config(&config.path))
}
