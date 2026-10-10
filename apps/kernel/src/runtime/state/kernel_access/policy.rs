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
        let forbidden = matches!(
            request,
            LocalDaemonRequest::RequestKernelAccess(_)
                // Approvals belong to the user, including routine ones.
                | LocalDaemonRequest::RespondToInteraction(_)
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
        ) || matches!(request, LocalDaemonRequest::SetUserConfigValue(config)
            if protected_config(&config.path))
            || matches!(request, LocalDaemonRequest::UnsetUserConfigValue(config)
                if protected_config(&config.path));
        if forbidden {
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
                    serde_json::json!({ "owner_user_id": grant.summary.owner_user_id, "suppressed": state.suppressed_denials }));
                state.last_denial = Some(now);
                state.suppressed_denials = 0;
            } else {
                state.suppressed_denials += 1;
            }
            return Err(error(
                "external access cannot answer approvals, change authority, disclose secrets or reach a remote kernel",
            ));
        }
        Ok(())
    }
}

fn protected_config(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    ["kernel_access", "credential_vault", "relay"]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}.")))
}
