use super::*;

impl KernelRuntimeOwnedState {
    /// Why a started window is no longer live, or `None` while it is. Pending
    /// authorizations (no prompt yet) only depend on the session and requester.
    pub(super) fn sudo_end_reason(&self, turn: &KernelSudoTurn) -> Option<&'static str> {
        let session = match self.session_store.get_session(&turn.session_id) {
            Ok(session) if session.status() != SessionStatus::Ended => session,
            _ => return Some("session_ended"),
        };
        if !requester_grant_live(
            turn,
            &self.kernel_access.lock().expect("access state poisoned"),
        ) {
            return Some("requester_ended");
        }
        if turn
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Some("expired");
        }
        let prompt_id = turn.prompt_id.as_deref()?;
        if turn.deadline.is_none() {
            return Some("expired");
        }
        if self
            .agent_store
            .get_agent(&turn.agent_id)
            .ok()
            .is_none_or(|agent| {
                agent.session_id() != turn.session_id
                    || agent
                        .remote_execution()
                        .map(|remote| &remote.execution_lease_id)
                        != turn.placement.as_ref()
            })
        {
            return Some("placement_changed");
        }
        let task = turn.task_id.as_deref().and_then(|id| {
            self.durable_state_store
                .agent_tasks(Some(&turn.session_id), Some(&turn.agent_id))
                .ok()?
                .into_iter()
                .find(|task| task.task_id == id)
        });
        let work_open = match task {
            Some(task) => !matches!(
                task.state,
                crate::durable_state::agent_lifecycle::ExecutionState::Done
                    | crate::durable_state::agent_lifecycle::ExecutionState::Cancelled
            ),
            // Without durable tasks the authorized work is the first turn.
            None => self
                .prompt_state_owner
                .sudo_bound_prompt(&session, &turn.agent_id)
                .is_some_and(|(entry, prompt)| entry == turn.entry_id && prompt == prompt_id),
        };
        (!work_open).then_some("work_ended")
    }
}

impl KernelRuntimeState {
    pub(super) fn sudo_live(&self, turn: &KernelSudoTurn) -> bool {
        self.owned.sudo_end_reason(turn).is_none()
    }

    /// The live window bound to `run`'s current turn. The provider bearer never
    /// changes: each use resolves the exact agent, running turn and window.
    pub(super) fn sudo_for_run(&self, run_id: &str) -> Result<KernelSudoTurn, DaemonError> {
        let denied = || error("this provider turn has no sudo authority");
        let (session_id, agent) = match self.leased_agent_for_projected_run(run_id) {
            // A10: a leased turn is the home agent's current worker run.
            Some(agent) => (agent.session_id().to_string(), agent.id().to_string()),
            None => {
                let run = self
                    .owned
                    .provider_store
                    .get_run(run_id)
                    .map_err(|_| denied())?;
                let agent = run.agent_instance_id().ok_or_else(denied)?;
                if run.state() != crate::provider::ProviderRunState::Running
                    || self
                        .owned
                        .provider_store
                        .get_run_for_agent(run.session_id(), agent)
                        .is_none_or(|current| current.id() != run_id)
                {
                    return Err(denied());
                }
                (run.session_id().to_string(), agent.to_string())
            }
        };
        let agent = agent.as_str();
        self.sweep_sudo();
        let session = self.owned.session_store.get_session(&session_id)?;
        let (entry, prompt) = self
            .owned
            .prompt_state_owner
            .sudo_bound_prompt(&session, agent)
            .ok_or_else(denied)?;
        let mut turn = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get(&entry)
            .cloned()
            .filter(|turn| turn.agent_id == agent && turn.session_id == session_id)
            .ok_or_else(denied)?;
        if !self.sudo_live(&turn) {
            return Err(denied());
        }
        turn.prompt_id = Some(prompt);
        turn.provider_run_id = Some(run_id.into());
        Ok(turn)
    }

    pub(crate) fn sudo_for_auth_token(&self, token: &str) -> Result<KernelSudoTurn, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(error("sudo requires one active provider run"));
        };
        self.sudo_for_run(run.id())
    }

    /// Lists the sudo tool for the whole live window, so waits and wakes do
    /// not churn the provider catalog; each call still needs a bound turn.
    pub(crate) fn sudo_window_open_for_auth_token(&self, token: &str) -> bool {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return false;
        };
        let Some(agent) = run.agent_instance_id() else {
            return false;
        };
        if self.worker_sudo_grant_open(agent) {
            return true;
        }
        let windows = self.owned.sudo_windows_for_session(run.session_id());
        windows
            .iter()
            .any(|turn| turn.agent_id == agent && self.sudo_live(turn))
    }

    pub(in crate::runtime::state) fn sudo_for_provider_run(
        &self,
        run_id: &str,
    ) -> Result<KernelSudoTurn, DaemonError> {
        self.sudo_for_run(run_id)
    }

    /// Privileged admission and the pre-effect recheck: the same live window
    /// must still be bound to a running turn of its owner-authorized work.
    pub(crate) fn authorize_sudo_request(
        &self,
        id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<String, DaemonError> {
        let session = self.check_sudo_request(id, request)?;
        self.require_sudo_scope(id, request)?;
        Ok(session)
    }

    pub(super) fn check_sudo_request(
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
            .ok_or_else(|| error("sudo window ended or was revoked"))?;
        let session = self.owned.session_store.get_session(&turn.session_id)?;
        if !self.sudo_live(&turn)
            || self
                .owned
                .prompt_state_owner
                .sudo_bound_prompt(&session, &turn.agent_id)
                .is_none_or(|(entry, _)| entry != turn.entry_id)
        {
            return Err(error("sudo window ended or was revoked"));
        }
        if sudo_request_forbidden(request) {
            return Err(error("sudo cannot answer approvals, grant authority, read secrets, change the passkey or access configuration"));
        }
        self.check_sudo_creator_scope(&turn, request)?;
        Ok(turn.session_id.clone())
    }
}

pub(super) fn sudo_request_forbidden(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::RequestKernelSudo(_)
        | LocalDaemonRequest::ExtendKernelSudo(_)
        // Approvals belong to the user, including for elevated agents.
        | LocalDaemonRequest::RespondToInteraction(_)
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
            | LocalDaemonRequest::SetCredentialSecret(_)
            | LocalDaemonRequest::GetUserConfig(_)
            | LocalDaemonRequest::EndSession(_)
            | LocalDaemonRequest::DeleteSession(_)
            | LocalDaemonRequest::DeleteKernel(_)
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
            // MP-08/MP-10/MP-11 F6: credential enrollment remains owner input.
            | LocalDaemonRequest::StartProviderLogin(_)
            | LocalDaemonRequest::SendProviderLoginInput(_)
            | LocalDaemonRequest::StartSliceProviderLogin(_)
            | LocalDaemonRequest::ImportSliceProviderAuth(_)
            | LocalDaemonRequest::RequestCredentialEnrollmentInteraction(_)
            | LocalDaemonRequest::ArmDeploymentCredentialEnrollment(_)
            | LocalDaemonRequest::PrepareManagedEnvironmentGitCredentialEnrollment(_)
            | LocalDaemonRequest::SetProviderAccountCredential(_)
            | LocalDaemonRequest::GetProviderAccountProfile(_)
            | LocalDaemonRequest::ImportNativeProviderAccountProfile(_)
    ) || matches!(request, LocalDaemonRequest::SetUserConfigValue(config) if sudo_config_forbidden(&config.path))
        || matches!(request, LocalDaemonRequest::UnsetUserConfigValue(config) if sudo_config_forbidden(&config.path))
        || matches!(request, LocalDaemonRequest::SubmitPrompt(prompt) if is_sudo_prompt(&prompt.prompt) || is_sudo_control(&prompt.prompt))
        || matches!(request, LocalDaemonRequest::SubmitPrompts(prompts) if prompts.prompts.iter().any(|prompt| is_sudo_prompt(&prompt.prompt) || is_sudo_control(&prompt.prompt)))
}

fn sudo_arguments(prompt: &str) -> Option<&str> {
    prompt
        .trim_start()
        .strip_prefix("/sudo")
        .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

pub(crate) fn is_sudo_control(prompt: &str) -> bool {
    let Some(arguments) = sudo_arguments(prompt) else {
        return false;
    };
    let mut words = arguments.split_whitespace();
    if !matches!(words.next(), Some("status" | "extend" | "revoke")) {
        return false;
    }
    // MP-08/MP-10/MP-11 P2: match the terminal's complete control grammar.
    // A control verb followed by ordinary task text is an elevation prompt.
    let valid_target = words.next().is_none_or(|target| {
        target.strip_prefix("sudo:").is_some_and(|id| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    });
    valid_target && words.next().is_none()
}

pub(crate) fn is_sudo_prompt(prompt: &str) -> bool {
    sudo_arguments(prompt).is_some() && !is_sudo_control(prompt)
}

pub(super) fn parse_sudo_prompt(prompt: &str) -> Option<&str> {
    sudo_arguments(prompt).map(str::trim)
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
