//! MP-08 / MP-10 / MP-11 S02: consent covers the task session once.
//! The session workspace and public provider catalog are part of that scope.
//! New host resources still require explicit scope authorization.
//! Extend changes time, never this envelope.
use super::*;
use sha2::{Digest, Sha256};

fn fingerprint(request: &LocalDaemonRequest) -> Result<String, DaemonError> {
    let bytes = serde_json::to_vec(request).map_err(|_| error("cannot bind sudo operation"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn inventory(request: &LocalDaemonRequest) -> bool {
    matches!(request, LocalDaemonRequest::ListSessions(_))
}

impl KernelRuntimeState {
    pub(super) fn require_sudo_scope(
        &self,
        id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        if inventory(request)
            || self.sudo_task_resource_covers(id, request)
            || self
                .owned
                .sudo_scopes
                .lock()
                .expect("sudo scopes poisoned")
                .get(id)
                .is_some_and(|scopes| fingerprint(request).is_ok_and(|key| scopes.contains(&key)))
        {
            Ok(())
        } else {
            Err(error("this typed operation is outside the owner-approved sudo scope; request fresh owner authorization through chariox_kernel_request"))
        }
    }

    /// The initial owner sheet names this resource boundary. Matching by
    /// session, rather than hashing each parameter set, lets the bound task
    /// do its already-authorized work. Keep this explicit: a request carrying
    /// a session id can also change host state, identities or capabilities.
    fn sudo_task_resource_covers(&self, id: &str, request: &LocalDaemonRequest) -> bool {
        let Some(turn) = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get(id)
            .cloned()
        else {
            return false;
        };
        let Ok(session) = self.owned.session_store.get_session(&turn.session_id) else {
            return false;
        };
        // These public metadata operations do not choose an identity, link an
        // account or expose provider credentials.
        if matches!(
            request,
            LocalDaemonRequest::GetProviderCatalog(_)
                | LocalDaemonRequest::GetProviderCommandCatalogs(_)
        ) {
            return true;
        }
        if let LocalDaemonRequest::GetProviderRun(r) = request {
            return self
                .owned
                .provider_store
                .get_run_for_agent(&turn.session_id, &turn.agent_id)
                .is_some_and(|run| {
                    run.id() == r.provider_run_id
                        && run.state() == crate::provider::ProviderRunState::Running
                })
                || self
                    .leased_agent_for_projected_run(&r.provider_run_id)
                    .is_some_and(|agent| {
                        agent.id() == turn.agent_id && agent.session_id() == turn.session_id
                    });
        }
        let workspace = match request {
            LocalDaemonRequest::ListWorkspaceFiles(r) => Some((&r.workspace_id, &r.worktree_id)),
            LocalDaemonRequest::GetWorkspaceFileContent(r) => {
                Some((&r.workspace_id, &r.worktree_id))
            }
            LocalDaemonRequest::GetWorkspaceGitOverview(r) => {
                Some((&r.workspace_id, &r.worktree_id))
            }
            LocalDaemonRequest::GenerateWorkspaceCommitMessage(r)
                if r.session_id == turn.session_id && r.agent_id == turn.agent_id =>
            {
                Some((&r.workspace_id, &r.worktree_id))
            }
            LocalDaemonRequest::CommitWorkspaceChanges(r) => {
                Some((&r.workspace_id, &r.worktree_id))
            }
            LocalDaemonRequest::PushWorkspaceBranch(r) => Some((&r.workspace_id, &r.worktree_id)),
            LocalDaemonRequest::CommitAndPushWorkspaceChanges(r) => {
                Some((&r.workspace_id, &r.worktree_id))
            }
            _ => None,
        };
        if let Some((workspace, worktree)) = workspace {
            return workspace == session.workspace_id() && worktree == session.worktree_id();
        }
        let target = match request {
            LocalDaemonRequest::GetSessionState(r) => &r.session_id,
            LocalDaemonRequest::ListAgents(r) => &r.session_id,
            LocalDaemonRequest::AliasSession(r) => &r.session_id,
            LocalDaemonRequest::AliasAgent(r) => &r.session_id,
            LocalDaemonRequest::DestroyAgent(r) => &r.session_id,
            LocalDaemonRequest::AcknowledgeAgentOutputSeen(r) => &r.session_id,
            _ => return false,
        };
        target == &turn.session_id
    }

    pub(super) fn check_sudo_creator_scope(
        &self,
        turn: &KernelSudoTurn,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        // Sudo cannot bypass the immutable creator rule or claim another
        // agent's work. Reuse the ordinary shared fences for these operations.
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "sudo-scope",
            None,
            None,
            request,
        );
        let workflow = crate::runtime::workflow_actor::is_workflow_command(request);
        if command.command_type.starts_with("agent.") || workflow {
            let target = match request {
                LocalDaemonRequest::AliasAgent(r) => Some(&r.agent_id),
                LocalDaemonRequest::DestroyAgent(r) => Some(&r.agent_id),
                LocalDaemonRequest::UpdateAgentConfig(r) => Some(&r.agent_id),
                LocalDaemonRequest::UpdateAgentProfile(r) => Some(&r.agent_id),
                LocalDaemonRequest::UpdateAgentSubstitutes(r) => Some(&r.agent_id),
                _ => None,
            };
            if let Some(target) = target {
                let actor = self.owned.agent_store.get_agent(&turn.agent_id)?;
                let child = self.owned.agent_store.get_agent(target)?;
                crate::runtime::room_tool_admission::direct_child(&actor, &child)?;
            }
            self.authorize_room_agent_request(&turn.agent_id, request)?;
            if workflow {
                self.owned
                    .ensure_workflow_request_controlled_by_metaagent(request, &turn.agent_id)?;
            }
        }
        Ok(())
    }

    pub(crate) async fn confirm_sudo_scope(
        &self,
        turn: &KernelSudoTurn,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        self.check_sudo_request(&turn.entry_id, request)?;
        if self.require_sudo_scope(&turn.entry_id, request).is_ok() {
            return Ok(());
        }
        let encoded = serde_json::to_string_pretty(request)
            .map_err(|_| error("cannot describe sudo operation"))?;
        if encoded.len() > 16_384 {
            return Err(error(
                "sudo operation is too large for an explicit owner review",
            ));
        }
        let key = fingerprint(request)?;
        let id = format!("{}:scope:{}:{key}", turn.entry_id, turn.revision);
        let remaining = turn
            .deadline
            .map_or(0, |d| {
                d.saturating_duration_since(std::time::Instant::now())
                    .as_secs()
            })
            .min(300);
        if remaining == 0 {
            return Err(error("sudo window expired"));
        }
        let interaction = RuntimeInteraction::for_kernel_operation(&id, &id, "Authorize sudo operation scope",
            format!("Agent {} requests this additional exact typed operation for owner work {} in sudo window {}. Review the agent-supplied parameters below. Approval authorizes only these parameters while the original work and window remain live; it does not extend time or permit answering approvals.\n{}", turn.agent_id, turn.task_id.as_deref().unwrap_or(&turn.entry_id), turn.entry_id, encoded),
            vec![RuntimeInteractionChoice::new("refuse", "Refuse", "refuse", None), RuntimeInteractionChoice::new("approve", "Authorize scope", "approve", None).requiring_passkey()]).with_timeout_sec(remaining);
        let rx = self
            .create_kernel_operation_interaction(&turn.session_id, &turn.owner_user_id, interaction)
            .await?;
        let answer = tokio::time::timeout(Duration::from_secs(remaining), rx)
            .await
            .map_err(|_| error("sudo scope authorization expired"))?
            .map_err(|_| error("sudo scope authorization cancelled"))?;
        if answer.choice_id.as_deref() != Some("approve") {
            return Err(error("sudo operation scope refused"));
        }
        let bound = self.sudo_for_provider_run(
            turn.provider_run_id
                .as_deref()
                .ok_or_else(|| error("sudo scope has no provider run"))?,
        )?;
        if bound.prompt_id != turn.prompt_id || bound.task_id != turn.task_id {
            return Err(error("sudo work turn changed before scope authorization"));
        }
        self.check_sudo_request(&turn.entry_id, request)?;
        let access = self.owned.sudo_turns.lock().expect("access state poisoned");
        let current = access
            .get(&turn.entry_id)
            .filter(|current| {
                current.revision == turn.revision
                    && current
                        .deadline
                        .is_some_and(|d| std::time::Instant::now() < d)
            })
            .ok_or_else(|| error("sudo window changed before scope authorization"))?;
        // Only a digest is audited; request content is not a durable receipt.
        self.owned.durable_state_store.append_event("kernel_access.sudo_scope", Some(turn.entry_id.clone()), serde_json::json!({"task_id": current.task_id, "authorization_revision": current.revision, "request_digest": key}))?;
        self.owned
            .sudo_scopes
            .lock()
            .expect("sudo scopes poisoned")
            .entry(turn.entry_id.clone())
            .or_default()
            .insert(key);
        Ok(())
    }
}
