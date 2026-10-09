use super::*;

impl KernelRuntimeState {
    pub(super) fn sudo_live(&self, turn: &KernelSudoTurn) -> bool {
        let Ok(session) = self.owned.session_store.get_session(&turn.session_id) else {
            return false;
        };
        if session.status() == SessionStatus::Ended {
            return false;
        }
        let Some(prompt_id) = turn.prompt_id.as_deref() else {
            return self.external_sudo_grant_live(turn);
        };
        self.owned.prompt_state_owner.sudo_turn_live(
            &session,
            &turn.agent_id,
            prompt_id,
            &turn.entry_id,
        ) && turn.provider_run_id.as_deref().is_some_and(|id| {
            self.owned.provider_store.get_run(id).is_ok_and(|run| {
                run.agent_instance_id() == Some(turn.agent_id.as_str())
                    && run.state() == crate::provider::ProviderRunState::Running
            })
        })
    }

    // No session or prompt locks: safe at the final sudo-store admission boundary.
    pub(super) fn external_sudo_grant_live(&self, turn: &KernelSudoTurn) -> bool {
        requester_grant_live(
            turn,
            &self
                .owned
                .kernel_access
                .lock()
                .expect("access state poisoned"),
        )
    }

    pub(crate) fn sudo_for_auth_token(&self, token: &str) -> Result<KernelSudoTurn, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(error("sudo requires one active provider run"));
        };
        self.sweep_sudo();
        let turns = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        turns
            .into_iter()
            .find(|turn| turn.provider_run_id.as_deref() == Some(run.id()) && self.sudo_live(turn))
            .ok_or_else(|| error("this provider turn has no sudo authority"))
    }

    pub(crate) fn authorize_sudo_request(
        &self,
        id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<String, DaemonError> {
        let turn = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get(id)
            .cloned()
            .filter(|turn| turn.prompt_id.is_some())
            .ok_or_else(|| error("sudo turn ended or was revoked"))?;
        if !self.sudo_live(&turn) {
            return Err(error("sudo turn ended or was revoked"));
        }
        if sudo_request_forbidden(request) {
            return Err(error("sudo cannot grant authority, read secrets, change the passkey or access configuration"));
        }
        if let LocalDaemonRequest::RespondToInteraction(answer) = request {
            let pending = self.owned.pending_interactions.write();
            if pending.get(&answer.interaction_id).is_none_or(|pending| {
                pending.terminal_credential_owner.is_some()
                    || pending
                        .passkey_prompt
                        .as_ref()
                        .is_some_and(|prompt| prompt.kind != PasskeyPromptKind::CriticalApproval)
            }) {
                return Err(error(
                    "sudo cannot answer access, sudo or credential prompts",
                ));
            }
        }
        Ok(turn.session_id.clone())
    }
}

fn sudo_request_forbidden(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::RequestKernelSudo(_)
        | LocalDaemonRequest::RequestKernelAccess(_)
            | LocalDaemonRequest::ListKernelAccessGrants(_)
            | LocalDaemonRequest::RevokeKernelAccessGrant(_)
            | LocalDaemonRequest::ManageCredentialVault(_)
            | LocalDaemonRequest::GetCredential(_)
            | LocalDaemonRequest::ListCredentials(_)
            // Sudo must not serialize literal MCP env/header credentials either.
            | LocalDaemonRequest::GetMcpServer(_)
            | LocalDaemonRequest::ListMcpServers(_)
            | LocalDaemonRequest::ImportMcpServers(_)
            // Pairing and Cloud identity can outlive the authorizing turn or
            // return relay credentials. They remain host-terminal operations.
            | LocalDaemonRequest::CreatePairingInvite(_)
            | LocalDaemonRequest::JoinPairingInvite(_)
            | LocalDaemonRequest::CreateTerminalPairingLink(_)
            | LocalDaemonRequest::JoinTerminalPairingLink(_)
            | LocalDaemonRequest::RecordPairedClient(_)
            | LocalDaemonRequest::ApproveRemoteMachine(_)
            | LocalDaemonRequest::CreateSessionInvite(_)
            | LocalDaemonRequest::JoinSessionInvite(_)
            | LocalDaemonRequest::CreateCloudSessionInvite(_)
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
            | LocalDaemonRequest::ExportDebugBundle(_)
            | LocalDaemonRequest::SetProviderAccountCredential(_)
            | LocalDaemonRequest::GetProviderAccountProfile(_)
            | LocalDaemonRequest::ImportNativeProviderAccountProfile(_)
    ) || matches!(request, LocalDaemonRequest::SetUserConfigValue(config) if sudo_config_forbidden(&config.path))
        || matches!(request, LocalDaemonRequest::UnsetUserConfigValue(config) if sudo_config_forbidden(&config.path))
        || matches!(request, LocalDaemonRequest::RespondToInteraction(answer) if answer.passkey.is_some() || answer.passkey_remember_minutes.is_some())
        || matches!(request, LocalDaemonRequest::SubmitPrompt(prompt) if is_sudo_prompt(&prompt.prompt))
        || matches!(request, LocalDaemonRequest::SubmitPrompts(prompts) if prompts.prompts.iter().any(|prompt| is_sudo_prompt(&prompt.prompt)))
}

pub(crate) fn is_sudo_prompt(prompt: &str) -> bool {
    parse_sudo_prompt(prompt).is_some()
}

pub(super) fn parse_sudo_prompt(prompt: &str) -> Option<&str> {
    let rest = prompt.trim_start().strip_prefix("/sudo")?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

fn sudo_config_forbidden(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    ["kernel_access", "credential_vault", "relay"]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}.")))
}

pub(super) fn requester_grant_live(
    turn: &KernelSudoTurn,
    access: &crate::runtime::kernel_access::AccessState,
) -> bool {
    turn.requester.as_ref().is_none_or(|requester| {
        access.grants.get(&requester.grant_id).is_some_and(|grant| {
            grant.summary.owner_user_id == turn.owner_user_id
                && std::time::Instant::now() < grant.deadline
                && grant.holder.alive()
        })
    })
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn sudo_refuses_raw_registry_credentials_and_provider_imports() {
        for request in [
            LocalDaemonRequest::ListCredentials(crate::local::ListCredentialsRequest),
            LocalDaemonRequest::GetMcpServer(crate::local::GetMcpServerRequest {
                workspace_id: None,
                name: "literal-secret".into(),
            }),
            LocalDaemonRequest::ListMcpServers(crate::local::ListMcpServersRequest {
                workspace_id: None,
            }),
            LocalDaemonRequest::ImportMcpServers(crate::local::ImportMcpServersRequest {
                workspace_id: None,
                provider: "codex".into(),
                name: None,
            }),
        ] {
            assert!(sudo_request_forbidden(&request));
        }
    }
}
