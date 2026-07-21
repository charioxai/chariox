//! Session, attachment, and agent lifecycle mutations for owned runtime state.
//!
//! This module owns local state transitions that do not need provider I/O: session creation,
//! focus, attachment bookkeeping, cleanup, and projection-facing state changes.

use super::*;

pub(super) enum ProviderTerminalResizeTarget {
    None,
    Local {
        provider_run_id: String,
    },
    Remote {
        remote_execution: crate::agent::RemoteAgentBinding,
        worker_provider_run_id: String,
    },
}

impl KernelRuntimeOwnedState {
    pub(super) fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let projection_sequence = self.session_projection.change_sequence();
        let session = self.build_session_snapshot(session_id)?;
        let session = self.update_session_projection(session);
        if self.session_projection.change_sequence() != projection_sequence {
            self.runtime_projection_changes.record_change();
        }
        Ok(session)
    }

    pub(super) fn session_snapshot_without_projection_update(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.build_session_snapshot(session_id)
    }

    fn build_session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let mut session = self.session_store.get_session(session_id)?;
        let agents = self.agent_store.get_session_agents(session_id);
        session.set_agents(agents);
        self.project_session_runtime_view(&mut session);
        Ok(session)
    }

    pub(super) fn project_session_runtime_view(
        &self,
        session: &mut crate::session::RuntimeSession,
    ) {
        self.prompt_state_owner.project_into_session(session);
        let prompt_session = session.clone();
        let active_prompt_agent_id = self.prompt_state_owner.active_prompt_agent_id(session);
        let projected_run_id = crate::runtime::projection::projected_active_provider_run_id(
            session,
            |provider_run_id| {
                self.provider_store
                    .get_run(provider_run_id)
                    .ok()
                    .or_else(|| self.provider_run_projection.get(provider_run_id))
            },
            |agent_id| {
                self.provider_store
                    .get_run_for_agent(session.id(), agent_id)
                    .or_else(|| {
                        self.provider_run_projection
                            .get_for_agent(session.id(), agent_id)
                    })
            },
            |agent_id| {
                self.prompt_state_owner
                    .active_prompt_for_agent_snapshot(&prompt_session, agent_id)
                    .is_some()
            },
            active_prompt_agent_id,
        );
        session.set_active_provider_run(projected_run_id);
    }

    pub(super) fn update_session_projection(
        &self,
        mut session: crate::session::RuntimeSession,
    ) -> crate::session::RuntimeSession {
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        session
    }

    pub(super) fn create_session_response(
        &self,
        request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create session started",
            serde_json::json!({
                "workspace_id": request.workspace_id,
                "worktree_id": request.worktree_id,
                "has_alias": request.alias.as_ref().is_some_and(|alias| !alias.trim().is_empty()),
                "has_agent_defaults": request.agent_defaults.is_some(),
            }),
        );
        let mut session =
            SessionStateOwner::new(self.session_store.clone()).create_session(request)?;
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create session record stored",
            serde_json::json!({
                "session_id": session.id(),
                "workspace_id": session.workspace_id(),
                "worktree_id": session.worktree_id(),
            }),
        );
        let agent_request =
            agent_request_from_session_defaults(&session, Some(session.owner_user_id()))
                .with_worktree(session.worktree_id());
        let session_snapshot = self.session_store.get_session(session.id())?;
        let agent = self
            .agent_store
            .create_agent_for_session(agent_request, &session_snapshot)?;
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create session default agent stored",
            serde_json::json!({
                "session_id": session.id(),
                "agent_id": agent.id(),
            }),
        );
        self.session_store
            .write()
            .set_focused_agent(session.id(), Some(agent.id().to_string()))?;
        session = self.session_store.get_session(session.id())?;
        let agents = self.agent_store.get_session_agents(session.id());
        session.set_agents(agents);
        session = self.update_session_projection(session);
        self.runtime_projection_changes.record_change();
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create session completed",
            serde_json::json!({
                "session_id": session.id(),
                "agent_count": session.agents().len(),
            }),
        );
        Ok(LocalDaemonResponse::SessionCreated { session, agent })
    }

    pub(super) fn update_session_config(
        &self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        if requires_idle {
            let session = self.session_store.get_session(session_id)?;
            if self.prompt_state_owner.has_any_active_prompt(&session) {
                return Err(DaemonError::ConfigChangeRejectedWhilePromptRunning {
                    session_id: session_id.to_string(),
                });
            }
        }
        let (_session, config) = SessionStateOwner::new(self.session_store.clone()).update_config(
            session_id,
            attachment_id,
            values,
            false,
        )?;
        self.runtime_projection_changes.record_change();
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|id| id != attachment_id)
            .collect::<Vec<_>>();
        if !recipient_attachment_ids.is_empty() {
            let active_provider_run_id = self
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                .map(str::to_string);
            self.record_notice(
                session_id,
                active_provider_run_id.as_deref(),
                recipient_attachment_ids,
                format!(
                    "Attachment `{attachment_id}` updated configuration for session `{session_id}`."
                ),
            );
        }
        Ok(config)
    }

    pub(super) fn alias_session(
        &self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        SessionStateOwner::new(self.session_store.clone())
            .assign_session_alias(session_id, alias)?;
        self.session_snapshot(session_id)
    }

    pub(super) fn resize_terminal(
        &self,
        session_id: &str,
        requested_provider_run_id: Option<&str>,
    ) -> Result<ProviderTerminalResizeTarget, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let provider_run_id = requested_provider_run_id
            .map(str::to_string)
            .or_else(|| session.active_provider_run_id().map(str::to_string))
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;

        let _ = self
            .reconcile_provider_run_liveness_provider_phase(session_id, &provider_run_id, None)
            .or_else(|error| {
                if matches!(error, DaemonError::ProviderRunNotFound { .. })
                    && self
                        .provider_run_projection
                        .get(&provider_run_id)
                        .is_some_and(|run| run.session_id() == session_id)
                {
                    Ok(None)
                } else {
                    Err(error)
                }
            })?;
        let provider_run = match self.ensure_provider_run_in_session(session_id, &provider_run_id) {
            Ok(run) => run,
            Err(DaemonError::ProviderRunNotFound { .. }) => {
                let Some(projected) = self.provider_run_projection.get(&provider_run_id) else {
                    return Err(DaemonError::ProviderRunNotFound { provider_run_id });
                };
                if projected.session_id() != session_id {
                    return Err(DaemonError::ProviderRunNotInSession {
                        session_id: session_id.to_string(),
                        provider_run_id,
                    });
                }
                if projected.state() == crate::provider::ProviderRunState::Ended {
                    return Err(DaemonError::InvalidProviderRunState {
                        provider_run_id,
                        state: projected.state(),
                        operation: "resize terminal",
                    });
                }
                if projected.endpoint_mode() == crate::provider::AgentEndpointMode::External {
                    return Ok(ProviderTerminalResizeTarget::None);
                }
                let agent_id =
                    projected
                        .agent_instance_id()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "resize terminal",
                            message: format!(
                                "projected provider run `{provider_run_id}` has no agent binding"
                            ),
                        })?;
                let agent = self.agent_store.get_agent(agent_id)?;
                let remote_execution = agent.remote_execution().cloned().ok_or_else(|| {
                    DaemonError::LocalTransport {
                        operation: "resize terminal",
                        message: format!(
                            "projected provider run `{provider_run_id}` is not bound to a remote worker"
                        ),
                    }
                })?;
                let worker_provider_run_id = remote_execution
                    .active_worker_provider_run_id
                    .clone()
                    .or_else(|| {
                        crate::provider::worker_provider_run_id_from_projected_leased_id(
                            &remote_execution.leased_agent_id,
                            &provider_run_id,
                        )
                    })
                    .unwrap_or(provider_run_id);
                return Ok(ProviderTerminalResizeTarget::Remote {
                    remote_execution,
                    worker_provider_run_id,
                });
            }
            Err(error) => return Err(error),
        };
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id,
                state: provider_run.state(),
                operation: "resize terminal",
            });
        }
        if provider_run.endpoint_mode() == crate::provider::AgentEndpointMode::External {
            return Ok(ProviderTerminalResizeTarget::None);
        }
        Ok(ProviderTerminalResizeTarget::Local {
            provider_run_id: provider_run.id().to_string(),
        })
    }

    pub(super) fn end_session(
        &self,
        session_id: &str,
    ) -> Result<(crate::session::RuntimeSession, Vec<String>), DaemonError> {
        let session = self.session_store.get_session(session_id)?;

        if session.status() == crate::session::SessionStatus::Ended {
            self.clear_session_prompt_runtime_state(session_id);
            self.remove_session_workflow_dispatch_claims(session_id);
            self.prompt_state_owner.remove_session(session_id);
            self.external_provider_sessions.detach_session(session_id);
            self.attached_provider_transcript_cursors
                .detach_session(session_id);
            let ended = self.session_store.end_session(session_id)?;
            return Ok((ended, Vec::new()));
        }

        let removed_attachments = self.attachment_store.remove_session_attachments(session_id);
        let terminated_runs = self
            .provider_store
            .terminate_session_runs_provider_only(session_id)?;
        let terminated_run_ids = terminated_runs
            .runs()
            .iter()
            .map(|outcome| outcome.run().id().to_string())
            .collect::<Vec<_>>();
        for outcome in terminated_runs.into_runs() {
            if self
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(outcome.run().id())
            {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            self.provider_run_projection.update(outcome.into_run());
        }

        let removed_agents = self.agent_store.remove_session_agents(session_id);
        let removed_agent_ids: Vec<_> = removed_agents
            .iter()
            .map(|agent| format!("{} ({})", agent.agent_ref(), agent.id()))
            .collect();

        self.clear_session_prompt_runtime_state(session_id);
        self.remove_session_workflow_dispatch_claims(session_id);
        self.prompt_state_owner.remove_session(session_id);
        self.external_provider_sessions.detach_session(session_id);
        self.attached_provider_transcript_cursors
            .detach_session(session_id);
        let mut ended = self.session_store.end_session(session_id)?;
        ended.set_agents(removed_agents);
        self.runtime_projection_changes.record_change();
        crate::logging::info_with_fields(
            "daemon.session",
            "session ended",
            serde_json::json!({
                "session_id": session_id,
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        );
        Ok((ended, terminated_run_ids))
    }

    fn remove_session_workflow_dispatch_claims(&self, session_id: &str) {
        let _ = self.prompt_workspace_claims.remove_matching(|claim| {
            claim.session_id == session_id && claim.operation == "workflow_node_dispatch"
        });
    }

    pub(super) fn delete_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<(crate::session::RuntimeSession, Vec<String>), DaemonError> {
        let session = self
            .session_store
            .read()
            .resolve_session_ref(session_ref, workspace_id)?;
        let session_id = session.id().to_string();
        let (ended, terminated_run_ids) =
            if session.status() == crate::session::SessionStatus::Ended {
                self.clear_session_prompt_runtime_state(&session_id);
                self.external_provider_sessions.detach_session(&session_id);
                self.attached_provider_transcript_cursors
                    .detach_session(&session_id);
                (session, Vec::new())
            } else {
                self.end_session(&session_id)?
            };
        let mut deleted = self.session_store.delete_session(ended.id())?;
        deleted.set_agents(ended.agents().to_vec());
        self.session_projection.remove(deleted.id());
        self.runtime_projection_changes.record_change();
        crate::logging::info_with_fields(
            "daemon.session",
            "session deleted",
            serde_json::json!({
                "session_id": deleted.id(),
                "session_alias": deleted.alias(),
            }),
        );
        Ok((deleted, terminated_run_ids))
    }
}

pub(super) fn agent_request_from_session_defaults(
    session: &crate::session::RuntimeSession,
    owner_user_id: Option<&str>,
) -> crate::agent::CreateAgentRequest {
    let defaults = session.agent_defaults();
    let mut request = crate::agent::CreateAgentRequest::new(session.id(), &defaults.provider);
    if let Some(owner_user_id) = owner_user_id {
        request = request.with_owner_user_id(owner_user_id.to_string());
    }
    if let Some(model) = defaults.model.as_deref() {
        request = request.with_model(model.to_string());
    }
    if let Some(effort) = defaults.effort.as_deref() {
        request = request.with_effort(effort.to_string());
    }
    if let Some(account_profile) = defaults.account_profile.as_deref() {
        request = request.with_account_profile(account_profile.to_string());
    }
    if let Some(execution_mode) = defaults.execution_mode {
        request = request.with_execution_mode_override(execution_mode);
    }
    if let Some(permission_level) = defaults.permission_level {
        request = request.with_permission_level_override(permission_level);
    }
    request
}
